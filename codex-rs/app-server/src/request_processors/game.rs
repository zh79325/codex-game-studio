use codex_app_server_protocol::*;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_thread_store::CreateProjectParams as StoreCreateProjectParams;
use codex_thread_store::StoredProjectRoot;
use codex_thread_store::ThreadStore;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::error_code::method_not_found;
use crate::game_execution_port::AppServerCodexExecutionPort;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingMessageSender;

pub(crate) struct GameRequestProcessor {
    adapter: Arc<GameAppServerAdapter>,
    execution: Arc<AppServerCodexExecutionPort>,
    thread_store: Arc<dyn ThreadStore>,
    outgoing: Arc<OutgoingMessageSender>,
}

impl GameRequestProcessor {
    pub(crate) fn new(
        adapter: Arc<GameAppServerAdapter>,
        execution: Arc<AppServerCodexExecutionPort>,
        thread_store: Arc<dyn ThreadStore>,
        outgoing: Arc<OutgoingMessageSender>,
    ) -> Self {
        Self {
            adapter,
            execution,
            thread_store,
            outgoing,
        }
    }
    pub(crate) fn ping(&self, _params: GamePingParams) -> GamePingResponse {
        self.adapter.ping()
    }

    pub(crate) async fn project_create(
        &self,
        params: GameProjectCreateParams,
    ) -> std::result::Result<GameProjectCreateResponse, JSONRPCErrorError> {
        let name = params.name.trim().to_string();
        if name.is_empty() {
            return Err(invalid_params("project name must not be empty"));
        }
        let root = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path_checked(
            PathBuf::from(&params.root),
        )
        .map_err(|error| invalid_params(format!("invalid project root: {error}")))?
        .to_string_lossy()
        .into_owned();
        let idempotency_key = format!("game-project:create:{root}");
        if idempotency_key.len() > 512 {
            return Err(invalid_params("project root is too long"));
        }
        let mut metadata = BTreeMap::new();
        metadata.insert("codex.game.kind".to_string(), "focus".to_string());
        let created = self
            .thread_store
            .create_project(StoreCreateProjectParams {
                name,
                roots: vec![StoredProjectRoot { path: root }],
                metadata,
                thread_ids: Vec::new(),
                idempotency_key,
            })
            .await
            .map_err(game_project_store_error)?;
        let project_id = created.project.id.clone();
        let project_root = created
            .project
            .roots
            .first()
            .ok_or_else(|| internal_error("created game project has no root"))?
            .path
            .clone();
        self.notify_recovery_status(GameBackendStatus::Recovering)
            .await;
        let response = self
            .adapter
            .project_create(
                project_id.clone(),
                GameProjectCreateParams {
                    name: created.project.name,
                    root: project_root,
                },
            )
            .await;
        self.notify_recovery_status(self.adapter.ping().status)
            .await;
        match response {
            Ok(response) => {
                if created.created {
                    self.outgoing
                        .send_server_notification(ServerNotification::ProjectChanged(
                            ProjectChangedNotification {
                                project_id,
                                change_type: ProjectChangeType::Created,
                            },
                        ))
                        .await;
                }
                Ok(response)
            }
            Err(error) => {
                if created.created {
                    let _ = self.thread_store.delete_project(project_id).await;
                }
                Err(game_error(error))
            }
        }
    }

    pub(crate) async fn project_open(
        &self,
        params: GameProjectOpenParams,
    ) -> std::result::Result<GameProjectOpenResponse, JSONRPCErrorError> {
        self.notify_recovery_status(GameBackendStatus::Recovering)
            .await;
        let response = self.adapter.project_open(params).await.map_err(game_error);
        self.notify_recovery_status(self.adapter.ping().status)
            .await;
        response
    }

    pub(crate) fn project_read(
        &self,
        params: GameProjectReadParams,
    ) -> std::result::Result<GameProjectReadResponse, JSONRPCErrorError> {
        self.adapter.project_read(params).map_err(game_error)
    }

    pub(crate) async fn project_list(
        &self,
        params: GameProjectListParams,
    ) -> std::result::Result<GameProjectListResponse, JSONRPCErrorError> {
        self.adapter.project_list(params).await.map_err(game_error)
    }

