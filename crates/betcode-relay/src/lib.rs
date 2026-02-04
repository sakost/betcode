//! BetCode Relay Server Library
//!
//! Core functionality for the BetCode relay:
//! - SQLite storage for users, machines, tokens, and message buffer
//! - JWT authentication and password hashing
//! - gRPC services (Auth, Tunnel, Machine)
//! - Connection registry for tunnel management
//! - Request routing through tunnels to daemons

pub mod auth;
pub mod registry;
pub mod router;
pub mod server;
pub mod storage;
pub mod tls;
