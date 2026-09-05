use crate::RouteDecision;
use crate::StartTurnRequest;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAuditContext {
    pub project_root: PathBuf,
    pub target_dir: PathBuf,
    pub conversation_id: String,
    pub turn: u64,
    pub target: String,
    pub agent_code: String,
    pub attempt_id: String,
    pub attempt_no: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAuditFailure {
    pub stage: String,
    pub kind: String,
    pub message: String,
    pub retryable: bool,
    pub token_limit_related: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnAuditRetry {
    pub kind: String,
    pub status: String,
    pub retry_attempt_id: Option<String>,
    pub retry_attempt_no: Option<u32>,
    pub max_output_tokens: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnAuditUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnAuditCompletion {
    pub response: Option<String>,
    pub error: Option<String>,
    pub usage: Option<TurnAuditUsage>,
    pub duration_ms: Option<i64>,
    pub time_to_first_token_ms: Option<i64>,
}

pub fn write_turn_audit_request(
    context: &TurnAuditContext,
    route: &RouteDecision,
    request: &StartTurnRequest,
) -> io::Result<()> {
    if !conversation_audit_enabled(&context.project_root) {
        return Ok(());
    }

    let path = audit_path(context);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let input = request
        .model_input()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let provider = if route.provider.is_empty() {
        "configured"
    } else {
        &route.provider
    };
    let model = if route.model.is_empty() {
        "configured"
    } else {
        &route.model
    };
    let max_output_tokens = request
        .max_output_tokens
        .map_or_else(|| "provider default".to_string(), |value| value.to_string());
    let body = format!(
        "# LLM 对话审计\n\n- 会话：{}\n- 轮次：{}\n- 时间：{}\n- 目标：{}\n- Agent：{}\n- Attempt：{}\n- Attempt no：{}\n\n## 调用 1：主回答\n\n- Provider：{}\n- Model：{}\n- Max output tokens：{}\n\n### Request\n\n#### 1.1 user\n\n{}",
        context.conversation_id,
        context.turn,
        now(),
        context.target,
        context.agent_code,
        context.attempt_id,
        context.attempt_no,
        provider,
        model,
        max_output_tokens,
        code_block(&redact_data_urls(&input), "text"),
    );
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(body.as_bytes())?;
    file.flush()?;
    file.sync_all()
}

pub fn append_turn_audit_completion(
    context: &TurnAuditContext,
    completion: &TurnAuditCompletion,
) -> io::Result<()> {
    let path = audit_path(context);
    if !path.exists() {
        return Ok(());
    }

    let mut body = String::new();
    if let Some(error) = completion.error.as_deref() {
        let normalized = error.to_ascii_lowercase();
        let (kind, token_limit_related) = if normalized.contains("reason: length")
            || (normalized.contains("finish_reason") && normalized.contains("length"))
        {
            ("output_length", true)
        } else if error.starts_with("运行已中断") {
            ("turn_aborted", false)
        } else {
            ("turn_execution", false)
        };
        append_failure_body(
            &mut body,
            context,
            &TurnAuditFailure {
                stage: "turn_completion".to_string(),
                kind: kind.to_string(),
                message: error.to_string(),
                retryable: token_limit_related,
                token_limit_related,
            },
        );
    } else if let Some(response) = completion.response.as_deref() {
        body.push_str("\n### Response\n\n");
        body.push_str(&code_block(&redact_data_urls(response), "markdown"));
    }
    if let Some(usage) = &completion.usage {
        body.push_str(&format!(
            "\n- Input tokens：{}\n- Cached input tokens：{}\n- Output tokens：{}\n- Reasoning output tokens：{}\n- Total tokens：{}\n",
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.output_tokens,
            usage.reasoning_output_tokens,
            usage.total_tokens,
        ));
    }
    if let Some(duration_ms) = completion.duration_ms {
        body.push_str(&format!("- Latency：{duration_ms} ms\n"));
    }
    if let Some(time_to_first_token_ms) = completion.time_to_first_token_ms {
        body.push_str(&format!(
            "- Time to first token：{time_to_first_token_ms} ms\n"
        ));
    }
    append(&path, &body)
}

pub fn append_turn_audit_failure(
    context: &TurnAuditContext,
    failure: &TurnAuditFailure,
) -> io::Result<()> {
    let path = audit_path(context);
    if !path.exists() {
        return Ok(());
    }
    let mut body = String::new();
    append_failure_body(&mut body, context, failure);
    append(&path, &body)
}

pub fn append_turn_audit_retry(
    context: &TurnAuditContext,
    retry: &TurnAuditRetry,
) -> io::Result<()> {
    let path = audit_path(context);
    if !path.exists() {
        return Ok(());
    }
    let retry_attempt_id = retry.retry_attempt_id.as_deref().unwrap_or("none");
    let retry_attempt_no = retry
        .retry_attempt_no
        .map_or_else(|| "none".to_string(), |value| value.to_string());
    let max_output_tokens = retry
        .max_output_tokens
        .map_or_else(|| "provider default".to_string(), |value| value.to_string());
    let mut body = format!(
        "\n### Retry Event\n\n- Time：{}\n- Kind：{}\n- Status：{}\n- Source attempt：{}\n- Source attempt no：{}\n- Retry attempt：{}\n- Retry attempt no：{}\n- Max output tokens：{}\n",
        now(),
        retry.kind,
        retry.status,
        context.attempt_id,
        context.attempt_no,
        retry_attempt_id,
        retry_attempt_no,
        max_output_tokens,
    );
    if let Some(error) = retry.error.as_deref() {
        body.push_str("\n#### Retry Error\n\n");
        body.push_str(&code_block(
            &truncate_chars(&redact_data_urls(error), 4096),
            "text",
        ));
    }
    append(&path, &body)
}

pub fn append_turn_audit_stream_termination(
    context: &TurnAuditContext,
    stage: &str,
    reason: &str,
) -> io::Result<()> {
    let path = audit_path(context);
    if !path.exists() {
        return Ok(());
    }
    let reason = truncate_chars(&redact_data_urls(reason), 4096);
    append(
        &path,
        &format!(
            "\n### Stream Terminated\n\n- Time：{}\n- Stage：{stage}\n\n{}",
            now(),
            code_block(&reason, "text"),
        ),
    )
}

pub fn append_turn_audit_start_error(context: &TurnAuditContext, error: &str) -> io::Result<()> {
    append_turn_audit_failure(
        context,
        &TurnAuditFailure {
            stage: "turn_start".to_string(),
            kind: "start_error".to_string(),
            message: error.to_string(),
            retryable: false,
            token_limit_related: false,
        },
    )
}

fn append_failure_body(body: &mut String, context: &TurnAuditContext, failure: &TurnAuditFailure) {
    body.push_str(&format!(
        "\n### Failure Event\n\n- Time：{}\n- Stage：{}\n- Kind：{}\n- Attempt：{}\n- Attempt no：{}\n- Retryable：{}\n- Token limit related：{}\n\n#### Error\n\n",
        now(),
        failure.stage,
        failure.kind,
        context.attempt_id,
        context.attempt_no,
        failure.retryable,
        failure.token_limit_related,
    ));
    body.push_str(&code_block(
        &truncate_chars(&redact_data_urls(&failure.message), 4096),
        "text",
    ));
}

fn conversation_audit_enabled(project_root: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(project_root.join("project.json")) else {
        return false;
    };
    let Ok(Value::Object(document)) = serde_json::from_str::<Value>(&contents) else {
        return false;
    };
    document
        .get("conversationAudit")
        .or_else(|| document.get("conversation_audit"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn audit_path(context: &TurnAuditContext) -> PathBuf {
    let agent_code = context
        .agent_code
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    context
        .target_dir
        .join("tmp")
        .join("conversation")
        .join(format!(
            "{agent_code}-turn-{}-{}.md",
            context.turn, context.attempt_id
        ))
}

fn append(path: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().append(true).open(path)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()
}

fn code_block(content: &str, language: &str) -> String {
    let longest = content
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    format!("{fence}{language}\n{content}\n{fence}\n")
}

fn truncate_chars(content: &str, max_chars: usize) -> String {
    let mut chars = content.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated after {max_chars} characters]")
    } else {
        truncated
    }
}

fn redact_data_urls(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut remaining = content;
    while let Some(start) = remaining.find("data:") {
        output.push_str(&remaining[..start]);
        let candidate = &remaining[start..];
        let Some(marker) = candidate.find(";base64,").filter(|marker| *marker <= 128) else {
            output.push_str(candidate);
            return output;
        };
        let payload_start = marker + ";base64,".len();
        let payload_len = candidate[payload_start..]
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=')
            })
            .map(char::len_utf8)
            .sum::<usize>();
        if payload_len == 0 {
            output.push_str(&candidate[..payload_start]);
            remaining = &candidate[payload_start..];
            continue;
        }
        let payload = &candidate[payload_start..payload_start + payload_len];
        let mime = &candidate["data:".len()..marker];
        let digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
        output.push_str(&format!(
            "[data URL omitted: mime={}, encoded_chars={}, sha256={}]",
            if mime.is_empty() { "unknown" } else { mime },
            payload.len(),
            &digest[..16]
        ));
        remaining = &candidate[payload_start + payload_len..];
    }
    output.push_str(remaining);
    output
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
