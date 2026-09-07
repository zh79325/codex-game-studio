use crate::GameService;
use crate::GameServiceError;
use crate::model3d_audit::Model3dAuditFile;
use codex_game_domain::Character;
use codex_game_domain::CharacterState;
use codex_game_domain::Model3dAsset;
use codex_game_domain::Model3dCheckpoint;
use codex_game_domain::Model3dJob;
use codex_game_domain::Model3dJobStatus;
use codex_game_domain::Model3dStage;
use codex_game_model3d::MODEL3D_PIPELINE_VERSION;
use codex_game_model3d::Model3dError;
use codex_game_model3d::Model3dProvider;
use codex_game_model3d::Model3dProviderAdapter;
use codex_game_model3d::animation_presets;
use codex_game_model3d::expected_animation_names;
use codex_game_model3d::infer_rig_kind;
use codex_game_model3d::validate_complete_glb;
use codex_game_store::ProjectStore;
use sha2::Digest;
use sha2::Sha256;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use uuid::Uuid;

const TRIPO_PROVIDER_CODE: &str = "tripo3d-zzh";

impl GameService {
    pub async fn start_character_model3d(
        &self,
        project_id: &str,
        character_id: &str,
        provider: Model3dProviderAdapter,
    ) -> Result<Model3dJob, GameServiceError> {
        let project = self.read_project(project_id)?;
        let character = self.read_character(project_id, character_id).await?;
        validate_character(&character)?;
        let view_paths = model_view_paths(&project.root, &character)?;
        let source_hash = source_hash(&view_paths)?;
        let timestamp = now();
        let candidate = Model3dJob {
            id: Uuid::now_v7().to_string(),
            project_id: project_id.to_string(),
            character_id: character_id.to_string(),
            provider_code: TRIPO_PROVIDER_CODE.to_string(),
            source_hash,
            pipeline_version: MODEL3D_PIPELINE_VERSION,
            status: Model3dJobStatus::Pending,
            stage: Model3dStage::UploadingViews,
            inferred_rig_kind: infer_rig_kind(&character.hard_constraints),
            rig_kind: None,
            checkpoint: Model3dCheckpoint::default(),
            asset: None,
            error: None,
            created_at: timestamp,
            updated_at: timestamp,
        };
        let store = self.project_store(project_id, true)?;
        let mut job = store.insert_or_reuse_model3d_job(&candidate).await?;
        if matches!(
            job.status,
            Model3dJobStatus::Succeeded | Model3dJobStatus::Running
        ) {
            return Ok(job);
        }
        // A `needsAttention` job is retried here on purpose: reaching this method
        // means the user explicitly asked for it. Completed checkpoint steps are
        // reused, so an already-submitted paid task is never resubmitted.
        if !store.claim_model3d_job(&job.id, timestamp).await? {
            return store.read_model3d_job(&job.id).await?.ok_or_else(|| {
                GameServiceError::InvalidCharacterOperation("3D 任务状态已变化".to_string())
            });
        }
        job.status = Model3dJobStatus::Running;
        job.error = None;
        job.updated_at = timestamp;
        let returned_job = job.clone();
        let models_relative_path = Path::new(&character.dir_name).join("models");
        let asset_relative_path = models_relative_path
            .join(&job.id)
            .join("character-complete.glb");
        let output_path = Path::new(&project.root).join(&asset_relative_path);
        let manifest_path = Path::new(&project.root)
            .join(&models_relative_path)
            .join("model3d-manifest.json");
        let asset_relative_path = asset_relative_path.to_string_lossy().into_owned();
        let audit = Arc::new(Model3dAuditFile::create(
            Path::new(&project.root),
            &Path::new(&project.root).join(&character.dir_name),
            &job,
        ));
        let provider = provider.with_audit_sink(audit);
        tokio::spawn(async move {
            if let Err(error) = run_model3d_pipeline(
                Arc::clone(&store),
                &mut job,
                view_paths,
                output_path,
                manifest_path,
                asset_relative_path,
                provider,
            )
            .await
            {
                let uncertain = submission_may_be_uncertain(&job, &error);
                job.status = if uncertain {
                    Model3dJobStatus::NeedsAttention
                } else {
                    Model3dJobStatus::Failed
                };
                job.error = Some(if uncertain {
                    format!("远程付费请求结果不确定，已停止自动重试：{error}")
                } else {
                    error.to_string()
                });
                job.updated_at = now();
                if let Err(store_error) = store.update_model3d_job(&job).await {
                    tracing::error!(job_id = %job.id, %store_error, "保存 3D 任务失败状态失败");
                }
            }
        });
        Ok(returned_job)
    }

