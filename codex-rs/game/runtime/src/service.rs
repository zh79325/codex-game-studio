use crate::character_files::read_project_characters;
use crate::character_files::write_character_file;
use codex_game_domain::AgentAction;
use codex_game_domain::AgentActionKind;
use codex_game_domain::AgentActionPayload;
use codex_game_domain::AgentHandoff;
use codex_game_domain::AgentVerdict;
use codex_game_domain::ArtBibleVersion;
use codex_game_domain::ArtBibleVersionId;
use codex_game_domain::ArtifactDraftRecord;
use codex_game_domain::Character;
use codex_game_domain::CharacterState;
use codex_game_domain::ContextPackage;
use codex_game_domain::Conversation;
use codex_game_domain::ConversationId;
use codex_game_domain::ConversationMemory;
use codex_game_domain::ConversationMessage;
use codex_game_domain::ConversationStatus;
use codex_game_domain::ConversationTargetKind;
use codex_game_domain::Generation;
use codex_game_domain::MAX_HANDOFFS;
use codex_game_domain::MessageStatus;
use codex_game_domain::Project;
use codex_game_domain::ProjectId;
use codex_game_domain::ProjectMemory;
use codex_game_domain::ProjectState;
use codex_game_domain::ReviewSubject;
use codex_game_domain::Task;
use codex_game_domain::TaskAttemptStatus;
use codex_game_domain::TaskStatus;
use codex_game_domain::WorkflowContext;
use codex_game_domain::WorkflowVerdictSummary;
use codex_game_store::ProjectAccess;
use codex_game_store::ProjectStore;
use codex_game_store::StoreError;
use codex_game_store::finalize_project_json;
use codex_game_store::list_registered_projects;
use codex_game_store::open_studio_store;
use codex_game_store::register_project as register_studio_project;
use codex_game_store::unregister_project;
use codex_game_store::unregister_project_by_root;
use codex_game_store::update_project_json;
use codex_game_store::write_art_bible;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
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

