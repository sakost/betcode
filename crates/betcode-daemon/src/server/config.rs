//! Server configuration.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// TCP bind address (if using TCP transport).
    pub tcp_addr: Option<SocketAddr>,

    /// Unix socket path (if using Unix transport).
    pub unix_socket: Option<PathBuf>,

    /// Maximum concurrent sessions.
    pub max_sessions: usize,

    /// Maximum concurrent clients per session.
    pub max_clients_per_session: usize,

    /// Maximum concurrent subprocess pool size.
    pub max_processes: usize,

    /// Heartbeat timeout in seconds.
    pub heartbeat_timeout_secs: u64,

    /// Buffer size for subprocess stdin channel.
    pub stdin_channel_size: usize,

    /// Buffer size for subprocess stdout channel.
    pub stdout_channel_size: usize,

    /// HTTP/2 keepalive interval in seconds.
    pub keepalive_interval_secs: u64,

    /// HTTP/2 keepalive timeout in seconds.
    pub keepalive_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            tcp_addr: Some(SocketAddr::from(([127, 0, 0, 1], 50051))),
            unix_socket: None,
            max_sessions: 10,
            max_clients_per_session: 5,
            max_processes: 5,
            heartbeat_timeout_secs: 30,
            stdin_channel_size: 32,
            stdout_channel_size: 256,
            keepalive_interval_secs: 30,
            keepalive_timeout_secs: 10,
        }
    }
}

impl ServerConfig {
    /// Create a new server config with TCP transport.
    pub fn tcp(addr: SocketAddr) -> Self {
        Self {
            tcp_addr: Some(addr),
            unix_socket: None,
            ..Default::default()
        }
    }

    /// Create a new server config with Unix socket transport.
    #[cfg(unix)]
    pub fn unix(path: PathBuf) -> Self {
        Self {
            tcp_addr: None,
            unix_socket: Some(path),
            ..Default::default()
        }
    }

    /// Set max sessions.
    #[must_use]
    pub const fn with_max_sessions(mut self, max: usize) -> Self {
        self.max_sessions = max;
        self
    }

    /// Set max clients per session.
    #[must_use]
    pub const fn with_max_clients_per_session(mut self, max: usize) -> Self {
        self.max_clients_per_session = max;
        self
    }

    /// Set max concurrent subprocess pool size.
    #[must_use]
    pub const fn with_max_processes(mut self, max: usize) -> Self {
        self.max_processes = max;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = ServerConfig::default();
        assert!(config.tcp_addr.is_some());
        assert_eq!(config.max_sessions, 10);
    }

    #[test]
    fn tcp_config() {
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let config = ServerConfig::tcp(addr);
        assert_eq!(config.tcp_addr, Some(addr));
        assert!(config.unix_socket.is_none());
    }
}
