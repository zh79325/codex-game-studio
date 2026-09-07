use crate::ProjectAccess;
use crate::ProjectStore;
use crate::StoreError;
use codex_game_domain::Model3dAsset;
use codex_game_domain::Model3dCheckpoint;
use codex_game_domain::Model3dJob;
use codex_game_domain::Model3dJobStatus;
use codex_game_domain::Model3dStage;
use codex_game_domain::RigKind;
use sqlx::Row;

impl ProjectStore {
    pub async fn insert_or_reuse_model3d_job(
        &self,
        candidate: &Model3dJob,
    ) -> Result<Model3dJob, StoreError> {
        self.require_model3d_writable()?;
        let mut transaction = self.pool().begin().await?;
        if let Some(row) = sqlx::query(
            "SELECT * FROM character_model3d_jobs WHERE project_id = ? AND character_id = ? AND provider_code = ? AND source_hash = ? AND pipeline_version = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&candidate.project_id)
        .bind(&candidate.character_id)
        .bind(&candidate.provider_code)
        .bind(&candidate.source_hash)
        .bind(candidate.pipeline_version as i64)
        .fetch_optional(&mut *transaction)
        .await?
        {
            transaction.commit().await?;
            return model3d_job_from_row(&row);
        }
        sqlx::query("INSERT INTO character_model3d_jobs(id, project_id, character_id, provider_code, source_hash, pipeline_version, status, stage, inferred_rig_kind, rig_kind, checkpoint_json, asset_json, error_message, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&candidate.id)
            .bind(&candidate.project_id)
            .bind(&candidate.character_id)
            .bind(&candidate.provider_code)
            .bind(&candidate.source_hash)
            .bind(candidate.pipeline_version as i64)
            .bind(job_status_name(candidate.status))
            .bind(job_stage_name(candidate.stage))
            .bind(rig_kind_name(candidate.inferred_rig_kind))
            .bind(candidate.rig_kind.map(rig_kind_name))
            .bind(serde_json::to_string(&candidate.checkpoint)?)
            .bind(candidate.asset.as_ref().map(serde_json::to_string).transpose()?)
            .bind(&candidate.error)
            .bind(candidate.created_at)
            .bind(candidate.updated_at)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(candidate.clone())
    }

    pub async fn read_model3d_job(&self, job_id: &str) -> Result<Option<Model3dJob>, StoreError> {
        sqlx::query("SELECT * FROM character_model3d_jobs WHERE id = ?")
            .bind(job_id)
            .fetch_optional(self.pool())
            .await?
            .as_ref()
            .map(model3d_job_from_row)
            .transpose()
    }

    pub async fn latest_model3d_job(
        &self,
        project_id: &str,
        character_id: &str,
    ) -> Result<Option<Model3dJob>, StoreError> {
        sqlx::query("SELECT * FROM character_model3d_jobs WHERE project_id = ? AND character_id = ? ORDER BY created_at DESC, id DESC LIMIT 1")
            .bind(project_id)
            .bind(character_id)
            .fetch_optional(self.pool())
            .await?
            .as_ref()
            .map(model3d_job_from_row)
            .transpose()
    }

    pub async fn recover_running_model3d_jobs(
        &self,
        project_id: &str,
        updated_at: i64,
    ) -> Result<(), StoreError> {
        self.require_model3d_writable()?;
        sqlx::query("UPDATE character_model3d_jobs SET status = 'failed', error_message = '应用退出导致任务中断，可重新启动以从检查点继续', updated_at = ? WHERE project_id = ? AND status = 'running'")
            .bind(updated_at)
            .bind(project_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Takes ownership of a job so the pipeline can run it. `needsAttention` is
    /// claimable because the only caller is an explicit user-initiated start, and
    /// that click is the human review the status asks for. Nothing claims a job
    /// automatically: crash recovery only moves `running` jobs to `failed`.
    pub async fn claim_model3d_job(
        &self,
        job_id: &str,
        updated_at: i64,
    ) -> Result<bool, StoreError> {
        self.require_model3d_writable()?;
        let result = sqlx::query("UPDATE character_model3d_jobs SET status = 'running', error_message = NULL, updated_at = ? WHERE id = ? AND status IN ('pending', 'failed', 'needsAttention')")
            .bind(updated_at)
            .bind(job_id)
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn update_model3d_job(&self, job: &Model3dJob) -> Result<(), StoreError> {
        self.require_model3d_writable()?;
        let updated = sqlx::query("UPDATE character_model3d_jobs SET status = ?, stage = ?, rig_kind = ?, checkpoint_json = ?, asset_json = ?, error_message = ?, updated_at = ? WHERE id = ? AND project_id = ? AND character_id = ?")
            .bind(job_status_name(job.status))
            .bind(job_stage_name(job.stage))
            .bind(job.rig_kind.map(rig_kind_name))
            .bind(serde_json::to_string(&job.checkpoint)?)
            .bind(job.asset.as_ref().map(serde_json::to_string).transpose()?)
            .bind(&job.error)
            .bind(job.updated_at)
            .bind(&job.id)
            .bind(&job.project_id)
            .bind(&job.character_id)
            .execute(self.pool())
            .await?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::NotFound(format!("model3d job {}", job.id)));
        }
        Ok(())
    }

    fn require_model3d_writable(&self) -> Result<(), StoreError> {
        if self.access() == ProjectAccess::ReadOnly {
            Err(StoreError::ReadOnly)
        } else {
            Ok(())
        }
    }
}

fn model3d_job_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Model3dJob, StoreError> {
    let status = parse_status(row.try_get::<String, _>("status")?.as_str())?;
    let stage = parse_stage(row.try_get::<String, _>("stage")?.as_str())?;
    let inferred_rig_kind = parse_rig_kind(row.try_get::<String, _>("inferred_rig_kind")?.as_str());
    let rig_kind = row
        .try_get::<Option<String>, _>("rig_kind")?
        .as_deref()
        .map(parse_rig_kind);
    let checkpoint =
        serde_json::from_str::<Model3dCheckpoint>(&row.try_get::<String, _>("checkpoint_json")?)?;
    let asset = row
        .try_get::<Option<String>, _>("asset_json")?
        .map(|value| serde_json::from_str::<Model3dAsset>(&value))
        .transpose()?;
    Ok(Model3dJob {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        character_id: row.try_get("character_id")?,
        provider_code: row.try_get("provider_code")?,
        source_hash: row.try_get("source_hash")?,
        pipeline_version: row.try_get::<i64, _>("pipeline_version")? as u32,
        status,
        stage,
        inferred_rig_kind,
        rig_kind,
        checkpoint,
        asset,
        error: row.try_get("error_message")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn job_status_name(status: Model3dJobStatus) -> &'static str {
    match status {
        Model3dJobStatus::Pending => "pending",
        Model3dJobStatus::Running => "running",
        Model3dJobStatus::Succeeded => "succeeded",
        Model3dJobStatus::Failed => "failed",
        Model3dJobStatus::NeedsAttention => "needsAttention",
    }
}

fn parse_status(value: &str) -> Result<Model3dJobStatus, StoreError> {
    match value {
        "pending" => Ok(Model3dJobStatus::Pending),
        "running" => Ok(Model3dJobStatus::Running),
        "succeeded" => Ok(Model3dJobStatus::Succeeded),
        "failed" => Ok(Model3dJobStatus::Failed),
        "needsAttention" => Ok(Model3dJobStatus::NeedsAttention),
        _ => Err(StoreError::InvalidData(format!(
            "unknown model3d status: {value}"
        ))),
    }
}

fn job_stage_name(stage: Model3dStage) -> &'static str {
    match stage {
        Model3dStage::UploadingViews => "uploadingViews",
        Model3dStage::GeneratingModel => "generatingModel",
        Model3dStage::CheckingRig => "checkingRig",
        Model3dStage::Rigging => "rigging",
        Model3dStage::Retargeting => "retargeting",
        Model3dStage::Downloading => "downloading",
        Model3dStage::Validating => "validating",
        Model3dStage::Completed => "completed",
    }
}

fn parse_stage(value: &str) -> Result<Model3dStage, StoreError> {
    match value {
        "uploadingViews" => Ok(Model3dStage::UploadingViews),
        "generatingModel" => Ok(Model3dStage::GeneratingModel),
        "checkingRig" => Ok(Model3dStage::CheckingRig),
        "rigging" => Ok(Model3dStage::Rigging),
        "retargeting" => Ok(Model3dStage::Retargeting),
        "downloading" => Ok(Model3dStage::Downloading),
        "validating" => Ok(Model3dStage::Validating),
        "completed" => Ok(Model3dStage::Completed),
        _ => Err(StoreError::InvalidData(format!(
            "unknown model3d stage: {value}"
        ))),
    }
}

fn rig_kind_name(kind: RigKind) -> &'static str {
    kind.as_provider_value()
}

fn parse_rig_kind(value: &str) -> RigKind {
    RigKind::from_provider_value(value)
}

#[cfg(test)]
#[path = "model3d_tests.rs"]
mod tests;

