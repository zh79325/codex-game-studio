use crate::Capability;
use crate::CodexExecutionPort;
use crate::ExecutionError;
use crate::QuotaRequirement;
use crate::RouteCandidate;
use crate::RouteDecision;
use crate::RouteError;
use crate::RouteEvent;
use crate::RouteFailureKind;
use crate::RouteOutcome;
use crate::RouteSelector;
use crate::StartThreadRequest;
use crate::StartTurnRequest;
use crate::SteerTurnRequest;
use crate::TurnAuditContext;
use crate::append_turn_audit_start_error;
use crate::bundled_agent_definition;
use crate::write_turn_audit_request;
use codex_game_domain::AiCapability;
use codex_game_domain::ContextPackage;
use codex_game_domain::ConversationCodexThread;
use codex_game_domain::ConversationCodexThreadId;
use codex_game_domain::ConversationId;
use codex_game_domain::Interaction;
use codex_game_domain::InteractionId;
use codex_game_domain::Task;
use codex_game_domain::TaskAttempt;
use codex_game_domain::TaskAttemptId;
use codex_game_domain::TaskAttemptStatus;
use codex_game_domain::TaskId;
use codex_game_domain::TaskStatus;
use codex_game_domain::ThreadBindingStatus;
use codex_game_store::ProjectStore;
use codex_game_store::ProviderAccountMetadata;
use codex_game_store::StoreError;
use codex_game_store::StoredRouteBinding;
use codex_game_store::StudioUsageEntry;
use codex_game_store::list_ai_providers;
use codex_game_store::load_ai_route_models;
use codex_game_store::load_route_binding;
use codex_game_store::open_studio_store;
use codex_game_store::record_ai_route_failure;
use codex_game_store::record_ai_route_success;
use codex_game_store::record_route_selection;
use codex_game_store::record_usage;
use codex_game_store::reserve_ai_usage;
use codex_game_store::upsert_provider_accounts;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ExecuteTaskRequest {
    pub project_root: String,
    pub conversation_id: String,
    pub conversation_turn: u64,
    pub target_id: String,
    pub audit_target: String,
    pub audit_target_dir: PathBuf,
    pub stage: String,
    pub agent_code: String,
    pub idempotency_key: String,
    pub prompt: String,
    pub context: ContextPackage,
    pub capability: Capability,
    pub internal_executor_code: Option<String>,
    pub internal_executor_capability: Option<Capability>,
}

#[derive(Debug, Clone)]
pub struct TaskExecution {
    pub task: Task,
    pub attempt: TaskAttempt,
    pub binding: ConversationCodexThread,
}

pub const MAX_ACTION_CONTRACT_RETRIES: u32 = 3;
const OUTPUT_LENGTH_RETRY_TOKEN_LIMITS: [u64; 4] = [32_000, 64_000, 128_000, 200_000];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskAttemptRetry {
    pub attempt_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub agent_code: String,
    pub turn_id: String,
}

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error("context package exceeds the configured bounds")]
    ContextTooLarge,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error(transparent)]
    Route(#[from] RouteError),
}

/// Serializes work per Conversation/Agent binding and coordinates durable attempts with Codex turns.
#[derive(Debug)]
pub struct TaskOrchestrator {
    binding_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    routes: RouteSelector,
    studio_storage: Option<PathBuf>,
}

impl Default for TaskOrchestrator {
    fn default() -> Self {
        Self::new(
            vec![RouteCandidate {
                account_id: "configured".to_string(),
                provider: String::new(),
                model: String::new(),
                capabilities: vec![Capability::TextReasoning, Capability::TextStructuredOutput],
                available: true,
            }],
            None,
        )
    }
}

impl TaskOrchestrator {
    pub fn new(candidates: Vec<RouteCandidate>, studio_storage: Option<PathBuf>) -> Self {
        Self {
            binding_locks: Mutex::new(HashMap::new()),
            routes: RouteSelector::new(candidates),
            studio_storage,
        }
    }
    pub fn set_route_quota(
        &self,
        account_id: &str,
        metric: &str,
        amount: u64,
    ) -> Result<(), RouteError> {
        self.routes.set_quota(account_id, metric, amount)
    }

