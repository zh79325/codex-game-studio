use codex_app_server_protocol::GameAgentHandoffNotification;
use codex_app_server_protocol::GameAttemptUpdatedNotification;
use codex_app_server_protocol::GameCharacterUpdatedNotification;
use codex_app_server_protocol::GameConversationActorNotification;
use codex_app_server_protocol::GameConversationDeltaNotification;
use codex_app_server_protocol::GameConversationErrorNotification;
use codex_app_server_protocol::GameConversationFocusNotification;
use codex_app_server_protocol::GameConversationTurnNotification;
use codex_app_server_protocol::GameGenerationUpdatedNotification;
use codex_app_server_protocol::GameTaskUpdatedNotification;
use codex_app_server_protocol::ServerNotification;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_game_app_server_adapter::GameTurnEventContext;
use codex_game_runtime::TurnAuditCompletion;
use codex_game_runtime::TurnAuditEvent;
use codex_game_runtime::TurnAuditUsage;
use codex_game_runtime::append_turn_audit_completion;
use codex_http_client::unregister_stream_response_audit;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::game_execution_port::AppServerCodexExecutionPort;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingMessageSender;

#[derive(Debug, Clone)]
pub(crate) struct GameThreadEvent {
    pub(crate) thread_id: ThreadId,
    pub(crate) connection_ids: Vec<ConnectionId>,
    pub(crate) event: Event,
}

pub(crate) type GameEventSender = mpsc::UnboundedSender<GameThreadEvent>;
pub(crate) type GameEventReceiver = mpsc::UnboundedReceiver<GameThreadEvent>;

pub(crate) fn game_event_channel() -> (GameEventSender, GameEventReceiver) {
    mpsc::unbounded_channel()
}

