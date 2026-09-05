use codex_game_app_server_protocol::*;
use codex_game_domain::AgentActionKind;
use codex_game_domain::AgentCapability;
use codex_game_domain::AgentHandoff;
use codex_game_domain::ArtBibleVersion;
use codex_game_domain::ArtifactDraftRecord;
use codex_game_domain::Character;
use codex_game_domain::CharacterState;
use codex_game_domain::Conversation;
use codex_game_domain::ConversationMemory;
use codex_game_domain::ConversationMessage;
use codex_game_domain::ConversationStatus;
use codex_game_domain::ConversationTargetKind;
use codex_game_domain::Generation;
use codex_game_domain::MessageStatus;
use codex_game_domain::Project;
use codex_game_domain::ProjectState;
use codex_game_domain::TaskAttemptStatus;
use codex_game_domain::internal_executors_for_stage;
use codex_game_runtime::Capability;
use codex_game_runtime::CodexExecutionPort;
use codex_game_runtime::ExecuteTaskRequest;
use codex_game_runtime::GAME_PROTOCOL_VERSION;
use codex_game_runtime::GameRuntime;
use codex_game_runtime::GameServiceError;
use codex_game_runtime::MAX_ACTION_CONTRACT_RETRIES;
use codex_game_runtime::PreparedConversationTurn;
use codex_game_runtime::RouteCandidate;
use codex_game_runtime::RouteFailureKind;
use codex_game_runtime::RouteOutcome;
use codex_game_runtime::TaskExecution;
use codex_game_runtime::TurnOutputCompletion;
use codex_game_runtime::bundled_agent_definitions;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

mod ai_config;

