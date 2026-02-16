//! `SQLite` storage for `BetCode` daemon.
//!
//! Provides persistence for sessions, messages, worktrees, and permissions.

mod db;
mod models;
mod queries;
mod queries_subagents;
mod repo_queries;

pub use db::{Database, DatabaseError};
pub use models::*;
pub use repo_queries::GitRepoParams;
