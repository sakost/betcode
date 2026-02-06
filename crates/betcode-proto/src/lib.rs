//! BetCode Protocol Buffers
//!
//! Generated protobuf code for the BetCode gRPC API.
//!
//! This crate contains:
//! - `AgentService` for conversation management
//! - `VersionService` for capability negotiation
//! - `ConfigService` for settings management
//! - `Health` services for health checking

#![allow(clippy::derive_partial_eq_without_eq)]

/// BetCode v1 API definitions.
///
/// All generated types and services are included here.
pub mod v1 {
    tonic::include_proto!("betcode.v1");
}

// Re-export v1 as the default API version for convenience
pub use v1::*;

// Re-export prost_types for downstream crates that need Struct/Value conversion
pub use prost_types;