pub use ai_config::RealtimeSpeechRoute;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTurnProjection {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub turn_id: Option<String>,
    pub status: String,
    pub agent_code: Option<String>,
    pub handoff_target: Option<String>,
    pub handoff_reason: Option<String>,
    pub director_resume_reason: Option<String>,
    pub character: Option<GameCharacter>,
    pub generations: Vec<GameGeneration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTurnEventContext {
    pub conversation_id: String,
    pub agent_code: String,
}

/// Adapts game runtime operations to app-server protocol responses.
#[derive(Debug, Default)]
pub struct GameAppServerAdapter {
    runtime: GameRuntime,
    studio_storage: PathBuf,
}

impl GameAppServerAdapter {
    pub fn new(studio_storage: PathBuf) -> Self {
        Self {
            runtime: GameRuntime::new(studio_storage.clone()),
            studio_storage,
        }
    }

    pub fn new_with_routes(studio_storage: PathBuf, candidates: Vec<RouteCandidate>) -> Self {
        Self {
            runtime: GameRuntime::new_with_routes(studio_storage.clone(), candidates),
            studio_storage,
        }
    }

    pub fn ping(&self) -> GamePingResponse {
        GamePingResponse {
            protocol_version: GAME_PROTOCOL_VERSION,
            backend_version: env!("CARGO_PKG_VERSION").to_string(),
            status: self.runtime.status().into(),
        }
    }

    pub async fn turn_event_context(
        &self,
        turn_id: &str,
    ) -> Result<Option<GameTurnEventContext>, GameServiceError> {
        self.runtime
            .service()
            .turn_attempt_context(turn_id)
            .await
            .map(|context| {
                context.map(|context| GameTurnEventContext {
                    conversation_id: context.conversation_id,
                    agent_code: context.agent_code,
                })
            })
    }

    pub async fn turn_audit_context(
        &self,
        turn_id: &str,
    ) -> Result<Option<codex_game_runtime::TurnAuditContext>, GameServiceError> {
        self.runtime.service().turn_audit_context(turn_id).await
    }

    pub async fn observe_turn_completed<E: CodexExecutionPort>(
        &self,
        execution: &E,
        turn_id: &str,
        output: Option<&str>,
        terminal_error: Option<&str>,
    ) -> Result<Option<GameTurnProjection>, GameServiceError> {
        let failed = terminal_error.is_some();
        let completion = if let Some(error) = terminal_error {
            if is_output_length_error(error)
                && let Some(context) = self.runtime.service().turn_attempt_context(turn_id).await?
                && let Some(audit_context) =
                    self.runtime.service().turn_audit_context(turn_id).await?
            {
                let (_, store) = self
                    .runtime
                    .service()
                    .execution_context(&context.conversation_id)?;
                if let Ok(Some(retry)) = self
                    .runtime
                    .orchestrator()
                    .retry_output_length(execution, store.as_ref(), &context, audit_context)
                    .await
                {
                    return Ok(Some(retry_projection(retry)));
                }
            }
            self.runtime
                .service()
                .complete_turn(turn_id, TaskAttemptStatus::Failed)
                .await?
        } else {
            match self
                .runtime
                .service()
                .complete_turn_output(turn_id, output)
                .await?
            {
                TurnOutputCompletion::Completed(completion) => completion,
                TurnOutputCompletion::ActionProtocolViolation(violation) => {
                    let retry_error = if violation.context.attempt_no <= MAX_ACTION_CONTRACT_RETRIES
                    {
                        match self.runtime.service().turn_audit_context(turn_id).await? {
                            Some(audit_context) => self
                                .runtime
                                .orchestrator()
                                .retry_action_contract(
                                    execution,
                                    violation.store.as_ref(),
                                    &violation.context,
                                    audit_context,
                                    &violation.message,
                                )
                                .await
                                .map_err(|error| error.to_string()),
                            None => Err("无法构建 Action 契约重试审计上下文".to_string()),
                        }
                    } else {
                        Err(String::new())
                    };
                    match retry_error {
                        Ok(retry) => return Ok(Some(retry_projection(retry))),
                        Err(error) => {
                            self.runtime
                                .service()
                                .fail_action_protocol_violation(
                                    violation,
                                    (!error.is_empty()).then_some(error.as_str()),
                                )
                                .await?
                        }
                    }
                }
            }
        };
        if failed && let Some(completion) = &completion {
            self.runtime
                .orchestrator()
                .report_route_outcome(
                    &completion.conversation_id,
                    RouteOutcome::Failed(RouteFailureKind::Retryable),
                )
                .await
                .map_err(|_| GameServiceError::StateUnavailable)?;
        } else if let Some(completion) = &completion {
            self.runtime
                .orchestrator()
                .report_route_outcome(&completion.conversation_id, RouteOutcome::Succeeded)
                .await
                .map_err(|_| GameServiceError::StateUnavailable)?;
        }
        let Some(completion) = completion else {
            return Ok(None);
        };
        let mut projection = turn_projection(completion);
        if projection.status == "succeeded" {
            let snapshot = self
                .runtime
                .service()
                .read_conversation(&projection.conversation_id)
                .await?;
            if snapshot.conversation.target_kind == ConversationTargetKind::Character
                && let Some(character_id) = snapshot.conversation.target_ref
            {
                let project_id = snapshot.conversation.project_id.as_str();
                projection.character = Some(character_dto(
                    self.runtime
                        .service()
                        .read_character(project_id, &character_id)
                        .await?,
                ));
                projection.generations = self
                    .runtime
                    .service()
                    .list_generations(project_id, &character_id, None)
                    .await?
                    .into_iter()
                    .filter(|generation| generation.task_id.as_deref() == Some(&projection.task_id))
                    .map(generation_dto)
                    .collect();
            }
        }
        Ok(Some(projection))
    }

    pub async fn observe_turn_aborted(
        &self,
        turn_id: &str,
    ) -> Result<Option<GameTurnProjection>, GameServiceError> {
        self.runtime
            .service()
            .complete_turn(turn_id, TaskAttemptStatus::Cancelled)
            .await
            .map(|completion| completion.map(turn_projection))
    }

    pub fn project_inspect(
        &self,
        params: GameProjectInspectParams,
    ) -> Result<GameProjectInspectResponse, GameServiceError> {
        self.runtime
            .service()
            .inspect_project_dir(&params.root)
            .map(|state| GameProjectInspectResponse {
                root: state.root,
                occupied: state.occupied,
                project_id: state.project_id,
                supported: state.supported,
            })
    }

    pub async fn project_create(
        &self,
        project_id: String,
        params: GameProjectCreateParams,
    ) -> Result<GameProjectCreateResponse, GameServiceError> {
        let state = self.runtime.service().inspect_project_dir(&params.root)?;
        if state.occupied {
            return Err(GameServiceError::InvalidProjectPath(
                "目录已存在 project.json，请直接打开该项目".to_string(),
            ));
        }
        clear_stale_project_state(Path::new(&state.root))?;
        self.runtime
            .create_project(
                project_id,
                params.name.unwrap_or_else(|| "未命名素材项目".to_string()),
                state.root,
            )
            .await
            .map(|project| GameProjectCreateResponse {
                project: project_dto(project),
            })
    }

    pub async fn project_open(
        &self,
        params: GameProjectOpenParams,
    ) -> Result<GameProjectOpenResponse, GameServiceError> {
        self.runtime
            .open_project(params.root, params.read_only.unwrap_or(false))
            .await
            .map(|project| GameProjectOpenResponse {
                project: project_dto(project),
            })
    }

    pub fn project_read(
        &self,
        params: GameProjectReadParams,
    ) -> Result<GameProjectReadResponse, GameServiceError> {
        self.runtime
            .service()
            .read_project(&params.project_id)
            .map(|project| GameProjectReadResponse {
                project: project_dto(project),
            })
    }

    pub async fn project_list(
        &self,
        _params: GameProjectListParams,
    ) -> Result<GameProjectListResponse, GameServiceError> {
        self.runtime
            .service()
            .list_projects()
            .await
            .map(|projects| GameProjectListResponse {
                projects: projects.into_iter().map(project_dto).collect(),
            })
    }

    pub async fn project_delete(
        &self,
        params: GameProjectDeleteParams,
    ) -> Result<GameProjectDeleteResponse, GameServiceError> {
        self.runtime
            .service()
            .remove_project(&params.project_id)
            .await?;
        Ok(GameProjectDeleteResponse {})
    }

    pub async fn project_commit_art_bible(
        &self,
        params: GameProjectCommitArtBibleParams,
    ) -> Result<GameProjectCommitArtBibleResponse, GameServiceError> {
        self.runtime
            .service()
            .commit_art_bible_draft(&params.conversation_id, &params.draft_id)
            .await
            .map(|document| GameProjectCommitArtBibleResponse {
                version: art_bible_dto(document.version),
                markdown: document.markdown,
            })
    }

    pub async fn project_finalize(
        &self,
        params: GameProjectFinalizeParams,
    ) -> Result<GameProjectFinalizeResponse, GameServiceError> {
        self.runtime
            .service()
            .finalize_project(&params.project_id, params.name, params.code)
            .await
            .map(|project| GameProjectFinalizeResponse {
                project: project_dto(project),
            })
    }

    pub async fn conversation_ensure(
        &self,
        params: GameConversationEnsureParams,
    ) -> Result<GameConversationEnsureResponse, GameServiceError> {
        let target_kind = parse_target_kind(&params.target_kind)?;
        let title = params.title.unwrap_or_else(|| match target_kind {
            ConversationTargetKind::Project => "项目美术基调".to_string(),
            ConversationTargetKind::Character => "角色素材".to_string(),
        });
        self.runtime
            .service()
            .ensure_conversation(
                &params.project_id,
                target_kind,
                params.target_ref,
                title,
                params
                    .director_agent_code
                    .unwrap_or_else(|| "studio_director".to_string()),
            )
            .await
            .map(|conversation| GameConversationEnsureResponse {
                conversation: conversation_dto(conversation),
            })
    }

    pub async fn conversation_submit<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameConversationSubmitParams,
    ) -> Result<(GameConversationSubmitResponse, Option<TaskExecution>), String> {
        let conversation_id = params.conversation_id;
        let prepared = self
            .runtime
            .service()
            .prepare_conversation_turn(
                &conversation_id,
                params.content,
                params.recipient_agent_code,
            )
            .await
            .map_err(|error| error.to_string())?;
        let task = self.execute_prepared(execution, &prepared).await?;
        let snapshot = self
            .conversation_snapshot(&conversation_id)
            .await
            .map_err(|error| error.to_string())?;
        let response = GameConversationSubmitResponse {
            conversation: snapshot.conversation,
            messages: snapshot.messages,
            drafts: snapshot.drafts,
            memories: snapshot.memories,
            handoffs: snapshot.handoffs,
        };
        Ok((response, task))
    }

    pub async fn continue_handoff<E: CodexExecutionPort>(
        &self,
        execution: &E,
        conversation_id: &str,
        target_agent: &str,
    ) -> Result<Option<TaskExecution>, String> {
        let prepared = self
            .runtime
            .service()
            .prepare_handoff_turn(conversation_id, target_agent)
            .await
            .map_err(|error| error.to_string())?;
        self.execute_prepared(execution, &prepared).await
    }

    async fn execute_prepared<E: CodexExecutionPort>(
        &self,
        execution: &E,
        prepared: &PreparedConversationTurn,
    ) -> Result<Option<TaskExecution>, String> {
        let context = match self
            .runtime
            .service()
            .build_conversation_context(prepared)
            .await
        {
            Ok(context) => context,
            Err(error) => {
                self.runtime
                    .service()
                    .complete_prepared_blocked(
                        prepared,
                        format!("{} 无法构建执行上下文：{error}", prepared.agent_code),
                    )
                    .await
                    .map_err(|persist_error| persist_error.to_string())?;
                return Ok(None);
            }
        };
        let capability = match capability_for_agent(&prepared.agent_code) {
            Ok(capability) => capability,
            Err(error) => {
                self.runtime
                    .service()
                    .complete_prepared_blocked(prepared, error)
                    .await
                    .map_err(|persist_error| persist_error.to_string())?;
                return Ok(None);
            }
        };
        let internal_executor = (prepared.agent_code == "visual_designer")
            .then(|| {
                internal_executors_for_stage(
                    prepared.conversation.target_kind.as_str(),
                    &prepared.stage,
                )
                .first()
                .copied()
            })
            .flatten();
        let internal_executor_capability = match internal_executor {
            Some("image_t2i") => Some(Capability::ImageTextToImage),
            Some("image_i2i") => Some(Capability::ImageImageToImage),
            Some(_) | None => None,
        };
        let (audit_target, audit_target_dir) = match prepared.conversation.target_kind {
            ConversationTargetKind::Project => {
                ("project".to_string(), PathBuf::from(&prepared.project.root))
            }
            ConversationTargetKind::Character => {
                let character_id = prepared
                    .conversation
                    .target_ref
                    .as_deref()
                    .ok_or_else(|| "角色会话缺少 targetRef".to_string())?;
                let character = prepared
                    .store
                    .read_character(prepared.project.id.as_str(), character_id)
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("character not found: {character_id}"))?;
                (
                    format!("character:{}", character.id),
                    Path::new(&prepared.project.root).join(character.dir_name),
                )
            }
        };
        match self
            .runtime
            .orchestrator()
            .execute(
                execution,
                prepared.store.as_ref(),
                ExecuteTaskRequest {
                    project_root: prepared.project.root.clone(),
                    conversation_id: prepared.conversation.id.as_str().to_string(),
                    conversation_turn: prepared.conversation.turn,
                    target_id: prepared
                        .conversation
                        .target_ref
                        .clone()
                        .unwrap_or_else(|| prepared.project.id.as_str().to_string()),
                    audit_target,
                    audit_target_dir,
                    stage: prepared.stage.clone(),
                    agent_code: prepared.agent_code.clone(),
                    idempotency_key: prepared.assistant_message.id.clone(),
                    prompt: prepared.user_message.content.clone(),
                    context,
                    capability,
                    internal_executor_code: internal_executor.map(str::to_string),
                    internal_executor_capability,
                },
            )
            .await
        {
            Ok(execution) => Ok(Some(execution)),
            Err(error) => {
                let reason = format!(
                    "{} 当前无法执行：{}。请配置兼容且可用的 Provider 模型后重试。",
                    prepared.agent_code, error
                );
                self.runtime
                    .service()
                    .complete_prepared_blocked(prepared, reason)
                    .await
                    .map_err(|persist_error| persist_error.to_string())?;
                Ok(None)
            }
        }
    }

    pub async fn conversation_read(
        &self,
        params: GameConversationReadParams,
    ) -> Result<GameConversationReadResponse, GameServiceError> {
        self.conversation_snapshot(&params.conversation_id).await
    }

    async fn conversation_snapshot(
        &self,
        conversation_id: &str,
    ) -> Result<GameConversationReadResponse, GameServiceError> {
        self.runtime
            .service()
            .read_conversation(conversation_id)
            .await
            .map(conversation_snapshot_dto)
    }

    pub async fn conversation_interrupt<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameConversationInterruptParams,
    ) -> Result<GameConversationInterruptResponse, String> {
        let (_, store) = self
            .runtime
            .service()
            .execution_context(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        let running = store
            .running_attempts(&params.conversation_id)
            .await
            .map_err(|error| error.to_string())?;
        for attempt in running {
            self.runtime
                .orchestrator()
                .interrupt(
                    execution,
                    &params.conversation_id,
                    &attempt.agent_code,
                    attempt.thread_id,
                    attempt.turn_id,
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(GameConversationInterruptResponse {})
    }

    pub async fn resume_director<E: CodexExecutionPort>(
        &self,
        execution: &E,
        conversation_id: &str,
        reason: String,
    ) -> Result<Option<TaskExecution>, String> {
        let prepared = self
            .runtime
            .service()
            .prepare_director_resume_turn_if_idle(conversation_id, reason)
            .await
            .map_err(|error| error.to_string())?;
        match prepared {
            Some(prepared) => self.execute_prepared(execution, &prepared).await,
            None => Ok(None),
        }
    }

    pub async fn conversation_commit_drafts<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameConversationCommitDraftsParams,
    ) -> Result<(GameConversationCommitDraftsResponse, Option<TaskExecution>), String> {
        let target_paths = self
            .runtime
            .service()
            .commit_conversation_drafts(&params.conversation_id, &params.draft_ids)
            .await
            .map_err(|error| error.to_string())?;
        let prepared = self
            .runtime
            .service()
            .prepare_director_resume_turn(
                &params.conversation_id,
                format!(
                    "用户已确认并提交本轮草稿：{}。请根据最新流程状态决定下一步。",
                    target_paths.join("、")
                ),
            )
            .await
            .map_err(|error| error.to_string())?;
        let task = self.execute_prepared(execution, &prepared).await?;
        Ok((GameConversationCommitDraftsResponse {}, task))
    }

    pub async fn character_create(
        &self,
        params: GameCharacterCreateParams,
    ) -> Result<GameCharacterCreateResponse, GameServiceError> {
        self.runtime
            .service()
            .create_character(
                &params.project_id,
                params.name,
                params.group,
                params.overwrite,
            )
            .await
            .map(|character| GameCharacterCreateResponse {
                character: character_dto(character),
            })
    }

    pub async fn character_group_create(
        &self,
        params: GameCharacterGroupCreateParams,
    ) -> Result<GameCharacterGroupCreateResponse, GameServiceError> {
        self.runtime
            .service()
            .create_character_group(&params.project_id, params.name)
            .await
            .map(|group| GameCharacterGroupCreateResponse { group })
    }

    pub async fn character_list(
        &self,
        params: GameCharacterListParams,
    ) -> Result<GameCharacterListResponse, GameServiceError> {
        let characters = self
            .runtime
            .service()
            .list_characters(&params.project_id)
            .await?;
        let mut groups = self
            .runtime
            .service()
            .list_character_groups(&params.project_id)
            .await?;
        groups.extend(
            characters
                .iter()
                .filter_map(|listed| listed.character.group.clone()),
        );
        groups.sort();
        groups.dedup();
        Ok(GameCharacterListResponse {
            characters: characters
                .into_iter()
                .map(|listed| GameListedCharacter {
                    character: character_dto(listed.character),
                    model_file_exists: listed.model_file_exists,
                })
                .collect(),
            groups,
        })
    }

    pub async fn character_delete(
        &self,
        params: GameCharacterDeleteParams,
    ) -> Result<GameCharacterDeleteResponse, GameServiceError> {
        self.runtime
            .service()
            .remove_character(&params.project_id, &params.character_id)
            .await?;
        Ok(GameCharacterDeleteResponse {})
    }

    pub async fn character_read(
        &self,
        params: GameCharacterReadParams,
    ) -> Result<GameCharacterReadResponse, GameServiceError> {
        let character = self
            .runtime
            .service()
            .read_character(&params.project_id, &params.character_id)
            .await?;
        let generations = self
            .runtime
            .service()
            .list_generations(&params.project_id, &params.character_id, None)
            .await?;
        let workflow_progress = self
            .runtime
            .service()
            .character_workflow_progress(&params.project_id, &params.character_id)
            .await?;
        Ok(GameCharacterReadResponse {
            character: character_dto(character),
            generations: generations.into_iter().map(generation_dto).collect(),
            workflow_progress: GameCharacterWorkflowProgress {
                status_label: workflow_progress.status_label,
                steps: workflow_progress
                    .steps
                    .into_iter()
                    .map(|step| GameCharacterWorkflowStep {
                        key: step.key,
                        label: step.label,
                        status: step.status,
                    })
                    .collect(),
                needs_resume: workflow_progress.needs_resume,
                continuation_key: workflow_progress.continuation_key,
            },
        })
    }

    async fn character_response_with_resume<E: CodexExecutionPort>(
        &self,
        execution: &E,
        character: Character,
        reason: String,
        resume: bool,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let task = if resume {
            let prepared = self
                .runtime
                .service()
                .prepare_character_director_resume_turn_if_idle(
                    &character.project_id,
                    &character.id,
                    reason,
                )
                .await
                .map_err(|error| error.to_string())?;
            match prepared {
                Some(prepared) => self.execute_prepared(execution, &prepared).await?,
                None => None,
            }
        } else {
            None
        };
        Ok((
            GameCharacterResponse {
                character: character_dto(character),
                execution_started: task.is_some(),
            },
            task,
        ))
    }

    pub async fn character_confirm_spec<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterConfirmSpecParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let character = self
            .runtime
            .service()
            .confirm_character_spec(&params.project_id, &params.character_id, &params.draft_id)
            .await
            .map_err(|error| error.to_string())?;
        self.character_response_with_resume(
            execution,
            character,
            "用户已确认角色设定，请根据最新工作流继续生成效果图。".to_string(),
            true,
        )
        .await
    }

    pub async fn generation_register(
        &self,
        params: GameGenerationRegisterParams,
    ) -> Result<GameGenerationRegisterResponse, GameServiceError> {
        self.runtime
            .service()
            .register_generation(
                &params.project_id,
                &params.character_id,
                &params.stage,
                params.variant,
                params.file_path,
                params.source,
                params.asset_spec,
            )
            .await
            .map(|generation| GameGenerationRegisterResponse {
                generation: generation_dto(generation),
            })
    }

    pub async fn generation_list(
        &self,
        params: GameGenerationListParams,
    ) -> Result<GameGenerationListResponse, GameServiceError> {
        self.runtime
            .service()
            .list_generations(
                &params.project_id,
                &params.character_id,
                params.stage.as_deref(),
            )
            .await
            .map(|generations| GameGenerationListResponse {
                generations: generations.into_iter().map(generation_dto).collect(),
            })
    }

    pub async fn character_reject_spec<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterRejectSpecParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let reason = params.reason;
        let character = self
            .runtime
            .service()
            .reject_character_stage(
                &params.project_id,
                &params.character_id,
                "spec",
                reason.clone(),
            )
            .await
            .map_err(|error| error.to_string())?;
        self.character_response_with_resume(
            execution,
            character,
            format!("用户补充了角色设定修改要求：{reason}。请根据最新工作流继续处理。"),
            true,
        )
        .await
    }

    pub async fn character_confirm_render<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterConfirmRenderParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let character = self
            .runtime
            .service()
            .confirm_character_render(
                &params.project_id,
                &params.character_id,
                &params.generation_id,
            )
            .await
            .map_err(|error| error.to_string())?;
        self.character_response_with_resume(
            execution,
            character,
            "用户已确认角色效果图，请根据最新工作流继续生成四视图。".to_string(),
            true,
        )
        .await
    }

    pub async fn character_reject_render<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterRejectRenderParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let reason = params.reason;
        let character = self
            .runtime
            .service()
            .reject_character_stage(
                &params.project_id,
                &params.character_id,
                "render",
                reason.clone(),
            )
            .await
            .map_err(|error| error.to_string())?;
        self.character_response_with_resume(
            execution,
            character,
            format!("用户补充了效果图修改要求：{reason}。请根据最新工作流继续处理。"),
            true,
        )
        .await
    }

    pub async fn character_confirm_views<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterConfirmViewsParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let character = self
            .runtime
            .service()
            .confirm_character_views(
                &params.project_id,
                &params.character_id,
                &params.generation_ids,
            )
            .await
            .map_err(|error| error.to_string())?;
        self.character_response_with_resume(execution, character, String::new(), false)
            .await
    }

    pub async fn character_reject_views<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterRejectViewsParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let reason = params.reason;
        let character = self
            .runtime
            .service()
            .reject_character_stage(
                &params.project_id,
                &params.character_id,
                "views",
                reason.clone(),
            )
            .await
            .map_err(|error| error.to_string())?;
        self.character_response_with_resume(
            execution,
            character,
            format!("用户补充了四视图修改要求：{reason}。请根据最新工作流继续处理。"),
            true,
        )
        .await
    }

    pub async fn character_resume<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameCharacterResumeParams,
    ) -> Result<(GameCharacterResponse, Option<TaskExecution>), String> {
        let prepared = self
            .runtime
            .service()
            .prepare_character_resume_if_needed(
                &params.project_id,
                &params.character_id,
                &params.continuation_key,
            )
            .await
            .map_err(|error| error.to_string())?;
        let task = match prepared {
            Some(prepared) => self.execute_prepared(execution, &prepared).await?,
            None => None,
        };
        let character = self
            .runtime
            .service()
            .read_character(&params.project_id, &params.character_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok((
            GameCharacterResponse {
                character: character_dto(character),
                execution_started: task.is_some(),
            },
            task,
        ))
    }

    pub async fn task_list(
        &self,
        params: GameTaskListParams,
    ) -> Result<GameTaskListResponse, String> {
        let (_, store) = self
            .runtime
            .service()
            .execution_context(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        store
            .list_tasks(&params.conversation_id)
            .await
            .map(|tasks| GameTaskListResponse {
                tasks: tasks
                    .into_iter()
                    .map(|task| GameTask {
                        id: task.id.as_str().to_string(),
                        stage: task.stage,
                        agent_code: task.agent_code,
                        status: format!("{:?}", task.status).to_lowercase(),
                    })
                    .collect(),
            })
            .map_err(|error| error.to_string())
    }

    pub fn art_bible_list(
        &self,
        params: GameArtBibleListParams,
    ) -> Result<GameArtBibleListResponse, GameServiceError> {
        self.runtime
            .service()
            .list_art_bibles(&params.project_id)
            .map(|versions| GameArtBibleListResponse {
                versions: versions.into_iter().map(art_bible_dto).collect(),
            })
    }

    pub fn art_bible_read(
        &self,
        params: GameArtBibleReadParams,
    ) -> Result<GameArtBibleReadResponse, GameServiceError> {
        self.runtime
            .service()
            .read_art_bible(&params.project_id, params.version)
            .map(|document| GameArtBibleReadResponse {
                version: art_bible_dto(document.version),
                markdown: document.markdown,
            })
    }
}

fn is_output_length_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("incomplete response returned, reason: length")
        || (normalized.contains("finish_reason") && normalized.contains("length"))
}

