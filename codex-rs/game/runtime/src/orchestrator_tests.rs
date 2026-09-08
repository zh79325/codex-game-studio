use super::*;
use crate::StartedThread;
use crate::StartedTurn;
use codex_game_domain::AiProvider;
use codex_game_domain::LimitPolicy;
use codex_game_domain::ProviderModel;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tempfile::tempdir;

#[derive(Default)]
struct FakeExecution {
    starts: AtomicUsize,
    turns: AtomicUsize,
    interrupts: AtomicUsize,
    max_output_tokens: Mutex<Vec<Option<u64>>>,
}

impl CodexExecutionPort for FakeExecution {
    async fn start_thread(
        &self,
        _request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        let number = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(StartedThread {
            thread_id: format!("thread-{number}"),
            session_id: format!("session-{number}"),
        })
    }

    async fn thread_available(&self, _thread_id: &str) -> bool {
        self.starts.load(Ordering::SeqCst) > 0
    }

    async fn start_turn(&self, request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        self.max_output_tokens
            .lock()
            .expect("max output token lock")
            .push(request.max_output_tokens);
        let number = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(StartedTurn {
            turn_id: format!("turn-{number}"),
        })
    }

    async fn steer_turn(&self, _request: SteerTurnRequest) -> Result<(), ExecutionError> {
        Ok(())
    }