    pub async fn read_character_model3d_job(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<Option<Model3dJob>, GameServiceError> {
        self.read_character(project_id, character_id).await?;
        self.project_store(project_id, false)?
            .latest_model3d_job(project_id, character_id)
            .await
            .map_err(Into::into)
    }
}

async fn run_model3d_pipeline<P: Model3dProvider + 'static>(
    store: Arc<ProjectStore>,
    job: &mut Model3dJob,
    view_paths: [PathBuf; 4],
    output_path: PathBuf,
    manifest_path: PathBuf,
    asset_relative_path: String,
    provider: P,
) -> Result<(), GameServiceError> {
    set_stage(&store, job, Model3dStage::UploadingViews).await?;
    for (index, path) in view_paths.iter().enumerate() {
        if job.checkpoint.uploaded_view_tokens[index].is_none() {
            let token = provider.upload_image(path).await?;
            job.checkpoint.uploaded_view_tokens[index] = Some(token);
            persist(&store, job).await?;
        }
    }
    let tokens = &job.checkpoint.uploaded_view_tokens;
    let view_tokens = [
        required_token(tokens, 0)?,
        required_token(tokens, 1)?,
        required_token(tokens, 2)?,
        required_token(tokens, 3)?,
    ];

    set_stage(&store, job, Model3dStage::GeneratingModel).await?;
    let model_task_id = match job.checkpoint.model_task_id.clone() {
        Some(task_id) => task_id,
        None => {
            let task_id = provider.generate_textured_model(view_tokens).await?;
            job.checkpoint.model_task_id = Some(task_id.clone());
            persist(&store, job).await?;
            task_id
        }
    };
    provider.wait_for_task(&model_task_id).await?;

    set_stage(&store, job, Model3dStage::CheckingRig).await?;
    let check_task_id = match job.checkpoint.rig_check_task_id.clone() {
        Some(task_id) => task_id,
        None => {
            let task_id = provider.check_rig(&model_task_id).await?;
            job.checkpoint.rig_check_task_id = Some(task_id.clone());
            persist(&store, job).await?;
            task_id
        }
    };
    let check = provider.wait_for_task(&check_task_id).await?;
    let rig_kind = check.rig_kind.unwrap_or(job.inferred_rig_kind);
    job.rig_kind = Some(rig_kind);
    if !check.riggable.unwrap_or(false) {
        job.status = Model3dJobStatus::NeedsAttention;
        job.error = Some("Tripo 检测结果为不可绑骨，请调整四视图后重试".to_string());
        persist(&store, job).await?;
        return Ok(());
    }
    let rig_type = check
        .rig_type
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Model3dError::Provider("绑骨检查未返回 rig_type".to_string()))?;

    set_stage(&store, job, Model3dStage::Rigging).await?;
    let rig_task_id = match job.checkpoint.rig_task_id.clone() {
        Some(task_id) => task_id,
        None => {
            let task_id = provider.rig_model(&model_task_id, &rig_type).await?;
            job.checkpoint.rig_task_id = Some(task_id.clone());
            persist(&store, job).await?;
            task_id
        }
    };
    provider.wait_for_task(&rig_task_id).await?;

    let animations = animation_presets(rig_kind);
    set_stage(&store, job, Model3dStage::Retargeting).await?;
    let animation_task_id = match job.checkpoint.animation_task_id.clone() {
        Some(task_id) => task_id,
        None => {
            let task_id = provider
                .retarget_animation(&rig_task_id, &animations)
                .await?;
            job.checkpoint.animation_task_id = Some(task_id.clone());
            persist(&store, job).await?;
            task_id
        }
    };
    provider.wait_for_task(&animation_task_id).await?;

    set_stage(&store, job, Model3dStage::Downloading).await?;
    let bytes = provider.download_artifact(&animation_task_id).await?;
    set_stage(&store, job, Model3dStage::Validating).await?;
    let expected_animations = expected_animation_names(rig_kind);
    let summary = validate_complete_glb(&bytes, &expected_animations)?;
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temporary = output_path.with_extension("glb.tmp");
    tokio::fs::write(&temporary, &bytes).await?;
    tokio::fs::rename(&temporary, &output_path).await?;
    job.asset = Some(Model3dAsset {
        path: asset_relative_path,
        sha256: sha256(&bytes),
        bytes: bytes.len() as u64,
        rig_kind,
        animation_clips: summary.animation_names,
    });
    job.status = Model3dJobStatus::Succeeded;
    job.stage = Model3dStage::Completed;
    job.error = None;
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "jobId": &job.id,
        "sourceHash": &job.source_hash,
        "pipelineVersion": job.pipeline_version,
        "providerCode": &job.provider_code,
        "asset": &job.asset,
    }))
    .map_err(|error| Model3dError::InvalidInput(error.to_string()))?;
    let manifest_temporary = manifest_path.with_extension("json.tmp");
    tokio::fs::write(&manifest_temporary, manifest).await?;
    tokio::fs::rename(manifest_temporary, manifest_path).await?;
    persist(&store, job).await?;
    Ok(())
}