fn retry_projection(retry: codex_game_runtime::TaskAttemptRetry) -> GameTurnProjection {
    GameTurnProjection {
        attempt_id: retry.attempt_id,
        task_id: retry.task_id,
        conversation_id: retry.conversation_id,
        turn_id: Some(retry.turn_id),
        status: "running".to_string(),
        agent_code: Some(retry.agent_code),
        handoff_target: None,
        handoff_reason: None,
        director_resume_reason: None,
        character: None,
        generations: Vec::new(),
    }
}

fn turn_projection(completion: codex_game_runtime::CompletedTaskAttempt) -> GameTurnProjection {
    let (handoff_target, handoff_reason) = completion
        .action
        .as_ref()
        .filter(|action| action.action == AgentActionKind::Handoff)
        .map(|action| (action.target_agent.clone(), Some(action.reason.clone())))
        .unwrap_or((None, None));
    let director_resume_reason = (completion.agent_code.as_deref() == Some("spec_writer"))
        .then_some(completion.action.as_ref())
        .flatten()
        .filter(|action| action.action == AgentActionKind::AskUser)
        .and_then(|action| action.payload.drafts.as_ref())
        .is_some_and(|drafts| {
            drafts
                .iter()
                .any(|draft| draft.target_path == "docs/角色定稿.md")
        })
        .then(|| "角色设计师已提交待审角色设定草稿，请根据当前工作流继续派单。".to_string());
    GameTurnProjection {
        attempt_id: completion.attempt_id,
        task_id: completion.task_id,
        conversation_id: completion.conversation_id,
        turn_id: None,
        status: format!("{:?}", completion.status).to_lowercase(),
        agent_code: completion.agent_code,
        handoff_target,
        handoff_reason,
        director_resume_reason,
        character: None,
        generations: Vec::new(),
    }
}

