mod artifact;
mod backend;
mod dialect;
mod extension;
mod tool;

pub use dialect::ImageApiDialect;
pub use extension::ImageGenerationRouteOverride;
pub use extension::ImageGenerationToolRouteOverride;
pub use extension::install;
pub use tool::ImageGenerationTurnGate;

pub(crate) const IMAGE_GEN_NAMESPACE: &str = "image_gen";
pub(crate) const IMAGEGEN_TOOL_NAME: &str = "imagegen";