const CONVERSATION_ALREADY_RUNNING: &str = "该会话已有一轮正在运行";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtBibleDocument {
    pub version: ArtBibleVersion,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSnapshot {
    pub conversation: Conversation,
    pub messages: Vec<ConversationMessage>,
    pub drafts: Vec<ArtifactDraftRecord>,
    pub memories: Vec<ConversationMemory>,
    pub handoffs: Vec<AgentHandoff>,
}

#[derive(Debug, Clone)]
pub struct PreparedConversationTurn {
    pub user_message: ConversationMessage,
    pub assistant_message: ConversationMessage,
    pub conversation: Conversation,
    pub agent_code: String,
    pub stage: String,
    pub project: Project,
    pub store: Arc<ProjectStore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirState {
    pub root: String,
    pub occupied: bool,
    pub project_id: Option<String>,
    pub supported: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListedCharacter {
    pub character: Character,
    pub model_file_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterWorkflowStep {
    pub key: String,
    pub label: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterWorkflowProgress {
    pub status_label: String,
    pub steps: Vec<CharacterWorkflowStep>,
    pub needs_resume: bool,
    pub continuation_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecReviewStatus {
    AwaitingDraft,
    Pending,
    Approved,
    Concerns,
    Rejected,
    Error,
}

impl SpecReviewStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingDraft => "awaiting_draft",
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Concerns => "concerns",
            Self::Rejected => "rejected",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
struct CharacterWorkflowFacts {
    pending_spec: Option<ArtifactDraftRecord>,
    spec_review_status: SpecReviewStatus,
    latest_spec_verdict: Option<AgentVerdict>,
    latest_spec_rejection_id: Option<String>,
    render_rejection_id: Option<String>,
    views_rejection_id: Option<String>,
    render_rejected_after_generation: bool,
    views_rejected_after_generation: bool,
    failed_design_stage: Option<String>,
    has_running_task: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedTaskAttempt {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub status: TaskAttemptStatus,
    pub agent_code: Option<String>,
    pub action: Option<AgentAction>,
}

#[derive(Debug, Clone)]
pub struct ActionProtocolViolation {
    pub codex_turn_id: String,
    pub assistant_message_id: String,
    pub message: String,
    pub context: codex_game_store::TurnAttemptContext,
    pub store: Arc<ProjectStore>,
}

#[derive(Debug)]
pub enum TurnOutputCompletion {
    Completed(Option<CompletedTaskAttempt>),
    ActionProtocolViolation(ActionProtocolViolation),
}

#[derive(Debug, Error)]
pub enum GameServiceError {
    #[error("game runtime state is unavailable")]
    StateUnavailable,
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("conversation not found: {0}")]
    ConversationNotFound(String),
    #[error("project is read-only: {0}")]
    ReadOnly(String),
    #[error("invalid project path: {0}")]
    InvalidProjectPath(String),
    #[error("invalid action output: {0}")]
    InvalidAction(String),
    #[error("project gate is not satisfied: {0}")]
    ProjectGate(String),
    #[error("character not found: {0}")]
    CharacterNotFound(String),
    #[error("invalid character operation: {0}")]
    InvalidCharacterOperation(String),
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
        if root_path.join("project.json").exists() {
            return Err(GameServiceError::InvalidProjectPath(
                "目录已存在 project.json，请直接打开该项目".to_string(),
            ));
        }
        let project = Project {
            id: ProjectId::new(project_id),
            name,
            code: None,
            root: root_path.to_string_lossy().into_owned(),
            state: ProjectState::Drafting,
        };
        update_project_json(
            &root_path.join("project.json"),
            project.id.as_str(),
            &project.name,
            "drafting",
        )?;
        fs::create_dir_all(root_path.join(".codex-game"))?;
        let store = Arc::new(ProjectStore::open(&root_path).await?);
        if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            unregister_project_by_root(&studio, &project.root).await?;
            studio.close().await;
        }
        self.register_project(&project, store.access()).await?;
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        projects.retain(|_, session| session.project.root != project.root);
        projects.insert(
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
        if value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            != Some(2)
        {
            return Err(GameServiceError::InvalidProjectPath(
                "不支持旧项目，请新建项目".to_string(),
            ));
        }
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
            Some("drafting") => ProjectState::Drafting,
            Some("styleSettled") => ProjectState::StyleSettled,
            Some("ready") => ProjectState::Ready,
            _ => {
                return Err(GameServiceError::InvalidProjectPath(
                    "不支持旧项目，请新建项目".to_string(),
                ));
            }
        };
        let project = Project {
            id: ProjectId::new(id),
            name: name.to_string(),
            code: value
                .get("code")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
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
        let art_bibles = store.load_art_bible_versions(project.id.as_str()).await?;
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
                        drafts: Vec::new(),
                        memories: Vec::new(),
                        handoffs: Vec::new(),
                    },
                )
            })
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

    pub async fn remove_project(&self, project_id: &str) -> Result<(), GameServiceError> {
        if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            unregister_project(&studio, project_id).await?;
            studio.close().await;
        }
        self.projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?
            .remove(project_id);
        Ok(())
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

    pub async fn turn_attempt_context(
        &self,
        codex_turn_id: &str,
    ) -> Result<Option<codex_game_store::TurnAttemptContext>, GameServiceError> {
        for store in self.writable_stores()? {
            if let Some(context) = store.turn_attempt_context(codex_turn_id).await? {
                return Ok(Some(context));
            }
        }
        Ok(None)
    }

    pub async fn turn_audit_context(
        &self,
        codex_turn_id: &str,
    ) -> Result<Option<crate::TurnAuditContext>, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?
            .values()
            .filter(|session| !session.read_only)
            .map(|session| (session.project.clone(), Arc::clone(&session.store)))
            .collect::<Vec<_>>();
        for (project, store) in projects {
            let Some(context) = store.turn_attempt_context(codex_turn_id).await? else {
                continue;
            };
            let (target, target_dir) = if context.target_id == project.id.as_str() {
                ("project".to_string(), PathBuf::from(&project.root))
            } else {
                let character = self
                    .read_character(project.id.as_str(), &context.target_id)
                    .await?;
                (
                    format!("character:{}", character.id),
                    Path::new(&project.root).join(character.dir_name),
                )
            };
            return Ok(Some(crate::TurnAuditContext {
                project_root: PathBuf::from(project.root),
                target_dir,
                conversation_id: context.conversation_id,
                turn: context.turn,
                target,
                agent_code: context.agent_code,
                attempt_id: context.attempt_id,
            }));
        }
        Ok(None)
    }

    pub async fn complete_turn(
        &self,
        codex_turn_id: &str,
        status: TaskAttemptStatus,
    ) -> Result<Option<CompletedTaskAttempt>, GameServiceError> {
        let stores = self.writable_stores()?;
        for store in stores {
            let agent_code = store
                .turn_attempt_context(codex_turn_id)
                .await?
                .map(|context| context.agent_code);
            if let Some(completion) = store.complete_turn(codex_turn_id, status).await? {
                let message_status = match status {
                    TaskAttemptStatus::Cancelled | TaskAttemptStatus::Interrupted => {
                        MessageStatus::Interrupted
                    }
                    TaskAttemptStatus::Failed | TaskAttemptStatus::Unknown => MessageStatus::Failed,
                    _ => MessageStatus::Completed,
                };
                let fallback = match message_status {
                    MessageStatus::Interrupted => "运行已中断".to_string(),
                    MessageStatus::Failed => "Agent 执行失败，可重试本轮。".to_string(),
                    _ => String::new(),
                };
                self.apply_latest_running_message(
                    &completion.conversation_id,
                    message_status,
                    fallback,
                )?;
                return Ok(Some(CompletedTaskAttempt {
                    attempt_id: completion.attempt_id,
                    task_id: completion.task_id,
                    conversation_id: completion.conversation_id,
                    status,
                    agent_code,
                    action: None,
                }));
            }
        }
        Ok(None)
    }

    pub async fn complete_turn_output(
        &self,
        codex_turn_id: &str,
        output: Option<&str>,
    ) -> Result<TurnOutputCompletion, GameServiceError> {
        for store in self.writable_stores()? {
            let Some(context) = store.turn_attempt_context(codex_turn_id).await? else {
                continue;
            };
            let (assistant_message_id, director_agent, allowed_handoffs) = {
                let projects = self
                    .projects
                    .lock()
                    .map_err(|_| GameServiceError::StateUnavailable)?;
                let session = projects
                    .values()
                    .find(|session| session.conversations.contains_key(&context.conversation_id))
                    .ok_or_else(|| {
                        GameServiceError::ConversationNotFound(context.conversation_id.clone())
                    })?;
                let snapshot = &session.conversations[&context.conversation_id];
                let assistant = snapshot
                    .messages
                    .iter()
                    .rev()
                    .find(|message| {
                        message.role == "assistant"
                            && message.agent_code == context.agent_code
                            && message.status == MessageStatus::Thinking
                    })
                    .ok_or_else(|| {
                        GameServiceError::InvalidAction("找不到本轮待完成的 Agent 消息".to_string())
                    })?;
                let director = snapshot.conversation.director_agent_code.clone();
                let allowed = context.context.allowed_handoffs.clone();
                (assistant.id.clone(), director, allowed)
            };
            let parsed = output
                .ok_or_else(|| "Agent 未返回输出".to_string())
                .and_then(|value| {
                    super::parse_agent_turn(
                        value,
                        &context.agent_code,
                        &director_agent,
                        &allowed_handoffs,
                    )
                    .map_err(|error| error.to_string())
                });
            let parsed = match parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    return Ok(TurnOutputCompletion::ActionProtocolViolation(
                        ActionProtocolViolation {
                            codex_turn_id: codex_turn_id.to_string(),
                            assistant_message_id,
                            message: format!("Action 协议校验失败：{error}"),
                            context,
                            store,
                        },
                    ));
                }
            };
            if context.agent_code == "spec_reviewer"
                && parsed.action.action == AgentActionKind::Handoff
            {
                let expected_subject = context.context.review_subject.as_ref().map(|item| &item.id);
                let verdict = parsed.action.payload.verdict.as_ref();
                if expected_subject.is_none()
                    || verdict.is_none_or(|verdict| {
                        verdict.token != "SPEC-CHECK"
                            || Some(&verdict.subject_id) != expected_subject
                    })
                {
                    return Ok(TurnOutputCompletion::ActionProtocolViolation(
                        ActionProtocolViolation {
                            codex_turn_id: codex_turn_id.to_string(),
                            assistant_message_id,
                            message:
                                "Action 协议校验失败：SPEC-CHECK subject_id 必须匹配当前待审草稿"
                                    .to_string(),
                            context,
                            store,
                        },
                    ));
                }
            }
            let (generations, updated_character) = match self
                .prepare_action_generations(&context, &parsed.action)
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    let message = format!("Action 产物校验失败：{error}");
                    let completion = store
                        .fail_action_turn(
                            codex_turn_id,
                            &assistant_message_id,
                            &message,
                            MessageStatus::Failed,
                            now(),
                        )
                        .await?;
                    self.apply_failed_message(
                        &context.conversation_id,
                        &assistant_message_id,
                        message,
                    )?;
                    return Ok(TurnOutputCompletion::Completed(completion.map(
                        |completion| CompletedTaskAttempt {
                            attempt_id: completion.attempt_id,
                            task_id: completion.task_id,
                            conversation_id: completion.conversation_id,
                            status: TaskAttemptStatus::Failed,
                            agent_code: Some(context.agent_code),
                            action: None,
                        },
                    )));
                }
            };
            let meta_backup = if let Some(character) = updated_character.as_ref() {
                let project = self.read_project(&character.project_id)?;
                let path = Path::new(&project.root)
                    .join(&character.dir_name)
                    .join(".model.json");
                let previous = fs::read_to_string(&path).ok();
                if let Err(error) = write_character_file(&project, character) {
                    let message = format!("Action 产物元数据写入失败：{error}");
                    let completion = store
                        .fail_action_turn(
                            codex_turn_id,
                            &assistant_message_id,
                            &message,
                            MessageStatus::Failed,
                            now(),
                        )
                        .await?;
                    self.apply_failed_message(
                        &context.conversation_id,
                        &assistant_message_id,
                        message,
                    )?;
                    return Ok(TurnOutputCompletion::Completed(completion.map(
                        |completion| CompletedTaskAttempt {
                            attempt_id: completion.attempt_id,
                            task_id: completion.task_id,
                            conversation_id: completion.conversation_id,
                            status: TaskAttemptStatus::Failed,
                            agent_code: Some(context.agent_code),
                            action: None,
                        },
                    )));
                }
                Some((path, previous))
            } else {
                None
            };
            let completion = match store
                .commit_action_turn(
                    codex_turn_id,
                    &assistant_message_id,
                    &parsed.text,
                    &parsed.action,
                    &generations,
                    updated_character.as_ref(),
                    now(),
                )
                .await
            {
                Ok(completion) => completion,
                Err(error) => {
                    if let Some((path, previous)) = meta_backup {
                        restore_file(&path, previous.as_deref());
                    }
                    return Err(error.into());
                }
            };
            let drafts = store.list_drafts(&context.conversation_id).await?;
            let memories = store
                .list_conversation_memories(&context.conversation_id)
                .await?;
            let handoffs = store.list_handoffs(&context.conversation_id).await?;
            {
                let mut projects = self
                    .projects
                    .lock()
                    .map_err(|_| GameServiceError::StateUnavailable)?;
                let session =
                    find_conversation_session_mut(&mut projects, &context.conversation_id)?;
                let snapshot = session
                    .conversations
                    .get_mut(&context.conversation_id)
                    .ok_or_else(|| {
                        GameServiceError::ConversationNotFound(context.conversation_id.clone())
                    })?;
                let message = snapshot
                    .messages
                    .iter_mut()
                    .find(|message| message.id == assistant_message_id)
                    .ok_or_else(|| {
                        GameServiceError::InvalidAction("找不到本轮 Agent 消息".to_string())
                    })?;
                message.content = parsed.text;
                message.action = Some(parsed.action.clone());
                message.status = MessageStatus::Completed;
                snapshot.conversation.focus_agent_code =
                    parsed.action.target_agent.clone().or_else(|| {
                        if parsed.action.action == AgentActionKind::Handoff {
                            None
                        } else {
                            Some(context.agent_code.clone())
                        }
                    });
                snapshot.conversation.status = ConversationStatus::Active;
                snapshot.conversation.updated_at = now();
                snapshot.drafts = drafts;
                snapshot.memories = memories;
                snapshot.handoffs = handoffs;
            }
            return Ok(TurnOutputCompletion::Completed(Some(
                CompletedTaskAttempt {
                    attempt_id: completion.attempt_id,
                    task_id: completion.task_id,
                    conversation_id: completion.conversation_id,
                    status: TaskAttemptStatus::Succeeded,
                    agent_code: Some(context.agent_code),
                    action: Some(parsed.action),
                },
            )));
        }
        Ok(TurnOutputCompletion::Completed(None))
    }

    pub async fn fail_action_protocol_violation(
        &self,
        violation: ActionProtocolViolation,
        retry_start_error: Option<&str>,
    ) -> Result<Option<CompletedTaskAttempt>, GameServiceError> {
        let final_message = if violation.context.attempt_no > 1 {
            format!(
                "{}（已自动重试 {} 次）",
                violation.message,
                violation.context.attempt_no - 1
            )
        } else {
            violation.message.clone()
        };
        let message = retry_start_error.map_or_else(
            || final_message.clone(),
            |error| format!("{final_message}；自动重试启动失败：{error}"),
        );
        let completion = violation
            .store
            .fail_action_turn(
                &violation.codex_turn_id,
                &violation.assistant_message_id,
                &message,
                MessageStatus::Failed,
                now(),
            )
            .await?;
        self.apply_failed_message(
            &violation.context.conversation_id,
            &violation.assistant_message_id,
            message,
        )?;
        Ok(completion.map(|completion| CompletedTaskAttempt {
            attempt_id: completion.attempt_id,
            task_id: completion.task_id,
            conversation_id: completion.conversation_id,
            status: TaskAttemptStatus::Failed,
            agent_code: Some(violation.context.agent_code),
            action: None,
        }))
    }

    async fn prepare_action_generations(
        &self,
        context: &codex_game_store::TurnAttemptContext,
        action: &AgentAction,
    ) -> Result<(Vec<Generation>, Option<Character>), GameServiceError> {
        if context.agent_code != "visual_designer" {
            return Ok((Vec::new(), None));
        }
        let result = action.payload.result.as_ref().ok_or_else(|| {
            GameServiceError::InvalidAction("视觉设计任务必须返回 payload.result".to_string())
        })?;
        if action.action == AgentActionKind::Blocked {
            return Ok((Vec::new(), None));
        }
        if result.artifacts.is_empty() {
            return Err(GameServiceError::InvalidAction(
                "图片执行成功时必须返回至少一个产物".to_string(),
            ));
        }
        let expected_executor = match context.stage.as_str() {
            "render" => "image_t2i",
            "views" => "image_i2i",
            _ => {
                return Err(GameServiceError::InvalidAction(
                    "图片产物只能登记到 render 或 views 阶段".to_string(),
                ));
            }
        };
        let (project, target_kind) = {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .values()
                .find(|session| session.conversations.contains_key(&context.conversation_id))
                .ok_or_else(|| {
                    GameServiceError::ConversationNotFound(context.conversation_id.clone())
                })?;
            let target_kind = session.conversations[&context.conversation_id]
                .conversation
                .target_kind;
            (session.project.clone(), target_kind)
        };
        if target_kind != ConversationTargetKind::Character {
            return Err(GameServiceError::InvalidAction(
                "图片执行 Agent 只能为角色会话登记产物".to_string(),
            ));
        }
        let mut character = self
            .read_character(project.id.as_str(), &context.target_id)
            .await?;
        let next_state = match (context.stage.as_str(), character.state) {
            ("render", CharacterState::S1SpecConfirmed) => Some(CharacterState::S2RenderGenerated),
            ("render", CharacterState::S2RenderGenerated) => None,
            ("views", CharacterState::S3RenderConfirmed) => Some(CharacterState::S4ViewsGenerated),
            ("views", CharacterState::S4ViewsGenerated) => None,
            _ => {
                return Err(GameServiceError::InvalidCharacterOperation(format!(
                    "当前角色状态不允许登记 {} 产物",
                    context.stage
                )));
            }
        };
        if result.artifacts.len() != 1 {
            return Err(GameServiceError::InvalidAction(
                "角色图片 Agent 每次必须且只能返回一张 2048x2048 图片".to_string(),
            ));
        }
        let mut paths = HashSet::new();
        let mut generations = Vec::with_capacity(result.artifacts.len());
        for artifact in &result.artifacts {
            let executor = artifact
                .get("executor")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    GameServiceError::InvalidAction("视觉产物缺少内部 executor".to_string())
                })?;
            if executor != expected_executor {
                return Err(GameServiceError::InvalidAction(format!(
                    "{} 阶段必须使用 {expected_executor} 内部执行器",
                    context.stage
                )));
            }
            let file_path = artifact
                .get("path")
                .and_then(serde_json::Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .ok_or_else(|| GameServiceError::InvalidAction("图片产物缺少 path".to_string()))?
                .to_string();
            if !Path::new(&file_path).starts_with("tmp") {
                return Err(GameServiceError::InvalidAction(
                    "图片执行产物必须写入项目 tmp/ 临时目录".to_string(),
                ));
            }
            if !paths.insert(file_path.clone()) {
                return Err(GameServiceError::InvalidAction(
                    "图片产物路径不能重复".to_string(),
                ));
            }
            let size = artifact
                .get("size")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if size != "2048x2048" {
                return Err(GameServiceError::InvalidAction(
                    "角色图片产物尺寸必须为 2048x2048".to_string(),
                ));
            }
            let path = safe_project_path(&project.root, &file_path)?;
            if !path.is_file() {
                return Err(GameServiceError::InvalidCharacterOperation(format!(
                    "生成文件不存在：{file_path}"
                )));
            }
            let canonical_root = fs::canonicalize(&project.root)?;
            let canonical_tmp = fs::canonicalize(Path::new(&project.root).join("tmp"))?;
            let canonical_path = fs::canonicalize(&path)?;
            if !canonical_tmp.starts_with(&canonical_root)
                || !canonical_path.starts_with(&canonical_tmp)
            {
                return Err(GameServiceError::InvalidCharacterOperation(
                    "图片执行产物必须真实位于项目 tmp/ 临时目录".to_string(),
                ));
            }
            let variant = artifact
                .get("variant")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| (context.stage == "views").then(|| "quad".to_string()));
            if context.stage == "views" && variant.as_deref() != Some("quad") {
                return Err(GameServiceError::InvalidAction(
                    "四视图产物必须是单张 2×2 四宫格（variant=quad）".to_string(),
                ));
            }
            let mut asset_spec = artifact.clone();
            asset_spec.insert(
                "submitted_by".to_string(),
                serde_json::Value::String(context.agent_code.clone()),
            );
            generations.push(Generation {
                id: Uuid::now_v7().to_string(),
                project_id: project.id.as_str().to_string(),
                target_kind: "character".to_string(),
                target_ref: context.target_id.clone(),
                stage: context.stage.clone(),
                variant,
                file_path,
                file_hash: Some(bytes_hash(&fs::read(&canonical_path)?)),
                is_final: false,
                source: expected_executor.to_string(),
                task_id: Some(context.task_id.clone()),
                asset_spec: serde_json::to_value(asset_spec)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
                created_at: now(),
            });
        }
        if let Some(next_state) = next_state {
            character.state = codex_game_domain::advance_character(character.state, next_state)
                .map_err(|error| GameServiceError::InvalidCharacterOperation(error.to_string()))?;
            character.updated_at = now();
        }
        Ok((generations, Some(character)))
    }

    fn apply_failed_message(
        &self,
        conversation_id: &str,
        assistant_message_id: &str,
        content: String,
    ) -> Result<(), GameServiceError> {
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = find_conversation_session_mut(&mut projects, conversation_id)?;
        let snapshot = session
            .conversations
            .get_mut(conversation_id)
            .ok_or_else(|| GameServiceError::ConversationNotFound(conversation_id.to_string()))?;
        let message = snapshot
            .messages
            .iter_mut()
            .find(|message| message.id == assistant_message_id)
            .ok_or_else(|| GameServiceError::InvalidAction("找不到本轮 Agent 消息".to_string()))?;
        message.content = content;
        message.status = MessageStatus::Failed;
        snapshot.conversation.status = ConversationStatus::Active;
        snapshot.conversation.updated_at = now();
        Ok(())
    }

    fn apply_latest_running_message(
        &self,
        conversation_id: &str,
        status: MessageStatus,
        fallback_content: String,
    ) -> Result<(), GameServiceError> {
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = find_conversation_session_mut(&mut projects, conversation_id)?;
        let snapshot = session
            .conversations
            .get_mut(conversation_id)
            .ok_or_else(|| GameServiceError::ConversationNotFound(conversation_id.to_string()))?;
        if let Some(message) = snapshot
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.status == MessageStatus::Thinking)
        {
            if message.content.is_empty() {
                message.content = fallback_content;
            }
            message.status = status;
        }
        snapshot.conversation.status = ConversationStatus::Active;
        snapshot.conversation.updated_at = now();
        Ok(())
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
        target_kind: ConversationTargetKind,
        target_ref: Option<String>,
        title: String,
        director_agent_code: String,
    ) -> Result<Conversation, GameServiceError> {
        let (conversation, store) = {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .get_mut(project_id)
                .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
            if target_kind == ConversationTargetKind::Character
                && session.project.state != ProjectState::Ready
            {
                return Err(GameServiceError::ProjectGate(
                    "Art Bible 与立项尚未确认".to_string(),
                ));
            }
            if let Some(snapshot) = session.conversations.values().find(|snapshot| {
                snapshot.conversation.target_kind == target_kind
                    && snapshot.conversation.target_ref == target_ref
            }) {
                return Ok(snapshot.conversation.clone());
            }
            require_writable(session)?;
            let timestamp = now();
            let conversation = Conversation {
                id: ConversationId::new(Uuid::now_v7().to_string()),
                project_id: ProjectId::new(project_id),
                target_kind,
                target_ref,
                title,
                director_agent_code,
                focus_agent_code: None,
                status: ConversationStatus::Active,
                turn: 0,
                created_at: timestamp,
                updated_at: timestamp,
            };
            session.conversations.insert(
                conversation.id.as_str().to_string(),
                ConversationSnapshot {
                    conversation: conversation.clone(),
                    messages: Vec::new(),
                    drafts: Vec::new(),
                    memories: Vec::new(),
                    handoffs: Vec::new(),
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
        self.prepare_conversation_turn(conversation_id, content, None)
            .await
            .map(|prepared| prepared.user_message)
    }

    pub async fn prepare_conversation_turn(
        &self,
        conversation_id: &str,
        content: String,
        recipient_agent_code: Option<String>,
    ) -> Result<PreparedConversationTurn, GameServiceError> {
        self.prepare_conversation_turn_with_visibility(
            conversation_id,
            content,
            recipient_agent_code,
            false,
        )
        .await
    }

    pub async fn prepare_character_director_resume_turn(
        &self,
        project_id: &str,
        character_id: &str,
        content: String,
    ) -> Result<PreparedConversationTurn, GameServiceError> {
        let conversation = self.find_target_conversation(
            project_id,
            ConversationTargetKind::Character,
            Some(character_id),
        )?;
        self.prepare_director_resume_turn(conversation.id.as_str(), content)
            .await
    }

    pub async fn prepare_character_director_resume_turn_if_idle(
        &self,
        project_id: &str,
        character_id: &str,
        content: String,
    ) -> Result<Option<PreparedConversationTurn>, GameServiceError> {
        let conversation = self.find_target_conversation(
            project_id,
            ConversationTargetKind::Character,
            Some(character_id),
        )?;
        self.prepare_director_resume_turn_if_idle(conversation.id.as_str(), content)
            .await
    }

    pub async fn prepare_director_resume_turn(
        &self,
        conversation_id: &str,
        content: String,
    ) -> Result<PreparedConversationTurn, GameServiceError> {
        let director_agent_code = self
            .read_conversation(conversation_id)
            .await?
            .conversation
            .director_agent_code;
        self.prepare_conversation_turn_with_visibility(
            conversation_id,
            content,
            Some(director_agent_code),
            true,
        )
        .await
    }

    pub async fn prepare_director_resume_turn_if_idle(
        &self,
        conversation_id: &str,
        content: String,
    ) -> Result<Option<PreparedConversationTurn>, GameServiceError> {
        match self
            .prepare_director_resume_turn(conversation_id, content)
            .await
        {
            Ok(prepared) => Ok(Some(prepared)),
            Err(GameServiceError::InvalidAction(message))
                if message == CONVERSATION_ALREADY_RUNNING =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn prepare_conversation_turn_with_visibility(
        &self,
        conversation_id: &str,
        content: String,
        recipient_agent_code: Option<String>,
        folded: bool,
    ) -> Result<PreparedConversationTurn, GameServiceError> {
        if content.trim().is_empty() {
            return Err(GameServiceError::InvalidAction("消息不能为空".to_string()));
        }
        let (project, store, target_kind, target_ref, current_focus, director, status) = {
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
            let conversation = &session.conversations[conversation_id].conversation;
            (
                session.project.clone(),
                Arc::clone(&session.store),
                conversation.target_kind,
                conversation.target_ref.clone(),
                conversation.focus_agent_code.clone(),
                conversation.director_agent_code.clone(),
                conversation.status,
            )
        };
        if status == ConversationStatus::Running {
            return Err(GameServiceError::InvalidAction(
                CONVERSATION_ALREADY_RUNNING.to_string(),
            ));
        }
        let stage = match target_kind {
            ConversationTargetKind::Project => "project".to_string(),
            ConversationTargetKind::Character => {
                let character_id = target_ref.as_deref().ok_or_else(|| {
                    GameServiceError::InvalidCharacterOperation(
                        "角色会话缺少 targetRef".to_string(),
                    )
                })?;
                self.read_character(project.id.as_str(), character_id)
                    .await?
                    .state
                    .stage()
                    .to_string()
            }
        };
        let previous_focus = current_focus.clone();
        let agent_code = recipient_agent_code
            .clone()
            .or(current_focus)
            .unwrap_or(director);
        let allowed = codex_game_domain::agents_for_stage(target_kind.as_str(), &stage);
        if !allowed.contains(&agent_code.as_str()) {
            return Err(GameServiceError::InvalidAction(format!(
                "Agent {agent_code} 不允许处理 {stage} 阶段"
            )));
        }
        let timestamp = now();
        let (conversation, user_message, assistant_message) = {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = find_conversation_session_mut(&mut projects, conversation_id)?;
            let snapshot = session
                .conversations
                .get_mut(conversation_id)
                .ok_or_else(|| {
                    GameServiceError::ConversationNotFound(conversation_id.to_string())
                })?;
            if snapshot.conversation.status == ConversationStatus::Running {
                return Err(GameServiceError::InvalidAction(
                    CONVERSATION_ALREADY_RUNNING.to_string(),
                ));
            }
            snapshot.conversation.turn += 1;
            snapshot.conversation.focus_agent_code = Some(agent_code.clone());
            snapshot.conversation.status = ConversationStatus::Running;
            snapshot.conversation.updated_at = timestamp;
            let turn = snapshot.conversation.turn;
            let user_message = ConversationMessage {
                id: Uuid::now_v7().to_string(),
                conversation_id: ConversationId::new(conversation_id),
                turn,
                role: "user".to_string(),
                content,
                agent_code: "user".to_string(),
                recipient_agent_code,
                status: MessageStatus::Completed,
                token_count: 0,
                folded,
                attachments: Vec::new(),
                action: None,
                created_at: timestamp,
            };
            let assistant_message = ConversationMessage {
                id: Uuid::now_v7().to_string(),
                conversation_id: ConversationId::new(conversation_id),
                turn,
                role: "assistant".to_string(),
                content: String::new(),
                agent_code: agent_code.clone(),
                recipient_agent_code: None,
                status: MessageStatus::Thinking,
                token_count: 0,
                folded: false,
                attachments: Vec::new(),
                action: None,
                created_at: timestamp,
            };
            snapshot.messages.push(user_message.clone());
            snapshot.messages.push(assistant_message.clone());
            (
                snapshot.conversation.clone(),
                user_message,
                assistant_message,
            )
        };
        if let Err(error) = store
            .begin_conversation_turn(&conversation, &user_message, &assistant_message)
            .await
        {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = find_conversation_session_mut(&mut projects, conversation_id)?;
            if let Some(snapshot) = session.conversations.get_mut(conversation_id) {
                snapshot.messages.retain(|message| {
                    message.id != user_message.id && message.id != assistant_message.id
                });
                snapshot.conversation.turn = snapshot.conversation.turn.saturating_sub(1);
                snapshot.conversation.focus_agent_code = previous_focus;
                snapshot.conversation.status = ConversationStatus::Active;
            }
            return Err(error.into());
        }
        Ok(PreparedConversationTurn {
            user_message,
            assistant_message,
            conversation,
            agent_code,
            stage,
            project,
            store,
        })
    }

    pub async fn complete_prepared_blocked(
        &self,
        prepared: &PreparedConversationTurn,
        reason: String,
    ) -> Result<(), GameServiceError> {
        let action = AgentAction {
            action: AgentActionKind::Blocked,
            target_agent: None,
            reason: reason.clone(),
            payload: AgentActionPayload::default(),
        };
        let timestamp = now();
        prepared
            .store
            .complete_prepared_action(
                prepared.conversation.id.as_str(),
                &prepared.assistant_message.id,
                &reason,
                &action,
                timestamp,
            )
            .await?;
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session =
            find_conversation_session_mut(&mut projects, prepared.conversation.id.as_str())?;
        let snapshot = session
            .conversations
            .get_mut(prepared.conversation.id.as_str())
            .ok_or_else(|| {
                GameServiceError::ConversationNotFound(
                    prepared.conversation.id.as_str().to_string(),
                )
            })?;
        let message = snapshot
            .messages
            .iter_mut()
            .find(|message| message.id == prepared.assistant_message.id)
            .ok_or_else(|| {
                GameServiceError::InvalidAction("找不到待完成的 Agent 消息".to_string())
            })?;
        message.content = reason;
        message.action = Some(action);
        message.status = MessageStatus::Completed;
        snapshot.conversation.status = ConversationStatus::Active;
        snapshot.conversation.updated_at = timestamp;
        Ok(())
    }

    pub async fn read_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationSnapshot, GameServiceError> {
        let (mut snapshot, store) = {
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
            (
                session.conversations[conversation_id].clone(),
                Arc::clone(&session.store),
            )
        };
        snapshot.drafts = store.list_drafts(conversation_id).await?;
        snapshot.memories = store.list_conversation_memories(conversation_id).await?;
        snapshot.handoffs = store.list_handoffs(conversation_id).await?;
        Ok(snapshot)
    }

    pub async fn prepare_handoff_turn(
        &self,
        conversation_id: &str,
        target_agent: &str,
    ) -> Result<PreparedConversationTurn, GameServiceError> {
        let snapshot = self.read_conversation(conversation_id).await?;
        let latest_message = snapshot
            .messages
            .iter()
            .rev()
            .find(|message| message.action.is_some())
            .ok_or_else(|| GameServiceError::InvalidAction("没有可继续的 handoff".to_string()))?;
        let latest_action = latest_message
            .action
            .as_ref()
            .ok_or_else(|| GameServiceError::InvalidAction("没有可继续的 handoff".to_string()))?;
        if latest_action.action != AgentActionKind::Handoff
            || latest_action.target_agent.as_deref() != Some(target_agent)
        {
            return Err(GameServiceError::InvalidAction(
                "handoff 目标与上一条 Action 不一致".to_string(),
            ));
        }
        if latest_message.agent_code != snapshot.conversation.director_agent_code
            && target_agent != snapshot.conversation.director_agent_code
        {
            return Err(GameServiceError::InvalidAction(
                "专业 Agent 只能将控制权交回总管".to_string(),
            ));
        }
        let handoff_count = snapshot
            .handoffs
            .iter()
            .filter(|handoff| handoff.turn == snapshot.conversation.turn)
            .count();
        if handoff_count > MAX_HANDOFFS {
            return Err(GameServiceError::InvalidAction(format!(
                "单轮自动 handoff 不得超过 {MAX_HANDOFFS} 次"
            )));
        }
        let (project, store) = {
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
            (session.project.clone(), Arc::clone(&session.store))
        };
        let stage = match snapshot.conversation.target_kind {
            ConversationTargetKind::Project => "project".to_string(),
            ConversationTargetKind::Character => {
                let character_id =
                    snapshot.conversation.target_ref.as_deref().ok_or_else(|| {
                        GameServiceError::InvalidCharacterOperation(
                            "角色会话缺少 targetRef".to_string(),
                        )
                    })?;
                self.read_character(project.id.as_str(), character_id)
                    .await?
                    .state
                    .stage()
                    .to_string()
            }
        };
        let allowed =
            codex_game_domain::agents_for_stage(snapshot.conversation.target_kind.as_str(), &stage);
        if target_agent != snapshot.conversation.director_agent_code
            && !allowed.contains(&target_agent)
        {
            return Err(GameServiceError::InvalidAction(format!(
                "Agent {target_agent} 不允许处理 {stage} 阶段"
            )));
        }
        let timestamp = now();
        let mut conversation = snapshot.conversation.clone();
        conversation.focus_agent_code = Some(target_agent.to_string());
        conversation.status = ConversationStatus::Running;
        conversation.updated_at = timestamp;
        let assistant_message = ConversationMessage {
            id: Uuid::now_v7().to_string(),
            conversation_id: ConversationId::new(conversation_id),
            turn: conversation.turn,
            role: "assistant".to_string(),
            content: String::new(),
            agent_code: target_agent.to_string(),
            recipient_agent_code: None,
            status: MessageStatus::Thinking,
            token_count: 0,
            folded: false,
            attachments: Vec::new(),
            action: None,
            created_at: timestamp,
        };
        store
            .begin_handoff_continuation(&conversation, &assistant_message)
            .await?;
        {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = find_conversation_session_mut(&mut projects, conversation_id)?;
            let current = session
                .conversations
                .get_mut(conversation_id)
                .ok_or_else(|| {
                    GameServiceError::ConversationNotFound(conversation_id.to_string())
                })?;
            current.conversation = conversation.clone();
            current.messages.push(assistant_message.clone());
        }
        let user_message = snapshot
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user" && message.turn == conversation.turn)
            .cloned()
            .ok_or_else(|| {
                GameServiceError::InvalidAction("handoff 缺少原始用户消息".to_string())
            })?;
        Ok(PreparedConversationTurn {
            user_message,
            assistant_message,
            conversation,
            agent_code: target_agent.to_string(),
            stage,
            project,
            store,
        })
    }

    pub async fn build_conversation_context(
        &self,
        prepared: &PreparedConversationTurn,
    ) -> Result<ContextPackage, GameServiceError> {
        let snapshot = self
            .read_conversation(prepared.conversation.id.as_str())
            .await?;
        let art_bible_path = Path::new(&prepared.project.root).join("art-bible.md");
        let art_bible = fs::read_to_string(art_bible_path).ok();
        let character = if prepared.conversation.target_kind == ConversationTargetKind::Character {
            let character_id = prepared.conversation.target_ref.as_deref().ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation("角色会话缺少 targetRef".to_string())
            })?;
            Some(
                self.read_character(prepared.project.id.as_str(), character_id)
                    .await?,
            )
        } else {
            None
        };
        let character_context = character
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let project_memories = prepared
            .store
            .list_project_memories(
                prepared.project.id.as_str(),
                prepared.conversation.target_ref.as_deref(),
            )
            .await?;
        let workflow_facts = if let Some(character) = character.as_ref() {
            let tasks = prepared
                .store
                .list_tasks(prepared.conversation.id.as_str())
                .await?;
            let generations = prepared
                .store
                .list_generations(
                    prepared.project.id.as_str(),
                    "character",
                    character.id.as_str(),
                    None,
                )
                .await?;
            Some(character_workflow_facts(
                character,
                &snapshot.messages,
                &snapshot.drafts,
                &tasks,
                &generations,
                &project_memories,
            ))
        } else {
            None
        };
        let workflow_context =
            character
                .as_ref()
                .zip(workflow_facts.as_ref())
                .map(|(character, facts)| WorkflowContext {
                    phase: serde_json::to_value(character.state)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_else(|| character.state.stage().to_string()),
                    pending_draft_id: facts.pending_spec.as_ref().map(|draft| draft.id.clone()),
                    review_status: facts.spec_review_status.as_str().to_string(),
                    latest_verdict: facts.latest_spec_verdict.as_ref().map(|verdict| {
                        WorkflowVerdictSummary {
                            token: verdict.token.clone(),
                            subject_id: verdict.subject_id.clone(),
                            decision: verdict.decision.clone(),
                        }
                    }),
                });
        let review_subject = (prepared.agent_code == "spec_reviewer")
            .then(|| workflow_facts.as_ref()?.pending_spec.as_ref())
            .flatten()
            .map(|draft| ReviewSubject {
                id: draft.id.clone(),
                target_path: draft.target_path.clone(),
                content: draft.content.clone(),
            });
        let handoff_count = snapshot
            .handoffs
            .iter()
            .filter(|handoff| handoff.turn == prepared.conversation.turn)
            .count();
        let allowed_handoffs = allowed_handoffs_for(
            prepared.conversation.target_kind.as_str(),
            &prepared.stage,
            &prepared.agent_code,
            &prepared.conversation.director_agent_code,
            handoff_count,
            character.as_ref(),
            workflow_facts.as_ref(),
        );
        let conversation_history = snapshot
            .messages
            .iter()
            .rev()
            .take(12)
            .rev()
            .map(|message| {
                format!(
                    "{}({}): {}",
                    message.role, message.agent_code, message.content
                )
            })
            .collect();
        let memories = project_memories
            .into_iter()
            .map(|memory| format!("{}:{}", memory.kind, memory.content))
            .collect();
        Ok(ContextPackage {
            conversation_history,
            context_version: prepared.conversation.turn,
            contract_version: 2,
            agent_definition_version: "2".to_string(),
            // Agent 输出是面向用户的正文加尾置 Action 块，不能启用整条回复 JSON schema。
            output_schema: String::new(),
            target_kind: prepared.conversation.target_kind.as_str().to_string(),
            target_ref: prepared.conversation.target_ref.clone(),
            stage: prepared.stage.clone(),
            art_bible,
            character_context,
            workflow_context,
            review_subject,
            memories,
            allowed_handoffs,
            action_protocol: action_protocol_instruction(
                &prepared.agent_code,
                &prepared.conversation.director_agent_code,
            ),
        })
    }

    pub fn inspect_project_dir(&self, root: &str) -> Result<ProjectDirState, GameServiceError> {
        let root = absolute_root(root)?;
        let project_json = root.join("project.json");
        let occupied = project_json.exists();
        if !occupied {
            return Ok(ProjectDirState {
                root: root.to_string_lossy().into_owned(),
                occupied,
                project_id: None,
                supported: true,
            });
        }
        let value = fs::read_to_string(&project_json)
            .ok()
            .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok());
        Ok(ProjectDirState {
            root: root.to_string_lossy().into_owned(),
            occupied,
            project_id: value
                .as_ref()
                .and_then(|value| value.get("projectId").or_else(|| value.get("id")))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            supported: value
                .as_ref()
                .and_then(|value| value.get("schemaVersion"))
                .and_then(serde_json::Value::as_u64)
                == Some(2),
        })
    }

    pub async fn commit_art_bible_draft(
        &self,
        conversation_id: &str,
        draft_id: &str,
    ) -> Result<ArtBibleDocument, GameServiceError> {
        let snapshot = self.read_conversation(conversation_id).await?;
        if snapshot.conversation.target_kind != ConversationTargetKind::Project {
            return Err(GameServiceError::ProjectGate(
                "只有项目会话可以确认 Art Bible".to_string(),
            ));
        }
        let draft = snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == draft_id && draft.status == "pending")
            .cloned()
            .ok_or_else(|| {
                GameServiceError::InvalidAction("Art Bible 草稿不存在或已提交".to_string())
            })?;
        if draft.target_path != "art-bible.md" {
            return Err(GameServiceError::InvalidAction(
                "Art Bible 草稿目标必须是 art-bible.md".to_string(),
            ));
        }
        let (project, store, version_number) = {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .get(snapshot.conversation.project_id.as_str())
                .ok_or_else(|| {
                    GameServiceError::ProjectNotFound(
                        snapshot.conversation.project_id.as_str().to_string(),
                    )
                })?;
            require_writable(session)?;
            (
                session.project.clone(),
                Arc::clone(&session.store),
                session.art_bibles.len() as u64 + 1,
            )
        };
        let path = Path::new(&project.root).join("art-bible.md");
        let project_json = Path::new(&project.root).join("project.json");
        validate_draft_baseline(&path, draft.based_on_hash.as_deref())?;
        let backups = vec![
            (path.clone(), fs::read(&path).ok()),
            (project_json.clone(), fs::read(&project_json).ok()),
        ];
        write_art_bible(&path, &draft.content)?;
        if let Err(error) = update_project_json(
            &project_json,
            project.id.as_str(),
            &project.name,
            "styleSettled",
        ) {
            restore_files(&backups);
            return Err(error.into());
        }
        let version = ArtBibleVersion {
            id: ArtBibleVersionId::new(Uuid::now_v7().to_string()),
            project_id: project.id.clone(),
            version: version_number,
            content_hash: content_hash(&draft.content),
            source_artifact_ids: Vec::new(),
            created_at: now(),
        };
        if let Err(error) = store
            .commit_art_bible_gate(&version, &draft.content, &draft.id)
            .await
        {
            restore_files(&backups);
            return Err(error.into());
        }
        let document = ArtBibleDocument {
            version,
            markdown: draft.content,
        };
        let drafts = store.list_drafts(conversation_id).await?;
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = projects
            .get_mut(project.id.as_str())
            .ok_or_else(|| GameServiceError::ProjectNotFound(project.id.as_str().to_string()))?;
        session.project.state = ProjectState::StyleSettled;
        session.art_bibles.push(document.clone());
        if let Some(current) = session.conversations.get_mut(conversation_id) {
            current.drafts = drafts;
        }
        Ok(document)
    }

    pub async fn commit_conversation_drafts(
        &self,
        conversation_id: &str,
        draft_ids: &[String],
    ) -> Result<Vec<String>, GameServiceError> {
        if draft_ids.is_empty() {
            return Err(GameServiceError::InvalidAction(
                "至少选择一个草稿".to_string(),
            ));
        }
        let snapshot = self.read_conversation(conversation_id).await?;
        let project = self.read_project(snapshot.conversation.project_id.as_str())?;
        let store = self.project_store(project.id.as_str(), true)?;
        let mut selected = Vec::with_capacity(draft_ids.len());
        for draft_id in draft_ids {
            if selected
                .iter()
                .any(|draft: &ArtifactDraftRecord| draft.id == *draft_id)
            {
                return Err(GameServiceError::InvalidAction(
                    "草稿列表不能包含重复项".to_string(),
                ));
            }
            let draft = snapshot
                .drafts
                .iter()
                .find(|draft| draft.id == *draft_id && draft.status == "pending")
                .cloned()
                .ok_or_else(|| {
                    GameServiceError::InvalidAction(format!("草稿不存在或已提交：{draft_id}"))
                })?;
            if matches!(draft.target_path.as_str(), "project.json" | "art-bible.md")
                || draft.target_path.starts_with(".codex-game/")
            {
                return Err(GameServiceError::InvalidAction(format!(
                    "该文件必须通过专用人工门禁提交：{}",
                    draft.target_path
                )));
            }
            let path = safe_project_path(&project.root, &draft.target_path)?;
            validate_draft_baseline(&path, draft.based_on_hash.as_deref())?;
            selected.push(draft);
        }
        let backups = selected
            .iter()
            .map(|draft| {
                let path = safe_project_path(&project.root, &draft.target_path)?;
                Ok((path.clone(), fs::read(&path).ok()))
            })
            .collect::<Result<Vec<_>, GameServiceError>>()?;
        for draft in &selected {
            let path = safe_project_path(&project.root, &draft.target_path)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Err(error) = write_art_bible(&path, &draft.content) {
                restore_files(&backups);
                return Err(error.into());
            }
        }
        if let Err(error) = store.mark_drafts_committed(draft_ids).await {
            restore_files(&backups);
            return Err(error.into());
        }
        let drafts = store.list_drafts(conversation_id).await?;
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = find_conversation_session_mut(&mut projects, conversation_id)?;
        if let Some(current) = session.conversations.get_mut(conversation_id) {
            current.drafts = drafts;
        }
        Ok(selected
            .into_iter()
            .map(|draft| draft.target_path)
            .collect())
    }

    pub async fn finalize_project(
        &self,
        project_id: &str,
        name: String,
        code: String,
    ) -> Result<Project, GameServiceError> {
        if name.trim().is_empty() || !is_valid_code(&code) {
            return Err(GameServiceError::ProjectGate(
                "项目名或代号不合法".to_string(),
            ));
        }
        let (mut project, store) = {
            let projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .get(project_id)
                .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
            require_writable(session)?;
            if session.project.state != ProjectState::StyleSettled {
                return Err(GameServiceError::ProjectGate(
                    "必须先确认 Art Bible".to_string(),
                ));
            }
            (session.project.clone(), Arc::clone(&session.store))
        };
        let root = Path::new(&project.root);
        let characters = root.join("characters");
        let created_characters = !characters.exists();
        let managed_files = [
            root.join("project.json"),
            root.join(".gitignore"),
            root.join(".gitattributes"),
        ];
        let backups = managed_files
            .iter()
            .map(|path| (path.clone(), fs::read(path).ok()))
            .collect::<Vec<_>>();
        let write_result = (|| -> Result<(), GameServiceError> {
            fs::create_dir_all(&characters)?;
            append_line_once(&root.join(".gitignore"), "/.codex-game/local/")?;
            append_line_once(&root.join(".gitattributes"), "*.png binary")?;
            project.name = name.trim().to_string();
            project.code = Some(code.clone());
            project.state = ProjectState::Ready;
            finalize_project_json(
                &root.join("project.json"),
                project.id.as_str(),
                &project.name,
                &code,
            )?;
            Ok(())
        })();
        if let Err(error) = write_result {
            restore_files(&backups);
            if created_characters {
                let _ = fs::remove_dir(&characters);
            }
            return Err(error);
        }
        if let Err(error) = self.register_project(&project, store.access()).await {
            restore_files(&backups);
            if created_characters {
                let _ = fs::remove_dir(&characters);
            }
            return Err(error);
        }
        {
            let mut projects = self
                .projects
                .lock()
                .map_err(|_| GameServiceError::StateUnavailable)?;
            let session = projects
                .get_mut(project_id)
                .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
            session.project = project.clone();
        }
        Ok(project)
    }

    pub async fn list_characters(
        &self,
        project_id: &str,
    ) -> Result<Vec<ListedCharacter>, GameServiceError> {
        let project = self.read_project(project_id)?;
        let disk_characters = read_project_characters(&project)?;
        let disk_ids = disk_characters
            .iter()
            .map(|character| character.id.clone())
            .collect::<HashSet<_>>();
        let mut characters = disk_characters
            .into_iter()
            .map(|character| ListedCharacter {
                character,
                model_file_exists: true,
            })
            .collect::<Vec<_>>();
        characters.extend(
            self.project_store(project_id, false)?
                .list_characters(project_id)
                .await?
                .into_iter()
                .filter(|character| !disk_ids.contains(&character.id))
                .map(|character| ListedCharacter {
                    character,
                    model_file_exists: false,
                }),
        );
        characters.sort_by(|left, right| {
            left.character
                .name
                .cmp(&right.character.name)
                .then_with(|| left.character.id.cmp(&right.character.id))
        });
        Ok(characters)
    }

    pub async fn list_character_groups(
        &self,
        project_id: &str,
    ) -> Result<Vec<String>, GameServiceError> {
        let store = self.project_store(project_id, false)?;
        store
            .list_character_groups(project_id)
            .await
            .map_err(Into::into)
    }

    pub async fn read_character(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<Character, GameServiceError> {
        let project = self.read_project(project_id)?;
        let character = read_project_characters(&project)?
            .into_iter()
            .find(|character| character.id == character_id)
            .ok_or_else(|| GameServiceError::CharacterNotFound(character_id.to_string()))?;
        let store = self.project_store(project_id, false)?;
        if store.access() == ProjectAccess::ReadWrite {
            store.upsert_character(&character).await?;
        }
        Ok(character)
    }

    pub async fn character_workflow_progress(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<CharacterWorkflowProgress, GameServiceError> {
        let character = self.read_character(project_id, character_id).await?;
        let store = self.project_store(project_id, false)?;
        let conversation = self
            .find_target_conversation(
                project_id,
                ConversationTargetKind::Character,
                Some(character_id),
            )
            .ok();
        let generations = store
            .list_generations(project_id, "character", character_id, None)
            .await?;
        let memories = store
            .list_project_memories(project_id, Some(character_id))
            .await?;
        let conversation_running = conversation
            .as_ref()
            .is_some_and(|conversation| conversation.status == ConversationStatus::Running);
        let (messages, drafts, tasks) = if let Some(conversation) = conversation {
            let snapshot = self.read_conversation(conversation.id.as_str()).await?;
            (
                snapshot.messages,
                store.list_drafts(conversation.id.as_str()).await?,
                store.list_tasks(conversation.id.as_str()).await?,
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        let facts = character_workflow_facts(
            &character,
            &messages,
            &drafts,
            &tasks,
            &generations,
            &memories,
        );
        let spec_confirmation_ready = matches!(
            facts.spec_review_status,
            SpecReviewStatus::Approved | SpecReviewStatus::Concerns
        );
        let spec_review_failed = facts.spec_review_status == SpecReviewStatus::Error;
        let continuation_key = if conversation_running || facts.has_running_task {
            None
        } else {
            match character.state {
                CharacterState::S0SpecDrafting => match facts.spec_review_status {
                    SpecReviewStatus::Pending | SpecReviewStatus::Rejected => facts
                        .pending_spec
                        .as_ref()
                        .map(|draft| format!("spec:{}", draft.id)),
                    SpecReviewStatus::AwaitingDraft => facts
                        .latest_spec_rejection_id
                        .as_ref()
                        .map(|id| format!("spec-rewrite:{id}")),
                    SpecReviewStatus::Approved
                    | SpecReviewStatus::Concerns
                    | SpecReviewStatus::Error => None,
                },
                CharacterState::S1SpecConfirmed => character
                    .gate_spec_confirmed_at
                    .map(|timestamp| format!("render:{timestamp}")),
                CharacterState::S2RenderGenerated => facts
                    .render_rejection_id
                    .as_ref()
                    .map(|id| format!("render-retry:{id}")),
                CharacterState::S3RenderConfirmed => character
                    .gate_render_confirmed_at
                    .map(|timestamp| format!("views:{timestamp}")),
                CharacterState::S4ViewsGenerated => facts
                    .views_rejection_id
                    .as_ref()
                    .map(|id| format!("views-retry:{id}")),
                CharacterState::S5ViewsConfirmed => None,
            }
        };

        Ok(build_character_workflow_progress(
            character.state,
            spec_confirmation_ready,
            spec_review_failed,
            facts.render_rejected_after_generation,
            facts.views_rejected_after_generation,
            facts.failed_design_stage.as_deref(),
            continuation_key,
        ))
    }

    pub async fn prepare_character_resume_if_needed(
        &self,
        project_id: &str,
        character_id: &str,
        continuation_key: &str,
    ) -> Result<Option<PreparedConversationTurn>, GameServiceError> {
        let progress = self
            .character_workflow_progress(project_id, character_id)
            .await?;
        let Some(current_key) = progress.continuation_key else {
            return Ok(None);
        };
        if current_key != continuation_key {
            return Err(GameServiceError::InvalidCharacterOperation(
                "续跑标识已过期，请刷新角色状态".to_string(),
            ));
        }
        self.prepare_character_director_resume_turn_if_idle(
            project_id,
            character_id,
            format!("检测到角色工作流待续跑（{current_key}），请根据当前状态继续处理。"),
        )
        .await
    }

    pub async fn remove_character(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<(), GameServiceError> {
        let project = self.read_project(project_id)?;
        if read_project_characters(&project)?
            .iter()
            .any(|character| character.id == character_id)
        {
            return Err(GameServiceError::InvalidCharacterOperation(
                "角色文件仍然存在，不能移除角色记录".to_string(),
            ));
        }
        self.project_store(project_id, true)?
            .delete_character_record(project_id, character_id)
            .await?;
        Ok(())
    }

    pub async fn create_character_group(
        &self,
        project_id: &str,
        name: String,
    ) -> Result<String, GameServiceError> {
        let project = self.read_project(project_id)?;
        if project.state != ProjectState::Ready {
            return Err(GameServiceError::ProjectGate(
                "Art Bible 与立项尚未确认".to_string(),
            ));
        }
        let name = safe_segment(&name)?;
        fs::create_dir_all(Path::new(&project.root).join("characters").join(&name))?;
        self.project_store(project_id, true)?
            .insert_character_group(project_id, &name, now())
            .await?;
        Ok(name)
    }

    pub async fn create_character(
        &self,
        project_id: &str,
        name: String,
        group: Option<String>,
        overwrite: bool,
    ) -> Result<Character, GameServiceError> {
        let project = self.read_project(project_id)?;
        if project.state != ProjectState::Ready {
            return Err(GameServiceError::ProjectGate(
                "Art Bible 与立项尚未确认".to_string(),
            ));
        }
        let store = self.project_store(project_id, true)?;
        let dir_name = safe_segment(&name)?;
        let group = group
            .filter(|value| !value.trim().is_empty())
            .map(|value| safe_segment(&value))
            .transpose()?;
        let relative_dir = group
            .as_ref()
            .map(|group| PathBuf::from("characters").join(group).join(&dir_name))
            .unwrap_or_else(|| PathBuf::from("characters").join(&dir_name));
        let character_dir = Path::new(&project.root).join(&relative_dir);
        if character_dir.exists() && !overwrite {
            return Err(GameServiceError::InvalidCharacterOperation(
                "角色目录已存在，请明确选择覆盖".to_string(),
            ));
        }
        if overwrite && character_dir.exists() {
            fs::remove_dir_all(&character_dir)?;
        }
        fs::create_dir_all(character_dir.join("docs"))?;
        fs::create_dir_all(character_dir.join("images"))?;
        let timestamp = now();
        let character = Character {
            id: Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            name: name.trim().to_string(),
            group,
            dir_name: relative_dir.to_string_lossy().into_owned(),
            state: CharacterState::S0SpecDrafting,
            spec_path: None,
            render_path: None,
            view_paths: BTreeMap::new(),
            hard_constraints: Vec::new(),
            gate_spec_confirmed_at: None,
            gate_render_confirmed_at: None,
            gate_views_confirmed_at: None,
            created_at: timestamp,
            updated_at: timestamp,
        };
        write_character_file(&project, &character)?;
        if let Some(group) = character.group.as_deref() {
            store
                .insert_character_group(project_id, group, timestamp)
                .await?;
        }
        store.insert_character(&character).await?;
        Ok(character)
    }

    pub async fn confirm_character_spec(
        &self,
        project_id: &str,
        character_id: &str,
        draft_id: &str,
    ) -> Result<Character, GameServiceError> {
        let project = self.read_project(project_id)?;
        let store = self.project_store(project_id, true)?;
        let mut character = self.read_character(project_id, character_id).await?;
        if character.state != CharacterState::S0SpecDrafting {
            return Err(GameServiceError::InvalidCharacterOperation(
                "角色设定已确认或尚未处于 S0".to_string(),
            ));
        }
        let conversation = self.find_target_conversation(
            project_id,
            ConversationTargetKind::Character,
            Some(character_id),
        )?;
        let draft = store
            .list_drafts(conversation.id.as_str())
            .await?
            .into_iter()
            .find(|draft| draft.id == draft_id && draft.status == "pending")
            .ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation(
                    "角色设定草稿不存在或已提交".to_string(),
                )
            })?;
        if draft.target_path != "docs/角色定稿.md" {
            return Err(GameServiceError::InvalidCharacterOperation(
                "角色设定草稿目标必须是 docs/角色定稿.md".to_string(),
            ));
        }
        let constraints = self
            .collect_verdict_constraints(project_id, character_id, "SPEC-CHECK", &draft.id)
            .await?;
        let relative = PathBuf::from(&character.dir_name).join(&draft.target_path);
        let path = Path::new(&project.root).join(&relative);
        let meta_path = Path::new(&project.root)
            .join(&character.dir_name)
            .join(".model.json");
        validate_draft_baseline(&path, draft.based_on_hash.as_deref())?;
        character.state =
            codex_game_domain::advance_character(character.state, CharacterState::S1SpecConfirmed)
                .map_err(|error| GameServiceError::InvalidCharacterOperation(error.to_string()))?;
        character.spec_path = Some(relative.to_string_lossy().into_owned());
        character.hard_constraints = constraints;
        let timestamp = now();
        character.gate_spec_confirmed_at = Some(timestamp);
        character.updated_at = timestamp;
        let backups = vec![
            (path.clone(), fs::read(&path).ok()),
            (meta_path.clone(), fs::read(&meta_path).ok()),
        ];
        if let Err(error) = write_art_bible(&path, &draft.content)
            .map_err(GameServiceError::from)
            .and_then(|_| {
                write_character_file(&project, &character).map_err(GameServiceError::from)
            })
        {
            restore_files(&backups);
            return Err(error);
        }
        if let Err(error) = store
            .commit_character_spec_gate(&draft.id, &character, timestamp)
            .await
        {
            restore_files(&backups);
            return Err(error.into());
        }
        Ok(character)
    }

    pub async fn register_generation(
        &self,
        project_id: &str,
        character_id: &str,
        stage: &str,
        variant: Option<String>,
        file_path: String,
        source: String,
        asset_spec: serde_json::Value,
    ) -> Result<Generation, GameServiceError> {
        let project = self.read_project(project_id)?;
        let store = self.project_store(project_id, true)?;
        let mut character = self.read_character(project_id, character_id).await?;
        let next_state = match (stage, character.state) {
            ("render", CharacterState::S1SpecConfirmed) => Some(CharacterState::S2RenderGenerated),
            ("render", CharacterState::S2RenderGenerated) => None,
            ("views", CharacterState::S3RenderConfirmed) => Some(CharacterState::S4ViewsGenerated),
            ("views", CharacterState::S4ViewsGenerated) => None,
            ("render" | "views", _) => {
                return Err(GameServiceError::InvalidCharacterOperation(format!(
                    "当前角色状态不允许登记 {stage} 产物"
                )));
            }
            _ => {
                return Err(GameServiceError::InvalidCharacterOperation(
                    "generation stage 必须为 render 或 views".to_string(),
                ));
            }
        };
        if stage == "views" && variant.as_deref().is_some_and(|value| value != "quad") {
            return Err(GameServiceError::InvalidCharacterOperation(
                "四视图候选必须是单张 2×2 四宫格（variant=quad）".to_string(),
            ));
        }
        let variant = if stage == "views" {
            Some("quad".to_string())
        } else {
            variant
        };
        let path = safe_project_path(&project.root, &file_path)?;
        if !path.is_file() {
            return Err(GameServiceError::InvalidCharacterOperation(
                "生成文件不存在".to_string(),
            ));
        }
        let generation = Generation {
            id: Uuid::now_v7().to_string(),
            project_id: project_id.to_string(),
            target_kind: "character".to_string(),
            target_ref: character_id.to_string(),
            stage: stage.to_string(),
            variant,
            file_path,
            file_hash: Some(bytes_hash(&fs::read(&path)?)),
            is_final: false,
            source,
            task_id: None,
            asset_spec,
            created_at: now(),
        };
        store.insert_generation(&generation).await?;
        if let Some(next_state) = next_state {
            character.state = codex_game_domain::advance_character(character.state, next_state)
                .map_err(|error| GameServiceError::InvalidCharacterOperation(error.to_string()))?;
            character.updated_at = now();
            store.update_character(&character).await?;
            write_character_file(&project, &character)?;
        }
        Ok(generation)
    }

    pub async fn list_generations(
        &self,
        project_id: &str,
        character_id: &str,
        stage: Option<&str>,
    ) -> Result<Vec<Generation>, GameServiceError> {
        let store = self.project_store(project_id, false)?;
        store
            .list_generations(project_id, "character", character_id, stage)
            .await
            .map_err(Into::into)
    }

    pub async fn confirm_character_render(
        &self,
        project_id: &str,
        character_id: &str,
        generation_id: &str,
    ) -> Result<Character, GameServiceError> {
        let project = self.read_project(project_id)?;
        let store = self.project_store(project_id, true)?;
        let mut character = self.read_character(project_id, character_id).await?;
        if character.state != CharacterState::S2RenderGenerated {
            return Err(GameServiceError::InvalidCharacterOperation(
                "只能在 S2 确认渲染图".to_string(),
            ));
        }
        let generation = store
            .list_generations(project_id, "character", character_id, Some("render"))
            .await?
            .into_iter()
            .find(|generation| generation.id == generation_id)
            .ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation("渲染记录不存在".to_string())
            })?;
        let source = safe_project_path(&project.root, &generation.file_path)?;
        if !source.is_file() {
            return Err(GameServiceError::InvalidCharacterOperation(
                "渲染文件不存在".to_string(),
            ));
        }
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        let relative =
            PathBuf::from(&character.dir_name).join(format!("images/render-final.{extension}"));
        let target = Path::new(&project.root).join(&relative);
        let meta_path = Path::new(&project.root)
            .join(&character.dir_name)
            .join(".model.json");
        character.state = codex_game_domain::advance_character(
            character.state,
            CharacterState::S3RenderConfirmed,
        )
        .map_err(|error| GameServiceError::InvalidCharacterOperation(error.to_string()))?;
        character.render_path = Some(relative.to_string_lossy().into_owned());
        let timestamp = now();
        character.gate_render_confirmed_at = Some(timestamp);
        character.updated_at = timestamp;
        let backups = vec![
            (target.clone(), fs::read(&target).ok()),
            (meta_path.clone(), fs::read(&meta_path).ok()),
        ];
        if let Err(error) = fs::copy(source, &target)
            .map(|_| ())
            .map_err(GameServiceError::from)
            .and_then(|_| {
                write_character_file(&project, &character).map_err(GameServiceError::from)
            })
        {
            restore_files(&backups);
            return Err(error);
        }
        let generation_ids = [generation_id.to_string()];
        if let Err(error) = store
            .commit_character_generation_gate("render", &generation_ids, &character, timestamp)
            .await
        {
            restore_files(&backups);
            return Err(error.into());
        }
        Ok(character)
    }

    pub async fn confirm_character_views(
        &self,
        project_id: &str,
        character_id: &str,
        generation_ids: &[String],
    ) -> Result<Character, GameServiceError> {
        let project = self.read_project(project_id)?;
        let store = self.project_store(project_id, true)?;
        let mut character = self.read_character(project_id, character_id).await?;
        if character.state != CharacterState::S4ViewsGenerated {
            return Err(GameServiceError::InvalidCharacterOperation(
                "只能在 S4 确认四视图".to_string(),
            ));
        }
        if generation_ids.len() != 1 {
            return Err(GameServiceError::InvalidCharacterOperation(
                "必须选择一张完整的 2×2 四视图候选".to_string(),
            ));
        }
        let available = store
            .list_generations(project_id, "character", character_id, Some("views"))
            .await?;
        let generation = available
            .iter()
            .find(|generation| generation.id == generation_ids[0])
            .ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation("四视图记录不存在".to_string())
            })?;
        if generation.variant.as_deref() != Some("quad") {
            return Err(GameServiceError::InvalidCharacterOperation(
                "四视图候选必须是完整的 2×2 四宫格".to_string(),
            ));
        }
        let source = safe_project_path(&project.root, &generation.file_path)?;
        if !source.is_file() {
            return Err(GameServiceError::InvalidCharacterOperation(format!(
                "四视图文件不存在：{}",
                generation.file_path
            )));
        }
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        let relative =
            PathBuf::from(&character.dir_name).join(format!("images/views-final.{extension}"));
        let target = Path::new(&project.root).join(&relative);
        let meta_path = Path::new(&project.root)
            .join(&character.dir_name)
            .join(".model.json");
        let archived = relative.to_string_lossy().into_owned();
        let selected = ["front", "right", "back", "left"]
            .into_iter()
            .map(|view| (view.to_string(), archived.clone()))
            .collect::<BTreeMap<_, _>>();
        character.state =
            codex_game_domain::advance_character(character.state, CharacterState::S5ViewsConfirmed)
                .map_err(|error| GameServiceError::InvalidCharacterOperation(error.to_string()))?;
        character.view_paths = selected;
        let timestamp = now();
        character.gate_views_confirmed_at = Some(timestamp);
        character.updated_at = timestamp;
        let backups = vec![
            (target.clone(), fs::read(&target).ok()),
            (meta_path.clone(), fs::read(&meta_path).ok()),
        ];
        if let Err(error) = fs::copy(source, &target)
            .map(|_| ())
            .map_err(GameServiceError::from)
            .and_then(|_| {
                write_character_file(&project, &character).map_err(GameServiceError::from)
            })
        {
            restore_files(&backups);
            return Err(error);
        }
        if let Err(error) = store
            .commit_character_generation_gate("views", generation_ids, &character, timestamp)
            .await
        {
            restore_files(&backups);
            return Err(error.into());
        }
        Ok(character)
    }

    pub async fn reject_character_stage(
        &self,
        project_id: &str,
        character_id: &str,
        stage: &str,
        reason: String,
    ) -> Result<Character, GameServiceError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(GameServiceError::InvalidCharacterOperation(
                "拒绝当前结果时必须说明原因".to_string(),
            ));
        }
        let character = self.read_character(project_id, character_id).await?;
        let expected = match stage {
            "spec" => CharacterState::S0SpecDrafting,
            "render" => CharacterState::S2RenderGenerated,
            "views" => CharacterState::S4ViewsGenerated,
            _ => {
                return Err(GameServiceError::InvalidCharacterOperation(
                    "拒绝阶段必须是 spec、render 或 views".to_string(),
                ));
            }
        };
        if character.state != expected {
            return Err(GameServiceError::InvalidCharacterOperation(format!(
                "当前角色状态不允许拒绝 {stage} 结果"
            )));
        }
        let store = self.project_store(project_id, true)?;
        store
            .record_character_rejection(&character, stage, reason, now())
            .await?;
        Ok(character)
    }

    async fn collect_verdict_constraints(
        &self,
        project_id: &str,
        character_id: &str,
        token: &str,
        subject_id: &str,
    ) -> Result<Vec<serde_json::Value>, GameServiceError> {
        let conversation = self.find_target_conversation(
            project_id,
            ConversationTargetKind::Character,
            Some(character_id),
        )?;
        let snapshot = self.read_conversation(conversation.id.as_str()).await?;
        let verdict = snapshot
            .messages
            .iter()
            .rev()
            .filter_map(|message| message.action.as_ref()?.payload.verdict.as_ref())
            .find(|verdict| verdict.token == token && verdict.subject_id == subject_id)
            .ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation(
                    "缺少与当前角色设定草稿匹配的审校结论".to_string(),
                )
            })?;
        if !matches!(verdict.decision.as_str(), "APPROVE" | "CONCERNS") {
            return Err(GameServiceError::InvalidCharacterOperation(
                "当前角色设定草稿未通过审校，不能确认".to_string(),
            ));
        }
        verdict
            .constraints
            .iter()
            .map(|constraint| {
                serde_json::to_value(constraint)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error).into())
            })
            .collect()
    }

    fn project_store(
        &self,
        project_id: &str,
        writable: bool,
    ) -> Result<Arc<ProjectStore>, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = projects
            .get(project_id)
            .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
        if writable {
            require_writable(session)?;
        }
        Ok(Arc::clone(&session.store))
    }

    fn find_target_conversation(
        &self,
        project_id: &str,
        target_kind: ConversationTargetKind,
        target_ref: Option<&str>,
    ) -> Result<Conversation, GameServiceError> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| GameServiceError::StateUnavailable)?;
        let session = projects
            .get(project_id)
            .ok_or_else(|| GameServiceError::ProjectNotFound(project_id.to_string()))?;
        session
            .conversations
            .values()
            .find(|snapshot| {
                snapshot.conversation.target_kind == target_kind
                    && snapshot.conversation.target_ref.as_deref() == target_ref
            })
            .map(|snapshot| snapshot.conversation.clone())
            .ok_or_else(|| {
                GameServiceError::ConversationNotFound(target_ref.unwrap_or(project_id).to_string())
            })
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