fn parse_target_kind(value: &str) -> Result<ConversationTargetKind, GameServiceError> {
    match value {
        "project" => Ok(ConversationTargetKind::Project),
        "character" => Ok(ConversationTargetKind::Character),
        _ => Err(GameServiceError::InvalidAction(format!(
            "未知会话目标类型：{value}"
        ))),
    }
}

fn capability_for_agent(agent_code: &str) -> Result<Capability, String> {
    let definition = bundled_agent_definitions()?
        .into_iter()
        .find(|definition| definition.agent_code == agent_code)
        .ok_or_else(|| format!("未知 Agent：{agent_code}"))?;
    Ok(match definition.capability {
        AgentCapability::Text => Capability::TextStructuredOutput,
        AgentCapability::T2i => Capability::ImageTextToImage,
        AgentCapability::I2i => Capability::ImageImageToImage,
        AgentCapability::Vision => Capability::VisionAnalysis,
        AgentCapability::Model3d => Capability::Model3d,
        AgentCapability::T2v => Capability::VideoTextToVideo,
        AgentCapability::I2v => Capability::VideoImageToVideo,
        AgentCapability::Speech => {
            return Err("实时语音 Agent 只能通过 game/speech API 调用".to_string());
        }
    })
}

fn clear_stale_project_state(root: &Path) -> Result<(), GameServiceError> {
    let art_bible = root.join("art-bible.md");
    if art_bible.exists() {
        fs::remove_file(art_bible)?;
    }
    let local = root.join(".codex-game/local");
    if local.exists() {
        fs::remove_dir_all(local)?;
    }
    Ok(())
}