    async fn interrupt_turn(
        &self,
        _thread_id: String,
        _turn_id: String,
    ) -> Result<(), ExecutionError> {
        self.interrupts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn request(root: &str, key: &str) -> ExecuteTaskRequest {
    ExecuteTaskRequest {
        project_root: root.to_string(),
        workspace_root: root.to_string(),
        conversation_id: "conversation-1".to_string(),
        conversation_turn: 1,
        target_id: "project-1".to_string(),
        audit_target: "project".to_string(),
        audit_target_dir: root.into(),
        stage: "project".to_string(),
        agent_code: "art_bible_designer".to_string(),
        idempotency_key: key.to_string(),
        prompt: "define the project art bible".to_string(),
        local_image_paths: Vec::new(),
        media_generation_consumed: false,
        context: ContextPackage {
            conversation_history: Vec::new(),
            context_version: 1,
            contract_version: 1,
            agent_definition_version: "1".to_string(),
            output_schema: "{}".to_string(),
            action_schema: "{}".to_string(),
            action_examples: "example".to_string(),
            target_kind: "project".to_string(),
            target_ref: None,
            stage: "project".to_string(),
            art_bible: None,
            character_context: None,
            workflow_context: None,
            review_subject: None,
            visual_focus: None,
            recovery_context: None,
            memories: Vec::new(),
            allowed_handoffs: Vec::new(),
            agent_role_descriptions: Vec::new(),
            action_protocol: "strict action".to_string(),
        },
        capability: Capability::TextStructuredOutput,
        internal_executors: Vec::new(),
    }
}

fn candidate(account_id: &str, capability: Capability) -> RouteCandidate {
    RouteCandidate {
        account_id: account_id.to_string(),
        provider: format!("provider-{account_id}"),
        model: format!("model-{account_id}"),
        capabilities: vec![capability],
        available: true,
    }
}

fn in_memory_orchestrator() -> TaskOrchestrator {
    TaskOrchestrator::new(
        vec![candidate("default", Capability::TextStructuredOutput)],
        None,
    )
}

async fn seed_agent_routes(
    studio_root: &Path,
    agent_code: &str,
    routes: &[(&str, &str, Capability)],
) {
    let pool = codex_game_store::open_studio_store(studio_root)
        .await
        .expect("studio store");
    let mut model_ids = Vec::with_capacity(routes.len());
    for (sort_no, (model_id, provider_code, capability)) in routes.iter().enumerate() {
        codex_game_store::upsert_ai_provider(
            &pool,
            &AiProvider {
                code: (*provider_code).to_string(),
                name: (*provider_code).to_string(),
                base_url: "https://example.test".to_string(),
                driver: "openai".to_string(),
                auth_style: "bearer".to_string(),
                priority: sort_no as i64,
                enabled: true,
                remark: String::new(),
                has_key: false,
                key_mask: None,
                models: Vec::new(),
            },
        )
        .await
        .expect("upsert provider");
        codex_game_store::upsert_ai_model(
            &pool,
            &ProviderModel {
                id: (*model_id).to_string(),
                provider_code: (*provider_code).to_string(),
                model_id: format!("remote-{model_id}"),
                display_name: (*model_id).to_string(),
                capabilities: vec![match capability {
                    Capability::TextStructuredOutput => AiCapability::TextStructuredOutput,
                    Capability::ImageTextToImage => AiCapability::ImageTextToImage,
                    Capability::ImageImageToImage => AiCapability::ImageImageToImage,
                    other => panic!("unsupported test capability: {other:?}"),
                }],
                driver: "openai".to_string(),
                api_path: "/v1/responses".to_string(),
                enabled: true,
                sort_no: sort_no as i64,
                params: serde_json::json!({}),
                remark: String::new(),
                limits: Vec::<LimitPolicy>::new(),
            },
        )
        .await
        .expect("upsert model");
        model_ids.push((*model_id).to_string());
    }
    codex_game_store::write_agent_binding(&pool, agent_code, &model_ids)
        .await
        .expect("write agent binding");
    pool.close().await;
}

struct FailoverExecution;

impl CodexExecutionPort for FailoverExecution {
    async fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        if request.route.account_id == "account-a" {
            return Err(ExecutionError::Retryable("rate limited".to_string()));
        }
        Ok(StartedThread {
            thread_id: "thread-b".to_string(),
            session_id: "session-b".to_string(),
        })
    }

    async fn thread_available(&self, _thread_id: &str) -> bool {
        false
    }

    async fn start_turn(&self, _request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        Ok(StartedTurn {
            turn_id: "turn-b".to_string(),
        })
    }

    async fn steer_turn(&self, _request: SteerTurnRequest) -> Result<(), ExecutionError> {
        Ok(())
    }

    async fn interrupt_turn(
        &self,
        _thread_id: String,
        _turn_id: String,
    ) -> Result<(), ExecutionError> {
        Ok(())
    }
}

struct InternalRouteFailoverExecution {
    starts: AtomicUsize,
}

impl CodexExecutionPort for InternalRouteFailoverExecution {
    async fn start_thread(
        &self,
        request: StartThreadRequest,
    ) -> Result<StartedThread, ExecutionError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            request
                .image_generation_tools
                .iter()
                .map(|tool| tool.tool_name.as_str())
                .collect::<Vec<_>>(),
            vec!["image_t2i", "image_i2i"]
        );
        let image_route = request
            .image_generation_tools
            .iter()
            .find(|tool| tool.tool_name == "image_t2i")
            .expect("text-to-image route")
            .route
            .clone();
        assert_eq!(
            request
                .image_generation_tools
                .iter()
                .find(|tool| tool.tool_name == "image_i2i")
                .expect("image-to-image route")
                .route
                .account_id
                .as_str(),
            "image-edit-model"
        );
        assert_eq!(request.route.account_id, "main-model");
        if image_route.account_id == "image-model-a" {
            return Err(ExecutionError::RouteUnavailable {
                route: image_route,
                message: "image provider unavailable".to_string(),
            });
        }
        Ok(StartedThread {
            thread_id: "thread-image-b".to_string(),
            session_id: "session-image-b".to_string(),
        })
    }

    async fn thread_available(&self, _thread_id: &str) -> bool {
        false
    }

    async fn start_turn(&self, _request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
        Ok(StartedTurn {
            turn_id: "turn-image-b".to_string(),
        })
    }

    async fn steer_turn(&self, _request: SteerTurnRequest) -> Result<(), ExecutionError> {
        Ok(())
    }

    async fn interrupt_turn(
        &self,
        _thread_id: String,
        _turn_id: String,
    ) -> Result<(), ExecutionError> {
        Ok(())
    }
}

