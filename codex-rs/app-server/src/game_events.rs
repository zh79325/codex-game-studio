use codex_app_server_protocol::GameAgentHandoffNotification;
use codex_app_server_protocol::GameAttemptUpdatedNotification;
use codex_app_server_protocol::GameCharacterUpdatedNotification;
use codex_app_server_protocol::GameConversationActorNotification;
use codex_app_server_protocol::GameConversationDeltaNotification;
use codex_app_server_protocol::GameConversationErrorNotification;
use codex_app_server_protocol::GameConversationFocusNotification;
use codex_app_server_protocol::GameConversationTurnNotification;
use codex_app_server_protocol::GameGenerationUpdatedNotification;
use codex_app_server_protocol::GameTaskUpdatedNotification;
use codex_app_server_protocol::ServerNotification;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_game_runtime::TurnAuditCompletion;
use codex_game_runtime::TurnAuditUsage;
use codex_game_runtime::append_turn_audit_completion;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::game_execution_port::AppServerCodexExecutionPort;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingMessageSender;

#[derive(Debug, Clone)]
pub(crate) struct GameThreadEvent {
    pub(crate) thread_id: ThreadId,
    pub(crate) connection_ids: Vec<ConnectionId>,
    pub(crate) event: Event,
}

pub(crate) type GameEventSender = mpsc::UnboundedSender<GameThreadEvent>;
pub(crate) type GameEventReceiver = mpsc::UnboundedReceiver<GameThreadEvent>;

pub(crate) fn game_event_channel() -> (GameEventSender, GameEventReceiver) {
    mpsc::unbounded_channel()
}

