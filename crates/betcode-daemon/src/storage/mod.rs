//! SQLite storage for BetCode daemon.
//!
//! Provides persistence for sessions, messages, worktrees, and permissions.

mod db;
mod models;
mod queries;

pub use db::{Database, DatabaseError};
pub use models::*;