#[tokio::test]
async fn initial_turn_uses_the_agent_max_output_tokens() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let store = ProjectStore::open(&project_root).await.expect("store");
    let execution = FakeExecution::default();
    let mut task_request = request(project_root.to_str().expect("root"), "message-review");
    task_request.agent_code = "vision_reviewer".to_string();

    in_memory_orchestrator()
        .execute(&execution, &store, task_request)
        .await
        .expect("review execution");

    assert_eq!(
        execution
            .max_output_tokens
            .lock()
            .expect("max output token lock")
            .as_slice(),
        &[Some(64_000)]
    );
}

#[tokio::test]
async fn renamed_agent_uses_legacy_model_binding() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let studio_root = directory.path().join("studio");
    let store = ProjectStore::open(&project_root).await.expect("store");
    seed_agent_routes(
        &studio_root,
        "game_designer",
        &[(
            "legacy-model",
            "legacy-provider",
            Capability::TextStructuredOutput,
        )],
    )
    .await;

    let execution = TaskOrchestrator::new(Vec::new(), Some(studio_root))
        .execute(
            &FakeExecution::default(),
            &store,
            request(project_root.to_str().expect("root"), "legacy-binding"),
        )
        .await
        .expect("legacy binding remains usable");

    assert_eq!(execution.binding.agent_code, "art_bible_designer");
}

#[tokio::test]
async fn route_failover_is_persisted_before_starting_the_turn() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let studio_root = directory.path().join("studio");
    let store = ProjectStore::open(&project_root).await.expect("store");
    seed_agent_routes(
        &studio_root,
        "art_bible_designer",
        &[
            ("account-a", "provider-a", Capability::TextStructuredOutput),
            ("account-b", "provider-b", Capability::TextStructuredOutput),
        ],
    )
    .await;
    let orchestrator = TaskOrchestrator::new(Vec::new(), Some(studio_root.clone()));

    let execution = orchestrator
        .execute(
            &FailoverExecution,
            &store,
            request(project_root.to_str().expect("root"), "message-1"),
        )
        .await
        .expect("failover execution");

    assert_eq!(execution.binding.codex_thread_id, "thread-b");
    let studio = codex_game_store::open_studio_store(&studio_root)
        .await
        .expect("studio");
    let events = codex_game_store::list_route_events(&studio)
        .await
        .expect("route events");
    let event_types = events
        .iter()
        .map(|event| event.event_type.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec!["route.selected", "route.switched", "usage.updated"]
    );
    // Secrets contract: persisted route event payloads must never carry credentials.
    for event in &events {
        let payload = event.payload_json.to_ascii_lowercase();
        for forbidden in ["secret", "token", "credential", "password", "apikey"] {
            assert!(
                !payload.contains(forbidden),
                "route event payload leaked {forbidden}: {}",
                event.payload_json
            );
        }
    }
}

#[tokio::test]
async fn exhausted_routes_include_the_last_failure_reason() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let studio_root = directory.path().join("studio");
    let store = ProjectStore::open(&project_root).await.expect("store");
    seed_agent_routes(
        &studio_root,
        "art_bible_designer",
        &[("account-a", "provider-a", Capability::TextStructuredOutput)],
    )
    .await;
    let orchestrator = TaskOrchestrator::new(Vec::new(), Some(studio_root));

    let error = orchestrator
        .execute(
            &FailoverExecution,
            &store,
            request(project_root.to_str().expect("root"), "message-exhausted"),
        )
        .await
        .expect_err("single failed route must be exhausted");

    assert!(matches!(
        error,
        OrchestrationError::Execution(ExecutionError::CapabilityUnavailable(message))
            if message.contains("art_bible_designer")
                && message.contains("provider-a/remote-account-a")
                && message.contains("rate limited")
    ));
}

