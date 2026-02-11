//! Permission engine types.

use super::pending::PendingRequest;

/// Response to a permission request from a client.
#[derive(Debug, Clone)]
pub struct PermissionResponse {
    /// Request ID being responded to.
    pub request_id: String,
    /// Whether permission was granted.
    pub granted: bool,
    /// Whether to remember this decision for the session.
    pub remember_session: bool,
    /// Whether to remember this decision permanently.
    pub remember_permanent: bool,
}

/// Session-scoped permission grant.
#[derive(Debug, Clone)]
pub struct SessionGrant {
    pub tool_name: String,
    pub path_pattern: Option<String>,
    pub granted: bool,
}

/// Result of permission evaluation.
#[derive(Debug)]
pub enum PermissionEvaluation {
    /// Permission allowed (immediately).
    Allowed { cached: bool },
    /// Permission denied (immediately).
    Denied { cached: bool },
    /// Permission requires user approval (request created).
    Pending { request: PendingRequest },
}

/// Result of processing a permission response.
#[derive(Debug)]
pub struct ProcessedResponse {
    /// The original request.
    pub request: PendingRequest,
    /// Whether permission was granted.
    pub granted: bool,
}

/// Permission engine errors.
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("Permission request not found: {request_id}")]
    RequestNotFound { request_id: String },

    #[error("Database error: {0}")]
    Database(String),
}