pub(crate) fn spawn_game_event_observer(
    mut receiver: GameEventReceiver,
    adapter: Arc<GameAppServerAdapter>,
    execution: Arc<AppServerCodexExecutionPort>,
    outgoing: Arc<OutgoingMessageSender>,
) {
    tokio::spawn(async move {
        let mut active_game_turns = HashMap::<ThreadId, String>::new();
        let mut usage_by_thread = HashMap::<ThreadId, TurnAuditUsage>::new();
        let mut partial_output_by_turn = HashMap::<String, String>::new();
        let mut first_output_turns = HashSet::<String>::new();
        while let Some(observed) = receiver.recv().await {
            if let EventMsg::TurnStarted(started) = &observed.event.msg {
                if matches!(
                    adapter.turn_event_context(&started.turn_id).await,
                    Ok(Some(_))
                ) {
                    active_game_turns.insert(observed.thread_id, started.turn_id.clone());
                    partial_output_by_turn.insert(started.turn_id.clone(), String::new());
                    record_turn_event(
                        adapter.as_ref(),
                        &started.turn_id,
                        "ai_turn_started",
                        "started",
                        "AI 已开始处理本轮请求",
                        serde_json::json!({}),
                    )
                    .await;
                }
                continue;
            }
            if let EventMsg::TokenCount(token_count) = &observed.event.msg {
                if active_game_turns.contains_key(&observed.thread_id)
                    && let Some(info) = &token_count.info
                {
                    let usage = &info.last_token_usage;
                    usage_by_thread.insert(
                        observed.thread_id,
                        TurnAuditUsage {
                            input_tokens: usage.input_tokens,
                            cached_input_tokens: usage.cached_input_tokens,
                            output_tokens: usage.output_tokens,
                            reasoning_output_tokens: usage.reasoning_output_tokens,
                            total_tokens: usage.total_tokens,
                        },
                    );
                }
                continue;
            }
            if let EventMsg::AgentMessageContentDelta(delta) = &observed.event.msg {
                match adapter.turn_event_context(&delta.turn_id).await {
                    Ok(Some(context)) => {
                        active_game_turns.insert(observed.thread_id, delta.turn_id.clone());
                        append_bounded_partial(
                            partial_output_by_turn
                                .entry(delta.turn_id.clone())
                                .or_default(),
                            &delta.delta,
                        );
                        if first_output_turns.insert(delta.turn_id.clone()) {
                            record_turn_event(
                                adapter.as_ref(),
                                &delta.turn_id,
                                "ai_first_output",
                                "succeeded",
                                "AI 已返回首段输出",
                                serde_json::json!({}),
                            )
                            .await;
                        }
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationDelta(
                                GameConversationDeltaNotification {
                                    conversation_id: context.conversation_id,
                                    turn_id: delta.turn_id.clone(),
                                    agent_code: context.agent_code,
                                    delta: delta.delta.clone(),
                                },
                            ))
                            .await;
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(
                        thread_id = %observed.thread_id,
                        turn_id = %delta.turn_id,
                        "failed to resolve game delta context: {error}"
                    ),
                }
                continue;
            }
            let lifecycle = match &observed.event.msg {
                EventMsg::ImageGenerationBegin(event) => active_game_turns
                    .get(&observed.thread_id)
                    .cloned()
                    .map(|turn_id| {
                        (
                            turn_id,
                            "image_generation_started",
                            "started",
                            "图片生成请求已开始".to_string(),
                            serde_json::json!({
                                "callId": event.call_id.as_str(),
                                "tool": event.tool_name.as_deref(),
                            }),
                        )
                    }),
                EventMsg::ImageGenerationEnd(event) => active_game_turns
                    .get(&observed.thread_id)
                    .cloned()
                    .map(|turn_id| {
                        let succeeded = event.status == "completed" && event.saved_path.is_some();
                        let event_name = if succeeded {
                            "image_generation_succeeded"
                        } else {
                            "image_generation_failed"
                        };
                        (
                            turn_id,
                            event_name,
                            if succeeded { "succeeded" } else { "failed" },
                            if succeeded {
                                "AI 合图已返回并保存".to_string()
                            } else {
                                "AI 合图失败或未保存".to_string()
                            },
                            serde_json::json!({
                                "callId": event.call_id.as_str(),
                                "tool": event.tool_name.as_deref(),
                                "status": event.status.as_str(),
                                "savedPath": event.saved_path.as_ref().map(|path| path.as_path().display().to_string()),
                                "prompt": event.revised_prompt.as_deref().map(|prompt| truncate_text(prompt, 2048)),
                                "resultBytes": event.result.len(),
                                "failure": event.failure,
                            }),
                        )
                    }),
                EventMsg::ExecCommandBegin(event) => Some((
                    event.turn_id.clone(),
                    "tool_started",
                    "started",
                    "命令工具已开始".to_string(),
                    serde_json::json!({
                        "callId": event.call_id,
                        "tool": "exec_command",
                        "command": truncate_text(&event.command.join(" "), 2048),
                    }),
                )),
                EventMsg::ExecCommandEnd(event) => Some((
                    event.turn_id.clone(),
                    if event.exit_code == 0 {
                        "tool_succeeded"
                    } else {
                        "tool_failed"
                    },
                    if event.exit_code == 0 {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    "命令工具已结束".to_string(),
                    serde_json::json!({
                        "callId": event.call_id,
                        "tool": "exec_command",
                        "exitCode": event.exit_code,
                        "output": truncate_text(&event.aggregated_output, 4096),
                    }),
                )),
                EventMsg::ExecApprovalRequest(event) => Some((
                    event.turn_id.clone(),
                    "approval_requested",
                    "waiting",
                    "命令工具正在等待权限确认".to_string(),
                    serde_json::json!({
                        "callId": event.call_id,
                        "command": truncate_text(&event.command.join(" "), 2048),
                        "reason": event.reason,
                    }),
                )),
                _ => None,
            };
            if let Some((turn_id, event, status, message, payload)) = lifecycle {
                record_turn_event(adapter.as_ref(), &turn_id, event, status, &message, payload)
                    .await;
                continue;
            }
            let terminal_error = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => {
                    completed.error.as_ref().map(|error| error.message.clone())
                }
                _ => None,
            };
            let turn_id = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => completed.turn_id.as_str(),
                EventMsg::TurnAborted(aborted) => {
                    aborted.turn_id.as_deref().unwrap_or(&observed.event.id)
                }
                _ => continue,
            };
            active_game_turns.remove(&observed.thread_id);
            unregister_stream_response_audit(&observed.thread_id.to_string());
            let event_context = adapter.turn_event_context(turn_id).await.ok().flatten();
            let audit_context = adapter.turn_audit_context(turn_id).await.ok().flatten();
            let partial_output = partial_output_by_turn.remove(turn_id);
            first_output_turns.remove(turn_id);
            record_turn_event(
                adapter.as_ref(),
                turn_id,
                if matches!(observed.event.msg, EventMsg::TurnAborted(_)) {
                    "ai_turn_aborted"
                } else if terminal_error.is_some() {
                    "ai_turn_failed"
                } else {
                    "ai_turn_completed"
                },
                if terminal_error.is_some()
                    || matches!(observed.event.msg, EventMsg::TurnAborted(_))
                {
                    "failed"
                } else {
                    "succeeded"
                },
                terminal_error.as_deref().unwrap_or("AI 本轮处理已结束"),
                serde_json::json!({}),
            )
            .await;
            if let Some(context) = audit_context.as_ref() {
                let completion = match &observed.event.msg {
                    EventMsg::TurnComplete(completed) => TurnAuditCompletion {
                        response: completed.last_agent_message.clone(),
                        error: terminal_error.clone(),
                        usage: usage_by_thread.remove(&observed.thread_id),
                        duration_ms: completed.duration_ms,
                        time_to_first_token_ms: completed.time_to_first_token_ms,
                    },
                    EventMsg::TurnAborted(aborted) => TurnAuditCompletion {
                        error: Some(format!("运行已中断：{:?}", aborted.reason)),
                        usage: usage_by_thread.remove(&observed.thread_id),
                        duration_ms: aborted.duration_ms,
                        ..TurnAuditCompletion::default()
                    },
                    _ => unreachable!(),
                };
                if let Err(error) = append_turn_audit_completion(context, &completion) {
                    tracing::warn!(
                        thread_id = %observed.thread_id,
                        turn_id,
                        "failed to write game turn audit completion: {error}"
                    );
                }
            }
            let result = match &observed.event.msg {
                EventMsg::TurnComplete(completed) => {
                    adapter
                        .observe_turn_completed(
                            execution.as_ref(),
                            &completed.turn_id,
                            completed.last_agent_message.as_deref(),
                            completed.error.as_ref().map(|error| error.message.as_str()),
                        )
                        .await
                }
                EventMsg::TurnAborted(_) => {
                    adapter
                        .observe_turn_aborted(turn_id, partial_output.as_deref())
                        .await
                }
                _ => unreachable!(),
            };
            match result {
                Ok(Some(projection)) => {
                    let conversation_id = projection.conversation_id.clone();
                    let notification_turn_id = projection
                        .turn_id
                        .clone()
                        .unwrap_or_else(|| turn_id.to_string());
                    let is_running = projection.status == "running";
                    let director_resume_reason = projection.director_resume_reason.clone();
                    if let Some(context) = event_context.as_ref() {
                        record_task_event(
                            adapter.as_ref(),
                            context,
                            audit_context.as_ref(),
                            "attempt_terminal",
                            if matches!(observed.event.msg, EventMsg::TurnAborted(_)) {
                                "interrupted"
                            } else if terminal_error.is_some() {
                                "failed"
                            } else {
                                "succeeded"
                            },
                            "任务尝试已进入终态",
                            serde_json::json!({ "projectionStatus": projection.status.as_str() }),
                        )
                        .await;
                    }
                    if let Some(context) = event_context.as_ref() {
                        let (action, target) =
                            if let Some(target) = projection.handoff_target.as_deref() {
                                ("handoff", Some(target))
                            } else if director_resume_reason.is_some() {
                                ("resume_director", Some("studio_director"))
                            } else {
                                ("none", None)
                            };
                        record_task_event(
                            adapter.as_ref(),
                            context,
                            audit_context.as_ref(),
                            "next_action_selected",
                            "succeeded",
                            "已根据本轮结果选择后续动作",
                            serde_json::json!({ "action": action, "targetAgent": target }),
                        )
                        .await;
                    }
                    outgoing
                        .send_server_notification(ServerNotification::GameAttemptUpdated(
                            GameAttemptUpdatedNotification {
                                conversation_id: conversation_id.clone(),
                                task_id: projection.task_id.clone(),
                                attempt_id: projection.attempt_id,
                                turn_id: Some(notification_turn_id.clone()),
                                status: projection.status.clone(),
                            },
                        ))
                        .await;
                    outgoing
                        .send_server_notification(ServerNotification::GameTaskUpdated(
                            GameTaskUpdatedNotification {
                                conversation_id: conversation_id.clone(),
                                task_id: projection.task_id,
                                status: projection.status.clone(),
                            },
                        ))
                        .await;
                    outgoing
                        .send_server_notification(ServerNotification::GameConversationTurn(
                            GameConversationTurnNotification {
                                conversation_id: conversation_id.clone(),
                                status: projection.status,
                            },
                        ))
                        .await;
                    if let Some(agent_code) = projection.agent_code.clone() {
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationActor(
                                GameConversationActorNotification {
                                    conversation_id: conversation_id.clone(),
                                    turn_id: Some(notification_turn_id.clone()),
                                    agent_code: agent_code.clone(),
                                    status: if is_running { "working" } else { "idle" }.to_string(),
                                },
                            ))
                            .await;
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationFocus(
                                GameConversationFocusNotification {
                                    conversation_id: conversation_id.clone(),
                                    agent_code: projection
                                        .handoff_target
                                        .clone()
                                        .unwrap_or(agent_code),
                                },
                            ))
                            .await;
                    }
                    if let Some(message) = terminal_error {
                        outgoing
                            .send_server_notification(ServerNotification::GameConversationError(
                                GameConversationErrorNotification {
                                    conversation_id: conversation_id.clone(),
                                    turn_id: Some(turn_id.to_string()),
                                    message,
                                },
                            ))
                            .await;
                    }
                    if let Some(character) = projection.character {
                        outgoing
                            .send_server_notification(ServerNotification::GameCharacterUpdated(
                                GameCharacterUpdatedNotification { character },
                            ))
                            .await;
                    }
                    for generation in projection.generations {
                        outgoing
                            .send_server_notification(ServerNotification::GameGenerationUpdated(
                                GameGenerationUpdatedNotification { generation },
                            ))
                            .await;
                    }
                    if let Some(target_agent) = projection.handoff_target {
                        let from_agent = projection.agent_code.unwrap_or_default();
                        let reason = projection.handoff_reason.unwrap_or_default();
                        outgoing
                            .send_server_notification(ServerNotification::GameAgentHandoff(
                                GameAgentHandoffNotification {
                                    conversation_id: conversation_id.clone(),
                                    from_agent_code: from_agent,
                                    to_agent_code: target_agent.clone(),
                                    reason,
                                },
                            ))
                            .await;
                        let Some(connection_id) = observed.connection_ids.first().copied() else {
                            tracing::warn!(
                                thread_id = %observed.thread_id,
                                "cannot continue game handoff without a subscribed connection"
                            );
                            continue;
                        };
                        let scoped_execution = execution.scoped(connection_id);
                        if let Some(context) = event_context.as_ref() {
                            record_task_event(
                                adapter.as_ref(),
                                context,
                                audit_context.as_ref(),
                                "next_action_started",
                                "started",
                                "开始执行 Agent handoff",
                                serde_json::json!({ "targetAgent": target_agent.as_str() }),
                            )
                            .await;
                        }
                        match adapter
                            .continue_handoff(&scoped_execution, &conversation_id, &target_agent)
                            .await
                        {
                            Ok(Some(started)) => {
                                if let Some(context) = event_context.as_ref() {
                                    record_task_event(
                                        adapter.as_ref(),
                                        context,
                                        audit_context.as_ref(),
                                        "next_action_succeeded",
                                        "succeeded",
                                        "Agent handoff 已启动",
                                        serde_json::json!({
                                            "targetAgent": target_agent.as_str(),
                                            "startedTaskId": started.task.id.as_str(),
                                        }),
                                    )
                                    .await;
                                }
                                let task_id = started.task.id.as_str().to_string();
                                let agent_code = started.task.agent_code;
                                let turn_id = started.attempt.codex_turn_id;
                                outgoing
                                    .send_server_notification(ServerNotification::GameTaskUpdated(
                                        GameTaskUpdatedNotification {
                                            conversation_id: conversation_id.clone(),
                                            task_id: task_id.clone(),
                                            status: "running".to_string(),
                                        },
                                    ))
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameAttemptUpdated(
                                            GameAttemptUpdatedNotification {
                                                conversation_id: conversation_id.clone(),
                                                task_id,
                                                attempt_id: started.attempt.id.as_str().to_string(),
                                                turn_id: turn_id.clone(),
                                                status: "running".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationTurn(
                                            GameConversationTurnNotification {
                                                conversation_id: conversation_id.clone(),
                                                status: "running".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationActor(
                                            GameConversationActorNotification {
                                                conversation_id: conversation_id.clone(),
                                                turn_id,
                                                agent_code: agent_code.clone(),
                                                status: "working".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationFocus(
                                            GameConversationFocusNotification {
                                                conversation_id: conversation_id.clone(),
                                                agent_code,
                                            },
                                        ),
                                    )
                                    .await;
                            }
                            Ok(None) => {
                                if let Some(context) = event_context.as_ref() {
                                    record_task_event(
                                        adapter.as_ref(),
                                        context,
                                        audit_context.as_ref(),
                                        "next_action_failed",
                                        "failed",
                                        "Agent handoff 未启动",
                                        serde_json::json!({ "targetAgent": target_agent.as_str() }),
                                    )
                                    .await;
                                }
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationTurn(
                                            GameConversationTurnNotification {
                                                conversation_id: conversation_id.clone(),
                                                status: "blocked".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                            }
                            Err(error) => {
                                if let Some(context) = event_context.as_ref() {
                                    record_task_event(
                                        adapter.as_ref(),
                                        context,
                                        audit_context.as_ref(),
                                        "next_action_failed",
                                        "failed",
                                        "Agent handoff 执行失败",
                                        serde_json::json!({
                                            "targetAgent": target_agent.as_str(),
                                            "error": error.as_str(),
                                        }),
                                    )
                                    .await;
                                }
                                tracing::warn!(
                                    thread_id = %observed.thread_id,
                                    "failed to continue game handoff: {error}"
                                );
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationError(
                                            GameConversationErrorNotification {
                                                conversation_id: conversation_id.clone(),
                                                turn_id: Some(turn_id.to_string()),
                                                message: error,
                                            },
                                        ),
                                    )
                                    .await;
                            }
                        }
                    }
                    if let Some(reason) = director_resume_reason {
                        let Some(connection_id) = observed.connection_ids.first().copied() else {
                            tracing::warn!(
                                thread_id = %observed.thread_id,
                                "cannot resume game director without a subscribed connection"
                            );
                            continue;
                        };
                        let scoped_execution = execution.scoped(connection_id);
                        if let Some(context) = event_context.as_ref() {
                            record_task_event(
                                adapter.as_ref(),
                                context,
                                audit_context.as_ref(),
                                "next_action_started",
                                "started",
                                "开始将控制权回交总管",
                                serde_json::json!({ "targetAgent": "studio_director" }),
                            )
                            .await;
                        }
                        match adapter
                            .resume_director(&scoped_execution, &conversation_id, reason)
                            .await
                        {
                            Ok(Some(started)) => {
                                if let Some(context) = event_context.as_ref() {
                                    record_task_event(
                                        adapter.as_ref(),
                                        context,
                                        audit_context.as_ref(),
                                        "next_action_succeeded",
                                        "succeeded",
                                        "控制权已回交总管",
                                        serde_json::json!({
                                            "targetAgent": "studio_director",
                                            "startedTaskId": started.task.id.as_str(),
                                        }),
                                    )
                                    .await;
                                }
                                let task_id = started.task.id.as_str().to_string();
                                let agent_code = started.task.agent_code;
                                let resumed_turn_id = started.attempt.codex_turn_id;
                                outgoing
                                    .send_server_notification(ServerNotification::GameTaskUpdated(
                                        GameTaskUpdatedNotification {
                                            conversation_id: conversation_id.clone(),
                                            task_id: task_id.clone(),
                                            status: "running".to_string(),
                                        },
                                    ))
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameAttemptUpdated(
                                            GameAttemptUpdatedNotification {
                                                conversation_id: conversation_id.clone(),
                                                task_id,
                                                attempt_id: started.attempt.id.as_str().to_string(),
                                                turn_id: resumed_turn_id.clone(),
                                                status: "running".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationTurn(
                                            GameConversationTurnNotification {
                                                conversation_id: conversation_id.clone(),
                                                status: "running".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationActor(
                                            GameConversationActorNotification {
                                                conversation_id: conversation_id.clone(),
                                                turn_id: resumed_turn_id,
                                                agent_code: agent_code.clone(),
                                                status: "working".to_string(),
                                            },
                                        ),
                                    )
                                    .await;
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationFocus(
                                            GameConversationFocusNotification {
                                                conversation_id: conversation_id.clone(),
                                                agent_code,
                                            },
                                        ),
                                    )
                                    .await;
                            }
                            Ok(None) => {
                                if let Some(context) = event_context.as_ref() {
                                    record_task_event(
                                        adapter.as_ref(),
                                        context,
                                        audit_context.as_ref(),
                                        "next_action_failed",
                                        "failed",
                                        "总管恢复未启动",
                                        serde_json::json!({ "targetAgent": "studio_director" }),
                                    )
                                    .await;
                                }
                            }
                            Err(error) => {
                                if let Some(context) = event_context.as_ref() {
                                    record_task_event(
                                        adapter.as_ref(),
                                        context,
                                        audit_context.as_ref(),
                                        "next_action_failed",
                                        "failed",
                                        "总管恢复执行失败",
                                        serde_json::json!({
                                            "targetAgent": "studio_director",
                                            "error": error.as_str(),
                                        }),
                                    )
                                    .await;
                                }
                                tracing::warn!(
                                    thread_id = %observed.thread_id,
                                    "failed to resume game director: {error}"
                                );
                                outgoing
                                    .send_server_notification(
                                        ServerNotification::GameConversationError(
                                            GameConversationErrorNotification {
                                                conversation_id: conversation_id.clone(),
                                                turn_id: Some(turn_id.to_string()),
                                                message: error,
                                            },
                                        ),
                                    )
                                    .await;
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        thread_id = %observed.thread_id,
                        event_id = %observed.event.id,
                        "failed to project game turn event: {error}"
                    );
                }
            }
        }
    });
}

async fn record_task_event(
    adapter: &GameAppServerAdapter,
    context: &GameTurnEventContext,
    audit_context: Option<&codex_game_runtime::TurnAuditContext>,
    event: &str,
    status: &str,
    message: &str,
    payload: serde_json::Value,
) {
    let mut payload = payload;
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "status".to_string(),
            serde_json::Value::String(status.to_string()),
        );
        object.insert(
            "attemptId".to_string(),
            serde_json::Value::String(context.attempt_id.clone()),
        );
        object.insert(
            "stage".to_string(),
            serde_json::Value::String(context.stage.clone()),
        );
    }
    if let Err(error) = adapter
        .record_task_event(
            context,
            audit_context,
            &TurnAuditEvent {
                event: event.to_string(),
                status: status.to_string(),
                message: message.to_string(),
                payload,
            },
        )
        .await
    {
        tracing::warn!(
            task_id = context.task_id,
            event,
            "failed to record game task lifecycle event: {error}"
        );
    }
}

async fn record_turn_event(
    adapter: &GameAppServerAdapter,
    turn_id: &str,
    event: &str,
    status: &str,
    message: &str,
    payload: serde_json::Value,
) {
    if let Err(error) = adapter
        .record_turn_event(
            turn_id,
            &TurnAuditEvent {
                event: event.to_string(),
                status: status.to_string(),
                message: message.to_string(),
                payload,
            },
        )
        .await
    {
        tracing::warn!(
            turn_id,
            event,
            "failed to record game lifecycle event: {error}"
        );
    }
}

fn append_bounded_partial(output: &mut String, delta: &str) {
    const MAX_PARTIAL_OUTPUT_CHARS: usize = 8 * 1024;
    let remaining = MAX_PARTIAL_OUTPUT_CHARS.saturating_sub(output.chars().count());
    output.extend(delta.chars().take(remaining));
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated after {max_chars} characters]")
    } else {
        truncated
    }
}
