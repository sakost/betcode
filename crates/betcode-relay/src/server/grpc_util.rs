//! Shared gRPC utility helpers.

/// Check if a gRPC Status represents a normal peer disconnect
/// (client exit, daemon shutdown, TLS close without notify, etc.).
pub fn is_peer_disconnect(status: &tonic::Status) -> bool {
    let msg = status.message();
    msg.contains("h2 protocol error")
        || msg.contains("broken pipe")
        || msg.contains("connection reset")
        || msg.contains("close_notify")
}
