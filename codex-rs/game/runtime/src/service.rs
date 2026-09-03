use codex_game_domain::ArtBibleVersion;
use codex_game_domain::ArtBibleVersionId;
use codex_game_domain::Artifact;
use codex_game_domain::ArtifactContent;
use codex_game_domain::ArtifactId;
use codex_game_domain::Conversation;
use codex_game_domain::ConversationId;
use codex_game_domain::ConversationMessage;
use codex_game_domain::FocusWorkflow;
use codex_game_domain::FocusWorkflowId;
use codex_game_domain::Project;
use codex_game_domain::ProjectId;
use codex_game_domain::ProjectState;
use codex_game_domain::ReviewReport;
use codex_game_domain::StructuredBrief;
use codex_game_domain::SynthesisResult;
use codex_game_domain::TaskAttemptStatus;
use codex_game_domain::UserDecision;
use codex_game_domain::WorkflowCommand;
use codex_game_domain::WorkflowError;
use codex_game_domain::WorkflowState;
use codex_game_store::ProjectAccess;
use codex_game_store::ProjectStore;
use codex_game_store::StoreError;
use codex_game_store::list_registered_projects;
use codex_game_store::open_studio_store;
use codex_game_store::register_project as register_studio_project;
use codex_game_store::update_project_json;
use codex_game_store::write_art_bible;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtBibleDocument {
    pub version: ArtBibleVersion,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSnapshot {
    pub conversation: Conversation,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedTaskAttempt {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub status: TaskAttemptStatus,
    pub artifacts: Vec<Artifact>,
    pub workflow: Option<FocusWorkflow>,
    pub conflict_count: Option<u64>,
}

#[derive(Debug, Error)]
pub enum GameServiceError {
    #[error("game runtime state is unavailable")]
    StateUnavailable,
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("conversation not found: {0}")]
    ConversationNotFound(String),
    #[error("focus workflow not found for conversation: {0}")]
    WorkflowNotFound(String),
    #[error("project is read-only: {0}")]
    ReadOnly(String),
    #[error("invalid project path: {0}")]
    InvalidProjectPath(String),
    #[error("invalid design decision: {0}")]
    InvalidDesignDecision(String),
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug)]
pub struct GameService {
    projects: Mutex<HashMap<String, ProjectSession>>,
    studio_storage: Option<PathBuf>,
}

impl Default for GameService {
    fn default() -> Self {
        Self {
            projects: Mutex::new(HashMap::new()),
            studio_storage: None,
        }
    }
}

#[derive(Debug)]
struct ProjectSession {
    project: Project,
    read_only: bool,
    store: Arc<ProjectStore>,
    conversations: HashMap<String, ConversationSnapshot>,
    workflows: HashMap<String, FocusWorkflow>,
    art_bibles: Vec<ArtBibleDocument>,
}

impl GameService {
    pub fn new(studio_storage: PathBuf) -> Self {
        Self {
            projects: Mutex::new(HashMap::new()),
            studio_storage: Some(studio_storage),
        }
    }

    pub async fn create_project(
        &self,
        project_id: String,
        name: String,
        root: String,
    ) -> Result<Project, GameServiceError> {
        let root_path = absolute_root(&root)?;
        fs::create_dir_all(&root_path)?;
        let project = Project {
            id: ProjectId::new(project_id),
            name,
            root: root_path.to_string_lossy().into_owned(),
            state: ProjectState::Unversioned,
        };
        update_project_json(
            &root_path.join("project.json"),
            project.id.as_str(),
            &project.name,
            "unversioned",
        )?;
        fs::create_dir_all(root_path.join(".codex-game"))?;
        let store = Arc::new(ProjectStore::open(&root_path).await?);
        self.register_project(&project, store.access()).await?;
        self.projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?
            .insert(
                project.id.as_str().to_string(),
                ProjectSession::new(project.clone(), false, store),
            );
        Ok(project)
    }