    pub async fn report_route_outcome(
        &self,
        conversation_id: &str,
        outcome: RouteOutcome,
    ) -> Result<(), OrchestrationError> {
        let scope = format!("conversation:{conversation_id}");
        if let Some(decision) = self.routes.current_binding(&scope)? {
            self.report_route(&decision, outcome, "turn execution failed")
                .await?;
        }
        Ok(())
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "serializes binding creation and turn start per conversation+agent"
    )]
    pub async fn execute<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        request: ExecuteTaskRequest,
    ) -> Result<TaskExecution, OrchestrationError> {
        if !request.context.is_bounded() {
            return Err(OrchestrationError::ContextTooLarge);
        }
        self.routes.reset_transient_failures()?;

        let lock = self
            .binding_lock(&request.conversation_id, &request.agent_code)
            .await;
        let _guard = lock.lock().await;
        let (binding, route) = self.ensure_binding(execution, store, &request).await?;
        let timestamp = now();
        let interaction = Interaction {
            id: InteractionId::new(Uuid::now_v7().to_string()),
            conversation_id: ConversationId::new(request.conversation_id.clone()),
            idempotency_key: request.idempotency_key.clone(),
            created_at: timestamp,
        };
        let mut task = Task {
            id: TaskId::new(Uuid::now_v7().to_string()),
            interaction_id: interaction.id.clone(),
            target_id: request.target_id.clone(),
            stage: request.stage.clone(),
            agent_code: request.agent_code.clone(),
            input_artifact_ids: Vec::new(),
            input_version: request.context.context_version,
            contract_version: request.context.contract_version,
            status: TaskStatus::Pending,
        };
        let mut attempt = TaskAttempt {
            id: TaskAttemptId::new(Uuid::now_v7().to_string()),
            task_id: task.id.clone(),
            attempt_no: 1,
            conversation_codex_thread_id: binding.id.clone(),
            codex_turn_id: None,
            output_artifact_id: None,
            status: TaskAttemptStatus::Pending,
        };
        let (binding, route) = self
            .reserve_usage_with_failover(
                execution,
                store,
                &request,
                attempt.id.as_str(),
                binding,
                route,
            )
            .await?;
        attempt.conversation_codex_thread_id = binding.id.clone();
        if let (Some(capability), Some(route)) = (
            request.internal_executor_capability,
            self.internal_executor_route(&request).await?,
        ) {
            self.reserve_request_usage(
                &route,
                capability,
                &format!("{}:internal-image", request.idempotency_key),
            )
            .await?;
        }
        store
            .create_task_attempt(
                &interaction,
                &task,
                &attempt,
                &request.prompt,
                &request.context,
            )
            .await?;

        let audit_context = TurnAuditContext {
            project_root: PathBuf::from(&request.project_root),
            target_dir: request.audit_target_dir,
            conversation_id: request.conversation_id,
            turn: request.conversation_turn,
            target: request.audit_target,
            agent_code: request.agent_code.clone(),
            attempt_id: attempt.id.as_str().to_string(),
        };
        let start_request = StartTurnRequest {
            thread_id: binding.codex_thread_id.clone(),
            attempt_id: attempt.id.as_str().to_string(),
            agent_definition: bundled_agent_definition(&request.agent_code)
                .ok_or_else(|| {
                    ExecutionError::InvalidRequest(format!(
                        "unknown game agent: {}",
                        request.agent_code
                    ))
                })?
                .to_string(),
            prompt: request.prompt,
            context: request.context,
            max_output_tokens: None,
            audit_context: Some(audit_context.clone()),
        };
        if let Err(error) = write_turn_audit_request(&audit_context, &route, &start_request) {
            tracing::warn!(
                path = %audit_context.target_dir.display(),
                "failed to write game turn audit request: {error}"
            );
        }
        let turn = match execution.start_turn(start_request).await {
            Ok(turn) => turn,
            Err(error) => {
                if let Err(audit_error) =
                    append_turn_audit_start_error(&audit_context, &error.to_string())
                {
                    tracing::warn!(
                        path = %audit_context.target_dir.display(),
                        "failed to write game turn audit error: {audit_error}"
                    );
                }
                if let Some(kind) = route_failure_kind(&error) {
                    self.report_route(&route, RouteOutcome::Failed(kind), &error.to_string())
                        .await?;
                }
                store
                    .mark_attempt_status(attempt.id.as_str(), TaskAttemptStatus::Failed)
                    .await?;
                return Err(error.into());
            }
        };
        store
            .bind_turn_to_attempt(attempt.id.as_str(), &turn.turn_id, now())
            .await?;
        task.status = TaskStatus::Running;
        attempt.status = TaskAttemptStatus::Running;
        attempt.codex_turn_id = Some(turn.turn_id);
        Ok(TaskExecution {
            task,
            attempt,
            binding,
        })
    }

    pub async fn retry_action_contract<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        context: &codex_game_store::TurnAttemptContext,
        audit_context: TurnAuditContext,
        validation_error: &str,
    ) -> Result<TaskAttemptRetry, OrchestrationError> {
        self.retry_task_attempt(
            execution,
            store,
            context,
            audit_context,
            format!(
                "系统检测到上一轮输出未通过 Action 契约校验：{validation_error}\n这是第 {} 次自动重试。请根据校验错误修正输出，重新给出完整回复，并严格遵守 Agent 定义中的 Action 协议。不要解释本次重试。",
                context.attempt_no
            ),
            None,
            "Action 契约",
        )
        .await
    }

    pub async fn retry_output_length<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        context: &codex_game_store::TurnAttemptContext,
        audit_context: TurnAuditContext,
    ) -> Result<Option<TaskAttemptRetry>, OrchestrationError> {
        let Some(max_output_tokens) = OUTPUT_LENGTH_RETRY_TOKEN_LIMITS
            .get(context.attempt_no.saturating_sub(1) as usize)
            .copied()
        else {
            return Ok(None);
        };
        self.retry_task_attempt(
            execution,
            store,
            context,
            audit_context,
            format!(
                "系统检测到上一轮输出因达到长度上限而中断。这是第 {} 次自动重试，本轮输出上限已提高到 {max_output_tokens} tokens。请重新给出完整回复，保持内容精炼，并严格遵守 Agent 定义中的 Action 协议。不要解释本次重试。",
                context.attempt_no
            ),
            Some(max_output_tokens),
            "输出长度",
        )
        .await
        .map(Some)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "serializes retries per conversation+agent"
    )]
    #[allow(clippy::too_many_arguments)]
    async fn retry_task_attempt<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        context: &codex_game_store::TurnAttemptContext,
        mut audit_context: TurnAuditContext,
        prompt: String,
        max_output_tokens: Option<u64>,
        retry_kind: &str,
    ) -> Result<TaskAttemptRetry, OrchestrationError> {
        let lock = self
            .binding_lock(&context.conversation_id, &context.agent_code)
            .await;
        let _guard = lock.lock().await;
        let retry_attempt = TaskAttempt {
            id: TaskAttemptId::new(Uuid::now_v7().to_string()),
            task_id: TaskId::new(context.task_id.clone()),
            attempt_no: context.attempt_no + 1,
            conversation_codex_thread_id: ConversationCodexThreadId::new(
                context.conversation_codex_thread_id.clone(),
            ),
            codex_turn_id: None,
            output_artifact_id: None,
            status: TaskAttemptStatus::Pending,
        };
        audit_context.attempt_id = retry_attempt.id.as_str().to_string();
        let request = StartTurnRequest {
            thread_id: context.codex_thread_id.clone(),
            attempt_id: retry_attempt.id.as_str().to_string(),
            agent_definition: bundled_agent_definition(&context.agent_code)
                .ok_or_else(|| {
                    ExecutionError::InvalidRequest(format!(
                        "unknown game agent: {}",
                        context.agent_code
                    ))
                })?
                .to_string(),
            prompt,
            context: context.context.clone(),
            max_output_tokens,
            audit_context: Some(audit_context.clone()),
        };
        store
            .begin_task_attempt_retry(&context.attempt_id, &retry_attempt)
            .await?;
        let turn = match execution.start_turn(request).await {
            Ok(turn) => turn,
            Err(error) => {
                if let Err(audit_error) =
                    append_turn_audit_start_error(&audit_context, &error.to_string())
                {
                    tracing::warn!(
                        path = %audit_context.target_dir.display(),
                        retry_kind,
                        "failed to write game retry audit error: {audit_error}"
                    );
                }
                store
                    .rollback_task_attempt_retry(&context.attempt_id, retry_attempt.id.as_str())
                    .await?;
                return Err(error.into());
            }
        };
        store
            .bind_turn_to_attempt(retry_attempt.id.as_str(), &turn.turn_id, now())
            .await?;
        Ok(TaskAttemptRetry {
            attempt_id: retry_attempt.id.as_str().to_string(),
            task_id: context.task_id.clone(),
            conversation_id: context.conversation_id.clone(),
            agent_code: context.agent_code.clone(),
            turn_id: turn.turn_id,
        })
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "serializes steering against the active binding"
    )]
    pub async fn steer<E: CodexExecutionPort>(
        &self,
        execution: &E,
        conversation_id: &str,
        agent_code: &str,
        request: SteerTurnRequest,
    ) -> Result<(), OrchestrationError> {
        let lock = self.binding_lock(conversation_id, agent_code).await;
        let _guard = lock.lock().await;
        execution.steer_turn(request).await?;
        Ok(())
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "serializes interruption against the active binding"
    )]
    pub async fn interrupt<E: CodexExecutionPort>(
        &self,
        execution: &E,
        conversation_id: &str,
        agent_code: &str,
        thread_id: String,
        turn_id: String,
    ) -> Result<(), OrchestrationError> {
        let lock = self.binding_lock(conversation_id, agent_code).await;
        let _guard = lock.lock().await;
        execution.interrupt_turn(thread_id, turn_id).await?;
        Ok(())
    }

    async fn ensure_binding<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        request: &ExecuteTaskRequest,
    ) -> Result<(ConversationCodexThread, RouteDecision), OrchestrationError> {
        let previous = store
            .active_thread(&request.conversation_id, &request.agent_code)
            .await?;
        let scope = format!("conversation:{}", request.conversation_id);
        let (route, event) = self
            .select_route(request.capability, &scope, &request.agent_code)
            .await?;
        if request.internal_executor_code.is_none()
            && matches!(event, RouteEvent::Selected { .. })
            && let Some(binding) = &previous
            && execution.thread_available(&binding.codex_thread_id).await
        {
            return Ok((binding.clone(), route));
        }
        let image_generation_route = self.internal_executor_route(request).await?;
        match execution
            .start_thread(StartThreadRequest {
                cwd: request.project_root.clone(),
                agent_code: request.agent_code.clone(),
                route: route.clone(),
                image_generation_route,
            })
            .await
        {
            Ok(started) => {
                self.persist_binding(
                    store,
                    request,
                    previous,
                    started,
                    route,
                    "thread-unavailable",
                )
                .await
            }
            Err(error) => {
                let Some(kind) = route_failure_kind(&error) else {
                    return Err(error.into());
                };
                self.report_route(&route, RouteOutcome::Failed(kind), &error.to_string())
                    .await?;
                let next_route = self
                    .select_route(request.capability, &scope, &request.agent_code)
                    .await?
                    .0;
                self.start_replacement_thread(execution, store, request, previous, next_route)
                    .await
            }
        }
    }

    async fn start_replacement_thread<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        request: &ExecuteTaskRequest,
        previous: Option<ConversationCodexThread>,
        mut route: RouteDecision,
    ) -> Result<(ConversationCodexThread, RouteDecision), OrchestrationError> {
        loop {
            match execution
                .start_thread(StartThreadRequest {
                    cwd: request.project_root.clone(),
                    agent_code: request.agent_code.clone(),
                    route: route.clone(),
                    image_generation_route: self.internal_executor_route(request).await?,
                })
                .await
            {
                Ok(started) => {
                    return self
                        .persist_binding(store, request, previous, started, route, "route-switched")
                        .await;
                }
                Err(error) => {
                    let Some(kind) = route_failure_kind(&error) else {
                        return Err(error.into());
                    };
                    self.report_route(&route, RouteOutcome::Failed(kind), &error.to_string())
                        .await?;
                    route = self
                        .select_route(
                            request.capability,
                            &format!("conversation:{}", request.conversation_id),
                            &request.agent_code,
                        )
                        .await?
                        .0;
                }
            }
        }
    }

    async fn persist_binding(
        &self,
        store: &ProjectStore,
        request: &ExecuteTaskRequest,
        previous: Option<ConversationCodexThread>,
        started: crate::StartedThread,
        route: RouteDecision,
        replacement_reason: &str,
    ) -> Result<(ConversationCodexThread, RouteDecision), OrchestrationError> {
        let timestamp = now();
        let expected_binding_version = previous.as_ref().map(|binding| binding.binding_version);
        let replacement_reason = previous.as_ref().map(|_| replacement_reason.to_string());
        let binding = ConversationCodexThread {
            id: ConversationCodexThreadId::new(Uuid::now_v7().to_string()),
            conversation_id: ConversationId::new(request.conversation_id.clone()),
            agent_code: request.agent_code.clone(),
            codex_thread_id: started.thread_id,
            codex_session_id: started.session_id,
            status: ThreadBindingStatus::Active,
            binding_version: expected_binding_version.map_or(1, |version| version + 1),
            context_version: request.context.context_version,
            agent_definition_version: request.context.agent_definition_version.clone(),
            forked_from_id: previous.as_ref().map(|binding| binding.id.clone()),
            replacement_reason: replacement_reason.clone(),
            created_at: timestamp,
            last_used_at: timestamp,
        };
        store
            .replace_active_thread(
                &binding,
                expected_binding_version,
                replacement_reason.as_deref(),
            )
            .await?;
        Ok((binding, route))
    }

    async fn internal_executor_route(
        &self,
        request: &ExecuteTaskRequest,
    ) -> Result<Option<RouteDecision>, OrchestrationError> {
        let (Some(agent_code), Some(capability)) = (
            request.internal_executor_code.as_deref(),
            request.internal_executor_capability,
        ) else {
            if request.internal_executor_code.is_some()
                || request.internal_executor_capability.is_some()
            {
                return Err(ExecutionError::InvalidRequest(
                    "internal executor code and capability must be configured together".to_string(),
                )
                .into());
            }
            return Ok(None);
        };
        let candidates = if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            load_ai_route_models(&studio, agent_code, now())
                .await?
                .into_iter()
                .map(|candidate| RouteCandidate {
                    account_id: candidate.id,
                    provider: candidate.provider,
                    model: candidate.model,
                    capabilities: candidate
                        .capabilities
                        .into_iter()
                        .map(runtime_capability)
                        .collect(),
                    available: candidate.available,
                })
                .collect()
        } else {
            self.routes.candidates()?
        };
        let selector = RouteSelector::new(candidates);
        let scope = format!(
            "conversation:{}:internal:{}",
            request.conversation_id, agent_code
        );
        Ok(Some(selector.select(capability, &scope)?.0))
    }

    async fn select_route(
        &self,
        capability: Capability,
        scope: &str,
        agent_code: &str,
    ) -> Result<(RouteDecision, RouteEvent), OrchestrationError> {
        if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            let candidates = load_ai_route_models(&studio, agent_code, now())
                .await?
                .into_iter()
                .map(|candidate| RouteCandidate {
                    account_id: candidate.id,
                    provider: candidate.provider,
                    model: candidate.model,
                    capabilities: candidate
                        .capabilities
                        .into_iter()
                        .map(runtime_capability)
                        .collect(),
                    available: candidate.available,
                })
                .collect::<Vec<_>>();
            if !candidates.is_empty() || !list_ai_providers(&studio).await?.is_empty() {
                self.routes.replace_candidates(candidates)?;
            }
            let accounts = self
                .routes
                .candidates()?
                .into_iter()
                .map(|candidate| ProviderAccountMetadata {
                    id: candidate.account_id,
                    provider: candidate.provider,
                    model: candidate.model,
                    enabled: candidate.available,
                })
                .collect::<Vec<_>>();
            upsert_provider_accounts(&studio, &accounts).await?;
            if let Some(binding) = load_route_binding(&studio, scope).await? {
                self.routes.restore_binding(
                    binding.scope_key,
                    RouteDecision {
                        account_id: binding.provider_account_id,
                        provider: binding.provider,
                        model: binding.model,
                    },
                )?;
            }
        }
        let (decision, event) = self.routes.select(capability, scope)?;
        if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            let event_payload_json =
                serde_json::to_string(&event).map_err(StoreError::Serialization)?;
            record_route_selection(
                &studio,
                &StoredRouteBinding {
                    scope_key: scope.to_string(),
                    provider_account_id: decision.account_id.clone(),
                    provider: decision.provider.clone(),
                    model: decision.model.clone(),
                },
                RouteSelector::event_type(&event),
                &event_payload_json,
                now(),
            )
            .await?;
        }
        Ok((decision, event))
    }

    async fn reserve_usage_with_failover<E: CodexExecutionPort>(
        &self,
        execution: &E,
        store: &ProjectStore,
        request: &ExecuteTaskRequest,
        idempotency_key: &str,
        mut binding: ConversationCodexThread,
        mut decision: RouteDecision,
    ) -> Result<(ConversationCodexThread, RouteDecision), OrchestrationError> {
        loop {
            match self
                .reserve_request_usage(&decision, request.capability, idempotency_key)
                .await
            {
                Ok(()) => return Ok((binding, decision)),
                Err(OrchestrationError::Route(RouteError::QuotaExceeded { .. })) => {
                    self.routes.report(
                        &decision,
                        RouteOutcome::Failed(RouteFailureKind::CapabilityUnavailable),
                    )?;
                    (binding, decision) = self.ensure_binding(execution, store, request).await?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn reserve_request_usage(
        &self,
        decision: &RouteDecision,
        capability: Capability,
        idempotency_key: &str,
    ) -> Result<(), OrchestrationError> {
        let (metric, limit_kind) = if matches!(
            capability,
            Capability::ImageTextToImage | Capability::ImageImageToImage
        ) {
            ("images", codex_game_domain::LimitKind::Images)
        } else {
            ("calls", codex_game_domain::LimitKind::Calls)
        };
        let requirements = [QuotaRequirement {
            metric: metric.to_string(),
            amount: 1,
        }];
        let Some(studio_storage) = &self.studio_storage else {
            self.routes
                .reserve_usage(decision, idempotency_key, &requirements)?;
            return Ok(());
        };
        let studio = open_studio_store(studio_storage).await?;
        let reserved = reserve_ai_usage(
            &studio,
            &decision.account_id,
            idempotency_key,
            &[(limit_kind, 1)],
            now(),
        )
        .await
        .map_err(|error| match error {
            StoreError::Conflict(_) => RouteError::QuotaExceeded {
                account_id: decision.account_id.clone(),
                metric: metric.to_string(),
            }
            .into(),
            other => OrchestrationError::Store(other),
        })?;
        if !reserved {
            return Ok(());
        }
        let events = vec![RouteEvent::UsageUpdated {
            account_id: decision.account_id.clone(),
            metric: metric.to_string(),
            amount: 1,
        }];
        let entries = events
            .into_iter()
            .filter_map(|event| {
                let event_payload_json = serde_json::to_string(&event).ok()?;
                match event {
                    RouteEvent::UsageUpdated { metric, amount, .. } => Some(StudioUsageEntry {
                        metric,
                        amount,
                        event_payload_json,
                    }),
                    RouteEvent::Selected { .. } | RouteEvent::Switched { .. } => None,
                }
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            record_usage(
                &studio,
                &decision.account_id,
                idempotency_key,
                &entries,
                now(),
            )
            .await?;
        }
        Ok(())
    }

    async fn report_route(
        &self,
        decision: &RouteDecision,
        outcome: RouteOutcome,
        reason: &str,
    ) -> Result<(), OrchestrationError> {
        self.routes.report(decision, outcome)?;
        if let Some(studio_storage) = &self.studio_storage {
            let studio = open_studio_store(studio_storage).await?;
            match outcome {
                RouteOutcome::Succeeded => {
                    record_ai_route_success(&studio, &decision.account_id).await?
                }
                RouteOutcome::Failed(
                    RouteFailureKind::Retryable
                    | RouteFailureKind::CapabilityUnavailable
                    | RouteFailureKind::Fatal,
                ) => record_ai_route_failure(&studio, &decision.account_id, reason, now()).await?,
                RouteOutcome::Failed(
                    RouteFailureKind::ContextTooLarge | RouteFailureKind::InvalidRequest,
                ) => {}
            }
        }
        Ok(())
    }

    async fn binding_lock(&self, conversation_id: &str, agent_code: &str) -> Arc<Mutex<()>> {
        let key = format!("{conversation_id}\u{1f}{agent_code}");
        let mut locks = self.binding_locks.lock().await;
        Arc::clone(locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
    }
}

fn runtime_capability(capability: AiCapability) -> Capability {
    match capability {
        AiCapability::TextReasoning => Capability::TextReasoning,
        AiCapability::TextStructuredOutput => Capability::TextStructuredOutput,
        AiCapability::VisionAnalysis => Capability::VisionAnalysis,
        AiCapability::ImageTextToImage => Capability::ImageTextToImage,
        AiCapability::ImageImageToImage => Capability::ImageImageToImage,
        AiCapability::ImageReferenceConsistency => Capability::ImageReferenceConsistency,
        AiCapability::VideoTextToVideo => Capability::VideoTextToVideo,
        AiCapability::VideoImageToVideo => Capability::VideoImageToVideo,
        AiCapability::Model3d => Capability::Model3d,
        AiCapability::SpeechRecognition => Capability::SpeechRecognition,
    }
}

fn route_failure_kind(error: &ExecutionError) -> Option<RouteFailureKind> {
    match error {
        ExecutionError::Retryable(_) => Some(RouteFailureKind::Retryable),
        ExecutionError::CapabilityUnavailable(_) => Some(RouteFailureKind::CapabilityUnavailable),
        ExecutionError::Fatal(_) => Some(RouteFailureKind::Fatal),
        ExecutionError::ContextTooLarge(_) | ExecutionError::InvalidRequest(_) => None,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod tests;
