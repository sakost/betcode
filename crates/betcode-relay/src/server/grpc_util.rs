//! Shared gRPC utility helpers.

use std::collections::HashMap;
use std::pin::Pin;

use prost::Message;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status};
use tracing::warn;

use betcode_proto::v1::{FrameType, TunnelFrame, tunnel_frame};

use crate::router::RequestRouter;
use crate::server::agent_proxy::{decode_response, router_error_to_status};
use crate::storage::{DatabaseError, RelayDatabase};

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

/// Verify the caller owns the given machine, returning `NOT_FOUND` or
/// `PERMISSION_DENIED` on failure.
#[allow(clippy::result_large_err)]
pub async fn verify_machine_ownership(
    db: &RelayDatabase,
    machine_id: &str,
    user_id: &str,
) -> Result<(), Status> {
    let machine = db.get_machine(machine_id).await.map_err(|e| match e {
        DatabaseError::NotFound(_) => Status::not_found("Machine not found"),
        other => {
            warn!(error = %other, machine_id, "DB error during ownership check");
            Status::internal("Internal error")
        }
    })?;

    if machine.owner_id != user_id {
        return Err(Status::permission_denied("Not your machine"));
    }
    Ok(())
}

/// Generate a new request ID and encode a protobuf message into a buffer.
///
/// This is the shared preamble for `forward_unary` and `forward_server_stream`.
fn encode_request<Req: Message>(req: &Req) -> Result<(String, Vec<u8>), Status> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut buf = Vec::with_capacity(req.encoded_len());
    req.encode(&mut buf)
        .map_err(|e| Status::internal(format!("Encode error: {e}")))?;
    Ok((request_id, buf))
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
    let (request_id, buf) = encode_request(req)?;
    let frame = router
        .forward_request(machine_id, &request_id, method, buf, HashMap::new())
        .await
        .map_err(router_error_to_status)?;
    decode_response(&frame)
}

/// Forward a unary RPC request end-to-end: extract claims and machine-id from the
/// gRPC `Request`, forward through the tunnel, decode the response, and wrap it in
/// `Response`.
///
/// This is the one-liner that every unary proxy method delegates to.
pub async fn forward_unary_rpc<Req: Message, Resp: Message + Default>(
    router: &RequestRouter,
    db: &RelayDatabase,
    request: Request<Req>,
    method: &str,
) -> Result<Response<Resp>, Status> {
    let claims = crate::server::interceptor::extract_claims(&request)?;
    let machine_id = crate::server::agent_proxy::extract_machine_id(&request)?;
    verify_machine_ownership(db, &machine_id, &claims.sub).await?;
    let resp = forward_unary(router, &machine_id, method, &request.into_inner()).await?;
    Ok(Response::new(resp))
}

/// Forward a server-streaming RPC end-to-end: extract claims and machine-id from the
/// gRPC `Request`, forward through the tunnel, and wrap the result stream in `Response`.
///
/// This is the streaming counterpart of `forward_unary_rpc`.
pub async fn forward_stream_rpc<Req, Resp>(
    router: &RequestRouter,
    db: &RelayDatabase,
    request: Request<Req>,
    method: &str,
    channel_size: usize,
) -> Result<Response<Pin<Box<dyn tokio_stream::Stream<Item = Result<Resp, Status>> + Send>>>, Status>
where
    Req: Message,
    Resp: Message + Default + Send + 'static,
{
    let claims = crate::server::interceptor::extract_claims(&request)?;
    let machine_id = crate::server::agent_proxy::extract_machine_id(&request)?;
    verify_machine_ownership(db, &machine_id, &claims.sub).await?;
    let req = request.into_inner();
    let stream = forward_server_stream(router, &machine_id, method, &req, channel_size).await?;
    Ok(Response::new(stream))
}

/// Forward a server-streaming RPC through the tunnel and return a boxed `Stream`.
///
/// Encodes `req`, opens a tunnel stream, then spawns a task that decodes each
/// `TunnelFrame` into `Resp` and pushes it onto the returned stream.
///
/// Used by `CommandProxyService::execute_service_command` and
/// `AgentProxyService::resume_session`.
pub async fn forward_server_stream<Req, Resp>(
    router: &RequestRouter,
    machine_id: &str,
    method: &str,
    req: &Req,
    channel_size: usize,
) -> Result<Pin<Box<dyn tokio_stream::Stream<Item = Result<Resp, Status>> + Send>>, Status>
where
    Req: Message,
    Resp: Message + Default + Send + 'static,
{
    let (request_id, buf) = encode_request(req)?;

    let mut stream_rx = router
        .forward_stream(machine_id, &request_id, method, buf, HashMap::new())
        .await
        .map_err(router_error_to_status)?;

    let (tx, rx) = mpsc::channel::<Result<Resp, Status>>(channel_size);
    let mid = machine_id.to_string();
    tokio::spawn(async move {
        while let Some(frame) = stream_rx.recv().await {
            match decode_stream_frame::<Resp>(&frame, &mid) {
                StreamFrameAction::Send(item) => {
                    if tx.send(item).await.is_err() {
                        break;
                    }
                }
                StreamFrameAction::Break(item) => {
                    if let Some(item) = item {
                        let _ = tx.send(item).await;
                    }
                    break;
                }
                StreamFrameAction::Skip => {}
            }
        }
    });

    Ok(Box::pin(ReceiverStream::new(rx)))
}

