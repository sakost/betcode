//! `CommandService` gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use betcode_proto::v1::command_service_server::CommandService;
use betcode_proto::v1::{
    AddPluginRequest, AddPluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, ExecuteServiceCommandRequest,
    GetCommandRegistryRequest, GetCommandRegistryResponse, GetPluginStatusRequest,
    GetPluginStatusResponse, ListAgentsRequest, ListAgentsResponse, ListPathRequest,
    ListPathResponse, ListPluginsRequest, ListPluginsResponse, RemovePluginRequest,
    RemovePluginResponse, ServiceCommandOutput,
};

use crate::commands::CommandRegistry;
use crate::commands::service_executor::{ServiceExecutor, ServiceOutput};
use crate::completion::agent_lister::AgentLister;
use crate::completion::file_index::FileIndex;
use crate::plugin::manager::PluginManager;

/// `CommandService` gRPC handler.
#[derive(Clone)]
pub struct CommandServiceImpl {
    registry: Arc<RwLock<CommandRegistry>>,
    file_index: Arc<RwLock<FileIndex>>,
    agent_lister: Arc<RwLock<AgentLister>>,
    service_executor: Arc<RwLock<ServiceExecutor>>,
    plugin_manager: Arc<RwLock<PluginManager>>,
    /// When sent `true`, triggers graceful daemon shutdown.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl CommandServiceImpl {
    pub const fn new(
        registry: Arc<RwLock<CommandRegistry>>,
        file_index: Arc<RwLock<FileIndex>>,
        agent_lister: Arc<RwLock<AgentLister>>,
        service_executor: Arc<RwLock<ServiceExecutor>>,
        plugin_manager: Arc<RwLock<PluginManager>>,
        shutdown_tx: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self {
            registry,
            file_index,
            agent_lister,
            service_executor,
            plugin_manager,
            shutdown_tx,
        }
    }
}

/// Normalise a `max_results` value: 0 means "use default" (20).
const fn effective_max(max_results: u32) -> usize {
    if max_results == 0 {
        20
    } else {
        max_results as usize
    }
}

type ServiceCommandStream =
    Pin<Box<dyn Stream<Item = Result<ServiceCommandOutput, Status>> + Send>>;

#[allow(clippy::too_many_lines)]
#[tonic::async_trait]
impl CommandService for CommandServiceImpl {
    type ExecuteServiceCommandStream = ServiceCommandStream;

