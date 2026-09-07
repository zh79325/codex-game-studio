use crate::Model3dAuditCall;
use crate::Model3dAuditOutcome;
use crate::Model3dAuditSink;
use crate::Model3dError;
use crate::Model3dProvider;
use crate::Model3dProviderTask;
use crate::Result;
use crate::wait_timeout;
use codex_game_domain::RigKind;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tripo3d_sdk::ClientOptions;
use tripo3d_sdk::TripoClient;
use tripo3d_sdk::WaitOptions;
use tripo3d_sdk::constants::model_version;
use tripo3d_sdk::models::FileInput;
use tripo3d_sdk::params::MultiviewToModelParams;
use tripo3d_sdk::params::RetargetAnimationParams;
use tripo3d_sdk::params::RigCheckParams;
use tripo3d_sdk::params::RigModelParams;

pub struct TripoModel3dProvider {
    client: TripoClient,
    audit: Option<Arc<dyn Model3dAuditSink>>,
}

impl TripoModel3dProvider {
    pub fn new(api_key: String) -> Result<Self> {
        let client = TripoClient::new(ClientOptions {
            api_key: Some(api_key),
            timeout: Some(std::time::Duration::from_secs(90)),
            user_agent: Some("codex-game-studio/model3d".to_string()),
            ..Default::default()
        })
        .map_err(|error| provider_failure(error).0)?;
        Ok(Self {
            client,
            audit: None,
        })
    }

    pub fn with_audit_sink(mut self, sink: Arc<dyn Model3dAuditSink>) -> Self {
        self.audit = Some(sink);
        self
    }

    /// Runs one provider call and records its exact request payload alongside the
    /// decoded response or the structured provider error.
    async fn audited<T>(
        &self,
        method: &str,
        request: Value,
        call: impl Future<Output = std::result::Result<T, tripo3d_sdk::Error>>,
        response_of: impl FnOnce(&T) -> Value,
    ) -> Result<T> {
        let started = Instant::now();
        match call.await {
            Ok(value) => {
                let response = response_of(&value);
                self.record(
                    method,
                    request,
                    response,
                    Model3dAuditOutcome::Success,
                    started,
                );
                Ok(value)
            }
            Err(error) => {
                let (converted, response) = provider_failure(error);
                self.record(
                    method,
                    request,
                    response,
                    Model3dAuditOutcome::Failure,
                    started,
                );
                Err(converted)
            }
        }
    }

