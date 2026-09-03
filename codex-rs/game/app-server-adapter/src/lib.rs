use codex_game_app_server_protocol::*;
use codex_game_domain::ArtBibleVersion;
use codex_game_domain::ArtifactContent;
use codex_game_domain::ArtifactId;
use codex_game_domain::ContextPackage;
use codex_game_domain::Conversation;
use codex_game_domain::FocusWorkflow;
use codex_game_domain::Project;
use codex_game_domain::ProjectState;
use codex_game_domain::TaskAttemptStatus;
use codex_game_domain::UserDecision;
use codex_game_domain::WorkflowCommand;
use codex_game_domain::WorkflowState;
use codex_game_import::LegacyProjectImporter;
use codex_game_runtime::Capability;
use codex_game_runtime::CodexExecutionPort;
use codex_game_runtime::ExecuteTaskRequest;
use codex_game_runtime::GAME_PROTOCOL_VERSION;
use codex_game_runtime::GameRuntime;
use codex_game_runtime::GameServiceError;
use codex_game_runtime::RouteCandidate;
use codex_game_runtime::RouteFailureKind;
use codex_game_runtime::RouteOutcome;
use codex_game_runtime::TaskExecution;
use codex_game_runtime::review_agent_codes;
use futures::future::try_join_all;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTurnProjection {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub status: String,
    pub artifacts: Vec<(String, String)>,
    pub workflow: Option<GameFocusWorkflow>,
    pub conflict_count: Option<u64>,
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
}

impl GameAppServerAdapter {
    pub fn new(studio_storage: PathBuf) -> Self {
        Self {
            runtime: GameRuntime::new(studio_storage),
        }
    }

    pub fn new_with_routes(studio_storage: PathBuf, candidates: Vec<RouteCandidate>) -> Self {
        Self {
            runtime: GameRuntime::new_with_routes(studio_storage, candidates),
        }
    }

    pub fn ping(&self) -> GamePingResponse {
        GamePingResponse {
            protocol_version: GAME_PROTOCOL_VERSION,
            backend_version: env!("CARGO_PKG_VERSION").to_string(),
            status: self.runtime.status().into(),
        }
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
                .map_err(|_| GameServiceError::StateUnavailable)?;
        }
        Ok(completion.map(turn_projection))
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