fn project_state_name(state: ProjectState) -> &'static str {
    match state {
        ProjectState::Drafting => "drafting",
        ProjectState::StyleSettled => "styleSettled",
        ProjectState::Ready => "ready",
    }
}

fn validate_draft_baseline(path: &Path, expected: Option<&str>) -> Result<(), GameServiceError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let current = fs::read(path).unwrap_or_default();
    if bytes_hash(&current) != expected {
        return Err(GameServiceError::InvalidAction(
            "草稿基线已变化，请刷新后重新生成".to_string(),
        ));
    }
    Ok(())
}

fn character_workflow_facts(
    character: &Character,
    messages: &[ConversationMessage],
    drafts: &[ArtifactDraftRecord],
    tasks: &[Task],
    generations: &[Generation],
    memories: &[ProjectMemory],
) -> CharacterWorkflowFacts {
    let latest_spec_rejection = memories
        .iter()
        .filter(|memory| {
            memory.character_ref.as_deref() == Some(character.id.as_str())
                && memory.kind == "spec_rejection"
        })
        .max_by_key(|memory| (memory.updated_at, memory.id.as_str()));
    let pending_spec = drafts
        .iter()
        .filter(|draft| draft.status == "pending" && draft.target_path == "docs/角色定稿.md")
        .filter(|draft| {
            latest_spec_rejection.is_none_or(|rejection| {
                (draft.created_at, draft.id.as_str())
                    > (rejection.updated_at, rejection.id.as_str())
            })
        })
        .max_by_key(|draft| (draft.created_at, draft.id.as_str()))
        .cloned();
    let latest_spec_verdict = messages
        .iter()
        .rev()
        .filter_map(|message| message.action.as_ref()?.payload.verdict.as_ref())
        .find(|verdict| verdict.token == "SPEC-CHECK")
        .cloned();
    let matching_verdict = pending_spec.as_ref().and_then(|draft| {
        messages
            .iter()
            .rev()
            .filter_map(|message| message.action.as_ref()?.payload.verdict.as_ref())
            .find(|verdict| verdict.token == "SPEC-CHECK" && verdict.subject_id == draft.id)
    });
    let latest_spec_task = tasks.iter().rev().find(|task| {
        task.target_id == character.id
            && task.stage == "spec"
            && matches!(task.agent_code.as_str(), "spec_writer" | "spec_reviewer")
    });
    let spec_review_status = if pending_spec.is_none() {
        SpecReviewStatus::AwaitingDraft
    } else if let Some(verdict) = matching_verdict {
        match verdict.decision.as_str() {
            "APPROVE" => SpecReviewStatus::Approved,
            "CONCERNS" => SpecReviewStatus::Concerns,
            "REJECT" => SpecReviewStatus::Rejected,
            _ => SpecReviewStatus::Error,
        }
    } else if latest_spec_task.is_some_and(|task| task.agent_code == "spec_reviewer") {
        match latest_spec_task.map(|task| task.status) {
            Some(TaskStatus::Pending | TaskStatus::Running) => SpecReviewStatus::Pending,
            Some(TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled) | None => {
                SpecReviewStatus::Error
            }
        }
    } else {
        SpecReviewStatus::Pending
    };
    let rejection_after_generation = |stage: &str| {
        let latest_generation = generations
            .iter()
            .filter(|generation| generation.stage == stage)
            .max_by_key(|generation| (generation.created_at, generation.id.as_str()));
        memories
            .iter()
            .filter(|memory| {
                memory.character_ref.as_deref() == Some(character.id.as_str())
                    && memory.kind == format!("{stage}_rejection")
            })
            .max_by_key(|memory| (memory.updated_at, memory.id.as_str()))
            .filter(|rejected| {
                latest_generation.is_some_and(|generated| {
                    (rejected.updated_at, rejected.id.as_str())
                        > (generated.created_at, generated.id.as_str())
                })
            })
    };
    let render_rejection = rejection_after_generation("render");
    let views_rejection = rejection_after_generation("views");
    let render_rejected_after_generation = render_rejection.is_some();
    let views_rejected_after_generation = views_rejection.is_some();
    let current_design_stage = match character.state {
        CharacterState::S0SpecDrafting => Some("spec"),
        CharacterState::S1SpecConfirmed => Some("render"),
        CharacterState::S2RenderGenerated if render_rejected_after_generation => Some("render"),
        CharacterState::S3RenderConfirmed => Some("views"),
        CharacterState::S4ViewsGenerated if views_rejected_after_generation => Some("views"),
        CharacterState::S2RenderGenerated
        | CharacterState::S4ViewsGenerated
        | CharacterState::S5ViewsConfirmed => None,
    };
    let failed_design_stage = current_design_stage
        .filter(|stage| {
            tasks
                .iter()
                .rev()
                .find(|task| task.target_id == character.id && task.stage == *stage)
                .is_some_and(|task| task.status == TaskStatus::Failed)
        })
        .map(str::to_string);

    CharacterWorkflowFacts {
        pending_spec,
        spec_review_status,
        latest_spec_verdict,
        latest_spec_rejection_id: latest_spec_rejection.map(|memory| memory.id.clone()),
        render_rejection_id: render_rejection.map(|memory| memory.id.clone()),
        views_rejection_id: views_rejection.map(|memory| memory.id.clone()),
        render_rejected_after_generation,
        views_rejected_after_generation,
        failed_design_stage,
        has_running_task: tasks
            .iter()
            .any(|task| matches!(task.status, TaskStatus::Pending | TaskStatus::Running)),
    }
}

