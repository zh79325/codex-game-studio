use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const ACTION_START: &str = "<-------- ACTION-START------->";
pub const ACTION_END: &str = "<-------- ACTION-END------->";
pub const MAX_CHOICE_GROUPS: usize = 4;
pub const MAX_HANDOFFS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActionKind {
    AskUser,
    Handoff,
    Done,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceGroup {
    pub item: String,
    pub options: Vec<String>,
    pub recommended: Vec<String>,
    pub multiple: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProgress {
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    pub next_step: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDraft {
    pub target_path: String,
    pub content: String,
    #[serde(default)]
    pub based_on_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationMemoryInput {
    pub scope: String,
    pub kind: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectNamingSuggestion {
    pub name: String,
    pub code: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    pub code: String,
    pub name: String,
    pub category: String,
    pub size: String,
    pub format: String,
    pub file_name: String,
    pub description: String,
    pub anchors: String,
    pub constraints: Vec<String>,
    pub view_background_color: String,
    pub prompt: String,
    pub negative_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerdictConstraint {
    pub item: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentVerdict {
    pub token: String,
    pub decision: String,
    pub sections: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    pub constraints: Vec<VerdictConstraint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResultStatus {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResult {
    pub status: AgentResultStatus,
    pub artifacts: Vec<BTreeMap<String, Value>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentActionPayload {
    #[serde(default)]
    pub choices: Option<Vec<ChoiceGroup>>,
    #[serde(default)]
    pub progress: Option<AgentProgress>,
    #[serde(default)]
    pub drafts: Option<Vec<ArtifactDraft>>,
    #[serde(default)]
    pub memories: Option<Vec<ConversationMemoryInput>>,
    #[serde(default)]
    pub naming: Option<Vec<ProjectNamingSuggestion>>,
    #[serde(default)]
    pub asset_specs: Option<Vec<AssetSpec>>,
    #[serde(default)]
    pub verdict: Option<AgentVerdict>,
    #[serde(default)]
    pub result: Option<AgentResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAction {
    pub action: AgentActionKind,
    pub target_agent: Option<String>,
    pub reason: String,
    pub payload: AgentActionPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentTurnOutput {
    pub text: String,
    pub action: AgentAction,
}
