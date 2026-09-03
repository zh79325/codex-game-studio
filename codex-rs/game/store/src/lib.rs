mod database;
mod project_files;
mod studio;

pub use database::*;
pub use project_files::*;
pub use studio::*;

use codex_game_domain::BackendStatus;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;

/// Process-local recovery barrier shared by game RPC handlers.
#[derive(Debug)]
pub struct StoreState {
    status: AtomicU8,
}

impl StoreState {
    pub fn new(status: BackendStatus) -> Self {
        Self {
            status: AtomicU8::new(status_code(status)),
        }
    }

    pub fn status(&self) -> BackendStatus {
        match self.status.load(Ordering::Acquire) {
            0 => BackendStatus::Starting,
            1 => BackendStatus::Recovering,
            3 => BackendStatus::ReadOnly,
            _ => BackendStatus::Ready,
        }
    }

    pub fn set_status(&self, status: BackendStatus) {
        self.status.store(status_code(status), Ordering::Release);
    }
}

fn status_code(status: BackendStatus) -> u8 {
    match status {
        BackendStatus::Starting => 0,
        BackendStatus::Recovering => 1,
        BackendStatus::Ready => 2,
        BackendStatus::ReadOnly => 3,
    }
}
