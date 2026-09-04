use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterState {
    S0SpecDrafting,
    S1SpecConfirmed,
    S2RenderGenerated,
    S3RenderConfirmed,
    S4ViewsGenerated,
    S5ViewsConfirmed,
}

impl CharacterState {
    pub fn stage(self) -> &'static str {
        match self {
            Self::S0SpecDrafting => "spec",
            Self::S1SpecConfirmed | Self::S2RenderGenerated => "render",
            Self::S3RenderConfirmed | Self::S4ViewsGenerated | Self::S5ViewsConfirmed => "views",
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::S0SpecDrafting => Some(Self::S1SpecConfirmed),
            Self::S1SpecConfirmed => Some(Self::S2RenderGenerated),
            Self::S2RenderGenerated => Some(Self::S3RenderConfirmed),
            Self::S3RenderConfirmed => Some(Self::S4ViewsGenerated),
            Self::S4ViewsGenerated => Some(Self::S5ViewsConfirmed),
            Self::S5ViewsConfirmed => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub group: Option<String>,
    pub dir_name: String,
    pub state: CharacterState,
    pub spec_path: Option<String>,
    pub render_path: Option<String>,
    pub view_paths: BTreeMap<String, String>,
    pub hard_constraints: Vec<serde_json::Value>,
    pub gate_spec_confirmed_at: Option<i64>,
    pub gate_render_confirmed_at: Option<i64>,
    pub gate_views_confirmed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CharacterWorkflowError {
    #[error("角色必须逐步推进，当前 {current:?}，目标 {target:?}")]
    InvalidTransition {
        current: CharacterState,
        target: CharacterState,
    },
    #[error("人工门禁缺少已落盘的输入：{0}")]
    MissingFinalInput(&'static str),
    #[error("该人工门禁已经确认")]
    AlreadyConfirmed,
}

pub fn advance_character(
    current: CharacterState,
    target: CharacterState,
) -> Result<CharacterState, CharacterWorkflowError> {
    if current.next() == Some(target) {
        Ok(target)
    } else {
        Err(CharacterWorkflowError::InvalidTransition { current, target })
    }
}

pub fn agents_for_stage(target_kind: &str, stage: &str) -> &'static [&'static str] {
    match (target_kind, stage) {
        ("project", "project") => &["studio_director", "game_designer"],
        ("character", "spec") => &["studio_director", "spec_writer", "spec_reviewer"],
        ("character", "render") => &[
            "studio_director",
            "prompt_smith",
            "image_t2i",
            "image_i2i",
            "vision_reviewer",
        ],
        ("character", "views") => &[
            "studio_director",
            "prompt_smith",
            "image_i2i",
            "vision_reviewer",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_state_only_advances_one_gate_at_a_time() {
        let states = [
            CharacterState::S0SpecDrafting,
            CharacterState::S1SpecConfirmed,
            CharacterState::S2RenderGenerated,
            CharacterState::S3RenderConfirmed,
            CharacterState::S4ViewsGenerated,
            CharacterState::S5ViewsConfirmed,
        ];
        for transition in states.windows(2) {
            assert_eq!(
                advance_character(transition[0], transition[1]),
                Ok(transition[1])
            );
        }
        assert!(
            advance_character(
                CharacterState::S0SpecDrafting,
                CharacterState::S2RenderGenerated
            )
            .is_err()
        );
        assert!(
            advance_character(
                CharacterState::S3RenderConfirmed,
                CharacterState::S2RenderGenerated
            )
            .is_err()
        );
        assert!(
            advance_character(
                CharacterState::S5ViewsConfirmed,
                CharacterState::S5ViewsConfirmed
            )
            .is_err()
        );
    }
}
