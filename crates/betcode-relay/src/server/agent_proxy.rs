//! AgentService proxy that forwards calls through the tunnel to daemons.

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
    AgentEvent, AgentRequest, CancelTurnRequest, CancelTurnResponse, CompactSessionRequest,
    CompactSessionResponse, FrameType, InputLockRequest, InputLockResponse, ListSessionsRequest,
    ListSessionsResponse, ResumeSessionRequest, StreamPayload, TunnelFrame,
};

use crate::router::{RequestRouter, RouterError};
use crate::server::interceptor::extract_claims;

type EventStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<AgentEvent, Status>> + Send>>;

/// Extract machine_id from gRPC request metadata.
#[allow(clippy::result_large_err)]
pub fn extract_machine_id<T>(req: &Request<T>) -> Result<String, Status> {
    req.metadata()
        .get("x-machine-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| Status::invalid_argument("Missing x-machine-id metadata header"))
}

/// Map a RouterError to a gRPC Status.
fn router_error_to_status(err: RouterError) -> Status {
    match err {
        RouterError::MachineOffline(m) => Status::unavailable(format!("Machine offline: {}", m)),
        RouterError::Buffered(m) => {
            Status::unavailable(format!("Machine offline, request buffered: {}", m))
        }
        RouterError::Timeout(r) => Status::deadline_exceeded(format!("Request timed out: {}", r)),
        RouterError::SendFailed(m) => Status::internal(format!("Failed to send to machine: {}", m)),
        RouterError::ResponseDropped(r) => Status::internal(format!("Response dropped: {}", r)),
    }
}

/// Decode a response payload from a TunnelFrame.
#[allow(clippy::result_large_err)]
fn decode_response<M: Message + Default>(frame: &TunnelFrame) -> Result<M, Status> {
    if frame.frame_type == FrameType::Error as i32 {
        if let Some(betcode_proto::v1::tunnel_frame::Payload::Error(ref e)) = frame.payload {
            return Err(Status::internal(format!("Daemon error: {}", e.message)));
        }
        return Err(Status::internal("Daemon returned error frame"));
    }
    match &frame.payload {
        Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) => {
            M::decode(p.data.as_slice())
                .map_err(|e| Status::internal(format!("Failed to decode response: {}", e)))
        }
        _ => Err(Status::internal("Unexpected response payload format")),
    }
}

/// Background task that handles the Converse bidi stream proxy.
///
/// Reads the first message from the client, sets up the tunnel route, then
/// forwards messages in both directions. Runs in a spawned task so the gRPC
/// handler can return the response stream immediately (avoiding deadlock).
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
                .send(Err(Status::internal(format!("Stream error: {}", e))))
                .await;
            return Err(Status::internal(format!("Stream error: {}", e)));
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
        .map_err(|e| Status::internal(format!("Encode error: {}", e)))?;

    let (client_tx, mut event_rx) = router
        .forward_bidi_stream(
            machine_id,
            &request_id,
            "AgentService/Converse",
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
                        timestamp: Some(prost_types::Timestamp::from(
                            std::time::SystemTime::now(),
                        )),
                        payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                            StreamPayload {
                                method: String::new(),
                                data,
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
                    warn!(error = %e, "Client stream error in converse proxy");
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
                if let Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(p)) =
                    frame.payload
                {
                    match AgentEvent::decode(p.data.as_slice()) {
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
                        .send(Err(Status::internal(format!("Daemon error: {}", e.message))))
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

/// Proxies AgentService calls through the tunnel to a target daemon.
pub struct AgentProxyService {
    router: Arc<RequestRouter>,
}

impl AgentProxyService {
    pub fn new(router: Arc<RequestRouter>) -> Self {
        Self { router }
    }

    async fn forward_unary<Req: Message, Resp: Message + Default>(
        &self,
        machine_id: &str,
        method: &str,
        req: &Req,
    ) -> Result<Resp, Status> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.encode(&mut buf)
            .map_err(|e| Status::internal(format!("Encode error: {}", e)))?;
        let frame = self
            .router
            .forward_request(machine_id, &request_id, method, buf, HashMap::new())
            .await
            .map_err(router_error_to_status)?;
        decode_response(&frame)
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
        let mid = machine_id.clone();
        tokio::spawn(async move {
            if let Err(e) = converse_proxy_task(router, &mid, in_stream, out_tx).await {
                warn!(machine_id = %mid, error = %e, "Converse proxy task failed");
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
        let resp = self
            .forward_unary(
                &machine_id,
                "AgentService/ListSessions",
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
            .map_err(|e| Status::internal(format!("Encode error: {}", e)))?;

        let mut stream_rx = self
            .router
            .forward_stream(
                &machine_id,
                &request_id,
                "AgentService/ResumeSession",
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
                            match AgentEvent::decode(p.data.as_slice()) {
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
        let resp = self
            .forward_unary(
                &machine_id,
                "AgentService/CompactSession",
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
        let resp = self
            .forward_unary(
                &machine_id,
                "AgentService/CancelTurn",
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
        let resp = self
            .forward_unary(
                &machine_id,
                "AgentService/RequestInputLock",
                &request.into_inner(),
            )
            .await?;
        Ok(Response::new(resp))
    }
}

#[cfg(test)]
#[path = "agent_proxy_tests.rs"]
mod tests;
