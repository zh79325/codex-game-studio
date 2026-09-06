use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_api::ArkImageGenerationRequest;
use codex_api::ArkImageInput;
use codex_api::ArkImageOutputFormat;
use codex_api::ArkImageResponseFormat;
use codex_api::ArkSequentialImageGeneration;
use codex_api::ImageBackground;
use codex_api::ImageEditRequest;
use codex_api::ImageGenerationRequest;
use codex_api::ImageQuality;
use codex_api::ImageUrl;
use codex_game_app_server_adapter::GameAppServerAdapter;
use codex_game_runtime::RouteDecision;
use codex_game_store::load_ai_route_models;
use codex_game_store::open_studio_store;
use codex_model_provider::create_model_provider;
use codex_model_provider_info::ModelProviderInfo;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::time::SystemTime;

use super::CodexImagesBackend;
use super::ark_request;
use crate::dialect::ImageApiDialect;

#[test]
fn converts_seedream_lite_t2i_to_ark_contract() {
    assert_eq!(
        ark_request(
            "doubao-seedream-5.0-lite".to_string(),
            "孙悟空站在城市天台".to_string(),
            Vec::new(),
        ),
        ArkImageGenerationRequest {
            model: "doubao-seedream-5.0-lite".to_string(),
            prompt: "孙悟空站在城市天台".to_string(),
            image: None,
            size: "2K".to_string(),
            response_format: ArkImageResponseFormat::B64Json,
            sequential_image_generation: Some(ArkSequentialImageGeneration::Disabled),
            stream: false,
            watermark: false,
            output_format: Some(ArkImageOutputFormat::Png),
        }
    );
}

#[test]
fn converts_single_reference_i2i_to_ark_contract() {
    assert_eq!(
        ark_request(
            "doubao-seedream-4.5".to_string(),
            "调整角色姿势".to_string(),
            vec!["data:image/png;base64,Zm9v".to_string()],
        ),
        ArkImageGenerationRequest {
            model: "doubao-seedream-4.5".to_string(),
            prompt: "调整角色姿势".to_string(),
            image: Some(ArkImageInput::Single(
                "data:image/png;base64,Zm9v".to_string()
            )),
            size: "2K".to_string(),
            response_format: ArkImageResponseFormat::B64Json,
            sequential_image_generation: Some(ArkSequentialImageGeneration::Disabled),
            stream: false,
            watermark: false,
            output_format: None,
        }
    );
}

#[test]
fn omits_unsupported_group_mode_for_seedream_pro() {
    let request = ark_request(
        "doubao-seedream-5-0-pro-260628".to_string(),
        "生成角色立绘".to_string(),
        Vec::new(),
    );

    assert_eq!(request.sequential_image_generation, None);
    assert_eq!(request.output_format, Some(ArkImageOutputFormat::Png));
}

#[ignore = "调用当前 studio.db 中的付费图片 Provider；仅供显式手动执行"]
#[tokio::test]
async fn real_image_t2i_uses_current_studio_route() {
    let codex_home = PathBuf::from(
        std::env::var("CODEX_REAL_IMAGE_CODEX_HOME")
            .expect("set CODEX_REAL_IMAGE_CODEX_HOME to the project-local codex-home"),
    );
    let pool = open_studio_store(&codex_home)
        .await
        .expect("open current studio database");
    let now = SystemTime::UNIX_EPOCH
        .elapsed()
        .expect("system time should follow the Unix epoch")
        .as_secs() as i64;
    let route = load_ai_route_models(&pool, "image_t2i", now)
        .await
        .expect("load image_t2i routes")
        .into_iter()
        .find(|route| route.available)
        .expect("image_t2i should have an available database route");
    let route = RouteDecision {
        account_id: route.id,
        provider: route.provider,
        model: route.model,
    };
    let default_output_path = codex_home
        .parent()
        .expect("project-local codex-home should have a parent")
        .join("tmp/seedream-sun-wukong.png");
    let resolved = GameAppServerAdapter::new(codex_home)
        .resolve_ai_execution_route(&route)
        .await
        .expect("resolve image_t2i route and secret");
    assert_eq!(resolved.auth_style, "bearer");
    let api_dialect =
        ImageApiDialect::try_from(resolved.driver.as_str()).expect("supported image driver");
    let request_model =
        std::env::var("CODEX_REAL_IMAGE_MODEL").unwrap_or_else(|_| resolved.model.clone());
    let provider = ModelProviderInfo {
        name: resolved.provider_name,
        base_url: Some(resolved.base_url),
        experimental_bearer_token: Some(resolved.api_key.into()),
        stream_max_retries: Some(0),
        stream_idle_timeout_ms: Some(180_000),
        ..Default::default()
    };
    let backend = CodexImagesBackend::new(create_model_provider(provider, None), None, api_dialect);
    let prompt = "生成一张 1:1 方形、完整角色全身效果图。低多边形风格化孙悟空，180 cm、7 头身、精瘦肌肉、短吻猴面、琥珀金非发光双眼、两颗可见上犬齿、鎏金紧箍凸起；贴体短毛、赤足、每手五指、每脚五趾；仅一条连接身体的尾巴垂至脚踝，尾端五束金色簇毛；横置 180 cm 如意长棒，鎏金棒身、两端朱砂红金属箍。躯干侧转 20°，肩线左高右低，战斗姿态机敏锋利。背景为日落后现代都市天台，三栋远景高楼、少量霓虹紫广告牌、瓦砾棕地面，暖主光、冷补光、暖轮廓光。角色轮廓、四肢、武器和尾巴必须清晰分离。禁止披风、斗篷、长袍、宽大衣袖、额外尾巴、翅膀、棘刺、发光眼睛、写实人脸、裁切身体、文字、水印。";
    let (response, _) = backend
        .generate(
            ImageGenerationRequest {
                prompt: prompt.to_string(),
                background: Some(ImageBackground::Auto),
                model: request_model,
                n: None,
                quality: Some(ImageQuality::Auto),
                size: Some("auto".to_string()),
            },
            "real-seedream-t2i",
        )
        .await
        .unwrap_or_else(|error| panic!("real image_t2i failed: {}", error.message()));
    let image = response
        .data
        .first()
        .expect("real image_t2i should return image data");
    let bytes = BASE64_STANDARD
        .decode(image.b64_json.as_bytes())
        .expect("image_t2i should return valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let output_path = std::env::var("CODEX_REAL_IMAGE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or(default_output_path);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).expect("create generated image output directory");
    }
    std::fs::write(&output_path, bytes).expect("write generated image artifact");
    eprintln!("generated image saved to {}", output_path.display());
}

