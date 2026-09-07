#![expect(
    clippy::disallowed_methods,
    reason = "game stores own their isolated SQLite configuration"
)]

use codex_game_domain::AgentAction;
use codex_game_domain::AgentHandoff;
use codex_game_domain::ArtBibleVersion;
use codex_game_domain::ArtBibleVersionId;
use codex_game_domain::ArtifactDraftRecord;
use codex_game_domain::ArtifactSlot;
use codex_game_domain::Character;
use codex_game_domain::CharacterState;
use codex_game_domain::ContextPackage;
use codex_game_domain::Conversation;
use codex_game_domain::ConversationCodexThread;
use codex_game_domain::ConversationCodexThreadId;
use codex_game_domain::ConversationId;
use codex_game_domain::ConversationMemory;
use codex_game_domain::ConversationMessage;
use codex_game_domain::ConversationStatus;
use codex_game_domain::ConversationTargetKind;
use codex_game_domain::Generation;
use codex_game_domain::GenerationReviewStatus;
use codex_game_domain::Interaction;
use codex_game_domain::InteractionId;
use codex_game_domain::MessageStatus;
use codex_game_domain::Project;
use codex_game_domain::ProjectId;
use codex_game_domain::ProjectMemory;
use codex_game_domain::Task;
use codex_game_domain::TaskAttempt;
use codex_game_domain::TaskAttemptStatus;
use codex_game_domain::TaskEvent;
use codex_game_domain::TaskId;
use codex_game_domain::TaskStatus;
use codex_game_domain::ThreadBindingStatus;
use sqlx::ConnectOptions;
use sqlx::Row;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteRow;
use sqlx::sqlite::SqliteSynchronous;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;

