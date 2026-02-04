//! Tunnel client configuration.

use std::path::PathBuf;
use std::time::Duration;

/// Configuration for the daemon's tunnel connection to the relay.
#[derive(Debug, Clone)]
pub struct TunnelConfig {
    /// Relay server URL (e.g., "https://relay.betcode.io:443").
    pub relay_url: String,

    /// Machine identifier for this daemon.
    pub machine_id: String,

    /// Human-readable machine name.
    pub machine_name: String,

    /// Username for relay authentication.
    pub username: String,

    /// Password for relay authentication.
    pub password: String,

    /// Reconnection policy.
    pub reconnect: ReconnectPolicy,

    /// Heartbeat interval.
    pub heartbeat_interval: Duration,

    /// Path to CA certificate for verifying the relay's TLS certificate.
    /// When set, the client will use TLS with this CA cert.
    pub ca_cert_path: Option<PathBuf>,
}

/// Exponential backoff reconnection policy.
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// Initial delay before first reconnect attempt.
    pub initial_delay: Duration,
    /// Maximum delay between reconnect attempts.
    pub max_delay: Duration,
    /// Multiplier applied to delay after each failed attempt.
    pub multiplier: f64,
    /// Maximum number of reconnect attempts (None = unlimited).
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: None,
        }
    }
}

impl ReconnectPolicy {
    /// Calculate the delay for a given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_ms = self.initial_delay.as_millis() as f64;
        let delay_ms = base_ms * self.multiplier.powi(attempt as i32);
        let capped_ms = delay_ms.min(self.max_delay.as_millis() as f64);
        Duration::from_millis(capped_ms as u64)
    }

    /// Whether another attempt should be made.
    pub fn should_retry(&self, attempt: u32) -> bool {
        match self.max_attempts {
            Some(max) => attempt < max,
            None => true,
        }
    }
}

impl TunnelConfig {
    /// Create a new tunnel config with required fields and defaults.
    pub fn new(
        relay_url: String,
        machine_id: String,
        machine_name: String,
        username: String,
        password: String,
    ) -> Self {
        Self {
            relay_url,
            machine_id,
            machine_name,
            username,
            password,
            reconnect: ReconnectPolicy::default(),
            heartbeat_interval: Duration::from_secs(30),
            ca_cert_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reconnect_policy() {
        let policy = ReconnectPolicy::default();
        assert_eq!(policy.initial_delay, Duration::from_secs(1));
        assert_eq!(policy.max_delay, Duration::from_secs(60));
        assert_eq!(policy.multiplier, 2.0);
        assert!(policy.max_attempts.is_none());
    }

    #[test]
    fn exponential_backoff_delays() {
        let policy = ReconnectPolicy::default();

        // 1s, 2s, 4s, 8s, 16s, 32s, 60s (capped), 60s
        assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(2));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(4));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_secs(8));
        assert_eq!(policy.delay_for_attempt(4), Duration::from_secs(16));
        assert_eq!(policy.delay_for_attempt(5), Duration::from_secs(32));
        assert_eq!(policy.delay_for_attempt(6), Duration::from_secs(60)); // capped
        assert_eq!(policy.delay_for_attempt(7), Duration::from_secs(60)); // still capped
    }

    #[test]
    fn retry_with_max_attempts() {
        let policy = ReconnectPolicy {
            max_attempts: Some(3),
            ..Default::default()
        };

        assert!(policy.should_retry(0));
        assert!(policy.should_retry(1));
        assert!(policy.should_retry(2));
        assert!(!policy.should_retry(3));
        assert!(!policy.should_retry(4));
    }

    #[test]
    fn retry_unlimited() {
        let policy = ReconnectPolicy::default();
        assert!(policy.should_retry(0));
        assert!(policy.should_retry(100));
        assert!(policy.should_retry(u32::MAX));
    }

    #[test]
    fn tunnel_config_new() {
        let config = TunnelConfig::new(
            "https://relay.example.com:443".into(),
            "machine-1".into(),
            "My Machine".into(),
            "user".into(),
            "pass".into(),
        );

        assert_eq!(config.relay_url, "https://relay.example.com:443");
        assert_eq!(config.machine_id, "machine-1");
        assert_eq!(config.machine_name, "My Machine");
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert!(config.ca_cert_path.is_none());
    }
}
