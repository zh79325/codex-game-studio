//! 真实 Tripo 外网调用的定向测试。
//!
//! 全部用例默认 `#[ignore]`，必须显式指定用例名手动执行，避免误触发付费请求。
//! 三级递进：先用免费接口确认鉴权与余额，再验证上传通路，最后才跑付费建模。
//!
//! ```bash
//! # 1) 免费：确认密钥有效并打印真实余额
//! cargo nextest run -p codex-game-model3d \
//!   -E 'test(real_tripo_balance_reports_account_credit)' --run-ignored all
//!
//! # 2) 免费：确认 multipart 上传通路
//! CODEX_REAL_TRIPO_VIEWS=front.png,left.png,back.png,right.png \
//! cargo nextest run -p codex-game-model3d \
//!   -E 'test(real_tripo_upload_accepts_a_view)' --run-ignored all
//!
//! # 3) 付费：完整建模并落盘 GLB
//! CODEX_REAL_TRIPO_VIEWS=front.png,left.png,back.png,right.png \
//! CODEX_REAL_TRIPO_OUTPUT=/tmp/character-complete.glb \
//! cargo nextest run -p codex-game-model3d \
//!   -E 'test(real_tripo_full_pipeline_produces_animated_glb)' --run-ignored all
//! ```

use super::*;
use crate::animation_presets;
use crate::expected_animation_names;
use crate::validate_complete_glb;
use codex_game_domain::RigKind;
use std::path::PathBuf;

const SECRETS_ENV: &str = "CODEX_REAL_TRIPO_SECRETS";
const VIEWS_ENV: &str = "CODEX_REAL_TRIPO_VIEWS";
const OUTPUT_ENV: &str = "CODEX_REAL_TRIPO_OUTPUT";
const DEFAULT_SECRETS: &str = "../../../.codex-game/local/ai-secrets.json";

/// 把每次调用打到 stderr，便于在测试输出里直接看到请求与响应。
/// 头部在进入这里之前已脱敏，因此不会泄露密钥。
struct StderrAuditSink;

impl Model3dAuditSink for StderrAuditSink {
    fn record(&self, call: &Model3dAuditCall) {
        eprintln!(
            "\n[tripo] {} → {} ({} ms)\n  headers : {}\n  request : {}\n  response: {}",
            call.method,
            call.outcome.as_str(),
            call.duration_ms,
            call.headers,
            call.request,
            call.response,
        );
    }
}