    pub async fn open_project(
        &self,
        root: String,
        read_only: bool,
    ) -> Result<Project, GameServiceError> {
        let root_path = absolute_root(&root)?;
        let document = fs::read_to_string(root_path.join("project.json"))?;
        let value: serde_json::Value = serde_json::from_str(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let id = value
            .get("projectId")
            .or_else(|| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| GameServiceError::InvalidProjectPath(root.clone()))?;
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Untitled Game");
        let state = match value.get("state").and_then(serde_json::Value::as_str) {
            Some("focusInProgress") => ProjectState::FocusInProgress,
            Some("versioned") => ProjectState::Versioned,
            _ => ProjectState::Unversioned,
        };
        let mut project = Project {
            id: ProjectId::new(id),
            name: name.to_string(),
            root: root_path.to_string_lossy().into_owned(),
            state,
        };
        {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            if let Some(session) = projects.get(project.id.as_str())
                && (!session.read_only || read_only)
            {
                return Ok(session.project.clone());
            }
        }
        let store = Arc::new(if read_only {
            ProjectStore::open_read_only(&root_path).await?
        } else {
            ProjectStore::open(&root_path).await?
        });
        let recovered = store.access() == ProjectAccess::ReadOnly;
        let conversations = store.load_conversations(project.id.as_str()).await?;
        let workflows = store.load_workflows(project.id.as_str()).await?;
        let art_bibles = store.load_art_bible_versions(project.id.as_str()).await?;
        project.state = recovered_project_state(&workflows, &art_bibles);
        if store.access() == ProjectAccess::ReadWrite {
            store.recover_incomplete_attempts().await?;
            if let Some((_, markdown)) = art_bibles.last() {
                write_art_bible(&root_path.join("art-bible.md"), markdown)?;
            }
            update_project_json(
                &root_path.join("project.json"),
                project.id.as_str(),
                &project.name,
                project_state_name(project.state),
            )?;
        }
        self.register_project(&project, store.access()).await?;
        let mut session = ProjectSession::new(project.clone(), recovered, store);
        session.conversations = conversations
            .into_iter()
            .map(|(conversation, messages)| {
                (
                    conversation.id.as_str().to_string(),
                    ConversationSnapshot {
                        conversation,
                        messages,
                    },
                )
            })
            .collect();
        session.workflows = workflows
            .into_iter()
            .map(|workflow| (workflow.conversation_id.as_str().to_string(), workflow))
            .collect();
        session.art_bibles = art_bibles
            .into_iter()
            .map(|(version, markdown)| ArtBibleDocument { version, markdown })
            .collect();
        self.projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?
            .insert(project.id.as_str().to_string(), session);
        Ok(project)
    }

    async fn register_project(
        &self,
        project: &Project,
        access: ProjectAccess,
    ) -> Result<(), GameServiceError> {
        let Some(studio_storage) = &self.studio_storage else {
            return Ok(());
        };
        let studio = open_studio_store(studio_storage).await?;
        register_studio_project(&studio, project, access, now()).await?;
        studio.close().await;
        Ok(())
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, GameServiceError> {
        let mut projects = if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            let registered = list_registered_projects(&studio).await?;
            studio.close().await;
            registered
                .into_iter()
                .map(|(project, _)| project)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let sessions = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        if projects.is_empty() {
            projects.extend(sessions.values().map(|session| session.project.clone()));
        } else {
            for project in &mut projects {
                if let Some(session) = sessions.get(project.id.as_str()) {
                    *project = session.project.clone();
                }
            }
        }
        projects.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(projects)
    }

    pub fn read_project(&self, project_id: &str) -> Result<Project, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        projects
            .get(project_id)
            .map(|session| session.project.clone())
            .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))
    }

    pub fn is_project_read_only(&self, project_id: &str) -> Result<bool, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        projects
            .get(project_id)
            .map(|session| session.read_only)
            .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))
    }

    pub fn execution_context(
        &self,
        conversation_id: &str,
    ) -> Result<(Project, Arc<ProjectStore>), GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = projects
            .values()
            .find(|session| session.conversations.contains_key(conversation_id))
            .ok_or_else(|| GameServiceError::ConversationNotFound(conversation_id.to_string()))?;
        require_writable(session)?;
        Ok((session.project.clone(), Arc::clone(&session.store)))
    }

    pub async fn complete_turn(
        &self,
        codex_turn_id: &str,
        status: TaskAttemptStatus,
    ) -> Result<Option<CompletedTaskAttempt>, GameServiceError> {
        let stores = self.writable_stores()?;
        for store in stores {
            if let Some(completion) = store.complete_turn(codex_turn_id, status).await? {
                return Ok(Some(CompletedTaskAttempt {
                    attempt_id: completion.attempt_id,
                    task_id: completion.task_id,
                    conversation_id: completion.conversation_id,
                    status,
                    artifacts: Vec::new(),
                    workflow: None,
                    conflict_count: None,
                }));
            }
        }
        Ok(None)
    }

    pub async fn complete_turn_output(
        &self,
        codex_turn_id: &str,
        output: Option<&str>,
    ) -> Result<Option<CompletedTaskAttempt>, GameServiceError> {
        for store in self.writable_stores()? {
            let Some(context) = store.turn_attempt_context(codex_turn_id).await? else {
                continue;
            };
            let current_workflow = self.read_focus(&context.conversation_id)?;
            if current_workflow.id.as_str() != context.workflow_id
                || current_workflow.input_version != context.input_version
                || current_workflow.workflow_version != context.workflow_version
            {
                return self
                    .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                    .await;
            }
            let (artifact_contents, mut next_workflow, conflict_count) = match context
                .stage
                .as_str()
            {
                "brief" if context.agent_code == super::BRIEF_AGENT => {
                    let Some(brief) = output.and_then(parse_structured_output::<StructuredBrief>)
                    else {
                        return self
                            .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                            .await;
                    };
                    let mut workflow = current_workflow.clone();
                    super::advance_workflow(
                        &mut workflow,
                        WorkflowCommand::SubmitClarification,
                        context.input_version,
                    )?;
                    (
                        vec![ArtifactContent::StructuredBrief(brief)],
                        Some(workflow),
                        None,
                    )
                }
                "review" if super::review_agent_codes().contains(&context.agent_code.as_str()) => {
                    let Some(review) = output.and_then(parse_structured_output::<ReviewReport>)
                    else {
                        return self
                            .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                            .await;
                    };
                    if review.agent_code != context.agent_code
                        || current_workflow.state != WorkflowState::Reviewing
                    {
                        return self
                            .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                            .await;
                    }
                    (vec![ArtifactContent::ReviewReport(review)], None, None)
                }
                "synthesis" if context.agent_code == super::SYNTHESIS_AGENT => {
                    let Some(result) = output.and_then(parse_structured_output::<SynthesisResult>)
                    else {
                        return self
                            .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                            .await;
                    };
                    if current_workflow.state != WorkflowState::Merging {
                        return self
                            .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                            .await;
                    }
                    let high_impact_conflicts = result
                        .conflicts
                        .conflicts
                        .iter()
                        .filter(|conflict| conflict.high_impact)
                        .count() as u64;
                    let mut workflow = current_workflow.clone();
                    super::advance_workflow(
                        &mut workflow,
                        WorkflowCommand::CompleteMerge,
                        context.input_version,
                    )?;
                    (
                        vec![
                            ArtifactContent::ArtBibleDraft(result.draft),
                            ArtifactContent::ConflictSet(result.conflicts),
                        ],
                        Some(workflow),
                        Some(high_impact_conflicts),
                    )
                }
                _ => {
                    return self
                        .complete_turn(codex_turn_id, TaskAttemptStatus::Succeeded)
                        .await;
                }
            };
            let artifacts = artifact_contents
                .into_iter()
                .map(|content| Artifact {
                    id: ArtifactId::new(Uuid::now_v7().to_string()),
                    input_version: context.input_version,
                    workflow_version: context.workflow_version,
                    content,
                    created_at: now(),
                })
                .collect::<Vec<_>>();
            let completion = match store
                .commit_turn_artifacts(
                    codex_turn_id,
                    &context.workflow_id,
                    &artifacts,
                    next_workflow.as_ref(),
                )
                .await
            {
                Ok(completion) => completion,
                Err(StoreError::NotFound(_)) => {
                    return self
                        .complete_turn(codex_turn_id, TaskAttemptStatus::Failed)
                        .await;
                }
                Err(error) => return Err(error.into()),
            };
            if context.stage == "review"
                && store
                    .count_committed_artifacts(&context.workflow_id, "review", "reviewReport")
                    .await?
                    == super::review_agent_codes().len() as u64
            {
                let expected_workflow_version = current_workflow.workflow_version;
                let mut workflow = current_workflow;
                super::advance_workflow(
                    &mut workflow,
                    WorkflowCommand::CompleteReviews,
                    context.input_version,
                )?;
                store
                    .update_workflow(&workflow, expected_workflow_version)
                    .await?;
                next_workflow = Some(workflow);
            }
            if let Some(workflow) = &next_workflow {
                let mut projects = self
                    .projects
                    .lock()
                    .map_err(|_| GameServiceError::StateUnavailable)?;
                let session =
                    find_conversation_session_mut(&mut projects, &context.conversation_id)?;
                session
                    .workflows
                    .insert(context.conversation_id.clone(), workflow.clone());
            }
            return Ok(Some(CompletedTaskAttempt {
                attempt_id: completion.attempt_id,
                task_id: completion.task_id,
                conversation_id: completion.conversation_id,
                status: TaskAttemptStatus::Succeeded,
                artifacts,
                workflow: next_workflow,
                conflict_count,
            }));
        }
        Ok(None)
    }

    fn writable_stores(&self) -> Result<Vec<Arc<ProjectStore>>, GameServiceError> {
        Ok(self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?
            .values()
            .filter(|session| !session.read_only)
            .map(|session| Arc::clone(&session.store))
            .collect())
    }

    pub async fn ensure_conversation(
        &self,
        project_id: &str,
        target_id: Option<String>,
    ) -> Result<Conversation, GameServiceError> {
        let (conversation, store) = {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .get_mut(project_id)
                .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
            if let Some(snapshot) = session
                .conversations
                .values()
                .find(|snapshot| snapshot.conversation.target_id == target_id)
            {
                return Ok(snapshot.conversation.clone());
            }
            require_writable(session)?;
            let conversation = Conversation {
                id: ConversationId::new(Uuid::now_v7().to_string()),
                project_id: ProjectId::new(project_id),
                target_id,
                created_at: now(),
            };
            session.conversations.insert(
                conversation.id.as_str().to_string(),
                ConversationSnapshot {
                    conversation: conversation.clone(),
                    messages: Vec::new(),
                },
            );
            (conversation, Arc::clone(&session.store))
        };
        store.insert_conversation(&conversation).await?;
        Ok(conversation)
    }

    pub async fn submit_message(
        &self,
        conversation_id: &str,
        content: String,
    ) -> Result<ConversationMessage, GameServiceError> {
        let (message, store) = {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = find_conversation_session_mut(&mut projects, conversation_id)?;
            require_writable(session)?;
            let message = ConversationMessage {
                id: Uuid::now_v7().to_string(),
                conversation_id: ConversationId::new(conversation_id),
                role: "user".to_string(),
                content,
                created_at: now(),
            };
            session
                .conversations
                .get_mut(conversation_id)
                .ok_or_else(|| GameServiceError::ConversationNotFound(conversation_id.to_string()))?
                .messages
                .push(message.clone());
            (message, Arc::clone(&session.store))
        };
        store.insert_message(&message).await?;
        Ok(message)
    }

    pub fn read_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationSnapshot, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        projects
            .values()
            .find_map(|session| session.conversations.get(conversation_id).cloned())
            .ok_or_else(|| GameServiceError::ConversationNotFound(conversation_id.to_string()))
    }

    pub async fn start_focus(
        &self,
        conversation_id: &str,
    ) -> Result<FocusWorkflow, GameServiceError> {
        let (workflow, store) = {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = find_conversation_session_mut(&mut projects, conversation_id)?;
            require_writable(session)?;
            if let Some(workflow) = session.workflows.get(conversation_id) {
                return Ok(workflow.clone());
            }
            let mut workflow = FocusWorkflow {
                id: FocusWorkflowId::new(Uuid::now_v7().to_string()),
                project_id: session.project.id.clone(),
                conversation_id: ConversationId::new(conversation_id),
                state: WorkflowState::Draft,
                input_version: 1,
                workflow_version: 1,
            };
            super::advance_workflow(&mut workflow, WorkflowCommand::StartFocus, 1)?;
            session.project.state = ProjectState::FocusInProgress;
            update_project_json(
                &Path::new(&session.project.root).join("project.json"),
                session.project.id.as_str(),
                &session.project.name,
                "focusInProgress",
            )?;
            session
                .workflows
                .insert(conversation_id.to_string(), workflow.clone());
            (workflow, Arc::clone(&session.store))
        };
        store.upsert_workflow(&workflow).await?;
        Ok(workflow)
    }

    pub fn read_focus(&self, conversation_id: &str) -> Result<FocusWorkflow, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        projects
            .values()
            .find_map(|session| session.workflows.get(conversation_id).cloned())
            .ok_or_else(|| GameServiceError::WorkflowNotFound(conversation_id.to_string()))
    }

    pub async fn read_focus_artifacts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<Artifact>, GameServiceError> {
        let (workflow_id, store) = {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .values()
                .find(|session| session.conversations.contains_key(conversation_id))
                .ok_or_else(|| {
                    GameServiceError::ConversationNotFound(conversation_id.to_string())
                })?;
            let workflow = session
                .workflows
                .get(conversation_id)
                .ok_or_else(|| GameServiceError::WorkflowNotFound(conversation_id.to_string()))?;
            (workflow.id.as_str().to_string(), Arc::clone(&session.store))
        };
        store
            .artifacts_for_workflow(&workflow_id, "")
            .await
            .map_err(Into::into)
    }

    pub async fn record_conflict_decision(
        &self,
        conversation_id: &str,
        expected_input_version: u64,
        decision: UserDecision,
    ) -> Result<(FocusWorkflow, Artifact), GameServiceError> {
        let (current_workflow, store) = {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .values()
                .find(|session| session.conversations.contains_key(conversation_id))
                .ok_or_else(|| {
                    GameServiceError::ConversationNotFound(conversation_id.to_string())
                })?;
            require_writable(session)?;
            let workflow = session
                .workflows
                .get(conversation_id)
                .cloned()
                .ok_or_else(|| GameServiceError::WorkflowNotFound(conversation_id.to_string()))?;
            (workflow, Arc::clone(&session.store))
        };
        let conflicts = store
            .artifacts_for_workflow(current_workflow.id.as_str(), "conflictSet")
            .await?
            .into_iter()
            .rev()
            .find_map(|artifact| match artifact.content {
                ArtifactContent::ConflictSet(conflicts) => Some(conflicts),
                _ => None,
            })
            .ok_or_else(|| {
                GameServiceError::InvalidDesignDecision(
                    "the workflow has no committed conflict set".to_string(),
                )
            })?;
        let conflict = conflicts
            .conflicts
            .iter()
            .find(|conflict| conflict.key == decision.conflict_key)
            .ok_or_else(|| {
                GameServiceError::InvalidDesignDecision(format!(
                    "unknown conflict key {}",
                    decision.conflict_key
                ))
            })?;
        if !conflict.options.contains(&decision.selected_option) {
            return Err(GameServiceError::InvalidDesignDecision(format!(
                "option is not valid for conflict {}",
                decision.conflict_key
            )));
        }
        let duplicate = store
            .artifacts_for_workflow(current_workflow.id.as_str(), "userDecision")
            .await?
            .into_iter()
            .any(|artifact| {
                matches!(
                    artifact.content,
                    ArtifactContent::UserDecision(existing)
                        if existing.conflict_key == decision.conflict_key
                )
            });
        if duplicate {
            return Err(GameServiceError::InvalidDesignDecision(format!(
                "conflict {} already has a decision",
                decision.conflict_key
            )));
        }
        let mut workflow = current_workflow.clone();
        super::advance_workflow(
            &mut workflow,
            WorkflowCommand::RecordConflictDecision,
            expected_input_version,
        )?;
        let artifact = Artifact {
            id: ArtifactId::new(Uuid::now_v7().to_string()),
            input_version: current_workflow.input_version,
            workflow_version: current_workflow.workflow_version,
            content: ArtifactContent::UserDecision(decision),
            created_at: now(),
        };
        store
            .commit_workflow_artifact(&workflow, current_workflow.workflow_version, &artifact)
            .await?;
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        find_conversation_session_mut(&mut projects, conversation_id)?
            .workflows
            .insert(conversation_id.to_string(), workflow.clone());
        Ok((workflow, artifact))
    }

    pub async fn advance_focus(
        &self,
        conversation_id: &str,
        command: WorkflowCommand,
        expected_input_version: u64,
        markdown: Option<String>,
    ) -> Result<(FocusWorkflow, Option<ArtBibleDocument>), GameServiceError> {
        let (current_workflow, project, store, next_art_bible_version) = {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .values()
                .find(|session| session.conversations.contains_key(conversation_id))
                .ok_or_else(|| {
                    GameServiceError::ConversationNotFound(conversation_id.to_string())
                })?;
            require_writable(session)?;
            let workflow = session
                .workflows
                .get(conversation_id)
                .cloned()
                .ok_or_else(|| GameServiceError::WorkflowNotFound(conversation_id.to_string()))?;
            (
                workflow,
                session.project.clone(),
                Arc::clone(&session.store),
                session.art_bibles.len() as u64 + 1,
            )
        };
        let mut workflow = current_workflow.clone();
        super::advance_workflow(&mut workflow, command, expected_input_version)?;
        let document = if command == WorkflowCommand::ConfirmArtBible {
            let draft = store
                .artifacts_for_workflow(current_workflow.id.as_str(), "artBibleDraft")
                .await?
                .into_iter()
                .rev()
                .find_map(|artifact| match artifact.content {
                    ArtifactContent::ArtBibleDraft(draft) => Some(draft),
                    _ => None,
                })
                .ok_or_else(|| {
                    GameServiceError::InvalidDesignDecision(
                        "the workflow has no committed Art Bible draft".to_string(),
                    )
                })?;
            if markdown
                .as_deref()
                .is_some_and(|value| value != draft.markdown)
            {
                return Err(GameServiceError::InvalidDesignDecision(
                    "confirmation content does not match the committed Art Bible draft".to_string(),
                ));
            }
            let markdown = draft.markdown;
            let conflicts = store
                .artifacts_for_workflow(current_workflow.id.as_str(), "conflictSet")
                .await?;
            let decisions = store
                .artifacts_for_workflow(current_workflow.id.as_str(), "userDecision")
                .await?;
            let decided_keys = decisions
                .iter()
                .filter_map(|artifact| match &artifact.content {
                    ArtifactContent::UserDecision(decision) => Some(decision.conflict_key.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if let Some(unresolved) =
                conflicts
                    .iter()
                    .find_map(|artifact| match &artifact.content {
                        ArtifactContent::ConflictSet(conflicts) => conflicts
                            .conflicts
                            .iter()
                            .find(|conflict| {
                                conflict.high_impact
                                    && !decided_keys.contains(&conflict.key.as_str())
                            })
                            .map(|conflict| conflict.key.clone()),
                        _ => None,
                    })
            {
                return Err(GameServiceError::InvalidDesignDecision(format!(
                    "high-impact conflict {unresolved} has not been resolved"
                )));
            }
            let mut source_artifact_ids = Vec::new();
            for artifact_type in [
                "structuredBrief",
                "reviewReport",
                "conflictSet",
                "artBibleDraft",
                "userDecision",
            ] {
                source_artifact_ids.extend(
                    store
                        .artifacts_for_workflow(current_workflow.id.as_str(), artifact_type)
                        .await?
                        .into_iter()
                        .map(|artifact| artifact.id),
                );
            }
            let version = ArtBibleVersion {
                id: ArtBibleVersionId::new(Uuid::now_v7().to_string()),
                project_id: project.id.clone(),
                version: next_art_bible_version,
                content_hash: format!("{:x}", Sha256::digest(markdown.as_bytes())),
                source_artifact_ids,
                created_at: now(),
            };
            Some(ArtBibleDocument { version, markdown })
        } else {
            None
        };
        let project_json_path = Path::new(&project.root).join("project.json");
        if command == WorkflowCommand::VersionArtBible {
            update_project_json(
                &project_json_path,
                project.id.as_str(),
                &project.name,
                "versioned",
            )?;
        }
        if let Some(document) = &document {
            let art_bible_path = Path::new(&project.root).join("art-bible.md");
            let previous_markdown = fs::read_to_string(&art_bible_path).ok();
            write_art_bible(&art_bible_path, &document.markdown)?;
            if let Err(error) = store
                .commit_art_bible_version(
                    &workflow,
                    current_workflow.workflow_version,
                    &document.version,
                    &document.markdown,
                )
                .await
            {
                if let Some(previous_markdown) = previous_markdown {
                    let _ = write_art_bible(&art_bible_path, &previous_markdown);
                } else {
                    let _ = fs::remove_file(&art_bible_path);
                }
                return Err(error.into());
            }
        } else if let Err(error) = store
            .update_workflow(&workflow, current_workflow.workflow_version)
            .await
        {
            if command == WorkflowCommand::VersionArtBible {
                let _ = update_project_json(
                    &project_json_path,
                    project.id.as_str(),
                    &project.name,
                    project_state_name(project.state),
                );
            }
            return Err(error.into());
        }
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = find_conversation_session_mut(&mut projects, conversation_id)?;
        session
            .workflows
            .insert(conversation_id.to_string(), workflow.clone());
        if command == WorkflowCommand::VersionArtBible {
            session.project.state = ProjectState::Versioned;
        }
        if let Some(document) = &document {
            session.art_bibles.push(document.clone());
        }
        Ok((workflow, document))
    }

    pub fn list_art_bibles(
        &self,
        project_id: &str,
    ) -> Result<Vec<ArtBibleVersion>, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = projects
            .get(project_id)
            .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
        Ok(session
            .art_bibles
            .iter()
            .map(|document| document.version.clone())
            .collect())
    }

    pub fn read_art_bible(
        &self,
        project_id: &str,
        version: u64,
    ) -> Result<ArtBibleDocument, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        projects
            .get(project_id)
            .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?
            .art_bibles
            .iter()
            .find(|document| document.version.version == version)
            .cloned()
            .ok_or_else(|| GameServiceError::ProjectNotFound(format!("{project_id}@{version}")))
    }
}

