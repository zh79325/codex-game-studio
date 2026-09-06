use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<ImageBackground>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<ImageQuality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImageEditRequest {
    pub images: Vec<ImageUrl>,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<ImageBackground>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<ImageQuality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArkImageGenerationRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ArkImageInput>,
    pub size: String,
    pub response_format: ArkImageResponseFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequential_image_generation: Option<ArkSequentialImageGeneration>,
    pub stream: bool,
    pub watermark: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<ArkImageOutputFormat>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ArkImageInput {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArkImageResponseFormat {
    B64Json,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArkSequentialImageGeneration {
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArkImageOutputFormat {
    Png,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageUrl {
    pub image_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageBackground {
    Transparent,
    Opaque,
    Auto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageQuality {
    Low,
    Medium,
    High,
    Auto,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ImageResponse {
    pub created: u64,
    pub data: Vec<ImageData>,
    #[serde(default)]
    pub background: Option<ImageBackground>,
    #[serde(default)]
    pub quality: Option<ImageQuality>,
    #[serde(default)]
    pub size: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ImageData {
    pub b64_json: String,
}
