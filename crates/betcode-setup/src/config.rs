use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Result, bail};

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
    pub addr: SocketAddr,
}

impl RelaySetupConfig {
    /// Validate the configuration. Returns an error on invalid values.
    pub fn validate(&self) -> Result<()> {
        if self.domain.is_empty() {
            bail!("domain must not be empty");
        }
        if self.jwt_secret.len() < 32 {
            bail!(
                "JWT secret must be at least 32 characters (got {})",
                self.jwt_secret.len()
            );
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

/// Deployment mode for the daemon.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonMode {
    /// System-level service (root, /etc/systemd/system/)
    System,
    /// User-level service (no root, ~/.config/systemd/user/)
    User,
}

#[cfg(unix)]
impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::User => write!(f, "user"),
        }
    }
}

/// Configuration collected for daemon deployment.
#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct DaemonSetupConfig {
    pub mode: DaemonMode,
    /// System user to run as (system mode only)
    pub user: String,
    pub addr: SocketAddr,
    pub db_path: PathBuf,
    pub max_processes: usize,
    pub max_sessions: usize,
    pub relay_url: Option<String>,
    pub machine_id: Option<String>,
    pub machine_name: String,
    pub relay_username: Option<String>,
    pub relay_password: Option<String>,
    pub relay_custom_ca_cert: Option<PathBuf>,
    pub worktree_dir: Option<PathBuf>,
    pub daemon_binary_path: Option<PathBuf>,
    /// Whether to enable lingering for user services
    pub enable_linger: bool,
}