/// Whether a paid submission may have been accepted remotely despite the local
/// error. Only transport-level failures are ambiguous: when the provider answers
/// with an error envelope it definitively refused the request, so the job must be
/// reported as a plain failure that the user can retry.
fn submission_may_be_uncertain(job: &Model3dJob, error: &GameServiceError) -> bool {
    if matches!(error, GameServiceError::Model3d(model3d_error) if model3d_error.is_provider_rejection())
    {
        return false;
    }
    match job.stage {
        Model3dStage::GeneratingModel => job.checkpoint.model_task_id.is_none(),
        Model3dStage::CheckingRig => job.checkpoint.rig_check_task_id.is_none(),
        Model3dStage::Rigging => job.checkpoint.rig_task_id.is_none(),
        Model3dStage::Retargeting => job.checkpoint.animation_task_id.is_none(),
        Model3dStage::UploadingViews
        | Model3dStage::Downloading
        | Model3dStage::Validating
        | Model3dStage::Completed => false,
    }
}

fn required_token(tokens: &[Option<String>; 4], index: usize) -> Result<String, Model3dError> {
    tokens[index]
        .clone()
        .ok_or_else(|| Model3dError::InvalidInput("四视图上传检查点不完整".to_string()))
}

async fn set_stage(
    store: &ProjectStore,
    job: &mut Model3dJob,
    stage: Model3dStage,
) -> Result<(), GameServiceError> {
    job.stage = stage;
    persist(store, job).await
}

async fn persist(store: &ProjectStore, job: &mut Model3dJob) -> Result<(), GameServiceError> {
    job.updated_at = now();
    store.update_model3d_job(job).await?;
    Ok(())
}

fn validate_character(character: &Character) -> Result<(), GameServiceError> {
    if character.state != CharacterState::S5ViewsConfirmed {
        return Err(GameServiceError::InvalidCharacterOperation(
            "只有已确认四视图的角色才能生成 3D 模型".to_string(),
        ));
    }
    Ok(())
}

fn model_view_paths(
    project_root: &str,
    character: &Character,
) -> Result<[PathBuf; 4], GameServiceError> {
    Ok([
        model_view_path(project_root, character, "front")?,
        model_view_path(project_root, character, "left")?,
        model_view_path(project_root, character, "back")?,
        model_view_path(project_root, character, "right")?,
    ])
}

fn model_view_path(
    project_root: &str,
    character: &Character,
    view: &str,
) -> Result<PathBuf, GameServiceError> {
    let relative = character.view_paths.get(view).ok_or_else(|| {
        GameServiceError::InvalidCharacterOperation(format!("缺少已确认的 {view} 视图"))
    })?;
    let path = safe_project_path(project_root, relative)?;
    path.is_file().then_some(path).ok_or_else(|| {
        GameServiceError::InvalidCharacterOperation(format!("{view} 视图文件不存在"))
    })
}

fn safe_project_path(project_root: &str, relative: &str) -> Result<PathBuf, GameServiceError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(GameServiceError::InvalidCharacterOperation(
            "3D 输入图片路径必须位于项目目录内".to_string(),
        ));
    }
    Ok(Path::new(project_root).join(path))
}

fn source_hash(paths: &[PathBuf; 4]) -> Result<String, GameServiceError> {
    let mut hasher = Sha256::new();
    hasher.update(MODEL3D_PIPELINE_VERSION.to_le_bytes());
    for path in paths {
        hasher.update(std::fs::read(path)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
#[path = "model3d_tests.rs"]
mod tests;
