use codex_game_domain::ACTION_END;
use codex_game_domain::ACTION_START;
use codex_game_domain::AgentAction;
use codex_game_domain::AgentActionKind;
use codex_game_domain::AgentActionPayload;
use codex_game_domain::AgentResultStatus;
use schemars::schema_for;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionContractProfile {
    pub instruction: String,
    pub schema: String,
    pub examples: String,
}

pub fn action_contract_profile(
    agent_code: &str,
    stage: &str,
    director_agent: &str,
    allowed_handoffs: &[String],
    project_task: Option<&str>,
) -> ActionContractProfile {
    let schema = serde_json::to_string_pretty(&profile_schema(
        agent_code,
        stage,
        director_agent,
        allowed_handoffs,
        project_task,
    ))
    .unwrap_or_else(|_| "{}".to_string());
    let instruction = instruction(agent_code, stage, director_agent, allowed_handoffs);
    let examples = examples(
        agent_code,
        stage,
        director_agent,
        allowed_handoffs,
        project_task,
    );
    ActionContractProfile {
        instruction,
        schema,
        examples,
    }
}

pub fn validate_action_profile(
    agent_code: &str,
    director_agent: &str,
    allowed_handoffs: &[String],
    action: &AgentAction,
) -> Result<(), String> {
    let payload = &action.payload;
    let valid = match agent_code {
        "studio_director" => {
            match action.action {
                AgentActionKind::Handoff => {
                    action.target_agent.as_deref().is_some_and(|target| {
                        allowed_handoffs.iter().any(|allowed| allowed == target)
                    }) && payload_is_empty(payload)
                }
                AgentActionKind::AskUser => payload_has_only_choices(payload),
                AgentActionKind::Done | AgentActionKind::Blocked => payload_is_empty(payload),
            }
        }
        "spec_writer" => match action.action {
            AgentActionKind::AskUser => payload_has_only_choices(payload),
            AgentActionKind::Done => payload_has_only_drafts(payload),
            AgentActionKind::Blocked => payload_is_empty(payload),
            AgentActionKind::Handoff => false,
        },
        "spec_reviewer" => match action.action {
            AgentActionKind::Done => payload.verdict.is_some() && payload_has_only_verdict(payload),
            AgentActionKind::Blocked => payload_is_empty(payload),
            AgentActionKind::AskUser | AgentActionKind::Handoff => false,
        },
        "visual_designer" => match action.action {
            AgentActionKind::Done => payload.result.as_ref().is_some_and(|result| {
                result.status == AgentResultStatus::Success && payload_has_only_result(payload)
            }),
            AgentActionKind::Blocked => payload.result.as_ref().is_some_and(|result| {
                result.status == AgentResultStatus::Failed && payload_has_only_result(payload)
            }),
            AgentActionKind::AskUser => payload_has_only_choices(payload),
            AgentActionKind::Handoff => false,
        },
        _ => {
            return Ok(());
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "当前 Agent {agent_code} 的 action/payload 组合不属于本轮 ActionContractProfile；director={director_agent}"
        ))
    }
}

fn payload_is_empty(payload: &AgentActionPayload) -> bool {
    payload.choices.is_none()
        && payload.progress.is_none()
        && payload.drafts.is_none()
        && payload.memories.is_none()
        && payload.naming.is_none()
        && payload.asset_specs.is_none()
        && payload.verdict.is_none()
        && payload.result.is_none()
}

fn payload_has_only_choices(payload: &AgentActionPayload) -> bool {
    payload
        .choices
        .as_ref()
        .is_some_and(|items| !items.is_empty())
        && payload.drafts.is_none()
        && payload.memories.is_none()
        && payload.naming.is_none()
        && payload.asset_specs.is_none()
        && payload.verdict.is_none()
        && payload.result.is_none()
}

fn payload_has_only_drafts(payload: &AgentActionPayload) -> bool {
    payload
        .drafts
        .as_ref()
        .is_some_and(|items| !items.is_empty())
        && payload.choices.is_none()
        && payload.memories.is_none()
        && payload.naming.is_none()
        && payload.asset_specs.is_none()
        && payload.verdict.is_none()
        && payload.result.is_none()
}

fn payload_has_only_verdict(payload: &AgentActionPayload) -> bool {
    payload.choices.is_none()
        && payload.progress.is_none()
        && payload.drafts.is_none()
        && payload.memories.is_none()
        && payload.naming.is_none()
        && payload.asset_specs.is_none()
        && payload.result.is_none()
}

fn payload_has_only_result(payload: &AgentActionPayload) -> bool {
    payload.choices.is_none()
        && payload.progress.is_none()
        && payload.drafts.is_none()
        && payload.memories.is_none()
        && payload.naming.is_none()
        && payload.asset_specs.is_none()
        && payload.verdict.is_none()
}

