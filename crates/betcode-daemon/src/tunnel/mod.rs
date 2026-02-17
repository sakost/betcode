//! Tunnel client for connecting the daemon to a relay server.
//!
//! Provides outbound tunnel connectivity with automatic reconnection,
//! frame-level request handling, heartbeat keepalive, and certificate
//! rotation.

pub mod cert_rotation;
pub mod client;
pub mod config;
pub mod error;
pub mod handler;
pub mod heartbeat;

pub use cert_rotation::{RotationResult, spawn_cert_monitor};
pub use client::TunnelClient;
pub use config::TunnelConfig;
pub use error::TunnelClientError;
pub use handler::TunnelRequestHandler;
