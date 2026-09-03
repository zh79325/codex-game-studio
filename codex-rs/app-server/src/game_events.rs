use codex_app_server_protocol::GameArtifactCommittedNotification;
use codex_app_server_protocol::GameAttemptUpdatedNotification;
use codex_app_server_protocol::GameDesignConfirmationRequiredNotification;
use codex_app_server_protocol::GameTaskUpdatedNotification;
use codex_app_server_protocol::GameWorkflowUpdatedNotification;
use codex_app_server_protocol::ServerNotification;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
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
        while let Some(observed) = receiver.recv().await {
            let (turn_id, result) = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => (
                    completed.turn_id.as_str(),
                    adapter
                        .observe_turn_completed(
                            &completed.turn_id,
                            completed.last_agent_message.as_deref(),
                            completed.error.is_some(),
                        )
                        .await,
                ),
                EventMsg::TurnAborted(aborted) => {
                    let turn_id = aborted.turn_id.as_deref().unwrap_or(&observed.event.id);
                    (turn_id, adapter.observe_turn_aborted(turn_id).await)
                }
                _ => continue,
            };
            match result {
                Ok(Some(projection)) => {
                    let conversation_id = projection.conversation_id;
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
                                status: projection.status,
                            },
                        ))
                        .await;
                    for (artifact_id, artifact_type) in projection.artifacts {
                        outgoing
                            .send_server_notification(ServerNotification::GameArtifactCommitted(
                                GameArtifactCommittedNotification {
                                    conversation_id: conversation_id.clone(),
                                    artifact_id,
                                    artifact_type,
                                },
                            ))
                            .await;
                    }
                    let workflow_id = projection
                        .workflow
                        .as_ref()
                        .map(|workflow| workflow.id.clone());
                    if let Some(workflow) = projection.workflow {
                        let should_start_synthesis = workflow.state == "merging";
                        outgoing
                            .send_server_notification(ServerNotification::GameWorkflowUpdated(
                                GameWorkflowUpdatedNotification { workflow },
                            ))
                            .await;
                        if should_start_synthesis {
                            let Some(connection_id) = observed.connection_ids.first().copied()
                            else {
                                tracing::warn!(
                                    thread_id = %observed.thread_id,
                                    "cannot start game synthesis without a subscribed connection"
                                );
                                continue;
                            };
                            let scoped_execution = execution.scoped(connection_id);
                            match adapter
                                .start_synthesis(&scoped_execution, &conversation_id)
                                .await
                            {
                                Ok(started) => {
                                    outgoing
                                        .send_server_notification(
                                            ServerNotification::GameTaskUpdated(
                                                GameTaskUpdatedNotification {
                                                    conversation_id: conversation_id.clone(),
                                                    task_id: started.task.id.as_str().to_string(),
                                                    status: "running".to_string(),
                                                },
                                            ),
                                        )
                                        .await;
                                    outgoing
                                        .send_server_notification(
                                            ServerNotification::GameAttemptUpdated(
                                                GameAttemptUpdatedNotification {
                                                    conversation_id: conversation_id.clone(),
                                                    task_id: started.task.id.as_str().to_string(),
                                                    attempt_id: started
                                                        .attempt
                                                        .id
                                                        .as_str()
                                                        .to_string(),
                                                    turn_id: started.attempt.codex_turn_id,
                                                    status: "running".to_string(),
                                                },
                                            ),
                                        )
                                        .await;
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        thread_id = %observed.thread_id,
                                        "failed to start game synthesis: {error}"
                                    );
                                }
                            }
                        }
                    }
                    if let (Some(workflow_id), Some(conflict_count)) =
                        (workflow_id, projection.conflict_count)
                    {
                        outgoing
                            .send_server_notification(
                                ServerNotification::GameDesignConfirmationRequired(
                                    GameDesignConfirmationRequiredNotification {
                                        conversation_id,
                                        workflow_id,
                                        conflict_count,
                                    },
                                ),
                            )
                            .await;
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
