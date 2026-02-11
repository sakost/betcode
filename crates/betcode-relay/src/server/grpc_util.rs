//! Shared gRPC utility helpers.

use std::collections::HashMap;

use prost::Message;
use tonic::{Code, Status};

use crate::router::RequestRouter;
use crate::server::agent_proxy::{decode_response, router_error_to_status};

/// Check if a gRPC Status represents a normal peer disconnect
/// (client exit, daemon shutdown, TLS close without notify, etc.).
///
/// We first check the gRPC status code because it is the most reliable
/// indicator: `Unavailable` and `Cancelled` are the canonical codes that
/// tonic surfaces when the transport layer drops. If the code is something
/// else we fall back to substring matching on the message text.
///
/// NOTE: The substring checks below are **fragile** -- the exact wording
/// is an implementation detail of hyper / h2 / rustls and may change
/// across library versions.  Prefer adding new `Code` matches when
/// possible and only resort to substring matching for edge cases where
/// the code alone is ambiguous.
pub fn is_peer_disconnect(status: &tonic::Status) -> bool {
    // Primary signal: gRPC status code.
    match status.code() {
        Code::Unavailable | Code::Cancelled => return true,
        _ => {}
    }

    // Fallback: substring matching for cases where the code may be
    // Internal/Unknown but the root cause is still a transport disconnect.
    let msg = status.message();
    msg.contains("h2 protocol error")
        || msg.contains("broken pipe")
        || msg.contains("connection reset")
        || msg.contains("close_notify")
}

/// Forward a unary RPC request through the tunnel to a daemon and decode the response.
///
/// This is the shared implementation used by all proxy services
/// (`AgentProxyService`, `CommandProxyService`, `GitLabProxyService`, `WorktreeProxyService`).
pub async fn forward_unary<Req: Message, Resp: Message + Default>(
    router: &RequestRouter,
    machine_id: &str,
    method: &str,
    req: &Req,
) -> Result<Resp, Status> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut buf = Vec::with_capacity(req.encoded_len());
    req.encode(&mut buf)
        .map_err(|e| Status::internal(format!("Encode error: {e}")))?;
    let frame = router
        .forward_request(machine_id, &request_id, method, buf, HashMap::new())
        .await
        .map_err(router_error_to_status)?;
    decode_response(&frame)
}
