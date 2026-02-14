//! Shared gRPC relay connection helpers.
//!
//! Used by both `auth_cmd` and `machine_cmd` to establish a TLS-capable
//! channel to the relay server.

use std::path::Path;

use tonic::transport::{Certificate, Channel, ClientTlsConfig};

/// Build a gRPC channel to the relay, with optional custom CA cert for TLS.
pub async fn relay_channel(url: &str, ca_cert: Option<&Path>) -> anyhow::Result<Channel> {
    let mut endpoint = Channel::from_shared(url.to_string())?;
    if url.starts_with("https://") {
        let mut tls_config = ClientTlsConfig::new().with_enabled_roots();
        if let Some(ca_path) = ca_cert {
            let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
                anyhow::anyhow!("Failed to read CA cert {}: {}", ca_path.display(), e)
            })?;
            tls_config = tls_config.ca_certificate(Certificate::from_pem(ca_pem));
        }
        endpoint = endpoint
            .tls_config(tls_config)
            .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
    }
    endpoint
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to relay: {e}"))
}
