use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_extension_api::ExtensionDataInit;
use codex_game_app_server_adapter::AiExecutionRoute;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_game_runtime::CodexExecutionPort;
use codex_game_runtime::ExecutionError;
use codex_game_runtime::StartThreadRequest;
use codex_game_runtime::StartTurnRequest;
use codex_game_runtime::StartedThread;
use codex_game_runtime::StartedTurn;
use codex_game_runtime::SteerTurnRequest;
use codex_game_runtime::TurnAuditContext;
use codex_game_runtime::append_turn_audit_stream_termination;
use codex_http_client::StreamResponseAudit;
use codex_http_client::StreamResponseAuditEvent;
use codex_http_client::register_stream_response_audit;
use codex_http_client::unregister_stream_response_audit;
use codex_image_generation_extension::ImageApiDialect;
use codex_image_generation_extension::ImageGenerationRouteOverride;
use codex_image_generation_extension::ImageGenerationToolRouteOverride;
use codex_image_generation_extension::ImageGenerationTurnGate;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Op;
use codex_protocol::turn_input::StartIfIdleSubmission;
use codex_protocol::turn_input::SteerSubmission;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_redacted_string::RedactedString;
use std::collections::HashMap;
use std::sync::Arc;

use crate::outgoing_message::ConnectionId;
use crate::request_processors::ListenerTaskContext;
use crate::request_processors::ensure_conversation_listener;

pub(crate) struct AppServerCodexExecutionPort {
    thread_manager: Arc<ThreadManager>,
    base_config: Arc<Config>,
    game_adapter: Arc<GameAppServerAdapter>,
    listener_context: ListenerTaskContext,
}

pub(crate) struct ConnectionCodexExecutionPort<'a> {
    execution: &'a AppServerCodexExecutionPort,
    connection_id: ConnectionId,
}

struct ResolvedModelProvider {
    model: String,
    driver: String,
    provider: ModelProviderInfo,
}

struct GameTurnStreamAudit {
    context: TurnAuditContext,
}

impl StreamResponseAudit for GameTurnStreamAudit {
    fn record_stream_event(&self, event: StreamResponseAuditEvent) {
        let termination = match event {
            StreamResponseAuditEvent::StreamTerminated { stage, reason } => Some((stage, reason)),
            StreamResponseAuditEvent::EventConsumerDropped { stage } => Some((
                stage,
                "downstream event consumer closed; provider stream reading stopped".to_string(),
            )),
            StreamResponseAuditEvent::DeltaWithoutActiveItem {
                event_type,
                delta_bytes,
                action: "panic",
            } => Some((
                "turn_state_machine",
                format!("event_type={event_type}; delta_bytes={delta_bytes}; action=panic"),
            )),
            StreamResponseAuditEvent::DeltaWithoutActiveItem { .. } => None,
        };
        let Some((stage, reason)) = termination else {
            return;
        };
        if let Err(error) = append_turn_audit_stream_termination(&self.context, stage, &reason) {
            tracing::warn!(
                attempt_id = %self.context.attempt_id,
                "failed to write game turn stream termination to audit: {error}"
            );
        }
    }
}

impl AppServerCodexExecutionPort {
    pub(crate) fn new(
        thread_manager: Arc<ThreadManager>,
        base_config: Arc<Config>,
        game_adapter: Arc<GameAppServerAdapter>,
        listener_context: ListenerTaskContext,
    ) -> Self {
        Self {
            thread_manager,
            base_config,
            game_adapter,
            listener_context,
        }
    }

    pub(crate) fn scoped(&self, connection_id: ConnectionId) -> ConnectionCodexExecutionPort<'_> {
        ConnectionCodexExecutionPort {
            execution: self,
            connection_id,
        }
    }

    async fn thread(
        &self,
        thread_id: &str,
    ) -> Result<Arc<codex_core::CodexThread>, ExecutionError> {
        let thread_id = ThreadId::from_string(thread_id)
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        self.thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))
    }

    async fn resolve_model_provider(
        &self,
        route: &codex_game_runtime::RouteDecision,
    ) -> Result<ResolvedModelProvider, ExecutionError> {
        let resolved = self
            .game_adapter
            .resolve_ai_execution_route(route)
            .await
            .map_err(|message| ExecutionError::RouteUnavailable {
                route: route.clone(),
                message,
            })?;
        let model = resolved.model.clone();
        let driver = resolved.driver.clone();
        let provider =
            model_provider_info(resolved).map_err(|message| ExecutionError::RouteUnavailable {
                route: route.clone(),
                message,
            })?;
        Ok(ResolvedModelProvider {
            model,
            driver,
            provider,
        })
    }
}