pub(crate) fn spawn_game_event_observer(
    mut receiver: GameEventReceiver,
    adapter: Arc<GameAppServerAdapter>,
    execution: Arc<AppServerCodexExecutionPort>,
    outgoing: Arc<OutgoingMessageSender>,
) {
    tokio::spawn(async move {
        let mut active_game_turns = HashMap::<ThreadId, String>::new();
        let mut usage_by_thread = HashMap::<ThreadId, TurnAuditUsage>::new();
        let mut partial_response_by_turn = HashMap::<String, String>::new();
        while let Some(observed) = receiver.recv().await {
            if let EventMsg::TurnStarted(started) = &observed.event.msg {
                if matches!(
                    adapter.turn_event_context(&started.turn_id).await,
                    Ok(Some(_))
                ) {
                    active_game_turns.insert(observed.thread_id, started.turn_id.clone());
                }
                continue;
            }
            if let EventMsg::TokenCount(token_count) = &observed.event.msg {
                if active_game_turns.contains_key(&observed.thread_id)
                    && let Some(info) = &token_count.info
                {
                    let usage = &info.last_token_usage;
                    usage_by_thread.insert(
                        observed.thread_id,
                        TurnAuditUsage {
                            input_tokens: usage.input_tokens,
                            cached_input_tokens: usage.cached_input_tokens,
                            output_tokens: usage.output_tokens,
                            reasoning_output_tokens: usage.reasoning_output_tokens,
                            total_tokens: usage.total_tokens,
                        },
                    );
                }
                continue;
            }
            if let EventMsg::AgentMessageContentDelta(delta) = &observed.event.msg {
                match adapter.turn_event_context(&delta.turn_id).await {
                    Ok(Some(context)) => {
                        active_game_turns.insert(observed.thread_id, delta.turn_id.clone());
                        partial_response_by_turn
                            .entry(delta.turn_id.clone())
                            .or_default()
                            .push_str(&delta.delta);
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationDelta(
                                GameConversationDeltaNotification {
                                    conversation_id: context.conversation_id,
                                    turn_id: delta.turn_id.clone(),
                                    agent_code: context.agent_code,
                                    delta: delta.delta.clone(),
                                },
                            ))
                            .await;
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(
                        thread_id = %observed.thread_id,
                        turn_id = %delta.turn_id,
                        "failed to resolve game delta context: {error}"
                    ),
                }
                continue;
            }
            let terminal_error = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => {
                    completed.error.as_ref().map(|error| error.message.clone())
                }
                _ => None,
            };
            let turn_id = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => completed.turn_id.as_str(),
                EventMsg::TurnAborted(aborted) => {
                    aborted.turn_id.as_deref().unwrap_or(&observed.event.id)
                }
                _ => continue,
            };
            active_game_turns.remove(&observed.thread_id);
            match adapter.turn_audit_context(turn_id).await {
                Ok(Some(context)) => {
                    let completion = match &observed.event.msg {
                        EventMsg::TurnComplete(completed) => TurnAuditCompletion {
                            response: completed.last_agent_message.clone(),
                            partial_response: partial_response_by_turn.remove(turn_id),
                            error: terminal_error.clone(),
                            usage: usage_by_thread.remove(&observed.thread_id),
                            duration_ms: completed.duration_ms,
                            time_to_first_token_ms: completed.time_to_first_token_ms,
                        },
                        EventMsg::TurnAborted(aborted) => TurnAuditCompletion {
                            partial_response: partial_response_by_turn.remove(turn_id),
                            error: Some(format!("运行已中断：{:?}", aborted.reason)),
                            usage: usage_by_thread.remove(&observed.thread_id),
                            duration_ms: aborted.duration_ms,
                            ..TurnAuditCompletion::default()
                        },
                        _ => unreachable!(),
                    };
                    if let Err(error) = append_turn_audit_completion(&context, &completion) {
                        tracing::warn!(
                            thread_id = %observed.thread_id,
                            turn_id,
                            "failed to write game turn audit completion: {error}"
                        );
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    thread_id = %observed.thread_id,
                    turn_id,
                    "failed to resolve game turn audit context: {error}"
                ),
            }
            let result = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => {
                    adapter
                        .observe_turn_completed(
                            &completed.turn_id,
                            completed.last_agent_message.as_deref(),
                            completed.error.is_some(),
                        )
                        .await
                }
                EventMsg::TurnAborted(_) => adapter.observe_turn_aborted(turn_id).await,
                _ => unreachable!(),
            };
            match result {
                Ok(Some(projection)) => {
                    let conversation_id = projection.conversation_id.clone();
                    outgoing
                        .send_server_notification(ServerNotification::GameAttemptUpdated(
                            GameAttemptUpdatedNotification {
                                conversation_id: conversation_id.clone(),
                                task_id: projection.task_id.clone(),
                                attempt_id: projection.attempt_id,
                                turn_id: Some(turn_id.to_string()),
                                status: projection.status.clone(),
                            },
                        ))
                        .await;
                    outgoing
                        .send_server_notification(ServerNotification::GameTaskUpdated(
                            GameTaskUpdatedNotification {
                                conversation_id: conversation_id.clone(),
                                task_id: projection.task_id,
                                status: projection.status.clone(),
                            },
                        ))
                        .await;
                    outgoing
                        .send_server_notification(ServerNotification::GameConversationTurn(
                            GameConversationTurnNotification {
                                conversation_id: conversation_id.clone(),
                                status: projection.status,
                            },
                        ))
                        .await;
                    if let Some(agent_code) = projection.agent_code.clone() {
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationActor(
                                GameConversationActorNotification {
                                    conversation_id: conversation_id.clone(),
                                    turn_id: Some(turn_id.to_string()),
                                    agent_code: agent_code.clone(),
                                    status: "idle".to_string(),
                                },
                            ))
                            .await;
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationFocus(
                                GameConversationFocusNotification {
                                    conversation_id: conversation_id.clone(),
                                    agent_code: projection
                                        .handoff_target
                                        .clone()
                                        .unwrap_or(agent_code),
                                },
                            ))
                            .await;
                    }
                    if let Some(message) = terminal_error {
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationError(
                                GameConversationErrorNotification {
                                    conversation_id: conversation_id.clone(),
                                    turn_id: Some(turn_id.to_string()),
                                    message,
                                },
                            ))
                            .await;
                    }
                    if let Some(character) = projection.character {
                        outgoing
                            .send_server_notification(ServerNotification::GameCharacterUpdated(
                                GameCharacterUpdatedNotification { character },
                            ))
                            .await;
                    }
                    for generation in projection.generations {
                        outgoing
                            .send_server_notification(ServerNotification::GameGenerationUpdated(
                                GameGenerationUpdatedNotification { generation },
                            ))
                            .await;
                    }
                    if let Some(target_agent) = projection.handoff_target {
                        let from_agent = projection.agent_code.unwrap_or_default();
                        let reason = projection.handoff_reason.unwrap_or_default();
                        outgoing
                            .send_server_notification(ServerNotification::GameAgentHandoff(
                                GameAgentHandoffNotification {
                                    conversation_id: conversation_id.clone(),
                                    from_agent_code: from_agent,
                                    to_agent_code: target_agent.clone(),
                                    reason,
                                },
                            ))
                            .await;
                        let Some(connection_id) = observed.connection_ids.first().copied() else {
                            tracing::warn!(
                                thread_id = %observed.thread_id,
                                "cannot continue game handoff without a subscribed connection"
                            );
                            continue;
                        };
                        let scoped_execution = execution.scoped(connection_id);
                        match adapter
                            .continue_handoff(&scoped_execution, &conversation_id, &target_agent)
                            .await
                        {
                            Ok(Some(started)) => {
                                let task_id = started.task.id.as_str().to_string();
                                let agent_code = started.task.agent_code;
                                let turn_id = started.attempt.codex_turn_id;
                                outgoing
                                    .send_server_notification(ServerNotification::GameTaskUpdated(
                                        GameTaskUpdatedNotification {
                                            conversation_id: conversation_id.clone(),
                                            task_id: task_id.clone(),
                                            status: "running".to_string(),
                                        },
                                    ))
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameAttemptUpdated(
                                            GameAttemptUpdatedNotification {
                                                conversation_id: conversation_id.clone(),
                                                task_id,
                                                attempt_id: started.attempt.id.as_str().to_string(),
                                                turn_id: turn_id.clone(),
                                                status: "running".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationTurn(
                                            GameConversationTurnNotification {
                                                conversation_id: conversation_id.clone(),
                                                status: "running".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationActor(
                                            GameConversationActorNotification {
                                                conversation_id: conversation_id.clone(),
                                                turn_id,
                                                agent_code: agent_code.clone(),
                                                status: "working".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationFocus(
                                            GameConversationFocusNotification {
                                                conversation_id,
                                                agent_code,
                                            },
                                        ),
                                    )
                                    .await;
                            }
                            Ok(None) => {
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationTurn(
                                            GameConversationTurnNotification {
                                                conversation_id,
                                                status: "blocked".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                            }
                            Err(error) => {
                                tracing::warn!(
                                    thread_id = %observed.thread_id,
                                    "failed to continue game handoff: {error}"
                                );
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationError(
                                            GameConversationErrorNotification {
                                                conversation_id,
                                                turn_id: Some(turn_id.to_string()),
                                                message: error,
                                            },
                                        ),
                                    )
                                    .await;
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        thread_id = %observed.thread_id,
                        event_id = %observed.event.id,
                        "failed to project game turn event: {error}"
                    );
                }
            }
        }
    });
}
