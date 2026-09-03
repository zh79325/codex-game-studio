use crate::ArtBibleVersionId;
use crate::ArtifactId;
use crate::ConversationCodexThreadId;
use crate::ConversationId;
use crate::FocusWorkflowId;
use crate::InteractionId;
use crate::ProjectId;
use crate::TaskAttemptId;
use crate::TaskId;
use crate::WorkflowState;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root: String,
    pub state: ProjectState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectState {
    Unversioned,
    FocusInProgress,
    Versioned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: ConversationId,
    pub project_id: ProjectId,
    pub target_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub id: String,
    pub conversation_id: ConversationId,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub id: InteractionId,
    pub conversation_id: ConversationId,
    pub idempotency_key: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusWorkflow {
    pub id: FocusWorkflowId,
    pub project_id: ProjectId,
    pub conversation_id: ConversationId,
    pub state: WorkflowState,
    pub input_version: u64,
    pub workflow_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: TaskId,
    pub interaction_id: InteractionId,
    pub target_id: String,
    pub stage: String,
    pub agent_code: String,
    pub input_artifact_ids: Vec<ArtifactId>,
    pub input_version: u64,
    pub workflow_version: u64,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskAttemptStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAttempt {
    pub id: TaskAttemptId,
    pub task_id: TaskId,
    pub attempt_no: u32,
    pub conversation_codex_thread_id: ConversationCodexThreadId,
    pub codex_turn_id: Option<String>,
    pub output_artifact_id: Option<ArtifactId>,
    pub status: TaskAttemptStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadBindingStatus {
    Active,
    Archived,
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCodexThread {
    pub id: ConversationCodexThreadId,
    pub conversation_id: ConversationId,
    pub agent_code: String,
    pub codex_thread_id: String,
    pub codex_session_id: String,
    pub status: ThreadBindingStatus,
    pub binding_version: u64,
    pub context_version: u64,
    pub agent_definition_version: String,
    pub forked_from_id: Option<ConversationCodexThreadId>,
    pub replacement_reason: Option<String>,
    pub created_at: i64,
    pub last_used_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtBibleVersion {
    pub id: ArtBibleVersionId,
    pub project_id: ProjectId,
    pub version: u64,
    pub content_hash: String,
    pub source_artifact_ids: Vec<ArtifactId>,
    pub created_at: i64,
}

pub const MAX_CONTEXT_SUMMARIES: usize = 32;
pub const MAX_CONTEXT_SUMMARY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackage {
    pub brief_artifact_id: ArtifactId,
    pub confirmed_decisions: Vec<String>,
    pub artifact_summaries: Vec<String>,
    pub context_version: u64,
    pub workflow_version: u64,
    pub agent_definition_version: String,
    pub output_schema: String,
}

impl ContextPackage {
    pub fn is_bounded(&self) -> bool {
        self.artifact_summaries.len() <= MAX_CONTEXT_SUMMARIES
            && self
                .artifact_summaries
                .iter()
                .all(|summary| summary.len() <= MAX_CONTEXT_SUMMARY_BYTES)
    }
}
