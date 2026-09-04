use codex_app_server_protocol::GameAttemptUpdatedNotification;
use codex_app_server_protocol::GameCharacterUpdatedNotification;
use codex_app_server_protocol::GameConversationActorNotification;
use codex_app_server_protocol::GameConversationFocusNotification;
use codex_app_server_protocol::GameConversationTurnNotification;
use codex_app_server_protocol::GameGenerationUpdatedNotification;
use codex_app_server_protocol::GameRecoveryStatusNotification;
use codex_app_server_protocol::GameTaskUpdatedNotification;
use codex_app_server_protocol::ProjectChangeType;
use codex_app_server_protocol::ProjectChangedNotification;
use codex_app_server_protocol::ServerNotification;
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

    pub(crate) fn project_inspect(
        &self,
        params: GameProjectInspectParams,
    ) -> std::result::Result<GameProjectInspectResponse, JSONRPCErrorError> {
        self.adapter.project_inspect(params).map_err(game_error)
    }

    pub(crate) async fn project_create(
        &self,
        mut params: GameProjectCreateParams,
    ) -> std::result::Result<GameProjectCreateResponse, JSONRPCErrorError> {
        let root = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path_checked(
            PathBuf::from(&params.root),
        )
        .map_err(|error| invalid_params(format!("invalid project root: {error}")))?
        .to_string_lossy()
        .into_owned();
        params.root = root.clone();
        let display_name = params
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "未命名素材项目".to_string());
        let idempotency_key = format!("game-project:create:{root}");
        if idempotency_key.len() > 512 {
            return Err(invalid_params("project root is too long"));
        }
        let mut metadata = BTreeMap::new();
        metadata.insert("codex.game.kind".to_string(), "asset".to_string());
        let created = self
            .thread_store
            .create_project(StoreCreateProjectParams {
                name: display_name,
                roots: vec![StoredProjectRoot { path: root }],
                metadata,
                thread_ids: Vec::new(),
                idempotency_key,
            })
            .await
            .map_err(game_project_store_error)?;
        let project_id = created.project.id.clone();
        self.notify_recovery_status(GameBackendStatus::Recovering)
            .await;
        let response = self
            .adapter
            .project_create(project_id.clone(), params)
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

    pub(crate) async fn project_commit_art_bible(
        &self,
        params: GameProjectCommitArtBibleParams,
    ) -> std::result::Result<GameProjectCommitArtBibleResponse, JSONRPCErrorError> {
        self.adapter
            .project_commit_art_bible(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn project_finalize(
        &self,
        params: GameProjectFinalizeParams,
    ) -> std::result::Result<GameProjectFinalizeResponse, JSONRPCErrorError> {
        self.adapter
            .project_finalize(params)
            .await
            .map_err(game_error)
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
        if let Some(task_execution) = task_execution {
            self.notify_task_started(&conversation_id, task_execution)
                .await;
        } else {
            self.outgoing
                .send_server_notification(ServerNotification::GameConversationTurn(
                    GameConversationTurnNotification {
                        conversation_id,
                        status: "blocked".to_string(),
                    },
                ))
                .await;
        }
        Ok(response)
    }

    pub(crate) async fn conversation_read(
        &self,
        params: GameConversationReadParams,
    ) -> std::result::Result<GameConversationReadResponse, JSONRPCErrorError> {
        self.adapter
            .conversation_read(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn conversation_interrupt(
        &self,
        connection_id: ConnectionId,
        params: GameConversationInterruptParams,
    ) -> std::result::Result<GameConversationInterruptResponse, JSONRPCErrorError> {
        let conversation_id = params.conversation_id.clone();
        let execution = self.execution.scoped(connection_id);
        let (response, cancelled) = self
            .adapter
            .conversation_interrupt(&execution, params)
            .await
            .map_err(game_error)?;
        for attempt in cancelled {
            self.outgoing
                .send_server_notification(ServerNotification::GameAttemptUpdated(
                    GameAttemptUpdatedNotification {
                        conversation_id: conversation_id.clone(),
                        task_id: attempt.task_id,
                        attempt_id: attempt.attempt_id,
                        turn_id: Some(attempt.turn_id),
                        status: "cancelled".to_string(),
                    },
                ))
                .await;
        }
        Ok(response)
    }

    pub(crate) async fn conversation_commit_drafts(
        &self,
        params: GameConversationCommitDraftsParams,
    ) -> std::result::Result<GameConversationCommitDraftsResponse, JSONRPCErrorError> {
        self.adapter
            .conversation_commit_drafts(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn character_create(
        &self,
        params: GameCharacterCreateParams,
    ) -> std::result::Result<GameCharacterCreateResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_create(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn character_list(
        &self,
        params: GameCharacterListParams,
    ) -> std::result::Result<GameCharacterListResponse, JSONRPCErrorError> {
        self.adapter
            .character_list(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn character_read(
        &self,
        params: GameCharacterReadParams,
    ) -> std::result::Result<GameCharacterReadResponse, JSONRPCErrorError> {
        self.adapter
            .character_read(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn character_confirm_spec(
        &self,
        params: GameCharacterConfirmSpecParams,
    ) -> std::result::Result<GameCharacterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_confirm_spec(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn character_reject_spec(
        &self,
        params: GameCharacterRejectSpecParams,
    ) -> std::result::Result<GameCharacterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_reject_spec(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn character_confirm_render(
        &self,
        params: GameCharacterConfirmRenderParams,
    ) -> std::result::Result<GameCharacterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_confirm_render(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn character_reject_render(
        &self,
        params: GameCharacterRejectRenderParams,
    ) -> std::result::Result<GameCharacterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_reject_render(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn character_confirm_views(
        &self,
        params: GameCharacterConfirmViewsParams,
    ) -> std::result::Result<GameCharacterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_confirm_views(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn character_reject_views(
        &self,
        params: GameCharacterRejectViewsParams,
    ) -> std::result::Result<GameCharacterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .character_reject_views(params)
            .await
            .map_err(game_error)?;
        self.notify_character_updated(response.character.clone())
            .await;
        Ok(response)
    }

    pub(crate) async fn generation_register(
        &self,
        params: GameGenerationRegisterParams,
    ) -> std::result::Result<GameGenerationRegisterResponse, JSONRPCErrorError> {
        let response = self
            .adapter
            .generation_register(params)
            .await
            .map_err(game_error)?;
        self.outgoing
            .send_server_notification(ServerNotification::GameGenerationUpdated(
                GameGenerationUpdatedNotification {
                    generation: response.generation.clone(),
                },
            ))
            .await;
        Ok(response)
    }

    pub(crate) async fn generation_list(
        &self,
        params: GameGenerationListParams,
    ) -> std::result::Result<GameGenerationListResponse, JSONRPCErrorError> {
        self.adapter
            .generation_list(params)
            .await
            .map_err(game_error)
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

    pub(crate) async fn ai_provider_list(
        &self,
        params: GameAiProviderListParams,
    ) -> std::result::Result<GameAiProviderListResponse, JSONRPCErrorError> {
        self.adapter
            .ai_provider_list(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn ai_provider_create(
        &self,
        params: GameAiProviderCreateParams,
    ) -> std::result::Result<GameAiProviderCreateResponse, JSONRPCErrorError> {
        self.adapter
            .ai_provider_create(params)
            .await
            .map(|provider| GameAiProviderCreateResponse { provider })
            .map_err(game_error)
    }

    pub(crate) async fn ai_provider_update(
        &self,
        params: GameAiProviderWriteParams,
    ) -> std::result::Result<GameAiProviderUpdateResponse, JSONRPCErrorError> {
        self.adapter
            .ai_provider_update(params)
            .await
            .map(|provider| GameAiProviderUpdateResponse { provider })
            .map_err(game_error)
    }

    pub(crate) async fn ai_provider_delete(
        &self,
        params: GameAiProviderDeleteParams,
    ) -> std::result::Result<GameAiProviderDeleteResponse, JSONRPCErrorError> {
        self.adapter
            .ai_provider_delete(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn ai_model_create(
        &self,
        params: GameAiModelWriteParams,
    ) -> std::result::Result<GameAiModelCreateResponse, JSONRPCErrorError> {
        self.adapter
            .ai_model_write(params)
            .await
            .map(|model| GameAiModelCreateResponse { model })
            .map_err(game_error)
    }

    pub(crate) async fn ai_model_update(
        &self,
        params: GameAiModelWriteParams,
    ) -> std::result::Result<GameAiModelUpdateResponse, JSONRPCErrorError> {
        self.adapter
            .ai_model_write(params)
            .await
            .map(|model| GameAiModelUpdateResponse { model })
            .map_err(game_error)
    }

    pub(crate) async fn ai_model_delete(
        &self,
        params: GameAiModelDeleteParams,
    ) -> std::result::Result<GameAiModelDeleteResponse, JSONRPCErrorError> {
        self.adapter
            .ai_model_delete(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn ai_agent_list(
        &self,
        params: GameAiAgentListParams,
    ) -> std::result::Result<GameAiAgentListResponse, JSONRPCErrorError> {
        self.adapter.ai_agent_list(params).await.map_err(game_error)
    }

    pub(crate) async fn ai_agent_binding_write(
        &self,
        params: GameAiAgentBindingWriteParams,
    ) -> std::result::Result<GameAiAgentBindingWriteResponse, JSONRPCErrorError> {
        self.adapter
            .ai_agent_binding_write(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn ai_usage_read(
        &self,
        params: GameAiUsageReadParams,
    ) -> std::result::Result<GameAiUsageReadResponse, JSONRPCErrorError> {
        self.adapter.ai_usage_read(params).await.map_err(game_error)
    }

    pub(crate) async fn ai_usage_reset(
        &self,
        params: GameAiUsageResetParams,
    ) -> std::result::Result<GameAiUsageResetResponse, JSONRPCErrorError> {
        self.adapter
            .ai_usage_reset(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn ai_breaker_clear(
        &self,
        params: GameAiBreakerClearParams,
    ) -> std::result::Result<GameAiBreakerClearResponse, JSONRPCErrorError> {
        self.adapter
            .ai_breaker_clear(params)
            .await
            .map_err(game_error)
    }

    pub(crate) fn provider_preset_list(
        &self,
        params: GameProviderPresetListParams,
    ) -> std::result::Result<GameProviderPresetListResponse, JSONRPCErrorError> {
        self.adapter
            .provider_preset_list(params)
            .map_err(game_error)
    }

    pub(crate) async fn ai_config_export(
        &self,
        params: GameAiConfigExportParams,
    ) -> std::result::Result<GameAiConfigExportResponse, JSONRPCErrorError> {
        self.adapter
            .ai_config_export(params)
            .await
            .map_err(game_error)
    }

    pub(crate) async fn ai_config_import(
        &self,
        params: GameAiConfigImportParams,
    ) -> std::result::Result<GameAiConfigImportResponse, JSONRPCErrorError> {
        self.adapter
            .ai_config_import(params)
            .await
            .map_err(game_error)
    }

    async fn notify_task_started(
        &self,
        conversation_id: &str,
        execution: codex_game_runtime::TaskExecution,
    ) {
        let task_id = execution.task.id.as_str().to_string();
        let agent_code = execution.task.agent_code;
        let turn_id = execution.attempt.codex_turn_id;
        self.outgoing
            .send_server_notification(ServerNotification::GameTaskUpdated(
                GameTaskUpdatedNotification {
                    conversation_id: conversation_id.to_string(),
                    task_id: task_id.clone(),
                    status: "running".to_string(),
                },
            ))
            .await;
        self.outgoing
            .send_server_notification(ServerNotification::GameAttemptUpdated(
                GameAttemptUpdatedNotification {
                    conversation_id: conversation_id.to_string(),
                    task_id,
                    attempt_id: execution.attempt.id.as_str().to_string(),
                    turn_id: turn_id.clone(),
                    status: "running".to_string(),
                },
            ))
            .await;
        self.outgoing
            .send_server_notification(ServerNotification::GameConversationTurn(
                GameConversationTurnNotification {
                    conversation_id: conversation_id.to_string(),
                    status: "running".to_string(),
                },
            ))
            .await;
        self.outgoing
            .send_server_notification(ServerNotification::GameConversationActor(
                GameConversationActorNotification {
                    conversation_id: conversation_id.to_string(),
                    turn_id,
                    agent_code: agent_code.clone(),
                    status: "working".to_string(),
                },
            ))
            .await;
        self.outgoing
            .send_server_notification(ServerNotification::GameConversationFocus(
                GameConversationFocusNotification {
                    conversation_id: conversation_id.to_string(),
                    agent_code,
                },
            ))
            .await;
    }

    async fn notify_character_updated(&self, character: GameCharacter) {
        self.outgoing
            .send_server_notification(ServerNotification::GameCharacterUpdated(
                GameCharacterUpdatedNotification { character },
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