fn project_dto(project: Project) -> GameProject {
    GameProject {
        id: project.id.as_str().to_string(),
        name: project.name,
        code: project.code,
        root: project.root,
        state: match project.state {
            ProjectState::Drafting => "drafting",
            ProjectState::StyleSettled => "styleSettled",
            ProjectState::Ready => "ready",
        }
        .to_string(),
    }
}

fn conversation_dto(conversation: Conversation) -> GameConversation {
    GameConversation {
        id: conversation.id.as_str().to_string(),
        project_id: conversation.project_id.as_str().to_string(),
        target_kind: conversation.target_kind.as_str().to_string(),
        target_ref: conversation.target_ref,
        title: conversation.title,
        director_agent_code: conversation.director_agent_code,
        focus_agent_code: conversation.focus_agent_code,
        status: match conversation.status {
            ConversationStatus::Active => "active",
            ConversationStatus::Running => "running",
        }
        .to_string(),
        turn: conversation.turn,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
    }
}

fn message_dto(message: ConversationMessage) -> GameMessage {
    GameMessage {
        id: message.id,
        turn: message.turn,
        role: message.role,
        content: message.content,
        agent_code: message.agent_code,
        recipient_agent_code: message.recipient_agent_code,
        status: match message.status {
            MessageStatus::Thinking => "thinking",
            MessageStatus::Completed => "completed",
            MessageStatus::Failed => "failed",
            MessageStatus::Interrupted => "interrupted",
        }
        .to_string(),
        token_count: message.token_count,
        folded: message.folded,
        attachments: message.attachments,
        action: message
            .action
            .and_then(|action| serde_json::to_value(action).ok()),
        created_at: message.created_at,
    }
}

