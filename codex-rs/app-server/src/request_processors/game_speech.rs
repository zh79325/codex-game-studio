use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use codex_app_server_protocol::GameSpeechCompletedNotification;
use codex_app_server_protocol::GameSpeechErrorNotification;
use codex_app_server_protocol::GameSpeechStartResponse;
use codex_app_server_protocol::GameSpeechTranscriptNotification;
use codex_app_server_protocol::ServerNotification;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_game_app_server_adapter::RealtimeSpeechRoute;
use futures::SinkExt;
use futures::StreamExt;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::HOST;
use tokio_tungstenite::tungstenite::http::header::HeaderName;
use tokio_tungstenite::MaybeTlsStream;
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingMessageSender;
use crate::request_processors::game_speech_protocol::ServerFrame;
use crate::request_processors::game_speech_protocol::audio_request;
use crate::request_processors::game_speech_protocol::decode_server_frame;
use crate::request_processors::game_speech_protocol::full_client_request;

const FINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const PCM_SAMPLE_BYTES: u64 = 2;

#[derive(Clone)]
pub(super) struct RealtimeSpeechSessions {
    adapter: Arc<GameAppServerAdapter>,
    outgoing: Arc<OutgoingMessageSender>,
    sessions: Arc<Mutex<HashMap<String, SpeechSessionHandle>>>,
}

#[derive(Clone)]
struct SpeechSessionHandle {
    connection_id: ConnectionId,
    commands: mpsc::Sender<SpeechCommand>,
}

enum SpeechCommand {
    Audio(Vec<u8>),
    Finish,
    Cancel,
}

enum SpeechOutcome {
    Completed { text: String, duration_ms: u64 },
    Cancelled,
}

type SpeechSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

impl RealtimeSpeechSessions {
    pub(super) fn new(
        adapter: Arc<GameAppServerAdapter>,
        outgoing: Arc<OutgoingMessageSender>,
    ) -> Self {
        Self {
            adapter,
            outgoing,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(super) async fn start(
        &self,
        connection_id: ConnectionId,
    ) -> Result<GameSpeechStartResponse, String> {
        let route = self.adapter.realtime_speech_route().await?;
        let session_id = Uuid::now_v7().to_string();
        let socket = connect(&route, &session_id).await?;
        let (commands, receiver) = mpsc::channel(32);
        self.sessions.lock().await.insert(
            session_id.clone(),
            SpeechSessionHandle {
                connection_id,
                commands,
            },
        );
        let response = GameSpeechStartResponse {
            session_id: session_id.clone(),
            sample_rate: route.sample_rate,
            channels: route.channels,
            chunk_ms: route.chunk_ms,
        };
        let sessions = self.clone();
        let task_session_id = session_id;
        tokio::spawn(async move {
            let result = sessions
                .run_session(connection_id, &task_session_id, route.clone(), socket, receiver)
                .await;
            sessions.sessions.lock().await.remove(&task_session_id);
            match result {
                Ok(SpeechOutcome::Completed { text, duration_ms }) => {
                    if let Err(error) = sessions
                        .adapter
                        .record_realtime_speech_success(&route.provider_model_id)
                        .await
                    {
                        tracing::warn!(session_id = %task_session_id, %error, "failed to record ASR route success");
                    }
                    sessions
                        .notify(
                            connection_id,
                            ServerNotification::GameSpeechCompleted(
                                GameSpeechCompletedNotification {
                                    session_id: task_session_id,
                                    text,
                                    duration_ms,
                                },
                            ),
                        )
                        .await;
                }
                Ok(SpeechOutcome::Cancelled) => {}
                Err(error) => {
                    if let Err(record_error) = sessions
                        .adapter
                        .record_realtime_speech_failure(&route.provider_model_id, &error)
                        .await
                    {
                        tracing::warn!(session_id = %task_session_id, %record_error, "failed to record ASR route failure");
                    }
                    sessions
                        .notify(
                            connection_id,
                            ServerNotification::GameSpeechError(GameSpeechErrorNotification {
                                session_id: task_session_id,
                                message: error,
                            }),
                        )
                        .await;
                }
            }
        });
        Ok(response)
    }

    pub(super) async fn append_audio(
        &self,
        connection_id: ConnectionId,
        session_id: &str,
        audio_base64: &str,
    ) -> Result<(), String> {
        let audio = base64::engine::general_purpose::STANDARD
            .decode(audio_base64)
            .map_err(|error| format!("无效的语音音频数据：{error}"))?;
        if audio.is_empty() {
            return Ok(());
        }
        self.send_command(connection_id, session_id, SpeechCommand::Audio(audio))
            .await
    }

    pub(super) async fn finish(
        &self,
        connection_id: ConnectionId,
        session_id: &str,
    ) -> Result<(), String> {
        self.send_command(connection_id, session_id, SpeechCommand::Finish)
            .await
    }

    pub(super) async fn cancel(
        &self,
        connection_id: ConnectionId,
        session_id: &str,
    ) -> Result<(), String> {
        let handle = self.take_owned_session(connection_id, session_id).await?;
        handle
            .commands
            .send(SpeechCommand::Cancel)
            .await
            .map_err(|_| "语音识别会话已结束".to_string())
    }

    async fn send_command(
        &self,
        connection_id: ConnectionId,
        session_id: &str,
        command: SpeechCommand,
    ) -> Result<(), String> {
        let handle = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| "语音识别会话不存在或已结束".to_string())?;
        if handle.connection_id != connection_id {
            return Err("无权访问该语音识别会话".to_string());
        }
        handle
            .commands
            .send(command)
            .await
            .map_err(|_| "语音识别会话已结束".to_string())
    }

    async fn take_owned_session(
        &self,
        connection_id: ConnectionId,
        session_id: &str,
    ) -> Result<SpeechSessionHandle, String> {
        let mut sessions = self.sessions.lock().await;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| "语音识别会话不存在或已结束".to_string())?;
        if handle.connection_id != connection_id {
            return Err("无权访问该语音识别会话".to_string());
        }
        sessions
            .remove(session_id)
            .ok_or_else(|| "语音识别会话不存在或已结束".to_string())
    }

