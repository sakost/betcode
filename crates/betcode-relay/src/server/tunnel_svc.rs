//! TunnelService gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, error, info, instrument, warn};

use betcode_proto::v1::tunnel_service_server::TunnelService;
use betcode_proto::v1::{
    FrameType, TunnelFrame, TunnelHeartbeat, TunnelRegisterRequest, TunnelRegisterResponse,
};

use crate::buffer::BufferManager;
use crate::registry::ConnectionRegistry;
use crate::server::interceptor::extract_claims;
use crate::storage::RelayDatabase;

type TunnelStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<TunnelFrame, Status>> + Send>>;

pub struct TunnelServiceImpl {
    registry: Arc<ConnectionRegistry>,
    db: RelayDatabase,
    buffer: Arc<BufferManager>,
}

impl TunnelServiceImpl {
    pub fn new(
        registry: Arc<ConnectionRegistry>,
        db: RelayDatabase,
        buffer: Arc<BufferManager>,
    ) -> Self {
        Self {
            registry,
            db,
            buffer,
        }
    }
}

#[tonic::async_trait]
impl TunnelService for TunnelServiceImpl {
    type OpenTunnelStream = TunnelStream;

    #[instrument(skip(self, request), fields(rpc = "OpenTunnel"))]
    async fn open_tunnel(
        &self,
        request: Request<Streaming<TunnelFrame>>,
    ) -> Result<Response<Self::OpenTunnelStream>, Status> {
        let claims = extract_claims(&request)?;
        let owner_id = claims.sub.clone();

        let mut in_stream = request.into_inner();

        // Channel for sending frames from relay to daemon
        let (relay_tx, relay_rx) = mpsc::channel::<TunnelFrame>(128);
        // Channel for the output stream back to daemon
        let (out_tx, out_rx) = mpsc::channel::<Result<TunnelFrame, Status>>(128);

        let registry = Arc::clone(&self.registry);
        let db = self.db.clone();
        let buffer = Arc::clone(&self.buffer);

        tokio::spawn(async move {
            // Wait for first frame to identify the machine
            let machine_id = match in_stream.next().await {
                Some(Ok(frame)) if frame.frame_type == FrameType::Control as i32 => {
                    if let Some(betcode_proto::v1::tunnel_frame::Payload::Control(ctrl)) =
                        &frame.payload
                    {
                        ctrl.params.get("machine_id").cloned().unwrap_or_default()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            };

            if machine_id.is_empty() {
                let _ = out_tx
                    .send(Err(Status::invalid_argument(
                        "First frame must identify machine",
                    )))
                    .await;
                return;
            }

            info!(machine_id = %machine_id, owner_id = %owner_id, "Tunnel opened");

            // Register the connection
            let conn = registry
                .register(machine_id.clone(), owner_id, relay_tx)
                .await;

            // Update machine status to online
            if let Err(e) = db.update_machine_status(&machine_id, "online").await {
                warn!(machine_id = %machine_id, error = %e, "Failed to update machine status");
            }

            // Drain any buffered messages for this machine
            match buffer.drain_buffer(&machine_id).await {
                Ok(count) if count > 0 => {
                    info!(machine_id = %machine_id, count, "Drained buffered messages on reconnect");
                }
                Err(e) => {
                    warn!(machine_id = %machine_id, error = %e, "Failed to drain buffer on reconnect");
                }
                _ => {}
            }

            // Forward frames from relay_rx to daemon (out_tx)
            let out_tx_fwd = out_tx.clone();
            let relay_rx_handle = tokio::spawn(async move {
                let mut relay_rx = relay_rx;
                while let Some(frame) = relay_rx.recv().await {
                    if out_tx_fwd.send(Ok(frame)).await.is_err() {
                        break;
                    }
                }
            });

            // Process incoming frames from daemon
            let conn_ref = Arc::clone(&conn);
            let mut daemon_frame_count = 0u64;
            while let Some(result) = in_stream.next().await {
                daemon_frame_count += 1;
                match result {
                    Ok(frame) => {
                        let frame_type = frame.frame_type;
                        let rid = frame.request_id.clone();

                        if frame_type == FrameType::StreamEnd as i32 {
                            debug!(
                                request_id = %rid, machine_id = %machine_id,
                                daemon_frame_count,
                                "Received StreamEnd from daemon"
                            );
                            // StreamEnd: deliver final frame to stream channel, then close it.
                            // If no stream channel, try unary pending as fallback.
                            if conn_ref.has_stream_pending(&rid).await {
                                conn_ref.send_stream_frame(&rid, frame).await;
                                conn_ref.complete_stream(&rid).await;
                            } else if !conn_ref.complete_pending(&rid, frame).await {
                                warn!(request_id = %rid, "No pending waiter for StreamEnd");
                            }
                        } else if frame_type == FrameType::StreamData as i32 {
                            let has_stream = conn_ref.has_stream_pending(&rid).await;
                            debug!(
                                request_id = %rid, machine_id = %machine_id,
                                has_stream_pending = has_stream,
                                daemon_frame_count,
                                "Received StreamData from daemon, dispatching"
                            );
                            // StreamData: try stream channel first, fall back to unary
                            if !conn_ref.send_stream_frame(&rid, frame.clone()).await
                                && !conn_ref.complete_pending(&rid, frame).await
                            {
                                warn!(request_id = %rid, "No pending waiter for StreamData");
                            }
                        } else if frame_type == FrameType::Response as i32 {
                            // Unary response: complete the oneshot pending
                            if !conn_ref.complete_pending(&rid, frame).await {
                                warn!(request_id = %rid, "No pending waiter for Response");
                            }
                        }
                    }
                    Err(e) => {
                        error!(machine_id = %machine_id, error = %e, "Tunnel stream error");
                        break;
                    }
                }
            }

            // Cleanup
            info!(machine_id = %machine_id, "Tunnel closed");
            conn.cancel_all_pending().await;
            registry.unregister(&machine_id).await;
            relay_rx_handle.abort();

            if let Err(e) = db.update_machine_status(&machine_id, "offline").await {
                warn!(machine_id = %machine_id, error = %e, "Failed to update machine status");
            }
        });

        let out_stream = ReceiverStream::new(out_rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

    #[instrument(skip(self, request), fields(rpc = "Register"))]
    async fn register(
        &self,
        request: Request<TunnelRegisterRequest>,
    ) -> Result<Response<TunnelRegisterResponse>, Status> {
        let user_id = {
            let claims = extract_claims(&request)?;
            claims.sub.clone()
        };
        let req = request.into_inner();

        // Auto-register machine if it doesn't exist, otherwise verify ownership
        let _machine = match self.db.get_machine(&req.machine_id).await {
            Ok(m) => {
                if m.owner_id != user_id {
                    return Err(Status::permission_denied("Not your machine"));
                }
                m
            }
            Err(_) => {
                let metadata_json = serde_json::to_string(&req.capabilities)
                    .unwrap_or_else(|_| "{}".to_string());
                info!(
                    machine_id = %req.machine_id,
                    machine_name = %req.machine_name,
                    "Auto-registering machine on first tunnel connect"
                );
                self.db
                    .create_machine(
                        &req.machine_id,
                        &req.machine_name,
                        &user_id,
                        &metadata_json,
                    )
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to auto-register machine: {}", e))
                    })?
            }
        };

        info!(
            machine_id = %req.machine_id,
            machine_name = %req.machine_name,
            "Tunnel registration accepted"
        );

        Ok(Response::new(TunnelRegisterResponse {
            accepted: true,
            relay_id: "relay-1".to_string(),
            heartbeat_interval_secs: 30,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "Heartbeat"))]
    async fn heartbeat(
        &self,
        request: Request<TunnelHeartbeat>,
    ) -> Result<Response<TunnelHeartbeat>, Status> {
        // Validate JWT
        {
            extract_claims(&request)?;
        }
        let req = request.into_inner();

        // Update last_seen
        if let Err(e) = self.db.touch_machine(&req.machine_id).await {
            warn!(
                machine_id = %req.machine_id,
                error = %e,
                "Failed to touch machine"
            );
        }

        Ok(Response::new(TunnelHeartbeat {
            machine_id: req.machine_id,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            active_sessions: 0,
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
        }))
    }
}
