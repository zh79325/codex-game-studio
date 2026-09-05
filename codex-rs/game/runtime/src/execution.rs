use crate::RouteDecision;
use crate::TurnAuditContext;
use codex_game_domain::ContextPackage;
use std::future::Future;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartThreadRequest {
    pub cwd: String,
    pub agent_code: String,
    pub route: RouteDecision,
    pub image_generation_route: Option<RouteDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedThread {
    pub thread_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartTurnRequest {
    pub thread_id: String,
    pub attempt_id: String,
    pub agent_definition: String,
    pub prompt: String,
    pub context: ContextPackage,
    pub audit_context: Option<TurnAuditContext>,
}

impl StartTurnRequest {
    pub fn model_input(&self) -> Result<String, serde_json::Error> {
        let context = serde_json::to_string(&self.context)?;
        Ok(format!(
            "<game_agent_definition>\n{}\n</game_agent_definition>\n\n{}\n\n<game_context attempt_id=\"{}\">\n{}\n</game_context>",
            self.agent_definition, self.prompt, self.attempt_id, context
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedTurn {
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteerTurnRequest {
    pub thread_id: String,
    pub expected_turn_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExecutionError {
    #[error("execution request is invalid: {0}")]
    InvalidRequest(String),
    #[error("execution can be retried: {0}")]
    Retryable(String),
    #[error("execution context is too large: {0}")]
    ContextTooLarge(String),
    #[error("execution capability is unavailable: {0}")]
    CapabilityUnavailable(String),
    #[error("execution failed: {0}")]
    Fatal(String),
}

/// Boundary through which deterministic game workflows invoke Codex sessions.
pub trait CodexExecutionPort: Send + Sync {
    fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> impl Future<Output = Result<StartedThread, ExecutionError>> + Send;

    fn thread_available(&self, thread_id: &str) -> impl Future<Output = bool> + Send;

    fn start_turn(
        &self,
        request: StartTurnRequest,
    ) -> impl Future<Output = Result<StartedTurn, ExecutionError>> + Send;

    fn steer_turn(
        &self,
        request: SteerTurnRequest,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;

    fn interrupt_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;
}
