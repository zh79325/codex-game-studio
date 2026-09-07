mod audit;
mod glb;
mod tripo;

use codex_game_domain::RigKind;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

pub use audit::Model3dAuditCall;
pub use audit::Model3dAuditOutcome;
pub use audit::Model3dAuditSink;
pub use glb::GlbSummary;
pub use glb::validate_complete_glb;
pub use tripo::TripoModel3dProvider;

pub const MODEL3D_PIPELINE_VERSION: u32 = 1;
pub const DEFAULT_MODEL3D_MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum Model3dError {
    #[error("invalid 3D model request: {0}")]
    InvalidInput(String),
    #[error("3D model provider request failed: {0}")]
    Provider(String),
    /// The provider answered with a well-formed error envelope, which proves the
    /// submission reached it and was refused. No remote task was created, so the
    /// request is a deterministic failure and is safe to retry after the cause is
    /// fixed.
    #[error("3D model provider rejected the request: {message}")]
    ProviderRejected { code: i64, message: String },
    #[error("3D model task returned no complete GLB")]
    MissingArtifact,
    #[error("3D model asset is invalid: {0}")]
    InvalidGlb(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Model3dError {
    /// Whether the provider explicitly refused the request. Callers use this to
    /// avoid treating a definitive refusal as an uncertain paid submission.
    pub fn is_provider_rejection(&self) -> bool {
        matches!(self, Self::ProviderRejected { .. })
    }
}

pub type Result<T> = std::result::Result<T, Model3dError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model3dProviderTask {
    pub id: String,
    pub riggable: Option<bool>,
    pub rig_type: Option<String>,
    pub rig_kind: Option<RigKind>,
}

/// Provider-neutral contract for creating a textured, rigged and animated 3D character.
/// Implementations submit remote tasks but never persist provider URLs or credentials.
pub trait Model3dProvider: Send + Sync {
    fn upload_image(&self, path: &Path) -> impl Future<Output = Result<String>> + Send;

    fn generate_textured_model(
        &self,
        view_tokens: [String; 4],
    ) -> impl Future<Output = Result<String>> + Send;

    fn check_rig(&self, model_task_id: &str) -> impl Future<Output = Result<String>> + Send;

    fn rig_model(
        &self,
        model_task_id: &str,
        rig_type: &str,
    ) -> impl Future<Output = Result<String>> + Send;

    fn retarget_animation(
        &self,
        rig_task_id: &str,
        animations: &[String],
    ) -> impl Future<Output = Result<String>> + Send;

    fn wait_for_task(
        &self,
        task_id: &str,
    ) -> impl Future<Output = Result<Model3dProviderTask>> + Send;

    fn download_artifact(&self, task_id: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

pub enum Model3dProviderAdapter {
    Tripo(TripoModel3dProvider),
}

impl Model3dProviderAdapter {
    /// Attaches an audit sink that records every provider API call. Applied after
    /// construction because the audit destination depends on the job being run.
    pub fn with_audit_sink(self, sink: Arc<dyn Model3dAuditSink>) -> Self {
        match self {
            Self::Tripo(provider) => Self::Tripo(provider.with_audit_sink(sink)),
        }
    }
}

impl Model3dProvider for Model3dProviderAdapter {
    async fn upload_image(&self, path: &Path) -> Result<String> {
        match self {
            Self::Tripo(provider) => provider.upload_image(path).await,
        }
    }

    async fn generate_textured_model(&self, view_tokens: [String; 4]) -> Result<String> {
        match self {
            Self::Tripo(provider) => provider.generate_textured_model(view_tokens).await,
        }
    }

    async fn check_rig(&self, model_task_id: &str) -> Result<String> {
        match self {
            Self::Tripo(provider) => provider.check_rig(model_task_id).await,
        }
    }

    async fn rig_model(&self, model_task_id: &str, rig_type: &str) -> Result<String> {
        match self {
            Self::Tripo(provider) => provider.rig_model(model_task_id, rig_type).await,
        }
    }

    async fn retarget_animation(&self, rig_task_id: &str, animations: &[String]) -> Result<String> {
        match self {
            Self::Tripo(provider) => provider.retarget_animation(rig_task_id, animations).await,
        }
    }

    async fn wait_for_task(&self, task_id: &str) -> Result<Model3dProviderTask> {
        match self {
            Self::Tripo(provider) => provider.wait_for_task(task_id).await,
        }
    }

    async fn download_artifact(&self, task_id: &str) -> Result<Vec<u8>> {
        match self {
            Self::Tripo(provider) => provider.download_artifact(task_id).await,
        }
    }
}

pub fn animation_presets(rig_kind: RigKind) -> Vec<String> {
    match rig_kind {
        RigKind::Biped => ["preset:idle", "preset:walk", "preset:run"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        RigKind::Quadruped
        | RigKind::Hexapod
        | RigKind::Octopod
        | RigKind::Avian
        | RigKind::Serpentine
        | RigKind::Aquatic
        | RigKind::Other => vec!["preset:walk".to_string()],
    }
}

pub fn expected_animation_names(rig_kind: RigKind) -> Vec<String> {
    animation_presets(rig_kind)
        .into_iter()
        .map(|preset| preset.rsplit(':').next().unwrap_or(&preset).to_string())
        .collect()
}

pub fn infer_rig_kind(constraints: &[serde_json::Value]) -> RigKind {
    let text = constraints
        .iter()
        .filter_map(|constraint| {
            let item = constraint.get("item").and_then(serde_json::Value::as_str)?;
            let value = constraint
                .get("value")
                .and_then(serde_json::Value::as_str)?;
            Some(format!("{item} {value}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if contains_any(&text, &["六足", "6足", "hexapod"]) {
        RigKind::Hexapod
    } else if contains_any(&text, &["八足", "8足", "octopod"]) {
        RigKind::Octopod
    } else if contains_any(&text, &["四足", "4足", "quadruped"]) {
        RigKind::Quadruped
    } else if contains_any(&text, &["蛇形", "serpentine"]) {
        RigKind::Serpentine
    } else if contains_any(&text, &["水生", "aquatic"]) {
        RigKind::Aquatic
    } else if contains_any(&text, &["鸟类", "avian", "翅膀"]) {
        RigKind::Avian
    } else if contains_any(&text, &["直立双足", "双足", "biped", "humanoid", "人形"]) {
        RigKind::Biped
    } else {
        RigKind::Other
    }
}

fn contains_any(value: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|candidate| value.contains(candidate))
}

pub fn wait_timeout() -> Duration {
    Duration::from_secs(30 * 60)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
