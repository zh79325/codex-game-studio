use crate::AgentAction;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoff {
    pub id: i64,
    pub conversation_id: String,
    pub turn: u64,
    pub from_agent_code: String,
    pub to_agent_code: String,
    pub source: String,
    pub reason: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDraftRecord {
    pub id: String,
    pub conversation_id: String,
    pub target_path: String,
    pub content: String,
    pub based_on_hash: Option<String>,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMemory {
    pub id: String,
    pub conversation_id: String,
    pub scope: String,
    pub kind: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemory {
    pub id: String,
    pub project_id: String,
    pub character_ref: Option<String>,
    pub kind: String,
    pub content: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generation {
    pub id: String,
    pub project_id: String,
    pub target_kind: String,
    pub target_ref: String,
    pub stage: String,
    pub variant: Option<String>,
    pub file_path: String,
    pub file_hash: Option<String>,
    pub is_final: bool,
    pub source: String,
    pub task_id: Option<String>,
    pub asset_spec: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: String,
    pub sequence: u64,
    pub timestamp: i64,
    pub level: String,
    pub event: String,
    pub message: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub turn: u64,
    pub messages: Vec<crate::ConversationMessage>,
    pub action: Option<AgentAction>,
    pub drafts: Vec<ArtifactDraftRecord>,
    pub handoffs: Vec<AgentHandoff>,
}