fn model_provider_info(route: AiExecutionRoute) -> Result<ModelProviderInfo, String> {
    let (experimental_bearer_token, http_headers) = match route.auth_style.as_str() {
        "bearer" => (Some(RedactedString::from(route.api_key)), None),
        "x-api-key" => (
            None,
            Some(HashMap::from([(
                "x-api-key".to_string(),
                RedactedString::from(route.api_key),
            )])),
        ),
        other => return Err(format!("不支持的 Provider 鉴权方式：{other}")),
    };
    Ok(ModelProviderInfo {
        name: route.provider_name,
        base_url: Some(route.base_url),
        experimental_bearer_token,
        http_headers,
        stream_max_retries: Some(0),
        stream_idle_timeout_ms: Some(60_000),
        ..Default::default()
    })
}

impl CodexExecutionPort for ConnectionCodexExecutionPort<'_> {
    async fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        let started = self.execution.start_thread(request).await?;
        let thread_id = ThreadId::from_string(&started.thread_id)
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        ensure_conversation_listener(
            self.execution.listener_context.clone(),
            thread_id,
            self.connection_id,
            false,
        )
        .await
        .map_err(|error| ExecutionError::Retryable(error.message))?;
        Ok(started)
    }

    async fn thread_available(&self, thread_id: &str) -> bool {
        self.execution.thread_available(thread_id).await
    }

    async fn start_turn(&self, request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        self.execution.start_turn(request).await
    }

    async fn steer_turn(&self, request: SteerTurnRequest) -> Result<(), ExecutionError> {
        self.execution.steer_turn(request).await
    }

    async fn interrupt_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> Result<(), ExecutionError> {
        self.execution.interrupt_turn(thread_id, turn_id).await
    }
}

