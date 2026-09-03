use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiCapability {
    TextReasoning,
    TextStructuredOutput,
    VisionAnalysis,
    ImageTextToImage,
    ImageImageToImage,
    ImageReferenceConsistency,
    VideoTextToVideo,
    VideoImageToVideo,
    Model3d,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitKind {
    Calls,
    InputTokens,
    OutputTokens,
    TotalTokens,
    Tokens,
    Credits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitPolicy {
    pub limit_kind: LimitKind,
    pub max_value: u64,
    pub period_expr: String,
    pub group_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
    pub id: String,
    pub provider_code: String,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<AiCapability>,
    pub driver: String,
    pub api_path: String,
    pub enabled: bool,
    pub sort_no: i64,
    pub params: Value,
    pub remark: String,
    pub limits: Vec<LimitPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProvider {
    pub code: String,
    pub name: String,
    pub base_url: String,
    pub driver: String,
    #[serde(default = "default_auth_style")]
    pub auth_style: String,
    pub priority: i64,
    pub enabled: bool,
    #[serde(default)]
    pub remark: String,
    pub has_key: bool,
    pub key_mask: Option<String>,
    pub models: Vec<ProviderModel>,
}

fn default_auth_style() -> String {
    "bearer".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub agent_code: String,
    pub role: String,
    pub capability: AiCapability,
    pub output_contract: String,
    pub source_file: String,
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBudget {
    pub limit_kind: LimitKind,
    pub used: u64,
    pub limit: u64,
    pub period_expr: String,
    pub window_key: String,
    pub group_name: String,
    pub source: String,
    pub exhausted: bool,
    pub unlimited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakerState {
    pub failure_count: u32,
    pub last_reason: Option<String>,
    pub opened_at: Option<i64>,
    pub retry_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub provider_code: String,
    pub provider_name: String,
    pub provider_model_id: String,
    pub model_id: String,
    pub provider_enabled: bool,
    pub enabled: bool,
    pub has_key: bool,
    pub agents: Vec<String>,
    pub budgets: Vec<UsageBudget>,
    pub breaker: Option<BreakerState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProviderPresetModel {
    pub model_id: String,
    pub capabilities: Vec<AiCapability>,
    pub driver: String,
    pub api_path: String,
    pub limit_kind: LimitKind,
    pub default_period: String,
    pub params: Value,
    pub remark: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ProviderPreset {
    pub code: String,
    pub vendor: String,
    pub plan: String,
    pub label: String,
    pub base_url: String,
    pub driver: String,
    pub auth_style: String,
    pub key_prefix: Option<String>,
    pub models: Vec<ProviderPresetModel>,
}