    pub async fn project_create(
        &self,
        project_id: String,
        params: GameProjectCreateParams,
    ) -> Result<GameProjectCreateResponse, GameServiceError> {
        self.runtime
            .create_project(project_id, params.name, params.root)
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

    pub async fn project_import(
        &self,
        params: GameProjectImportParams,
    ) -> Result<GameProjectImportResponse, String> {
        let destination = PathBuf::from(&params.destination);
        let report = LegacyProjectImporter::import(Path::new(&params.source), &destination)
            .await
            .map_err(|error| error.to_string())?;
        let project = match self.runtime.open_project(params.destination, false).await {
            Ok(project) => project,
            Err(error) => {
                let rollback = fs::remove_dir_all(&destination)
                    .err()
                    .map(|rollback| format!("; rollback failed: {rollback}"))
                    .unwrap_or_default();
                return Err(format!("{error}{rollback}"));
            }
        };
        Ok(GameProjectImportResponse {
            project: project_dto(project),
            warnings: report.warnings,
        })
    }

    pub async fn conversation_ensure(
        &self,
        params: GameConversationEnsureParams,
    ) -> Result<GameConversationEnsureResponse, GameServiceError> {
        self.runtime
            .service()
            .ensure_conversation(&params.project_id, params.target_id)
            .await
            .map(|conversation| GameConversationEnsureResponse {
                conversation: conversation_dto(conversation),
            })
    }

    pub async fn conversation_submit<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameConversationSubmitParams,
    ) -> Result<
        (
            GameConversationSubmitResponse,
            codex_game_runtime::TaskExecution,
        ),
        String,
    > {
        let content = params.content;
        let message = self
            .runtime
            .service()
            .submit_message(&params.conversation_id, content.clone())
            .await
            .map_err(|error| error.to_string())?;
        let workflow = self
            .runtime
            .service()
            .read_focus(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        let (project, store) = self
            .runtime
            .service()
            .execution_context(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        let context = ContextPackage {
            brief_artifact_id: ArtifactId::new(Uuid::now_v7().to_string()),
            confirmed_decisions: Vec::new(),
            artifact_summaries: Vec::new(),
            context_version: workflow.input_version,
            workflow_version: workflow.workflow_version,
            agent_definition_version: "1".to_string(),
            output_schema: structured_brief_schema(),
        };
        let capability = Capability::TextStructuredOutput;
        let task_execution = self
            .runtime
            .orchestrator()
            .execute(
                execution,
                store.as_ref(),
                ExecuteTaskRequest {
                    project_root: project.root,
                    conversation_id: params.conversation_id,
                    target_id: workflow.id.as_str().to_string(),
                    stage: "brief".to_string(),
                    agent_code: "brief".to_string(),
                    idempotency_key: message.id.clone(),
                    prompt: content,
                    context,
                    capability,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok((
            GameConversationSubmitResponse {
                message: GameMessage {
                    id: message.id,
                    role: message.role,
                    content: message.content,
                    created_at: message.created_at,
                },
            },
            task_execution,
        ))
    }

    pub fn conversation_read(
        &self,
        params: GameConversationReadParams,
    ) -> Result<GameConversationReadResponse, GameServiceError> {
        self.runtime
            .service()
            .read_conversation(&params.conversation_id)
            .map(|snapshot| GameConversationReadResponse {
                conversation: conversation_dto(snapshot.conversation),
                messages: snapshot
                    .messages
                    .into_iter()
                    .map(|message| GameMessage {
                        id: message.id,
                        role: message.role,
                        content: message.content,
                        created_at: message.created_at,
                    })
                    .collect(),
            })
    }

    pub async fn focus_start(
        &self,
        params: GameFocusStartParams,
    ) -> Result<GameFocusStartResponse, GameServiceError> {
        self.runtime
            .service()
            .start_focus(&params.conversation_id)
            .await
            .map(|workflow| GameFocusStartResponse {
                workflow: workflow_dto(workflow),
            })
    }

    pub async fn focus_read(
        &self,
        params: GameFocusReadParams,
    ) -> Result<GameFocusReadResponse, GameServiceError> {
        let workflow = self.runtime.service().read_focus(&params.conversation_id)?;
        let artifacts = self
            .runtime
            .service()
            .read_focus_artifacts(&params.conversation_id)
            .await?;
        let mut reviews = Vec::new();
        let mut conflicts = Vec::new();
        let mut art_bible_draft = None;
        let mut decisions = Vec::new();
        for artifact in artifacts {
            match artifact.content {
                ArtifactContent::ReviewReport(report) => reviews.push(GameReviewReport {
                    agent_code: report.agent_code,
                    findings: report.findings,
                    risks: report.risks,
                    recommendations: report.recommendations,
                }),
                ArtifactContent::ConflictSet(set) => {
                    conflicts = set
                        .conflicts
                        .into_iter()
                        .map(|conflict| GameConflict {
                            key: conflict.key,
                            description: conflict.description,
                            options: conflict.options,
                            high_impact: conflict.high_impact,
                        })
                        .collect();
                }
                ArtifactContent::ArtBibleDraft(draft) => {
                    art_bible_draft = Some(draft.markdown);
                }
                ArtifactContent::UserDecision(decision) => decisions.push(GameUserDecision {
                    conflict_key: decision.conflict_key,
                    selected_option: decision.selected_option,
                    note: decision.note,
                }),
                ArtifactContent::StructuredBrief(_) | ArtifactContent::Other(_) => {}
            }
        }
        Ok(GameFocusReadResponse {
            workflow: workflow_dto(workflow),
            reviews,
            conflicts,
            art_bible_draft,
            decisions,
        })
    }

    pub async fn focus_decide<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameFocusDecideParams,
    ) -> Result<
        (
            GameFocusDecideResponse,
            Vec<TaskExecution>,
            Vec<(String, String)>,
        ),
        String,
    > {
        let action = params.action;
        let conversation_id = params.conversation_id.clone();
        if action == GameFocusAction::RecordConflictDecision {
            let decision = params
                .user_decision
                .ok_or_else(|| "recordConflictDecision requires userDecision".to_string())?;
            let (workflow, artifact) = self
                .runtime
                .service()
                .record_conflict_decision(
                    &conversation_id,
                    params.expected_input_version,
                    UserDecision {
                        conflict_key: decision.conflict_key,
                        selected_option: decision.selected_option,
                        note: decision.note,
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
            return Ok((
                GameFocusDecideResponse {
                    workflow: workflow_dto(workflow),
                    art_bible: None,
                },
                Vec::new(),
                vec![(artifact.id.as_str().to_string(), "userDecision".to_string())],
            ));
        }
        if params.user_decision.is_some() {
            return Err("userDecision is only valid for recordConflictDecision".to_string());
        }
        let command = match action {
            GameFocusAction::SubmitClarification => WorkflowCommand::SubmitClarification,
            GameFocusAction::AcceptBrief => WorkflowCommand::AcceptBrief,
            GameFocusAction::CompleteReviews => WorkflowCommand::CompleteReviews,
            GameFocusAction::CompleteMerge => WorkflowCommand::CompleteMerge,
            GameFocusAction::RecordConflictDecision => unreachable!("handled above"),
            GameFocusAction::ConfirmArtBible => WorkflowCommand::ConfirmArtBible,
            GameFocusAction::VersionArtBible => WorkflowCommand::VersionArtBible,
        };
        let (workflow, art_bible) = self
            .runtime
            .service()
            .advance_focus(
                &conversation_id,
                command,
                params.expected_input_version,
                params.art_bible_markdown,
            )
            .await
            .map_err(|error| error.to_string())?;
        let tasks = if action == GameFocusAction::AcceptBrief {
            self.start_reviews(execution, &conversation_id, &workflow)
                .await?
        } else {
            Vec::new()
        };
        Ok((
            GameFocusDecideResponse {
                workflow: workflow_dto(workflow),
                art_bible: art_bible.map(|document| art_bible_dto(document.version)),
            },
            tasks,
            Vec::new(),
        ))
    }

    async fn start_reviews<E: CodexExecutionPort>(
        &self,
        execution: &E,
        conversation_id: &str,
        workflow: &FocusWorkflow,
    ) -> Result<Vec<TaskExecution>, String> {
        let (project, store) = self
            .runtime
            .service()
            .execution_context(conversation_id)
            .map_err(|error| error.to_string())?;
        let brief = store
            .latest_artifact(conversation_id, "structuredBrief")
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "accepted focus workflow has no committed brief".to_string())?;
        if !matches!(&brief.content, ArtifactContent::StructuredBrief(_)) {
            return Err("latest brief artifact has an invalid type".to_string());
        }
        let summary = serde_json::to_string(&brief.content).map_err(|error| error.to_string())?;
        try_join_all(review_agent_codes().into_iter().map(|agent_code| {
            self.runtime.orchestrator().execute(
                execution,
                store.as_ref(),
                ExecuteTaskRequest {
                    project_root: project.root.clone(),
                    conversation_id: conversation_id.to_string(),
                    target_id: workflow.id.as_str().to_string(),
                    stage: "review".to_string(),
                    agent_code: agent_code.to_string(),
                    idempotency_key: format!(
                        "focus-review:{}:{}:{}",
                        workflow.id.as_str(),
                        agent_code,
                        workflow.workflow_version
                    ),
                    prompt: format!(
                        "评审当前 StructuredBrief。输出中的 agentCode 必须为 {agent_code}。"
                    ),
                    context: ContextPackage {
                        brief_artifact_id: brief.id.clone(),
                        confirmed_decisions: Vec::new(),
                        artifact_summaries: vec![summary.clone()],
                        context_version: workflow.input_version,
                        workflow_version: workflow.workflow_version,
                        agent_definition_version: "1".to_string(),
                        output_schema: review_report_schema(),
                    },
                    capability: Capability::TextStructuredOutput,
                },
            )
        }))
        .await
        .map_err(|error| error.to_string())
    }

    pub async fn start_synthesis<E: CodexExecutionPort>(
        &self,
        execution: &E,
        conversation_id: &str,
    ) -> Result<TaskExecution, String> {
        let workflow = self
            .runtime
            .service()
            .read_focus(conversation_id)
            .map_err(|error| error.to_string())?;
        if workflow.state != WorkflowState::Merging {
            return Err("synthesis requires a merging workflow".to_string());
        }
        let (project, store) = self
            .runtime
            .service()
            .execution_context(conversation_id)
            .map_err(|error| error.to_string())?;
        let brief = store
            .latest_artifact(conversation_id, "structuredBrief")
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "synthesis requires a committed brief".to_string())?;
        let reviews = store
            .artifacts_for_workflow(workflow.id.as_str(), "reviewReport")
            .await
            .map_err(|error| error.to_string())?;
        if reviews.len() != review_agent_codes().len() {
            return Err("synthesis requires all review artifacts".to_string());
        }
        let artifact_summaries = std::iter::once(&brief)
            .chain(reviews.iter())
            .map(|artifact| serde_json::to_string(&artifact.content))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        self.runtime
            .orchestrator()
            .execute(
                execution,
                store.as_ref(),
                ExecuteTaskRequest {
                    project_root: project.root,
                    conversation_id: conversation_id.to_string(),
                    target_id: workflow.id.as_str().to_string(),
                    stage: "synthesis".to_string(),
                    agent_code: "synthesis".to_string(),
                    idempotency_key: format!(
                        "focus-synthesis:{}:{}",
                        workflow.id.as_str(),
                        workflow.workflow_version
                    ),
                    prompt: "综合 Brief 与三份评审，生成 Art Bible 草案和冲突集合。".to_string(),
                    context: ContextPackage {
                        brief_artifact_id: brief.id,
                        confirmed_decisions: Vec::new(),
                        artifact_summaries,
                        context_version: workflow.input_version,
                        workflow_version: workflow.workflow_version,
                        agent_definition_version: "1".to_string(),
                        output_schema: synthesis_result_schema(),
                    },
                    capability: Capability::TextStructuredOutput,
                },
            )
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn focus_retry<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameFocusRetryParams,
    ) -> Result<(GameFocusRetryResponse, TaskExecution), String> {
        let (project, store) = self
            .runtime
            .service()
            .execution_context(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        if store
            .latest_retryable_task(&params.conversation_id)
            .await
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("focus workflow has no failed or cancelled task to retry".to_string());
        }
        let (workflow, _) = self
            .runtime
            .service()
            .advance_focus(
                &params.conversation_id,
                WorkflowCommand::Retry,
                params.expected_input_version,
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        let task = self
            .runtime
            .orchestrator()
            .retry(
                execution,
                store.as_ref(),
                project.root,
                &params.conversation_id,
                &workflow,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok((
            GameFocusRetryResponse {
                workflow: workflow_dto(workflow),
            },
            task,
        ))
    }

    pub async fn focus_cancel<E: CodexExecutionPort>(
        &self,
        execution: &E,
        params: GameFocusCancelParams,
    ) -> Result<(GameFocusCancelResponse, Vec<CancelledTaskAttempt>), String> {
        let current = self
            .runtime
            .service()
            .read_focus(&params.conversation_id)
            .map_err(|error| error.to_string())?;
        codex_game_domain::validate_input_version(
            params.expected_input_version,
            current.input_version,
        )
        .map_err(|error| error.to_string())?;
        current
            .state
            .apply(WorkflowCommand::Cancel)
            .map_err(|error| error.to_string())?;
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
        let (workflow, _) = self
            .runtime
            .service()
            .advance_focus(
                &params.conversation_id,
                WorkflowCommand::Cancel,
                params.expected_input_version,
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok((
            GameFocusCancelResponse {
                workflow: workflow_dto(workflow),
            },
            cancelled,
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

fn turn_projection(completion: codex_game_runtime::CompletedTaskAttempt) -> GameTurnProjection {
    let artifacts = completion
        .artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.id.as_str().to_string(),
                match &artifact.content {
                    ArtifactContent::StructuredBrief(_) => "structuredBrief",
                    ArtifactContent::ReviewReport(_) => "reviewReport",
                    ArtifactContent::ConflictSet(_) => "conflictSet",
                    ArtifactContent::ArtBibleDraft(_) => "artBibleDraft",
                    ArtifactContent::UserDecision(_) => "userDecision",
                    ArtifactContent::Other(_) => "other",
                }
                .to_string(),
            )
        })
        .collect();
    GameTurnProjection {
        attempt_id: completion.attempt_id,
        task_id: completion.task_id,
        conversation_id: completion.conversation_id,
        status: format!("{:?}", completion.status).to_lowercase(),
        artifacts,
        workflow: completion.workflow.map(workflow_dto),
        conflict_count: completion.conflict_count,
    }
}

fn synthesis_result_schema() -> String {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["draft", "conflicts"],
        "properties": {
            "draft": {
                "type": "object",
                "additionalProperties": false,
                "required": ["markdown", "unresolvedAssumptions"],
                "properties": {
                    "markdown": { "type": "string" },
                    "unresolvedAssumptions": { "type": "array", "items": { "type": "string" } }
                }
            },
            "conflicts": {
                "type": "object",
                "additionalProperties": false,
                "required": ["conflicts"],
                "properties": {
                    "conflicts": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["key", "description", "options", "highImpact"],
                            "properties": {
                                "key": { "type": "string" },
                                "description": { "type": "string" },
                                "options": { "type": "array", "items": { "type": "string" } },
                                "highImpact": { "type": "boolean" }
                            }
                        }
                    }
                }
            }
        }
    })
    .to_string()
}

fn review_report_schema() -> String {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["agentCode", "findings", "risks", "recommendations"],
        "properties": {
            "agentCode": { "type": "string" },
            "findings": { "type": "array", "items": { "type": "string" } },
            "risks": { "type": "array", "items": { "type": "string" } },
            "recommendations": { "type": "array", "items": { "type": "string" } }
        }
    })
    .to_string()
}

fn structured_brief_schema() -> String {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "coreExperience",
            "themeAndMood",
            "targetPlayers",
            "playerPerspective",
            "gameplayPillars",
            "openQuestions"
        ],
        "properties": {
            "coreExperience": { "type": "string" },
            "themeAndMood": { "type": "string" },
            "targetPlayers": { "type": "string" },
            "playerPerspective": { "type": "string" },
            "gameplayPillars": { "type": "array", "items": { "type": "string" } },
            "openQuestions": { "type": "array", "items": { "type": "string" } }
        }
    })
    .to_string()
}

