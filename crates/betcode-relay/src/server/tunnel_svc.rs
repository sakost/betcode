//! TunnelService gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{error, info, warn};

use betcode_proto::v1::tunnel_service_server::TunnelService;
use betcode_proto::v1::{
    FrameType, TunnelFrame, TunnelHeartbeat, TunnelRegisterRequest, TunnelRegisterResponse,
};

use crate::registry::ConnectionRegistry;
use crate::server::interceptor::extract_claims;
use crate::storage::RelayDatabase;

type TunnelStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<TunnelFrame, Status>> + Send>>;

pub struct TunnelServiceImpl {
    registry: Arc<ConnectionRegistry>,
    db: RelayDatabase,
}

impl TunnelServiceImpl {
    pub fn new(registry: Arc<ConnectionRegistry>, db: RelayDatabase) -> Self {
        Self { registry, db }
    }
}

#[tonic::async_trait]
impl TunnelService for TunnelServiceImpl {
    type OpenTunnelStream = TunnelStream;

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
            while let Some(result) = in_stream.next().await {
                match result {
                    Ok(frame) => {
                        let frame_type = frame.frame_type;
                        if frame_type == FrameType::Response as i32
                            || frame_type == FrameType::StreamData as i32
                            || frame_type == FrameType::StreamEnd as i32
                        {
                            // Route response to pending waiter
                            let rid = frame.request_id.clone();
                            if !conn_ref.complete_pending(&rid, frame).await {
                                warn!(
                                    request_id = %rid,
                                    "No pending waiter for response"
                                );
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

    async fn register(
        &self,
        request: Request<TunnelRegisterRequest>,
    ) -> Result<Response<TunnelRegisterResponse>, Status> {
        let user_id = {
            let claims = extract_claims(&request)?;
            claims.sub.clone()
        };
        let req = request.into_inner();

        // Verify machine exists and belongs to user
        let machine = self
            .db
            .get_machine(&req.machine_id)
            .await
            .map_err(|_| Status::not_found("Machine not found"))?;

        if machine.owner_id != user_id {
            return Err(Status::permission_denied("Not your machine"));
        }

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
