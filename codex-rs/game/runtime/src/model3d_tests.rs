use super::*;
use codex_game_model3d::Model3dAuditSink;
use codex_game_model3d::Model3dProviderTask;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Mutex;
use tempfile::tempdir;

struct FakeProvider {
    calls: Arc<Mutex<Vec<String>>>,
    artifact: Vec<u8>,
}

impl Model3dProvider for FakeProvider {
    async fn upload_image(&self, path: &Path) -> codex_game_model3d::Result<String> {
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("upload:{name}"));
        Ok(format!("token-{name}"))
    }

    async fn generate_textured_model(
        &self,
        view_tokens: [String; 4],
    ) -> codex_game_model3d::Result<String> {
        self.calls
            .lock()
            .expect("test setup")
            .push(format!("model:{}", view_tokens.join(",")));
        Ok("model-task".to_string())
    }

    async fn check_rig(&self, model_task_id: &str) -> codex_game_model3d::Result<String> {
        self.calls
            .lock()
            .expect("test setup")
            .push(format!("check:{model_task_id}"));
        Ok("check-task".to_string())
    }

    async fn rig_model(
        &self,
        model_task_id: &str,
        rig_type: &str,
    ) -> codex_game_model3d::Result<String> {
        self.calls
            .lock()
            .expect("test setup")
            .push(format!("rig:{model_task_id}:{rig_type}"));
        Ok("rig-task".to_string())
    }

    async fn retarget_animation(
        &self,
        rig_task_id: &str,
        animations: &[String],
    ) -> codex_game_model3d::Result<String> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("animate:{rig_task_id}:{}", animations.join(",")));
        Ok("animation-task".to_string())
    }

    async fn wait_for_task(
        &self,
        task_id: &str,
    ) -> codex_game_model3d::Result<Model3dProviderTask> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(format!("wait:{task_id}"));
        let is_check = task_id == "check-task";
        Ok(Model3dProviderTask {
            id: task_id.to_string(),
            riggable: is_check.then_some(true),
            rig_type: is_check.then(|| "biped".to_string()),
            rig_kind: is_check.then_some(codex_game_domain::RigKind::Biped),
        })
    }

    async fn download_artifact(&self, task_id: &str) -> codex_game_model3d::Result<Vec<u8>> {
        self.calls
            .lock()
            .expect("test setup")
            .push(format!("download:{task_id}"));
        Ok(self.artifact.clone())
    }
}

#[tokio::test]
async fn pipeline_uses_detected_rig_type_and_persists_animated_glb() {
    let directory = tempdir().expect("test setup");
    let store = Arc::new(
        ProjectStore::open(directory.path())
            .await
            .expect("test setup"),
    );
    let view_paths = ["front", "left", "back", "right"].map(|view| {
        let path = directory.path().join(format!("{view}.png"));
        std::fs::write(&path, view.as_bytes()).expect("test setup");
        path
    });
    let output_path = directory
        .path()
        .join("characters/hero/models/character.glb");
    let manifest_path = directory
        .path()
        .join("characters/hero/models/model3d-manifest.json");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = FakeProvider {
        calls: Arc::clone(&calls),
        artifact: complete_glb(),
    };
    let mut job = pending_job();
    store
        .insert_or_reuse_model3d_job(&job)
        .await
        .expect("test setup");

    run_model3d_pipeline(
        Arc::clone(&store),
        &mut job,
        view_paths,
        output_path.clone(),
        manifest_path.clone(),
        "characters/hero/models/character.glb".to_string(),
        provider,
    )
    .await
    .expect("test setup");

    assert_eq!(job.status, Model3dJobStatus::Succeeded);
    assert_eq!(job.stage, Model3dStage::Completed);
    assert_eq!(job.rig_kind, Some(codex_game_domain::RigKind::Biped));
    assert_eq!(
        std::fs::read(output_path).expect("test setup"),
        complete_glb()
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).expect("test setup"))
            .expect("test setup");
    assert_eq!(
        manifest["asset"]["path"],
        "characters/hero/models/character.glb"
    );
    assert_eq!(
        store.read_model3d_job("job-1").await.expect("test setup"),
        Some(job)
    );
    assert_eq!(
        *calls.lock().expect("calls lock"),
        vec![
            "upload:front",
            "upload:left",
            "upload:back",
            "upload:right",
            "model:token-front,token-left,token-back,token-right",
            "wait:model-task",
            "check:model-task",
            "wait:check-task",
            "rig:model-task:biped",
            "wait:rig-task",
            "animate:rig-task:preset:idle,preset:walk,preset:run",
            "wait:animation-task",
            "download:animation-task",
        ]
    );
}

