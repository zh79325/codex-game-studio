use codex_game_domain::AgentAction;
use codex_game_domain::AgentActionKind;
use codex_game_domain::AgentActionPayload;
use codex_game_domain::AgentHandoff;
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
use codex_game_domain::ProjectState;
use codex_game_domain::TaskAttemptStatus;
use codex_game_store::ProjectAccess;
use codex_game_store::ProjectStore;
use codex_game_store::StoreError;
use codex_game_store::finalize_project_json;
use codex_game_store::list_registered_projects;
use codex_game_store::open_studio_store;
use codex_game_store::register_project as register_studio_project;
use codex_game_store::unregister_project;
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
pub struct CompletedTaskAttempt {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub status: TaskAttemptStatus,
    pub agent_code: Option<String>,
    pub action: Option<AgentAction>,
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
    ) -> Result<Option<CompletedTaskAttempt>, GameServiceError> {
        for store in self.writable_stores()? {
            let Some(context) = store.turn_attempt_context(codex_turn_id).await? else {
                continue;
            };
            let (assistant_message_id, allowed_handoffs) = {
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
                let handoff_count = snapshot
                    .handoffs
                    .iter()
                    .filter(|handoff| handoff.turn == snapshot.conversation.turn)
                    .count();
                let stage_agents = codex_game_domain::agents_for_stage(
                    snapshot.conversation.target_kind.as_str(),
                    &context.stage,
                );
                let allowed = if handoff_count >= MAX_HANDOFFS {
                    Vec::new()
                } else {
                    stage_agents
                        .iter()
                        .copied()
                        .filter(|agent| *agent != context.agent_code)
                        .collect()
                };
                (assistant.id.clone(), allowed)
            };
            let parsed = output
                .ok_or_else(|| "Agent 未返回输出".to_string())
                .and_then(|value| {
                    super::parse_agent_turn(value, &context.agent_code, &allowed_handoffs)
                        .map_err(|error| error.to_string())
                });
            let parsed = match parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    let message = format!("Action 协议校验失败：{error}");
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
                    return Ok(completion.map(|completion| CompletedTaskAttempt {
                        attempt_id: completion.attempt_id,
                        task_id: completion.task_id,
                        conversation_id: completion.conversation_id,
                        status: TaskAttemptStatus::Failed,
                        agent_code: Some(context.agent_code),
                        action: None,
                    }));
                }
            };
            let (generations, updated_character) = match self
                .prepare_action_generations(&context, &parsed.action, &store)
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
                    return Ok(completion.map(|completion| CompletedTaskAttempt {
                        attempt_id: completion.attempt_id,
                        task_id: completion.task_id,
                        conversation_id: completion.conversation_id,
                        status: TaskAttemptStatus::Failed,
                        agent_code: Some(context.agent_code),
                        action: None,
                    }));
                }
            };
            let meta_backup = if let Some(character) = updated_character.as_ref() {
                let project = self.read_project(&character.project_id)?;
                let path = Path::new(&project.root)
                    .join(&character.dir_name)
                    .join(".model.json");
                let previous = fs::read_to_string(&path).ok();
                if let Err(error) = write_character_meta(&project, character) {
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
                    return Ok(completion.map(|completion| CompletedTaskAttempt {
                        attempt_id: completion.attempt_id,
                        task_id: completion.task_id,
                        conversation_id: completion.conversation_id,
                        status: TaskAttemptStatus::Failed,
                        agent_code: Some(context.agent_code),
                        action: None,
                    }));
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
            return Ok(Some(CompletedTaskAttempt {
                attempt_id: completion.attempt_id,
                task_id: completion.task_id,
                conversation_id: completion.conversation_id,
                status: TaskAttemptStatus::Succeeded,
                agent_code: Some(context.agent_code),
                action: Some(parsed.action),
            }));
        }
        Ok(None)
    }

    async fn prepare_action_generations(
        &self,
        context: &codex_game_store::TurnAttemptContext,
        action: &AgentAction,
        store: &ProjectStore,
    ) -> Result<(Vec<Generation>, Option<Character>), GameServiceError> {
        if !matches!(context.agent_code.as_str(), "image_t2i" | "image_i2i") {
            return Ok((Vec::new(), None));
        }
        let result = action.payload.result.as_ref().ok_or_else(|| {
            GameServiceError::InvalidAction("图片执行 Agent 必须返回 payload.result".to_string())
        })?;
        if action.action == AgentActionKind::Blocked {
            return Ok((Vec::new(), None));
        }
        if result.artifacts.is_empty() {
            return Err(GameServiceError::InvalidAction(
                "图片执行成功时必须返回至少一个产物".to_string(),
            ));
        }
        if !matches!(context.stage.as_str(), "render" | "views") {
            return Err(GameServiceError::InvalidAction(
                "图片产物只能登记到 render 或 views 阶段".to_string(),
            ));
        }
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
        let mut character = store
            .read_character(project.id.as_str(), &context.target_id)
            .await?
            .ok_or_else(|| GameServiceError::CharacterNotFound(context.target_id.clone()))?;
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
            generations.push(Generation {
                id: Uuid::now_v7().to_string(),
                project_id: project.id.as_str().to_string(),
                target_kind: "character".to_string(),
                target_ref: context.target_id.clone(),
                stage: context.stage.clone(),
                variant,
                file_path,
                file_hash: Some(bytes_hash(&fs::read(&path)?)),
                is_final: false,
                source: context.agent_code.clone(),
                task_id: Some(context.task_id.clone()),
                asset_spec: serde_json::to_value(artifact)
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
                "该会话已有一轮正在运行".to_string(),
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
                store
                    .read_character(project.id.as_str(), character_id)
                    .await?
                    .ok_or_else(|| GameServiceError::CharacterNotFound(character_id.to_string()))?
                    .state
                    .stage()
                    .to_string()
            }
        };
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
            snapshot.conversation.turn += 1;
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
                folded: false,
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
        let latest_action = snapshot
            .messages
            .iter()
            .rev()
            .find_map(|message| message.action.as_ref())
            .ok_or_else(|| GameServiceError::InvalidAction("没有可继续的 handoff".to_string()))?;
        if latest_action.action != AgentActionKind::Handoff
            || latest_action.target_agent.as_deref() != Some(target_agent)
        {
            return Err(GameServiceError::InvalidAction(
                "handoff 目标与上一条 Action 不一致".to_string(),
            ));
        }
        let handoff_count = snapshot
            .handoffs
            .iter()
            .filter(|handoff| handoff.turn == snapshot.conversation.turn)
            .count();
        if handoff_count >= MAX_HANDOFFS {
            return Err(GameServiceError::InvalidAction(
                "单轮自动 handoff 不得超过两次".to_string(),
            ));
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
                store
                    .read_character(project.id.as_str(), character_id)
                    .await?
                    .ok_or_else(|| GameServiceError::CharacterNotFound(character_id.to_string()))?
                    .state
                    .stage()
                    .to_string()
            }
        };
        let allowed =
            codex_game_domain::agents_for_stage(snapshot.conversation.target_kind.as_str(), &stage);
        if !allowed.contains(&target_agent) {
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
        let character_context = if prepared.conversation.target_kind
            == ConversationTargetKind::Character
        {
            let character_id = prepared.conversation.target_ref.as_deref().ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation("角色会话缺少 targetRef".to_string())
            })?;
            prepared
                .store
                .read_character(prepared.project.id.as_str(), character_id)
                .await?
                .map(|character| serde_json::to_string(&character))
                .transpose()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        } else {
            None
        };
        let handoff_count = snapshot
            .handoffs
            .iter()
            .filter(|handoff| handoff.turn == prepared.conversation.turn)
            .count();
        let allowed_handoffs = if handoff_count >= MAX_HANDOFFS {
            Vec::new()
        } else {
            codex_game_domain::agents_for_stage(
                prepared.conversation.target_kind.as_str(),
                &prepared.stage,
            )
            .iter()
            .filter(|agent| **agent != prepared.agent_code.as_str())
            .map(|agent| (*agent).to_string())
            .collect::<Vec<_>>()
        };
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
        let memories = prepared
            .store
            .list_project_memories(
                prepared.project.id.as_str(),
                prepared.conversation.target_ref.as_deref(),
            )
            .await?
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
            memories,
            allowed_handoffs,
            action_protocol: action_protocol_instruction(),
        })
    }

    pub fn inspect_project_dir(&self, root: &str) -> Result<ProjectDirState, GameServiceError> {
        let root = absolute_root(root)?;
        let project_json = root.join("project.json");
        let occupied = project_json.exists() || root.join(".codex-game/local/project.db").exists();
        if !project_json.exists() {
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
    ) -> Result<(), GameServiceError> {
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
        Ok(())
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
    ) -> Result<Vec<Character>, GameServiceError> {
        let store = self.project_store(project_id, false)?;
        store.list_characters(project_id).await.map_err(Into::into)
    }

    pub async fn read_character(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<Character, GameServiceError> {
        let store = self.project_store(project_id, false)?;
        store
            .read_character(project_id, character_id)
            .await?
            .ok_or_else(|| GameServiceError::CharacterNotFound(character_id.to_string()))
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
            id: Uuid::now_v7().to_string(),
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
        write_character_meta(&project, &character)?;
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
            .require_approved_verdict(project_id, character_id, "SPEC-CHECK", draft.created_at)
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
            .and_then(|_| write_character_meta(&project, &character))
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
            write_character_meta(&project, &character)?;
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
        self.require_approved_verdict(
            project_id,
            character_id,
            "VIEW-CHECK",
            generation.created_at,
        )
        .await?;
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
            .and_then(|_| write_character_meta(&project, &character))
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
        self.require_approved_verdict(
            project_id,
            character_id,
            "VIEW-CHECK",
            generation.created_at,
        )
        .await?;
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
            .and_then(|_| write_character_meta(&project, &character))
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

    async fn require_approved_verdict(
        &self,
        project_id: &str,
        character_id: &str,
        token: &str,
        not_before: i64,
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
            .filter(|message| message.created_at >= not_before)
            .filter_map(|message| message.action.as_ref()?.payload.verdict.as_ref())
            .find(|verdict| verdict.token == token)
            .ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation(format!(
                    "缺少针对当前候选的 {token} 审校结论"
                ))
            })?;
        if verdict.decision != "APPROVE" {
            return Err(GameServiceError::InvalidCharacterOperation(format!(
                "最新 {token} 结论为 {}，不能通过人工门禁",
                verdict.decision
            )));
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

fn write_character_meta(project: &Project, character: &Character) -> Result<(), GameServiceError> {
    let path = Path::new(&project.root)
        .join(&character.dir_name)
        .join(".model.json");
    let content = serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": 2,
        "character": character,
    }))
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_art_bible(&path, &format!("{content}\n"))?;
    Ok(())
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

fn action_protocol_instruction() -> String {
    format!(
        "每次回复最后必须且只能包含一个 {start} JSON {end} 块。顶层只允许 action、target_agent、reason、payload；action 只允许 ask_user、handoff、done、blocked。handoff 目标必须来自 allowed_handoffs，其他动作 target_agent 必须为 null。",
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
