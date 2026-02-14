//! `AgentService` proxy that forwards calls through the tunnel to daemons.
//!
//! The relay acts as a pure frame forwarder — it never decodes the encrypted
//! payload content. Only routing metadata (method, `machine_id`, `request_id`)
//! is visible to the relay.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use prost::Message;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use tracing::{info, instrument, warn};

use betcode_proto::v1::agent_service_server::AgentService;
use betcode_proto::v1::{
    AgentEvent, AgentRequest, CancelTurnRequest, CancelTurnResponse, ClearSessionGrantsRequest,
    ClearSessionGrantsResponse, CompactSessionRequest, CompactSessionResponse, EncryptedPayload,
    FrameType, InputLockRequest, InputLockResponse, KeyExchangeRequest, KeyExchangeResponse,
    ListSessionGrantsRequest, ListSessionGrantsResponse, ListSessionsRequest,
    ListSessionsResponse, RenameSessionRequest, RenameSessionResponse, ResumeSessionRequest,
    SetSessionGrantRequest, SetSessionGrantResponse, StreamPayload, TunnelFrame,
};

use betcode_proto::methods::{
    METHOD_CANCEL_TURN, METHOD_CLEAR_SESSION_GRANTS, METHOD_COMPACT_SESSION, METHOD_CONVERSE,
    METHOD_EXCHANGE_KEYS, METHOD_LIST_SESSIONS, METHOD_LIST_SESSION_GRANTS,
    METHOD_RENAME_SESSION, METHOD_REQUEST_INPUT_LOCK, METHOD_RESUME_SESSION,
    METHOD_SET_SESSION_GRANT,
};

use crate::router::{RequestRouter, RouterError};
use crate::server::interceptor::extract_claims;

type EventStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<AgentEvent, Status>> + Send>>;

/// Extract `machine_id` from gRPC request metadata.
#[allow(clippy::result_large_err)]
pub fn extract_machine_id<T>(req: &Request<T>) -> Result<String, Status> {
    req.metadata()
        .get("x-machine-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| Status::invalid_argument("Missing x-machine-id metadata header"))
}

/// Map a `RouterError` to a gRPC `Status`.
pub fn router_error_to_status(err: RouterError) -> Status {
    match err {
        RouterError::MachineOffline(m) => Status::unavailable(format!("Machine offline: {m}")),
        RouterError::Buffered(m) => {
            Status::unavailable(format!("Machine offline, request buffered: {m}"))
        }
        RouterError::Timeout(r) => Status::deadline_exceeded(format!("Request timed out: {r}")),
        RouterError::SendFailed(m) => Status::internal(format!("Failed to send to machine: {m}")),
        RouterError::ResponseDropped(r) => Status::internal(format!("Response dropped: {r}")),
    }
}

/// Decode a response payload from a `TunnelFrame`.
///
/// The relay decodes only the outer frame envelope — the encrypted payload
/// inside `StreamPayload` is forwarded opaquely to the client for decryption.
#[allow(clippy::result_large_err)]
pub fn decode_response<M: Message + Default>(frame: &TunnelFrame) -> Result<M, Status> {
    if frame.frame_type == FrameType::Error as i32 {
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(ref e)) = frame.payload {
            return Err(Status::internal(format!("Daemon error: {}", e.message)));
        }
        return Err(Status::internal("Daemon returned error frame"));
    }
    match &frame.payload {
        Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) => {
            let data = p
                .encrypted
                .as_ref()
                .map_or(&[][..], |e| &e.ciphertext[..]);
            M::decode(data)
                .map_err(|e| Status::internal(format!("Failed to decode response: {e}")))
        }
        _ => Err(Status::internal("Unexpected response payload format")),
    }
}

