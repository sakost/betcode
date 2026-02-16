//! Tunnel client that connects the daemon to a relay server.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::{Request, Streaming};
use tracing::{error, info, warn};

use betcode_crypto::IdentityKeyPair;
use betcode_proto::v1::auth_service_client::AuthServiceClient;
use betcode_proto::v1::tunnel_service_client::TunnelServiceClient;
use betcode_proto::v1::{
    FrameType, LoginRequest, TunnelControl, TunnelControlType, TunnelFrame, TunnelRegisterRequest,
};

use super::config::TunnelConfig;
use super::error::TunnelClientError;
use super::handler::TunnelRequestHandler;

use crate::relay::SessionRelay;
use crate::server::{
    CommandServiceImpl, ConfigServiceImpl, GitLabServiceImpl, GitRepoServiceImpl,
    WorktreeServiceImpl,
};
use crate::session::SessionMultiplexer;
use crate::storage::Database;

/// Tunnel client that maintains a persistent connection to the relay.
pub struct TunnelClient {
    config: TunnelConfig,
    relay: Arc<SessionRelay>,
    multiplexer: Arc<SessionMultiplexer>,
    db: Database,
    /// X25519 identity keypair for E2E encryption key exchange.
    identity: Arc<IdentityKeyPair>,
    /// Optional `CommandServiceImpl` for handling command RPCs through the tunnel.
    command_service: Option<Arc<CommandServiceImpl>>,
    /// Optional `GitLabServiceImpl` for handling GitLab RPCs through the tunnel.
    gitlab_service: Option<Arc<GitLabServiceImpl>>,
    /// Optional `GitRepoServiceImpl` for handling repo RPCs through the tunnel.
    repo_service: Option<Arc<GitRepoServiceImpl>>,
    /// Optional `WorktreeServiceImpl` for handling worktree RPCs through the tunnel.
    worktree_service: Option<Arc<WorktreeServiceImpl>>,
    /// Optional `ConfigServiceImpl` for handling config RPCs through the tunnel.
    config_service: Option<Arc<ConfigServiceImpl>>,
}

impl TunnelClient {
    pub fn new(
        config: TunnelConfig,
        relay: Arc<SessionRelay>,
        multiplexer: Arc<SessionMultiplexer>,
        db: Database,
    ) -> Result<Self, TunnelClientError> {
        let identity = Arc::new(Self::load_identity(&config)?);
        info!(
            fingerprint = %identity.fingerprint(),
            "Loaded identity keypair"
        );
        Ok(Self {
            config,
            relay,
            multiplexer,
            db,
            identity,
            command_service: None,
            gitlab_service: None,
            repo_service: None,
            worktree_service: None,
            config_service: None,
        })
    }

    /// Set the `CommandService` implementation for handling command RPCs through the tunnel.
    pub fn set_command_service(&mut self, service: Arc<CommandServiceImpl>) {
        self.command_service = Some(service);
    }

    /// Set the `GitLabService` implementation for handling GitLab RPCs through the tunnel.
    pub fn set_gitlab_service(&mut self, service: Arc<GitLabServiceImpl>) {
        self.gitlab_service = Some(service);
    }

    /// Set the `GitRepoService` implementation for handling repo RPCs through the tunnel.
    pub fn set_repo_service(&mut self, service: Arc<GitRepoServiceImpl>) {
        self.repo_service = Some(service);
    }

    /// Set the `WorktreeService` implementation for handling worktree RPCs through the tunnel.
    pub fn set_worktree_service(&mut self, service: Arc<WorktreeServiceImpl>) {
        self.worktree_service = Some(service);
    }

    /// Set the `ConfigService` implementation for handling config RPCs through the tunnel.
    pub fn set_config_service(&mut self, service: Arc<ConfigServiceImpl>) {
        self.config_service = Some(service);
    }

