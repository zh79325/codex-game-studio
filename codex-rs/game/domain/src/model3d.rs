use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Model3dJobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    NeedsAttention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Model3dStage {
    UploadingViews,
    GeneratingModel,
    CheckingRig,
    Rigging,
    Retargeting,
    Downloading,
    Validating,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RigKind {
    Biped,
    Quadruped,
    Hexapod,
    Octopod,
    Avian,
    Serpentine,
    Aquatic,
    Other,
}

impl RigKind {
    pub fn as_provider_value(self) -> &'static str {
        match self {
            Self::Biped => "biped",
            Self::Quadruped => "quadruped",
            Self::Hexapod => "hexapod",
            Self::Octopod => "octopod",
            Self::Avian => "avian",
            Self::Serpentine => "serpentine",
            Self::Aquatic => "aquatic",
            Self::Other => "others",
        }
    }

    pub fn from_provider_value(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "biped" => Self::Biped,
            "quadruped" => Self::Quadruped,
            "hexapod" => Self::Hexapod,
            "octopod" => Self::Octopod,
            "avian" => Self::Avian,
            "serpentine" => Self::Serpentine,
            "aquatic" => Self::Aquatic,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Model3dCheckpoint {
    pub uploaded_view_tokens: [Option<String>; 4],
    pub model_task_id: Option<String>,
    pub rig_check_task_id: Option<String>,
    pub rig_task_id: Option<String>,
    pub animation_task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model3dAsset {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub rig_kind: RigKind,
    pub animation_clips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model3dJob {
    pub id: String,
    pub project_id: String,
    pub character_id: String,
    pub provider_code: String,
    pub source_hash: String,
    pub pipeline_version: u32,
    pub status: Model3dJobStatus,
    pub stage: Model3dStage,
    pub inferred_rig_kind: RigKind,
    pub rig_kind: Option<RigKind>,
    pub checkpoint: Model3dCheckpoint,
    pub asset: Option<Model3dAsset>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
