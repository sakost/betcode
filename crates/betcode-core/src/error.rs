//! Error types for `BetCode` core library.

use thiserror::Error;

/// Result type alias using `BetCode` Error.
pub type Result<T> = std::result::Result<T, Error>;

/// Core error types for `BetCode` operations.
#[derive(Debug, Error)]
pub enum Error {
    /// NDJSON parsing error
    #[error("Failed to parse NDJSON: {0}")]
    NdjsonParse(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Permission rule error
    #[error("Permission rule error: {0}")]
    Permission(String),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