    /// Load or generate the X25519 identity keypair.
    fn load_identity(config: &TunnelConfig) -> Result<IdentityKeyPair, TunnelClientError> {
        let path = config.identity_key_path.clone().unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("betcode")
                .join("identity.key")
        });
        IdentityKeyPair::load_or_generate(&path)
            .map_err(|e| TunnelClientError::Auth(format!("Failed to load identity key: {e}")))
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

            let started = std::time::Instant::now();
            match self.connect_and_run(&mut shutdown).await {
                Ok(()) => {
                    info!("Tunnel connection closed cleanly");
                    return;
                }
                Err(e) => {
                    // Reset backoff if connection was up for >60s
                    if started.elapsed() > std::time::Duration::from_secs(60) {
                        attempt = 0;
                    }

                    if !self.config.reconnect.should_retry(attempt) {
                        error!(error = %e, attempt, "Max reconnect attempts reached");
                        return;
                    }

                    let delay = self.config.reconnect.delay_for_attempt(attempt);
                    warn!(error = %e, attempt, delay_ms = delay.as_millis(), "Reconnecting");

                    tokio::select! {
                        () = sleep(delay) => {}
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
    ) -> Result<(), TunnelClientError> {
        let mut endpoint = Channel::from_shared(self.config.relay_url.clone())
            .map_err(|e| TunnelClientError::Connection(e.to_string()))?
            .http2_keep_alive_interval(std::time::Duration::from_secs(30))
            .keep_alive_timeout(std::time::Duration::from_secs(10));

        // Configure TLS for https:// URLs
        if self.config.relay_url.starts_with("https://") {
            let mut tls_config = ClientTlsConfig::new().with_enabled_roots();
            if let Some(ca_path) = &self.config.ca_cert_path {
                let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
                    TunnelClientError::Connection(format!(
                        "Failed to read CA cert {}: {}",
                        ca_path.display(),
                        e
                    ))
                })?;
                tls_config = tls_config.ca_certificate(Certificate::from_pem(ca_pem));
                info!(ca_cert = %ca_path.display(), "TLS configured with custom CA cert");
            }
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| TunnelClientError::Connection(e.to_string()))?;
        }

        let channel = endpoint.connect().await.map_err(|e| {
            // Log the full error chain for debugging
            tracing::debug!(error = ?e, "connection error details");
            TunnelClientError::Connection(format!("{e}: {}", error_chain(&e)))
        })?;

        let token = self.authenticate(channel.clone()).await?;

        let mut tunnel_client = TunnelServiceClient::new(channel);
        self.register_machine(&mut tunnel_client, &token).await?;

        let (outbound_tx, outbound_rx) = mpsc::channel::<TunnelFrame>(128);
        self.send_init_frame(&outbound_tx).await?;

        let mut handler = TunnelRequestHandler::new(
            self.config.machine_id.clone(),
            Arc::clone(&self.relay),
            Arc::clone(&self.multiplexer),
            self.db.clone(),
            outbound_tx.clone(),
            None, // Crypto session will be set by ExchangeKeys RPC
            Some(Arc::clone(&self.identity)),
        );
        if let Some(cmd_svc) = &self.command_service {
            handler.set_command_service(Arc::clone(cmd_svc));
        }
        if let Some(gitlab_svc) = &self.gitlab_service {
            handler.set_gitlab_service(Arc::clone(gitlab_svc));
        }
        if let Some(repo_svc) = &self.repo_service {
            handler.set_repo_service(Arc::clone(repo_svc));
        }
        if let Some(worktree_svc) = &self.worktree_service {
            handler.set_worktree_service(Arc::clone(worktree_svc));
        }
        if let Some(config_svc) = &self.config_service {
            handler.set_config_service(Arc::clone(config_svc));
        }
        let handler = Arc::new(handler);

        let outbound_stream = ReceiverStream::new(outbound_rx);
        let mut request = Request::new(outbound_stream);
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| TunnelClientError::Auth("Invalid token format".into()))?,
        );

        let response = tunnel_client
            .open_tunnel(request)
            .await
            .map_err(|e| TunnelClientError::Connection(e.to_string()))?;

        info!(machine_id = %self.config.machine_id, "Tunnel connected");

        self.tunnel_loop(handler, response.into_inner(), outbound_tx, shutdown)
            .await
    }

    /// Authenticate with the relay and get an access token.
    async fn authenticate(&self, channel: Channel) -> Result<String, TunnelClientError> {
        let mut auth_client = AuthServiceClient::new(channel);

        let response = auth_client
            .login(LoginRequest {
                username: self.config.username.clone(),
                password: self.config.password.clone(),
            })
            .await
            .map_err(|e| TunnelClientError::Auth(e.to_string()))?;

        let login_resp = response.into_inner();
        info!(user_id = %login_resp.user_id, "Authenticated with relay");
        Ok(login_resp.access_token)
    }

    /// Register this machine with the relay.
    async fn register_machine(
        &self,
        client: &mut TunnelServiceClient<Channel>,
        token: &str,
    ) -> Result<(), TunnelClientError> {
        let mut request = Request::new(TunnelRegisterRequest {
            machine_id: self.config.machine_id.clone(),
            machine_name: self.config.machine_name.clone(),
            capabilities: HashMap::default(),
            identity_pubkey: self.identity.public_bytes().to_vec(),
        });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| TunnelClientError::Auth("Invalid token format".into()))?,
        );

        let response = client
            .register(request)
            .await
            .map_err(|e| TunnelClientError::Registration(e.to_string()))?;

        if !response.into_inner().accepted {
            return Err(TunnelClientError::Registration(
                "Registration rejected".into(),
            ));
        }

        info!(machine_id = %self.config.machine_id, "Machine registered");
        Ok(())
    }

    /// Send the initial control frame identifying this machine.
    async fn send_init_frame(
        &self,
        tx: &mpsc::Sender<TunnelFrame>,
    ) -> Result<(), TunnelClientError> {
        let frame = TunnelFrame {
            request_id: String::new(),
            frame_type: FrameType::Control as i32,
            timestamp: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            payload: Some(betcode_proto::v1::tunnel_frame::Payload::Control(
                TunnelControl {
                    control_type: TunnelControlType::Unspecified as i32,
                    params: std::iter::once((
                        "machine_id".to_string(),
                        self.config.machine_id.clone(),
                    ))
                    .collect(),
                },
            )),
        };
        tx.send(frame)
            .await
            .map_err(|_| TunnelClientError::Connection("Failed to send init frame".into()))
    }

    /// Main tunnel loop: process incoming frames and send responses.
    async fn tunnel_loop(
        &self,
        handler: Arc<TunnelRequestHandler>,
        mut inbound: Streaming<TunnelFrame>,
        outbound_tx: mpsc::Sender<TunnelFrame>,
        shutdown: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), TunnelClientError> {
        let machine_id = self.config.machine_id.clone();
        let mut heartbeat_timer = tokio::time::interval(self.config.heartbeat_interval);
        heartbeat_timer.tick().await; // Skip first immediate tick

        loop {
            tokio::select! {
                frame_result = inbound.next() => {
                    match frame_result {
                        Some(Ok(frame)) => {
                            // Spawn frame processing as a task so the tunnel
                            // loop stays responsive for heartbeats and other
                            // concurrent frames. Without this, a slow RPC
                            // (e.g. git worktree add on a large repo) blocks
                            // the entire tunnel and causes relay timeouts.
                            let h = Arc::clone(&handler);
                            let tx = outbound_tx.clone();
                            tokio::spawn(async move {
                                let responses = h.handle_frame(frame).await;
                                for response in responses {
                                    if tx.send(response).await.is_err() {
                                        warn!("Outbound channel closed while sending response");
                                        break;
                                    }
                                }
                            });
                        }
                        Some(Err(e)) => {
                            return Err(TunnelClientError::Stream(e.to_string()));
                        }
                        None => {
                            return Err(TunnelClientError::Connection(
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
                                    params: std::iter::once((
                                        "machine_id".to_string(),
                                        machine_id.clone(),
                                    ))
                                    .collect(),
                                },
                            ),
                        ),
                    };
                    if outbound_tx.send(ping).await.is_err() {
                        return Err(TunnelClientError::Connection(
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

/// Walk the `source()` chain of an error and join into a single string.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut chain = Vec::new();
    let mut current = err.source();
    while let Some(e) = current {
        chain.push(e.to_string());
        current = e.source();
    }
    if chain.is_empty() {
        String::from("(no further details)")
    } else {
        chain.join(" -> ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subprocess::SubprocessManager;

    #[tokio::test]
    async fn tunnel_client_creation() {
        let dir = std::env::temp_dir().join(format!("betcode-test-{}", uuid::Uuid::new_v4()));
        let key_path = dir.join("identity.key");
        let mut config = TunnelConfig::new(
            "https://localhost:443".into(),
            "m1".into(),
            "Test Machine".into(),
            "user".into(),
            "pass".into(),
        );
        config.identity_key_path = Some(key_path);
        let db = Database::open_in_memory().await.unwrap();
        let sub = Arc::new(SubprocessManager::new(5));
        let mux = Arc::new(SessionMultiplexer::with_defaults());
        let command_registry = Arc::new(tokio::sync::RwLock::new(
            crate::commands::CommandRegistry::new(),
        ));
        let relay = Arc::new(SessionRelay::new(
            sub,
            Arc::clone(&mux),
            db.clone(),
            command_registry,
        ));
        let client = TunnelClient::new(config, relay, mux, db).unwrap();
        assert_eq!(client.config.machine_id, "m1");
        assert_eq!(client.identity.public_bytes().len(), 32);
        std::fs::remove_dir_all(&dir).ok();
    }
}
