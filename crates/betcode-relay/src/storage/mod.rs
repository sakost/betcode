//! SQLite storage for BetCode relay server.
//!
//! Provides persistence for users, tokens, machines, message buffer, and certificates.

mod db;
mod models;
mod queries;
mod queries_buffer;

#[cfg(test)]
mod tests;

pub use db::{DatabaseError, RelayDatabase};
pub use models::*;
