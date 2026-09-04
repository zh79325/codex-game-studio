use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_game_runtime::CodexExecutionPort;
use codex_game_runtime::ExecutionError;
use codex_game_runtime::StartThreadRequest;
use codex_game_runtime::StartTurnRequest;
use codex_game_runtime::StartedThread;
use codex_game_runtime::StartedTurn;
use codex_game_runtime::SteerTurnRequest;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Op;
use codex_protocol::turn_input::StartIfIdleSubmission;
use codex_protocol::turn_input::SteerSubmission;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::sync::Arc;

use crate::outgoing_message::ConnectionId;
use crate::request_processors::ListenerTaskContext;
use crate::request_processors::ensure_conversation_listener;

pub(crate) struct AppServerCodexExecutionPort {
    thread_manager: Arc<ThreadManager>,
    base_config: Arc<Config>,
    listener_context: ListenerTaskContext,
}

pub(crate) struct ConnectionCodexExecutionPort<'a> {
    execution: &'a AppServerCodexExecutionPort,
    connection_id: ConnectionId,
}

impl AppServerCodexExecutionPort {
    pub(crate) fn new(
        thread_manager: Arc<ThreadManager>,
        base_config: Arc<Config>,
        listener_context: ListenerTaskContext,
    ) -> Self {
        Self {
            thread_manager,
            base_config,
            listener_context,
        }
    }

    pub(crate) fn scoped(&self, connection_id: ConnectionId) -> ConnectionCodexExecutionPort<'_> {
        ConnectionCodexExecutionPort {
            execution: self,
            connection_id,
        }
    }

    async fn thread(
        &self,
        thread_id: &str,
    ) -> Result<Arc<codex_core::CodexThread>, ExecutionError> {
        let thread_id = ThreadId::from_string(thread_id)
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        self.thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))
    }
}

impl CodexExecutionPort for ConnectionCodexExecutionPort<'_> {
    async fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        let started = self.execution.start_thread(request).await?;
        let thread_id = ThreadId::from_string(&started.thread_id)
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        ensure_conversation_listener(
            self.execution.listener_context.clone(),
            thread_id,
            self.connection_id,
            false,
        )
        .await
        .map_err(|error| ExecutionError::Retryable(error.message))?;
        Ok(started)
    }

    async fn thread_available(&self, thread_id: &str) -> bool {
        self.execution.thread_available(thread_id).await
    }

    async fn start_turn(&self, request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        self.execution.start_turn(request).await
    }

    async fn steer_turn(&self, request: SteerTurnRequest) -> Result<(), ExecutionError> {
        self.execution.steer_turn(request).await
    }

    async fn interrupt_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> Result<(), ExecutionError> {
        self.execution.interrupt_turn(thread_id, turn_id).await
    }
}

impl CodexExecutionPort for AppServerCodexExecutionPort {
    async fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        let cwd = AbsolutePathBuf::from_absolute_path_checked(&request.cwd)
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        let mut config = self.base_config.as_ref().clone();
        if !request.route.model.is_empty() {
            config.model = Some(request.route.model.clone());
        }
        if !request.route.provider.is_empty() && config.model_provider_id != request.route.provider
        {
            config.model_provider = config
                .model_providers
                .get(&request.route.provider)
                .cloned()
                .ok_or_else(|| {
                    ExecutionError::CapabilityUnavailable(format!(
                        "model provider `{}` is not configured",
                        request.route.provider
                    ))
                })?;
            config.model_provider_id = request.route.provider;
        }
        config.cwd = cwd.clone();
        config.workspace_roots = vec![cwd];
        config.workspace_roots_explicit = true;
        let started = self
            .thread_manager
            .start_thread(codex_core::StartThreadOptions::new(config))
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))?;
        Ok(StartedThread {
            thread_id: started.thread_id.to_string(),
            session_id: started.session_configured.session_id.to_string(),
        })
    }

    async fn thread_available(&self, thread_id: &str) -> bool {
        self.thread(thread_id).await.is_ok()
    }

    async fn start_turn(&self, request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        let thread = self.thread(&request.thread_id).await?;
        let input = request
            .model_input()
            .map_err(|error| ExecutionError::InvalidRequest(error.to_string()))?;
        let mut turn = TurnInputRequest::user_input(vec![UserInput::Text {
            text: input,
            text_elements: Vec::new(),
        }]);
        if !request.context.output_schema.trim().is_empty() {
            let schema = serde_json::from_str(&request.context.output_schema).map_err(|error| {
                ExecutionError::InvalidRequest(format!("invalid output schema: {error}"))
            })?;
            turn.start.final_output_json_schema = Some(schema);
        }
        match thread
            .start_turn_if_idle(turn)
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))?
        {
            StartIfIdleSubmission::Started { turn_id } => Ok(StartedTurn { turn_id }),
            StartIfIdleSubmission::NotSubmitted { reason } => {
                Err(ExecutionError::Retryable(format!("{reason:?}")))
            }
        }
    }

    async fn steer_turn(&self, request: SteerTurnRequest) -> Result<(), ExecutionError> {
        let thread = self.thread(&request.thread_id).await?;
        let turn = TurnInputRequest::user_input(vec![UserInput::Text {
            text: request.message,
            text_elements: Vec::new(),
        }]);
        match thread
            .steer_turn(turn, request.expected_turn_id)
            .await
            .map_err(|error| ExecutionError::Retryable(error.to_string()))?
        {
            SteerSubmission::Steered { .. } => Ok(()),
            SteerSubmission::NotSubmitted { reason } => {
                Err(ExecutionError::InvalidRequest(format!("{reason:?}")))
            }
        }
    }

    async fn interrupt_turn(
        &self,
        thread_id: String,
        _turn_id: String,
    ) -> Result<(), ExecutionError> {
        self.thread(&thread_id)
            .await?
            .submit(Op::Interrupt)
            .await
            .map(|_| ())
            .map_err(|error| ExecutionError::Retryable(error.to_string()))
    }
}
