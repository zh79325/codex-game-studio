mod action;
mod agents;
mod audit;
mod execution;
mod orchestrator;
mod routing;
mod service;

pub use action::*;
pub use agents::*;
pub use audit::*;
pub use execution::*;
pub use orchestrator::*;
pub use routing::*;
pub use service::*;

use codex_game_domain::BackendStatus;
use codex_game_domain::Project;
use codex_game_store::StoreState;
use std::path::PathBuf;
use tokio::sync::Mutex;

pub const GAME_PROTOCOL_VERSION: u32 = 1;

/// Owns deterministic game workflow state.
#[derive(Debug)]
pub struct GameRuntime {
    store_state: StoreState,
    recovery_lock: Mutex<()>,
    service: GameService,
    orchestrator: TaskOrchestrator,
}

impl Default for GameRuntime {
    #[expect(
        clippy::expect_used,
        reason = "bundled agent definitions are static and covered by tests"
    )]
    fn default() -> Self {
        validate_bundled_agents().expect("bundled agent definitions must be valid");
        Self {
            store_state: StoreState::new(BackendStatus::Ready),
            recovery_lock: Mutex::new(()),
            service: GameService::default(),
            orchestrator: TaskOrchestrator::default(),
        }
    }
}

impl GameRuntime {
    pub fn new(studio_storage: PathBuf) -> Self {
        Self::new_with_routes(
            studio_storage,
            vec![RouteCandidate {
                account_id: "configured".to_string(),
                provider: String::new(),
                model: String::new(),
                capabilities: vec![Capability::TextReasoning, Capability::TextStructuredOutput],
                available: true,
            }],
        )
    }

    #[expect(
        clippy::expect_used,
        reason = "bundled agent definitions are static and covered by tests"
    )]
    pub fn new_with_routes(studio_storage: PathBuf, candidates: Vec<RouteCandidate>) -> Self {
        validate_bundled_agents().expect("bundled agent definitions must be valid");
        Self {
            store_state: StoreState::new(BackendStatus::Ready),
            recovery_lock: Mutex::new(()),
            service: GameService::new(studio_storage.clone()),
            orchestrator: TaskOrchestrator::new(candidates, Some(studio_storage)),
        }
    }

    pub fn status(&self) -> BackendStatus {
        self.store_state.status()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "serializes project recovery for the operation's full duration"
    )]
    pub async fn create_project(
        &self,
        project_id: String,
        name: String,
        root: String,
    ) -> Result<Project, GameServiceError> {
        let _recovery_guard = self.recovery_lock.lock().await;
        self.store_state.set_status(BackendStatus::Recovering);
        let result = self.service.create_project(project_id, name, root).await;
        self.store_state.set_status(BackendStatus::Ready);
        result
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "serializes project recovery for the operation's full duration"
    )]
    pub async fn open_project(
        &self,
        root: String,
        requested_read_only: bool,
    ) -> Result<Project, GameServiceError> {
        let _recovery_guard = self.recovery_lock.lock().await;
        self.store_state.set_status(BackendStatus::Recovering);
        let result = self.service.open_project(root, requested_read_only).await;
        let status = match &result {
            Ok(project)
                if self
                    .service
                    .is_project_read_only(project.id.as_str())
                    .unwrap_or(false) =>
            {
                BackendStatus::ReadOnly
            }
            _ => BackendStatus::Ready,
        };
        self.store_state.set_status(status);
        result
    }

    pub fn service(&self) -> &GameService {
        &self.service
    }

    pub fn orchestrator(&self) -> &TaskOrchestrator {
        &self.orchestrator
    }
}
