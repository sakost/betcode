//! Heartbeat task for tunnel keepalive via unary RPC.

use tonic::transport::Channel;
use tonic::Request;
use tracing::{info, warn};

use betcode_proto::v1::tunnel_service_client::TunnelServiceClient;
use betcode_proto::v1::TunnelHeartbeat;

/// Spawn a heartbeat RPC task that periodically sends heartbeats via the
/// TunnelService.Heartbeat unary RPC (separate from the tunnel stream pings).
pub fn spawn_heartbeat_task(
    channel: Channel,
    token: String,
    machine_id: String,
    interval: std::time::Duration,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut client = TunnelServiceClient::new(channel);
        let mut timer = tokio::time::interval(interval);
        timer.tick().await; // Skip first immediate tick

        loop {
            tokio::select! {
                _ = timer.tick() => {
                    let mut request = Request::new(TunnelHeartbeat {
                        machine_id: machine_id.clone(),
                        timestamp: Some(prost_types::Timestamp::from(
                            std::time::SystemTime::now(),
                        )),
                        active_sessions: 0,
                        cpu_usage_percent: 0.0,
                        memory_usage_percent: 0.0,
                    });
                    if let Ok(val) = format!("Bearer {}", token).parse() {
                        request.metadata_mut().insert("authorization", val);
                    }
                    if let Err(e) = client.heartbeat(request).await {
                        warn!(error = %e, "Heartbeat RPC failed");
                    }
                }
                _ = shutdown.changed() => {
                    info!("Heartbeat task shutting down");
                    return;
                }
            }
        }
    })
}
