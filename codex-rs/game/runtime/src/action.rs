use codex_game_domain::ACTION_END;
use codex_game_domain::ACTION_START;
use codex_game_domain::AgentAction;
use codex_game_domain::AgentActionKind;
use codex_game_domain::AgentResultStatus;
use codex_game_domain::AgentTurnOutput;
use codex_game_domain::MAX_CHOICE_GROUPS;
use serde::Deserialize;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::Visitor;
use serde_json::Map;
use serde_json::Number;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionProtocolError {
    #[error("每轮必须且只能输出一个完整 Action 块")]
    MissingOrDuplicate,
    #[error("Action 块必须是整条回复的最后内容")]
    NotLast,
    #[error("Action JSON 不合法：{0}")]
    InvalidJson(String),
    #[error("Action reason 必须是非空单句")]
    InvalidReason,
    #[error("{0} 的 target_agent 必须是 null")]
    UnexpectedTarget(&'static str),
    #[error("handoff 必须指定当前阶段允许的其他 Agent")]
    InvalidHandoff,
    #[error("专业 Agent 完成工作后必须 handoff 回总管，不能直接 done")]
    SpecialistMustReturnToDirector,
    #[error("payload.choices 最多四组且每组至少两个不同选项")]
    InvalidChoices,
    #[error("包含 choices 时 action 必须是 ask_user")]
    ChoicesRequireAskUser,
    #[error("payload.result 与 action 不一致")]
    InvalidResult,
    #[error("Action payload 不合法：{0}")]
    InvalidPayload(String),
}

pub fn parse_agent_turn(
    output: &str,
    current_agent: &str,
    director_agent: &str,
    allowed_handoffs: &[String],
) -> Result<AgentTurnOutput, ActionProtocolError> {
    if output.matches(ACTION_START).count() != 1 || output.matches(ACTION_END).count() != 1 {
        return Err(ActionProtocolError::MissingOrDuplicate);
    }
    let start = output
        .find(ACTION_START)
        .ok_or(ActionProtocolError::MissingOrDuplicate)?;
    let body_start = start + ACTION_START.len();
    let end = output[body_start..]
        .find(ACTION_END)
        .map(|offset| body_start + offset)
        .ok_or(ActionProtocolError::MissingOrDuplicate)?;
    if !output[end + ACTION_END.len()..].trim().is_empty() {
        return Err(ActionProtocolError::NotLast);
    }
    let json = output[body_start..end].trim();
    let value = parse_json_without_duplicate_keys(json)?;
    let action: AgentAction = serde_json::from_value(value)
        .map_err(|error| ActionProtocolError::InvalidJson(error.to_string()))?;
    validate_action(&action, current_agent, director_agent, allowed_handoffs)?;
    Ok(AgentTurnOutput {
        text: output[..start].trim_end().to_string(),
        action,
    })
}

fn validate_action(
    action: &AgentAction,
    current_agent: &str,
    director_agent: &str,
    allowed_handoffs: &[String],
) -> Result<(), ActionProtocolError> {
    let reason = action.reason.trim();
    if reason.is_empty() || reason.contains(['\n', '\r']) || sentence_break_before_end(reason) {
        return Err(ActionProtocolError::InvalidReason);
    }

    match action.action {
        AgentActionKind::Handoff => {
            let target = action
                .target_agent
                .as_deref()
                .filter(|target| *target != current_agent)
                .filter(|target| allowed_handoffs.iter().any(|allowed| allowed == *target))
                .ok_or(ActionProtocolError::InvalidHandoff)?;
            if target.is_empty() || (current_agent != director_agent && target != director_agent) {
                return Err(ActionProtocolError::InvalidHandoff);
            }
        }
        AgentActionKind::AskUser => {
            if action.target_agent.is_some() {
                return Err(ActionProtocolError::UnexpectedTarget("ask_user"));
            }
        }
        AgentActionKind::Done => {
            if action.target_agent.is_some() {
                return Err(ActionProtocolError::UnexpectedTarget("done"));
            }
            if current_agent != director_agent {
                return Err(ActionProtocolError::SpecialistMustReturnToDirector);
            }
        }
        AgentActionKind::Blocked => {
            if action.target_agent.is_some() {
                return Err(ActionProtocolError::UnexpectedTarget("blocked"));
            }
        }
    }

    if let Some(groups) = &action.payload.choices {
        if groups.is_empty()
            || groups.len() > MAX_CHOICE_GROUPS
            || groups.iter().any(|group| {
                let options = group
                    .options
                    .iter()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .collect::<HashSet<_>>();
                group.item.trim().is_empty()
                    || options.len() < 2
                    || group
                        .recommended
                        .iter()
                        .any(|value| !options.contains(value.trim()))
                    || (!group.multiple && group.recommended.len() > 1)
            })
        {
            return Err(ActionProtocolError::InvalidChoices);
        }
        if !groups.is_empty() && action.action != AgentActionKind::AskUser {
            return Err(ActionProtocolError::ChoicesRequireAskUser);
        }
    }

    validate_payload(action, current_agent, director_agent)?;
    Ok(())
}

fn validate_payload(
    action: &AgentAction,
    current_agent: &str,
    director_agent: &str,
) -> Result<(), ActionProtocolError> {
    let payload = &action.payload;
    if payload.drafts.as_ref().is_some_and(|drafts| {
        drafts.iter().any(|draft| {
            draft.content.trim().is_empty() || !is_safe_relative_path(&draft.target_path)
        })
    }) {
        return Err(ActionProtocolError::InvalidPayload(
            "drafts 必须包含安全相对路径和非空内容".to_string(),
        ));
    }
    if payload.memories.as_ref().is_some_and(|memories| {
        memories.iter().any(|memory| {
            !matches!(memory.scope.as_str(), "project" | "character")
                || memory.kind.trim().is_empty()
                || memory.content.trim().is_empty()
        })
    }) {
        return Err(ActionProtocolError::InvalidPayload(
            "memories 的 scope、kind 或 content 不合法".to_string(),
        ));
    }
    if payload.naming.as_ref().is_some_and(|items| {
        items.is_empty()
            || items.len() > 3
            || items.iter().any(|item| {
                item.name.trim().is_empty()
                    || item.reason.trim().is_empty()
                    || !is_valid_project_code(&item.code)
            })
    }) {
        return Err(ActionProtocolError::InvalidPayload(
            "naming 必须包含一至三组合法名称与代号".to_string(),
        ));
    }
    if payload.asset_specs.as_ref().is_some_and(|items| {
        items.is_empty()
            || items.iter().any(|item| {
                [
                    &item.code,
                    &item.name,
                    &item.category,
                    &item.size,
                    &item.format,
                    &item.file_name,
                    &item.description,
                    &item.anchors,
                    &item.view_background_color,
                    &item.prompt,
                    &item.negative_prompt,
                ]
                .iter()
                .any(|value| value.trim().is_empty())
            })
    }) {
        return Err(ActionProtocolError::InvalidPayload(
            "asset_specs 缺少必填字段".to_string(),
        ));
    }
    if let Some(verdict) = &payload.verdict
        && (!matches!(verdict.token.as_str(), "SPEC-CHECK" | "VIEW-CHECK")
            || !matches!(verdict.decision.as_str(), "APPROVE" | "CONCERNS" | "REJECT")
            || verdict.sections.is_empty()
            || verdict
                .constraints
                .iter()
                .any(|item| item.item.trim().is_empty() || item.value.trim().is_empty()))
    {
        return Err(ActionProtocolError::InvalidPayload(
            "verdict 结构或枚举值不合法".to_string(),
        ));
    }
    if let Some(result) = &payload.result {
        let expected = match result.status {
            AgentResultStatus::Success if current_agent != director_agent => {
                AgentActionKind::Handoff
            }
            AgentResultStatus::Success => AgentActionKind::Done,
            AgentResultStatus::Failed => AgentActionKind::Blocked,
        };
        if action.action != expected
            || (result.status == AgentResultStatus::Success
                && (result.error.is_some() || result.artifacts.is_empty()))
            || (result.status == AgentResultStatus::Failed
                && (result.error.as_deref().is_none_or(str::is_empty)
                    || !result.artifacts.is_empty()))
        {
            return Err(ActionProtocolError::InvalidResult);
        }
    }

    let requires_ask_user = payload
        .choices
        .as_ref()
        .is_some_and(|items| !items.is_empty())
        || payload
            .drafts
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        || payload
            .naming
            .as_ref()
            .is_some_and(|items| !items.is_empty());
    if requires_ask_user && action.action != AgentActionKind::AskUser {
        return Err(ActionProtocolError::InvalidPayload(
            "choices、drafts 与 naming 只能配合 ask_user".to_string(),
        ));
    }
    if payload.asset_specs.is_some() && action.action != AgentActionKind::Handoff {
        return Err(ActionProtocolError::InvalidPayload(
            "asset_specs 只能配合 handoff".to_string(),
        ));
    }
    let successful_completion_action = if current_agent == director_agent {
        AgentActionKind::Done
    } else {
        AgentActionKind::Handoff
    };
    if payload.verdict.is_some() && action.action != successful_completion_action {
        return Err(ActionProtocolError::InvalidPayload(
            "verdict 必须随完成结果交回总管".to_string(),
        ));
    }
    if payload.naming.is_some() && (payload.choices.is_some() || payload.drafts.is_some()) {
        return Err(ActionProtocolError::InvalidPayload(
            "naming 不能与 choices 或 drafts 同时出现".to_string(),
        ));
    }
    let has_choices = payload
        .choices
        .as_ref()
        .is_some_and(|choices| !choices.is_empty());
    let has_drafts = payload
        .drafts
        .as_ref()
        .is_some_and(|drafts| !drafts.is_empty());
    if has_choices && has_drafts {
        return Err(ActionProtocolError::InvalidPayload(
            "choices 与 drafts 不能在同一轮出现；必须先让用户完成选择，下一轮才能输出 drafts"
                .to_string(),
        ));
    }
    Ok(())
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && !value
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn is_valid_project_code(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    value.len() <= 64
        && !value.starts_with("draft-")
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
}

fn parse_json_without_duplicate_keys(json: &str) -> Result<Value, ActionProtocolError> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let value = UniqueJsonValue::deserialize(&mut deserializer)
        .map_err(|error| ActionProtocolError::InvalidJson(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| ActionProtocolError::InvalidJson(error.to_string()))?;
    Ok(value.0)
}

struct UniqueJsonValue(Value);

impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("不含重复字段的 JSON")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJsonValue)
            .ok_or_else(|| E::custom("JSON 数字必须是有限值"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.0);
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some((key, value)) = entries.next_entry::<String, UniqueJsonValue>()? {
            if values.insert(key.clone(), value.0).is_some() {
                return Err(serde::de::Error::custom(format!("重复字段：{key}")));
            }
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}

fn sentence_break_before_end(reason: &str) -> bool {
    let mut chars = reason.chars().peekable();
    while let Some(character) = chars.next() {
        if matches!(character, '。' | '！' | '？' | '!' | '?')
            && chars.clone().any(|rest| !rest.is_whitespace())
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn output(action: Value) -> String {
        format!("正文\n{ACTION_START}\n{action}\n{ACTION_END}")
    }

    #[test]
    fn accepts_one_trailing_action_and_strips_it_from_display_text() {
        let parsed = parse_agent_turn(
            &output(json!({
                "action": "ask_user",
                "target_agent": null,
                "reason": "需要用户选择",
                "payload": {
                    "choices": [{
                        "item": "风格",
                        "options": ["写实", "卡通"],
                        "recommended": ["卡通"],
                        "multiple": false
                    }]
                }
            })),
            "game_designer",
            "studio_director",
            &["studio_director".to_string()],
        )
        .expect("valid action");

        assert_eq!(parsed.text, "正文");
        assert_eq!(parsed.action.action, AgentActionKind::AskUser);
    }

    #[test]
    fn rejects_duplicate_non_trailing_and_unknown_action_fields() {
        let valid = output(json!({
            "action": "done",
            "target_agent": null,
            "reason": "任务完成",
            "payload": {}
        }));
        assert_eq!(
            parse_agent_turn(
                &format!("{valid}\n{valid}"),
                "game_designer",
                "studio_director",
                &[],
            ),
            Err(ActionProtocolError::MissingOrDuplicate)
        );
        assert_eq!(
            parse_agent_turn(
                &format!("{valid}\n额外内容"),
                "game_designer",
                "studio_director",
                &[],
            ),
            Err(ActionProtocolError::NotLast)
        );
        let unknown = output(json!({
            "action": "done",
            "target_agent": null,
            "reason": "任务完成",
            "payload": {},
            "extra": true
        }));
        assert!(matches!(
            parse_agent_turn(&unknown, "game_designer", "studio_director", &[]),
            Err(ActionProtocolError::InvalidJson(_))
        ));
    }

    #[test]
    fn enforces_handoff_allowlist_and_choice_contract() {
        let handoff = output(json!({
            "action": "handoff",
            "target_agent": "spec_writer",
            "reason": "需要编写设定",
            "payload": {}
        }));
        assert_eq!(
            parse_agent_turn(&handoff, "studio_director", "studio_director", &[]),
            Err(ActionProtocolError::InvalidHandoff)
        );
        assert!(
            parse_agent_turn(
                &handoff,
                "studio_director",
                "studio_director",
                &["spec_writer".to_string()],
            )
            .is_ok()
        );

        let empty_choices = output(json!({
            "action": "ask_user",
            "target_agent": null,
            "reason": "需要用户选择",
            "payload": { "choices": [] }
        }));
        assert_eq!(
            parse_agent_turn(&empty_choices, "game_designer", "studio_director", &[],),
            Err(ActionProtocolError::InvalidChoices)
        );
    }

    #[test]
    fn specialists_must_return_control_to_the_director() {
        let done = output(json!({
            "action": "done",
            "target_agent": null,
            "reason": "设定编写完成",
            "payload": {}
        }));
        assert_eq!(
            parse_agent_turn(
                &done,
                "spec_writer",
                "studio_director",
                &["studio_director".to_string()],
            ),
            Err(ActionProtocolError::SpecialistMustReturnToDirector)
        );

        let direct_handoff = output(json!({
            "action": "handoff",
            "target_agent": "spec_reviewer",
            "reason": "交给审校继续处理",
            "payload": {}
        }));
        assert_eq!(
            parse_agent_turn(
                &direct_handoff,
                "spec_writer",
                "studio_director",
                &["studio_director".to_string(), "spec_reviewer".to_string()],
            ),
            Err(ActionProtocolError::InvalidHandoff)
        );

        let return_to_director = output(json!({
            "action": "handoff",
            "target_agent": "studio_director",
            "reason": "设定编写完成，交回总管决定下一步",
            "payload": {}
        }));
        assert!(
            parse_agent_turn(
                &return_to_director,
                "spec_writer",
                "studio_director",
                &["studio_director".to_string()],
            )
            .is_ok()
        );
    }

    #[test]
    fn enforces_choice_and_draft_sequence() {
        let choice_with_empty_drafts = output(json!({
            "action": "ask_user",
            "target_agent": null,
            "reason": "需要用户确认角色设定",
            "payload": {
                "choices": [{
                    "item": "主色",
                    "options": ["朱砂红", "深靛蓝"],
                    "recommended": ["朱砂红"],
                    "multiple": false
                }],
                "drafts": []
            }
        }));
        assert!(
            parse_agent_turn(
                &choice_with_empty_drafts,
                "spec_writer",
                "studio_director",
                &[],
            )
            .is_ok()
        );

        let mixed_interaction = output(json!({
            "action": "ask_user",
            "target_agent": null,
            "reason": "需要用户确认角色设定",
            "payload": {
                "choices": [{
                    "item": "主色",
                    "options": ["朱砂红", "深靛蓝"],
                    "recommended": ["朱砂红"],
                    "multiple": false
                }],
                "drafts": [{
                    "target_path": "docs/角色定稿.md",
                    "content": "# 角色设定"
                }]
            }
        }));

        assert_eq!(
            parse_agent_turn(&mixed_interaction, "spec_writer", "studio_director", &[],),
            Err(ActionProtocolError::InvalidPayload(
                "choices 与 drafts 不能在同一轮出现；必须先让用户完成选择，下一轮才能输出 drafts"
                    .to_string()
            ))
        );
    }

    #[test]
    fn enforces_result_status_and_artifact_contract() {
        let invalid_success = output(json!({
            "action": "handoff",
            "target_agent": "studio_director",
            "reason": "图片生成完成，交回总管",
            "payload": {
                "result": { "status": "success", "artifacts": [], "error": null }
            }
        }));
        assert_eq!(
            parse_agent_turn(
                &invalid_success,
                "image_t2i",
                "studio_director",
                &["studio_director".to_string()],
            ),
            Err(ActionProtocolError::InvalidResult)
        );

        let invalid_failure = output(json!({
            "action": "blocked",
            "target_agent": null,
            "reason": "缺少图片执行器",
            "payload": {
                "result": { "status": "failed", "artifacts": [], "error": null }
            }
        }));
        assert_eq!(
            parse_agent_turn(&invalid_failure, "image_t2i", "studio_director", &[],),
            Err(ActionProtocolError::InvalidResult)
        );
    }
}
