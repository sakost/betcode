//! `SQLite` storage for `BetCode` relay server.
//!
//! Provides persistence for users, tokens, machines, message buffer, and certificates.

mod db;
mod models;
mod queries;
mod queries_buffer;
mod queries_certs;
mod queries_notifications;

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests;

pub use db::{DatabaseError, RelayDatabase};
pub use models::*;
pub use queries_buffer::{BufferMessageParams, CertificateParams};
