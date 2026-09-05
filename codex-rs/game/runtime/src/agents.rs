use codex_game_domain::AgentCapability;
use codex_game_domain::AgentDefinition;
use codex_game_domain::AgentRoleType;
use serde::Deserialize;

pub const STUDIO_DIRECTOR_AGENT: &str = "studio_director";
pub const GAME_DESIGNER_AGENT: &str = "game_designer";
pub const SPEC_WRITER_AGENT: &str = "spec_writer";
pub const SPEC_REVIEWER_AGENT: &str = "spec_reviewer";
pub const VISUAL_DESIGNER_AGENT: &str = "visual_designer";
pub const PROMPT_SMITH_AGENT: &str = "prompt_smith";
pub const IMAGE_T2I_AGENT: &str = "image_t2i";
pub const IMAGE_I2I_AGENT: &str = "image_i2i";
pub const VISION_REVIEWER_AGENT: &str = "vision_reviewer";
pub const MODEL3D_AGENT: &str = "model3d";
pub const VIDEO_GEN_AGENT: &str = "video_gen";
pub const REALTIME_SPEECH_AGENT: &str = "realtime_speech";

pub const AGENT_CODES: [&str; 12] = [
    STUDIO_DIRECTOR_AGENT,
    GAME_DESIGNER_AGENT,
    SPEC_WRITER_AGENT,
    SPEC_REVIEWER_AGENT,
    VISUAL_DESIGNER_AGENT,
    PROMPT_SMITH_AGENT,
    IMAGE_T2I_AGENT,
    IMAGE_I2I_AGENT,
    VISION_REVIEWER_AGENT,
    MODEL3D_AGENT,
    VIDEO_GEN_AGENT,
    REALTIME_SPEECH_AGENT,
];

#[derive(Debug, Clone, Copy)]
pub struct BundledAgentDefinition {
    pub code: &'static str,
    pub markdown: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFrontmatter {
    agent_code: String,
    capability: AgentCapability,
    role: String,
    role_type: AgentRoleType,
    focusable: bool,
    aliases: Vec<String>,
    target_kinds: Vec<String>,
    stages: Vec<String>,
    max_turns: u32,
    conversational: bool,
    memory_scope: String,
    context_budget: u32,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    output_contract: String,
    allow_tools: Vec<String>,
}

pub const BUNDLED_AGENTS: [BundledAgentDefinition; 12] = [
    BundledAgentDefinition {
        code: STUDIO_DIRECTOR_AGENT,
        markdown: include_str!("../agents/studio_director.md"),
    },
    BundledAgentDefinition {
        code: GAME_DESIGNER_AGENT,
        markdown: include_str!("../agents/game_designer.md"),
    },
    BundledAgentDefinition {
        code: SPEC_WRITER_AGENT,
        markdown: include_str!("../agents/spec_writer.md"),
    },
    BundledAgentDefinition {
        code: SPEC_REVIEWER_AGENT,
        markdown: include_str!("../agents/spec_reviewer.md"),
    },
    BundledAgentDefinition {
        code: VISUAL_DESIGNER_AGENT,
        markdown: include_str!("../agents/visual_designer.md"),
    },
    BundledAgentDefinition {
        code: PROMPT_SMITH_AGENT,
        markdown: include_str!("../agents/prompt_smith.md"),
    },
    BundledAgentDefinition {
        code: IMAGE_T2I_AGENT,
        markdown: include_str!("../agents/image_t2i.md"),
    },
    BundledAgentDefinition {
        code: IMAGE_I2I_AGENT,
        markdown: include_str!("../agents/image_i2i.md"),
    },
    BundledAgentDefinition {
        code: VISION_REVIEWER_AGENT,
        markdown: include_str!("../agents/vision_reviewer.md"),
    },
    BundledAgentDefinition {
        code: MODEL3D_AGENT,
        markdown: include_str!("../agents/model3d.md"),
    },
    BundledAgentDefinition {
        code: VIDEO_GEN_AGENT,
        markdown: include_str!("../agents/video_gen.md"),
    },
    BundledAgentDefinition {
        code: REALTIME_SPEECH_AGENT,
        markdown: include_str!("../agents/realtime_speech.md"),
    },
];

pub fn bundled_agent_definitions() -> Result<Vec<AgentDefinition>, String> {
    BUNDLED_AGENTS
        .iter()
        .map(|definition| {
            let source = definition
                .markdown
                .strip_prefix("---\n")
                .ok_or_else(|| format!("{} 缺少 YAML frontmatter", definition.code))?;
            let (frontmatter, body) = source
                .split_once("\n---\n")
                .ok_or_else(|| format!("{} 的 YAML frontmatter 未闭合", definition.code))?;
            let metadata: AgentFrontmatter = serde_yaml::from_str(frontmatter)
                .map_err(|error| format!("{} 的 frontmatter 不合法：{error}", definition.code))?;
            if metadata.agent_code != definition.code {
                return Err(format!(
                    "{} 的 agent_code 与文件注册不一致",
                    definition.code
                ));
            }
            if !body.trim_start().starts_with("你是")
                || ["### 职责", "### 输出格式", "### 绝不可做"]
                    .iter()
                    .any(|section| !body.contains(section))
            {
                return Err(format!("{} 的提示词正文缺少固定章节", definition.code));
            }
            Ok(AgentDefinition {
                agent_code: metadata.agent_code,
                role: metadata.role,
                role_type: metadata.role_type,
                capability: metadata.capability,
                focusable: metadata.focusable,
                aliases: metadata.aliases,
                target_kinds: metadata.target_kinds,
                stages: metadata.stages,
                max_turns: metadata.max_turns,
                conversational: metadata.conversational,
                memory_scope: metadata.memory_scope,
                context_budget: metadata.context_budget,
                max_output_tokens: metadata.max_output_tokens,
                output_contract: metadata.output_contract,
                allow_tools: metadata.allow_tools,
                source_file: format!("game/runtime/agents/{}.md", definition.code),
                model_ids: Vec::new(),
            })
        })
        .collect()
}

pub fn validate_bundled_agents() -> Result<(), String> {
    bundled_agent_definitions().map(|_| ())
}

pub fn bundled_agent_definition(agent_code: &str) -> Option<&'static str> {
    BUNDLED_AGENTS
        .iter()
        .find(|definition| definition.code == agent_code)
        .map(|definition| definition.markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_agents_are_valid() {
        validate_bundled_agents().expect("bundled agents must validate at build time");
    }
}