/// Result of decoding a single `TunnelFrame` in a server-stream context.
enum StreamFrameAction<Resp> {
    /// Decoded successfully; send to client.
    Send(Result<Resp, Status>),
    /// Break out of the receive loop, optionally sending a final item first.
    Break(Option<Result<Resp, Status>>),
    /// Skip this frame (unrecognised type).
    Skip,
}

/// Decode a single `TunnelFrame` into a typed response for server-streaming RPCs.
fn decode_stream_frame<Resp: Message + Default>(
    frame: &TunnelFrame,
    machine_id: &str,
) -> StreamFrameAction<Resp> {
    match FrameType::try_from(frame.frame_type) {
        Ok(FrameType::StreamData) => {
            if let Some(tunnel_frame::Payload::StreamData(p)) = &frame.payload {
                let data = p.encrypted.as_ref().map_or(&[][..], |e| &e.ciphertext[..]);
                match Resp::decode(data) {
                    Ok(msg) => StreamFrameAction::Send(Ok(msg)),
                    Err(e) => {
                        warn!(error = %e, machine_id = %machine_id, "Failed to decode stream frame");
                        StreamFrameAction::Skip
                    }
                }
            } else {
                StreamFrameAction::Skip
            }
        }
        Ok(FrameType::Error) => {
            if let Some(tunnel_frame::Payload::Error(e)) = &frame.payload {
                StreamFrameAction::Break(Some(Err(Status::internal(format!(
                    "Daemon error: {}",
                    e.message
                )))))
            } else {
                StreamFrameAction::Break(None)
            }
        }
        _ => StreamFrameAction::Skip,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use tonic::{Code, Status};

    use super::{is_peer_disconnect, verify_machine_ownership};
    use crate::server::test_helpers::{test_db_with_owner, test_db_with_two_users};

    // ── Primary signal: gRPC status code ────────────────────────────

    #[test]
    fn unavailable_is_peer_disconnect() {
        let status = Status::unavailable("transport closing");
        assert!(is_peer_disconnect(&status));
    }

    #[test]
    fn cancelled_is_peer_disconnect() {
        let status = Status::cancelled("request cancelled");
        assert!(is_peer_disconnect(&status));
    }

    // ── Fallback: substring matching on Internal status ─────────────

    #[test]
    fn internal_h2_protocol_error_is_peer_disconnect() {
        let status = Status::new(Code::Internal, "stream error: h2 protocol error");
        assert!(is_peer_disconnect(&status));
    }

    #[test]
    fn internal_broken_pipe_is_peer_disconnect() {
        let status = Status::new(Code::Internal, "broken pipe");
        assert!(is_peer_disconnect(&status));
    }

    #[test]
    fn internal_connection_reset_is_peer_disconnect() {
        let status = Status::new(Code::Internal, "connection reset by peer");
        assert!(is_peer_disconnect(&status));
    }

    #[test]
    fn internal_close_notify_is_peer_disconnect() {
        let status = Status::new(Code::Internal, "received fatal alert: close_notify");
        assert!(is_peer_disconnect(&status));
    }

    // ── Negative cases ──────────────────────────────────────────────

    #[test]
    fn internal_unrelated_message_is_not_peer_disconnect() {
        let status = Status::new(Code::Internal, "some other error");
        assert!(!is_peer_disconnect(&status));
    }

    #[test]
    fn ok_is_not_peer_disconnect() {
        let status = Status::ok("success");
        assert!(!is_peer_disconnect(&status));
    }

    #[test]
    fn not_found_is_not_peer_disconnect() {
        let status = Status::not_found("resource missing");
        assert!(!is_peer_disconnect(&status));
    }

    // ── verify_machine_ownership ──────────────────────────────────────

    #[tokio::test]
    async fn owner_can_access_their_machine() {
        let db = test_db_with_owner().await;
        let result = verify_machine_ownership(&db, "m1", "u1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn non_owner_gets_permission_denied() {
        let db = test_db_with_two_users().await;
        let err = verify_machine_ownership(&db, "m1", "u2").await.unwrap_err();
        assert_eq!(err.code(), Code::PermissionDenied);
        assert!(err.message().contains("Not your machine"));
    }

    #[tokio::test]
    async fn nonexistent_machine_gets_not_found() {
        let db = test_db_with_owner().await;
        let err = verify_machine_ownership(&db, "no-such-machine", "u1")
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::NotFound);
        assert!(err.message().contains("Machine not found"));
    }
}
