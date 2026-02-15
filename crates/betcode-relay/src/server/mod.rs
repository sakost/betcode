//! gRPC server implementations for `BetCode` relay.

pub mod agent_proxy;
pub mod auth_svc;
pub mod command_proxy;
pub mod config_proxy;
pub mod git_repo_proxy;
pub mod gitlab_proxy;
pub mod grpc_util;
pub mod interceptor;
pub mod machine_svc;
pub mod tunnel_svc;
pub mod worktree_proxy;

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod auth_svc_tests;
#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
pub(crate) mod test_helpers;
#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tunnel_svc_tests;

pub use agent_proxy::AgentProxyService;
pub use auth_svc::AuthServiceImpl;
pub use command_proxy::CommandProxyService;
pub use config_proxy::ConfigProxyService;
pub use git_repo_proxy::GitRepoProxyService;
pub use gitlab_proxy::GitLabProxyService;
pub use interceptor::jwt_interceptor;
pub use machine_svc::MachineServiceImpl;
pub use tunnel_svc::TunnelServiceImpl;
pub use worktree_proxy::WorktreeProxyService;