#[ignore = "调用当前 studio.db 中的付费图片 Provider；仅供显式手动执行"]
#[tokio::test]
async fn real_image_i2i_uses_current_studio_route() {
    let codex_home = PathBuf::from(
        std::env::var("CODEX_REAL_IMAGE_CODEX_HOME")
            .expect("set CODEX_REAL_IMAGE_CODEX_HOME to the project-local codex-home"),
    );
    let local_dir = codex_home
        .parent()
        .expect("project-local codex-home should have a parent");
    let reference_path = std::env::var("CODEX_REAL_IMAGE_I2I_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| local_dir.join("tmp/seedream-sun-wukong.png"));
    let output_path = std::env::var("CODEX_REAL_IMAGE_I2I_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| local_dir.join("tmp/seedream-sun-wukong-i2i.png"));
    let reference_bytes = std::fs::read(&reference_path).unwrap_or_else(|error| {
        panic!(
            "read i2i reference image at {}: {error}",
            reference_path.display()
        )
    });
    let reference_image_url = format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(reference_bytes)
    );

    let pool = open_studio_store(&codex_home)
        .await
        .expect("open current studio database");
    let now = SystemTime::UNIX_EPOCH
        .elapsed()
        .expect("system time should follow the Unix epoch")
        .as_secs() as i64;
    let route = load_ai_route_models(&pool, "image_i2i", now)
        .await
        .expect("load image_i2i routes")
        .into_iter()
        .find(|route| route.available)
        .expect("image_i2i should have an available database route");
    let route = RouteDecision {
        account_id: route.id,
        provider: route.provider,
        model: route.model,
    };
    let resolved = GameAppServerAdapter::new(codex_home)
        .resolve_ai_execution_route(&route)
        .await
        .expect("resolve image_i2i route and secret");
    assert_eq!(resolved.auth_style, "bearer");
    let api_dialect =
        ImageApiDialect::try_from(resolved.driver.as_str()).expect("supported image driver");
    let provider = ModelProviderInfo {
        name: resolved.provider_name,
        base_url: Some(resolved.base_url),
        experimental_bearer_token: Some(resolved.api_key.into()),
        stream_max_retries: Some(0),
        stream_idle_timeout_ms: Some(180_000),
        ..Default::default()
    };
    let backend = CodexImagesBackend::new(create_model_provider(provider, None), None, api_dialect);
    let (response, _) = backend
        .edit(
            ImageEditRequest {
                images: vec![ImageUrl {
                    image_url: reference_image_url,
                }],
                prompt: "保留孙悟空的低多边形角色设计、服装、金箍棒与城市天台背景，将动作改为双手持棒向前突进，确保完整全身、尾巴和武器轮廓清晰，不添加文字或水印。".to_string(),
                background: Some(ImageBackground::Auto),
                model: resolved.model,
                n: None,
                quality: Some(ImageQuality::Auto),
                size: Some("auto".to_string()),
            },
            "real-seedream-i2i",
        )
        .await
        .unwrap_or_else(|error| panic!("real image_i2i failed: {}", error.message()));
    let image = response
        .data
        .first()
        .expect("real image_i2i should return image data");
    let bytes = BASE64_STANDARD
        .decode(image.b64_json.as_bytes())
        .expect("image_i2i should return valid base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).expect("create generated image output directory");
    }
    std::fs::write(&output_path, bytes).expect("write generated image artifact");
    eprintln!("generated image saved to {}", output_path.display());
}