/// Background task that handles the Converse bidi stream proxy.
///
/// Reads the first message from the client, sets up the tunnel route, then
/// forwards messages in both directions. Runs in a spawned task so the gRPC
/// handler can return the response stream immediately (avoiding deadlock).
#[allow(clippy::too_many_lines)]
async fn converse_proxy_task(
    router: Arc<RequestRouter>,
    machine_id: &str,
    mut in_stream: Streaming<AgentRequest>,
    out_tx: mpsc::Sender<Result<AgentEvent, Status>>,
) -> Result<(), Status> {
    // Read first message from client
    let first = match in_stream.next().await {
        Some(Ok(req)) => req,
        Some(Err(e)) => {
            let _ = out_tx
                .send(Err(Status::internal(format!("Stream error: {e}"))))
                .await;
            return Err(Status::internal(format!("Stream error: {e}")));
        }
        None => {
            let _ = out_tx
                .send(Err(Status::invalid_argument("Empty stream")))
                .await;
            return Err(Status::invalid_argument("Empty stream"));
        }
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let mut buf = Vec::with_capacity(first.encoded_len());
    first
        .encode(&mut buf)
        .map_err(|e| Status::internal(format!("Encode error: {e}")))?;

    let (client_tx, mut event_rx) = router
        .forward_bidi_stream(
            machine_id,
            &request_id,
            METHOD_CONVERSE,
            buf,
            HashMap::new(),
        )
        .await
        .map_err(router_error_to_status)?;

    // Forward client messages to daemon
    let rid = request_id.clone();
    tokio::spawn(async move {
        while let Some(result) = in_stream.next().await {
            match result {
                Ok(req) => {
                    let mut data = Vec::with_capacity(req.encoded_len());
                    if req.encode(&mut data).is_err() {
                        continue;
                    }
                    let frame = TunnelFrame {
                        request_id: rid.clone(),
                        frame_type: FrameType::StreamData as i32,
                        timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                            StreamPayload {
                                method: String::new(),
                                encrypted: Some(EncryptedPayload {
                                    ciphertext: data,
                                    nonce: Vec::new(),
                                    ephemeral_pubkey: Vec::new(),
                                }),
                                sequence: 0,
                                metadata: HashMap::new(),
                            },
                        )),
                    };
                    if client_tx.send(frame).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if super::grpc_util::is_peer_disconnect(&e) {
                        info!("Client disconnected from converse proxy");
                    } else {
                        warn!(error = %e, "Client stream error in converse proxy");
                    }
                    break;
                }
            }
        }
    });

    // Forward daemon events to client (in current task)
    let mid = machine_id.to_string();
    info!(request_id = %request_id, machine_id = %mid, "Converse proxy waiting for daemon events");
    let mut event_count = 0u64;
    while let Some(frame) = event_rx.recv().await {
        let ft = frame.frame_type;
        event_count += 1;
        info!(
            request_id = %request_id, frame_type = ft, event_count,
            "Converse proxy received frame from daemon"
        );
        match FrameType::try_from(ft) {
            Ok(FrameType::StreamData) => {
                if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) = frame.payload
                {
                    let data = p
                        .encrypted
                        .as_ref()
                        .map_or(&[][..], |e| &e.ciphertext[..]);
                    match AgentEvent::decode(data) {
                        Ok(event) => {
                            if out_tx.send(Ok(event)).await.is_err() {
                                warn!(request_id = %request_id, "Client receiver dropped");
                                break;
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, machine_id = %mid, "Failed to decode AgentEvent");
                        }
                    }
                }
            }
            Ok(FrameType::Error) => {
                if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) = frame.payload {
                    let _ = out_tx
                        .send(Err(Status::internal(format!(
                            "Daemon error: {}",
                            e.message
                        ))))
                        .await;
                }
                break;
            }
            _ => {
                info!(request_id = %request_id, frame_type = ft, "Ignoring non-StreamData frame");
            }
        }
    }
    info!(request_id = %request_id, event_count, "Converse proxy stream ended");
    Ok(())
}

/// Proxies `AgentService` calls through the tunnel to a target daemon.
pub struct AgentProxyService {
    router: Arc<RequestRouter>,
}

impl AgentProxyService {
    pub const fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }
}

#[tonic::async_trait]
impl AgentService for AgentProxyService {
    type ConverseStream = EventStream;
    type ResumeSessionStream = EventStream;

    #[instrument(skip(self, request), fields(rpc = "Converse"))]
    async fn converse(
        &self,
        request: Request<Streaming<AgentRequest>>,
    ) -> Result<Response<Self::ConverseStream>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let in_stream = request.into_inner();

