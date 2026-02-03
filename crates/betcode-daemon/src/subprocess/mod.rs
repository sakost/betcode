//! Subprocess management for Claude Code processes.

pub mod bridge;
pub mod manager;

pub use bridge::EventBridge;
pub use manager::{ProcessHandle, SpawnConfig, SubprocessError, SubprocessManager};