#[tokio::test]
async fn database_mode_does_not_fall_back_to_in_memory_candidates() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let studio_root = directory.path().join("studio");
    let store = ProjectStore::open(&project_root).await.expect("store");
    let orchestrator = TaskOrchestrator::new(
        vec![candidate("legacy", Capability::TextStructuredOutput)],
        Some(studio_root),
    );

    let error = orchestrator
        .execute(
            &FakeExecution::default(),
            &store,
            request(project_root.to_str().expect("root"), "message-empty"),
        )
        .await
        .expect_err("empty database must not use legacy candidates");

    assert!(matches!(
        error,
        OrchestrationError::Execution(ExecutionError::CapabilityUnavailable(message))
            if message.contains("art_bible_designer") && message.contains("TextStructuredOutput")
    ));
}

#[tokio::test]
async fn image_provider_failure_switches_only_the_internal_route() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let studio_root = directory.path().join("studio");
    let store = ProjectStore::open(&project_root).await.expect("store");
    seed_agent_routes(
        &studio_root,
        "art_bible_designer",
        &[(
            "main-model",
            "main-provider",
            Capability::TextStructuredOutput,
        )],
    )
    .await;
    seed_agent_routes(
        &studio_root,
        "image_t2i",
        &[
            (
                "image-model-a",
                "image-provider-a",
                Capability::ImageTextToImage,
            ),
            (
                "image-model-b",
                "image-provider-b",
                Capability::ImageTextToImage,
            ),
        ],
    )
    .await;
    seed_agent_routes(
        &studio_root,
        "image_i2i",
        &[(
            "image-edit-model",
            "image-edit-provider",
            Capability::ImageImageToImage,
        )],
    )
    .await;
    let orchestrator = TaskOrchestrator::new(Vec::new(), Some(studio_root.clone()));
    let execution = InternalRouteFailoverExecution {
        starts: AtomicUsize::new(0),
    };
    let mut task_request = request(project_root.to_str().expect("root"), "message-image");
    task_request.internal_executors = vec![
        InternalExecutor {
            agent_code: "image_t2i".to_string(),
            capability: Capability::ImageTextToImage,
        },
        InternalExecutor {
            agent_code: "image_i2i".to_string(),
            capability: Capability::ImageImageToImage,
        },
    ];

    orchestrator
        .execute(&execution, &store, task_request)
        .await
        .expect("image route failover");

    assert_eq!(execution.starts.load(Ordering::SeqCst), 2);
    let studio = codex_game_store::open_studio_store(&studio_root)
        .await
        .expect("studio");
    let usage = codex_game_store::read_ai_usage(&studio)
        .await
        .expect("read AI usage");
    let main = usage
        .iter()
        .find(|item| item.provider_model_id == "main-model")
        .expect("main model usage");
    assert!(main.breaker.is_none());
    let failed_image = usage
        .iter()
        .find(|item| item.provider_model_id == "image-model-a")
        .expect("failed image model usage");
    assert_eq!(
        failed_image
            .breaker
            .as_ref()
            .expect("failed image model breaker")
            .failure_count,
        1
    );
    assert_eq!(
        codex_game_store::load_route_binding(
            &studio,
            "conversation:conversation-1:internal:image_t2i",
        )
        .await
        .expect("load image binding")
        .expect("image binding")
        .provider_account_id,
        "image-model-b"
    );
    assert_eq!(
        codex_game_store::load_route_binding(&studio, "conversation:conversation-1")
            .await
            .expect("load main binding")
            .expect("main binding")
            .provider_account_id,
        "main-model"
    );
}

