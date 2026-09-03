use crate::ArtifactId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredBrief {
    pub core_experience: String,
    pub theme_and_mood: String,
    pub target_players: String,
    pub player_perspective: String,
    pub gameplay_pillars: Vec<String>,
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewReport {
    pub agent_code: String,
    pub findings: Vec<String>,
    pub risks: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conflict {
    pub key: String,
    pub description: String,
    pub options: Vec<String>,
    pub high_impact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictSet {
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtBibleDraft {
    pub markdown: String,
    pub unresolved_assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SynthesisResult {
    pub draft: ArtBibleDraft,
    pub conflicts: ConflictSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserDecision {
    pub conflict_key: String,
    pub selected_option: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "content", rename_all = "camelCase")]
pub enum ArtifactContent {
    StructuredBrief(StructuredBrief),
    ReviewReport(ReviewReport),
    ConflictSet(ConflictSet),
    ArtBibleDraft(ArtBibleDraft),
    UserDecision(UserDecision),
    Other(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: ArtifactId,
    pub input_version: u64,
    pub workflow_version: u64,
    pub content: ArtifactContent,
    pub created_at: i64,
}
