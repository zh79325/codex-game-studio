mod artifacts;
mod entities;
mod ids;
mod workflow;

pub use artifacts::*;
pub use entities::*;
pub use ids::*;
pub use workflow::*;

/// Current lifecycle state of the game backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendStatus {
    Starting,
    Recovering,
    Ready,
    ReadOnly,
}
