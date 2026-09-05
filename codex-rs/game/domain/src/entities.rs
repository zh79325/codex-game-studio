use crate::ArtBibleVersionId;
use crate::ArtifactId;
use crate::ConversationCodexThreadId;
use crate::ConversationId;
use crate::InteractionId;
use crate::ProjectId;
use crate::TaskAttemptId;
use crate::TaskId;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub code: Option<String>,
    pub root: String,
    pub state: ProjectState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectState {
    Drafting,
    StyleSettled,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTargetKind {
    Project,
    Character,
}

impl ConversationTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Character => "character",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationStatus {
    Active,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: ConversationId,
    pub project_id: ProjectId,
    pub target_kind: ConversationTargetKind,
    pub target_ref: Option<String>,
    pub title: String,
    pub director_agent_code: String,
    pub focus_agent_code: Option<String>,
    pub status: ConversationStatus,
    pub turn: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Thinking,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub id: String,
    pub conversation_id: ConversationId,
    pub turn: u64,
    pub role: String,
    pub content: String,
    pub agent_code: String,
    pub recipient_agent_code: Option<String>,
    pub status: MessageStatus,
    pub token_count: u64,
    pub folded: bool,
    pub attachments: Vec<serde_json::Value>,
    pub action: Option<crate::AgentAction>,
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
    pub contract_version: u64,
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
pub const MAX_REVIEW_SUBJECT_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowVerdictSummary {
    pub token: String,
    pub subject_id: String,
    pub decision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContext {
    pub phase: String,
    pub pending_draft_id: Option<String>,
    pub review_status: String,
    pub latest_verdict: Option<WorkflowVerdictSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSubject {
    pub id: String,
    pub target_path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackage {
    pub conversation_history: Vec<String>,
    pub context_version: u64,
    pub contract_version: u64,
    pub agent_definition_version: String,
    pub output_schema: String,
    #[serde(default)]
    pub target_kind: String,
    #[serde(default)]
    pub target_ref: Option<String>,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub art_bible: Option<String>,
    #[serde(default)]
    pub character_context: Option<String>,
    #[serde(default)]
    pub workflow_context: Option<WorkflowContext>,
    #[serde(default)]
    pub review_subject: Option<ReviewSubject>,
    #[serde(default)]
    pub memories: Vec<String>,
    #[serde(default)]
    pub allowed_handoffs: Vec<String>,
    #[serde(default)]
    pub action_protocol: String,
}

impl ContextPackage {
    pub fn is_bounded(&self) -> bool {
        self.conversation_history.len() <= MAX_CONTEXT_SUMMARIES
            && self.memories.len() <= MAX_CONTEXT_SUMMARIES
            && self
                .conversation_history
                .iter()
                .chain(self.memories.iter())
                .all(|summary| summary.len() <= MAX_CONTEXT_SUMMARY_BYTES)
            && self
                .art_bible
                .as_ref()
                .is_none_or(|value| value.len() <= 64 * 1024)
            && self
                .character_context
                .as_ref()
                .is_none_or(|value| value.len() <= 32 * 1024)
            && self.review_subject.as_ref().is_none_or(|subject| {
                subject.id.len() <= MAX_CONTEXT_SUMMARY_BYTES
                    && subject.target_path.len() <= MAX_CONTEXT_SUMMARY_BYTES
                    && subject.content.len() <= MAX_REVIEW_SUBJECT_BYTES
            })
            && self.action_protocol.len() <= MAX_CONTEXT_SUMMARY_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_with_review_subject(content: String) -> ContextPackage {
        ContextPackage {
            conversation_history: Vec::new(),
            context_version: 1,
            contract_version: 1,
            agent_definition_version: "1".to_string(),
            output_schema: String::new(),
            target_kind: "character".to_string(),
            target_ref: Some("character-1".to_string()),
            stage: "spec".to_string(),
            art_bible: None,
            character_context: None,
            workflow_context: None,
            review_subject: Some(ReviewSubject {
                id: "draft-1".to_string(),
                target_path: "docs/角色定稿.md".to_string(),
                content,
            }),
            memories: Vec::new(),
            allowed_handoffs: Vec::new(),
            action_protocol: String::new(),
        }
    }

    #[test]
    fn review_subject_is_limited_to_32_kib() {
        assert!(context_with_review_subject("x".repeat(MAX_REVIEW_SUBJECT_BYTES)).is_bounded());
        assert!(
            !context_with_review_subject("x".repeat(MAX_REVIEW_SUBJECT_BYTES + 1)).is_bounded()
        );
    }
}
