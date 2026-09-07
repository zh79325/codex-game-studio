//! 校验手动导入的 GLB 角色资产。
//!
//! 平台手动导出的模型不经过流水线，因此需要一条独立的校验入口来确认
//! 网格、蒙皮、动画与贴图是否完备。
//!
//! ```bash
//! CODEX_GLB_PATH=/path/to/角色.glb \
//! cargo nextest run -p codex-game-model3d \
//!   -E 'test(validates_imported_glb_asset)' --run-ignored all --no-capture
//! ```

use super::*;
use crate::expected_animation_names;
use codex_game_domain::RigKind;
use std::path::PathBuf;

const GLB_PATH_ENV: &str = "CODEX_GLB_PATH";

#[ignore = "校验本地 GLB 文件；需通过 CODEX_GLB_PATH 指定路径"]
#[test]
fn validates_imported_glb_asset() {
    let path = PathBuf::from(
        std::env::var(GLB_PATH_ENV)
            .unwrap_or_else(|_| panic!("设置 {GLB_PATH_ENV} 指向要校验的 .glb 文件")),
    );
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    eprintln!("[glb] {} （{} 字节）", path.display(), bytes.len());

    let expected = expected_animation_names(RigKind::Biped);
    eprintln!("[glb] 流水线期望的动画：{expected:?}");

    match validate_complete_glb(&bytes, &expected) {
        Ok(summary) => {
            eprintln!("[glb] 校验通过：{summary:#?}");
        }
        Err(error) => {
            // 先报告实际内容，再暴露差异，便于判断是资产问题还是期望值问题。
            let relaxed = validate_complete_glb(&bytes, &[])
                .expect("GLB 结构本身应可解析（网格/材质/贴图/蒙皮齐备）");
            eprintln!("[glb] 结构完备，但动画期望不匹配：{relaxed:#?}");
            panic!("按流水线期望校验失败：{error}");
        }
    }
}
