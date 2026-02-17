//! TLS configuration for the relay server.

use std::path::PathBuf;

use tonic::transport::{Certificate, Identity, ServerTlsConfig};
use tracing::info;

use super::certs::{CertError, generate_dev_bundle, write_dev_certs};

/// TLS configuration for the relay server.
#[derive(Debug, Clone)]
pub enum TlsMode {
    /// No TLS (plaintext). Development only.
    Disabled,
    /// Auto-generated self-signed certificates for development.
    DevSelfSigned {
        /// Directory to store generated certs.
        cert_dir: PathBuf,
    },
    /// User-provided certificate and key files.
    Custom {
        /// Path to PEM-encoded certificate file.
        cert_path: PathBuf,
        /// Path to PEM-encoded private key file.
        key_path: PathBuf,
    },
}

impl TlsMode {
    /// Build a tonic `ServerTlsConfig` from this mode.
    ///
    /// Returns `None` if TLS is disabled.
    pub fn to_server_tls_config(&self) -> Result<Option<ServerTlsConfig>, TlsConfigError> {
        match self {
            Self::Disabled => Ok(None),
            Self::DevSelfSigned { cert_dir } => {
                info!("Generating dev TLS certificates");
                let bundle = generate_dev_bundle(&["localhost", "127.0.0.1", "0.0.0.0"])
                    .map_err(|e| TlsConfigError::CertGeneration(e.to_string()))?;
                write_dev_certs(cert_dir, &bundle)
                    .map_err(|e| TlsConfigError::CertGeneration(e.to_string()))?;

                let identity = Identity::from_pem(&bundle.server_cert_pem, &bundle.server_key_pem);
                let tls_config = ServerTlsConfig::new().identity(identity);

                info!(cert_dir = %cert_dir.display(), "Dev TLS enabled");
                Ok(Some(tls_config))
            }
            Self::Custom {
                cert_path,
                key_path,
            } => {
                let cert_pem = std::fs::read_to_string(cert_path).map_err(|e| {
                    TlsConfigError::FileRead(format!(
                        "Failed to read cert {}: {}",
                        cert_path.display(),
                        e
                    ))
                })?;
                let key_pem = std::fs::read_to_string(key_path).map_err(|e| {
                    TlsConfigError::FileRead(format!(
                        "Failed to read key {}: {}",
                        key_path.display(),
                        e
                    ))
                })?;

                let identity = Identity::from_pem(cert_pem, key_pem);
                let tls_config = ServerTlsConfig::new().identity(identity);

                info!(
                    cert = %cert_path.display(),
                    key = %key_path.display(),
                    "Custom TLS enabled"
                );
                Ok(Some(tls_config))
            }
        }
    }
}

/// Apply mutual TLS client authentication to an existing `ServerTlsConfig`.
///
/// Uses optional client auth so that non-tunnel endpoints (like Auth) can
/// work without a client certificate. The tunnel service enforces the
/// certificate requirement at the application layer.
pub fn apply_mtls(tls: ServerTlsConfig, client_ca_pem: &str) -> ServerTlsConfig {
    let ca_cert = Certificate::from_pem(client_ca_pem);
    tls.client_ca_root(ca_cert).client_auth_optional(true)
}

/// TLS configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("Certificate generation error: {0}")]
    CertGeneration(String),

    #[error("File read error: {0}")]
    FileRead(String),
}

impl From<CertError> for TlsConfigError {
    fn from(e: CertError) -> Self {
        Self::CertGeneration(e.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn disabled_returns_none() {
        let mode = TlsMode::Disabled;
        let result = mode.to_server_tls_config().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn dev_self_signed_returns_config() {
        let dir = std::env::temp_dir().join("betcode-tls-test");
        let _ = std::fs::remove_dir_all(&dir);

        let mode = TlsMode::DevSelfSigned {
            cert_dir: dir.clone(),
        };
        let result = mode.to_server_tls_config().unwrap();
        assert!(result.is_some());

        // Cert files should exist
        assert!(dir.join("ca.pem").exists());
        assert!(dir.join("server.pem").exists());
        assert!(dir.join("server-key.pem").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_missing_cert_returns_error() {
        let mode = TlsMode::Custom {
            cert_path: PathBuf::from("/nonexistent/cert.pem"),
            key_path: PathBuf::from("/nonexistent/key.pem"),
        };
        assert!(mode.to_server_tls_config().is_err());
    }

    #[test]
    fn apply_mtls_does_not_panic() {
        let dir = std::env::temp_dir().join("betcode-mtls-test");
        let _ = std::fs::remove_dir_all(&dir);

        let mode = TlsMode::DevSelfSigned {
            cert_dir: dir.clone(),
        };
        let tls = mode.to_server_tls_config().unwrap().unwrap();
        let ca_pem = std::fs::read_to_string(dir.join("ca.pem")).unwrap();

        // Should not panic
        let _tls_with_mtls = super::apply_mtls(tls, &ca_pem);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
