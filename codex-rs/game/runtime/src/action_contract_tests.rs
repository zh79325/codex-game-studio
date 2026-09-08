use super::*;

#[test]
fn spec_writer_contract_exposes_exact_draft_shape() {
    let profile = action_contract_profile("spec_writer", "spec", "studio_director", &[], None);

    let schema: Value = serde_json::from_str(&profile.schema).expect("profile schema");
    assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(3));
    assert!(profile.schema.contains("artifact_slot"));
    assert!(!profile.schema.contains("target_path"));
    assert!(
        profile
            .examples
            .contains("\"artifact_slot\": \"character_spec\"")
    );
    assert!(profile.examples.contains("\"action\": \"done\""));
    assert!(!profile.examples.contains("target_path"));
}

#[test]
fn director_contract_uses_only_current_handoff_targets() {
    let profile = action_contract_profile(
        "studio_director",
        "render",
        "studio_director",
        &["visual_designer".to_string()],
        None,
    );

    assert!(profile.instruction.contains("visual_designer"));
    assert!(
        profile
            .examples
            .contains("\"target_agent\": \"visual_designer\"")
    );
    assert!(!profile.examples.contains("spec_writer"));
}

#[test]
fn project_naming_contract_exposes_only_the_required_step() {
    let director = action_contract_profile(
        "studio_director",
        "project",
        "studio_director",
        &["art_bible_designer".to_string()],
        Some("collectProjectNaming"),
    );
    let schema: Value = serde_json::from_str(&director.schema).expect("director schema");
    assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(2));
    assert!(director.examples.contains("\"action\": \"handoff\""));
    assert!(!director.examples.contains("\"action\": \"done\""));

    let designer = action_contract_profile(
        "art_bible_designer",
        "project",
        "studio_director",
        &[],
        Some("collectProjectNaming"),
    );
    assert!(designer.examples.contains("\"item\": \"项目名称\""));
    assert!(designer.examples.contains("\"item\": \"项目代号\""));
    assert!(!designer.examples.contains("\"action\": \"handoff\""));
}

#[test]
fn visual_contract_does_not_request_path_echoing() {
    let profile = action_contract_profile(
        "visual_designer",
        "render",
        "studio_director",
        &["studio_director".to_string()],
        None,
    );

    let schema: Value = serde_json::from_str(&profile.schema).expect("profile schema");
    assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(3));
    assert!(profile.examples.contains("\"revision_scope\": \"render\""));
    assert!(profile.examples.contains("\"action\": \"done\""));
    assert!(!profile.schema.contains("\"artifacts\""));
    assert!(!profile.examples.contains("\"path\""));
    assert!(!profile.examples.contains("\"artifacts\""));
}

#[test]
fn every_profile_example_passes_the_runtime_parser() {
    let cases = [
        ("studio_director", "spec", vec!["spec_writer".to_string()]),
        ("spec_writer", "spec", Vec::new()),
        ("spec_reviewer", "spec", Vec::new()),
        ("visual_designer", "render", Vec::new()),
        ("visual_designer", "views", Vec::new()),
    ];

    for (agent, stage, allowed_handoffs) in cases {
        let profile =
            action_contract_profile(agent, stage, "studio_director", &allowed_handoffs, None);
        for chunk in profile.examples.split(ACTION_END) {
            if chunk.trim().is_empty() {
                continue;
            }
            let output = format!("{}\n{}", chunk.trim(), ACTION_END);
            crate::action::parse_agent_turn(&output, agent, "studio_director", &allowed_handoffs)
                .unwrap_or_else(|error| panic!("{agent}/{stage} 示例不合法：{error}\n{output}"));
        }
    }
}