#[test]
fn provider_rejection_is_a_definitive_failure_before_the_task_exists() {
    let mut job = pending_job();
    job.stage = Model3dStage::GeneratingModel;
    assert_eq!(job.checkpoint.model_task_id, None);

    let rejected = GameServiceError::Model3d(Model3dError::ProviderRejected {
        code: 2010,
        message: "Tripo API error (code=2010): You don't have enough credit".to_string(),
    });
    assert!(!submission_may_be_uncertain(&job, &rejected));

    let transport = GameServiceError::Model3d(Model3dError::Provider(
        "request error: connection reset".to_string(),
    ));
    assert!(submission_may_be_uncertain(&job, &transport));
}

#[test]
fn audit_file_records_requests_and_responses_next_to_conversations() {
    let directory = tempdir().expect("test setup");
    let project_root = directory.path();
    let character_dir = project_root.join("characters/hero");
    std::fs::create_dir_all(&character_dir).expect("test setup");
    std::fs::write(
        project_root.join("project.json"),
        r#"{"schemaVersion":2,"projectId":"project-1"}"#,
    )
    .expect("test setup");

    let job = pending_job();
    let audit = Model3dAuditFile::create(project_root, &character_dir, &job);
    audit.record(&codex_game_model3d::Model3dAuditCall {
        method: "multiview_to_model".to_string(),
        request: json!({ "model": "P1", "face_limit": 5000 }),
        response: json!({ "kind": "api", "code": 2010 }),
        outcome: codex_game_model3d::Model3dAuditOutcome::Failure,
        duration_ms: 12,
    });

    let audit_path = character_dir
        .join("tmp/conversation")
        .join(format!("model3d-{}.md", job.id));
    let document = std::fs::read_to_string(audit_path).expect("test setup");
    assert!(document.contains("# 3D 模型生成审计"));
    assert!(document.contains("## multiview_to_model"));
    assert!(document.contains("- Outcome：failure"));
    assert!(document.contains("\"face_limit\": 5000"));
    assert!(document.contains("\"code\": 2010"));
}

fn pending_job() -> Model3dJob {
    Model3dJob {
        id: "job-1".to_string(),
        project_id: "project-1".to_string(),
        character_id: "hero".to_string(),
        provider_code: TRIPO_PROVIDER_CODE.to_string(),
        source_hash: "source".to_string(),
        pipeline_version: MODEL3D_PIPELINE_VERSION,
        status: Model3dJobStatus::Running,
        stage: Model3dStage::UploadingViews,
        inferred_rig_kind: codex_game_domain::RigKind::Other,
        rig_kind: None,
        checkpoint: Model3dCheckpoint::default(),
        asset: None,
        error: None,
        created_at: 1,
        updated_at: 1,
    }
}

fn complete_glb() -> Vec<u8> {
    let mut document = serde_json::to_vec(&json!({
        "asset": { "version": "2.0" },
        "meshes": [{}],
        "materials": [{}],
        "textures": [{}],
        "images": [{ "bufferView": 0, "mimeType": "image/png" }],
        "skins": [{}],
        "animations": [
            { "name": "Idle" },
            { "name": "Walk" },
            { "name": "Run" }
        ]
    }))
    .expect("test setup");
    while !document.len().is_multiple_of(4) {
        document.push(b' ');
    }
    let total_len = 20 + document.len();
    let mut bytes = Vec::with_capacity(total_len);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&(total_len as u32).to_le_bytes());
    bytes.extend_from_slice(&(document.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
    bytes.extend_from_slice(&document);
    bytes
}
