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
    code: Option<String>,
    root: String,
    state: String
});
game_dto!(GameConversation {
    id: String,
    project_id: String,
    target_kind: String,
    target_ref: Option<String>,
    title: String,
    director_agent_code: String,
    focus_agent_code: Option<String>,
    status: String,
    turn: u64,
    created_at: i64,
    updated_at: i64
});
game_dto!(GameMessage {
    id: String,
    turn: u64,
    role: String,
    content: String,
    agent_code: String,
    recipient_agent_code: Option<String>,
    status: String,
    token_count: u64,
    folded: bool,
    attachments: Vec<serde_json::Value>,
    action: Option<serde_json::Value>,
    created_at: i64
});
game_dto!(GameArtifactDraft {
    id: String,
    conversation_id: String,
    target_path: String,
    content: String,
    based_on_hash: Option<String>,
    status: String,
    created_at: i64
});
game_dto!(GameConversationMemory {
    id: String,
    conversation_id: String,
    scope: String,
    kind: String,
    content: String,
    created_at: i64
});
game_dto!(GameAgentHandoff {
    id: i64,
    conversation_id: String,
    turn: u64,
    from_agent_code: String,
    to_agent_code: String,
    source: String,
    reason: String,
    status: String,
    created_at: i64
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
game_dto!(GameTaskUpdatedNotification {
    conversation_id: String,
    task_id: String,
    status: String
});
game_dto!(GameAttemptUpdatedNotification { conversation_id: String, task_id: String, attempt_id: String, turn_id: Option<String>, status: String });
game_dto!(GameConversationTurnNotification {
    conversation_id: String,
    status: String
});
game_dto!(GameConversationDeltaNotification {
    conversation_id: String,
    turn_id: String,
    agent_code: String,
    delta: String
});
game_dto!(GameConversationActorNotification { conversation_id: String, turn_id: Option<String>, agent_code: String, status: String });
game_dto!(GameConversationFocusNotification {
    conversation_id: String,
    agent_code: String
});
game_dto!(GameConversationErrorNotification { conversation_id: String, turn_id: Option<String>, message: String });
game_dto!(GameAgentHandoffNotification {
    conversation_id: String,
    from_agent_code: String,
    to_agent_code: String,
    reason: String
});
game_dto!(GameCharacterUpdatedNotification {
    character: GameCharacter
});
game_dto!(GameGenerationUpdatedNotification {
    generation: GameGeneration
});
game_dto!(GameArtifactCommittedNotification {
    conversation_id: String,
    artifact_id: String,
    artifact_type: String
});
game_dto!(GameRecoveryStatusNotification {
    status: GameBackendStatus
});
game_dto!(GameSpeechTranscriptNotification {
    session_id: String,
    text: String,
    definite: bool
});
game_dto!(GameSpeechCompletedNotification {
    session_id: String,
    text: String,
    duration_ms: u64
});
game_dto!(GameSpeechErrorNotification {
    session_id: String,
    message: String
});

empty_params!(GameSpeechStartParams);
game_dto!(GameSpeechStartResponse {
    session_id: String,
    sample_rate: u32,
    channels: u16,
    chunk_ms: u32
});
game_dto!(GameSpeechChunkParams {
    session_id: String,
    audio_base64: String
});
empty_params!(GameSpeechChunkResponse);
game_dto!(GameSpeechFinishParams { session_id: String });
empty_params!(GameSpeechFinishResponse);
game_dto!(GameSpeechCancelParams { session_id: String });
empty_params!(GameSpeechCancelResponse);

game_dto!(GameProjectInspectParams { root: String });
game_dto!(GameProjectInspectResponse {
    root: String,
    occupied: bool,
    project_id: Option<String>,
    supported: bool
});
game_dto!(GameProjectCreateParams {
    name: Option<String>,
    root: String,
    overwrite: Option<bool>
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
game_dto!(GameProjectDeleteParams { project_id: String });
empty_params!(GameProjectDeleteResponse);
game_dto!(GameProjectCommitArtBibleParams {
    conversation_id: String,
    draft_id: String
});
game_dto!(GameProjectCommitArtBibleResponse {
    version: GameArtBibleVersion,
    markdown: String
});
game_dto!(GameProjectFinalizeParams {
    project_id: String,
    name: String,
    code: String
});
game_dto!(GameProjectFinalizeResponse {
    project: GameProject
});

game_dto!(GameConversationEnsureParams {
    project_id: String,
    target_kind: String,
    target_ref: Option<String>,
    title: Option<String>,
    director_agent_code: Option<String>
});
game_dto!(GameConversationEnsureResponse {
    conversation: GameConversation
});
game_dto!(GameConversationSubmitParams {
    conversation_id: String,
    content: String,
    recipient_agent_code: Option<String>
});
game_dto!(GameConversationSubmitResponse {
    conversation: GameConversation,
    messages: Vec<GameMessage>,
    drafts: Vec<GameArtifactDraft>,
    memories: Vec<GameConversationMemory>,
    handoffs: Vec<GameAgentHandoff>
});
game_dto!(GameConversationReadParams {
    conversation_id: String
});
game_dto!(GameConversationReadResponse {
    conversation: GameConversation,
    messages: Vec<GameMessage>,
    drafts: Vec<GameArtifactDraft>,
    memories: Vec<GameConversationMemory>,
    handoffs: Vec<GameAgentHandoff>
});
game_dto!(GameConversationInterruptParams {
    conversation_id: String
});
empty_params!(GameConversationInterruptResponse);
game_dto!(GameConversationCommitDraftsParams { conversation_id: String, draft_ids: Vec<String> });
empty_params!(GameConversationCommitDraftsResponse);

game_dto!(GameCharacter {
    id: String,
    project_id: String,
    name: String,
    group: Option<String>,
    dir_name: String,
    state: String,
    spec_path: Option<String>,
    render_path: Option<String>,
    view_paths: serde_json::Value,
    hard_constraints: Vec<serde_json::Value>,
    gate_spec_confirmed_at: Option<i64>,
    gate_render_confirmed_at: Option<i64>,
    gate_views_confirmed_at: Option<i64>,
    created_at: i64,
    updated_at: i64
});
game_dto!(GameGeneration {
    id: String,
    project_id: String,
    target_kind: String,
    target_ref: String,
    stage: String,
    variant: Option<String>,
    file_path: String,
    file_hash: Option<String>,
    is_final: bool,
    source: String,
    task_id: Option<String>,
    asset_spec: serde_json::Value,
    created_at: i64
});
game_dto!(GameCharacterCreateParams { project_id: String, name: String, group: Option<String>, overwrite: bool });
game_dto!(GameCharacterCreateResponse {
    character: GameCharacter
});
game_dto!(GameCharacterGroupCreateParams {
    project_id: String,
    name: String
});
game_dto!(GameCharacterGroupCreateResponse { group: String });
game_dto!(GameCharacterListParams { project_id: String });
game_dto!(GameCharacterListResponse {
    characters: Vec<GameCharacter>,
    groups: Vec<String>
});
game_dto!(GameCharacterReadParams {
    project_id: String,
    character_id: String
});
game_dto!(GameCharacterReadResponse { character: GameCharacter, generations: Vec<GameGeneration> });
game_dto!(GameCharacterConfirmSpecParams {
    project_id: String,
    character_id: String,
    draft_id: String
});
game_dto!(GameCharacterRejectSpecParams {
    project_id: String,
    character_id: String,
    reason: String
});
game_dto!(GameCharacterConfirmRenderParams {
    project_id: String,
    character_id: String,
    generation_id: String
});
game_dto!(GameCharacterRejectRenderParams {
    project_id: String,
    character_id: String,
    reason: String
});
game_dto!(GameCharacterConfirmViewsParams { project_id: String, character_id: String, generation_ids: Vec<String> });
game_dto!(GameCharacterRejectViewsParams {
    project_id: String,
    character_id: String,
    reason: String
});
game_dto!(GameCharacterResponse {
    character: GameCharacter
});
game_dto!(GameGenerationRegisterParams { project_id: String, character_id: String, stage: String, variant: Option<String>, file_path: String, source: String, asset_spec: serde_json::Value });
game_dto!(GameGenerationRegisterResponse {
    generation: GameGeneration
});
game_dto!(GameGenerationListParams { project_id: String, character_id: String, stage: Option<String> });
game_dto!(GameGenerationListResponse { generations: Vec<GameGeneration> });

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
    auth_style: String,
    priority: i64,
    enabled: bool,
    remark: String,
    has_key: bool,
    key_mask: Option<String>,
    models: Vec<GameAiModel>
});
game_dto!(GameAiAgent {
    agent_code: String,
    role: String,
    role_type: String,
    capability: String,
    required_model_capability: String,
    focusable: bool,
    aliases: Vec<String>,
    target_kinds: Vec<String>,
    stages: Vec<String>,
    max_turns: u32,
    conversational: bool,
    memory_scope: String,
    context_budget: u32,
    max_output_tokens: Option<u32>,
    output_contract: String,
    allow_tools: Vec<String>,
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
game_dto!(GameProviderPresetModel {
    model_id: String,
    capabilities: Vec<String>,
    driver: String,
    api_path: String,
    limit_kind: String,
    default_period: String,
    params_json: String,
    remark: String
});
game_dto!(GameProviderPreset {
    code: String,
    vendor: String,
    plan: String,
    label: String,
    base_url: String,
    driver: String,
    auth_style: String,
    key_prefix: Option<String>,
    models: Vec<GameProviderPresetModel>
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
game_dto!(GameAiAgentBinding {
    agent_code: String,
    model_ids: Vec<String>
});
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GameAiProviderCreateParams {
    pub provider: GameAiProvider,
    #[ts(optional = nullable)]
    pub api_key: Option<String>,
    pub agent_bindings: Vec<GameAiAgentBinding>,
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

empty_params!(GameProviderPresetListParams);
game_dto!(GameProviderPresetListResponse {
    presets: Vec<GameProviderPreset>,
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
