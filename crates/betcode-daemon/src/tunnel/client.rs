//! Tunnel client that connects the daemon to a relay server.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::{Request, Streaming};
use tracing::{error, info, warn};

use betcode_proto::v1::auth_service_client::AuthServiceClient;
use betcode_proto::v1::tunnel_service_client::TunnelServiceClient;
use betcode_proto::v1::{
    FrameType, LoginRequest, TunnelControl, TunnelControlType, TunnelFrame, TunnelRegisterRequest,
};

use super::config::TunnelConfig;
use super::error::TunnelError;
use super::handler::TunnelRequestHandler;

/// Tunnel client that maintains a persistent connection to the relay.
pub struct TunnelClient {
    config: TunnelConfig,
    handler: Arc<TunnelRequestHandler>,
}

impl TunnelClient {
    pub fn new(config: TunnelConfig) -> Self {
        let handler = Arc::new(TunnelRequestHandler::new(config.machine_id.clone()));
        Self { config, handler }
    }

    /// Run the tunnel client with automatic reconnection.
    ///
    /// This is the main entry point. It will connect to the relay, authenticate,
    /// open a tunnel, and handle frames. On disconnect, it reconnects with
    /// exponential backoff.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut attempt: u32 = 0;

        loop {
            if *shutdown.borrow() {
                info!("Tunnel client shutting down");
                return;
            }

            match self.connect_and_run(&mut shutdown).await {
                Ok(()) => {
                    info!("Tunnel connection closed cleanly");
                    return;
                }
                Err(e) => {
                    if !self.config.reconnect.should_retry(attempt) {
                        error!(error = %e, attempt, "Max reconnect attempts reached");
                        return;
                    }

                    let delay = self.config.reconnect.delay_for_attempt(attempt);
                    warn!(error = %e, attempt, delay_ms = delay.as_millis(), "Reconnecting");

                    tokio::select! {
                        _ = sleep(delay) => {}
                        _ = shutdown.changed() => {
                            info!("Tunnel client shutting down during reconnect wait");
                            return;
                        }
                    }

                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }

    /// Connect to the relay, authenticate, and run the tunnel loop.
    async fn connect_and_run(
        &self,
        shutdown: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), TunnelError> {
        let mut endpoint = Channel::from_shared(self.config.relay_url.clone())
            .map_err(|e| TunnelError::Connection(e.to_string()))?;

        // Configure TLS if CA cert is provided
        if let Some(ca_path) = &self.config.ca_cert_path {
            let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
                TunnelError::Connection(format!(
                    "Failed to read CA cert {}: {}",
                    ca_path.display(),
                    e
                ))
            })?;
            let ca_cert = Certificate::from_pem(ca_pem);
            let tls_config = ClientTlsConfig::new().ca_certificate(ca_cert);
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| TunnelError::Connection(e.to_string()))?;
            info!(ca_cert = %ca_path.display(), "TLS configured for relay connection");
        }

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| TunnelError::Connection(e.to_string()))?;

        let token = self.authenticate(channel.clone()).await?;

        let mut tunnel_client = TunnelServiceClient::new(channel);
        self.register_machine(&mut tunnel_client, &token).await?;

        let (outbound_tx, outbound_rx) = mpsc::channel::<TunnelFrame>(128);
        self.send_init_frame(&outbound_tx).await?;

        let outbound_stream = ReceiverStream::new(outbound_rx);
        let mut request = Request::new(outbound_stream);
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", token)
                .parse()
                .map_err(|_| TunnelError::Auth("Invalid token format".into()))?,
        );

        let response = tunnel_client
            .open_tunnel(request)
            .await
            .map_err(|e| TunnelError::Connection(e.to_string()))?;

        info!(machine_id = %self.config.machine_id, "Tunnel connected");

        self.tunnel_loop(response.into_inner(), outbound_tx, shutdown)
            .await
    }

    /// Authenticate with the relay and get an access token.
    async fn authenticate(&self, channel: Channel) -> Result<String, TunnelError> {
        let mut auth_client = AuthServiceClient::new(channel);

        let response = auth_client
            .login(LoginRequest {
                username: self.config.username.clone(),
                password: self.config.password.clone(),
            })
            .await
            .map_err(|e| TunnelError::Auth(e.to_string()))?;

        let login_resp = response.into_inner();
        info!(user_id = %login_resp.user_id, "Authenticated with relay");
        Ok(login_resp.access_token)
    }

    /// Register this machine with the relay.
    async fn register_machine(
        &self,
        client: &mut TunnelServiceClient<Channel>,
        token: &str,
    ) -> Result<(), TunnelError> {
        let mut request = Request::new(TunnelRegisterRequest {
            machine_id: self.config.machine_id.clone(),
            machine_name: self.config.machine_name.clone(),
            capabilities: Default::default(),
        });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", token)
                .parse()
                .map_err(|_| TunnelError::Auth("Invalid token format".into()))?,
        );

        let response = client
            .register(request)
            .await
            .map_err(|e| TunnelError::Registration(e.to_string()))?;

        if !response.into_inner().accepted {
            return Err(TunnelError::Registration("Registration rejected".into()));
        }

        info!(machine_id = %self.config.machine_id, "Machine registered");
        Ok(())
    }

    /// Send the initial control frame identifying this machine.
    async fn send_init_frame(&self, tx: &mpsc::Sender<TunnelFrame>) -> Result<(), TunnelError> {
        let frame = TunnelFrame {
            request_id: String::new(),
            frame_type: FrameType::Control as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Control(
                TunnelControl {
                    control_type: TunnelControlType::Unspecified as i32,
                    params: [("machine_id".to_string(), self.config.machine_id.clone())]
                        .into_iter()
                        .collect(),
                },
            )),
        };
        tx.send(frame)
            .await
            .map_err(|_| TunnelError::Connection("Failed to send init frame".into()))
    }

    /// Main tunnel loop: process incoming frames and send responses.
    async fn tunnel_loop(
        &self,
        mut inbound: Streaming<TunnelFrame>,
        outbound_tx: mpsc::Sender<TunnelFrame>,
        shutdown: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), TunnelError> {
        let handler = Arc::clone(&self.handler);
        let machine_id = self.config.machine_id.clone();
        let mut heartbeat_timer = tokio::time::interval(self.config.heartbeat_interval);
        heartbeat_timer.tick().await; // Skip first immediate tick

        loop {
            tokio::select! {
                frame_result = inbound.next() => {
                    match frame_result {
                        Some(Ok(frame)) => {
                            if let Some(response) = handler.handle_frame(frame) {
                                if outbound_tx.send(response).await.is_err() {
                                    return Err(TunnelError::Connection(
                                        "Outbound channel closed".into(),
                                    ));
                                }
                            }
                        }
                        Some(Err(e)) => {
                            return Err(TunnelError::Stream(e.to_string()));
                        }
                        None => {
                            return Err(TunnelError::Connection(
                                "Stream ended by relay".into(),
                            ));
                        }
                    }
                }
                _ = heartbeat_timer.tick() => {
                    let ping = TunnelFrame {
                        request_id: String::new(),
                        frame_type: FrameType::Control as i32,
                        timestamp: Some(prost_types::Timestamp::from(
                            std::time::SystemTime::now(),
                        )),
                        payload: Some(
                            betcode_proto::v1::tunnel_frame::Payload::Control(
                                TunnelControl {
                                    control_type: TunnelControlType::Ping as i32,
                                    params: [(
                                        "machine_id".to_string(),
                                        machine_id.clone(),
                                    )]
                                    .into_iter()
                                    .collect(),
                                },
                            ),
                        ),
                    };
                    if outbound_tx.send(ping).await.is_err() {
                        return Err(TunnelError::Connection(
                            "Outbound channel closed during heartbeat".into(),
                        ));
                    }
                }
                _ = shutdown.changed() => {
                    info!("Tunnel client received shutdown signal");
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_client_creation() {
        let config = TunnelConfig::new(
            "https://localhost:443".into(),
            "m1".into(),
            "Test Machine".into(),
            "user".into(),
            "pass".into(),
        );
        let client = TunnelClient::new(config);
        assert_eq!(client.config.machine_id, "m1");
    }
}
