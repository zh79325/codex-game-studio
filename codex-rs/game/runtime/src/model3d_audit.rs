use crate::audit::append;
use crate::audit::code_block;
use crate::audit::conversation_audit_enabled;
use crate::audit::now;
use crate::audit::truncate_chars;
use codex_game_domain::Model3dJob;
use codex_game_model3d::Model3dAuditCall;
use codex_game_model3d::Model3dAuditSink;
use serde_json::Value;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const MAX_AUDIT_CHARS: usize = 16_384;

/// Append-only Markdown audit trail for a single 3D model job, written next to
/// the agent conversation audits so every paid provider call can be traced back
/// to its exact request payload and provider response.
pub struct Model3dAuditFile {
    path: Option<PathBuf>,
}

impl Model3dAuditFile {
    /// Opens the audit file for `job`. Auditing is best-effort: when it is
    /// disabled for the project or the file cannot be created, the returned sink
    /// silently discards records instead of failing the pipeline.
    pub fn create(project_root: &Path, target_dir: &Path, job: &Model3dJob) -> Self {
        if !conversation_audit_enabled(project_root) {
            return Self { path: None };
        }
        let path = target_dir
            .join("tmp")
            .join("conversation")
            .join(format!("model3d-{}.md", job.id));
        match write_header(&path, job) {
            Ok(()) => Self { path: Some(path) },
            Err(error) => {
                tracing::warn!(job_id = %job.id, %error, "创建 3D 模型审计文件失败");
                Self { path: None }
            }
        }
    }
}

impl Model3dAuditSink for Model3dAuditFile {
    fn record(&self, call: &Model3dAuditCall) {
        let Some(path) = &self.path else {
            return;
        };
        let body = format!(
            "\n## {}\n\n- Time：{}\n- Outcome：{}\n- Duration：{} ms\n\n### Request\n\n#### Headers\n\n{}\n#### Body\n\n{}\n### Response\n\n{}",
            call.method,
            now(),
            call.outcome.as_str(),
            call.duration_ms,
            json_block(&call.headers),
            json_block(&call.request),
            json_block(&call.response),
        );
        if let Err(error) = append(path, &body) {
            tracing::warn!(method = %call.method, %error, "写入 3D 模型审计记录失败");
        }
    }
}

/// Appends the job header. Uses append mode so a resumed job keeps its earlier
/// calls and gains a new header marking where the resume started.
fn write_header(path: &Path, job: &Model3dJob) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = format!(
        "# 3D 模型生成审计\n\n- Job：{}\n- 项目：{}\n- 角色：{}\n- Provider：{}\n- Pipeline 版本：{}\n- 四视图哈希：{}\n- 记录开始：{}\n",
        job.id,
        job.project_id,
        job.character_id,
        job.provider_code,
        job.pipeline_version,
        job.source_hash,
        now(),
    );
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(body.as_bytes())?;
    file.flush()?;
    file.sync_all()
}

fn json_block(value: &Value) -> String {
    let rendered = serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| format!("<不可序列化：{error}>"));
    code_block(&truncate_chars(&rendered, MAX_AUDIT_CHARS), "json")
}
