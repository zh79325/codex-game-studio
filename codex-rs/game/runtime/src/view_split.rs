use image::GenericImageView;
use image::ImageFormat;
use image::ImageReader;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

// 侧视名称按角色自身方向定义：右视图脸朝画面左侧，左视图脸朝画面右侧。
const QUAD_LAYOUT: [(&str, u32, u32); 4] = [
    ("front", 0, 0),
    ("right", 1, 0),
    ("back", 0, 1),
    ("left", 1, 1),
];

pub(crate) fn split_quad_to_png_files(
    source: &Path,
    targets: &BTreeMap<String, PathBuf>,
) -> io::Result<()> {
    let image = ImageReader::open(source)?
        .with_guessed_format()?
        .decode()
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let (width, height) = image.dimensions();
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "quad view image dimensions must be even",
        ));
    }
    let view_width = width / 2;
    let view_height = height / 2;
    for (view, column, row) in QUAD_LAYOUT {
        let target = targets.get(view).ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("missing output path for {view} view"),
            )
        })?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        image
            .crop_imm(
                column * view_width,
                row * view_height,
                view_width,
                view_height,
            )
            .save_with_format(target, ImageFormat::Png)
            .map_err(io::Error::other)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "view_split_tests.rs"]
mod tests;