fn project_dto(project: Project) -> GameProject {
    GameProject {
        id: project.id.as_str().to_string(),
        name: project.name,
        root: project.root,
        state: match project.state {
            ProjectState::Unversioned => "unversioned",
            ProjectState::FocusInProgress => "focusInProgress",
            ProjectState::Versioned => "versioned",
        }
        .to_string(),
    }
}

fn conversation_dto(conversation: Conversation) -> GameConversation {
    GameConversation {
        id: conversation.id.as_str().to_string(),
        project_id: conversation.project_id.as_str().to_string(),
        target_id: conversation.target_id,
        created_at: conversation.created_at,
    }
}

fn workflow_dto(workflow: FocusWorkflow) -> GameFocusWorkflow {
    GameFocusWorkflow {
        id: workflow.id.as_str().to_string(),
        project_id: workflow.project_id.as_str().to_string(),
        conversation_id: workflow.conversation_id.as_str().to_string(),
        state: workflow_state_name(workflow.state).to_string(),
        input_version: workflow.input_version,
        workflow_version: workflow.workflow_version,
    }
}

fn workflow_state_name(state: WorkflowState) -> &'static str {
    match state {
        WorkflowState::Draft => "DRAFT",
        WorkflowState::Clarifying => "CLARIFYING",
        WorkflowState::BriefReady => "BRIEF_READY",
        WorkflowState::Reviewing => "REVIEWING",
        WorkflowState::Merging => "MERGING",
        WorkflowState::UserReview => "USER_REVIEW",
        WorkflowState::Confirmed => "CONFIRMED",
        WorkflowState::Versioned => "VERSIONED",
        WorkflowState::Cancelled => "CANCELLED",
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
