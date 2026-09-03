use codex_game_domain::BackendStatus;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GamePingParams {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GamePingResponse {
    pub protocol_version: u32,
    pub backend_version: String,
    pub status: GameBackendStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum GameBackendStatus {
    Starting,
    Recovering,
    Ready,
    ReadOnly,
}

impl From<BackendStatus> for GameBackendStatus {
    fn from(status: BackendStatus) -> Self {
        match status {
            BackendStatus::Starting => Self::Starting,
            BackendStatus::Recovering => Self::Recovering,
            BackendStatus::Ready => Self::Ready,
            BackendStatus::ReadOnly => Self::ReadOnly,
        }
    }
}

macro_rules! game_dto {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
        #[serde(rename_all = "camelCase")]
        #[ts(export_to = "v2/")]
        pub struct $name { $(pub $field: $ty),* }
    };
}

macro_rules! empty_params {
    ($name:ident) => {
        #[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
        #[serde(rename_all = "camelCase")]
        #[ts(export_to = "v2/")]
        pub struct $name {}
    };
}

game_dto!(GameProject {
    id: String,
    name: String,
    root: String,
    state: String
});
game_dto!(GameConversation { id: String, project_id: String, target_id: Option<String>, created_at: i64 });
game_dto!(GameMessage {
    id: String,
    role: String,
    content: String,
    created_at: i64
});
game_dto!(GameFocusWorkflow {
    id: String,
    project_id: String,
    conversation_id: String,
    state: String,
    input_version: u64,
    workflow_version: u64
});
game_dto!(GameArtBibleVersion {
    id: String,
    project_id: String,
    version: u64,
    content_hash: String,
    created_at: i64
});
game_dto!(GameTask {
    id: String,
    stage: String,
    agent_code: String,
    status: String
});
game_dto!(GameReviewReport {
    agent_code: String,
    findings: Vec<String>,
    risks: Vec<String>,
    recommendations: Vec<String>
});
game_dto!(GameConflict {
    key: String,
    description: String,
    options: Vec<String>,
    high_impact: bool
});
game_dto!(GameWorkflowUpdatedNotification {
    workflow: GameFocusWorkflow
});
game_dto!(GameTaskUpdatedNotification {
    conversation_id: String,
    task_id: String,
    status: String
});
game_dto!(GameAttemptUpdatedNotification { conversation_id: String, task_id: String, attempt_id: String, turn_id: Option<String>, status: String });
game_dto!(GameArtifactCommittedNotification {
    conversation_id: String,
    artifact_id: String,
    artifact_type: String
});
game_dto!(GameDesignConfirmationRequiredNotification {
    conversation_id: String,
    workflow_id: String,
    conflict_count: u64
});
game_dto!(GameRecoveryStatusNotification {
    status: GameBackendStatus
});

game_dto!(GameProjectCreateParams {
    name: String,
    root: String
});
game_dto!(GameProjectCreateResponse {
    project: GameProject
});
game_dto!(GameProjectOpenParams { root: String, read_only: Option<bool> });
game_dto!(GameProjectOpenResponse {
    project: GameProject
});
game_dto!(GameProjectReadParams { project_id: String });
game_dto!(GameProjectReadResponse {
    project: GameProject
});
empty_params!(GameProjectListParams);
game_dto!(GameProjectListResponse { projects: Vec<GameProject> });
game_dto!(GameProjectImportParams {
    source: String,
    destination: String
});
game_dto!(GameProjectImportResponse { project: GameProject, warnings: Vec<String> });

game_dto!(GameConversationEnsureParams { project_id: String, target_id: Option<String> });
game_dto!(GameConversationEnsureResponse {
    conversation: GameConversation
});
game_dto!(GameConversationSubmitParams {
    conversation_id: String,
    content: String
});
game_dto!(GameConversationSubmitResponse {
    message: GameMessage
});
game_dto!(GameConversationReadParams {
    conversation_id: String
});
game_dto!(GameConversationReadResponse { conversation: GameConversation, messages: Vec<GameMessage> });

game_dto!(GameFocusStartParams {
    conversation_id: String
});
game_dto!(GameFocusStartResponse {
    workflow: GameFocusWorkflow
});
game_dto!(GameFocusReadParams {
    conversation_id: String
});
game_dto!(GameFocusReadResponse {
    workflow: GameFocusWorkflow,
    reviews: Vec<GameReviewReport>,
    conflicts: Vec<GameConflict>,
    art_bible_draft: Option<String>,
    decisions: Vec<GameUserDecision>
});

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum GameFocusAction {
    SubmitClarification,
    AcceptBrief,
    CompleteReviews,
    CompleteMerge,
    RecordConflictDecision,
    ConfirmArtBible,
    VersionArtBible,
}

game_dto!(GameUserDecision {
    conflict_key: String,
    selected_option: String,
    note: Option<String>
});
game_dto!(GameFocusDecideParams { conversation_id: String, expected_input_version: u64, action: GameFocusAction, art_bible_markdown: Option<String>, user_decision: Option<GameUserDecision> });
game_dto!(GameFocusDecideResponse { workflow: GameFocusWorkflow, art_bible: Option<GameArtBibleVersion> });
game_dto!(GameFocusRetryParams {
    conversation_id: String,
    expected_input_version: u64
});
game_dto!(GameFocusRetryResponse {
    workflow: GameFocusWorkflow
});
game_dto!(GameFocusCancelParams {
    conversation_id: String,
    expected_input_version: u64
});
game_dto!(GameFocusCancelResponse {
    workflow: GameFocusWorkflow
});
game_dto!(GameTaskListParams {
    conversation_id: String
});
game_dto!(GameTaskListResponse { tasks: Vec<GameTask> });
game_dto!(GameArtBibleListParams { project_id: String });
game_dto!(GameArtBibleListResponse { versions: Vec<GameArtBibleVersion> });
game_dto!(GameArtBibleReadParams {
    project_id: String,
    version: u64
});
game_dto!(GameArtBibleReadResponse {
    version: GameArtBibleVersion,
    markdown: String
});

game_dto!(GameAiLimit {
    limit_kind: String,
    max_value: u64,
    period_expr: String,
    group_name: String
});
game_dto!(GameAiModel {
    id: String,
    provider_code: String,
    model_id: String,
    display_name: String,
    capabilities: Vec<String>,
    driver: String,
    api_path: String,
    enabled: bool,
    sort_no: i64,
    params_json: String,
    remark: String,
    limits: Vec<GameAiLimit>
});
game_dto!(GameAiProvider {
    code: String,
    name: String,
    base_url: String,
    driver: String,
    priority: i64,
    enabled: bool,
    has_key: bool,
    key_mask: Option<String>,
    models: Vec<GameAiModel>
});
game_dto!(GameAiAgent {
    agent_code: String,
    role: String,
    capability: String,
    output_contract: String,
    source_file: String,
    model_ids: Vec<String>
});
game_dto!(GameAiUsageBudget {
    limit_kind: String,
    used: u64,
    limit: u64,
    period_expr: String,
    window_key: String,
    group_name: String,
    source: String,
    exhausted: bool,
    unlimited: bool
});
game_dto!(GameAiBreaker {
    failure_count: u32,
    last_reason: Option<String>,
    opened_at: Option<i64>,
    retry_at: Option<i64>
});
game_dto!(GameAiModelUsage {
    provider_code: String,
    provider_name: String,
    provider_model_id: String,
    model_id: String,
    provider_enabled: bool,
    enabled: bool,
    has_key: bool,
    agents: Vec<String>,
    budgets: Vec<GameAiUsageBudget>,
    breaker: Option<GameAiBreaker>
});
game_dto!(GameModelRecommendation {
    provider_code: String,
    provider_name: String,
    driver: String,
    default_base_url: String,
    model_id: String,
    display_name: String,
    capabilities: Vec<String>,
    recommended: bool,
    default_limits: Vec<GameAiLimit>
});

empty_params!(GameAiProviderListParams);
game_dto!(GameAiProviderListResponse { providers: Vec<GameAiProvider> });

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GameAiProviderWriteParams {
    pub provider: GameAiProvider,
    #[ts(optional = nullable)]
    pub api_key: Option<String>,
}
game_dto!(GameAiProviderCreateResponse {
    provider: GameAiProvider
});
game_dto!(GameAiProviderUpdateResponse {
    provider: GameAiProvider
});
game_dto!(GameAiProviderDeleteParams { code: String });
empty_params!(GameAiProviderDeleteResponse);

game_dto!(GameAiModelWriteParams { model: GameAiModel });
game_dto!(GameAiModelCreateResponse { model: GameAiModel });
game_dto!(GameAiModelUpdateResponse { model: GameAiModel });
game_dto!(GameAiModelDeleteParams { model_id: String });
empty_params!(GameAiModelDeleteResponse);

empty_params!(GameAiAgentListParams);
game_dto!(GameAiAgentListResponse { agents: Vec<GameAiAgent> });
game_dto!(GameAiAgentBindingWriteParams {
    agent_code: String,
    model_ids: Vec<String>
});
empty_params!(GameAiAgentBindingWriteResponse);

empty_params!(GameAiUsageReadParams);
game_dto!(GameAiUsageReadResponse { items: Vec<GameAiModelUsage> });
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GameAiUsageResetParams {
    pub model_id: String,
    #[ts(optional = nullable)]
    pub limit_kind: Option<String>,
}
game_dto!(GameAiUsageResetResponse { cleared: u64 });
game_dto!(GameAiBreakerClearParams { model_id: String });
empty_params!(GameAiBreakerClearResponse);

empty_params!(GameModelRecommendationListParams);
game_dto!(GameModelRecommendationListResponse {
    recommendations: Vec<GameModelRecommendation>,
    path: String
});
empty_params!(GameAiConfigExportParams);
game_dto!(GameAiConfigExportResponse { json: String });
game_dto!(GameAiConfigImportParams {
    json: String,
    dry_run: bool
});
game_dto!(GameAiConfigImportResponse {
    provider_count: u64,
    model_count: u64,
    applied: bool
});