fn build_character_workflow_progress(
    state: CharacterState,
    spec_confirmation_ready: bool,
    spec_review_failed: bool,
    render_rejected_after_generation: bool,
    views_rejected_after_generation: bool,
    failed_design_stage: Option<&str>,
    continuation_key: Option<String>,
) -> CharacterWorkflowProgress {
    let mut statuses = ["wait"; 6];
    let current_design = match state {
        CharacterState::S0SpecDrafting => {
            if spec_confirmation_ready {
                statuses[0] = "finish";
                statuses[1] = "process";
                None
            } else {
                statuses[0] = "process";
                Some((0, "spec"))
            }
        }
        CharacterState::S1SpecConfirmed => {
            statuses[..2].fill("finish");
            statuses[2] = "process";
            Some((2, "render"))
        }
        CharacterState::S2RenderGenerated => {
            statuses[..3].fill("finish");
            statuses[3] = "process";
            if render_rejected_after_generation {
                statuses[2] = "process";
                statuses[3] = "wait";
                Some((2, "render"))
            } else {
                None
            }
        }
        CharacterState::S3RenderConfirmed => {
            statuses[..4].fill("finish");
            statuses[4] = "process";
            Some((4, "views"))
        }
        CharacterState::S4ViewsGenerated => {
            statuses[..5].fill("finish");
            statuses[5] = "process";
            if views_rejected_after_generation {
                statuses[4] = "process";
                statuses[5] = "wait";
                Some((4, "views"))
            } else {
                None
            }
        }
        CharacterState::S5ViewsConfirmed => {
            statuses.fill("finish");
            None
        }
    };
    if let Some((index, stage)) = current_design
        && failed_design_stage == Some(stage)
    {
        statuses[index] = "error";
    }
    if state == CharacterState::S0SpecDrafting && spec_review_failed {
        statuses[0] = "error";
    }
    let labels = [
        ("spec_design", "角色设定"),
        ("spec_confirm", "确认设定"),
        ("render_design", "效果图设计"),
        ("render_confirm", "确认效果图"),
        ("views_design", "四视图设计"),
        ("views_confirm", "确认四视图"),
    ];
    CharacterWorkflowProgress {
        status_label: if state == CharacterState::S5ViewsConfirmed {
            "角色视觉设计完成".to_string()
        } else {
            labels
                .iter()
                .zip(statuses)
                .find(|(_, status)| *status == "process" || *status == "error")
                .map(|((_, label), _)| (*label).to_string())
                .unwrap_or_else(|| "角色视觉设计".to_string())
        },
        steps: labels
            .into_iter()
            .zip(statuses)
            .map(|((key, label), status)| CharacterWorkflowStep {
                key: key.to_string(),
                label: label.to_string(),
                status: status.to_string(),
            })
            .collect(),
        needs_resume: continuation_key.is_some(),
        continuation_key,
    }
}