    async fn run_session(
        &self,
        connection_id: ConnectionId,
        session_id: &str,
        route: RealtimeSpeechRoute,
        mut socket: SpeechSocket,
        mut commands: mpsc::Receiver<SpeechCommand>,
    ) -> Result<SpeechOutcome, String> {
        let mut total_audio_bytes = 0_u64;
        let mut billed_seconds = 0_u64;
        let mut transcript = String::new();
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(SpeechCommand::Audio(audio)) => {
                        total_audio_bytes = total_audio_bytes.saturating_add(audio.len() as u64);
                        let bytes_per_second = u64::from(route.sample_rate)
                            .saturating_mul(u64::from(route.channels))
                            .saturating_mul(PCM_SAMPLE_BYTES);
                        let required_seconds = total_audio_bytes.div_ceil(bytes_per_second.max(1));
                        if required_seconds > billed_seconds {
                            let amount = required_seconds - billed_seconds;
                            self.adapter
                                .reserve_realtime_speech_usage(
                                    &route.provider_model_id,
                                    &format!("speech:{session_id}:{required_seconds}"),
                                    amount,
                                )
                                .await?;
                            billed_seconds = required_seconds;
                        }
                        socket
                            .send(Message::Binary(audio_request(&audio, false)?.into()))
                            .await
                            .map_err(|error| format!("发送语音数据失败：{error}"))?;
                    }
                    Some(SpeechCommand::Finish) => {
                        socket
                            .send(Message::Binary(audio_request(&[], true)?.into()))
                            .await
                            .map_err(|error| format!("结束语音识别失败：{error}"))?;
                        break;
                    }
                    Some(SpeechCommand::Cancel) | None => {
                        let _ = socket.close(None).await;
                        return Ok(SpeechOutcome::Cancelled);
                    }
                },
                incoming = socket.next() => {
                    if handle_incoming(
                        incoming,
                        &mut socket,
                        &self.outgoing,
                        connection_id,
                        session_id,
                        &mut transcript,
                    ).await? {
                        return Ok(SpeechOutcome::Completed {
                            text: transcript,
                            duration_ms: audio_duration_ms(total_audio_bytes, &route),
                        });
                    }
                }
            }
        }

        tokio::time::timeout(FINAL_RESPONSE_TIMEOUT, async {
            loop {
                let incoming = socket.next().await;
                if handle_incoming(
                    incoming,
                    &mut socket,
                    &self.outgoing,
                    connection_id,
                    session_id,
                    &mut transcript,
                )
                .await?
                {
                    return Ok::<(), String>(());
                }
            }
        })
        .await
        .map_err(|_| "等待语音识别最终结果超时".to_string())??;
        Ok(SpeechOutcome::Completed {
            text: transcript,
            duration_ms: audio_duration_ms(total_audio_bytes, &route),
        })
    }

    async fn notify(&self, connection_id: ConnectionId, notification: ServerNotification) {
        self.outgoing
            .send_server_notification_to_connections(&[connection_id], notification)
            .await;
    }
}

