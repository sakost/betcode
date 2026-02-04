//! Tunnel client for connecting the daemon to a relay server.
//!
//! Provides outbound tunnel connectivity with automatic reconnection,
//! frame-level request handling, and heartbeat keepalive.

pub mod client;
pub mod config;
pub mod error;
pub mod handler;
pub mod heartbeat;

pub use client::TunnelClient;
pub use config::TunnelConfig;
pub use error::TunnelClientError;
pub use handler::TunnelRequestHandler;
