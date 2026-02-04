//! Git worktree management for BetCode daemon.
//!
//! Manages git worktrees and their association with Claude sessions.
//! Each worktree gets its own working directory and can have sessions bound to it.

mod manager;

pub use manager::{WorktreeError, WorktreeInfo, WorktreeManager};