    #[allow(clippy::significant_drop_tightening)]
    async fn get_command_registry(
        &self,
        request: Request<GetCommandRegistryRequest>,
    ) -> Result<Response<GetCommandRegistryResponse>, Status> {
        let session_id = &request.get_ref().session_id;
        let registry = self.registry.read().await;
        let commands = registry
            .get_for_session(session_id)
            .into_iter()
            .map(core_entry_to_proto)
            .collect();
        Ok(Response::new(GetCommandRegistryResponse { commands }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        let req = request.into_inner();
        let max = effective_max(req.max_results);
        let lister = self.agent_lister.read().await;
        let agents = lister
            .search(&req.query, max)
            .into_iter()
            .map(core_agent_to_proto)
            .collect();
        Ok(Response::new(ListAgentsResponse { agents }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn list_path(
        &self,
        request: Request<ListPathRequest>,
    ) -> Result<Response<ListPathResponse>, Status> {
        let req = request.into_inner();
        let max = effective_max(req.max_results);
        let index = self.file_index.read().await;
        let entries = index
            .search(&req.query, max)
            .into_iter()
            .map(core_path_to_proto)
            .collect();
        Ok(Response::new(ListPathResponse { entries }))
    }

    async fn execute_service_command(
        &self,
        request: Request<ExecuteServiceCommandRequest>,
    ) -> Result<Response<Self::ExecuteServiceCommandStream>, Status> {
        let req = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<ServiceCommandOutput, Status>>(64);

        let executor = Arc::clone(&self.service_executor);
        let registry = Arc::clone(&self.registry);
        let file_index = Arc::clone(&self.file_index);
        let agent_lister = Arc::clone(&self.agent_lister);
        let shutdown_tx = self.shutdown_tx.clone();
        let command = req.command;
        let args = req.args;
        let session_id = req.session_id;

        tokio::spawn(async move {
            match command.as_str() {
                "pwd" => {
                    let result = executor.read().await.execute_pwd();
                    send_result(&tx, result).await;
                }
                "cd" => {
                    let path = args.first().map_or("~", std::string::String::as_str);
                    let mut exec = executor.write().await;
                    let result = match exec.execute_cd(path) {
                        Ok(()) => {
                            let cwd = exec.cwd().display().to_string();
                            drop(exec);
                            Ok(cwd)
                        }
                        Err(e) => Err(e),
                    };
                    send_result(&tx, result).await;
                }
                "bash" => {
                    let cmd = args.first().map_or("", std::string::String::as_str);
                    if cmd.is_empty() {
                        let _ = tx.send(Ok(error_output("No command provided"))).await;
                        return;
                    }
                    let (output_tx, mut output_rx) = mpsc::channel::<ServiceOutput>(64);
                    #[allow(clippy::significant_drop_tightening)]
                    let exec_result = {
                        let exec = executor.read().await;
                        exec.execute_bash(cmd, output_tx).await
                    };

                    // Forward output
                    while let Some(output) = output_rx.recv().await {
                        let proto = match output {
                            ServiceOutput::Stdout(line) => stdout_output(&line),
                            ServiceOutput::Stderr(line) => stderr_output(&line),
                            ServiceOutput::ExitCode(code) => exit_code_output(code),
                            ServiceOutput::Error(e) => error_output(&e),
                        };
                        if tx.send(Ok(proto)).await.is_err() {
                            break;
                        }
                    }

                    if let Err(e) = exec_result {
                        let _ = tx.send(Ok(error_output(&e.to_string()))).await;
                    }
                }
                "exit-daemon" => {
                    let _ = tx.send(Ok(stdout_output("Daemon shutting down..."))).await;
                    // Give a brief moment for the response to be sent before triggering shutdown
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let _ = shutdown_tx.send(true);
                }
                "reload-remote" => {
                    use crate::completion::agent_lister::{AgentInfo, AgentKind, AgentStatus};

                    let exec = executor.read().await;
                    let cwd = exec.cwd().to_path_buf();
                    let mut reg = registry.write().await;
                    let cmd_msg = match exec.execute_reload_remote(&mut reg, &session_id).await {
                        Ok(msg) => msg,
                        Err(e) => {
                            let _ = tx.send(Ok(error_output(&e.to_string()))).await;
                            return;
                        }
                    };
                    drop(exec);
                    drop(reg);

                    // Reload agents
                    let agents = betcode_core::commands::discover_agents(&cwd);
                    let agent_count = agents.len();
                    {
                        let mut lister = agent_lister.write().await;
                        *lister = crate::completion::agent_lister::AgentLister::new();
                        for name in agents {
                            lister.update(AgentInfo {
                                name,
                                kind: AgentKind::ClaudeInternal,
                                status: AgentStatus::Idle,
                                session_id: None,
                            });
                        }
                    }

                    // Reload file index
                    let file_count = match FileIndex::build(&cwd, 10_000).await {
                        Ok(new_index) => {
                            let count = new_index.entry_count();
                            *file_index.write().await = new_index;
                            count
                        }
                        Err(_) => 0,
                    };

                    let msg = format!("{cmd_msg}, {agent_count} agents, {file_count} files");
                    let _ = tx.send(Ok(stdout_output(&msg))).await;
                }
                other => {
                    let _ = tx
                        .send(Ok(error_output(&format!(
                            "Unknown service command: {other}"
                        ))))
                        .await;
                }
            }
        });

        let stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn list_plugins(
        &self,
        _request: Request<ListPluginsRequest>,
    ) -> Result<Response<ListPluginsResponse>, Status> {
        let mgr = self.plugin_manager.read().await;
        let plugins = mgr
            .list_plugins()
            .into_iter()
            .map(plugin_summary_to_proto)
            .collect();
        Ok(Response::new(ListPluginsResponse { plugins }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn get_plugin_status(
        &self,
        request: Request<GetPluginStatusRequest>,
    ) -> Result<Response<GetPluginStatusResponse>, Status> {
        let name = &request.get_ref().name;
        let mgr = self.plugin_manager.read().await;
        let client = mgr
            .get_plugin_status(name)
            .ok_or_else(|| Status::not_found(format!("plugin '{name}' not found")))?;
        let plugin = Some(plugin_client_to_proto(client));
        Ok(Response::new(GetPluginStatusResponse { plugin }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn add_plugin(
        &self,
        request: Request<AddPluginRequest>,
    ) -> Result<Response<AddPluginResponse>, Status> {
        let req = request.into_inner();
        if req.name.is_empty() {
            return Err(Status::invalid_argument("plugin name must not be empty"));
        }
        if req.socket_path.is_empty() {
            return Err(Status::invalid_argument("socket_path must not be empty"));
        }
        let mut mgr = self.plugin_manager.write().await;
        mgr.add_plugin(
            &req.name,
            &req.socket_path,
            std::time::Duration::from_secs(30),
        );
        let client = mgr
            .get_plugin_status(&req.name)
            .ok_or_else(|| Status::internal("failed to retrieve newly added plugin"))?;
        let plugin = Some(plugin_client_to_proto(client));
        Ok(Response::new(AddPluginResponse { plugin }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn remove_plugin(
        &self,
        request: Request<RemovePluginRequest>,
    ) -> Result<Response<RemovePluginResponse>, Status> {
        let name = &request.get_ref().name;
        let mut mgr = self.plugin_manager.write().await;
        let existed = mgr.get_plugin_status(name).is_some();
        mgr.remove_plugin(name);
        drop(mgr);
        Ok(Response::new(RemovePluginResponse { removed: existed }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn enable_plugin(
        &self,
        request: Request<EnablePluginRequest>,
    ) -> Result<Response<EnablePluginResponse>, Status> {
        let name = &request.get_ref().name;
        let mut mgr = self.plugin_manager.write().await;
        mgr.enable_plugin(name)
            .map_err(|e| Status::not_found(e.to_string()))?;
        let client = mgr
            .get_plugin_status(name)
            .ok_or_else(|| Status::internal("plugin disappeared after enable"))?;
        let plugin = Some(plugin_client_to_proto(client));
        Ok(Response::new(EnablePluginResponse { plugin }))
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn disable_plugin(
        &self,
        request: Request<DisablePluginRequest>,
    ) -> Result<Response<DisablePluginResponse>, Status> {
        let name = &request.get_ref().name;
        let mut mgr = self.plugin_manager.write().await;
        mgr.disable_plugin(name)
            .map_err(|e| Status::not_found(e.to_string()))?;
        let client = mgr
            .get_plugin_status(name)
            .ok_or_else(|| Status::internal("plugin disappeared after disable"))?;
        let plugin = Some(plugin_client_to_proto(client));
        Ok(Response::new(DisablePluginResponse { plugin }))
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers: core types -> proto types
// ---------------------------------------------------------------------------

fn core_entry_to_proto(
    entry: betcode_core::commands::CommandEntry,
) -> betcode_proto::v1::CommandEntry {
    betcode_proto::v1::CommandEntry {
        name: entry.name,
        description: entry.description,
        category: core_category_to_proto(&entry.category) as i32,
        execution_mode: core_exec_mode_to_proto(&entry.execution_mode) as i32,
        source: entry.source,
        args_schema: entry.args_schema,
        group: entry.group.unwrap_or_default(),
        display_name: entry.display_name.unwrap_or_default(),
    }
}

const fn core_category_to_proto(
    cat: &betcode_core::commands::CommandCategory,
) -> betcode_proto::v1::CommandCategory {
    match cat {
        betcode_core::commands::CommandCategory::Service => {
            betcode_proto::v1::CommandCategory::Service
        }
        betcode_core::commands::CommandCategory::ClaudeCode => {
            betcode_proto::v1::CommandCategory::ClaudeCode
        }
        betcode_core::commands::CommandCategory::Plugin => {
            betcode_proto::v1::CommandCategory::Plugin
        }
        betcode_core::commands::CommandCategory::Skill => betcode_proto::v1::CommandCategory::Skill,
        betcode_core::commands::CommandCategory::Mcp => betcode_proto::v1::CommandCategory::Mcp,
    }
}

const fn core_exec_mode_to_proto(
    mode: &betcode_core::commands::ExecutionMode,
) -> betcode_proto::v1::ExecutionMode {
    match mode {
        betcode_core::commands::ExecutionMode::Local => betcode_proto::v1::ExecutionMode::Local,
        betcode_core::commands::ExecutionMode::Passthrough => {
            betcode_proto::v1::ExecutionMode::Passthrough
        }
        betcode_core::commands::ExecutionMode::Plugin => betcode_proto::v1::ExecutionMode::Plugin,
    }
}

fn core_agent_to_proto(
    agent: crate::completion::agent_lister::AgentInfo,
) -> betcode_proto::v1::AgentInfo {
    use crate::completion::agent_lister::{AgentKind, AgentStatus};

    let kind = match agent.kind {
        AgentKind::ClaudeInternal => betcode_proto::v1::AgentKind::ClaudeInternal as i32,
        AgentKind::DaemonOrchestrated => betcode_proto::v1::AgentKind::DaemonOrchestrated as i32,
        AgentKind::TeamMember => betcode_proto::v1::AgentKind::TeamMember as i32,
    };

    let status = match agent.status {
        AgentStatus::Idle => betcode_proto::v1::CommandAgentStatus::Idle as i32,
        AgentStatus::Working => betcode_proto::v1::CommandAgentStatus::Working as i32,
        AgentStatus::Done => betcode_proto::v1::CommandAgentStatus::Done as i32,
        AgentStatus::Failed => betcode_proto::v1::CommandAgentStatus::Failed as i32,
    };

    betcode_proto::v1::AgentInfo {
        name: agent.name,
        kind,
        status,
        source: String::new(),
        session_id: agent.session_id,
    }
}

fn core_path_to_proto(
    indexed: crate::completion::file_index::IndexedPath,
) -> betcode_proto::v1::PathEntry {
    use crate::completion::file_index::PathKind;

    let kind = match indexed.kind {
        PathKind::File => betcode_proto::v1::PathKind::File as i32,
        PathKind::Directory => betcode_proto::v1::PathKind::Directory as i32,
        PathKind::Symlink => betcode_proto::v1::PathKind::Symlink as i32,
    };

    betcode_proto::v1::PathEntry {
        path: indexed.path,
        kind,
        size: 0,
        modified_at: 0,
    }
}

// ---------------------------------------------------------------------------
// Plugin conversion helpers: core types -> proto types
// ---------------------------------------------------------------------------

fn plugin_status_to_string(status: &crate::plugin::client::PluginStatus) -> String {
    match status {
        crate::plugin::client::PluginStatus::Healthy => "healthy".to_string(),
        crate::plugin::client::PluginStatus::Degraded => "degraded".to_string(),
        crate::plugin::client::PluginStatus::Unavailable => "unavailable".to_string(),
    }
}

fn plugin_summary_to_proto(
    summary: crate::plugin::manager::PluginSummary,
) -> betcode_proto::v1::PluginInfo {
    let healthy = summary.status == crate::plugin::client::PluginStatus::Healthy;
    betcode_proto::v1::PluginInfo {
        name: summary.name,
        status: plugin_status_to_string(&summary.status),
        enabled: summary.enabled,
        socket_path: String::new(),
        command_count: u32::try_from(summary.command_count).unwrap_or(u32::MAX),
        health_message: None,
        healthy: Some(healthy),
    }
}

fn plugin_client_to_proto(
    client: &crate::plugin::client::PluginClient,
) -> betcode_proto::v1::PluginInfo {
    let status = client.health.status();
    let healthy = status == crate::plugin::client::PluginStatus::Healthy;
    betcode_proto::v1::PluginInfo {
        name: client.name.clone(),
        status: plugin_status_to_string(&status),
        enabled: client.enabled,
        socket_path: client.socket_path.clone(),
        command_count: 0,
        health_message: None,
        healthy: Some(healthy),
    }
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

/// Send a `Result<String, E>` as either a stdout or error output event.
async fn send_result<E: std::fmt::Display>(
    tx: &mpsc::Sender<Result<ServiceCommandOutput, Status>>,
    result: Result<String, E>,
) {
    match result {
        Ok(msg) => {
            let _ = tx.send(Ok(stdout_output(&msg))).await;
        }
        Err(e) => {
            let _ = tx.send(Ok(error_output(&e.to_string()))).await;
        }
    }
}

fn stdout_output(line: &str) -> ServiceCommandOutput {
    ServiceCommandOutput {
        output: Some(
            betcode_proto::v1::service_command_output::Output::StdoutLine(line.to_string()),
        ),
    }
}

fn stderr_output(line: &str) -> ServiceCommandOutput {
    ServiceCommandOutput {
        output: Some(
            betcode_proto::v1::service_command_output::Output::StderrLine(line.to_string()),
        ),
    }
}

const fn exit_code_output(code: i32) -> ServiceCommandOutput {
    ServiceCommandOutput {
        output: Some(betcode_proto::v1::service_command_output::Output::ExitCode(
            code,
        )),
    }
}

fn error_output(msg: &str) -> ServiceCommandOutput {
    ServiceCommandOutput {
        output: Some(betcode_proto::v1::service_command_output::Output::Error(
            msg.to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
async fn create_test_service() -> CommandServiceImpl {
    create_test_service_with_dir(std::env::temp_dir().as_path()).await
}

#[cfg(test)]
async fn create_test_service_with_dir(path: &std::path::Path) -> CommandServiceImpl {
    let registry = Arc::new(RwLock::new(CommandRegistry::new()));
    let file_index = Arc::new(RwLock::new(
        FileIndex::build(path, 1000)
            .await
            .unwrap_or_else(|_| FileIndex::empty()),
    ));
    let agent_lister = Arc::new(RwLock::new(AgentLister::new()));
    let service_executor = Arc::new(RwLock::new(ServiceExecutor::new(path.to_path_buf())));
    let plugin_manager = Arc::new(RwLock::new(PluginManager::new()));
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    CommandServiceImpl::new(
        registry,
        file_index,
        agent_lister,
        service_executor,
        plugin_manager,
        shutdown_tx,
    )
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::items_after_statements,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::implicit_clone
)]
mod tests {
    use super::*;
    use betcode_proto::v1::*;

    /// Add a test plugin to the service and return the plugin info.
    async fn add_test_plugin(svc: &CommandServiceImpl) -> betcode_proto::v1::PluginInfo {
        svc.add_plugin(tonic::Request::new(AddPluginRequest {
            name: "test-plugin".to_string(),
            socket_path: "/tmp/test.sock".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .plugin
        .unwrap()
    }

    /// Execute a service command and return the first output event.
    async fn exec_first_output(
        svc: &CommandServiceImpl,
        command: &str,
        args: Vec<String>,
    ) -> service_command_output::Output {
        use tokio_stream::StreamExt;
        let request = tonic::Request::new(ExecuteServiceCommandRequest {
            command: command.to_string(),
            args,
            session_id: "test-session".to_string(),
        });
        let response = svc.execute_service_command(request).await.unwrap();
        let mut stream = response.into_inner();
        stream.next().await.unwrap().unwrap().output.unwrap()
    }

    #[tokio::test]
    async fn test_get_command_registry() {
        let service = create_test_service().await;
        let request = tonic::Request::new(GetCommandRegistryRequest {
            session_id: "test-session".to_string(),
        });
        let response = service.get_command_registry(request).await.unwrap();
        let entries = response.into_inner().commands;
        assert!(entries.iter().any(|e| e.name == "cd"));
        assert!(entries.iter().any(|e| e.name == "pwd"));
    }

    #[tokio::test]
    async fn test_list_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.rs"), "").unwrap();
        let service = create_test_service_with_dir(dir.path()).await;
        let request = tonic::Request::new(ListPathRequest {
            query: "test".to_string(),
            max_results: 10,
        });
        let response = service.list_path(request).await.unwrap();
        assert!(!response.into_inner().entries.is_empty());
    }

    #[tokio::test]
    async fn test_list_agents_empty() {
        let service = create_test_service().await;
        let request = tonic::Request::new(ListAgentsRequest {
            query: String::new(),
            max_results: 10,
        });
        let response = service.list_agents(request).await.unwrap();
        assert!(response.into_inner().agents.is_empty());
    }

    #[tokio::test]
    async fn test_execute_pwd() {
        let dir = tempfile::TempDir::new().unwrap();
        let service = create_test_service_with_dir(dir.path()).await;
        let output = exec_first_output(&service, "pwd", vec![]).await;
        assert!(matches!(
            output,
            service_command_output::Output::StdoutLine(_)
        ));
    }

    #[tokio::test]
    async fn test_execute_unknown_command() {
        let service = create_test_service().await;
        let output = exec_first_output(&service, "nonexistent", vec![]).await;
        match output {
            service_command_output::Output::Error(msg) => {
                assert!(msg.contains("Unknown service command"));
            }
            other => panic!("Expected Error output, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_exit_daemon_triggers_shutdown() {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let registry = Arc::new(RwLock::new(CommandRegistry::new()));
        let file_index = Arc::new(RwLock::new(FileIndex::empty()));
        let agent_lister = Arc::new(RwLock::new(AgentLister::new()));
        let service_executor = Arc::new(RwLock::new(ServiceExecutor::new(
            std::env::temp_dir().to_path_buf(),
        )));
        let plugin_manager = Arc::new(RwLock::new(PluginManager::new()));
        let service = CommandServiceImpl::new(
            registry,
            file_index,
            agent_lister,
            service_executor,
            plugin_manager,
            shutdown_tx,
        );
        let request = tonic::Request::new(ExecuteServiceCommandRequest {
            command: "exit-daemon".to_string(),
            args: vec![],
            session_id: "test-session".to_string(),
        });
        let response = service.execute_service_command(request).await.unwrap();
        let mut stream = response.into_inner();
        use tokio_stream::StreamExt;

        // First message should confirm shutdown
        let first = stream.next().await.unwrap().unwrap();
        match first.output {
            Some(service_command_output::Output::StdoutLine(msg)) => {
                assert!(
                    msg.contains("shutting down"),
                    "Expected shutdown message, got: {msg}"
                );
            }
            other => panic!("Expected StdoutLine, got {other:?}"),
        }

        // Wait for the shutdown signal (the command sleeps 100ms before sending)
        tokio::time::timeout(std::time::Duration::from_secs(2), shutdown_rx.changed())
            .await
            .expect("Timed out waiting for shutdown signal")
            .expect("Shutdown channel closed unexpectedly");
        assert!(*shutdown_rx.borrow(), "Shutdown signal should be true");
    }

    #[tokio::test]
    async fn test_list_plugins_empty() {
        let service = create_test_service().await;
        let resp = service
            .list_plugins(tonic::Request::new(ListPluginsRequest {}))
            .await
            .unwrap();
        assert!(resp.into_inner().plugins.is_empty());
    }

    #[tokio::test]
    async fn test_add_and_list_plugins() {
        let service = create_test_service().await;
        let resp = service
            .add_plugin(tonic::Request::new(AddPluginRequest {
                name: "test-plugin".to_string(),
                socket_path: "/tmp/test.sock".to_string(),
            }))
            .await
            .unwrap();
        let info = resp.into_inner().plugin.unwrap();
        assert_eq!(info.name, "test-plugin");
        assert!(info.enabled);
        assert_eq!(info.socket_path, "/tmp/test.sock");

        let list = service
            .list_plugins(tonic::Request::new(ListPluginsRequest {}))
            .await
            .unwrap();
        assert_eq!(list.into_inner().plugins.len(), 1);
    }

    #[tokio::test]
    async fn test_add_plugin_empty_name() {
        let service = create_test_service().await;
        let err = service
            .add_plugin(tonic::Request::new(AddPluginRequest {
                name: String::new(),
                socket_path: "/tmp/test.sock".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_get_plugin_status_found() {
        let service = create_test_service().await;
        add_test_plugin(&service).await;

        let resp = service
            .get_plugin_status(tonic::Request::new(GetPluginStatusRequest {
                name: "test-plugin".to_string(),
            }))
            .await
            .unwrap();
        let info = resp.into_inner().plugin.unwrap();
        assert_eq!(info.name, "test-plugin");
        assert_eq!(info.status, "healthy");
    }

    #[tokio::test]
    async fn test_get_plugin_status_not_found() {
        let service = create_test_service().await;
        let err = service
            .get_plugin_status(tonic::Request::new(GetPluginStatusRequest {
                name: "nonexistent".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_remove_plugin() {
        let service = create_test_service().await;
        add_test_plugin(&service).await;

        let resp = service
            .remove_plugin(tonic::Request::new(RemovePluginRequest {
                name: "test-plugin".to_string(),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().removed);

        let list = service
            .list_plugins(tonic::Request::new(ListPluginsRequest {}))
            .await
            .unwrap();
        assert!(list.into_inner().plugins.is_empty());
    }

    #[tokio::test]
    async fn test_remove_plugin_nonexistent() {
        let service = create_test_service().await;
        let resp = service
            .remove_plugin(tonic::Request::new(RemovePluginRequest {
                name: "nonexistent".to_string(),
            }))
            .await
            .unwrap();
        assert!(!resp.into_inner().removed);
    }

    #[tokio::test]
    async fn test_disable_and_enable_plugin() {
        let service = create_test_service().await;
        add_test_plugin(&service).await;

        // Disable
        let resp = service
            .disable_plugin(tonic::Request::new(DisablePluginRequest {
                name: "test-plugin".to_string(),
            }))
            .await
            .unwrap();
        let info = resp.into_inner().plugin.unwrap();
        assert!(!info.enabled);

        // Enable
        let resp = service
            .enable_plugin(tonic::Request::new(EnablePluginRequest {
                name: "test-plugin".to_string(),
            }))
            .await
            .unwrap();
        let info = resp.into_inner().plugin.unwrap();
        assert!(info.enabled);
    }

    #[tokio::test]
    async fn test_enable_plugin_not_found() {
        let service = create_test_service().await;
        let err = service
            .enable_plugin(tonic::Request::new(EnablePluginRequest {
                name: "nonexistent".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_disable_plugin_not_found() {
        let service = create_test_service().await;
        let err = service
            .disable_plugin(tonic::Request::new(DisablePluginRequest {
                name: "nonexistent".to_string(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }
}
