//! BetCode Core Library
//!
//! Shared functionality for BetCode components:
//! - NDJSON parsing for Claude Code stream-json protocol
//! - Configuration resolution and hierarchy
//! - Permission rule matching engine
//! - Common error types

pub mod error;

// TODO: Sprint 1.2 - Implement these modules
// pub mod ndjson;
// pub mod config;
// pub mod permissions;

pub use error::{Error, Result};