    pub(crate) async fn project_import(
        &self,
        params: GameProjectImportParams,
    ) -> std::result::Result<GameProjectImportResponse, JSONRPCErrorError> {
        self.notify_recovery_status(GameBackendStatus::Recovering)
            .await;
        let response = self
            .adapter
            .project_import(params)
            .await
            .map_err(game_error);
        self.notify_recovery_status(self.adapter.ping().status)
            .await;
        response
    }

    pub(crate) async fn conversation_ensure(
        &self,
        params: GameConversationEnsureParams,
    ) -> std::result::Result<GameConversationEnsureResponse, JSONRPCErrorError> {
        self.adapter
            .conversation_ensure(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn conversation_submit(
        &self,
        connection_id: ConnectionId,
        params: GameConversationSubmitParams,
    ) -> std::result::Result<GameConversationSubmitResponse, JSONRPCErrorError> {
        let conversation_id = params.conversation_id.clone();
        let execution = self.execution.scoped(connection_id);
        let (response, task_execution) = self
            .adapter
            .conversation_submit(&execution, params)
            .await
            .map_err(game_error)?;
        self.outgoing
            .send_server_notification(ServerNotification::GameTaskUpdated(
                GameTaskUpdatedNotification {
                    conversation_id: conversation_id.clone(),
                    task_id: task_execution.task.id.as_str().to_string(),
                    status: "running".to_string(),
                },
            ))
            .await;
        self.outgoing
            .send_server_notification(ServerNotification::GameAttemptUpdated(
                GameAttemptUpdatedNotification {
                    conversation_id,
                    task_id: task_execution.task.id.as_str().to_string(),
                    attempt_id: task_execution.attempt.id.as_str().to_string(),
                    turn_id: task_execution.attempt.codex_turn_id,
                    status: "running".to_string(),
                },
            ))
            .await;
        Ok(response)
    }

    pub(crate) fn conversation_read(
        &self,
        params: GameConversationReadParams,
    ) -> std::result::Result<GameConversationReadResponse, JSONRPCErrorError> {
        self.adapter.conversation_read(params).map_err(game_error)
    }

    pub(crate) async fn focus_start(
        &self,
        params: GameFocusStartParams,
    ) -> std::result::Result<GameFocusStartResponse, JSONRPCErrorError> {
        let response = self.adapter.focus_start(params).await.map_err(game_error)?;
        self.notify_workflow_updated(response.workflow.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn focus_read(
        &self,
        params: GameFocusReadParams,
    ) -> std::result::Result<GameFocusReadResponse, JSONRPCErrorError> {
        self.adapter.focus_read(params).await.map_err(game_error)
    }

    pub(crate) async fn focus_decide(
        &self,
        connection_id: ConnectionId,
        params: GameFocusDecideParams,
    ) -> std::result::Result<GameFocusDecideResponse, JSONRPCErrorError> {
        let conversation_id = params.conversation_id.clone();
        let execution = self.execution.scoped(connection_id);
        let (response, tasks, artifacts) = self
            .adapter
            .focus_decide(&execution, params)
            .await
            .map_err(game_error)?;
        self.notify_workflow_updated(response.workflow.clone())
            .await;
        for task in tasks {
            self.notify_task_started(&conversation_id, task).await;
        }
        for (artifact_id, artifact_type) in artifacts {
            self.outgoing
                .send_server_notification(ServerNotification::GameArtifactCommitted(
                    GameArtifactCommittedNotification {
                        conversation_id: conversation_id.clone(),
                        artifact_id,
                        artifact_type,
                    },
                ))
                .await;
        }
        if let Some(art_bible) = &response.art_bible {
            self.outgoing
                .send_server_notification(ServerNotification::GameArtifactCommitted(
                    GameArtifactCommittedNotification {
                        conversation_id,
                        artifact_id: art_bible.id.clone(),
                        artifact_type: "artBibleVersion".to_string(),
                    },
                ))
                .await;
        }
        Ok(response)
    }

    pub(crate) async fn focus_retry(
        &self,
        connection_id: ConnectionId,
        params: GameFocusRetryParams,
    ) -> std::result::Result<GameFocusRetryResponse, JSONRPCErrorError> {
        let conversation_id = params.conversation_id.clone();
        let execution = self.execution.scoped(connection_id);
        let (response, task) = self
            .adapter
            .focus_retry(&execution, params)
            .await
            .map_err(game_error)?;
        self.notify_workflow_updated(response.workflow.clone())
            .await;
        self.notify_task_started(&conversation_id, task).await;
        Ok(response)
    }

    pub(crate) async fn focus_cancel(
        &self,
        connection_id: ConnectionId,
        params: GameFocusCancelParams,
    ) -> std::result::Result<GameFocusCancelResponse, JSONRPCErrorError> {
        let conversation_id = params.conversation_id.clone();
        let execution = self.execution.scoped(connection_id);
        let (response, cancelled_attempts) = self
            .adapter
            .focus_cancel(&execution, params)
            .await
            .map_err(game_error)?;
        self.notify_workflow_updated(response.workflow.clone())
            .await;
        for cancelled in cancelled_attempts {
            self.outgoing
                .send_server_notification(ServerNotification::GameTaskUpdated(
                    GameTaskUpdatedNotification {
                        conversation_id: conversation_id.clone(),
                        task_id: cancelled.task_id.clone(),
                        status: "cancelled".to_string(),
                    },
                ))
                .await;
            self.outgoing
                .send_server_notification(ServerNotification::GameAttemptUpdated(
                    GameAttemptUpdatedNotification {
                        conversation_id: conversation_id.clone(),
                        task_id: cancelled.task_id,
                        attempt_id: cancelled.attempt_id,
                        turn_id: Some(cancelled.turn_id),
                        status: "cancelled".to_string(),
                    },
                ))
                .await;
        }
        Ok(response)
    }

    pub(crate) async fn task_list(
        &self,
        params: GameTaskListParams,
    ) -> std::result::Result<GameTaskListResponse, JSONRPCErrorError> {
        self.adapter.task_list(params).await.map_err(game_error)
    }

    pub(crate) fn art_bible_list(
        &self,
        params: GameArtBibleListParams,
    ) -> std::result::Result<GameArtBibleListResponse, JSONRPCErrorError> {
        self.adapter.art_bible_list(params).map_err(game_error)
    }

    pub(crate) fn art_bible_read(
        &self,
        params: GameArtBibleReadParams,
    ) -> std::result::Result<GameArtBibleReadResponse, JSONRPCErrorError> {
        self.adapter.art_bible_read(params).map_err(game_error)
    }

    async fn notify_task_started(
        &self,
        conversation_id: &str,
        execution: codex_game_runtime::TaskExecution,
    ) {
        self.outgoing
            .send_server_notification(ServerNotification::GameTaskUpdated(
                GameTaskUpdatedNotification {
                    conversation_id: conversation_id.to_string(),
                    task_id: execution.task.id.as_str().to_string(),
                    status: "running".to_string(),
                },
            ))
            .await;
        self.outgoing
            .send_server_notification(ServerNotification::GameAttemptUpdated(
                GameAttemptUpdatedNotification {
                    conversation_id: conversation_id.to_string(),
                    task_id: execution.task.id.as_str().to_string(),
                    attempt_id: execution.attempt.id.as_str().to_string(),
                    turn_id: execution.attempt.codex_turn_id,
                    status: "running".to_string(),
                },
            ))
            .await;
    }

    async fn notify_workflow_updated(&self, workflow: GameFocusWorkflow) {
        self.outgoing
            .send_server_notification(ServerNotification::GameWorkflowUpdated(
                GameWorkflowUpdatedNotification { workflow },
            ))
            .await;
    }

    async fn notify_recovery_status(&self, status: GameBackendStatus) {
        self.outgoing
            .send_server_notification(ServerNotification::GameRecoveryStatus(
                GameRecoveryStatusNotification { status },
            ))
            .await;
    }
}

fn game_error(error: impl ToString) -> JSONRPCErrorError {
    invalid_params(error.to_string())
}

fn game_project_store_error(error: codex_thread_store::ThreadStoreError) -> JSONRPCErrorError {
    match error {
        codex_thread_store::ThreadStoreError::Unsupported { .. } => {
            method_not_found("game/project/create is unavailable without sqlite state")
        }
        codex_thread_store::ThreadStoreError::InvalidRequest { message } => invalid_params(message),
        error => internal_error(format!("failed to register game project: {error}")),
    }
}