        // Return the response stream immediately to avoid deadlock.
        // The client can't send the first message until it receives the
        // response (which gives it the sender handle), so we must not
        // block on in_stream.next() before returning.
        let (out_tx, out_rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        let router = Arc::clone(&self.router);
        tokio::spawn(async move {
            if let Err(e) = converse_proxy_task(router, &machine_id, in_stream, out_tx).await {
                warn!(machine_id = %machine_id, error = %e, "Converse proxy task failed");
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(out_rx))))
    }

    #[instrument(skip(self, request), fields(rpc = "ListSessions"))]
    async fn list_sessions(
        &self,
        request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_SESSIONS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ResumeSession"))]
    async fn resume_session(
        &self,
        request: Request<ResumeSessionRequest>,
    ) -> Result<Response<Self::ResumeSessionStream>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let req = request.into_inner();
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.encode(&mut buf)
            .map_err(|e| Status::internal(format!("Encode error: {e}")))?;

        let mut stream_rx = self
            .router
            .forward_stream(
                &machine_id,
                &request_id,
                METHOD_RESUME_SESSION,
                buf,
                HashMap::new(),
            )
            .await
            .map_err(router_error_to_status)?;

        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);
        let mid = machine_id;
        tokio::spawn(async move {
            while let Some(frame) = stream_rx.recv().await {
                match FrameType::try_from(frame.frame_type) {
                    Ok(FrameType::StreamData) => {
                        if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) =
                            frame.payload
                        {
                            let data = p
                                .encrypted
                                .as_ref()
                                .map_or(&[][..], |e| &e.ciphertext[..]);
                            match AgentEvent::decode(data) {
                                Ok(event) => {
                                    if tx.send(Ok(event)).await.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!(error = %e, machine_id = %mid, "Failed to decode AgentEvent in resume");
                                }
                            }
                        }
                    }
                    Ok(FrameType::Error) => {
                        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(e)) =
                            frame.payload
                        {
                            let _ = tx
                                .send(Err(Status::internal(format!(
                                    "Daemon error: {}",
                                    e.message
                                ))))
                                .await;
                        }
                        break;
                    }
                    _ => {}
                }
            }
            info!(request_id = %request_id, "ResumeSession proxy stream ended");
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    #[instrument(skip(self, request), fields(rpc = "CompactSession"))]
    async fn compact_session(
        &self,
        request: Request<CompactSessionRequest>,
    ) -> Result<Response<CompactSessionResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_COMPACT_SESSION,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "CancelTurn"))]
    async fn cancel_turn(
        &self,
        request: Request<CancelTurnRequest>,
    ) -> Result<Response<CancelTurnResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_CANCEL_TURN,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "RequestInputLock"))]
    async fn request_input_lock(
        &self,
        request: Request<InputLockRequest>,
    ) -> Result<Response<InputLockResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_REQUEST_INPUT_LOCK,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ExchangeKeys"))]
    async fn exchange_keys(
        &self,
        request: Request<KeyExchangeRequest>,
    ) -> Result<Response<KeyExchangeResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_EXCHANGE_KEYS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ListSessionGrants"))]
    async fn list_session_grants(
        &self,
        request: Request<ListSessionGrantsRequest>,
    ) -> Result<Response<ListSessionGrantsResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_LIST_SESSION_GRANTS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "ClearSessionGrants"))]
    async fn clear_session_grants(
        &self,
        request: Request<ClearSessionGrantsRequest>,
    ) -> Result<Response<ClearSessionGrantsResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_CLEAR_SESSION_GRANTS,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "SetSessionGrant"))]
    async fn set_session_grant(
        &self,
        request: Request<SetSessionGrantRequest>,
    ) -> Result<Response<SetSessionGrantResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_SET_SESSION_GRANT,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }

    #[instrument(skip(self, request), fields(rpc = "RenameSession"))]
    async fn rename_session(
        &self,
        request: Request<RenameSessionRequest>,
    ) -> Result<Response<RenameSessionResponse>, Status> {
        let _claims = extract_claims(&request)?;
        let machine_id = extract_machine_id(&request)?;
        let resp = super::grpc_util::forward_unary(
            &self.router,
            &machine_id,
            METHOD_RENAME_SESSION,
            &request.into_inner(),
        )
        .await?;
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
#[path = "agent_proxy_tests.rs"]
mod tests;
