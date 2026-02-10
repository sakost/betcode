//! Permission bridge between Claude subprocess and clients.
//!
//! Handles permission requests from Claude and routes them to connected clients.

mod engine;
mod pending;
mod types;

pub use engine::{DaemonPermissionEngine, PermissionEvalRequest};
pub use pending::{PendingConfig, PendingManager, PendingRequest};
pub use types::{PermissionError, PermissionEvaluation, PermissionResponse, ProcessedResponse};
