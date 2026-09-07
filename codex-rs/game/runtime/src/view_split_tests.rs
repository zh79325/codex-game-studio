use super::*;
use image::Rgba;
use image::RgbaImage;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[test]
fn splits_quad_into_named_png_views() {
    let directory = tempdir().expect("temp directory");
    let source = directory.path().join("views-final.png");
    let mut quad = RgbaImage::new(4, 4);
    let colors = [
        ("front", 0, 0, Rgba([255, 0, 0, 255])),
        ("right", 2, 0, Rgba([0, 255, 0, 255])),
        ("back", 0, 2, Rgba([0, 0, 255, 255])),
        ("left", 2, 2, Rgba([255, 255, 0, 255])),
    ];
    for (_, origin_x, origin_y, color) in colors {
        for y in origin_y..origin_y + 2 {
            for x in origin_x..origin_x + 2 {
                quad.put_pixel(x, y, color);
            }
        }
    }
    quad.save(&source).expect("save source image");
    let targets = colors
        .into_iter()
        .map(|(view, _, _, _)| {
            (
                view.to_string(),
                directory.path().join(format!("views-{view}.png")),
            )
        })
        .collect::<BTreeMap<_, _>>();

    split_quad_to_png_files(&source, &targets).expect("split quad image");

    let actual = colors
        .into_iter()
        .map(|(view, _, _, _)| {
            let output = image::open(&targets[view]).expect("read split view");
            (view, output.dimensions(), output.get_pixel(0, 0))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            ("front", (2, 2), Rgba([255, 0, 0, 255])),
            ("right", (2, 2), Rgba([0, 255, 0, 255])),
            ("back", (2, 2), Rgba([0, 0, 255, 255])),
            ("left", (2, 2), Rgba([255, 255, 0, 255])),
        ]
    );
}

#[test]
fn rejects_odd_quad_dimensions_before_writing_outputs() {
    let directory = tempdir().expect("temp directory");
    let source = directory.path().join("views-final.png");
    RgbaImage::new(5, 4)
        .save(&source)
        .expect("save source image");

    let error = split_quad_to_png_files(&source, &BTreeMap::new())
        .expect_err("odd dimensions must be rejected");

    assert_eq!(error.kind(), ErrorKind::InvalidData);
}
