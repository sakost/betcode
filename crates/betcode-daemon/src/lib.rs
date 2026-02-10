//! BetCode Daemon Library
//!
//! Core functionality for the BetCode daemon:
//! - Subprocess management for Claude Code processes
//! - SQLite storage for sessions and messages
//! - gRPC server for client connections
//! - Session multiplexing for multi-client support
//! - Permission bridge for tool authorization

pub mod commands;
pub mod completion;
pub mod gitlab;
pub mod permission;
pub mod plugin;
pub mod relay;
pub mod server;
pub mod session;
pub mod storage;
pub mod subprocess;
pub mod tunnel;
pub mod worktree;
