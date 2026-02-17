//! `BetCode` Core Library
//!
//! Shared functionality for `BetCode` components:
//! - NDJSON parsing for Claude Code stream-json protocol
//! - Configuration resolution and hierarchy
//! - Permission rule matching engine
//! - Common error types

pub mod commands;
pub mod config;
pub mod db;
pub mod error;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod ndjson;
pub mod permissions;
pub mod tracing_init;

pub use config::Config;
pub use error::{Error, Result};
pub use permissions::{PermissionAction, PermissionEngine, PermissionRule};