    fn record(
        &self,
        method: &str,
        request: Value,
        response: Value,
        outcome: Model3dAuditOutcome,
        started: Instant,
    ) {
        let Some(sink) = &self.audit else {
            return;
        };
        sink.record(&Model3dAuditCall {
            method: method.to_string(),
            request,
            response,
            outcome,
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }
}

impl Model3dProvider for TripoModel3dProvider {
    async fn upload_image(&self, path: &Path) -> Result<String> {
        let bytes = tokio::fs::read(path).await?;
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("view.png");
        self.audited(
            "upload_file",
            json!({
                "filename": filename,
                "mimeType": "image/png",
                "bytes": bytes.len(),
            }),
            self.client.upload_file(bytes, filename, Some("image/png")),
            |uploaded| json!({ "fileToken": uploaded.file_token }),
        )
        .await
        .map(|uploaded| uploaded.file_token)
    }

    async fn generate_textured_model(&self, view_tokens: [String; 4]) -> Result<String> {
        let mut params = MultiviewToModelParams::from_files(
            view_tokens.map(|token| Some(FileInput::FileToken(token))),
        );
        params.model = Some(model_version::P1.to_string());
        params.face_limit = Some(5000);
        params.texture = Some(true);
        self.audited(
            "multiview_to_model",
            payload(&params),
            self.client.multiview_to_model(params.clone()),
            task_id_response,
        )
        .await
    }

    async fn check_rig(&self, model_task_id: &str) -> Result<String> {
        let params = RigCheckParams::new(model_task_id);
        self.audited(
            "rig_check",
            payload(&params),
            self.client.rig_check(params.clone()),
            task_id_response,
        )
        .await
    }

    async fn rig_model(&self, model_task_id: &str, rig_type: &str) -> Result<String> {
        let params = RigModelParams {
            rig_type: Some(rig_type.to_string()),
            spec: Some("mixamo".to_string()),
            ..RigModelParams::new(model_task_id)
        };
        self.audited(
            "rig_model",
            payload(&params),
            self.client.rig_model(params.clone()),
            task_id_response,
        )
        .await
    }

    async fn retarget_animation(&self, rig_task_id: &str, animations: &[String]) -> Result<String> {
        let params = RetargetAnimationParams {
            animations: Some(animations.to_vec()),
            out_format: Some("glb".to_string()),
            ..RetargetAnimationParams::new(rig_task_id)
        };
        self.audited(
            "retarget_animation",
            payload(&params),
            self.client.retarget_animation(params.clone()),
            task_id_response,
        )
        .await
    }

    async fn wait_for_task(&self, task_id: &str) -> Result<Model3dProviderTask> {
        let options = WaitOptions {
            timeout: Some(wait_timeout()),
            ..Default::default()
        };
        let task = self
            .audited(
                "wait_for_task",
                json!({
                    "taskId": task_id,
                    "timeoutMs": wait_timeout().as_millis(),
                }),
                self.client.wait_for_task(task_id, options),
                |task| serde_json::to_value(task).unwrap_or(Value::Null),
            )
            .await?;
        let output = task.output.unwrap_or_default();
        Ok(Model3dProviderTask {
            id: task.task_id,
            riggable: output.riggable,
            rig_type: output.rig_type.clone(),
            rig_kind: output.rig_type.as_deref().map(RigKind::from_provider_value),
        })
    }

    async fn download_artifact(&self, task_id: &str) -> Result<Vec<u8>> {
        let task = self
            .audited(
                "get_task",
                json!({ "taskId": task_id }),
                self.client.get_task(task_id),
                |task| serde_json::to_value(task).unwrap_or(Value::Null),
            )
            .await?;
        self.audited(
            "download_model",
            json!({ "taskId": task_id }),
            self.client.download_model(&task),
            |downloaded| {
                json!({
                    "downloaded": downloaded.is_some(),
                    "bytes": downloaded.as_ref().map(|model| model.data.len()),
                })
            },
        )
        .await?
        .map(|downloaded| downloaded.data)
        .ok_or(Model3dError::MissingArtifact)
    }
}

fn payload<T: Serialize>(params: &T) -> Value {
    serde_json::to_value(params).unwrap_or(Value::Null)
}

fn task_id_response(task_id: &String) -> Value {
    json!({ "taskId": task_id })
}

/// Converts an SDK error into the domain error plus its audit representation.
/// `Error::Api` carries a well-formed provider envelope, which means the request
/// was refused outright rather than left in an unknown state.
fn provider_failure(error: tripo3d_sdk::Error) -> (Model3dError, Value) {
    let rendered = error.to_string();
    let response = match &error {
        tripo3d_sdk::Error::Api {
            code,
            message,
            suggestion,
            status,
        } => json!({
            "kind": "api",
            "code": code,
            "message": message,
            "suggestion": suggestion,
            "httpStatus": status,
        }),
        tripo3d_sdk::Error::Request { status, body, .. } => json!({
            "kind": "request",
            "httpStatus": status,
            "body": body,
        }),
        tripo3d_sdk::Error::Task { task } => json!({
            "kind": "task",
            "task": serde_json::to_value(task).unwrap_or(Value::Null),
        }),
        tripo3d_sdk::Error::Timeout {
            task_id,
            timeout_ms,
        } => json!({
            "kind": "timeout",
            "taskId": task_id,
            "timeoutMs": timeout_ms,
        }),
        tripo3d_sdk::Error::InvalidArgument(message) => json!({
            "kind": "invalidArgument",
            "message": message,
        }),
        tripo3d_sdk::Error::Io(source) => json!({
            "kind": "io",
            "message": source.to_string(),
        }),
        tripo3d_sdk::Error::Serde(source) => json!({
            "kind": "serde",
            "message": source.to_string(),
        }),
    };
    let converted = match error {
        tripo3d_sdk::Error::Api { code, .. } => Model3dError::ProviderRejected {
            code,
            message: rendered,
        },
        tripo3d_sdk::Error::Request { .. }
        | tripo3d_sdk::Error::Task { .. }
        | tripo3d_sdk::Error::Timeout { .. }
        | tripo3d_sdk::Error::InvalidArgument(_)
        | tripo3d_sdk::Error::Io(_)
        | tripo3d_sdk::Error::Serde(_) => Model3dError::Provider(rendered),
    };
    (converted, response)
}
