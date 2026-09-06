use pretty_assertions::assert_eq;

use super::ImageApiDialect;

#[test]
fn maps_image_drivers_to_wire_dialects() {
    assert_eq!(
        ImageApiDialect::try_from("openai"),
        Ok(ImageApiDialect::OpenAi)
    );
    assert_eq!(
        ImageApiDialect::try_from("openai_compat"),
        Ok(ImageApiDialect::OpenAi)
    );
    assert_eq!(
        ImageApiDialect::try_from("ark_image"),
        Ok(ImageApiDialect::Ark)
    );
}

#[test]
fn rejects_unknown_image_driver() {
    assert_eq!(
        ImageApiDialect::try_from("unknown"),
        Err("不支持的图片模型 driver：unknown".to_string())
    );
}