fn conversation_snapshot_dto(
    snapshot: codex_game_runtime::ConversationSnapshot,
) -> GameConversationReadResponse {
    GameConversationReadResponse {
        conversation: conversation_dto(snapshot.conversation),
        messages: snapshot.messages.into_iter().map(message_dto).collect(),
        drafts: snapshot.drafts.into_iter().map(draft_dto).collect(),
        memories: snapshot.memories.into_iter().map(memory_dto).collect(),
        handoffs: snapshot.handoffs.into_iter().map(handoff_dto).collect(),
    }
}

fn draft_dto(draft: ArtifactDraftRecord) -> GameArtifactDraft {
    GameArtifactDraft {
        id: draft.id,
        conversation_id: draft.conversation_id,
        target_path: draft.target_path,
        content: draft.content,
        based_on_hash: draft.based_on_hash,
        status: draft.status,
        created_at: draft.created_at,
    }
}

fn memory_dto(memory: ConversationMemory) -> GameConversationMemory {
    GameConversationMemory {
        id: memory.id,
        conversation_id: memory.conversation_id,
        scope: memory.scope,
        kind: memory.kind,
        content: memory.content,
        created_at: memory.created_at,
    }
}

fn handoff_dto(handoff: AgentHandoff) -> GameAgentHandoff {
    GameAgentHandoff {
        id: handoff.id,
        conversation_id: handoff.conversation_id,
        turn: handoff.turn,
        from_agent_code: handoff.from_agent_code,
        to_agent_code: handoff.to_agent_code,
        source: handoff.source,
        reason: handoff.reason,
        status: handoff.status,
        created_at: handoff.created_at,
    }
}

fn character_dto(character: Character) -> GameCharacter {
    GameCharacter {
        id: character.id,
        project_id: character.project_id,
        name: character.name,
        group: character.group,
        dir_name: character.dir_name,
        state: match character.state {
            CharacterState::S0SpecDrafting => "S0_spec_drafting",
            CharacterState::S1SpecConfirmed => "S1_spec_confirmed",
            CharacterState::S2RenderGenerated => "S2_render_generated",
            CharacterState::S3RenderConfirmed => "S3_render_confirmed",
            CharacterState::S4ViewsGenerated => "S4_views_generated",
            CharacterState::S5ViewsConfirmed => "S5_views_confirmed",
        }
        .to_string(),
        spec_path: character.spec_path,
        render_path: character.render_path,
        view_paths: serde_json::to_value(character.view_paths).unwrap_or_default(),
        hard_constraints: character.hard_constraints,
        gate_spec_confirmed_at: character.gate_spec_confirmed_at,
        gate_render_confirmed_at: character.gate_render_confirmed_at,
        gate_views_confirmed_at: character.gate_views_confirmed_at,
        created_at: character.created_at,
        updated_at: character.updated_at,
    }
}

fn generation_dto(generation: Generation) -> GameGeneration {
    GameGeneration {
        id: generation.id,
        project_id: generation.project_id,
        target_kind: generation.target_kind,
        target_ref: generation.target_ref,
        stage: generation.stage,
        variant: generation.variant,
        file_path: generation.file_path,
        file_hash: generation.file_hash,
        is_final: generation.is_final,
        source: generation.source,
        task_id: generation.task_id,
        asset_spec: generation.asset_spec,
        created_at: generation.created_at,
    }
}

