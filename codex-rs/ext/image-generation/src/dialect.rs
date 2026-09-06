/// Identifies the provider-specific wire contract used for image requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageApiDialect {
    OpenAi,
    Ark,
}

impl TryFrom<&str> for ImageApiDialect {
    type Error = String;

    fn try_from(driver: &str) -> Result<Self, Self::Error> {
        match driver {
            "openai" | "openai_compat" => Ok(Self::OpenAi),
            "ark_image" => Ok(Self::Ark),
            other => Err(format!("不支持的图片模型 driver：{other}")),
        }
    }
}

#[cfg(test)]
#[path = "dialect_tests.rs"]
mod tests;