impl ProjectSession {
    fn new(project: Project, read_only: bool, store: Arc<ProjectStore>) -> Self {
        Self {
            project,
            read_only,
            store,
            conversations: HashMap::new(),
            workflows: HashMap::new(),
            art_bibles: Vec::new(),
        }
    }
}

fn find_conversation_session_mut<'a>(
    projects: &'a mut HashMap<String, ProjectSession>,
    conversation_id: &str,
) -> Result<&'a mut ProjectSession, GameServiceError> {
    projects
        .values_mut()
        .find(|session| session.conversations.contains_key(conversation_id))
        .ok_or_else(|| GameServiceError::ConversationNotFound(conversation_id.to_string()))
}

fn require_writable(session: &ProjectSession) -> Result<(), GameServiceError> {
    if session.read_only {
        Err(GameServiceError::ReadOnly(
            session.project.id.as_str().to_string(),
        ))
    } else {
        Ok(())
    }
}

fn recovered_project_state(
    workflows: &[FocusWorkflow],
    art_bibles: &[(ArtBibleVersion, String)],
) -> ProjectState {
    if workflows
        .iter()
        .any(|workflow| workflow.state == WorkflowState::Versioned)
        || (!art_bibles.is_empty()
            && workflows
                .iter()
                .all(|workflow| workflow.state == WorkflowState::Cancelled))
    {
        ProjectState::Versioned
    } else if workflows
        .iter()
        .any(|workflow| workflow.state != WorkflowState::Cancelled)
    {
        ProjectState::FocusInProgress
    } else {
        ProjectState::Unversioned
    }
}

