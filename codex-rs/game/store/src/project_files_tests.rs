use super::*;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[test]
fn enables_conversation_audit_for_new_projects() {
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("project.json");

    update_project_json(&path, "project-1", "Game", "drafting").expect("project config");

    let document: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("project contents"))
            .expect("valid project json");
    assert_eq!(document["conversationAudit"], Value::Bool(true));
}

#[test]
fn preserves_disabled_conversation_audit() {
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("project.json");
    fs::write(
        &path,
        r#"{"projectId":"project-1","conversationAudit":false}"#,
    )
    .expect("project config");

    update_project_json(&path, "project-1", "Game", "drafting").expect("update project config");

    let document: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("project contents"))
            .expect("valid project json");
    assert_eq!(document["conversationAudit"], Value::Bool(false));
}
