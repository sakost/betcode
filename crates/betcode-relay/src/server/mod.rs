//! gRPC server implementations for BetCode relay.

pub mod auth_svc;
pub mod interceptor;
pub mod tunnel_svc;

#[cfg(test)]
mod auth_svc_tests;

pub use auth_svc::AuthServiceImpl;
pub use interceptor::jwt_interceptor;
pub use tunnel_svc::TunnelServiceImpl;
