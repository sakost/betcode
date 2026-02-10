//! gRPC server implementations for BetCode relay.

pub mod agent_proxy;
pub mod auth_svc;
pub mod command_proxy;
pub mod gitlab_proxy;
pub mod grpc_util;
pub mod interceptor;
pub mod machine_svc;
pub mod tunnel_svc;
pub mod worktree_proxy;

#[cfg(test)]
mod auth_svc_tests;

pub use agent_proxy::AgentProxyService;
pub use auth_svc::AuthServiceImpl;
pub use command_proxy::CommandProxyService;
pub use gitlab_proxy::GitLabProxyService;
pub use interceptor::jwt_interceptor;
pub use machine_svc::MachineServiceImpl;
pub use tunnel_svc::TunnelServiceImpl;
pub use worktree_proxy::WorktreeProxyService;
