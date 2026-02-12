use std::fmt;
use std::path::PathBuf;

use anyhow::{bail, Result};

/// Deployment mode for the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentMode {
    Systemd,
    Docker,
}

impl fmt::Display for DeploymentMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Systemd => write!(f, "systemd"),
            Self::Docker => write!(f, "docker"),
        }
    }
}

/// Configuration collected for relay deployment.
#[derive(Debug, Clone)]
pub struct RelaySetupConfig {
    pub domain: String,
    pub jwt_secret: String,
    pub db_path: PathBuf,
    pub deployment_mode: DeploymentMode,
    pub relay_binary_path: Option<PathBuf>,
}

impl RelaySetupConfig {
    /// Validate the configuration. Returns an error on invalid values.
    pub fn validate(&self) -> Result<()> {
        if self.domain.is_empty() {
            bail!("domain must not be empty");
        }
        if self.jwt_secret.len() < 32 {
            bail!("JWT secret must be at least 32 characters (got {})", self.jwt_secret.len());
        }
        if self.deployment_mode == DeploymentMode::Systemd
            && self.relay_binary_path.is_none()
            && !std::path::Path::new("/usr/local/bin/betcode-relay").exists()
        {
            bail!(
                "relay binary not found at /usr/local/bin/betcode-relay and --relay-binary not provided"
            );
        }
        Ok(())
    }
}