fn project_state_name(state: ProjectState) -> &'static str {
    match state {
        ProjectState::Unversioned => "unversioned",
        ProjectState::FocusInProgress => "focusInProgress",
        ProjectState::Versioned => "versioned",
    }
}

fn parse_structured_output<T: serde::de::DeserializeOwned>(output: &str) -> Option<T> {
    let trimmed = output.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    serde_json::from_str(json).ok()
}

fn absolute_root(root: &str) -> Result<PathBuf, GameServiceError> {
    let path = PathBuf::from(root);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(GameServiceError::InvalidProjectPath(root.to_string()))
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Capability;
    use crate::CodexExecutionPort;
    use crate::ExecuteTaskRequest;
    use crate::ExecutionError;
    use crate::StartThreadRequest;
    use crate::StartTurnRequest;
    use crate::StartedThread;
    use crate::StartedTurn;
    use crate::SteerTurnRequest;
    use crate::TaskOrchestrator;
    use codex_game_domain::ArtBibleDraft;
    use codex_game_domain::Conflict;
    use codex_game_domain::ConflictSet;
    use codex_game_domain::ContextPackage;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use tempfile::tempdir;

    struct FakeExecution;

    impl CodexExecutionPort for FakeExecution {
        async fn start_thread(
            &self,
            _request: StartThreadRequest,
        ) -> Result<StartedThread, ExecutionError> {
            Ok(StartedThread {
                thread_id: "thread-1".to_string(),
                session_id: "session-1".to_string(),
            })
        }

        async fn thread_available(&self, _thread_id: &str) -> bool {
            true
        }

        async fn start_turn(
            &self,
            _request: StartTurnRequest,
        ) -> Result<StartedTurn, ExecutionError> {
            Ok(StartedTurn {
                turn_id: "turn-1".to_string(),
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

    #[derive(Default)]
    struct CountingExecution {
        threads: AtomicUsize,
        turns: AtomicUsize,
    }

    impl CodexExecutionPort for CountingExecution {
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

    #[tokio::test]
    async fn commits_valid_brief_only_after_turn_completion() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("game");
        let service = GameService::new(directory.path().join("app"));
        let project = service
            .create_project(
                Uuid::now_v7().to_string(),
                "Test Game".to_string(),
                root.to_string_lossy().into_owned(),
            )
            .await
            .expect("create project");
        let conversation = service
            .ensure_conversation(project.id.as_str(), None)
            .await
            .expect("ensure conversation");
        let workflow = service
            .start_focus(conversation.id.as_str())
            .await
            .expect("start focus");
        let (_, store) = service
            .execution_context(conversation.id.as_str())
            .expect("execution context");
        TaskOrchestrator::default()
            .execute(
                &FakeExecution,
                store.as_ref(),
                ExecuteTaskRequest {
                    project_root: root.to_string_lossy().into_owned(),
                    conversation_id: conversation.id.as_str().to_string(),
                    target_id: workflow.id.as_str().to_string(),
                    stage: "brief".to_string(),
                    agent_code: super::super::BRIEF_AGENT.to_string(),
                    idempotency_key: "message-1".to_string(),
                    prompt: "design a game".to_string(),
                    context: ContextPackage {
                        brief_artifact_id: ArtifactId::new("pending-brief"),
                        confirmed_decisions: Vec::new(),
                        artifact_summaries: Vec::new(),
                        context_version: workflow.input_version,
                        workflow_version: workflow.workflow_version,
                        agent_definition_version: "1".to_string(),
                        output_schema: "{}".to_string(),
                    },
                    capability: Capability::TextStructuredOutput,
                },
            )
            .await
            .expect("execute task");
        let completion = service
            .complete_turn_output(
                "turn-1",
                Some(
                    r#"{"coreExperience":"explore","themeAndMood":"quiet","targetPlayers":"solo","playerPerspective":"top-down","gameplayPillars":["discover"],"openQuestions":[]}"#,
                ),
            )
            .await
            .expect("complete turn")
            .expect("known turn");
        assert_eq!(completion.status, TaskAttemptStatus::Succeeded);
        assert!(matches!(
            completion
                .artifacts
                .first()
                .map(|artifact| &artifact.content),
            Some(ArtifactContent::StructuredBrief(_))
        ));
        assert_eq!(
            service
                .read_focus(conversation.id.as_str())
                .expect("workflow")
                .state,
            WorkflowState::BriefReady
        );
    }

    #[tokio::test]
    async fn completes_a_confirmed_versioned_art_bible() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("game");
        let service = GameService::new(directory.path().join("app"));
        let project = service
            .create_project(
                Uuid::now_v7().to_string(),
                "Test Game".to_string(),
                root.to_string_lossy().into_owned(),
            )
            .await
            .expect("create project");
        assert_eq!(
            service.list_projects().await.expect("list projects"),
            vec![project.clone()]
        );
        let conversation = service
            .ensure_conversation(project.id.as_str(), None)
            .await
            .expect("ensure conversation");
        service
            .submit_message(
                conversation.id.as_str(),
                "A quiet strategy game".to_string(),
            )
            .await
            .expect("submit");
        let mut workflow = service
            .start_focus(conversation.id.as_str())
            .await
            .expect("start focus");
        for command in [
            WorkflowCommand::SubmitClarification,
            WorkflowCommand::AcceptBrief,
            WorkflowCommand::CompleteReviews,
            WorkflowCommand::CompleteMerge,
        ] {
            (workflow, _) = service
                .advance_focus(conversation.id.as_str(), command, 1, None)
                .await
                .expect("advance");
        }
        let (_, store) = service
            .execution_context(conversation.id.as_str())
            .expect("execution context");
        let draft = Artifact {
            id: ArtifactId::new("draft-1"),
            input_version: workflow.input_version,
            workflow_version: workflow.workflow_version,
            content: ArtifactContent::ArtBibleDraft(ArtBibleDraft {
                markdown: "# Art Bible\n".to_string(),
                unresolved_assumptions: Vec::new(),
            }),
            created_at: now(),
        };
        store
            .commit_workflow_artifact(&workflow, workflow.workflow_version, &draft)
            .await
            .expect("commit draft");
        let conflicts = Artifact {
            id: ArtifactId::new("conflicts-1"),
            input_version: workflow.input_version,
            workflow_version: workflow.workflow_version,
            content: ArtifactContent::ConflictSet(ConflictSet {
                conflicts: vec![Conflict {
                    key: "camera".to_string(),
                    description: "Choose a camera".to_string(),
                    options: vec!["top-down".to_string(), "side-view".to_string()],
                    high_impact: true,
                }],
            }),
            created_at: now(),
        };
        store
            .commit_workflow_artifact(&workflow, workflow.workflow_version, &conflicts)
            .await
            .expect("commit conflicts");
        let unresolved = service
            .advance_focus(
                conversation.id.as_str(),
                WorkflowCommand::ConfirmArtBible,
                1,
                Some("# Art Bible\n".to_string()),
            )
            .await;
        assert!(matches!(
            unresolved,
            Err(GameServiceError::InvalidDesignDecision(_))
        ));
        (workflow, _) = service
            .record_conflict_decision(
                conversation.id.as_str(),
                1,
                UserDecision {
                    conflict_key: "camera".to_string(),
                    selected_option: "top-down".to_string(),
                    note: None,
                },
            )
            .await
            .expect("record conflict decision");
        assert_eq!(workflow.state, WorkflowState::UserReview);
        let mismatch = service
            .advance_focus(
                conversation.id.as_str(),
                WorkflowCommand::ConfirmArtBible,
                1,
                Some("# Replaced by client\n".to_string()),
            )
            .await;
        assert!(matches!(
            mismatch,
            Err(GameServiceError::InvalidDesignDecision(_))
        ));
        let (_, document) = service
            .advance_focus(
                conversation.id.as_str(),
                WorkflowCommand::ConfirmArtBible,
                1,
                None,
            )
            .await
            .expect("confirm");
        let document = document.expect("version");
        assert_eq!(document.version.version, 1);
        assert_eq!(document.version.source_artifact_ids.len(), 3);
        service
            .advance_focus(
                conversation.id.as_str(),
                WorkflowCommand::VersionArtBible,
                1,
                None,
            )
            .await
            .expect("version");
        assert_eq!(
            fs::read_to_string(root.join("art-bible.md")).expect("read art bible"),
            "# Art Bible\n"
        );
        assert_eq!(workflow.state, WorkflowState::UserReview);
        assert_eq!(
            service
                .read_project(project.id.as_str())
                .expect("project")
                .state,
            ProjectState::Versioned
        );

        update_project_json(
            &root.join("project.json"),
            project.id.as_str(),
            &project.name,
            "focusInProgress",
        )
        .expect("make project metadata stale");
        write_art_bible(&root.join("art-bible.md"), "stale\n").expect("make art bible stale");
        drop(store);
        drop(service);

        let reopened = GameService::new(directory.path().join("app"));
        let reopened_project = reopened
            .open_project(root.to_string_lossy().into_owned(), false)
            .await
            .expect("reopen project");
        assert_eq!(reopened_project.state, ProjectState::Versioned);
        assert_eq!(
            fs::read_to_string(root.join("art-bible.md")).expect("read repaired art bible"),
            "# Art Bible\n"
        );
        let metadata: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join("project.json")).expect("read repaired metadata"),
        )
        .expect("parse repaired metadata");
        assert_eq!(metadata["state"], "versioned");
    }

    #[tokio::test]
    async fn drives_reviews_and_synthesis_through_turn_completion() {
        fn task_request(
            root: &str,
            conversation_id: &str,
            workflow: &FocusWorkflow,
            stage: &str,
            agent_code: &str,
            key: &str,
        ) -> ExecuteTaskRequest {
            ExecuteTaskRequest {
                project_root: root.to_string(),
                conversation_id: conversation_id.to_string(),
                target_id: workflow.id.as_str().to_string(),
                stage: stage.to_string(),
                agent_code: agent_code.to_string(),
                idempotency_key: key.to_string(),
                prompt: "drive focus".to_string(),
                context: ContextPackage {
                    brief_artifact_id: ArtifactId::new(Uuid::now_v7().to_string()),
                    confirmed_decisions: Vec::new(),
                    artifact_summaries: Vec::new(),
                    context_version: workflow.input_version,
                    workflow_version: workflow.workflow_version,
                    agent_definition_version: "1".to_string(),
                    output_schema: "{}".to_string(),
                },
                capability: Capability::TextStructuredOutput,
            }
        }

        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("game");
        let root_string = root.to_string_lossy().into_owned();
        let service = GameService::new(directory.path().join("app"));
        let project = service
            .create_project(
                Uuid::now_v7().to_string(),
                "Test Game".to_string(),
                root_string.clone(),
            )
            .await
            .expect("create project");
        let conversation = service
            .ensure_conversation(project.id.as_str(), None)
            .await
            .expect("ensure conversation");
        let conversation_id = conversation.id.as_str().to_string();
        service
            .submit_message(&conversation_id, "A quiet exploration game".to_string())
            .await
            .expect("submit");
        let workflow = service
            .start_focus(&conversation_id)
            .await
            .expect("start focus");
        let input_version = workflow.input_version;
        let (_, store) = service
            .execution_context(&conversation_id)
            .expect("execution context");
        let orchestrator = TaskOrchestrator::default();
        let execution = CountingExecution::default();

        // Brief turn completion advances Clarifying -> BriefReady.
        let brief = orchestrator
            .execute(
                &execution,
                store.as_ref(),
                task_request(
                    &root_string,
                    &conversation_id,
                    &workflow,
                    "brief",
                    super::super::BRIEF_AGENT,
                    "brief-1",
                ),
            )
            .await
            .expect("brief task");
        service
            .complete_turn_output(
                brief.attempt.codex_turn_id.as_deref().expect("brief turn"),
                Some(
                    r#"{"coreExperience":"explore","themeAndMood":"quiet","targetPlayers":"solo","playerPerspective":"topDown","gameplayPillars":["discover"],"openQuestions":[]}"#,
                ),
            )
            .await
            .expect("complete brief")
            .expect("known brief turn");
        assert_eq!(
            service.read_focus(&conversation_id).expect("focus").state,
            WorkflowState::BriefReady
        );

        service
            .advance_focus(
                &conversation_id,
                WorkflowCommand::AcceptBrief,
                input_version,
                None,
            )
            .await
            .expect("accept brief");
        assert_eq!(
            service.read_focus(&conversation_id).expect("focus").state,
            WorkflowState::Reviewing
        );

        // Completing all three review turns auto-advances Reviewing -> Merging.
        for (index, agent_code) in super::super::review_agent_codes().into_iter().enumerate() {
            let reviewing = service.read_focus(&conversation_id).expect("focus");
            let review = orchestrator
                .execute(
                    &execution,
                    store.as_ref(),
                    task_request(
                        &root_string,
                        &conversation_id,
                        &reviewing,
                        "review",
                        agent_code,
                        &format!("review-{index}"),
                    ),
                )
                .await
                .expect("review task");
            let output = format!(
                r#"{{"agentCode":"{agent_code}","findings":["ok"],"risks":[],"recommendations":[]}}"#
            );
            service
                .complete_turn_output(
                    review
                        .attempt
                        .codex_turn_id
                        .as_deref()
                        .expect("review turn"),
                    Some(&output),
                )
                .await
                .expect("complete review")
                .expect("known review turn");
        }
        assert_eq!(
            service.read_focus(&conversation_id).expect("focus").state,
            WorkflowState::Merging
        );

        // Synthesis turn completion produces the draft plus conflict set.
        let merging = service.read_focus(&conversation_id).expect("focus");
        let synthesis = orchestrator
            .execute(
                &execution,
                store.as_ref(),
                task_request(
                    &root_string,
                    &conversation_id,
                    &merging,
                    "synthesis",
                    super::super::SYNTHESIS_AGENT,
                    "synthesis-1",
                ),
            )
            .await
            .expect("synthesis task");
        let completion = service
            .complete_turn_output(
                synthesis
                    .attempt
                    .codex_turn_id
                    .as_deref()
                    .expect("synthesis turn"),
                Some(
                    r##"{"draft":{"markdown":"# Synth Art Bible\n","unresolvedAssumptions":[]},"conflicts":{"conflicts":[{"key":"camera","description":"Choose a camera","options":["topDown","sideView"],"highImpact":true}]}}"##,
                ),
            )
            .await
            .expect("complete synthesis")
            .expect("known synthesis turn");
        assert_eq!(completion.conflict_count, Some(1));
        assert_eq!(
            service.read_focus(&conversation_id).expect("focus").state,
            WorkflowState::UserReview
        );

        // The high-impact conflict must be resolved before confirmation.
        let unresolved = service
            .advance_focus(
                &conversation_id,
                WorkflowCommand::ConfirmArtBible,
                input_version,
                None,
            )
            .await;
        assert!(matches!(
            unresolved,
            Err(GameServiceError::InvalidDesignDecision(_))
        ));

        service
            .record_conflict_decision(
                &conversation_id,
                input_version,
                UserDecision {
                    conflict_key: "camera".to_string(),
                    selected_option: "topDown".to_string(),
                    note: None,
                },
            )
            .await
            .expect("record conflict decision");
        let (_, document) = service
            .advance_focus(
                &conversation_id,
                WorkflowCommand::ConfirmArtBible,
                input_version,
                None,
            )
            .await
            .expect("confirm");
        assert_eq!(document.expect("art bible version").version.version, 1);
        service
            .advance_focus(
                &conversation_id,
                WorkflowCommand::VersionArtBible,
                input_version,
                None,
            )
            .await
            .expect("version");

        // The confirmed art bible is versioned and readable end-to-end.
        assert_eq!(
            service
                .list_art_bibles(project.id.as_str())
                .expect("versions")
                .len(),
            1
        );
        let stored = service
            .read_art_bible(project.id.as_str(), 1)
            .expect("read art bible");
        assert_eq!(stored.markdown, "# Synth Art Bible\n");
        assert_eq!(
            service
                .read_project(project.id.as_str())
                .expect("project")
                .state,
            ProjectState::Versioned
        );
        assert_eq!(
            fs::read_to_string(root.join("art-bible.md")).expect("read art bible file"),
            "# Synth Art Bible\n"
        );
    }
}