const PROJECT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    target_kind TEXT NOT NULL DEFAULT 'project',
    target_ref TEXT,
    title TEXT NOT NULL DEFAULT '',
    director_agent_code TEXT NOT NULL DEFAULT 'studio_director',
    focus_agent_code TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    turn INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    turn INTEGER NOT NULL DEFAULT 0,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    agent_code TEXT NOT NULL DEFAULT '',
    recipient_agent_code TEXT,
    status TEXT NOT NULL DEFAULT 'completed',
    token_count INTEGER NOT NULL DEFAULT 0,
    folded INTEGER NOT NULL DEFAULT 0,
    attachments_json TEXT NOT NULL DEFAULT '[]',
    action_json TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS interactions (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    interaction_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    stage TEXT NOT NULL,
    agent_code TEXT NOT NULL,
    input_version INTEGER NOT NULL DEFAULT 1,
    contract_version INTEGER NOT NULL DEFAULT 1,
    prompt TEXT NOT NULL DEFAULT '',
    context_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL,
    UNIQUE(interaction_id, stage, agent_code)
);
CREATE TABLE IF NOT EXISTS task_attempts (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    attempt_no INTEGER NOT NULL,
    conversation_codex_thread_id TEXT NOT NULL,
    codex_turn_id TEXT UNIQUE,
    output_artifact_id TEXT,
    status TEXT NOT NULL,
    UNIQUE(task_id, attempt_no)
);
CREATE TABLE IF NOT EXISTS art_bible_versions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    source_artifact_ids_json TEXT NOT NULL,
    markdown TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(project_id, version)
);
CREATE TABLE IF NOT EXISTS conversation_codex_threads (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    agent_code TEXT NOT NULL,
    codex_thread_id TEXT NOT NULL,
    codex_session_id TEXT NOT NULL,
    status TEXT NOT NULL,
    binding_version INTEGER NOT NULL,
    context_version INTEGER NOT NULL,
    agent_definition_version TEXT NOT NULL,
    forked_from_id TEXT,
    replacement_reason TEXT,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS active_conversation_agent_thread
ON conversation_codex_threads(conversation_id, agent_code)
WHERE status = 'active';
CREATE TABLE IF NOT EXISTS turn_attempt_bindings (
    codex_turn_id TEXT PRIMARY KEY,
    attempt_id TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS event_log (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_handoffs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    turn INTEGER NOT NULL,
    from_agent_code TEXT NOT NULL,
    to_agent_code TEXT NOT NULL,
    source TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_handoffs_by_conversation
ON agent_handoffs(conversation_id, turn, id);
CREATE TABLE IF NOT EXISTS artifact_drafts (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    artifact_slot TEXT NOT NULL,
    resolved_path TEXT NOT NULL,
    content TEXT NOT NULL,
    based_on_hash TEXT,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS artifact_drafts_by_conversation
ON artifact_drafts(conversation_id, status, created_at);
CREATE TABLE IF NOT EXISTS conversation_memory (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS project_memories (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    character_ref TEXT,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS characters (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    group_name TEXT,
    dir_name TEXT NOT NULL,
    state TEXT NOT NULL,
    spec_path TEXT,
    render_path TEXT,
    view_paths_json TEXT NOT NULL DEFAULT '{}',
    hard_constraints_json TEXT NOT NULL DEFAULT '[]',
    gate_spec_confirmed_at INTEGER,
    gate_render_confirmed_at INTEGER,
    gate_views_confirmed_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(project_id, dir_name)
);
CREATE TABLE IF NOT EXISTS character_groups (
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(project_id, name)
);
CREATE TABLE IF NOT EXISTS generations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    stage TEXT NOT NULL,
    variant TEXT,
    file_path TEXT NOT NULL,
    file_hash TEXT,
    is_final INTEGER NOT NULL DEFAULT 0,
    review_status TEXT NOT NULL DEFAULT 'pending',
    review_feedback TEXT,
    reviewed_at INTEGER,
    source TEXT NOT NULL,
    task_id TEXT,
    asset_spec_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS generations_by_target
ON generations(project_id, target_kind, target_ref, stage, created_at);
CREATE TABLE IF NOT EXISTS task_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    level TEXT NOT NULL,
    event TEXT NOT NULL,
    message TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}'
);
"#;

const STUDIO_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    root TEXT NOT NULL UNIQUE,
    access_mode TEXT NOT NULL,
    registered_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_accounts (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    enabled INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS route_bindings (
    scope_key TEXT PRIMARY KEY,
    provider_account_id TEXT NOT NULL,
    model TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS usage_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_account_id TEXT NOT NULL,
    metric TEXT NOT NULL,
    amount INTEGER NOT NULL,
    idempotency_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(provider_account_id, metric, idempotency_key)
);
CREATE TABLE IF NOT EXISTS route_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectAccess {
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAttemptCompletion {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAttemptContext {
    pub attempt_id: String,
    pub task_id: String,
    pub attempt_no: u32,
    pub conversation_id: String,
    pub turn: u64,
    pub target_id: String,
    pub stage: String,
    pub agent_code: String,
    pub input_version: u64,
    pub contract_version: u64,
    pub conversation_codex_thread_id: String,
    pub codex_thread_id: String,
    pub context: ContextPackage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAttempt {
    pub attempt_id: String,
    pub task_id: String,
    pub agent_code: String,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterruptedTaskRecovery {
    pub task_id: String,
    pub attempt_id: String,
    pub codex_turn_id: Option<String>,
    pub agent_code: String,
    pub stage: String,
    pub events: Vec<TaskEvent>,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("project database does not exist for read-only access: {0}")]
    MissingReadOnlyDatabase(PathBuf),
    #[error("cannot migrate project database because both paths exist: {legacy} and {local}")]
    StorageMigrationConflict { legacy: PathBuf, local: PathBuf },
    #[error("project store is read-only")]
    ReadOnly,
    #[error("store record was not found: {0}")]
    NotFound(String),
    #[error("store conflict: {0}")]
    Conflict(String),
    #[error("invalid store data: {0}")]
    InvalidData(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct ProjectStore {
    pool: SqlitePool,
    access: ProjectAccess,
    _writer_lock: Option<File>,
}

impl ProjectStore {
    pub async fn open(project_root: &Path) -> Result<Self, StoreError> {
        Self::open_with_access(project_root, false).await
    }

    pub async fn open_read_only(project_root: &Path) -> Result<Self, StoreError> {
        Self::open_with_access(project_root, true).await
    }

    async fn open_with_access(
        project_root: &Path,
        force_read_only: bool,
    ) -> Result<Self, StoreError> {
        let state_dir = project_root.join(".codex-game");
        let local_dir = state_dir.join("local");
        let legacy_database_path = state_dir.join("project.db");
        let local_database_path = local_dir.join("project.db");
        if force_read_only && !local_database_path.exists() && !legacy_database_path.exists() {
            return Err(StoreError::MissingReadOnlyDatabase(local_database_path));
        }
        if !force_read_only {
            fs::create_dir_all(&local_dir)?;
            ensure_local_storage_ignored(project_root)?;
            migrate_project_database(&legacy_database_path, &local_database_path)?;
        }
        let (access, writer_lock) = if force_read_only {
            (ProjectAccess::ReadOnly, None)
        } else {
            let lock_path = local_dir.join("project.lock");
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock_path)?;
            match lock.try_lock() {
                Ok(()) => (ProjectAccess::ReadWrite, Some(lock)),
                Err(std::fs::TryLockError::WouldBlock) => (ProjectAccess::ReadOnly, None),
                Err(std::fs::TryLockError::Error(err)) => return Err(StoreError::Io(err)),
            }
        };
        let database_path = if local_database_path.exists() || !force_read_only {
            local_database_path
        } else {
            legacy_database_path
        };
        if access == ProjectAccess::ReadOnly && !database_path.exists() {
            return Err(StoreError::MissingReadOnlyDatabase(database_path));
        }
        let pool = open_pool(&database_path, access).await?;
        if access == ProjectAccess::ReadWrite {
            run_schema(&pool, PROJECT_SCHEMA).await?;
            migrate_project_schema(&pool).await?;
        }
        Ok(Self {
            pool,
            access,
            _writer_lock: writer_lock,
        })
    }

    pub fn access(&self) -> ProjectAccess {
        self.access
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    fn require_writable(&self) -> Result<(), StoreError> {
        if self.access == ProjectAccess::ReadOnly {
            Err(StoreError::ReadOnly)
        } else {
            Ok(())
        }
    }

    pub async fn insert_conversation(&self, conversation: &Conversation) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query(
            "INSERT OR IGNORE INTO conversations(id, project_id, target_kind, target_ref, title, director_agent_code, focus_agent_code, status, turn, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(conversation.id.as_str())
        .bind(conversation.project_id.as_str())
        .bind(conversation.target_kind.as_str())
        .bind(&conversation.target_ref)
        .bind(&conversation.title)
        .bind(&conversation.director_agent_code)
        .bind(&conversation.focus_agent_code)
        .bind(conversation_status_name(conversation.status))
        .bind(conversation.turn as i64)
        .bind(conversation.created_at)
        .bind(conversation.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_conversation(&self, conversation: &Conversation) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query(
            "UPDATE conversations SET focus_agent_code = ?, status = ?, turn = ?, title = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&conversation.focus_agent_code)
        .bind(conversation_status_name(conversation.status))
        .bind(conversation.turn as i64)
        .bind(&conversation.title)
        .bind(conversation.updated_at)
        .bind(conversation.id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_message(&self, message: &ConversationMessage) -> Result<(), StoreError> {
        self.require_writable()?;
        insert_message_query(message).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn begin_conversation_turn(
        &self,
        conversation: &Conversation,
        user_message: &ConversationMessage,
        assistant_message: &ConversationMessage,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE conversations SET focus_agent_code = ?, status = ?, turn = ?, title = ?, updated_at = ? WHERE id = ? AND status = 'active'",
        )
        .bind(&conversation.focus_agent_code)
        .bind(conversation_status_name(conversation.status))
        .bind(conversation.turn as i64)
        .bind(&conversation.title)
        .bind(conversation.updated_at)
        .bind(conversation.id.as_str())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict("该会话已有一轮正在运行".to_string()));
        }
        insert_message_query(user_message)
            .execute(&mut *transaction)
            .await?;
        insert_message_query(assistant_message)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn begin_generation_revision_turn(
        &self,
        conversation: &Conversation,
        user_message: &ConversationMessage,
        assistant_message: &ConversationMessage,
        generation_id: &str,
        character_id: &str,
        feedback: &str,
        reviewed_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let reviewed = sqlx::query("UPDATE generations SET review_status = 'revisionRequested', review_feedback = ?, reviewed_at = ? WHERE id = ? AND project_id = ? AND target_kind = 'character' AND target_ref = ? AND review_status = 'pending'")
            .bind(feedback)
            .bind(reviewed_at)
            .bind(generation_id)
            .bind(conversation.project_id.as_str())
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;
        if reviewed.rows_affected() != 1 {
            return Err(StoreError::Conflict(format!(
                "generation {generation_id} is not pending"
            )));
        }
        let updated = sqlx::query(
            "UPDATE conversations SET focus_agent_code = ?, status = ?, turn = ?, title = ?, updated_at = ? WHERE id = ? AND status = 'active'",
        )
        .bind(&conversation.focus_agent_code)
        .bind(conversation_status_name(conversation.status))
        .bind(conversation.turn as i64)
        .bind(&conversation.title)
        .bind(conversation.updated_at)
        .bind(conversation.id.as_str())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict("该会话已有一轮正在运行".to_string()));
        }
        insert_message_query(user_message)
            .execute(&mut *transaction)
            .await?;
        insert_message_query(assistant_message)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn begin_handoff_continuation(
        &self,
        conversation: &Conversation,
        assistant_message: &ConversationMessage,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE conversations SET focus_agent_code = ?, status = 'running', updated_at = ? WHERE id = ? AND status = 'active'",
        )
        .bind(&conversation.focus_agent_code)
        .bind(conversation.updated_at)
        .bind(conversation.id.as_str())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict("该会话无法继续交接".to_string()));
        }
        insert_message_query(assistant_message)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn update_message(&self, message: &ConversationMessage) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query(
            "UPDATE messages SET content = ?, agent_code = ?, status = ?, token_count = ?, attachments_json = ?, action_json = ? WHERE id = ?",
        )
        .bind(&message.content)
        .bind(&message.agent_code)
        .bind(message_status_name(message.status))
        .bind(message.token_count as i64)
        .bind(serde_json::to_string(&message.attachments)?)
        .bind(message.action.as_ref().map(serde_json::to_string).transpose()?)
        .bind(&message.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn complete_prepared_action(
        &self,
        conversation_id: &str,
        assistant_message_id: &str,
        content: &str,
        action: &AgentAction,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE messages SET content = ?, status = 'completed', action_json = ? WHERE id = ? AND conversation_id = ? AND status = 'thinking'",
        )
        .bind(content)
        .bind(serde_json::to_string(action)?)
        .bind(assistant_message_id)
        .bind(conversation_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!(
                "thinking message {assistant_message_id}"
            )));
        }
        sqlx::query("UPDATE conversations SET status = 'active', updated_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(conversation_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn load_conversations(
        &self,
        project_id: &str,
    ) -> Result<Vec<(Conversation, Vec<ConversationMessage>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, target_kind, target_ref, title, director_agent_code, focus_agent_code, status, turn, created_at, updated_at FROM conversations WHERE project_id = ? ORDER BY created_at, id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let conversation_id: String = row.try_get("id")?;
            let message_rows = sqlx::query(
                "SELECT id, turn, role, content, agent_code, recipient_agent_code, status, token_count, folded, attachments_json, action_json, created_at FROM messages WHERE conversation_id = ? ORDER BY turn, created_at, id",
            )
            .bind(&conversation_id)
            .fetch_all(&self.pool)
            .await?;
            let messages = message_rows
                .into_iter()
                .map(|message| decode_message(&conversation_id, message))
                .collect::<Result<Vec<_>, StoreError>>()?;
            result.push((
                Conversation {
                    id: ConversationId::new(conversation_id),
                    project_id: ProjectId::new(project_id),
                    target_kind: parse_conversation_target_kind(
                        &row.try_get::<String, _>("target_kind")?,
                    )?,
                    target_ref: row.try_get("target_ref")?,
                    title: row.try_get("title")?,
                    director_agent_code: row.try_get("director_agent_code")?,
                    focus_agent_code: row.try_get("focus_agent_code")?,
                    status: parse_conversation_status(&row.try_get::<String, _>("status")?)?,
                    turn: row.try_get::<i64, _>("turn")? as u64,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                },
                messages,
            ));
        }
        Ok(result)
    }

    pub async fn commit_action_turn(
        &self,
        codex_turn_id: &str,
        assistant_message_id: &str,
        content: &str,
        action: &AgentAction,
        generations: &[Generation],
        character: Option<&Character>,
        created_at: i64,
    ) -> Result<TurnAttemptCompletion, StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT ta.id AS attempt_id, ta.task_id, i.conversation_id, t.agent_code FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id JOIN interactions i ON i.id = t.interaction_id WHERE ta.codex_turn_id = ? AND ta.status IN ('pending', 'running')",
        )
        .bind(codex_turn_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("active task attempt for turn {codex_turn_id}")))?;
        let completion = TurnAttemptCompletion {
            attempt_id: row.try_get("attempt_id")?,
            task_id: row.try_get("task_id")?,
            conversation_id: row.try_get("conversation_id")?,
        };
        let agent_code: String = row.try_get("agent_code")?;
        let action_json = serde_json::to_string(action)?;
        let updated = sqlx::query(
            "UPDATE messages SET content = ?, agent_code = ?, status = 'completed', action_json = ? WHERE id = ? AND conversation_id = ? AND status = 'thinking'",
        )
        .bind(content)
        .bind(&agent_code)
        .bind(action_json)
        .bind(assistant_message_id)
        .bind(&completion.conversation_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!(
                "thinking message {assistant_message_id}"
            )));
        }
        if action
            .payload
            .choices
            .as_ref()
            .is_some_and(|choices| !choices.is_empty())
        {
            sqlx::query(
                "UPDATE artifact_drafts SET status = 'superseded' WHERE conversation_id = ? AND status = 'pending'",
            )
            .bind(&completion.conversation_id)
            .execute(&mut *transaction)
            .await?;
        }
        for draft in action.payload.drafts.as_deref().unwrap_or_default() {
            let resolved_path = draft.artifact_slot.target_path();
            sqlx::query("UPDATE artifact_drafts SET status = 'superseded' WHERE conversation_id = ? AND artifact_slot = ? AND status = 'pending'")
                .bind(&completion.conversation_id)
                .bind(draft.artifact_slot.as_str())
                .execute(&mut *transaction)
                .await?;
            sqlx::query("INSERT INTO artifact_drafts(id, conversation_id, artifact_slot, resolved_path, content, based_on_hash, status, created_at) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)")
                .bind(uuid::Uuid::now_v7().to_string())
                .bind(&completion.conversation_id)
                .bind(draft.artifact_slot.as_str())
                .bind(resolved_path)
                .bind(&draft.content)
                .bind(&draft.based_on_hash)
                .bind(created_at)
                .execute(&mut *transaction)
                .await?;
        }
        let memory_scope = if action
            .payload
            .memories
            .as_ref()
            .is_some_and(|memories| !memories.is_empty())
        {
            let row = sqlx::query("SELECT project_id, target_ref FROM conversations WHERE id = ?")
                .bind(&completion.conversation_id)
                .fetch_one(&mut *transaction)
                .await?;
            Some((
                row.try_get::<String, _>("project_id")?,
                row.try_get::<Option<String>, _>("target_ref")?,
            ))
        } else {
            None
        };
        for memory in action.payload.memories.as_deref().unwrap_or_default() {
            let memory_id = uuid::Uuid::now_v7().to_string();
            sqlx::query("INSERT INTO conversation_memory(id, conversation_id, scope, kind, content, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(&memory_id)
                .bind(&completion.conversation_id)
                .bind(&memory.scope)
                .bind(&memory.kind)
                .bind(&memory.content)
                .bind(created_at)
                .execute(&mut *transaction)
                .await?;
            let (project_id, target_ref) = memory_scope.as_ref().ok_or_else(|| {
                StoreError::InvalidData("memory scope is unavailable".to_string())
            })?;
            let character_ref = (memory.scope == "character")
                .then(|| target_ref.clone())
                .flatten();
            sqlx::query("INSERT INTO project_memories(id, project_id, character_ref, kind, content, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 1, ?, ?)")
                .bind(uuid::Uuid::now_v7().to_string())
                .bind(project_id)
                .bind(character_ref)
                .bind(&memory.kind)
                .bind(&memory.content)
                .bind(created_at)
                .bind(created_at)
                .execute(&mut *transaction)
                .await?;
        }
        for generation in generations {
            if let Some(file_hash) = generation.file_hash.as_deref() {
                let duplicate_id: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM generations WHERE project_id = ? AND target_kind = ? AND target_ref = ? AND stage = ? AND file_hash = ? LIMIT 1",
                )
                .bind(&generation.project_id)
                .bind(&generation.target_kind)
                .bind(&generation.target_ref)
                .bind(&generation.stage)
                .bind(file_hash)
                .fetch_optional(&mut *transaction)
                .await?;
                if duplicate_id.is_some() {
                    continue;
                }
            }
            sqlx::query("UPDATE generations SET review_status = 'superseded', reviewed_at = ? WHERE project_id = ? AND target_kind = ? AND target_ref = ? AND stage = ? AND review_status IN ('pending', 'revisionRequested')")
                .bind(created_at)
                .bind(&generation.project_id)
                .bind(&generation.target_kind)
                .bind(&generation.target_ref)
                .bind(&generation.stage)
                .execute(&mut *transaction)
                .await?;
            if generation.stage == "render" {
                sqlx::query("UPDATE generations SET review_status = 'superseded', reviewed_at = ?, is_final = 0 WHERE project_id = ? AND target_kind = ? AND target_ref = ? AND stage = 'views' AND review_status IN ('pending', 'revisionRequested', 'accepted')")
                    .bind(created_at)
                    .bind(&generation.project_id)
                    .bind(&generation.target_kind)
                    .bind(&generation.target_ref)
                    .execute(&mut *transaction)
                    .await?;
            }
            sqlx::query("INSERT INTO generations(id, project_id, target_kind, target_ref, stage, variant, file_path, file_hash, is_final, review_status, review_feedback, reviewed_at, source, task_id, asset_spec_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&generation.id)
                .bind(&generation.project_id)
                .bind(&generation.target_kind)
                .bind(&generation.target_ref)
                .bind(&generation.stage)
                .bind(&generation.variant)
                .bind(&generation.file_path)
                .bind(&generation.file_hash)
                .bind(generation.is_final)
                .bind(generation_review_status_name(generation.review_status))
                .bind(&generation.review_feedback)
                .bind(generation.reviewed_at)
                .bind(&generation.source)
                .bind(&generation.task_id)
                .bind(serde_json::to_string(&generation.asset_spec)?)
                .bind(generation.created_at)
                .execute(&mut *transaction)
                .await?;
        }
        if let Some(character) = character {
            let updated = sqlx::query("UPDATE characters SET state = ?, spec_path = ?, render_path = ?, view_paths_json = ?, hard_constraints_json = ?, gate_spec_confirmed_at = ?, gate_render_confirmed_at = ?, gate_views_confirmed_at = ?, updated_at = ? WHERE id = ? AND project_id = ?")
                .bind(character_state_name(character.state))
                .bind(&character.spec_path)
                .bind(&character.render_path)
                .bind(serde_json::to_string(&character.view_paths)?)
                .bind(serde_json::to_string(&character.hard_constraints)?)
                .bind(character.gate_spec_confirmed_at)
                .bind(character.gate_render_confirmed_at)
                .bind(character.gate_views_confirmed_at)
                .bind(character.updated_at)
                .bind(&character.id)
                .bind(&character.project_id)
                .execute(&mut *transaction)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::NotFound(format!("character {}", character.id)));
            }
        }
        if let Some(target) = &action.target_agent {
            let turn: i64 = sqlx::query_scalar("SELECT turn FROM conversations WHERE id = ?")
                .bind(&completion.conversation_id)
                .fetch_one(&mut *transaction)
                .await?;
            sqlx::query("INSERT INTO agent_handoffs(conversation_id, turn, from_agent_code, to_agent_code, source, reason, status, created_at) VALUES (?, ?, ?, ?, 'agent', ?, 'delegated', ?)")
                .bind(&completion.conversation_id)
                .bind(turn)
                .bind(&agent_code)
                .bind(target)
                .bind(&action.reason)
                .bind(created_at)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE conversations SET focus_agent_code = ?, status = 'active', updated_at = ? WHERE id = ?")
                .bind(target)
                .bind(created_at)
                .bind(&completion.conversation_id)
                .execute(&mut *transaction)
                .await?;
        } else {
            sqlx::query("UPDATE conversations SET status = 'active', updated_at = ? WHERE id = ?")
                .bind(created_at)
                .bind(&completion.conversation_id)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query("UPDATE task_attempts SET status = 'succeeded' WHERE id = ?")
            .bind(&completion.attempt_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE tasks SET status = 'succeeded' WHERE id = ?")
            .bind(&completion.task_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(completion)
    }

    pub async fn fail_action_turn(
        &self,
        codex_turn_id: &str,
        assistant_message_id: &str,
        error: &str,
        status: MessageStatus,
        created_at: i64,
    ) -> Result<Option<TurnAttemptCompletion>, StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT ta.id AS attempt_id, ta.task_id, i.conversation_id FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id JOIN interactions i ON i.id = t.interaction_id WHERE ta.codex_turn_id = ? AND ta.status IN ('pending', 'running')",
        )
        .bind(codex_turn_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let completion = TurnAttemptCompletion {
            attempt_id: row.try_get("attempt_id")?,
            task_id: row.try_get("task_id")?,
            conversation_id: row.try_get("conversation_id")?,
        };
        sqlx::query("UPDATE messages SET content = ?, status = ? WHERE id = ? AND conversation_id = ? AND status = 'thinking'")
            .bind(error)
            .bind(message_status_name(status))
            .bind(assistant_message_id)
            .bind(&completion.conversation_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE conversations SET status = 'active', updated_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(&completion.conversation_id)
            .execute(&mut *transaction)
            .await?;
        let attempt_status = if status == MessageStatus::Interrupted {
            TaskAttemptStatus::Interrupted
        } else {
            TaskAttemptStatus::Failed
        };
        sqlx::query("UPDATE task_attempts SET status = ? WHERE id = ?")
            .bind(attempt_status_name(attempt_status))
            .bind(&completion.attempt_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE tasks SET status = 'failed' WHERE id = ?")
            .bind(&completion.task_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(completion))
    }

    pub async fn list_drafts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ArtifactDraftRecord>, StoreError> {
        let rows = sqlx::query("SELECT id, artifact_slot, resolved_path, content, based_on_hash, status, created_at FROM artifact_drafts WHERE conversation_id = ? ORDER BY created_at, id")
            .bind(conversation_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let artifact_slot_name: String = row.try_get("artifact_slot")?;
                let artifact_slot = ArtifactSlot::parse(&artifact_slot_name).ok_or_else(|| {
                    StoreError::InvalidData(format!("unknown artifact slot: {artifact_slot_name}"))
                })?;
                Ok(ArtifactDraftRecord {
                    id: row.try_get("id")?,
                    conversation_id: conversation_id.to_string(),
                    artifact_slot,
                    target_path: row.try_get("resolved_path")?,
                    content: row.try_get("content")?,
                    based_on_hash: row.try_get("based_on_hash")?,
                    status: row.try_get("status")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn list_handoffs(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<AgentHandoff>, StoreError> {
        let rows = sqlx::query("SELECT id, turn, from_agent_code, to_agent_code, source, reason, status, created_at FROM agent_handoffs WHERE conversation_id = ? ORDER BY turn, id")
            .bind(conversation_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AgentHandoff {
                    id: row.try_get("id")?,
                    conversation_id: conversation_id.to_string(),
                    turn: row.try_get::<i64, _>("turn")? as u64,
                    from_agent_code: row.try_get("from_agent_code")?,
                    to_agent_code: row.try_get("to_agent_code")?,
                    source: row.try_get("source")?,
                    reason: row.try_get("reason")?,
                    status: row.try_get("status")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn list_conversation_memories(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationMemory>, StoreError> {
        let rows = sqlx::query("SELECT id, scope, kind, content, created_at FROM conversation_memory WHERE conversation_id = ? ORDER BY created_at, id")
            .bind(conversation_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ConversationMemory {
                    id: row.try_get("id")?,
                    conversation_id: conversation_id.to_string(),
                    scope: row.try_get("scope")?,
                    kind: row.try_get("kind")?,
                    content: row.try_get("content")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn list_project_memories(
        &self,
        project_id: &str,
        character_ref: Option<&str>,
    ) -> Result<Vec<ProjectMemory>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, character_ref, kind, content, enabled, created_at, updated_at FROM project_memories WHERE project_id = ? AND enabled = 1 AND (character_ref IS NULL OR character_ref = ?) ORDER BY updated_at DESC, id DESC LIMIT 20",
        )
        .bind(project_id)
        .bind(character_ref)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProjectMemory {
                    id: row.try_get("id")?,
                    project_id: project_id.to_string(),
                    character_ref: row.try_get("character_ref")?,
                    kind: row.try_get("kind")?,
                    content: row.try_get("content")?,
                    enabled: row.try_get("enabled")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    pub async fn mark_drafts_committed(&self, draft_ids: &[String]) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        for draft_id in draft_ids {
            let updated = sqlx::query("UPDATE artifact_drafts SET status = 'committed' WHERE id = ? AND status = 'pending'")
                .bind(draft_id)
                .execute(&mut *transaction)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Conflict(format!(
                    "draft is not pending: {draft_id}"
                )));
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn commit_character_spec_gate(
        &self,
        draft_id: &str,
        character: &Character,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE artifact_drafts SET status = 'committed' WHERE id = ? AND status = 'pending'",
        )
        .bind(draft_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(format!(
                "draft is not pending: {draft_id}"
            )));
        }
        let updated = sqlx::query("UPDATE characters SET state = ?, spec_path = ?, render_path = ?, view_paths_json = ?, hard_constraints_json = ?, gate_spec_confirmed_at = ?, gate_render_confirmed_at = ?, gate_views_confirmed_at = ?, updated_at = ? WHERE id = ? AND project_id = ?")
            .bind(character_state_name(character.state))
            .bind(&character.spec_path)
            .bind(&character.render_path)
            .bind(serde_json::to_string(&character.view_paths)?)
            .bind(serde_json::to_string(&character.hard_constraints)?)
            .bind(character.gate_spec_confirmed_at)
            .bind(character.gate_render_confirmed_at)
            .bind(character.gate_views_confirmed_at)
            .bind(character.updated_at)
            .bind(&character.id)
            .bind(&character.project_id)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!("character {}", character.id)));
        }
        insert_character_event(
            &mut transaction,
            character,
            "spec_confirmed",
            "角色设定已通过人工门禁",
            serde_json::json!({ "draftId": draft_id }),
            created_at,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn insert_character_group(
        &self,
        project_id: &str,
        name: &str,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query(
            "INSERT OR IGNORE INTO character_groups(project_id, name, created_at) VALUES (?, ?, ?)",
        )
        .bind(project_id)
        .bind(name)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_character_groups(&self, project_id: &str) -> Result<Vec<String>, StoreError> {
        sqlx::query_scalar(
            "SELECT name FROM character_groups WHERE project_id = ? UNION SELECT group_name FROM characters WHERE project_id = ? AND group_name IS NOT NULL ORDER BY 1",
        )
        .bind(project_id)
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn insert_character(&self, character: &Character) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query("INSERT INTO characters(id, project_id, name, group_name, dir_name, state, spec_path, render_path, view_paths_json, hard_constraints_json, gate_spec_confirmed_at, gate_render_confirmed_at, gate_views_confirmed_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&character.id)
            .bind(&character.project_id)
            .bind(&character.name)
            .bind(&character.group)
            .bind(&character.dir_name)
            .bind(character_state_name(character.state))
            .bind(&character.spec_path)
            .bind(&character.render_path)
            .bind(serde_json::to_string(&character.view_paths)?)
            .bind(serde_json::to_string(&character.hard_constraints)?)
            .bind(character.gate_spec_confirmed_at)
            .bind(character.gate_render_confirmed_at)
            .bind(character.gate_views_confirmed_at)
            .bind(character.created_at)
            .bind(character.updated_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_character(&self, character: &Character) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query("INSERT INTO characters(id, project_id, name, group_name, dir_name, state, spec_path, render_path, view_paths_json, hard_constraints_json, gate_spec_confirmed_at, gate_render_confirmed_at, gate_views_confirmed_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET project_id = excluded.project_id, name = excluded.name, group_name = excluded.group_name, dir_name = excluded.dir_name, state = excluded.state, spec_path = excluded.spec_path, render_path = excluded.render_path, view_paths_json = excluded.view_paths_json, hard_constraints_json = excluded.hard_constraints_json, gate_spec_confirmed_at = excluded.gate_spec_confirmed_at, gate_render_confirmed_at = excluded.gate_render_confirmed_at, gate_views_confirmed_at = excluded.gate_views_confirmed_at, created_at = excluded.created_at, updated_at = excluded.updated_at")
            .bind(&character.id)
            .bind(&character.project_id)
            .bind(&character.name)
            .bind(&character.group)
            .bind(&character.dir_name)
            .bind(character_state_name(character.state))
            .bind(&character.spec_path)
            .bind(&character.render_path)
            .bind(serde_json::to_string(&character.view_paths)?)
            .bind(serde_json::to_string(&character.hard_constraints)?)
            .bind(character.gate_spec_confirmed_at)
            .bind(character.gate_render_confirmed_at)
            .bind(character.gate_views_confirmed_at)
            .bind(character.created_at)
            .bind(character.updated_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_character(&self, character: &Character) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query("UPDATE characters SET state = ?, spec_path = ?, render_path = ?, view_paths_json = ?, hard_constraints_json = ?, gate_spec_confirmed_at = ?, gate_render_confirmed_at = ?, gate_views_confirmed_at = ?, updated_at = ? WHERE id = ? AND project_id = ?")
            .bind(character_state_name(character.state))
            .bind(&character.spec_path)
            .bind(&character.render_path)
            .bind(serde_json::to_string(&character.view_paths)?)
            .bind(serde_json::to_string(&character.hard_constraints)?)
            .bind(character.gate_spec_confirmed_at)
            .bind(character.gate_render_confirmed_at)
            .bind(character.gate_views_confirmed_at)
            .bind(character.updated_at)
            .bind(&character.id)
            .bind(&character.project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_characters(&self, project_id: &str) -> Result<Vec<Character>, StoreError> {
        let rows = sqlx::query("SELECT id, name, group_name, dir_name, state, spec_path, render_path, view_paths_json, hard_constraints_json, gate_spec_confirmed_at, gate_render_confirmed_at, gate_views_confirmed_at, created_at, updated_at FROM characters WHERE project_id = ? ORDER BY name, id")
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| decode_character(project_id, row))
            .collect()
    }

    pub async fn read_character(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<Option<Character>, StoreError> {
        let row = sqlx::query("SELECT id, name, group_name, dir_name, state, spec_path, render_path, view_paths_json, hard_constraints_json, gate_spec_confirmed_at, gate_render_confirmed_at, gate_views_confirmed_at, created_at, updated_at FROM characters WHERE project_id = ? AND id = ?")
            .bind(project_id)
            .bind(character_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| decode_character(project_id, row)).transpose()
    }

    pub async fn delete_character_record(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM generations WHERE project_id = ? AND target_kind = 'character' AND target_ref = ?")
            .bind(project_id)
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;
        let deleted = sqlx::query("DELETE FROM characters WHERE project_id = ? AND id = ?")
            .bind(project_id)
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!("character {character_id}")));
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn insert_generation(&self, generation: &Generation) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query("INSERT INTO generations(id, project_id, target_kind, target_ref, stage, variant, file_path, file_hash, is_final, review_status, review_feedback, reviewed_at, source, task_id, asset_spec_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&generation.id)
            .bind(&generation.project_id)
            .bind(&generation.target_kind)
            .bind(&generation.target_ref)
            .bind(&generation.stage)
            .bind(&generation.variant)
            .bind(&generation.file_path)
            .bind(&generation.file_hash)
            .bind(generation.is_final)
            .bind(generation_review_status_name(generation.review_status))
            .bind(&generation.review_feedback)
            .bind(generation.reviewed_at)
            .bind(&generation.source)
            .bind(&generation.task_id)
            .bind(serde_json::to_string(&generation.asset_spec)?)
            .bind(generation.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_generations(
        &self,
        project_id: &str,
        target_kind: &str,
        target_ref: &str,
        stage: Option<&str>,
    ) -> Result<Vec<Generation>, StoreError> {
        let rows = sqlx::query("SELECT id, stage, variant, file_path, file_hash, is_final, review_status, review_feedback, reviewed_at, source, task_id, asset_spec_json, created_at FROM generations WHERE project_id = ? AND target_kind = ? AND target_ref = ? AND (? IS NULL OR stage = ?) ORDER BY created_at DESC, id DESC")
            .bind(project_id)
            .bind(target_kind)
            .bind(target_ref)
            .bind(stage)
            .bind(stage)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let asset_spec: String = row.try_get("asset_spec_json")?;
                Ok(Generation {
                    id: row.try_get("id")?,
                    project_id: project_id.to_string(),
                    target_kind: target_kind.to_string(),
                    target_ref: target_ref.to_string(),
                    stage: row.try_get("stage")?,
                    variant: row.try_get("variant")?,
                    file_path: row.try_get("file_path")?,
                    file_hash: row.try_get("file_hash")?,
                    is_final: row.try_get("is_final")?,
                    review_status: parse_generation_review_status(
                        row.try_get::<String, _>("review_status")?.as_str(),
                    )?,
                    review_feedback: row.try_get("review_feedback")?,
                    reviewed_at: row.try_get("reviewed_at")?,
                    source: row.try_get("source")?,
                    task_id: row.try_get("task_id")?,
                    asset_spec: serde_json::from_str(&asset_spec)?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn read_generation(
        &self,
        generation_id: &str,
    ) -> Result<Option<Generation>, StoreError> {
        let row = sqlx::query("SELECT project_id, target_kind, target_ref, stage, variant, file_path, file_hash, is_final, review_status, review_feedback, reviewed_at, source, task_id, asset_spec_json, created_at FROM generations WHERE id = ?")
            .bind(generation_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| {
            let asset_spec: String = row.try_get("asset_spec_json")?;
            Ok(Generation {
                id: generation_id.to_string(),
                project_id: row.try_get("project_id")?,
                target_kind: row.try_get("target_kind")?,
                target_ref: row.try_get("target_ref")?,
                stage: row.try_get("stage")?,
                variant: row.try_get("variant")?,
                file_path: row.try_get("file_path")?,
                file_hash: row.try_get("file_hash")?,
                is_final: row.try_get("is_final")?,
                review_status: parse_generation_review_status(
                    row.try_get::<String, _>("review_status")?.as_str(),
                )?,
                review_feedback: row.try_get("review_feedback")?,
                reviewed_at: row.try_get("reviewed_at")?,
                source: row.try_get("source")?,
                task_id: row.try_get("task_id")?,
                asset_spec: serde_json::from_str(&asset_spec)?,
                created_at: row.try_get("created_at")?,
            })
        })
        .transpose()
    }

    pub async fn review_generation(
        &self,
        project_id: &str,
        target_ref: &str,
        generation_id: &str,
        status: GenerationReviewStatus,
        feedback: Option<&str>,
        reviewed_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let updated = sqlx::query("UPDATE generations SET review_status = ?, review_feedback = ?, reviewed_at = ? WHERE id = ? AND project_id = ? AND target_kind = 'character' AND target_ref = ? AND review_status = 'pending'")
            .bind(generation_review_status_name(status))
            .bind(feedback)
            .bind(reviewed_at)
            .bind(generation_id)
            .bind(project_id)
            .bind(target_ref)
            .execute(&self.pool)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(format!(
                "generation {generation_id} is not pending"
            )));
        }
        Ok(())
    }

    pub async fn supersede_generation_candidates(
        &self,
        project_id: &str,
        target_ref: &str,
        stage: &str,
        except_id: &str,
        reviewed_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query("UPDATE generations SET review_status = 'superseded', reviewed_at = ? WHERE project_id = ? AND target_kind = 'character' AND target_ref = ? AND stage = ? AND id != ? AND review_status IN ('pending', 'revisionRequested')")
            .bind(reviewed_at)
            .bind(project_id)
            .bind(target_ref)
            .bind(stage)
            .bind(except_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_generation_final(
        &self,
        project_id: &str,
        target_ref: &str,
        generation_ids: &[String],
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE generations SET is_final = 0 WHERE project_id = ? AND target_kind = 'character' AND target_ref = ?")
            .bind(project_id)
            .bind(target_ref)
            .execute(&mut *transaction)
            .await?;
        for generation_id in generation_ids {
            let updated = sqlx::query("UPDATE generations SET is_final = 1 WHERE id = ? AND project_id = ? AND target_kind = 'character' AND target_ref = ?")
                .bind(generation_id)
                .bind(project_id)
                .bind(target_ref)
                .execute(&mut *transaction)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::NotFound(format!("generation {generation_id}")));
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn commit_character_generation_gate(
        &self,
        stage: &str,
        generation_ids: &[String],
        character: &Character,
        art_bible_publication: Option<(&ArtBibleVersion, &str)>,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE generations SET is_final = 0 WHERE project_id = ? AND target_kind = 'character' AND target_ref = ?")
            .bind(&character.project_id)
            .bind(&character.id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE generations SET review_status = 'superseded', reviewed_at = ? WHERE project_id = ? AND target_kind = 'character' AND target_ref = ? AND stage = ? AND review_status IN ('pending', 'revisionRequested')")
            .bind(created_at)
            .bind(&character.project_id)
            .bind(&character.id)
            .bind(stage)
            .execute(&mut *transaction)
            .await?;
        for generation_id in generation_ids {
            let updated = sqlx::query("UPDATE generations SET review_status = 'accepted', reviewed_at = ?, is_final = ? WHERE id = ? AND project_id = ? AND target_kind = 'character' AND target_ref = ? AND stage = ?")
                .bind(created_at)
                .bind(stage == "views")
                .bind(generation_id)
                .bind(&character.project_id)
                .bind(&character.id)
                .bind(stage)
                .execute(&mut *transaction)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::NotFound(format!("generation {generation_id}")));
            }
        }
        if stage == "views" {
            sqlx::query("UPDATE generations SET is_final = 1 WHERE project_id = ? AND target_kind = 'character' AND target_ref = ? AND stage = 'render' AND review_status = 'accepted'")
                .bind(&character.project_id)
                .bind(&character.id)
                .execute(&mut *transaction)
                .await?;
        }
        if let Some((version, markdown)) = art_bible_publication {
            sqlx::query("INSERT INTO art_bible_versions(id, project_id, version, content_hash, source_artifact_ids_json, markdown, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .bind(version.id.as_str())
                .bind(version.project_id.as_str())
                .bind(version.version as i64)
                .bind(&version.content_hash)
                .bind(serde_json::to_string(&version.source_artifact_ids)?)
                .bind(markdown)
                .bind(version.created_at)
                .execute(&mut *transaction)
                .await?;
        }
        let updated = sqlx::query("UPDATE characters SET state = ?, spec_path = ?, render_path = ?, view_paths_json = ?, hard_constraints_json = ?, gate_spec_confirmed_at = ?, gate_render_confirmed_at = ?, gate_views_confirmed_at = ?, updated_at = ? WHERE id = ? AND project_id = ?")
            .bind(character_state_name(character.state))
            .bind(&character.spec_path)
            .bind(&character.render_path)
            .bind(serde_json::to_string(&character.view_paths)?)
            .bind(serde_json::to_string(&character.hard_constraints)?)
            .bind(character.gate_spec_confirmed_at)
            .bind(character.gate_render_confirmed_at)
            .bind(character.gate_views_confirmed_at)
            .bind(character.updated_at)
            .bind(&character.id)
            .bind(&character.project_id)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!("character {}", character.id)));
        }
        insert_character_event(
            &mut transaction,
            character,
            &format!("{stage}_confirmed"),
            &format!("角色 {stage} 产物已通过人工门禁"),
            serde_json::json!({ "generationIds": generation_ids }),
            created_at,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn record_character_rejection(
        &self,
        character: &Character,
        stage: &str,
        reason: &str,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO project_memories(id, project_id, character_ref, kind, content, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 1, ?, ?)")
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&character.project_id)
            .bind(&character.id)
            .bind(format!("{stage}_rejection"))
            .bind(reason)
            .bind(created_at)
            .bind(created_at)
            .execute(&mut *transaction)
            .await?;
        insert_character_event(
            &mut transaction,
            character,
            &format!("{stage}_rejected"),
            reason,
            serde_json::json!({ "stage": stage, "reason": reason }),
            created_at,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn insert_art_bible_version(
        &self,
        version: &ArtBibleVersion,
        markdown: &str,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        sqlx::query(
            "INSERT INTO art_bible_versions(id, project_id, version, content_hash, source_artifact_ids_json, markdown, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(version.id.as_str())
        .bind(version.project_id.as_str())
        .bind(version.version as i64)
        .bind(&version.content_hash)
        .bind(serde_json::to_string(&version.source_artifact_ids).unwrap_or_else(|_| "[]".to_string()))
        .bind(markdown)
        .bind(version.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn commit_art_bible_gate(
        &self,
        version: &ArtBibleVersion,
        markdown: &str,
        draft_id: &str,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO art_bible_versions(id, project_id, version, content_hash, source_artifact_ids_json, markdown, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(version.id.as_str())
        .bind(version.project_id.as_str())
        .bind(version.version as i64)
        .bind(&version.content_hash)
        .bind(serde_json::to_string(&version.source_artifact_ids)?)
        .bind(markdown)
        .bind(version.created_at)
        .execute(&mut *transaction)
        .await?;
        let updated = sqlx::query(
            "UPDATE artifact_drafts SET status = 'committed' WHERE id = ? AND status = 'pending'",
        )
        .bind(draft_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Conflict(format!(
                "draft is not pending: {draft_id}"
            )));
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn load_art_bible_versions(
        &self,
        project_id: &str,
    ) -> Result<Vec<(ArtBibleVersion, String)>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, version, content_hash, source_artifact_ids_json, markdown, created_at FROM art_bible_versions WHERE project_id = ? ORDER BY version",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let sources: String = row.try_get("source_artifact_ids_json")?;
                Ok((
                    ArtBibleVersion {
                        id: ArtBibleVersionId::new(row.try_get::<String, _>("id")?),
                        project_id: ProjectId::new(project_id),
                        version: row.try_get::<i64, _>("version")? as u64,
                        content_hash: row.try_get("content_hash")?,
                        source_artifact_ids: serde_json::from_str(&sources).unwrap_or_default(),
                        created_at: row.try_get("created_at")?,
                    },
                    row.try_get("markdown")?,
                ))
            })
            .collect()
    }

    pub async fn active_thread(
        &self,
        conversation_id: &str,
        agent_code: &str,
    ) -> Result<Option<ConversationCodexThread>, StoreError> {
        let row = sqlx::query(
            "SELECT id, codex_thread_id, codex_session_id, binding_version, context_version, agent_definition_version, forked_from_id, replacement_reason, created_at, last_used_at FROM conversation_codex_threads WHERE conversation_id = ? AND agent_code = ? AND status = 'active'",
        )
        .bind(conversation_id)
        .bind(agent_code)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ConversationCodexThread {
                id: ConversationCodexThreadId::new(row.try_get::<String, _>("id")?),
                conversation_id: ConversationId::new(conversation_id),
                agent_code: agent_code.to_string(),
                codex_thread_id: row.try_get("codex_thread_id")?,
                codex_session_id: row.try_get("codex_session_id")?,
                status: ThreadBindingStatus::Active,
                binding_version: row.try_get::<i64, _>("binding_version")? as u64,
                context_version: row.try_get::<i64, _>("context_version")? as u64,
                agent_definition_version: row.try_get("agent_definition_version")?,
                forked_from_id: row
                    .try_get::<Option<String>, _>("forked_from_id")?
                    .map(ConversationCodexThreadId::new),
                replacement_reason: row.try_get("replacement_reason")?,
                created_at: row.try_get("created_at")?,
                last_used_at: row.try_get("last_used_at")?,
            })
        })
        .transpose()
    }

    pub async fn replace_active_thread(
        &self,
        binding: &ConversationCodexThread,
        expected_binding_version: Option<u64>,
        replacement_reason: Option<&str>,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        if let Some(expected) = expected_binding_version {
            let current: Option<i64> = sqlx::query_scalar(
                "SELECT binding_version FROM conversation_codex_threads WHERE conversation_id = ? AND agent_code = ? AND status = 'active'",
            )
            .bind(binding.conversation_id.as_str())
            .bind(&binding.agent_code)
            .fetch_optional(&mut *transaction)
            .await?;
            if current != Some(expected as i64) {
                return Err(StoreError::NotFound(
                    "active thread binding changed".to_string(),
                ));
            }
        }
        sqlx::query(
            "UPDATE conversation_codex_threads SET status = 'replaced', replacement_reason = ? WHERE conversation_id = ? AND agent_code = ? AND status = 'active'",
        )
        .bind(replacement_reason)
        .bind(binding.conversation_id.as_str())
        .bind(&binding.agent_code)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO conversation_codex_threads(id, conversation_id, agent_code, codex_thread_id, codex_session_id, status, binding_version, context_version, agent_definition_version, forked_from_id, replacement_reason, created_at, last_used_at) VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(binding.id.as_str())
        .bind(binding.conversation_id.as_str())
        .bind(&binding.agent_code)
        .bind(&binding.codex_thread_id)
        .bind(&binding.codex_session_id)
        .bind(binding.binding_version as i64)
        .bind(binding.context_version as i64)
        .bind(&binding.agent_definition_version)
        .bind(
            binding
                .forked_from_id
                .as_ref()
                .map(ConversationCodexThreadId::as_str),
        )
        .bind(&binding.replacement_reason)
        .bind(binding.created_at)
        .bind(binding.last_used_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn create_task_attempt(
        &self,
        interaction: &Interaction,
        task: &Task,
        attempt: &TaskAttempt,
        prompt: &str,
        context: &ContextPackage,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO interactions(id, conversation_id, idempotency_key, created_at) VALUES (?, ?, ?, ?)")
            .bind(interaction.id.as_str())
            .bind(interaction.conversation_id.as_str())
            .bind(&interaction.idempotency_key)
            .bind(interaction.created_at)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO tasks(id, interaction_id, target_id, stage, agent_code, input_version, contract_version, prompt, context_json, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending')")
            .bind(task.id.as_str())
            .bind(task.interaction_id.as_str())
            .bind(&task.target_id)
            .bind(&task.stage)
            .bind(&task.agent_code)
            .bind(task.input_version as i64)
            .bind(task.contract_version as i64)
            .bind(prompt)
            .bind(serde_json::to_string(context)?)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, status) VALUES (?, ?, ?, ?, 'pending')")
            .bind(attempt.id.as_str())
            .bind(attempt.task_id.as_str())
            .bind(attempt.attempt_no as i64)
            .bind(attempt.conversation_codex_thread_id.as_str())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn begin_task_attempt_retry(
        &self,
        current_attempt_id: &str,
        retry_attempt: &TaskAttempt,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT task_id, attempt_no, conversation_codex_thread_id FROM task_attempts WHERE id = ? AND status = 'running'",
        )
        .bind(current_attempt_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| StoreError::Conflict("自动重试的原始尝试已结束".to_string()))?;
        let task_id: String = row.try_get("task_id")?;
        let attempt_no = row.try_get::<i64, _>("attempt_no")? as u32;
        let binding_id: String = row.try_get("conversation_codex_thread_id")?;
        if retry_attempt.task_id.as_str() != task_id
            || retry_attempt.attempt_no != attempt_no + 1
            || retry_attempt.conversation_codex_thread_id.as_str() != binding_id
        {
            return Err(StoreError::Conflict(
                "自动重试上下文与原始尝试不一致".to_string(),
            ));
        }
        sqlx::query("UPDATE task_attempts SET status = 'failed' WHERE id = ?")
            .bind(current_attempt_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, status) VALUES (?, ?, ?, ?, 'pending')",
        )
        .bind(retry_attempt.id.as_str())
        .bind(retry_attempt.task_id.as_str())
        .bind(retry_attempt.attempt_no as i64)
        .bind(retry_attempt.conversation_codex_thread_id.as_str())
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE tasks SET status = 'running' WHERE id = ?")
            .bind(&task_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn rollback_task_attempt_retry(
        &self,
        current_attempt_id: &str,
        retry_attempt_id: &str,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let deleted = sqlx::query(
            "DELETE FROM task_attempts WHERE id = ? AND status = 'pending' AND codex_turn_id IS NULL",
        )
        .bind(retry_attempt_id)
        .execute(&mut *transaction)
        .await?;
        if deleted.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "自动重试已经开始，无法回滚".to_string(),
            ));
        }
        let restored = sqlx::query(
            "UPDATE task_attempts SET status = 'running' WHERE id = ? AND status = 'failed'",
        )
        .bind(current_attempt_id)
        .execute(&mut *transaction)
        .await?;
        if restored.rows_affected() != 1 {
            return Err(StoreError::Conflict(
                "自动重试的原始尝试无法恢复".to_string(),
            ));
        }
        sqlx::query(
            "UPDATE tasks SET status = 'running' WHERE id = (SELECT task_id FROM task_attempts WHERE id = ?)",
        )
        .bind(current_attempt_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_tasks(&self, conversation_id: &str) -> Result<Vec<Task>, StoreError> {
        let rows = sqlx::query(
            "SELECT t.id, t.interaction_id, t.target_id, t.stage, t.agent_code, t.input_version, t.contract_version, t.status FROM tasks t JOIN interactions i ON i.id = t.interaction_id WHERE i.conversation_id = ? ORDER BY i.created_at, t.id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let status: String = row.try_get("status")?;
                Ok(Task {
                    id: TaskId::new(row.try_get::<String, _>("id")?),
                    interaction_id: InteractionId::new(row.try_get::<String, _>("interaction_id")?),
                    target_id: row.try_get("target_id")?,
                    stage: row.try_get("stage")?,
                    agent_code: row.try_get("agent_code")?,
                    input_artifact_ids: Vec::new(),
                    input_version: row.try_get::<i64, _>("input_version")? as u64,
                    contract_version: row.try_get::<i64, _>("contract_version")? as u64,
                    status: parse_task_status(&status)?,
                })
            })
            .collect()
    }

    pub async fn append_task_event(
        &self,
        task_id: &str,
        level: &str,
        event: &str,
        message: &str,
        payload: &serde_json::Value,
        timestamp: i64,
    ) -> Result<TaskEvent, StoreError> {
        self.require_writable()?;
        let level = bounded_event_text(level, 64);
        let event = bounded_event_text(event, 128);
        let message = bounded_event_text(message, 8 * 1024);
        let mut payload = sanitize_task_event_payload(payload, None);
        let mut payload_json = serde_json::to_string(&payload)?;
        if payload_json.len() > 64 * 1024 {
            payload = serde_json::json!({
                "truncated": true,
                "originalBytes": payload_json.len(),
            });
            payload_json = serde_json::to_string(&payload)?;
        }
        let mut transaction = self.pool.begin().await?;
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM task_events WHERE task_id = ?",
        )
        .bind(task_id)
        .fetch_one(&mut *transaction)
        .await?;
        let result = sqlx::query(
            "INSERT INTO task_events(task_id, sequence, timestamp, level, event, message, payload_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(sequence)
        .bind(timestamp)
        .bind(&level)
        .bind(&event)
        .bind(&message)
        .bind(&payload_json)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(TaskEvent {
            id: result.last_insert_rowid(),
            task_id: task_id.to_string(),
            sequence: sequence as u64,
            timestamp,
            level,
            event,
            message,
            payload,
        })
    }

    pub async fn list_task_events(&self, task_id: &str) -> Result<Vec<TaskEvent>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, task_id, sequence, timestamp, level, event, message, payload_json FROM task_events WHERE task_id = ? ORDER BY sequence",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let payload_json: String = row.try_get("payload_json")?;
                Ok(TaskEvent {
                    id: row.try_get("id")?,
                    task_id: row.try_get("task_id")?,
                    sequence: row.try_get::<i64, _>("sequence")? as u64,
                    timestamp: row.try_get("timestamp")?,
                    level: row.try_get("level")?,
                    event: row.try_get("event")?,
                    message: row.try_get("message")?,
                    payload: serde_json::from_str(&payload_json)?,
                })
            })
            .collect()
    }

    pub async fn latest_interrupted_task_recovery(
        &self,
        conversation_id: &str,
        director_agent_code: &str,
    ) -> Result<Option<InterruptedTaskRecovery>, StoreError> {
        let row = sqlx::query(
            "SELECT t.id AS task_id, ta.id AS attempt_id, ta.codex_turn_id, t.agent_code, t.stage FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id JOIN interactions i ON i.id = t.interaction_id WHERE i.conversation_id = ? AND t.agent_code != ? AND ta.status IN ('cancelled', 'interrupted') AND NOT EXISTS (SELECT 1 FROM tasks newer JOIN interactions ni ON ni.id = newer.interaction_id WHERE ni.conversation_id = i.conversation_id AND newer.agent_code = t.agent_code AND newer.stage = t.stage AND newer.rowid > t.rowid) ORDER BY t.rowid DESC, ta.attempt_no DESC LIMIT 1",
        )
        .bind(conversation_id)
        .bind(director_agent_code)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let task_id: String = row.try_get("task_id")?;
        let events = self.list_task_events(&task_id).await?;
        Ok(Some(InterruptedTaskRecovery {
            task_id,
            attempt_id: row.try_get("attempt_id")?,
            codex_turn_id: row.try_get("codex_turn_id")?,
            agent_code: row.try_get("agent_code")?,
            stage: row.try_get("stage")?,
            events,
        }))
    }

    pub async fn running_attempts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<RunningAttempt>, StoreError> {
        let rows = sqlx::query(
            "SELECT ta.id AS attempt_id, ta.task_id, t.agent_code, ct.codex_thread_id, ta.codex_turn_id FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id JOIN interactions i ON i.id = t.interaction_id JOIN conversation_codex_threads ct ON ct.id = ta.conversation_codex_thread_id WHERE i.conversation_id = ? AND ta.status = 'running' AND ta.codex_turn_id IS NOT NULL",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(RunningAttempt {
                    attempt_id: row.try_get("attempt_id")?,
                    task_id: row.try_get("task_id")?,
                    agent_code: row.try_get("agent_code")?,
                    thread_id: row.try_get("codex_thread_id")?,
                    turn_id: row.try_get("codex_turn_id")?,
                })
            })
            .collect()
    }

    pub async fn mark_attempt_status(
        &self,
        attempt_id: &str,
        status: TaskAttemptStatus,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query("UPDATE task_attempts SET status = ? WHERE id = ?")
            .bind(attempt_status_name(status))
            .bind(attempt_id)
            .execute(&mut *transaction)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!("task attempt {attempt_id}")));
        }
        sqlx::query(
            "UPDATE tasks SET status = ? WHERE id = (SELECT task_id FROM task_attempts WHERE id = ?)",
        )
        .bind(task_status_for_attempt(status))
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn bind_turn_to_attempt(
        &self,
        attempt_id: &str,
        codex_turn_id: &str,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE task_attempts SET codex_turn_id = ?, status = 'running' WHERE id = ? AND codex_turn_id IS NULL",
        )
        .bind(codex_turn_id)
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!(
                "unbound task attempt {attempt_id}"
            )));
        }
        sqlx::query(
            "UPDATE tasks SET status = 'running' WHERE id = (SELECT task_id FROM task_attempts WHERE id = ?)",
        )
        .bind(attempt_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO turn_attempt_bindings(codex_turn_id, attempt_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(codex_turn_id)
        .bind(attempt_id)
        .bind(created_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn turn_attempt_context(
        &self,
        codex_turn_id: &str,
    ) -> Result<Option<TurnAttemptContext>, StoreError> {
        let row = sqlx::query(
            "SELECT ta.id AS attempt_id, ta.task_id, ta.attempt_no, ta.conversation_codex_thread_id, ct.codex_thread_id, i.conversation_id, m.turn, t.target_id, t.stage, t.agent_code, t.input_version, t.contract_version, t.context_json FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id JOIN interactions i ON i.id = t.interaction_id JOIN messages m ON m.id = i.idempotency_key JOIN conversation_codex_threads ct ON ct.id = ta.conversation_codex_thread_id WHERE ta.codex_turn_id = ? AND ta.status IN ('pending', 'running')",
        )
        .bind(codex_turn_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let context_json: String = row.try_get("context_json")?;
            Ok(TurnAttemptContext {
                attempt_id: row.try_get("attempt_id")?,
                task_id: row.try_get("task_id")?,
                attempt_no: row.try_get::<i64, _>("attempt_no")? as u32,
                conversation_id: row.try_get("conversation_id")?,
                turn: row.try_get::<i64, _>("turn")? as u64,
                target_id: row.try_get("target_id")?,
                stage: row.try_get("stage")?,
                agent_code: row.try_get("agent_code")?,
                input_version: row.try_get::<i64, _>("input_version")? as u64,
                contract_version: row.try_get::<i64, _>("contract_version")? as u64,
                conversation_codex_thread_id: row.try_get("conversation_codex_thread_id")?,
                codex_thread_id: row.try_get("codex_thread_id")?,
                context: serde_json::from_str(&context_json)?,
            })
        })
        .transpose()
    }

    pub async fn complete_turn(
        &self,
        codex_turn_id: &str,
        attempt_status: TaskAttemptStatus,
    ) -> Result<Option<TurnAttemptCompletion>, StoreError> {
        self.complete_turn_with_partial(codex_turn_id, attempt_status, None)
            .await
    }

    pub async fn complete_turn_with_partial(
        &self,
        codex_turn_id: &str,
        attempt_status: TaskAttemptStatus,
        partial_output: Option<&str>,
    ) -> Result<Option<TurnAttemptCompletion>, StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT ta.id AS attempt_id, ta.task_id, i.conversation_id FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id JOIN interactions i ON i.id = t.interaction_id WHERE ta.codex_turn_id = ? AND ta.status IN ('pending', 'running')",
        )
        .bind(codex_turn_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let completion = TurnAttemptCompletion {
            attempt_id: row.try_get("attempt_id")?,
            task_id: row.try_get("task_id")?,
            conversation_id: row.try_get("conversation_id")?,
        };
        sqlx::query("UPDATE task_attempts SET status = ? WHERE id = ?")
            .bind(attempt_status_name(attempt_status))
            .bind(&completion.attempt_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE tasks SET status = ? WHERE id = ?")
            .bind(task_status_for_attempt(attempt_status))
            .bind(&completion.task_id)
            .execute(&mut *transaction)
            .await?;
        let message_status = match attempt_status {
            TaskAttemptStatus::Cancelled | TaskAttemptStatus::Interrupted => "interrupted",
            TaskAttemptStatus::Failed | TaskAttemptStatus::Unknown => "failed",
            _ => "completed",
        };
        let partial_output = partial_output.filter(|output| !output.trim().is_empty());
        sqlx::query(
            "UPDATE messages SET status = ?, content = CASE WHEN content = '' AND ? IS NOT NULL THEN ? WHEN content = '' AND ? = 'failed' THEN 'Agent 执行失败，可重试本轮。' WHEN content = '' AND ? = 'interrupted' THEN '运行已中断' ELSE content END WHERE id = (SELECT id FROM messages WHERE conversation_id = ? AND status = 'thinking' ORDER BY turn DESC, created_at DESC LIMIT 1)",
        )
        .bind(message_status)
        .bind(partial_output)
        .bind(partial_output)
        .bind(message_status)
        .bind(message_status)
        .bind(&completion.conversation_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE conversations SET status = 'active' WHERE id = ?")
            .bind(&completion.conversation_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(completion))
    }

    pub async fn recover_incomplete_attempts(&self) -> Result<u64, StoreError> {
        self.require_writable()?;
        let mut transaction = self.pool.begin().await?;
        let interrupted = sqlx::query(
            "SELECT id, task_id, codex_turn_id FROM task_attempts WHERE status IN ('pending', 'running')",
        )
        .fetch_all(&mut *transaction)
        .await?;
        let result = sqlx::query(
            "UPDATE task_attempts SET status = 'interrupted' WHERE status IN ('pending', 'running')",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE tasks SET status = 'failed' WHERE status IN ('pending', 'running') AND EXISTS (SELECT 1 FROM task_attempts ta WHERE ta.task_id = tasks.id AND ta.status = 'interrupted')",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE messages SET status = 'interrupted', content = CASE WHEN content = '' THEN '运行在应用退出后中断，可重新发送。' ELSE content END WHERE status = 'thinking'")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE conversations SET status = 'active' WHERE status = 'running'")
            .execute(&mut *transaction)
            .await?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        for row in interrupted {
            let task_id: String = row.try_get("task_id")?;
            let attempt_id: String = row.try_get("id")?;
            let codex_turn_id: Option<String> = row.try_get("codex_turn_id")?;
            let sequence: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM task_events WHERE task_id = ?",
            )
            .bind(&task_id)
            .fetch_one(&mut *transaction)
            .await?;
            let payload = serde_json::json!({
                "status": "interrupted",
                "attemptId": attempt_id,
                "codexTurnId": codex_turn_id,
                "reason": "application_restarted",
            });
            sqlx::query("INSERT INTO task_events(task_id, sequence, timestamp, level, event, message, payload_json) VALUES (?, ?, ?, 'error', 'attempt_terminal', '应用退出后恢复未完成任务', ?)")
                .bind(task_id)
                .bind(sequence)
                .bind(timestamp)
                .bind(serde_json::to_string(&payload)?)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(result.rows_affected())
    }
}

fn insert_message_query(
    message: &ConversationMessage,
) -> sqlx::query::Query<'_, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    sqlx::query(
        "INSERT INTO messages(id, conversation_id, turn, role, content, agent_code, recipient_agent_code, status, token_count, folded, attachments_json, action_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&message.id)
    .bind(message.conversation_id.as_str())
    .bind(message.turn as i64)
    .bind(&message.role)
    .bind(&message.content)
    .bind(&message.agent_code)
    .bind(&message.recipient_agent_code)
    .bind(message_status_name(message.status))
    .bind(message.token_count as i64)
    .bind(message.folded)
    .bind(serde_json::to_string(&message.attachments).unwrap_or_else(|_| "[]".to_string()))
    .bind(message.action.as_ref().and_then(|action| serde_json::to_string(action).ok()))
    .bind(message.created_at)
}

fn decode_message(
    conversation_id: &str,
    row: SqliteRow,
) -> Result<ConversationMessage, StoreError> {
    let attachments: String = row.try_get("attachments_json")?;
    let action: Option<String> = row.try_get("action_json")?;
    Ok(ConversationMessage {
        id: row.try_get("id")?,
        conversation_id: ConversationId::new(conversation_id),
        turn: row.try_get::<i64, _>("turn")? as u64,
        role: row.try_get("role")?,
        content: row.try_get("content")?,
        agent_code: row.try_get("agent_code")?,
        recipient_agent_code: row.try_get("recipient_agent_code")?,
        status: parse_message_status(&row.try_get::<String, _>("status")?)?,
        token_count: row.try_get::<i64, _>("token_count")? as u64,
        folded: row.try_get("folded")?,
        attachments: serde_json::from_str(&attachments)?,
        action: action
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        created_at: row.try_get("created_at")?,
    })
}

fn decode_character(project_id: &str, row: SqliteRow) -> Result<Character, StoreError> {
    let views: String = row.try_get("view_paths_json")?;
    let constraints: String = row.try_get("hard_constraints_json")?;
    Ok(Character {
        id: row.try_get("id")?,
        project_id: project_id.to_string(),
        name: row.try_get("name")?,
        group: row.try_get("group_name")?,
        dir_name: row.try_get("dir_name")?,
        state: parse_character_state(&row.try_get::<String, _>("state")?)?,
        spec_path: row.try_get("spec_path")?,
        render_path: row.try_get("render_path")?,
        view_paths: serde_json::from_str(&views)?,
        hard_constraints: serde_json::from_str(&constraints)?,
        gate_spec_confirmed_at: row.try_get("gate_spec_confirmed_at")?,
        gate_render_confirmed_at: row.try_get("gate_render_confirmed_at")?,
        gate_views_confirmed_at: row.try_get("gate_views_confirmed_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn conversation_status_name(status: ConversationStatus) -> &'static str {
    match status {
        ConversationStatus::Active => "active",
        ConversationStatus::Running => "running",
    }
}

fn parse_conversation_status(value: &str) -> Result<ConversationStatus, sqlx::Error> {
    match value {
        "active" => Ok(ConversationStatus::Active),
        "running" => Ok(ConversationStatus::Running),
        other => Err(sqlx::Error::Decode(
            format!("unknown conversation status: {other}").into(),
        )),
    }
}

fn parse_conversation_target_kind(value: &str) -> Result<ConversationTargetKind, sqlx::Error> {
    match value {
        "project" => Ok(ConversationTargetKind::Project),
        "character" => Ok(ConversationTargetKind::Character),
        other => Err(sqlx::Error::Decode(
            format!("unknown conversation target kind: {other}").into(),
        )),
    }
}

fn message_status_name(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Thinking => "thinking",
        MessageStatus::Completed => "completed",
        MessageStatus::Failed => "failed",
        MessageStatus::Interrupted => "interrupted",
    }
}

fn parse_message_status(value: &str) -> Result<MessageStatus, sqlx::Error> {
    match value {
        "thinking" => Ok(MessageStatus::Thinking),
        "completed" | "done" => Ok(MessageStatus::Completed),
        "failed" => Ok(MessageStatus::Failed),
        "interrupted" | "cancelled" => Ok(MessageStatus::Interrupted),
        other => Err(sqlx::Error::Decode(
            format!("unknown message status: {other}").into(),
        )),
    }
}

fn generation_review_status_name(status: GenerationReviewStatus) -> &'static str {
    match status {
        GenerationReviewStatus::Pending => "pending",
        GenerationReviewStatus::Accepted => "accepted",
        GenerationReviewStatus::RevisionRequested => "revisionRequested",
        GenerationReviewStatus::Superseded => "superseded",
    }
}

fn parse_generation_review_status(value: &str) -> Result<GenerationReviewStatus, sqlx::Error> {
    match value {
        "pending" => Ok(GenerationReviewStatus::Pending),
        "accepted" => Ok(GenerationReviewStatus::Accepted),
        "revisionRequested" => Ok(GenerationReviewStatus::RevisionRequested),
        "superseded" => Ok(GenerationReviewStatus::Superseded),
        other => Err(sqlx::Error::Decode(
            format!("unknown generation review status: {other}").into(),
        )),
    }
}

fn character_state_name(state: CharacterState) -> &'static str {
    match state {
        CharacterState::S0SpecDrafting => "S0_spec_drafting",
        CharacterState::S1SpecConfirmed => "S1_spec_confirmed",
        CharacterState::S2RenderGenerated => "S2_render_generated",
        CharacterState::S3RenderConfirmed => "S3_render_confirmed",
        CharacterState::S4ViewsGenerated => "S4_views_generated",
        CharacterState::S5ViewsConfirmed => "S5_views_confirmed",
    }
}

fn parse_character_state(value: &str) -> Result<CharacterState, sqlx::Error> {
    match value {
        "S0_spec_drafting" => Ok(CharacterState::S0SpecDrafting),
        "S1_spec_confirmed" => Ok(CharacterState::S1SpecConfirmed),
        "S2_render_generated" => Ok(CharacterState::S2RenderGenerated),
        "S3_render_confirmed" => Ok(CharacterState::S3RenderConfirmed),
        "S4_views_generated" => Ok(CharacterState::S4ViewsGenerated),
        "S5_views_confirmed" => Ok(CharacterState::S5ViewsConfirmed),
        other => Err(sqlx::Error::Decode(
            format!("unknown character state: {other}").into(),
        )),
    }
}

fn parse_task_status(value: &str) -> Result<TaskStatus, sqlx::Error> {
    match value {
        "pending" => Ok(TaskStatus::Pending),
        "running" => Ok(TaskStatus::Running),
        "succeeded" => Ok(TaskStatus::Succeeded),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(sqlx::Error::Decode(
            format!("unknown task status: {other}").into(),
        )),
    }
}

fn attempt_status_name(status: TaskAttemptStatus) -> &'static str {
    match status {
        TaskAttemptStatus::Pending => "pending",
        TaskAttemptStatus::Running => "running",
        TaskAttemptStatus::Succeeded => "succeeded",
        TaskAttemptStatus::Failed => "failed",
        TaskAttemptStatus::Cancelled => "cancelled",
        TaskAttemptStatus::Interrupted => "interrupted",
        TaskAttemptStatus::Unknown => "unknown",
    }
}

async fn insert_character_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character: &Character,
    event: &str,
    message: &str,
    payload: serde_json::Value,
    created_at: i64,
) -> Result<(), StoreError> {
    let task_id = format!("character-gate:{}", character.id);
    let sequence: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM task_events WHERE task_id = ?",
    )
    .bind(&task_id)
    .fetch_one(&mut **transaction)
    .await?;
    sqlx::query("INSERT INTO task_events(task_id, sequence, timestamp, level, event, message, payload_json) VALUES (?, ?, ?, 'info', ?, ?, ?)")
        .bind(task_id)
        .bind(sequence)
        .bind(created_at)
        .bind(event)
        .bind(message)
        .bind(serde_json::to_string(&payload)?)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn task_status_for_attempt(status: TaskAttemptStatus) -> &'static str {
    match status {
        TaskAttemptStatus::Pending => "pending",
        TaskAttemptStatus::Running => "running",
        TaskAttemptStatus::Succeeded => "succeeded",
        TaskAttemptStatus::Cancelled => "cancelled",
        TaskAttemptStatus::Failed | TaskAttemptStatus::Interrupted | TaskAttemptStatus::Unknown => {
            "failed"
        }
    }
}

pub async fn register_project(
    studio: &SqlitePool,
    project: &Project,
    access: ProjectAccess,
    registered_at: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO projects(id, name, root, access_mode, registered_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, root = excluded.root, access_mode = excluded.access_mode",
    )
    .bind(project.id.as_str())
    .bind(&project.name)
    .bind(&project.root)
    .bind(match access {
        ProjectAccess::ReadWrite => "readWrite",
        ProjectAccess::ReadOnly => "readOnly",
    })
    .bind(registered_at)
    .execute(studio)
    .await?;
    Ok(())
}

pub async fn unregister_project(studio: &SqlitePool, project_id: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(project_id)
        .execute(studio)
        .await?;
    Ok(())
}

pub async fn unregister_project_by_root(
    studio: &SqlitePool,
    project_root: &str,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM projects WHERE root = ?")
        .bind(project_root)
        .execute(studio)
        .await?;
    Ok(())
}

pub async fn list_registered_projects(
    studio: &SqlitePool,
) -> Result<Vec<(Project, ProjectAccess)>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, name, root, access_mode FROM projects ORDER BY name COLLATE NOCASE, id",
    )
    .fetch_all(studio)
    .await?;
    rows.into_iter()
        .map(|row| {
            let access_mode: String = row.try_get("access_mode")?;
            let access = match access_mode.as_str() {
                "readWrite" => ProjectAccess::ReadWrite,
                "readOnly" => ProjectAccess::ReadOnly,
                other => {
                    return Err(StoreError::Database(sqlx::Error::Decode(
                        format!("unknown project access mode: {other}").into(),
                    )));
                }
            };
            let id = row.try_get::<String, _>("id")?;
            let registered_name = row.try_get::<String, _>("name")?;
            let root = row.try_get::<String, _>("root")?;
            let metadata = fs::read_to_string(Path::new(&root).join("project.json"))
                .ok()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                .filter(|value| {
                    value
                        .get("schemaVersion")
                        .and_then(serde_json::Value::as_u64)
                        == Some(2)
                });
            let name = metadata
                .as_ref()
                .and_then(|value| value.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&registered_name)
                .to_string();
            let code = metadata
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let state = match metadata
                .as_ref()
                .and_then(|value| value.get("state"))
                .and_then(serde_json::Value::as_str)
            {
                Some("styleSettled") => codex_game_domain::ProjectState::StyleSettled,
                Some("ready") => codex_game_domain::ProjectState::Ready,
                _ => codex_game_domain::ProjectState::Drafting,
            };
            Ok((
                Project {
                    id: ProjectId::new(id),
                    name,
                    code,
                    root,
                    state,
                },
                access,
            ))
        })
        .collect()
}

fn ensure_local_storage_ignored(project_root: &Path) -> Result<(), StoreError> {
    const IGNORE_ENTRY: &str = "/.codex-game/local/";
    let path = project_root.join(".gitignore");
    let existing = match fs::read_to_string(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if existing.lines().any(|line| line.trim() == IGNORE_ENTRY) {
        return Ok(());
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.write_all(format!("{IGNORE_ENTRY}\n").as_bytes())?;
    Ok(())
}

fn migrate_project_database(legacy: &Path, local: &Path) -> Result<(), StoreError> {
    if !legacy.exists() {
        return Ok(());
    }
    if local.exists() {
        return Err(StoreError::StorageMigrationConflict {
            legacy: legacy.to_path_buf(),
            local: local.to_path_buf(),
        });
    }
    fs::rename(legacy, local)?;
    for suffix in ["-shm", "-wal"] {
        let legacy_sidecar = PathBuf::from(format!("{}{suffix}", legacy.display()));
        if legacy_sidecar.exists() {
            fs::rename(
                &legacy_sidecar,
                PathBuf::from(format!("{}{suffix}", local.display())),
            )?;
        }
    }
    Ok(())
}

pub async fn open_studio_store(app_storage: &Path) -> Result<SqlitePool, StoreError> {
    fs::create_dir_all(app_storage)?;
    let pool = open_pool(&app_storage.join("studio.db"), ProjectAccess::ReadWrite).await?;
    run_schema(&pool, STUDIO_SCHEMA).await?;
    crate::initialize_ai_config_schema(&pool).await?;
    Ok(pool)
}

async fn open_pool(path: &Path, access: ProjectAccess) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(access == ProjectAccess::ReadWrite)
        .read_only(access == ProjectAccess::ReadOnly)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
        .disable_statement_logging();
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
}

async fn run_schema(pool: &SqlitePool, schema: &'static str) -> Result<(), sqlx::Error> {
    for statement in schema
        .split(';')
        .map(str::trim)
        .filter(|sql| !sql.is_empty())
    {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

async fn migrate_project_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'target_kind'",
        "ALTER TABLE conversations ADD COLUMN target_kind TEXT NOT NULL DEFAULT 'project'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'target_ref'",
        "ALTER TABLE conversations ADD COLUMN target_ref TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'title'",
        "ALTER TABLE conversations ADD COLUMN title TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'director_agent_code'",
        "ALTER TABLE conversations ADD COLUMN director_agent_code TEXT NOT NULL DEFAULT 'studio_director'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'focus_agent_code'",
        "ALTER TABLE conversations ADD COLUMN focus_agent_code TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'status'",
        "ALTER TABLE conversations ADD COLUMN status TEXT NOT NULL DEFAULT 'active'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'turn'",
        "ALTER TABLE conversations ADD COLUMN turn INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversations') WHERE name = 'updated_at'",
        "ALTER TABLE conversations ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'turn'",
        "ALTER TABLE messages ADD COLUMN turn INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'agent_code'",
        "ALTER TABLE messages ADD COLUMN agent_code TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'recipient_agent_code'",
        "ALTER TABLE messages ADD COLUMN recipient_agent_code TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'status'",
        "ALTER TABLE messages ADD COLUMN status TEXT NOT NULL DEFAULT 'completed'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'token_count'",
        "ALTER TABLE messages ADD COLUMN token_count INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'folded'",
        "ALTER TABLE messages ADD COLUMN folded INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'attachments_json'",
        "ALTER TABLE messages ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'action_json'",
        "ALTER TABLE messages ADD COLUMN action_json TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('art_bible_versions') WHERE name = 'markdown'",
        "ALTER TABLE art_bible_versions ADD COLUMN markdown TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversation_codex_threads') WHERE name = 'forked_from_id'",
        "ALTER TABLE conversation_codex_threads ADD COLUMN forked_from_id TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('conversation_codex_threads') WHERE name = 'replacement_reason'",
        "ALTER TABLE conversation_codex_threads ADD COLUMN replacement_reason TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'input_version'",
        "ALTER TABLE tasks ADD COLUMN input_version INTEGER NOT NULL DEFAULT 1",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'contract_version'",
        "ALTER TABLE tasks ADD COLUMN contract_version INTEGER NOT NULL DEFAULT 1",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'prompt'",
        "ALTER TABLE tasks ADD COLUMN prompt TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'context_json'",
        "ALTER TABLE tasks ADD COLUMN context_json TEXT NOT NULL DEFAULT '{}'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('generations') WHERE name = 'review_status'",
        "ALTER TABLE generations ADD COLUMN review_status TEXT NOT NULL DEFAULT 'pending'",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('generations') WHERE name = 'review_feedback'",
        "ALTER TABLE generations ADD COLUMN review_feedback TEXT",
    )
    .await?;
    ensure_column(
        pool,
        "SELECT COUNT(*) FROM pragma_table_info('generations') WHERE name = 'reviewed_at'",
        "ALTER TABLE generations ADD COLUMN reviewed_at INTEGER",
    )
    .await?;
    sqlx::query("UPDATE generations SET review_status = CASE WHEN is_final = 1 THEN 'accepted' ELSE 'superseded' END WHERE review_status = 'pending' AND EXISTS (SELECT 1 FROM generations AS final_generation WHERE final_generation.project_id = generations.project_id AND final_generation.target_kind = generations.target_kind AND final_generation.target_ref = generations.target_ref AND final_generation.stage = generations.stage AND final_generation.is_final = 1)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS unique_interaction_stage_agent ON tasks(interaction_id, stage, agent_code)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS unique_conversation_target ON conversations(project_id, target_kind, COALESCE(target_ref, ''))",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS messages_by_conversation_turn ON messages(conversation_id, turn, created_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn bounded_event_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    const SUFFIX: &str = "\n[truncated]";
    let mut boundary = max_bytes.saturating_sub(SUFFIX.len());
    while !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    format!("{}{SUFFIX}", &value[..boundary])
}

fn sanitize_task_event_payload(value: &serde_json::Value, key: Option<&str>) -> serde_json::Value {
    if key.is_some_and(is_sensitive_event_key) {
        return serde_json::Value::String("[redacted]".to_string());
    }
    match value {
        serde_json::Value::String(value) => {
            let sanitized = if value.starts_with("data:") && value.contains(";base64,") {
                "[data URL omitted]".to_string()
            } else {
                bounded_event_text(value, 8 * 1024)
            };
            serde_json::Value::String(sanitized)
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .take(64)
                .map(|value| sanitize_task_event_payload(value, None))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .iter()
                .take(64)
                .map(|(key, value)| {
                    (
                        bounded_event_text(key, 128),
                        sanitize_task_event_payload(value, Some(key)),
                    )
                })
                .collect(),
        ),
        value => value.clone(),
    }
}

fn is_sensitive_event_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "token"
            | "access_token"
            | "refreshtoken"
            | "refresh_token"
            | "bearer_token"
            | "secret"
            | "password"
            | "api_key"
            | "apikey"
            | "authorization"
    ) || key.ends_with("_secret")
}

async fn ensure_column(
    pool: &SqlitePool,
    count_sql: &'static str,
    alter_sql: &'static str,
) -> Result<(), sqlx::Error> {
    let count: i64 = sqlx::query_scalar(count_sql).fetch_one(pool).await?;
    if count == 0 {
        sqlx::query(alter_sql).execute(pool).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_game_domain::AgentActionKind;
    use codex_game_domain::AgentActionPayload;
    use codex_game_domain::ArtifactDraft;
    use codex_game_domain::ArtifactSlot;
    use codex_game_domain::ChoiceGroup;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[tokio::test]
    async fn explicit_and_contended_opens_are_read_only() {
        let directory = tempdir().expect("tempdir");
        let writer = ProjectStore::open(directory.path())
            .await
            .expect("open writer");
        let explicit_reader = ProjectStore::open_read_only(directory.path())
            .await
            .expect("open explicit reader");
        let contended_reader = ProjectStore::open(directory.path())
            .await
            .expect("open contended reader");

        assert_eq!(writer.access(), ProjectAccess::ReadWrite);
        assert_eq!(explicit_reader.access(), ProjectAccess::ReadOnly);
        assert_eq!(contended_reader.access(), ProjectAccess::ReadOnly);
    }

    #[tokio::test]
    async fn empty_character_groups_are_persisted() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        store
            .insert_character_group("project-1", "主角", 1)
            .await
            .expect("create group");
        store
            .insert_character_group("project-1", "主角", 2)
            .await
            .expect("create group idempotently");

        assert_eq!(
            store
                .list_character_groups("project-1")
                .await
                .expect("list groups"),
            vec!["主角".to_string()]
        );
    }

    #[tokio::test]
    async fn task_events_are_ordered_bounded_and_recoverable() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        sqlx::query("INSERT INTO conversations(id, project_id, target_kind, title, director_agent_code, status, turn, created_at, updated_at) VALUES ('conversation-1', 'project-1', 'character', '角色', 'studio_director', 'active', 1, 1, 1)")
            .execute(store.pool())
            .await
            .expect("seed conversation");
        sqlx::query("INSERT INTO interactions(id, conversation_id, idempotency_key, created_at) VALUES ('interaction-1', 'conversation-1', 'key-1', 1)")
            .execute(store.pool())
            .await
            .expect("seed interaction");
        sqlx::query("INSERT INTO tasks(id, interaction_id, target_id, stage, agent_code, status) VALUES ('task-1', 'interaction-1', 'character-1', 'render', 'visual_designer', 'failed')")
            .execute(store.pool())
            .await
            .expect("seed task");
        sqlx::query("INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, codex_turn_id, status) VALUES ('attempt-1', 'task-1', 1, 'thread-1', 'turn-1', 'interrupted')")
            .execute(store.pool())
            .await
            .expect("seed attempt");

        store
            .append_task_event(
                "task-1",
                "info",
                "image_generation_succeeded",
                &"m".repeat(9 * 1024),
                &serde_json::json!({
                    "attemptId": "attempt-1",
                    "savedPath": "media/render/result.png",
                    "api_key": "secret-value",
                    "result": "data:image/png;base64,aGVsbG8=",
                }),
                10,
            )
            .await
            .expect("append event");
        store
            .append_task_event(
                "task-1",
                "info",
                "partial_response",
                "partial",
                &serde_json::json!({ "attemptId": "attempt-1", "text": "done" }),
                11,
            )
            .await
            .expect("append second event");

        let recovery = store
            .latest_interrupted_task_recovery("conversation-1", "studio_director")
            .await
            .expect("load recovery")
            .expect("recovery exists");
        assert_eq!(recovery.attempt_id, "attempt-1");
        assert_eq!(recovery.events.len(), 2);
        assert_eq!(recovery.events[0].sequence, 1);
        assert_eq!(recovery.events[1].sequence, 2);
        assert!(recovery.events[0].message.len() <= 8 * 1024);
        assert_eq!(recovery.events[0].payload["api_key"], "[redacted]");
        assert_eq!(recovery.events[0].payload["result"], "[data URL omitted]");
    }

    #[tokio::test]
    async fn generation_revision_starts_atomically_with_feedback_message() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        let active_conversation = Conversation {
            id: ConversationId::new("conversation-1"),
            project_id: ProjectId::new("project-1"),
            target_kind: ConversationTargetKind::Character,
            target_ref: Some("character-1".to_string()),
            title: "角色视觉".to_string(),
            director_agent_code: "studio_director".to_string(),
            focus_agent_code: None,
            status: ConversationStatus::Active,
            turn: 0,
            created_at: 1,
            updated_at: 1,
        };
        store
            .insert_conversation(&active_conversation)
            .await
            .expect("seed conversation");
        store
            .insert_generation(&Generation {
                id: "generation-1".to_string(),
                project_id: "project-1".to_string(),
                target_kind: "character".to_string(),
                target_ref: "character-1".to_string(),
                stage: "render".to_string(),
                variant: None,
                file_path: "characters/character-1/tmp/focus/media/render/result.png".to_string(),
                file_hash: Some("hash".to_string()),
                is_final: false,
                review_status: GenerationReviewStatus::Pending,
                review_feedback: None,
                reviewed_at: None,
                source: "image_i2i".to_string(),
                task_id: Some("task-1".to_string()),
                asset_spec: serde_json::json!({}),
                created_at: 1,
            })
            .await
            .expect("seed generation");

        let mut running_conversation = active_conversation.clone();
        running_conversation.status = ConversationStatus::Running;
        running_conversation.focus_agent_code = Some("studio_director".to_string());
        running_conversation.turn = 1;
        running_conversation.updated_at = 2;
        let user_message = ConversationMessage {
            id: "message-user".to_string(),
            conversation_id: running_conversation.id.clone(),
            turn: 1,
            role: "user".to_string(),
            content: "请调整服装材质".to_string(),
            agent_code: "user".to_string(),
            recipient_agent_code: Some("studio_director".to_string()),
            status: MessageStatus::Completed,
            token_count: 0,
            folded: false,
            attachments: vec![serde_json::json!({ "kind": "feedbackImage" })],
            action: None,
            created_at: 2,
        };
        let assistant_message = ConversationMessage {
            id: "message-assistant".to_string(),
            conversation_id: running_conversation.id.clone(),
            turn: 1,
            role: "assistant".to_string(),
            content: String::new(),
            agent_code: "studio_director".to_string(),
            recipient_agent_code: None,
            status: MessageStatus::Thinking,
            token_count: 0,
            folded: false,
            attachments: Vec::new(),
            action: None,
            created_at: 2,
        };

        store
            .begin_generation_revision_turn(
                &running_conversation,
                &user_message,
                &assistant_message,
                "generation-1",
                "character-1",
                "请调整服装材质",
                2,
            )
            .await
            .expect("begin revision");

        let generation = store
            .read_generation("generation-1")
            .await
            .expect("read generation")
            .expect("generation exists");
        assert_eq!(
            generation.review_status,
            GenerationReviewStatus::RevisionRequested
        );
        assert_eq!(
            generation.review_feedback.as_deref(),
            Some("请调整服装材质")
        );
        let stored: (String, i64, String) = sqlx::query_as(
            "SELECT status, turn, focus_agent_code FROM conversations WHERE id = 'conversation-1'",
        )
        .fetch_one(store.pool())
        .await
        .expect("read conversation");
        assert_eq!(
            stored,
            ("running".to_string(), 1, "studio_director".to_string())
        );
        let attachments: String =
            sqlx::query_scalar("SELECT attachments_json FROM messages WHERE id = 'message-user'")
                .fetch_one(store.pool())
                .await
                .expect("read attachments");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&attachments).expect("valid attachments"),
            serde_json::json!([{ "kind": "feedbackImage" }])
        );
    }

    #[tokio::test]
    async fn generation_revision_rolls_back_when_conversation_is_busy() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        let conversation = Conversation {
            id: ConversationId::new("conversation-1"),
            project_id: ProjectId::new("project-1"),
            target_kind: ConversationTargetKind::Character,
            target_ref: Some("character-1".to_string()),
            title: "角色视觉".to_string(),
            director_agent_code: "studio_director".to_string(),
            focus_agent_code: Some("studio_director".to_string()),
            status: ConversationStatus::Running,
            turn: 1,
            created_at: 1,
            updated_at: 1,
        };
        store
            .insert_conversation(&conversation)
            .await
            .expect("seed busy conversation");
        store
            .insert_generation(&Generation {
                id: "generation-1".to_string(),
                project_id: "project-1".to_string(),
                target_kind: "character".to_string(),
                target_ref: "character-1".to_string(),
                stage: "render".to_string(),
                variant: None,
                file_path: "characters/character-1/tmp/focus/media/render/result.png".to_string(),
                file_hash: None,
                is_final: false,
                review_status: GenerationReviewStatus::Pending,
                review_feedback: None,
                reviewed_at: None,
                source: "image_i2i".to_string(),
                task_id: None,
                asset_spec: serde_json::json!({}),
                created_at: 1,
            })
            .await
            .expect("seed generation");
        let message = |id: &str, role: &str, status| ConversationMessage {
            id: id.to_string(),
            conversation_id: conversation.id.clone(),
            turn: 2,
            role: role.to_string(),
            content: String::new(),
            agent_code: role.to_string(),
            recipient_agent_code: None,
            status,
            token_count: 0,
            folded: false,
            attachments: Vec::new(),
            action: None,
            created_at: 2,
        };

        assert!(
            store
                .begin_generation_revision_turn(
                    &conversation,
                    &message("message-user", "user", MessageStatus::Completed),
                    &message(
                        "message-assistant",
                        "studio_director",
                        MessageStatus::Thinking,
                    ),
                    "generation-1",
                    "character-1",
                    "重试",
                    2,
                )
                .await
                .is_err()
        );

        let generation = store
            .read_generation("generation-1")
            .await
            .expect("read generation")
            .expect("generation exists");
        assert_eq!(generation.review_status, GenerationReviewStatus::Pending);
        let message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = 'conversation-1'",
        )
        .fetch_one(store.pool())
        .await
        .expect("count messages");
        assert_eq!(message_count, 0);
    }

    #[tokio::test]
    async fn binds_turn_and_attempt_in_one_store_operation() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        sqlx::query(
            "INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, status) VALUES ('a1', 't1', 1, 'ct1', 'pending')",
        )
        .execute(store.pool())
        .await
        .expect("seed attempt");
        store
            .bind_turn_to_attempt("a1", "turn1", 1)
            .await
            .expect("bind turn");
        let status: String = sqlx::query_scalar("SELECT status FROM task_attempts WHERE id = 'a1'")
            .fetch_one(store.pool())
            .await
            .expect("read status");
        let attempt: String = sqlx::query_scalar(
            "SELECT attempt_id FROM turn_attempt_bindings WHERE codex_turn_id = 'turn1'",
        )
        .fetch_one(store.pool())
        .await
        .expect("read binding");
        assert_eq!(status, "running");
        assert_eq!(attempt, "a1");
    }

    #[tokio::test]
    async fn completing_turn_updates_attempt_and_task_atomically() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        sqlx::query(
            "INSERT INTO interactions(id, conversation_id, idempotency_key, created_at) VALUES ('i1', 'c1', 'key1', 1)",
        )
        .execute(store.pool())
        .await
        .expect("seed interaction");
        sqlx::query(
            "INSERT INTO tasks(id, interaction_id, target_id, stage, agent_code, status) VALUES ('t1', 'i1', 'w1', 'brief', 'brief', 'running')",
        )
        .execute(store.pool())
        .await
        .expect("seed task");
        sqlx::query(
            "INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, codex_turn_id, status) VALUES ('a1', 't1', 1, 'ct1', 'turn1', 'running')",
        )
        .execute(store.pool())
        .await
        .expect("seed attempt");
        let completion = store
            .complete_turn("turn1", TaskAttemptStatus::Succeeded)
            .await
            .expect("complete turn")
            .expect("known turn");
        assert_eq!(completion.conversation_id, "c1");
        let statuses: (String, String) = sqlx::query_as(
            "SELECT ta.status, t.status FROM task_attempts ta JOIN tasks t ON t.id = ta.task_id WHERE ta.id = 'a1'",
        )
        .fetch_one(store.pool())
        .await
        .expect("read statuses");
        assert_eq!(statuses, ("succeeded".to_string(), "succeeded".to_string()));
    }

    #[tokio::test]
    async fn new_interaction_supersedes_obsolete_pending_drafts() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        sqlx::query(
            "INSERT INTO interactions(id, conversation_id, idempotency_key, created_at) VALUES ('i1', 'c1', 'key1', 1)",
        )
        .execute(store.pool())
        .await
        .expect("seed interaction");
        sqlx::query(
            "INSERT INTO tasks(id, interaction_id, target_id, stage, agent_code, status) VALUES ('t1', 'i1', 'w1', 'project', 'game_designer', 'running')",
        )
        .execute(store.pool())
        .await
        .expect("seed task");
        sqlx::query(
            "INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, codex_turn_id, status) VALUES ('a1', 't1', 1, 'ct1', 'turn1', 'running')",
        )
        .execute(store.pool())
        .await
        .expect("seed attempt");
        sqlx::query(
            "INSERT INTO messages(id, conversation_id, role, content, status, created_at) VALUES ('m1', 'c1', 'assistant', '', 'thinking', 1)",
        )
        .execute(store.pool())
        .await
        .expect("seed message");
        sqlx::query(
            "INSERT INTO artifact_drafts(id, conversation_id, artifact_slot, resolved_path, content, status, created_at) VALUES ('d1', 'c1', 'project_art_bible', 'art-bible.md', 'old art bible', 'pending', 1), ('d2', 'c1', 'project_manifest', 'project.json', 'old project', 'pending', 1)",
        )
        .execute(store.pool())
        .await
        .expect("seed drafts");
        let action = AgentAction {
            action: AgentActionKind::AskUser,
            target_agent: None,
            reason: "请确认更新后的风格".to_string(),
            payload: AgentActionPayload {
                drafts: Some(vec![ArtifactDraft {
                    artifact_slot: ArtifactSlot::ProjectArtBible,
                    content: "new art bible".to_string(),
                    based_on_hash: None,
                }]),
                ..Default::default()
            },
        };

        store
            .commit_action_turn("turn1", "m1", "updated", &action, &[], None, 2)
            .await
            .expect("commit action turn");

        let drafts: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT resolved_path, content, status FROM artifact_drafts WHERE conversation_id = 'c1' ORDER BY resolved_path, created_at, content",
        )
        .fetch_all(store.pool())
        .await
        .expect("read drafts");
        assert_eq!(
            drafts,
            vec![
                (
                    "art-bible.md".to_string(),
                    "old art bible".to_string(),
                    "superseded".to_string(),
                ),
                (
                    "art-bible.md".to_string(),
                    "new art bible".to_string(),
                    "pending".to_string(),
                ),
                (
                    "project.json".to_string(),
                    "old project".to_string(),
                    "pending".to_string(),
                ),
            ]
        );

        sqlx::query("UPDATE tasks SET status = 'running' WHERE id = 't1'")
            .execute(store.pool())
            .await
            .expect("reset task for choice turn");
        sqlx::query(
            "INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, codex_turn_id, status) VALUES ('a2', 't1', 2, 'ct1', 'turn2', 'running')",
        )
        .execute(store.pool())
        .await
        .expect("seed choice attempt");
        sqlx::query(
            "INSERT INTO messages(id, conversation_id, role, content, status, created_at) VALUES ('m2', 'c1', 'assistant', '', 'thinking', 3)",
        )
        .execute(store.pool())
        .await
        .expect("seed choice message");
        let choice_action = AgentAction {
            action: AgentActionKind::AskUser,
            target_agent: None,
            reason: "请先确认风格".to_string(),
            payload: AgentActionPayload {
                choices: Some(vec![ChoiceGroup {
                    item: "风格".to_string(),
                    options: vec!["写实".to_string(), "卡通".to_string()],
                    recommended: vec!["卡通".to_string()],
                    multiple: false,
                }]),
                ..Default::default()
            },
        };

        store
            .commit_action_turn("turn2", "m2", "请选择风格", &choice_action, &[], None, 4)
            .await
            .expect("commit choice turn");

        let pending_draft_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM artifact_drafts WHERE conversation_id = 'c1' AND status = 'pending'",
        )
        .fetch_one(store.pool())
        .await
        .expect("count pending drafts");
        assert_eq!(pending_draft_count, 0);
    }

    #[tokio::test]
    async fn commit_action_deduplicates_generation_by_stage_and_hash() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        sqlx::query("INSERT INTO conversations(id, project_id, target_kind, target_ref, title, director_agent_code, status, turn, created_at, updated_at) VALUES ('c1', 'project-1', 'character', 'character-1', '角色', 'studio_director', 'running', 1, 1, 1)")
            .execute(store.pool())
            .await
            .expect("seed conversation");
        sqlx::query("INSERT INTO interactions(id, conversation_id, idempotency_key, created_at) VALUES ('i1', 'c1', 'key1', 1)")
            .execute(store.pool())
            .await
            .expect("seed interaction");
        sqlx::query("INSERT INTO tasks(id, interaction_id, target_id, stage, agent_code, status) VALUES ('t1', 'i1', 'character-1', 'render', 'visual_designer', 'running')")
            .execute(store.pool())
            .await
            .expect("seed task");
        sqlx::query("INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, codex_turn_id, status) VALUES ('a1', 't1', 1, 'ct1', 'turn1', 'running')")
            .execute(store.pool())
            .await
            .expect("seed attempt");
        sqlx::query("INSERT INTO messages(id, conversation_id, role, content, agent_code, status, created_at) VALUES ('m1', 'c1', 'assistant', '', 'visual_designer', 'thinking', 1)")
            .execute(store.pool())
            .await
            .expect("seed message");
        let generation = |id: &str, task_id: &str| Generation {
            id: id.to_string(),
            project_id: "project-1".to_string(),
            target_kind: "character".to_string(),
            target_ref: "character-1".to_string(),
            stage: "render".to_string(),
            variant: None,
            file_path: "characters/character-1/tmp/focus/media/render/hash.png".to_string(),
            file_hash: Some("same-hash".to_string()),
            is_final: false,
            review_status: GenerationReviewStatus::Pending,
            review_feedback: None,
            reviewed_at: None,
            source: "image_t2i".to_string(),
            task_id: Some(task_id.to_string()),
            asset_spec: serde_json::json!({}),
            created_at: 1,
        };
        store
            .insert_generation(&generation("generation-existing", "old-task"))
            .await
            .expect("seed generation");
        let action = AgentAction {
            action: AgentActionKind::Handoff,
            target_agent: Some("studio_director".to_string()),
            reason: "提交恢复产物".to_string(),
            payload: AgentActionPayload::default(),
        };

        store
            .commit_action_turn(
                "turn1",
                "m1",
                "done",
                &action,
                &[generation("generation-duplicate", "t1")],
                None,
                2,
            )
            .await
            .expect("commit duplicate generation");

        let generations = store
            .list_generations("project-1", "character", "character-1", Some("render"))
            .await
            .expect("list generations");
        assert_eq!(generations.len(), 1);
        assert_eq!(generations[0].id, "generation-existing");
        assert_eq!(
            generations[0].review_status,
            GenerationReviewStatus::Pending
        );
    }

    #[tokio::test]
    async fn recovery_interrupts_unfinished_attempts() {
        let directory = tempdir().expect("tempdir");
        let store = ProjectStore::open(directory.path())
            .await
            .expect("open store");
        sqlx::query(
            "INSERT INTO task_attempts(id, task_id, attempt_no, conversation_codex_thread_id, status) VALUES ('a1', 't1', 1, 'ct1', 'running')",
        )
        .execute(store.pool())
        .await
        .expect("seed attempt");
        assert_eq!(
            store.recover_incomplete_attempts().await.expect("recover"),
            1
        );
        let events = store.list_task_events("t1").await.expect("task events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "attempt_terminal");
        assert_eq!(events[0].payload["status"], "interrupted");
        assert_eq!(events[0].payload["attemptId"], "a1");
    }

    #[test]
    fn appends_local_storage_ignore_without_overwriting_existing_rules() {
        let directory = tempdir().expect("tempdir");
        let gitignore = directory.path().join(".gitignore");
        fs::write(&gitignore, "target/\n").expect("seed gitignore");

        ensure_local_storage_ignored(directory.path()).expect("append ignore rule");
        ensure_local_storage_ignored(directory.path()).expect("keep ignore rule idempotent");

        assert_eq!(
            fs::read_to_string(gitignore).expect("read gitignore"),
            "target/\n/.codex-game/local/\n"
        );
    }

    #[test]
    fn migrates_legacy_project_database_and_sidecars() {
        let directory = tempdir().expect("tempdir");
        let legacy = directory.path().join("project.db");
        let local_dir = directory.path().join("local");
        let local = local_dir.join("project.db");
        fs::create_dir_all(&local_dir).expect("create local directory");
        fs::write(&legacy, b"database").expect("seed database");
        fs::write(format!("{}-wal", legacy.display()), b"wal").expect("seed wal");
        fs::write(format!("{}-shm", legacy.display()), b"shm").expect("seed shm");

        migrate_project_database(&legacy, &local).expect("migrate database");

        assert!(!legacy.exists());
        assert_eq!(
            fs::read(&local).expect("read migrated database"),
            b"database"
        );
        assert!(PathBuf::from(format!("{}-wal", local.display())).exists());
        assert!(PathBuf::from(format!("{}-shm", local.display())).exists());
    }

    #[test]
    fn rejects_project_database_migration_conflict() {
        let directory = tempdir().expect("tempdir");
        let legacy = directory.path().join("project.db");
        let local = directory.path().join("local.db");
        fs::write(&legacy, b"legacy").expect("seed legacy database");
        fs::write(&local, b"local").expect("seed local database");

        assert!(matches!(
            migrate_project_database(&legacy, &local),
            Err(StoreError::StorageMigrationConflict { .. })
        ));
        assert_eq!(fs::read(legacy).expect("read legacy database"), b"legacy");
        assert_eq!(fs::read(local).expect("read local database"), b"local");
    }
}