/// 从 ai-secrets.json 的 `3dModelKeys.tripo3d-zzh` 读取真实密钥。
fn real_api_key() -> String {
    let path = std::env::var(SECRETS_ENV).unwrap_or_else(|_| DEFAULT_SECRETS.to_string());
    let document = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {path} 失败：{error}；可用 {SECRETS_ENV} 指定路径"));
    let secrets: Value = serde_json::from_str(&document).expect("ai-secrets.json 不是合法 JSON");
    secrets
        .get("3dModelKeys")
        .and_then(|keys| keys.get("tripo3d-zzh"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| panic!("{path} 缺少 3dModelKeys.tripo3d-zzh"))
        .to_string()
}

fn real_provider() -> TripoModel3dProvider {
    let api_key = real_api_key();
    eprintln!("[tripo] 使用密钥 {}", mask_bearer(&api_key));
    TripoModel3dProvider::new(api_key)
        .expect("构建 Tripo provider")
        .with_audit_sink(Arc::new(StderrAuditSink))
}

/// 按 `front,left,back,right` 顺序读取四视图路径。
fn real_view_paths() -> [PathBuf; 4] {
    let raw = std::env::var(VIEWS_ENV).unwrap_or_else(|_| {
        panic!("设置 {VIEWS_ENV} 为按 front,left,back,right 顺序的四个图片路径")
    });
    let paths = raw
        .split(',')
        .map(|value| PathBuf::from(value.trim()))
        .collect::<Vec<_>>();
    let paths: [PathBuf; 4] = paths.try_into().unwrap_or_else(|values: Vec<PathBuf>| {
        panic!("{VIEWS_ENV} 需要 4 个路径，实际 {}", values.len())
    });
    for path in &paths {
        assert!(path.is_file(), "四视图文件不存在：{}", path.display());
    }
    paths
}

/// 免费调用。直接回答「密钥是否有效」与「账户余额到底是多少」，
/// 这是 `code=2010`（余额不足）唯一的权威依据。
#[ignore = "调用真实 Tripo 账户接口（免费）；仅供显式手动执行"]
#[tokio::test]
async fn real_tripo_balance_reports_account_credit() {
    let api_key = real_api_key();
    eprintln!("[tripo] 使用密钥 {}", mask_bearer(&api_key));
    let client = TripoClient::new(ClientOptions {
        api_key: Some(api_key),
        timeout: Some(std::time::Duration::from_secs(30)),
        user_agent: Some(USER_AGENT.to_string()),
        ..Default::default()
    })
    .expect("构建 Tripo 客户端");

    match client.get_balance().await {
        Ok(balance) => {
            eprintln!(
                "[tripo] 余额={} 冻结={:?} 其余字段={:?}",
                balance.balance, balance.frozen, balance.extra
            );
            assert!(
                balance.balance > 0.0,
                "账户可用余额为 {}，无法创建付费任务，这正是 code=2010 的来源",
                balance.balance
            );
        }
        Err(error) => {
            let (converted, detail) = provider_failure(error);
            panic!("查询余额失败：{converted}\n结构化响应：{detail}");
        }
    }
}

/// 免费调用。验证 multipart 上传通路与 bearer 鉴权是否打通。
#[ignore = "调用真实 Tripo 上传接口（免费）；仅供显式手动执行"]
#[tokio::test]
async fn real_tripo_upload_accepts_a_view() {
    let provider = real_provider();
    let [front, ..] = real_view_paths();

    let token = provider
        .upload_image(&front)
        .await
        .unwrap_or_else(|error| panic!("上传 {} 失败：{error}", front.display()));

    eprintln!("[tripo] file_token={token}");
    assert!(!token.trim().is_empty(), "上传应返回非空 file_token");
}

/// 付费调用。完整跑通 四视图上传 → P1 低面建模 → 可绑骨检查 → Mixamo 绑骨
/// → 预设动画烘焙 → 下载校验，并把 GLB 落盘。
#[ignore = "调用真实 Tripo 付费建模接口，会消耗账户余额；仅供显式手动执行"]
#[tokio::test]
async fn real_tripo_full_pipeline_produces_animated_glb() {
    let provider = real_provider();
    let view_paths = real_view_paths();

    let mut tokens = Vec::with_capacity(4);
    for path in &view_paths {
        tokens.push(
            provider
                .upload_image(path)
                .await
                .unwrap_or_else(|error| panic!("上传 {} 失败：{error}", path.display())),
        );
    }
    let view_tokens: [String; 4] = tokens.try_into().expect("四视图应产生 4 个 token");

    let model_task_id = provider
        .generate_textured_model(view_tokens)
        .await
        .unwrap_or_else(|error| panic!("提交建模任务失败：{error}"));
    eprintln!("[tripo] model_task_id={model_task_id}");
    provider
        .wait_for_task(&model_task_id)
        .await
        .unwrap_or_else(|error| panic!("建模任务失败：{error}"));

    let check_task_id = provider
        .check_rig(&model_task_id)
        .await
        .unwrap_or_else(|error| panic!("提交绑骨检查失败：{error}"));
    let check = provider
        .wait_for_task(&check_task_id)
        .await
        .unwrap_or_else(|error| panic!("绑骨检查失败：{error}"));
    eprintln!(
        "[tripo] riggable={:?} rig_type={:?}",
        check.riggable, check.rig_type
    );
    assert_eq!(check.riggable, Some(true), "模型不可绑骨，需调整四视图");
    let rig_type = check.rig_type.expect("绑骨检查应返回 rig_type");

    let rig_task_id = provider
        .rig_model(&model_task_id, &rig_type)
        .await
        .unwrap_or_else(|error| panic!("提交绑骨失败：{error}"));
    provider
        .wait_for_task(&rig_task_id)
        .await
        .unwrap_or_else(|error| panic!("绑骨任务失败：{error}"));

    let rig_kind = check.rig_kind.unwrap_or(RigKind::Biped);
    let animations = animation_presets(rig_kind);
    let animation_task_id = provider
        .retarget_animation(&rig_task_id, &animations)
        .await
        .unwrap_or_else(|error| panic!("提交动画烘焙失败：{error}"));
    provider
        .wait_for_task(&animation_task_id)
        .await
        .unwrap_or_else(|error| panic!("动画烘焙失败：{error}"));

    let bytes = provider
        .download_artifact(&animation_task_id)
        .await
        .unwrap_or_else(|error| panic!("下载 GLB 失败：{error}"));
    let summary = validate_complete_glb(&bytes, &expected_animation_names(rig_kind))
        .unwrap_or_else(|error| panic!("GLB 校验失败：{error}"));
    eprintln!(
        "[tripo] GLB {} 字节，动画={:?}",
        bytes.len(),
        summary.animation_names
    );

    let output_path = std::env::var(OUTPUT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("tripo-character-complete.glb"));
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).expect("创建 GLB 输出目录");
    }
    std::fs::write(&output_path, &bytes).expect("写出 GLB");
    eprintln!("[tripo] GLB 已保存到 {}", output_path.display());
}
