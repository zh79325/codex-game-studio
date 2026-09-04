use super::*;
use crate::StartedThread;
use crate::StartedTurn;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tempfile::tempdir;

#[derive(Default)]
struct FakeExecution {
    starts: AtomicUsize,
    turns: AtomicUsize,
    interrupts: AtomicUsize,
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

    async fn start_turn(&self, _request: StartTurnRequest) -> Result<StartedTurn, ExecutionError> {
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
        conversation_id: "conversation-1".to_string(),
        conversation_turn: 1,
        target_id: "project-1".to_string(),
        audit_target: "project".to_string(),
        audit_target_dir: root.into(),
        stage: "project".to_string(),
        agent_code: "game_designer".to_string(),
        idempotency_key: key.to_string(),
        prompt: "design a game".to_string(),
        context: ContextPackage {
            conversation_history: Vec::new(),
            context_version: 1,
            contract_version: 1,
            agent_definition_version: "1".to_string(),
            output_schema: "{}".to_string(),
            target_kind: "project".to_string(),
            target_ref: None,
            stage: "project".to_string(),
            art_bible: None,
            character_context: None,
            memories: Vec::new(),
            allowed_handoffs: Vec::new(),
            action_protocol: "strict action".to_string(),
        },
        capability: Capability::TextStructuredOutput,
    }
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

#[tokio::test]
async fn route_failover_is_persisted_before_starting_the_turn() {
    let directory = tempdir().expect("tempdir");
    let project_root = directory.path().join("project");
    let studio_root = directory.path().join("studio");
    let store = ProjectStore::open(&project_root).await.expect("store");
    let candidates = ["account-a", "account-b"]
        .into_iter()
        .map(|account_id| RouteCandidate {
            account_id: account_id.to_string(),
            provider: "provider".to_string(),
            model: "model".to_string(),
            capabilities: vec![Capability::TextStructuredOutput],
            available: true,
        })
        .collect();
    let orchestrator = TaskOrchestrator::new(candidates, Some(studio_root.clone()));

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
async fn concurrent_first_tasks_share_one_active_thread() {
    let directory = tempdir().expect("tempdir");
    let store = Arc::new(ProjectStore::open(directory.path()).await.expect("store"));
    let execution = Arc::new(FakeExecution::default());
    let orchestrator = Arc::new(TaskOrchestrator::default());
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
    let orchestrator = TaskOrchestrator::default();
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
            "game_designer",
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
    let first = TaskOrchestrator::default()
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
    };
    let rebuilt = TaskOrchestrator::default()
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
            .active_thread("conversation-1", "game_designer")
            .await
            .expect("active binding")
            .expect("binding")
            .id,
        rebuilt.binding.id
    );
}
