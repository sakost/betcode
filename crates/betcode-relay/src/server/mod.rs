//! gRPC server implementations for BetCode relay.

pub mod auth_svc;

#[cfg(test)]
mod auth_svc_tests;

pub use auth_svc::AuthServiceImpl;