pub fn render_action_contract(
    instruction: &str,
    schema: &str,
    examples: &str,
    version: u64,
) -> String {
    format!(
        "<action_contract version=\"{version}\">\n{instruction}\n\nJSON Schema：\n{schema}\n\n本轮合法完整示例：\n{examples}\n</action_contract>"
    )
}

fn profile_schema(
    agent_code: &str,
    stage: &str,
    _director_agent: &str,
    allowed_handoffs: &[String],
    project_task: Option<&str>,
) -> Value {
    let generated = serde_json::to_value(schema_for!(AgentAction))
        .unwrap_or_else(|_| json!({"definitions": {}}));
    let definitions = filtered_definitions(&generated, agent_code);
    let branches = match (agent_code, project_task) {
        ("studio_director", Some("draftProjectArtBible" | "collectProjectNaming")) => {
            director_handoff_schema_branches(allowed_handoffs)
        }
        ("studio_director", Some("confirmArtBible" | "completed")) => vec![
            action_branch("done", json!({"type": "null"}), empty_payload()),
            action_branch("blocked", json!({"type": "null"}), empty_payload()),
        ],
        ("art_bible_designer", Some("collectProjectNaming")) => vec![
            action_branch("ask_user", json!({"type": "null"}), choices_payload()),
            action_branch("blocked", json!({"type": "null"}), empty_payload()),
        ],
        ("studio_director", _) => director_schema_branches(allowed_handoffs),
        ("spec_writer", _) => vec![
            action_branch("ask_user", json!({"type": "null"}), choices_payload()),
            action_branch("done", json!({"type": "null"}), drafts_payload()),
            action_branch("blocked", json!({"type": "null"}), empty_payload()),
        ],
        ("spec_reviewer", _) => vec![
            action_branch("done", json!({"type": "null"}), verdict_payload()),
            action_branch("blocked", json!({"type": "null"}), empty_payload()),
        ],
        ("visual_designer", _) => vec![
            action_branch(
                "done",
                json!({"type": "null"}),
                visual_result_payload("success", Some(stage)),
            ),
            action_branch(
                "blocked",
                json!({"type": "null"}),
                visual_result_payload("failed", None),
            ),
            action_branch("ask_user", json!({"type": "null"}), choices_payload()),
        ],
        (_, _) => return generated,
    };
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": definitions,
        "oneOf": branches
    })
}

fn filtered_definitions(generated: &Value, agent_code: &str) -> Value {
    let names: &[&str] = match agent_code {
        "studio_director" => &["ChoiceGroup"],
        "spec_writer" => &["AgentProgress", "ChoiceGroup"],
        "spec_reviewer" => &[
            "AgentVerdict",
            "VerdictConstraint",
            "VerdictConstraintScope",
        ],
        "visual_designer" => &["AgentProgress", "ChoiceGroup"],
        _ => {
            return generated
                .get("definitions")
                .cloned()
                .unwrap_or_else(|| json!({}));
        }
    };
    let generated = generated.get("definitions").and_then(Value::as_object);
    let mut selected = Map::new();
    for name in names {
        if let Some(schema) = generated.and_then(|definitions| definitions.get(*name)) {
            selected.insert((*name).to_string(), schema.clone());
        }
    }
    Value::Object(selected)
}

fn director_handoff_schema_branches(allowed_handoffs: &[String]) -> Vec<Value> {
    let mut branches = allowed_handoffs
        .first()
        .map(|_| {
            vec![action_branch(
                "handoff",
                json!({"type": "string", "enum": allowed_handoffs}),
                empty_payload(),
            )]
        })
        .unwrap_or_default();
    branches.push(action_branch(
        "blocked",
        json!({"type": "null"}),
        empty_payload(),
    ));
    branches
}

fn director_schema_branches(allowed_handoffs: &[String]) -> Vec<Value> {
    let mut branches = Vec::new();
    if !allowed_handoffs.is_empty() {
        branches.push(action_branch(
            "handoff",
            json!({"type": "string", "enum": allowed_handoffs}),
            empty_payload(),
        ));
    }
    branches.push(action_branch(
        "ask_user",
        json!({"type": "null"}),
        choices_payload(),
    ));
    branches.push(action_branch(
        "done",
        json!({"type": "null"}),
        empty_payload(),
    ));
    branches.push(action_branch(
        "blocked",
        json!({"type": "null"}),
        empty_payload(),
    ));
    branches
}

fn action_branch(action: &str, target_agent: Value, payload: Value) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["action", "target_agent", "reason", "payload"],
        "properties": {
            "action": {"const": action},
            "target_agent": target_agent,
            "reason": {"type": "string", "minLength": 1},
            "payload": payload
        }
    })
}

