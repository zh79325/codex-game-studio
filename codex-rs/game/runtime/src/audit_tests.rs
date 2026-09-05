use super::*;
use codex_game_domain::ContextPackage;
use std::fs;
use tempfile::TempDir;

#[test]
fn writes_request_and_completion_when_setting_is_missing() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(
        temp.path().join("project.json"),
        r#"{"schemaVersion":2,"projectId":"project-1"}"#,
    )
    .expect("project config");
    let context = audit_context(&temp);
    let request = start_request("inspect data:image/png;base64,aGVsbG8=");

    write_turn_audit_request(&context, &route(), &request).expect("write request");
    append_turn_audit_completion(
        &context,
        &TurnAuditCompletion {
            response: Some("done".to_string()),
            usage: Some(TurnAuditUsage {
                input_tokens: 10,
                cached_input_tokens: 2,
                output_tokens: 3,
                reasoning_output_tokens: 1,
                total_tokens: 13,
            }),
            duration_ms: Some(25),
            time_to_first_token_ms: Some(5),
            ..TurnAuditCompletion::default()
        },
    )
    .expect("write completion");

    let contents = fs::read_to_string(audit_path(&context)).expect("audit contents");
    assert!(contents.contains("Provider：provider-1"));
    assert!(contents.contains("Model：model-1"));
    assert!(contents.contains("[data URL omitted: mime=image/png, encoded_chars=8, sha256="));
    assert!(!contents.contains("aGVsbG8="));
    assert!(contents.contains("### Response"));
    assert!(contents.contains("done"));
    assert!(!contents.contains("### Stream"));
    assert!(contents.contains("- Total tokens：13"));
    assert!(contents.contains("- Latency：25 ms"));
    assert!(contents.contains("- Time to first token：5 ms"));
}

#[test]
fn writes_abnormal_stream_termination() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(
        temp.path().join("project.json"),
        r#"{"schemaVersion":2,"projectId":"project-1"}"#,
    )
    .expect("project config");
    let context = audit_context(&temp);

    write_turn_audit_request(&context, &route(), &start_request("hello")).expect("write request");
    append_turn_audit_stream_termination(
        &context,
        "sse_parser",
        "stream closed before response.completed",
    )
    .expect("write stream termination");

    let contents = fs::read_to_string(audit_path(&context)).expect("audit contents");
    assert!(contents.contains("### Stream Terminated"));
    assert!(contents.contains("- Stage：sse_parser"));
    assert!(contents.contains("stream closed before response.completed"));
}

#[test]
fn skips_audit_when_explicitly_disabled() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(
        temp.path().join("project.json"),
        r#"{"schemaVersion":2,"projectId":"project-1","conversationAudit":false}"#,
    )
    .expect("project config");
    let context = audit_context(&temp);

    write_turn_audit_request(&context, &route(), &start_request("hello")).expect("skip request");

    assert!(!audit_path(&context).exists());
}

fn audit_context(temp: &TempDir) -> TurnAuditContext {
    TurnAuditContext {
        project_root: temp.path().to_path_buf(),
        target_dir: temp.path().to_path_buf(),
        conversation_id: "conversation-1".to_string(),
        turn: 2,
        target: "project".to_string(),
        agent_code: "director".to_string(),
        attempt_id: "attempt-1".to_string(),
    }
}

fn route() -> RouteDecision {
    RouteDecision {
        account_id: "account-1".to_string(),
        provider: "provider-1".to_string(),
        model: "model-1".to_string(),
    }
}

fn start_request(prompt: &str) -> StartTurnRequest {
    StartTurnRequest {
        thread_id: "thread-1".to_string(),
        attempt_id: "attempt-1".to_string(),
        agent_definition: "agent definition".to_string(),
        prompt: prompt.to_string(),
        context: ContextPackage {
            conversation_history: Vec::new(),
            context_version: 1,
            contract_version: 1,
            agent_definition_version: "v1".to_string(),
            output_schema: "{}".to_string(),
            target_kind: "project".to_string(),
            target_ref: None,
            stage: "planning".to_string(),
            art_bible: None,
            character_context: None,
            workflow_context: None,
            review_subject: None,
            memories: Vec::new(),
            allowed_handoffs: Vec::new(),
            action_protocol: "json".to_string(),
        },
        max_output_tokens: None,
        audit_context: None,
    }
}
