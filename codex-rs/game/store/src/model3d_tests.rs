use super::*;
use codex_game_domain::Model3dCheckpoint;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

/// A user-initiated start must be able to take over a job parked for human
/// review, otherwise an uncertain paid request strands the character forever.
#[tokio::test]
async fn claims_jobs_parked_for_human_review() {
    let directory = tempdir().expect("temp dir");
    let store = ProjectStore::open(directory.path())
        .await
        .expect("open store");
    let job = Model3dJob {
        id: "job-1".to_string(),
        project_id: "project-1".to_string(),
        character_id: "hero".to_string(),
        provider_code: "tripo3d-zzh".to_string(),
        source_hash: "source".to_string(),
        pipeline_version: 1,
        status: Model3dJobStatus::NeedsAttention,
        stage: Model3dStage::GeneratingModel,
        inferred_rig_kind: RigKind::Biped,
        rig_kind: None,
        checkpoint: Model3dCheckpoint::default(),
        asset: None,
        error: Some("远程付费请求结果不确定".to_string()),
        created_at: 1,
        updated_at: 1,
    };
    store
        .insert_or_reuse_model3d_job(&job)
        .await
        .expect("insert job");

    assert!(
        store
            .claim_model3d_job("job-1", 2)
            .await
            .expect("claim job")
    );
    assert_eq!(
        store.read_model3d_job("job-1").await.expect("read job"),
        Some(Model3dJob {
            status: Model3dJobStatus::Running,
            error: None,
            updated_at: 2,
            ..job
        })
    );
}

/// A job already running must not be claimed twice.
#[tokio::test]
async fn does_not_claim_a_running_job() {
    let directory = tempdir().expect("temp dir");
    let store = ProjectStore::open(directory.path())
        .await
        .expect("open store");
    let job = Model3dJob {
        id: "job-2".to_string(),
        project_id: "project-1".to_string(),
        character_id: "hero".to_string(),
        provider_code: "tripo3d-zzh".to_string(),
        source_hash: "source".to_string(),
        pipeline_version: 1,
        status: Model3dJobStatus::Running,
        stage: Model3dStage::GeneratingModel,
        inferred_rig_kind: RigKind::Biped,
        rig_kind: None,
        checkpoint: Model3dCheckpoint::default(),
        asset: None,
        error: None,
        created_at: 1,
        updated_at: 1,
    };
    store
        .insert_or_reuse_model3d_job(&job)
        .await
        .expect("insert job");

    assert!(
        !store
            .claim_model3d_job("job-2", 2)
            .await
            .expect("claim job")
    );
}