fn empty_payload() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

fn choices_payload() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["choices"],
        "properties": {
            "choices": {
                "type": "array",
                "minItems": 1,
                "maxItems": 4,
                "items": {"$ref": "#/definitions/ChoiceGroup"}
            },
            "progress": {"$ref": "#/definitions/AgentProgress"}
        }
    })
}

fn drafts_payload() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["drafts"],
        "properties": {
            "drafts": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["artifact_slot", "content"],
                    "properties": {
                        "artifact_slot": {"const": "character_spec"},
                        "content": {"type": "string", "minLength": 1},
                        "based_on_hash": {"type": ["string", "null"]}
                    }
                }
            },
            "progress": {"$ref": "#/definitions/AgentProgress"}
        }
    })
}

fn verdict_payload() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdict"],
        "properties": {
            "verdict": {"$ref": "#/definitions/AgentVerdict"}
        }
    })
}

fn visual_result_payload(status: &str, stage: Option<&str>) -> Value {
    let result = match stage {
        Some(stage) => json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["status", "error", "revision_scope", "focus_changes_summary"],
            "properties": {
                "status": {"const": status},
                "error": {"type": "null"},
                "revision_scope": {
                    "type": "string",
                    "enum": if stage == "views" { vec!["views", "render"] } else { vec!["render"] }
                },
                "focus_changes_summary": {"type": "string", "minLength": 1}
            }
        }),
        None => json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["status", "error"],
            "properties": {
                "status": {"const": status},
                "error": {"type": "string", "minLength": 1},
                "revision_scope": {"type": ["string", "null"], "enum": ["render", "views", null]},
                "focus_changes_summary": {"type": ["string", "null"]}
            }
        }),
    };
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["result"],
        "properties": {"result": result}
    })
}

fn instruction(
    agent_code: &str,
    stage: &str,
    _director_agent: &str,
    allowed_handoffs: &[String],
) -> String {
    let targets = if allowed_handoffs.is_empty() {
        "无；本轮不得输出 handoff".to_string()
    } else {
        allowed_handoffs.join("、")
    };
    let payload_rule = match agent_code {
        "studio_director" => {
            "handoff 的 payload 必须为 {}；ask_user 的 choices[] 每项必须且只能包含 item、options、recommended、multiple。"
        }
        "spec_writer" => {
            "对焦时只输出 ask_user + choices；草稿完成时只输出 done + drafts，drafts[] 必须且只能包含 artifact_slot、content、based_on_hash，角色规格的 artifact_slot 固定为 character_spec。"
        }
        "spec_reviewer" => {
            "成功审校只输出 done + verdict；verdict 必须包含 token、subject_id、decision、sections、constraints，后续确认或回派由系统执行。"
        }
        "visual_designer" => {
            "生成成功只输出 done + result；result 必须包含 status、error、revision_scope、focus_changes_summary，禁止返回 path、references、executor、prompt 或其他产物路径字段；后续确认门禁由系统创建。"
        }
        _ => "只使用 schema 中定义的字段；不需要的 payload 字段必须省略。",
    };
    format!(
        "当前 Agent：{agent_code}；当前阶段：{stage}；允许 handoff 目标：{targets}。\n请先阅读 game_context.agentRoleDescriptions，按其中的角色说明与职责边界协作；只有 handoffAllowed 为 true 的 Agent 才能作为 handoff 目标，internalExecutor 仅供系统内部执行。项目会话还必须严格遵守 game_context.projectWorkflowContext 和 game_context.currentTask；不得跳步、重做已完成步骤或替 currentTask.ownerAgent 产出专业内容。\n每次回复末尾必须且只能有一个 Action 块，Action 块后不得再输出内容。顶层必须且只能包含 action、target_agent、reason、payload。{payload_rule}运行时仍会执行严格字段、组合和业务校验。"
    )
}

