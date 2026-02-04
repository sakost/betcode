//! gRPC server implementations for BetCode relay.

pub mod agent_proxy;
pub mod auth_svc;
pub mod interceptor;
pub mod machine_svc;
pub mod tunnel_svc;

#[cfg(test)]
mod auth_svc_tests;

pub use agent_proxy::AgentProxyService;
pub use auth_svc::AuthServiceImpl;
pub use interceptor::jwt_interceptor;
pub use machine_svc::MachineServiceImpl;
pub use tunnel_svc::TunnelServiceImpl;