async fn connect(route: &RealtimeSpeechRoute, request_id: &str) -> Result<SpeechSocket, String> {
    let mut request = route
        .websocket_url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("无效的语音识别地址：{error}"))?;
    let headers = request.headers_mut();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(&route.api_key).map_err(|error| error.to_string())?,
    );
    headers.insert(
        HeaderName::from_static("x-api-resource-id"),
        HeaderValue::from_str(&route.resource_id).map_err(|error| error.to_string())?,
    );
    headers.insert(
        HeaderName::from_static("x-api-request-id"),
        HeaderValue::from_str(request_id).map_err(|error| error.to_string())?,
    );
    headers.insert(
        HeaderName::from_static("x-api-sequence"),
        HeaderValue::from_static("-1"),
    );
    if !headers.contains_key(HOST) {
        return Err("语音识别地址缺少 Host".to_string());
    }
    let (mut socket, response) = connect_async(request)
        .await
        .map_err(|error| format!("连接语音识别服务失败：{error}"))?;
    if let Some(log_id) = response.headers().get("x-tt-logid") {
        tracing::debug!(request_id, log_id = ?log_id, "connected to Volcengine ASR");
    }
    socket
        .send(Message::Binary(full_client_request(route)?.into()))
        .await
        .map_err(|error| format!("初始化语音识别失败：{error}"))?;
    Ok(socket)
}

async fn handle_incoming(
    incoming: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    socket: &mut SpeechSocket,
    outgoing: &OutgoingMessageSender,
    connection_id: ConnectionId,
    session_id: &str,
    transcript: &mut String,
) -> Result<bool, String> {
    match incoming {
        Some(Ok(Message::Binary(data))) => match decode_server_frame(&data)? {
            ServerFrame::Transcript {
                text,
                definite,
                is_final,
            } => {
                if text != *transcript || definite {
                    transcript.clone_from(&text);
                    outgoing
                        .send_server_notification_to_connections(
                            &[connection_id],
                            ServerNotification::GameSpeechTranscript(
                                GameSpeechTranscriptNotification {
                                    session_id: session_id.to_string(),
                                    text,
                                    definite,
                                },
                            ),
                        )
                        .await;
                }
                Ok(is_final)
            }
            ServerFrame::Error(error) => Err(error),
            ServerFrame::Ignored => Ok(false),
        },
        Some(Ok(Message::Ping(data))) => {
            socket
                .send(Message::Pong(data))
                .await
                .map_err(|error| format!("语音识别连接响应失败：{error}"))?;
            Ok(false)
        }
        Some(Ok(Message::Close(_))) | None => Ok(true),
        Some(Ok(_)) => Ok(false),
        Some(Err(error)) => Err(format!("读取语音识别结果失败：{error}")),
    }
}

fn audio_duration_ms(total_audio_bytes: u64, route: &RealtimeSpeechRoute) -> u64 {
    let bytes_per_second = u64::from(route.sample_rate)
        .saturating_mul(u64::from(route.channels))
        .saturating_mul(PCM_SAMPLE_BYTES);
    total_audio_bytes
        .saturating_mul(1_000)
        .div_ceil(bytes_per_second.max(1))
}