fn art_bible_dto(version: ArtBibleVersion) -> GameArtBibleVersion {
    GameArtBibleVersion {
        id: version.id.as_str().to_string(),
        project_id: version.project_id.as_str().to_string(),
        version: version.version,
        content_hash: version.content_hash,
        created_at: version.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_game_runtime::ExecutionError;
    use codex_game_runtime::StartThreadRequest;
    use codex_game_runtime::StartTurnRequest;
    use codex_game_runtime::StartedThread;
    use codex_game_runtime::StartedTurn;
    use codex_game_runtime::SteerTurnRequest;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use tempfile::tempdir;

    #[derive(Default)]
    struct FakeExecution {
        threads: AtomicUsize,
        turns: AtomicUsize,
        turn_requests: Mutex<Vec<StartTurnRequest>>,
    }

    impl CodexExecutionPort for FakeExecution {
        async fn start_thread(
            &self,
            _request: StartThreadRequest,
        ) -> Result<StartedThread, ExecutionError> {
            let number = self.threads.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(StartedThread {
                thread_id: format!("thread-{number}"),
                session_id: format!("session-{number}"),
            })
        }

        async fn thread_available(&self, _thread_id: &str) -> bool {
            true
        }

        async fn start_turn(
            &self,
            request: StartTurnRequest,
        ) -> Result<StartedTurn, ExecutionError> {
            self.turn_requests
                .lock()
                .expect("turn request lock")
                .push(request);
            let number = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(StartedTurn {
                turn_id: format!("turn-{number}"),
            })
        }

        async fn steer_turn(&self, _request: SteerTurnRequest) -> Result<(), ExecutionError> {
            Ok(())
        }

        async fn interrupt_turn(
            &self,
            _thread_id: String,
            _turn_id: String,
        ) -> Result<(), ExecutionError> {
            Ok(())
        }
    }

    async fn setup() -> (
        tempfile::TempDir,
        GameAppServerAdapter,
        FakeExecution,
        String,
    ) {
        let directory = tempdir().expect("tempdir");
        let adapter = GameAppServerAdapter::new(directory.path().join("studio"));
        let project_id = "project-1".to_string();
        adapter
            .project_create(
                project_id.clone(),
                GameProjectCreateParams {
                    name: None,
                    root: directory
                        .path()
                        .join("project")
                        .to_string_lossy()
                        .into_owned(),
                    overwrite: None,
                },
            )
            .await
            .expect("project");
        let conversation = adapter
            .conversation_ensure(GameConversationEnsureParams {
                project_id,
                target_kind: "project".to_string(),
                target_ref: None,
                title: None,
                director_agent_code: None,
            })
            .await
            .expect("conversation")
            .conversation;
        (
            directory,
            adapter,
            FakeExecution::default(),
            conversation.id,
        )
    }

    async fn submit(
        adapter: &GameAppServerAdapter,
        execution: &FakeExecution,
        conversation_id: &str,
        content: &str,
    ) -> String {
        adapter
            .conversation_submit(
                execution,
                GameConversationSubmitParams {
                    conversation_id: conversation_id.to_string(),
                    content: content.to_string(),
                    recipient_agent_code: None,
                },
            )
            .await
            .expect("submit")
            .1
            .expect("running task")
            .attempt
            .codex_turn_id
            .expect("turn id")
    }

    fn action(kind: &str, target: Option<&str>, reason: &str, payload: &str) -> String {
        let target = target.map_or_else(|| "null".to_string(), |value| format!("\"{value}\""));
        format!(
            "正文\n{}\n{{\"action\":\"{kind}\",\"target_agent\":{target},\"reason\":\"{reason}\",\"payload\":{payload}}}\n{}",
            codex_game_domain::ACTION_START,
            codex_game_domain::ACTION_END,
        )
    }

    #[test]
    fn spec_writer_draft_completion_requests_director_resume() {
        let completion = codex_game_runtime::CompletedTaskAttempt {
            attempt_id: "attempt-1".to_string(),
            task_id: "task-1".to_string(),
            conversation_id: "conversation-1".to_string(),
            status: TaskAttemptStatus::Succeeded,
            agent_code: Some("spec_writer".to_string()),
            action: Some(codex_game_domain::AgentAction {
                action: AgentActionKind::AskUser,
                target_agent: None,
                reason: "请确认角色设定".to_string(),
                payload: codex_game_domain::AgentActionPayload {
                    drafts: Some(vec![codex_game_domain::ArtifactDraft {
                        target_path: "docs/角色定稿.md".to_string(),
                        content: "# 角色设定".to_string(),
                        based_on_hash: None,
                    }]),
                    ..codex_game_domain::AgentActionPayload::default()
                },
            }),
        };

        let projection = turn_projection(completion);

        assert!(projection.handoff_target.is_none());
        assert!(projection.director_resume_reason.is_some());
    }

    #[tokio::test]
    async fn director_resume_is_a_noop_while_the_conversation_is_running() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let _turn_id = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;

        let resumed = adapter
            .resume_director(
                &execution,
                &conversation_id,
                "继续处理当前工作流".to_string(),
            )
            .await
            .expect("resume should be idempotent");

        assert!(resumed.is_none());
        assert_eq!(execution.turns.load(Ordering::SeqCst), 1);
        let snapshot = adapter
            .conversation_read(GameConversationReadParams { conversation_id })
            .await
            .expect("snapshot");
        assert_eq!(snapshot.conversation.turn, 1);
        assert_eq!(snapshot.messages.len(), 2);
    }

    #[tokio::test]
    async fn conversation_routes_specialist_completion_back_to_director() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let first_turn = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;
        let ask_user = action(
            "ask_user",
            None,
            "需要用户选择风格",
            r#"{"choices":[{"item":"风格","options":["写实","卡通"],"recommended":["卡通"],"multiple":false}]}"#,
        );
        adapter
            .observe_turn_completed(&execution, &first_turn, Some(&ask_user), None)
            .await
            .expect("ask user completion")
            .expect("projection");

        let second_turn = submit(&adapter, &execution, &conversation_id, "选择卡通").await;
        let handoff = action("handoff", Some("game_designer"), "交给美术设计师细化", "{}");
        adapter
            .observe_turn_completed(&execution, &second_turn, Some(&handoff), None)
            .await
            .expect("handoff completion")
            .expect("projection");
        let third = adapter
            .continue_handoff(&execution, &conversation_id, "game_designer")
            .await
            .expect("continue handoff")
            .expect("handoff task");
        let specialist_done = action("done", None, "美术基调已完成", "{}");
        let projection = adapter
            .observe_turn_completed(
                &execution,
                third.attempt.codex_turn_id.as_deref().expect("turn id"),
                Some(&specialist_done),
                None,
            )
            .await
            .expect("return completion")
            .expect("projection");
        assert_eq!(
            projection.handoff_target.as_deref(),
            Some("studio_director")
        );
        let director = adapter
            .continue_handoff(&execution, &conversation_id, "studio_director")
            .await
            .expect("continue to director")
            .expect("director task");
        let done = action("done", None, "本轮工作已完成", "{}");
        adapter
            .observe_turn_completed(
                &execution,
                director.attempt.codex_turn_id.as_deref().expect("turn id"),
                Some(&done),
                None,
            )
            .await
            .expect("director completion")
            .expect("projection");

        let snapshot = adapter
            .conversation_read(GameConversationReadParams { conversation_id })
            .await
            .expect("snapshot");
        assert_eq!(snapshot.conversation.status, "active");
        assert_eq!(snapshot.handoffs.len(), 2);
        assert_eq!(
            snapshot
                .messages
                .last()
                .and_then(|message| message.action.as_ref())
                .and_then(|action| action.get("action"))
                .and_then(serde_json::Value::as_str),
            Some("done")
        );
    }

    #[tokio::test]
    async fn committing_specialist_drafts_resumes_with_the_director() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let first_turn = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;
        let to_designer = action("handoff", Some("game_designer"), "交给设计师处理", "{}");
        adapter
            .observe_turn_completed(&execution, &first_turn, Some(&to_designer), None)
            .await
            .expect("director handoff")
            .expect("projection");
        let designer = adapter
            .continue_handoff(&execution, &conversation_id, "game_designer")
            .await
            .expect("continue to designer")
            .expect("designer task");
        let draft = action(
            "ask_user",
            None,
            "请确认设计草稿",
            r##"{"drafts":[{"target_path":"docs/视觉说明.md","content":"# 视觉说明"}]}"##,
        );
        adapter
            .observe_turn_completed(
                &execution,
                designer.attempt.codex_turn_id.as_deref().expect("turn id"),
                Some(&draft),
                None,
            )
            .await
            .expect("draft completion")
            .expect("projection");
        let draft_id = adapter
            .conversation_read(GameConversationReadParams {
                conversation_id: conversation_id.clone(),
            })
            .await
            .expect("snapshot")
            .drafts
            .into_iter()
            .find(|draft| draft.status == "pending")
            .expect("pending draft")
            .id;

        let (_, task) = adapter
            .conversation_commit_drafts(
                &execution,
                GameConversationCommitDraftsParams {
                    conversation_id: conversation_id.clone(),
                    draft_ids: vec![draft_id],
                },
            )
            .await
            .expect("commit drafts");

        let task = task.expect("director task");
        assert_eq!(task.task.agent_code, "studio_director");
        let snapshot = adapter
            .conversation_read(GameConversationReadParams { conversation_id })
            .await
            .expect("snapshot");
        assert_eq!(snapshot.conversation.status, "running");
        assert_eq!(
            snapshot.conversation.focus_agent_code.as_deref(),
            Some("studio_director")
        );
        assert!(
            snapshot
                .messages
                .iter()
                .any(|message| message.role == "user" && message.folded)
        );
    }

    #[tokio::test]
    async fn output_length_failure_increases_token_limit_until_cap() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let mut turn_id = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;
        let length_error =
            "stream disconnected before completion: Incomplete response returned, reason: length";

        for (retry_number, expected_limit) in
            [32_000, 64_000, 128_000, 200_000].into_iter().enumerate()
        {
            let projection = adapter
                .observe_turn_completed(&execution, &turn_id, None, Some(length_error))
                .await
                .expect("length retry")
                .expect("retry projection");
            assert_eq!(projection.status, "running");
            turn_id = projection.turn_id.expect("retry turn id");
            let requests = execution.turn_requests.lock().expect("turn request lock");
            assert_eq!(
                requests[retry_number + 1].max_output_tokens,
                Some(expected_limit)
            );
        }

        let projection = adapter
            .observe_turn_completed(&execution, &turn_id, None, Some(length_error))
            .await
            .expect("final length failure")
            .expect("failed projection");
        assert_eq!(projection.status, "failed");
        assert_eq!(execution.turns.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn non_length_failure_does_not_retry() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let turn_id = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;

        let projection = adapter
            .observe_turn_completed(
                &execution,
                &turn_id,
                None,
                Some("stream disconnected before completion: connection reset"),
            )
            .await
            .expect("terminal failure")
            .expect("failed projection");

        assert_eq!(projection.status, "failed");
        assert_eq!(execution.turns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn action_contract_retry_can_recover_with_a_valid_output() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let turn_id = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;
        let retry = adapter
            .observe_turn_completed(&execution, &turn_id, Some("协议格式错误"), None)
            .await
            .expect("contract retry")
            .expect("retry projection");
        let valid = action("done", None, "美术基调已完成", "{}");

        let completion = adapter
            .observe_turn_completed(
                &execution,
                retry.turn_id.as_deref().expect("retry turn id"),
                Some(&valid),
                None,
            )
            .await
            .expect("valid retry completion")
            .expect("completion projection");
        assert_eq!(completion.status, "succeeded");
        assert_eq!(execution.turns.load(Ordering::SeqCst), 2);

        let snapshot = adapter
            .conversation_read(GameConversationReadParams { conversation_id })
            .await
            .expect("snapshot");
        let assistant = snapshot.messages.last().expect("assistant message");
        assert_eq!(assistant.status, "completed");
        assert_eq!(assistant.content, "正文");
    }

    #[tokio::test]
    async fn action_contract_failure_retries_three_times_before_failing() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let mut turn_id = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;

        for retry_number in 1..=MAX_ACTION_CONTRACT_RETRIES {
            let projection = adapter
                .observe_turn_completed(&execution, &turn_id, Some("协议格式错误"), None)
                .await
                .expect("contract retry")
                .expect("retry projection");
            assert_eq!(projection.status, "running");
            turn_id = projection.turn_id.expect("retry turn id");
            let requests = execution.turn_requests.lock().expect("turn request lock");
            let prompt = &requests[retry_number as usize].prompt;
            assert!(prompt.contains("Action 契约校验"));
            assert!(prompt.contains(&format!("第 {retry_number} 次自动重试")));
        }

        let retrying_snapshot = adapter
            .conversation_read(GameConversationReadParams {
                conversation_id: conversation_id.clone(),
            })
            .await
            .expect("retrying snapshot");
        let retrying_assistant = retrying_snapshot
            .messages
            .last()
            .expect("retrying assistant message");
        assert_eq!(retrying_assistant.status, "thinking");
        assert!(retrying_assistant.content.is_empty());

        let projection = adapter
            .observe_turn_completed(&execution, &turn_id, Some("仍然格式错误"), None)
            .await
            .expect("final contract failure")
            .expect("failed projection");
        assert_eq!(projection.status, "failed");
        assert_eq!(execution.turns.load(Ordering::SeqCst), 4);

        let snapshot = adapter
            .conversation_read(GameConversationReadParams { conversation_id })
            .await
            .expect("snapshot");
        let assistant = snapshot.messages.last().expect("assistant message");
        assert_eq!(assistant.status, "failed");
        assert!(assistant.content.contains("Action 协议校验失败"));
        assert!(assistant.content.contains("已自动重试 3 次"));
    }
}
