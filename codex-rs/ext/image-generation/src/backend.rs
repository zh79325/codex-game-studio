use codex_api::ArkImageGenerationRequest;
use codex_api::ArkImageInput;
use codex_api::ArkImageOutputFormat;
use codex_api::ArkImageResponseFormat;
use codex_api::ArkSequentialImageGeneration;
use codex_api::ImageEditRequest;
use codex_api::ImageGenerationRequest;
use codex_api::ImageResponse;
use codex_api::ImagesClient;
use codex_api::ReqwestTransport;
use codex_api::map_api_error;
use codex_login::default_client::add_originator_header;
use codex_login::default_client::create_client;
use codex_model_provider::SharedModelProvider;
use codex_protocol::error::CodexErr;
use http::HeaderMap;
use http::HeaderValue;

use crate::dialect::ImageApiDialect;

const X_CODEX_IMAGE_TURN_ID_HEADER: &str = "x-codex-image-turn-id";

pub(crate) struct ImageBackendError {
    message: String,
    codex_error: CodexErr,
}

impl ImageBackendError {
    fn from_api(error: codex_api::ApiError) -> Self {
        let message = error.to_string();
        Self {
            message,
            codex_error: map_api_error(error),
        }
    }

    fn from_message(message: String) -> Self {
        Self {
            codex_error: CodexErr::Stream(message.clone()),
            message,
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn codex_error(&self) -> &CodexErr {
        &self.codex_error
    }
}

#[derive(Clone)]
pub(crate) struct CodexImagesBackend {
    provider: SharedModelProvider,
    originator: Option<String>,
    api_dialect: ImageApiDialect,
}

impl CodexImagesBackend {
    /// Creates a backend that sends image requests through the active model provider.
    pub(crate) fn new(
        provider: SharedModelProvider,
        originator: Option<String>,
        api_dialect: ImageApiDialect,
    ) -> Self {
        Self {
            provider,
            originator,
            api_dialect,
        }
    }

    /// Resolves the provider and auth required for the current image API request.
    async fn client(&self) -> Result<ImagesClient<ReqwestTransport>, ImageBackendError> {
        let provider = self
            .provider
            .api_provider()
            .await
            .map_err(|err| ImageBackendError::from_message(err.to_string()))?;
        let auth = self
            .provider
            .api_auth()
            .await
            .map_err(|err| ImageBackendError::from_message(err.to_string()))?;
        Ok(ImagesClient::new(
            ReqwestTransport::from_http_client(create_client()),
            provider,
            auth,
        ))
    }

    /// Sends a standalone image generation request through the configured Images client.
    pub(crate) async fn generate(
        &self,
        request: ImageGenerationRequest,
        turn_id: &str,
    ) -> Result<(ImageResponse, Option<String>), ImageBackendError> {
        let client = self.client().await?;
        let headers = image_request_headers(self.originator.as_deref(), turn_id);
        match self.api_dialect {
            ImageApiDialect::OpenAi => client.generate(&request, headers).await,
            ImageApiDialect::Ark => {
                client
                    .generate_ark(
                        &ark_request(request.model, request.prompt, Vec::new()),
                        headers,
                    )
                    .await
            }
        }
        .map_err(ImageBackendError::from_api)
    }

    /// Sends a standalone image edit request through the configured Images client.
    pub(crate) async fn edit(
        &self,
        request: ImageEditRequest,
        turn_id: &str,
    ) -> Result<(ImageResponse, Option<String>), ImageBackendError> {
        let client = self.client().await?;
        let headers = image_request_headers(self.originator.as_deref(), turn_id);
        match self.api_dialect {
            ImageApiDialect::OpenAi => client.edit(&request, headers).await,
            ImageApiDialect::Ark => {
                let images = request
                    .images
                    .into_iter()
                    .map(|image| image.image_url)
                    .collect();
                client
                    .generate_ark(&ark_request(request.model, request.prompt, images), headers)
                    .await
            }
        }
        .map_err(ImageBackendError::from_api)
    }
}

fn ark_request(model: String, prompt: String, images: Vec<String>) -> ArkImageGenerationRequest {
    let image = match images.as_slice() {
        [] => None,
        [image] => Some(ArkImageInput::Single(image.clone())),
        _ => Some(ArkImageInput::Multiple(images)),
    };
    let normalized_model = model.to_ascii_lowercase().replace('.', "-");
    let seedream_5 = normalized_model.contains("seedream-5");
    let seedream_5_pro = normalized_model.contains("seedream-5-0-pro");
    ArkImageGenerationRequest {
        model,
        prompt,
        image,
        size: "2K".to_string(),
        response_format: ArkImageResponseFormat::B64Json,
        sequential_image_generation: (!seedream_5_pro)
            .then_some(ArkSequentialImageGeneration::Disabled),
        stream: false,
        watermark: false,
        output_format: seedream_5.then_some(ArkImageOutputFormat::Png),
    }
}

fn image_request_headers(originator: Option<&str>, turn_id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(turn_id) = HeaderValue::from_str(turn_id) {
        headers.insert(X_CODEX_IMAGE_TURN_ID_HEADER, turn_id);
    }
    if let Some(originator) = originator {
        add_originator_header(&mut headers, originator);
    }
    headers
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