fn content_hash(content: &str) -> String {
    bytes_hash(content.as_bytes())
}

fn bytes_hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn restore_file(path: &Path, previous: Option<&str>) {
    if let Some(previous) = previous {
        let _ = write_art_bible(path, previous);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn restore_files(backups: &[(PathBuf, Option<Vec<u8>>)]) {
    for (path, previous) in backups {
        if let Some(previous) = previous {
            let _ = fs::write(path, previous);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

fn is_valid_code(code: &str) -> bool {
    let mut chars = code.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    code.len() <= 64
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}

fn safe_segment(value: &str) -> Result<String, GameServiceError> {
    let value = value.trim();
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(GameServiceError::InvalidCharacterOperation(
            "名称不能用于目录".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn safe_project_path(root: &str, relative: &str) -> Result<PathBuf, GameServiceError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(GameServiceError::InvalidCharacterOperation(
            "文件路径必须位于项目内".to_string(),
        ));
    }
    Ok(Path::new(root).join(relative))
}

fn append_line_once(path: &Path, line: &str) -> Result<(), io::Error> {
    let mut content = fs::read_to_string(path).unwrap_or_default();
    if content.lines().any(|existing| existing == line) {
        return Ok(());
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(line);
    content.push('\n');
    fs::write(path, content)
}

fn allowed_handoffs_for(
    target_kind: &str,
    stage: &str,
    current_agent: &str,
    director_agent: &str,
    handoff_count: usize,
    character: Option<&Character>,
    workflow: Option<&CharacterWorkflowFacts>,
) -> Vec<String> {
    if handoff_count >= MAX_HANDOFFS {
        return Vec::new();
    }
    if current_agent != director_agent {
        return vec![director_agent.to_string()];
    }
    if target_kind != "character" {
        return codex_game_domain::agents_for_stage(target_kind, stage)
            .iter()
            .filter(|agent| **agent != director_agent)
            .map(|agent| (*agent).to_string())
            .collect();
    }
    let (Some(character), Some(workflow)) = (character, workflow) else {
        return Vec::new();
    };
    let target = match character.state {
        CharacterState::S0SpecDrafting => match workflow.spec_review_status {
            SpecReviewStatus::AwaitingDraft | SpecReviewStatus::Rejected => Some("spec_writer"),
            SpecReviewStatus::Pending | SpecReviewStatus::Error => Some("spec_reviewer"),
            SpecReviewStatus::Approved | SpecReviewStatus::Concerns => None,
        },
        CharacterState::S1SpecConfirmed | CharacterState::S3RenderConfirmed => {
            Some("visual_designer")
        }
        CharacterState::S2RenderGenerated if workflow.render_rejected_after_generation => {
            Some("visual_designer")
        }
        CharacterState::S4ViewsGenerated if workflow.views_rejected_after_generation => {
            Some("visual_designer")
        }
        CharacterState::S2RenderGenerated
        | CharacterState::S4ViewsGenerated
        | CharacterState::S5ViewsConfirmed => None,
    };
    target.into_iter().map(str::to_string).collect()
}

fn action_protocol_instruction(current_agent: &str, director_agent: &str) -> String {
    let control_flow = if current_agent == director_agent {
        "你是当前会话总管，只有你可以决定并 handoff 给下一位专业 Agent；无需继续派单时才可使用 done。"
            .to_string()
    } else {
        format!(
            "你是专业 Agent，只负责当前任务；工作完成后必须 handoff 回总管 {director_agent}，不得直接 done，也不得 handoff 给其他专业 Agent。"
        )
    };
    format!(
        "每次回复最后必须且只能包含一个 {start} JSON {end} 块。顶层只允许 action、target_agent、reason、payload；action 只允许 ask_user、handoff、done、blocked。handoff 目标必须来自 allowed_handoffs，其他动作 target_agent 必须为 null。{control_flow} 一次只允许一个待用户完成的交互阶段：payload.choices 与非空 payload.drafts 不得同轮出现；存在待确认问题时只输出 choices，用户完成选择后的下一轮才能输出 drafts 进入最终确认。",
        start = codex_game_domain::ACTION_START,
        end = codex_game_domain::ACTION_END,
    )
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
    use tempfile::tempdir;

    fn expected_workflow(status_label: &str, statuses: [&str; 6]) -> CharacterWorkflowProgress {
        let labels = [
            ("spec_design", "角色设定"),
            ("spec_confirm", "确认设定"),
            ("render_design", "效果图设计"),
            ("render_confirm", "确认效果图"),
            ("views_design", "四视图设计"),
            ("views_confirm", "确认四视图"),
        ];
        CharacterWorkflowProgress {
            status_label: status_label.to_string(),
            steps: labels
                .into_iter()
                .zip(statuses)
                .map(|((key, label), status)| CharacterWorkflowStep {
                    key: key.to_string(),
                    label: label.to_string(),
                    status: status.to_string(),
                })
                .collect(),
            needs_resume: false,
            continuation_key: None,
        }
    }

    fn test_character(state: CharacterState) -> Character {
        Character {
            id: "character-1".to_string(),
            project_id: "project-1".to_string(),
            name: "角色".to_string(),
            group: None,
            dir_name: "character-1".to_string(),
            state,
            spec_path: None,
            render_path: None,
            view_paths: BTreeMap::new(),
            hard_constraints: Vec::new(),
            gate_spec_confirmed_at: None,
            gate_render_confirmed_at: None,
            gate_views_confirmed_at: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn test_workflow_facts(spec_review_status: SpecReviewStatus) -> CharacterWorkflowFacts {
        CharacterWorkflowFacts {
            pending_spec: None,
            spec_review_status,
            latest_spec_verdict: None,
            latest_spec_rejection_id: None,
            render_rejection_id: None,
            views_rejection_id: None,
            render_rejected_after_generation: false,
            views_rejected_after_generation: false,
            failed_design_stage: None,
            has_running_task: false,
        }
    }

    #[test]
    fn character_workflow_maps_persistent_states_to_six_business_steps() {
        let cases = [
            (
                CharacterState::S0SpecDrafting,
                false,
                "角色设定",
                ["process", "wait", "wait", "wait", "wait", "wait"],
            ),
            (
                CharacterState::S0SpecDrafting,
                true,
                "确认设定",
                ["finish", "process", "wait", "wait", "wait", "wait"],
            ),
            (
                CharacterState::S1SpecConfirmed,
                false,
                "效果图设计",
                ["finish", "finish", "process", "wait", "wait", "wait"],
            ),
            (
                CharacterState::S2RenderGenerated,
                false,
                "确认效果图",
                ["finish", "finish", "finish", "process", "wait", "wait"],
            ),
            (
                CharacterState::S3RenderConfirmed,
                false,
                "四视图设计",
                ["finish", "finish", "finish", "finish", "process", "wait"],
            ),
            (
                CharacterState::S4ViewsGenerated,
                false,
                "确认四视图",
                ["finish", "finish", "finish", "finish", "finish", "process"],
            ),
            (
                CharacterState::S5ViewsConfirmed,
                false,
                "角色视觉设计完成",
                ["finish", "finish", "finish", "finish", "finish", "finish"],
            ),
        ];

        for (state, spec_confirmation_ready, status_label, statuses) in cases {
            assert_eq!(
                build_character_workflow_progress(
                    state,
                    spec_confirmation_ready,
                    false,
                    false,
                    false,
                    None,
                    None,
                ),
                expected_workflow(status_label, statuses)
            );
        }
    }

    #[test]
    fn character_handoffs_follow_review_and_gate_state() {
        let character = test_character(CharacterState::S0SpecDrafting);
        let cases = [
            (SpecReviewStatus::AwaitingDraft, vec!["spec_writer"]),
            (SpecReviewStatus::Pending, vec!["spec_reviewer"]),
            (SpecReviewStatus::Error, vec!["spec_reviewer"]),
            (SpecReviewStatus::Rejected, vec!["spec_writer"]),
            (SpecReviewStatus::Approved, Vec::new()),
            (SpecReviewStatus::Concerns, Vec::new()),
        ];
        for (status, expected) in cases {
            let facts = test_workflow_facts(status);
            assert_eq!(
                allowed_handoffs_for(
                    "character",
                    "spec",
                    "studio_director",
                    "studio_director",
                    0,
                    Some(&character),
                    Some(&facts),
                ),
                expected
            );
        }

        let mut character = test_character(CharacterState::S2RenderGenerated);
        let mut facts = test_workflow_facts(SpecReviewStatus::Approved);
        assert!(
            allowed_handoffs_for(
                "character",
                "render",
                "studio_director",
                "studio_director",
                0,
                Some(&character),
                Some(&facts),
            )
            .is_empty()
        );
        facts.render_rejected_after_generation = true;
        assert_eq!(
            allowed_handoffs_for(
                "character",
                "render",
                "studio_director",
                "studio_director",
                0,
                Some(&character),
                Some(&facts),
            ),
            vec!["visual_designer"]
        );
        character.state = CharacterState::S5ViewsConfirmed;
        assert!(
            allowed_handoffs_for(
                "character",
                "views",
                "studio_director",
                "studio_director",
                0,
                Some(&character),
                Some(&facts),
            )
            .is_empty()
        );
    }

    #[test]
    fn spec_review_verdict_matches_the_pending_draft_id() {
        let character = test_character(CharacterState::S0SpecDrafting);
        let draft = ArtifactDraftRecord {
            id: "draft-current".to_string(),
            conversation_id: "conversation-1".to_string(),
            target_path: "docs/角色定稿.md".to_string(),
            content: "# 角色设定".to_string(),
            based_on_hash: None,
            status: "pending".to_string(),
            created_at: 2,
        };
        let verdict_message =
            |id: &str, subject_id: &str, decision: &str, created_at| ConversationMessage {
                id: id.to_string(),
                conversation_id: ConversationId::new("conversation-1"),
                turn: 1,
                role: "assistant".to_string(),
                content: String::new(),
                agent_code: "spec_reviewer".to_string(),
                recipient_agent_code: None,
                status: MessageStatus::Completed,
                token_count: 0,
                folded: false,
                attachments: Vec::new(),
                action: Some(AgentAction {
                    action: AgentActionKind::Handoff,
                    target_agent: Some("studio_director".to_string()),
                    reason: "审校完成".to_string(),
                    payload: AgentActionPayload {
                        verdict: Some(AgentVerdict {
                            token: "SPEC-CHECK".to_string(),
                            subject_id: subject_id.to_string(),
                            decision: decision.to_string(),
                            sections: BTreeMap::new(),
                            constraints: Vec::new(),
                        }),
                        ..AgentActionPayload::default()
                    },
                }),
                created_at,
            };
        let messages = vec![
            verdict_message("matching", "draft-current", "CONCERNS", 3),
            verdict_message("stale", "draft-old", "REJECT", 4),
        ];

        let facts = character_workflow_facts(&character, &messages, &[draft], &[], &[], &[]);

        assert_eq!(facts.spec_review_status, SpecReviewStatus::Concerns);
        assert_eq!(
            facts
                .pending_spec
                .as_ref()
                .map(|pending| pending.id.as_str()),
            Some("draft-current")
        );
        assert_eq!(
            facts
                .latest_spec_verdict
                .as_ref()
                .map(|verdict| verdict.subject_id.as_str()),
            Some("draft-old")
        );
    }

    #[test]
    fn workflow_progress_reports_resume_and_review_failure() {
        let resumable = build_character_workflow_progress(
            CharacterState::S1SpecConfirmed,
            false,
            false,
            false,
            false,
            None,
            Some("render:1".to_string()),
        );
        assert!(resumable.needs_resume);
        assert_eq!(resumable.continuation_key.as_deref(), Some("render:1"));

        let review_failed = build_character_workflow_progress(
            CharacterState::S0SpecDrafting,
            false,
            true,
            false,
            false,
            None,
            None,
        );
        assert_eq!(review_failed.steps[0].status, "error");
        assert!(!review_failed.needs_resume);

        let terminal = build_character_workflow_progress(
            CharacterState::S5ViewsConfirmed,
            false,
            false,
            false,
            false,
            None,
            None,
        );
        assert!(!terminal.needs_resume);
        assert!(terminal.continuation_key.is_none());
    }

    #[test]
    fn character_workflow_returns_to_design_after_rejection_and_marks_failures() {
        assert_eq!(
            build_character_workflow_progress(
                CharacterState::S2RenderGenerated,
                false,
                false,
                true,
                false,
                Some("render"),
                None,
            ),
            expected_workflow(
                "效果图设计",
                ["finish", "finish", "error", "wait", "wait", "wait"],
            )
        );
        assert_eq!(
            build_character_workflow_progress(
                CharacterState::S4ViewsGenerated,
                false,
                false,
                false,
                true,
                None,
                None,
            ),
            expected_workflow(
                "四视图设计",
                ["finish", "finish", "finish", "finish", "process", "wait"],
            )
        );
    }

    #[tokio::test]
    async fn project_gate_blocks_character_creation_before_finalize() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path().join("game");
        let service = GameService::new(directory.path().join("app"));
        let project = service
            .create_project(
                Uuid::now_v7().to_string(),
                "Untitled".to_string(),
                root.to_string_lossy().into_owned(),
            )
            .await
            .expect("create project");

        let result = service
            .create_character(project.id.as_str(), "Hero".to_string(), None, false)
            .await;

        assert!(matches!(result, Err(GameServiceError::ProjectGate(_))));
    }
}
