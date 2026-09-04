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
use codex_game_runtime::Capability;
use codex_game_runtime::CodexExecutionPort;
use codex_game_runtime::ExecuteTaskRequest;
use codex_game_runtime::GAME_PROTOCOL_VERSION;
use codex_game_runtime::GameRuntime;
use codex_game_runtime::GameServiceError;
use codex_game_runtime::PreparedConversationTurn;
use codex_game_runtime::RouteCandidate;
use codex_game_runtime::RouteFailureKind;
use codex_game_runtime::RouteOutcome;
use codex_game_runtime::TaskExecution;
use codex_game_runtime::bundled_agent_definitions;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

mod ai_config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTurnProjection {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub status: String,
    pub agent_code: Option<String>,
    pub handoff_target: Option<String>,
    pub handoff_reason: Option<String>,
    pub character: Option<GameCharacter>,
    pub generations: Vec<GameGeneration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTurnEventContext {
    pub conversation_id: String,
    pub agent_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelledTaskAttempt {
    pub task_id: String,
    pub attempt_id: String,
    pub turn_id: String,
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

    pub async fn observe_turn_completed(
        &self,
        turn_id: &str,
        output: Option<&str>,
        failed: bool,
    ) -> Result<Option<GameTurnProjection>, GameServiceError> {
        let completion = if failed {
            self.runtime
                .service()
                .complete_turn(turn_id, TaskAttemptStatus::Failed)
                .await?
        } else {
            self.runtime
                .service()
                .complete_turn_output(turn_id, output)
                .await?
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
        match self
            .runtime
            .orchestrator()
            .execute(
                execution,
                prepared.store.as_ref(),
                ExecuteTaskRequest {
                    project_root: prepared.project.root.clone(),
                    conversation_id: prepared.conversation.id.as_str().to_string(),
                    target_id: prepared
                        .conversation
                        .target_ref
                        .clone()
                        .unwrap_or_else(|| prepared.project.id.as_str().to_string()),
                    stage: prepared.stage.clone(),
                    agent_code: prepared.agent_code.clone(),
                    idempotency_key: prepared.assistant_message.id.clone(),
                    prompt: prepared.user_message.content.clone(),
                    context,
                    capability,
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
    ) -> Result<(GameConversationInterruptResponse, Vec<CancelledTaskAttempt>), String> {
        let (_, store) = self
            .runtime
            .service()
            .execution_context(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        let running = store
            .running_attempts(&params.conversation_id)
            .await
            .map_err(|error| error.to_string())?;
        let mut cancelled = Vec::with_capacity(running.len());
        for attempt in running {
            self.runtime
                .orchestrator()
                .interrupt(
                    execution,
                    store.as_ref(),
                    &params.conversation_id,
                    &attempt.agent_code,
                    &attempt.attempt_id,
                    attempt.thread_id,
                    attempt.turn_id.clone(),
                )
                .await
                .map_err(|error| error.to_string())?;
            cancelled.push(CancelledTaskAttempt {
                task_id: attempt.task_id,
                attempt_id: attempt.attempt_id,
                turn_id: attempt.turn_id,
            });
        }
        Ok((GameConversationInterruptResponse {}, cancelled))
    }

    pub async fn conversation_commit_drafts(
        &self,
        params: GameConversationCommitDraftsParams,
    ) -> Result<GameConversationCommitDraftsResponse, GameServiceError> {
        self.runtime
            .service()
            .commit_conversation_drafts(&params.conversation_id, &params.draft_ids)
            .await?;
        Ok(GameConversationCommitDraftsResponse {})
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

    pub async fn character_list(
        &self,
        params: GameCharacterListParams,
    ) -> Result<GameCharacterListResponse, GameServiceError> {
        self.runtime
            .service()
            .list_characters(&params.project_id)
            .await
            .map(|characters| GameCharacterListResponse {
                characters: characters.into_iter().map(character_dto).collect(),
            })
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
        Ok(GameCharacterReadResponse {
            character: character_dto(character),
            generations: generations.into_iter().map(generation_dto).collect(),
        })
    }

    pub async fn character_confirm_spec(
        &self,
        params: GameCharacterConfirmSpecParams,
    ) -> Result<GameCharacterResponse, GameServiceError> {
        self.runtime
            .service()
            .confirm_character_spec(&params.project_id, &params.character_id, &params.draft_id)
            .await
            .map(|character| GameCharacterResponse {
                character: character_dto(character),
            })
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

    pub async fn character_reject_spec(
        &self,
        params: GameCharacterRejectSpecParams,
    ) -> Result<GameCharacterResponse, GameServiceError> {
        self.runtime
            .service()
            .reject_character_stage(
                &params.project_id,
                &params.character_id,
                "spec",
                params.reason,
            )
            .await
            .map(|character| GameCharacterResponse {
                character: character_dto(character),
            })
    }

    pub async fn character_confirm_render(
        &self,
        params: GameCharacterConfirmRenderParams,
    ) -> Result<GameCharacterResponse, GameServiceError> {
        self.runtime
            .service()
            .confirm_character_render(
                &params.project_id,
                &params.character_id,
                &params.generation_id,
            )
            .await
            .map(|character| GameCharacterResponse {
                character: character_dto(character),
            })
    }

    pub async fn character_reject_render(
        &self,
        params: GameCharacterRejectRenderParams,
    ) -> Result<GameCharacterResponse, GameServiceError> {
        self.runtime
            .service()
            .reject_character_stage(
                &params.project_id,
                &params.character_id,
                "render",
                params.reason,
            )
            .await
            .map(|character| GameCharacterResponse {
                character: character_dto(character),
            })
    }

    pub async fn character_confirm_views(
        &self,
        params: GameCharacterConfirmViewsParams,
    ) -> Result<GameCharacterResponse, GameServiceError> {
        self.runtime
            .service()
            .confirm_character_views(
                &params.project_id,
                &params.character_id,
                &params.generation_ids,
            )
            .await
            .map(|character| GameCharacterResponse {
                character: character_dto(character),
            })
    }

    pub async fn character_reject_views(
        &self,
        params: GameCharacterRejectViewsParams,
    ) -> Result<GameCharacterResponse, GameServiceError> {
        self.runtime
            .service()
            .reject_character_stage(
                &params.project_id,
                &params.character_id,
                "views",
                params.reason,
            )
            .await
            .map(|character| GameCharacterResponse {
                character: character_dto(character),
            })
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

fn turn_projection(completion: codex_game_runtime::CompletedTaskAttempt) -> GameTurnProjection {
    let (handoff_target, handoff_reason) = completion
        .action
        .as_ref()
        .filter(|action| action.action == AgentActionKind::Handoff)
        .map(|action| (action.target_agent.clone(), Some(action.reason.clone())))
        .unwrap_or((None, None));
    GameTurnProjection {
        attempt_id: completion.attempt_id,
        task_id: completion.task_id,
        conversation_id: completion.conversation_id,
        status: format!("{:?}", completion.status).to_lowercase(),
        agent_code: completion.agent_code,
        handoff_target,
        handoff_reason,
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
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use tempfile::tempdir;

    #[derive(Default)]
    struct FakeExecution {
        threads: AtomicUsize,
        turns: AtomicUsize,
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
            _request: StartTurnRequest,
        ) -> Result<StartedTurn, ExecutionError> {
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

    #[tokio::test]
    async fn conversation_runs_ask_user_handoff_done_with_fake_execution() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let first_turn = submit(&adapter, &execution, &conversation_id, "定义美术基调").await;
        let ask_user = action(
            "ask_user",
            None,
            "需要用户选择风格",
            r#"{"choices":[{"item":"风格","options":["写实","卡通"],"recommended":["卡通"],"multiple":false}]}"#,
        );
        adapter
            .observe_turn_completed(&first_turn, Some(&ask_user), false)
            .await
            .expect("ask user completion")
            .expect("projection");

        let second_turn = submit(&adapter, &execution, &conversation_id, "选择卡通").await;
        let handoff = action("handoff", Some("game_designer"), "交给美术设计师细化", "{}");
        adapter
            .observe_turn_completed(&second_turn, Some(&handoff), false)
            .await
            .expect("handoff completion")
            .expect("projection");
        let third = adapter
            .continue_handoff(&execution, &conversation_id, "game_designer")
            .await
            .expect("continue handoff")
            .expect("handoff task");
        let done = action("done", None, "美术基调已完成", "{}");
        adapter
            .observe_turn_completed(
                third.attempt.codex_turn_id.as_deref().expect("turn id"),
                Some(&done),
                false,
            )
            .await
            .expect("done completion")
            .expect("projection");

        let snapshot = adapter
            .conversation_read(GameConversationReadParams { conversation_id })
            .await
            .expect("snapshot");
        assert_eq!(snapshot.conversation.status, "active");
        assert_eq!(snapshot.handoffs.len(), 1);
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
    async fn conversation_rejects_a_third_automatic_handoff() {
        let (_directory, adapter, execution, conversation_id) = setup().await;
        let first_turn = submit(&adapter, &execution, &conversation_id, "开始协作").await;
        let to_designer = action("handoff", Some("game_designer"), "交给设计师处理", "{}");
        adapter
            .observe_turn_completed(&first_turn, Some(&to_designer), false)
            .await
            .expect("first handoff")
            .expect("projection");
        let second = adapter
            .continue_handoff(&execution, &conversation_id, "game_designer")
            .await
            .expect("continue first")
            .expect("second task");
        let to_director = action("handoff", Some("studio_director"), "交回总管收口", "{}");
        adapter
            .observe_turn_completed(
                second.attempt.codex_turn_id.as_deref().expect("turn id"),
                Some(&to_director),
                false,
            )
            .await
            .expect("second handoff")
            .expect("projection");

        let error = adapter
            .continue_handoff(&execution, &conversation_id, "studio_director")
            .await
            .expect_err("third handoff must be rejected");
        assert!(error.contains("不得超过两次"));
    }
}
