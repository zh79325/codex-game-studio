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
