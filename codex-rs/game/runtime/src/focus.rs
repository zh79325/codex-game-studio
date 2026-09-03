use codex_game_domain::FocusWorkflow;
use codex_game_domain::WorkflowCommand;
use codex_game_domain::WorkflowError;
use codex_game_domain::validate_input_version;

pub const BRIEF_AGENT: &str = "brief";
pub const GAME_DESIGN_REVIEW_AGENT: &str = "game-design-review";
pub const VISUAL_STYLE_REVIEW_AGENT: &str = "visual-style-review";
pub const PRODUCTION_FEASIBILITY_REVIEW_AGENT: &str = "production-feasibility-review";
pub const SYNTHESIS_AGENT: &str = "synthesis";

pub const FOCUS_AGENT_CODES: [&str; 5] = [
    BRIEF_AGENT,
    GAME_DESIGN_REVIEW_AGENT,
    VISUAL_STYLE_REVIEW_AGENT,
    PRODUCTION_FEASIBILITY_REVIEW_AGENT,
    SYNTHESIS_AGENT,
];

#[derive(Debug, Clone, Copy)]
pub struct BundledAgentDefinition {
    pub code: &'static str,
    pub markdown: &'static str,
}

pub const BUNDLED_FOCUS_AGENTS: [BundledAgentDefinition; 5] = [
    BundledAgentDefinition {
        code: BRIEF_AGENT,
        markdown: include_str!("../agents/brief.md"),
    },
    BundledAgentDefinition {
        code: GAME_DESIGN_REVIEW_AGENT,
        markdown: include_str!("../agents/game-design-review.md"),
    },
    BundledAgentDefinition {
        code: VISUAL_STYLE_REVIEW_AGENT,
        markdown: include_str!("../agents/visual-style-review.md"),
    },
    BundledAgentDefinition {
        code: PRODUCTION_FEASIBILITY_REVIEW_AGENT,
        markdown: include_str!("../agents/production-feasibility-review.md"),
    },
    BundledAgentDefinition {
        code: SYNTHESIS_AGENT,
        markdown: include_str!("../agents/synthesis.md"),
    },
];

pub fn validate_bundled_agents() -> Result<(), &'static str> {
    for definition in BUNDLED_FOCUS_AGENTS {
        if !definition.markdown.starts_with("---\n")
            || !definition
                .markdown
                .contains(&format!("\ncode: {}\n", definition.code))
            || !definition.markdown.contains("\noutput_schema: ")
            || !definition.markdown.contains("\n---\n")
        {
            return Err("invalid bundled focus agent definition");
        }
    }
    Ok(())
}

pub fn bundled_agent_definition(agent_code: &str) -> Option<&'static str> {
    BUNDLED_FOCUS_AGENTS
        .iter()
        .find(|definition| definition.code == agent_code)
        .map(|definition| definition.markdown)
}

pub fn advance_workflow(
    workflow: &mut FocusWorkflow,
    command: WorkflowCommand,
    expected_input_version: u64,
) -> Result<(), WorkflowError> {
    validate_input_version(expected_input_version, workflow.input_version)?;
    workflow.state = workflow.state.apply(command)?;
    workflow.workflow_version += 1;
    Ok(())
}

pub fn review_agent_codes() -> [&'static str; 3] {
    [
        GAME_DESIGN_REVIEW_AGENT,
        VISUAL_STYLE_REVIEW_AGENT,
        PRODUCTION_FEASIBILITY_REVIEW_AGENT,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_agents_are_valid() {
        validate_bundled_agents().expect("bundled agents must validate at build time");
    }
}