impl CodexExecutionPort for AppServerCodexExecutionPort {
    async fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        let cwd = AbsolutePathBuf::from_absolute_path_checked(&request.cwd)
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        let resolved = self.resolve_model_provider(&request.route).await?;
        let mut config = self.base_config.as_ref().clone();
        config.model = Some(resolved.model);
        config.model_provider = resolved.provider;
        config.model_provider_id = request.route.provider.clone();
        let mut thread_extension_init = ExtensionDataInit::new();
        if !request.image_generation_tools.is_empty() {
            let media_stage = match request.stage.as_str() {
                "render" => "render",
                "views" => "views",
                _ => {
                    return Err(ExecutionError::InvalidRequest(format!(
                        "image generation is unavailable for stage {}",
                        request.stage
                    )));
                }
            };
            let mut tools = Vec::with_capacity(request.image_generation_tools.len());
            for tool in request.image_generation_tools {
                let resolved = self.resolve_model_provider(&tool.route).await?;
                let api_dialect =
                    ImageApiDialect::try_from(resolved.driver.as_str()).map_err(|message| {
                        ExecutionError::RouteUnavailable {
                            route: tool.route.clone(),
                            message,
                        }
                    })?;
                tools.push(ImageGenerationToolRouteOverride {
                    provider: resolved.provider,
                    model: resolved.model,
                    api_dialect,
                    save_root: Some(cwd.join("media").join(media_stage)),
                    tool_name: tool.tool_name,
                });
            }
            thread_extension_init.insert(ImageGenerationRouteOverride { tools });
        }
        config.cwd = cwd.clone();
        config.workspace_roots = vec![cwd];
        config.workspace_roots_explicit = true;
        let mut options = codex_core::StartThreadOptions::new(config);
        options.thread_extension_init = thread_extension_init;
        let started = self
            .thread_manager
            .start_thread(options)
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))?;
        Ok(StartedThread {
            thread_id: started.thread_id.to_string(),
            session_id: started.session_configured.session_id.to_string(),
        })
    }

    async fn thread_available(&self, thread_id: &str) -> bool {
        self.thread(thread_id).await.is_ok()
    }

    async fn start_turn(&self, request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        let thread = self.thread(&request.thread_id).await?;
        if let Some(gate) = thread
            .thread_extension_data()
            .get::<ImageGenerationTurnGate>()
        {
            gate.begin_scope(request.media_generation_scope.clone());
        }
        let input = request
            .model_input()
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        let mut user_input = vec![UserInput::Text {
            text: input,
            text_elements: Vec::new(),
        }];
        for path in request.local_image_paths {
            let path = AbsolutePathBuf::from_absolute_path_checked(path)
                .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
            user_input.push(UserInput::LocalImage {
                path: path.to_path_buf(),
                detail: None,
            });
        }
        let mut turn = TurnInputRequest::user_input(user_input);
        if !request.context.output_schema.trim().is_empty() {
            let schema = serde_json::from_str(&request.context.output_schema).map_err(|error| {
                ExecutionError::InvalidRequest(format!("invalid output schema: {error}"))
            })?;
            turn.start.final_output_json_schema = Some(schema);
        }
        turn.start.max_output_tokens = request.max_output_tokens;
        let audit_registered = if let Some(context) = request.audit_context {
            register_stream_response_audit(
                request.thread_id.clone(),
                Arc::new(GameTurnStreamAudit { context }),
            );
            true
        } else {
            false
        };
        let submission = match thread.start_turn_if_idle(turn).await {
            Ok(submission) => submission,
            Err(error) => {
                if audit_registered {
                    unregister_stream_response_audit(&request.thread_id);
                }
                return Err(ExecutionError::Retryable(error.to_string()));
            }
        };
        match submission {
            StartIfIdleSubmission::Started { turn_id } => Ok(StartedTurn { turn_id }),
            StartIfIdleSubmission::NotSubmitted { reason } => {
                if audit_registered {
                    unregister_stream_response_audit(&request.thread_id);
                }
                Err(ExecutionError::Retryable(format!("{reason:?}")))
            }
        }
    }

    async fn steer_turn(&self, request: SteerTurnRequest) -> Result<(), ExecutionError> {
        let thread = self.thread(&request.thread_id).await?;
        let turn = TurnInputRequest::user_input(vec![UserInput::Text {
            text: request.message,
            text_elements: Vec::new(),
        }]);
        match thread
            .steer_turn(turn, request.expected_turn_id)
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))?
        {
            SteerSubmission::Steered { .. } => Ok(()),
            SteerSubmission::NotSubmitted { reason } => {
                Err(ExecutionError::InvalidRequest(format!("{reason:?}")))
            }
        }
    }

    async fn interrupt_turn(
        &self,
        thread_id: String,
        _turn_id: String,
    ) -> Result<(), ExecutionError> {
        self.thread(&thread_id)
            .await?
            .submit(Op::Interrupt)
            .await
            .map(|_| ())
            .map_err(|error| ExecutionError::Retryable(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution_route(auth_style: &str) -> AiExecutionRoute {
        AiExecutionRoute {
            provider_code: "provider".to_string(),
            provider_name: "Provider".to_string(),
            model: "model".to_string(),
            driver: "openai".to_string(),
            base_url: "https://example.test/v1".to_string(),
            auth_style: auth_style.to_string(),
            api_key: "secret-key".to_string(),
        }
    }

    #[test]
    fn builds_redacted_bearer_auth_from_database_route() {
        let provider = model_provider_info(execution_route("bearer")).expect("provider");
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://example.test/v1")
        );
        assert_eq!(
            provider
                .experimental_bearer_token
                .as_ref()
                .map(|token| token.as_str()),
            Some("secret-key")
        );
        assert!(provider.http_headers.is_none());
        assert_eq!(
            format!("{:?}", provider.experimental_bearer_token),
            "Some(<redacted>)"
        );
    }

    #[test]
    fn builds_redacted_x_api_key_auth_from_database_route() {
        let provider = model_provider_info(execution_route("x-api-key")).expect("provider");
        assert!(provider.experimental_bearer_token.is_none());
        assert_eq!(
            provider
                .http_headers
                .as_ref()
                .and_then(|headers| headers.get("x-api-key"))
                .map(|key| key.as_str()),
            Some("secret-key")
        );
        assert_eq!(
            format!("{:?}", provider.http_headers),
            "Some({\"x-api-key\": <redacted>})"
        );
    }
}
