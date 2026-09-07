use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model3dAuditOutcome {
    Success,
    Failure,
}

impl Model3dAuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

/// One provider API round-trip. `request` is the exact payload handed to the
/// provider SDK and `response` is either the decoded success body or the
/// structured provider error, so a paid call can be reconstructed from the audit
/// trail alone.
///
/// `headers` describes the headers the SDK attaches to this transport. Secret
/// values are masked before they reach this struct, so an audit record is always
/// safe to write to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model3dAuditCall {
    pub method: String,
    pub headers: Value,
    pub request: Value,
    pub response: Value,
    pub outcome: Model3dAuditOutcome,
    pub duration_ms: u64,
}

/// Receives one record per provider API call so paid 3D requests stay traceable.
/// Implementations are best-effort: they must not block for long and must
/// swallow their own errors, because auditing must never fail the pipeline.
pub trait Model3dAuditSink: Send + Sync {
    fn record(&self, call: &Model3dAuditCall);
}
