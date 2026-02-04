//! Tunnel client error types.

/// Errors that can occur in the tunnel client.
#[derive(Debug, thiserror::Error)]
pub enum TunnelClientError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Registration error: {0}")]
    Registration(String),

    #[error("Stream error: {0}")]
    Stream(String),
}
