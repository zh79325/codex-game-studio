use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_game_runtime::CodexExecutionPort;
use codex_game_runtime::ExecutionError;
use codex_game_runtime::StartThreadRequest;
use codex_game_runtime::StartTurnRequest;
use codex_game_runtime::StartedThread;
use codex_game_runtime::StartedTurn;
use codex_game_runtime::SteerTurnRequest;
use codex_game_runtime::TurnAuditContext;
use codex_game_runtime::append_turn_audit_response_headers;
use codex_game_runtime::append_turn_audit_stream_operation;
use codex_game_runtime::append_turn_audit_stream_response;
use codex_http_client::StreamResponseAudit;
use codex_http_client::StreamResponseAuditEvent;
use codex_http_client::register_stream_response_audit;
use codex_http_client::unregister_stream_response_audit;
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

struct GameTurnStreamAudit {
    context: TurnAuditContext,
}

impl StreamResponseAudit for GameTurnStreamAudit {
    fn record_response_headers(&self, headers: &[u8]) {
        if let Err(error) = append_turn_audit_response_headers(&self.context, headers) {
            tracing::warn!(
                attempt_id = %self.context.attempt_id,
                "failed to write game turn response headers to audit: {error}"
            );
        }
    }

    fn record_response_chunk(&self, chunk: &[u8]) {
        if let Err(error) = append_turn_audit_stream_response(&self.context, chunk) {
            tracing::warn!(
                attempt_id = %self.context.attempt_id,
                "failed to write game turn stream response to audit: {error}"
            );
        }
    }

    fn record_stream_event(&self, event: StreamResponseAuditEvent) {
        let (stage, operation, detail) = match event {
            StreamResponseAuditEvent::StreamOpened { status } => (
                "http_transport",
                "stream_opened",
                format!("HTTP status: {status}"),
            ),
            StreamResponseAuditEvent::ResponseItemNormalized {
                event_type,
                reason,
                item_type,
                item_id,
                role,
            } => (
                "sse_parser",
                "response_item_normalized",
                format!(
                    "event_type={event_type}; reason={reason}; item_type={item_type:?}; item_id={item_id:?}; role={role:?}"
                ),
            ),
            StreamResponseAuditEvent::ResponseItemRejected {
                event_type,
                error,
                item_type,
                item_id,
                role,
            } => (
                "sse_parser",
                "response_item_rejected",
                format!(
                    "event_type={event_type}; error={error}; item_type={item_type:?}; item_id={item_id:?}; role={role:?}"
                ),
            ),
            StreamResponseAuditEvent::SseEventRejected {
                error,
                payload_bytes,
            } => (
                "sse_parser",
                "sse_event_rejected",
                format!("error={error}; payload_bytes={payload_bytes}"),
            ),
            StreamResponseAuditEvent::ProviderCompleted { response_id } => (
                "sse_parser",
                "provider_completed",
                format!("response_id={response_id}"),
            ),
            StreamResponseAuditEvent::StreamTerminated { stage, reason } => {
                (stage, "stream_terminated", reason)
            }
            StreamResponseAuditEvent::EventConsumerDropped { stage } => (
                stage,
                "event_consumer_dropped",
                "downstream event consumer closed; provider stream reading stopped".to_string(),
            ),
            StreamResponseAuditEvent::DeltaWithoutActiveItem {
                event_type,
                delta_bytes,
                action,
            } => (
                "turn_state_machine",
                "delta_without_active_item",
                format!("event_type={event_type}; delta_bytes={delta_bytes}; action={action}"),
            ),
        };
        if let Err(error) =
            append_turn_audit_stream_operation(&self.context, stage, operation, &detail)
        {
            tracing::warn!(
                attempt_id = %self.context.attempt_id,
                "failed to write game turn stream operation to audit: {error}"
            );
        }
    }
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
        let audit_registered = if let Some(context) = request.audit_context {
            register_stream_response_audit(
                request.thread_id.clone(),
                Arc::new(GameTurnStreamAudit { context }),
            );
            true
        } else {
            false
        };
        let submission = match thread.start_turn_if_idle(turn).await {
            Ok(submission) => submission,
            Err(error) => {
                if audit_registered {
                    unregister_stream_response_audit(&request.thread_id);
                }
                return Err(ExecutionError::Retryable(error.to_string()));
            }
        };
        match submission {
            StartIfIdleSubmission::Started { turn_id } => Ok(StartedTurn { turn_id }),
            StartIfIdleSubmission::NotSubmitted { reason } => {
                if audit_registered {
                    unregister_stream_response_audit(&request.thread_id);
                }
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