fn examples(
    agent_code: &str,
    stage: &str,
    director_agent: &str,
    allowed_handoffs: &[String],
    project_task: Option<&str>,
) -> String {
    let values = match (agent_code, project_task) {
        ("studio_director", Some("draftProjectArtBible" | "collectProjectNaming")) => {
            director_handoff_examples(allowed_handoffs)
        }
        ("studio_director", Some("confirmArtBible" | "completed")) => vec![json!({
            "action": "done",
            "target_agent": null,
            "reason": "当前项目状态已说明完毕",
            "payload": {}
        })],
        ("art_bible_designer", Some("collectProjectNaming")) => project_naming_examples(),
        ("studio_director", _) => director_examples(allowed_handoffs),
        ("spec_writer", _) => spec_writer_examples(),
        ("spec_reviewer", _) => vec![json!({
            "action": "done",
            "target_agent": null,
            "reason": "规格审校已完成",
            "payload": {
                "verdict": {
                    "token": "SPEC-CHECK",
                    "subject_id": "<reviewSubject.id>",
                    "decision": "APPROVE",
                    "sections": { "缺失维度": [] },
                    "constraints": [{ "scope": "identity", "item": "头身比", "value": "7 头身" }]
                }
            }
        })],
        ("visual_designer", _) => vec![
            json!({
                "action": "done",
                "target_agent": null,
                "reason": "视觉候选已生成并交由系统登记",
                "payload": {
                    "result": {
                        "status": "success",
                        "error": null,
                        "revision_scope": if stage == "views" { "views" } else { "render" },
                        "focus_changes_summary": "已按当前反馈更新对焦设定"
                    }
                }
            }),
            json!({
                "action": "blocked",
                "target_agent": null,
                "reason": "缺少生成所需的前置条件",
                "payload": {
                    "result": {
                        "status": "failed",
                        "error": "缺少生成所需的前置条件",
                        "revision_scope": null,
                        "focus_changes_summary": null
                    }
                }
            }),
        ],
        _ => generic_examples(director_agent, allowed_handoffs),
    };
    values
        .into_iter()
        .map(action_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn director_handoff_examples(allowed_handoffs: &[String]) -> Vec<Value> {
    allowed_handoffs
        .first()
        .map(|target| {
            vec![json!({
                "action": "handoff",
                "target_agent": target,
                "reason": "执行当前项目状态要求的唯一专业任务",
                "payload": {}
            })]
        })
        .unwrap_or_else(|| {
            vec![json!({
                "action": "blocked",
                "target_agent": null,
                "reason": "当前项目任务没有可用的专业 Agent",
                "payload": {}
            })]
        })
}

fn project_naming_examples() -> Vec<Value> {
    vec![json!({
        "action": "ask_user",
        "target_agent": null,
        "reason": "请选择一组对应的项目名称与项目代号",
        "payload": {
            "choices": [
                {
                    "item": "项目名称",
                    "options": ["项目名一", "项目名二"],
                    "recommended": ["项目名一"],
                    "multiple": false
                },
                {
                    "item": "项目代号",
                    "options": ["project-one", "project-two"],
                    "recommended": ["project-one"],
                    "multiple": false
                }
            ]
        }
    })]
}

fn director_examples(allowed_handoffs: &[String]) -> Vec<Value> {
    let mut examples = Vec::new();
    if let Some(target) = allowed_handoffs.first() {
        examples.push(json!({
            "action": "handoff",
            "target_agent": target,
            "reason": "该专业任务应由此 Agent 承接",
            "payload": {}
        }));
    }
    examples.push(json!({
        "action": "ask_user",
        "target_agent": null,
        "reason": "需要用户确认关键选项",
        "payload": {
            "choices": [{
                "item": "需要确认的事项",
                "options": ["方案 A", "方案 B"],
                "recommended": ["方案 A"],
                "multiple": false
            }]
        }
    }));
    examples.push(json!({
        "action": "done",
        "target_agent": null,
        "reason": "当前状态已说明完毕",
        "payload": {}
    }));
    examples
}

fn spec_writer_examples() -> Vec<Value> {
    vec![
        json!({
            "action": "ask_user",
            "target_agent": null,
            "reason": "需要用户确认角色外观",
            "payload": {
                "choices": [{
                    "item": "角色外观",
                    "options": ["方案 A", "方案 B"],
                    "recommended": ["方案 A"],
                    "multiple": false
                }],
                "progress": {
                    "decisions": [],
                    "open_questions": ["角色外观"],
                    "next_step": "确认后生成角色规格草稿"
                }
            }
        }),
        json!({
            "action": "done",
            "target_agent": null,
            "reason": "角色规格草稿已生成并提交系统审校",
            "payload": {
                "drafts": [{
                    "artifact_slot": "character_spec",
                    "content": "# 角色定稿\n...",
                    "based_on_hash": null
                }],
                "progress": {
                    "decisions": ["角色规格已完整"],
                    "open_questions": [],
                    "next_step": "等待用户确认草稿"
                }
            }
        }),
    ]
}

fn generic_examples(director_agent: &str, allowed_handoffs: &[String]) -> Vec<Value> {
    let target = allowed_handoffs
        .first()
        .map(String::as_str)
        .unwrap_or(director_agent);
    vec![json!({
        "action": "handoff",
        "target_agent": target,
        "reason": "当前专业任务已完成",
        "payload": {}
    })]
}

fn action_block(value: Value) -> String {
    let json = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string());
    format!("{ACTION_START}\n{json}\n{ACTION_END}")
}

#[cfg(test)]
#[path = "action_contract_tests.rs"]
mod tests;