#[tokio::test]
async fn concurrent_first_tasks_share_one_active_thread() {
    let directory = tempdir().expect("tempdir");
    let store = Arc::new(ProjectStore::open(directory.path()).await.expect("store"));
    let execution = Arc::new(FakeExecution::default());
    let orchestrator = Arc::new(in_memory_orchestrator());
    let first = {
        let store = Arc::clone(&store);
        let execution = Arc::clone(&execution);
        let orchestrator = Arc::clone(&orchestrator);
        let request = request(directory.path().to_str().expect("root"), "message-1");
        tokio::spawn(async move {
            orchestrator
                .execute(execution.as_ref(), store.as_ref(), request)
                .await
        })
    };
    let second = {
        let store = Arc::clone(&store);
        let execution = Arc::clone(&execution);
        let orchestrator = Arc::clone(&orchestrator);
        let request = request(directory.path().to_str().expect("root"), "message-2");
        tokio::spawn(async move {
            orchestrator
                .execute(execution.as_ref(), store.as_ref(), request)
                .await
        })
    };
    first.await.expect("join").expect("first task");
    second.await.expect("join").expect("second task");

    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    assert_eq!(execution.turns.load(Ordering::SeqCst), 2);
    assert_eq!(
        store
            .list_tasks("conversation-1")
            .await
            .expect("tasks")
            .len(),
        2
    );
}

#[tokio::test]
async fn interrupt_cancels_the_current_attempt_and_task() {
    let directory = tempdir().expect("tempdir");
    let store = ProjectStore::open(directory.path()).await.expect("store");
    let execution = FakeExecution::default();
    let orchestrator = in_memory_orchestrator();
    let started = orchestrator
        .execute(
            &execution,
            &store,
            request(directory.path().to_str().expect("root"), "message-1"),
        )
        .await
        .expect("task");

    let turn_id = started.attempt.codex_turn_id.clone().expect("running turn");
    orchestrator
        .interrupt(
            &execution,
            "conversation-1",
            "art_bible_designer",
            started.binding.codex_thread_id,
            turn_id.clone(),
        )
        .await
        .expect("interrupt");

    assert_eq!(execution.interrupts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.list_tasks("conversation-1").await.expect("tasks")[0].status,
        TaskStatus::Running
    );
    store
        .complete_turn(&turn_id, TaskAttemptStatus::Cancelled)
        .await
        .expect("complete aborted turn");
    assert_eq!(
        store.list_tasks("conversation-1").await.expect("tasks")[0].status,
        TaskStatus::Cancelled
    );
    assert!(
        store
            .running_attempts("conversation-1")
            .await
            .expect("running attempts")
            .is_empty()
    );
}

#[tokio::test]
async fn rebuilds_an_unavailable_active_thread() {
    let directory = tempdir().expect("tempdir");
    let store = ProjectStore::open(directory.path()).await.expect("store");
    let first_execution = FakeExecution::default();
    let first = in_memory_orchestrator()
        .execute(
            &first_execution,
            &store,
            request(directory.path().to_str().expect("root"), "message-1"),
        )
        .await
        .expect("first task");

    let restarted_execution = FakeExecution {
        starts: AtomicUsize::new(0),
        turns: AtomicUsize::new(1),
        interrupts: AtomicUsize::new(0),
        max_output_tokens: Mutex::new(Vec::new()),
    };
    let rebuilt = in_memory_orchestrator()
        .execute(
            &restarted_execution,
            &store,
            request(directory.path().to_str().expect("root"), "message-2"),
        )
        .await
        .expect("task after restart");

    assert_eq!(rebuilt.binding.binding_version, 2);
    assert_eq!(
        rebuilt.binding.forked_from_id,
        Some(first.binding.id.clone())
    );
    assert_eq!(
        rebuilt.binding.replacement_reason.as_deref(),
        Some("thread-unavailable")
    );
    assert_eq!(restarted_execution.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .active_thread("conversation-1", "art_bible_designer")
            .await
            .expect("active binding")
            .expect("binding")
            .id,
        rebuilt.binding.id
    );
}
