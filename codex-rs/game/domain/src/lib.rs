mod action;
mod ai_config;
mod character;
mod conversation;
mod entities;
mod ids;

pub use action::*;
pub use ai_config::*;
pub use character::*;
pub use conversation::*;
pub use entities::*;
pub use ids::*;

/// Current lifecycle state of the game backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendStatus {
    Starting,
    Recovering,
    Ready,
    ReadOnly,
}
