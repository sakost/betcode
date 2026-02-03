//! BetCode Daemon Library
//!
//! Core functionality for the BetCode daemon:
//! - Subprocess management for Claude Code processes
//! - SQLite storage for sessions and messages
//! - gRPC server for client connections
//! - Session multiplexing for multi-client support
//! - Permission bridge for tool authorization

pub mod permission;
pub mod server;
pub mod session;
pub mod storage;
pub mod subprocess;
