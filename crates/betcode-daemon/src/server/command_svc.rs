//! CommandService gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
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

use crate::commands::service_executor::{ServiceExecutor, ServiceOutput};
use crate::commands::CommandRegistry;
use crate::completion::agent_lister::AgentLister;
use crate::completion::file_index::FileIndex;

/// CommandService gRPC handler.
pub struct CommandServiceImpl {
    registry: Arc<RwLock<CommandRegistry>>,
    file_index: Arc<RwLock<FileIndex>>,
    agent_lister: Arc<RwLock<AgentLister>>,
    service_executor: Arc<RwLock<ServiceExecutor>>,
}

impl CommandServiceImpl {
    pub fn new(
        registry: Arc<RwLock<CommandRegistry>>,
        file_index: Arc<RwLock<FileIndex>>,
        agent_lister: Arc<RwLock<AgentLister>>,
        service_executor: Arc<RwLock<ServiceExecutor>>,
    ) -> Self {
        Self {
            registry,
            file_index,
            agent_lister,
            service_executor,
        }
    }
}

type ServiceCommandStream =
    Pin<Box<dyn Stream<Item = Result<ServiceCommandOutput, Status>> + Send>>;

#[tonic::async_trait]
impl CommandService for CommandServiceImpl {
    type ExecuteServiceCommandStream = ServiceCommandStream;

    async fn get_command_registry(
        &self,
        _request: Request<GetCommandRegistryRequest>,
    ) -> Result<Response<GetCommandRegistryResponse>, Status> {
        let registry = self.registry.read().await;
        let commands = registry
            .get_all()
            .into_iter()
            .map(core_entry_to_proto)
            .collect();
        Ok(Response::new(GetCommandRegistryResponse { commands }))
    }

    async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        let req = request.into_inner();
        let max = if req.max_results == 0 {
            20
        } else {
            req.max_results as usize
        };
        let lister = self.agent_lister.read().await;
        let agents = lister
            .search(&req.query, max)
            .into_iter()
            .map(core_agent_to_proto)
            .collect();
        Ok(Response::new(ListAgentsResponse { agents }))
    }

    async fn list_path(
        &self,
        request: Request<ListPathRequest>,
    ) -> Result<Response<ListPathResponse>, Status> {
        let req = request.into_inner();
        let max = if req.max_results == 0 {
            20
        } else {
            req.max_results as usize
        };
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
        let command = req.command;
        let args = req.args;

        tokio::spawn(async move {
            match command.as_str() {
                "pwd" => {
                    let exec = executor.read().await;
                    match exec.execute_pwd() {
                        Ok(cwd) => {
                            let _ = tx.send(Ok(stdout_output(&cwd))).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Ok(error_output(&e.to_string()))).await;
                        }
                    }
                }
                "cd" => {
                    let path = args.first().map(|s| s.as_str()).unwrap_or("~");
                    let mut exec = executor.write().await;
                    match exec.execute_cd(path) {
                        Ok(()) => {
                            let cwd = exec.cwd().display().to_string();
                            let _ = tx.send(Ok(stdout_output(&cwd))).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Ok(error_output(&e.to_string()))).await;
                        }
                    }
                }
                "bash" => {
                    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
                    if cmd.is_empty() {
                        let _ = tx.send(Ok(error_output("No command provided"))).await;
                        return;
                    }
                    let (output_tx, mut output_rx) = mpsc::channel::<ServiceOutput>(64);
                    let exec = executor.read().await;
                    let exec_result = exec.execute_bash(cmd, output_tx).await;

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
                    let _ = tx
                        .send(Ok(stdout_output("Daemon shutdown requested")))
                        .await;
                }
                "reload-commands" => {
                    let exec = executor.read().await;
                    let mut reg = registry.write().await;
                    match exec.execute_reload_commands(&mut reg) {
                        Ok(msg) => {
                            let _ = tx.send(Ok(stdout_output(&msg))).await;
                        }
                        Err(e) => {
                            let _ = tx.send(Ok(error_output(&e.to_string()))).await;
                        }
                    }
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

    async fn list_plugins(
        &self,
        _request: Request<ListPluginsRequest>,
    ) -> Result<Response<ListPluginsResponse>, Status> {
        Err(Status::unimplemented("Plugin management not yet available"))
    }

    async fn get_plugin_status(
        &self,
        _request: Request<GetPluginStatusRequest>,
    ) -> Result<Response<GetPluginStatusResponse>, Status> {
        Err(Status::unimplemented("Plugin management not yet available"))
    }

    async fn add_plugin(
        &self,
        _request: Request<AddPluginRequest>,
    ) -> Result<Response<AddPluginResponse>, Status> {
        Err(Status::unimplemented("Plugin management not yet available"))
    }

    async fn remove_plugin(
        &self,
        _request: Request<RemovePluginRequest>,
    ) -> Result<Response<RemovePluginResponse>, Status> {
        Err(Status::unimplemented("Plugin management not yet available"))
    }

    async fn enable_plugin(
        &self,
        _request: Request<EnablePluginRequest>,
    ) -> Result<Response<EnablePluginResponse>, Status> {
        Err(Status::unimplemented("Plugin management not yet available"))
    }

    async fn disable_plugin(
        &self,
        _request: Request<DisablePluginRequest>,
    ) -> Result<Response<DisablePluginResponse>, Status> {
        Err(Status::unimplemented("Plugin management not yet available"))
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
    }
}

fn core_category_to_proto(
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
    }
}

fn core_exec_mode_to_proto(
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
// Output helpers
// ---------------------------------------------------------------------------

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

fn exit_code_output(code: i32) -> ServiceCommandOutput {
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
    CommandServiceImpl::new(registry, file_index, agent_lister, service_executor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use betcode_proto::v1::*;

    #[tokio::test]
    async fn test_get_command_registry() {
        let service = create_test_service().await;
        let request = tonic::Request::new(GetCommandRegistryRequest {});
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
        let request = tonic::Request::new(ExecuteServiceCommandRequest {
            command: "pwd".to_string(),
            args: vec![],
        });
        let response = service.execute_service_command(request).await.unwrap();
        let mut stream = response.into_inner();
        use tokio_stream::StreamExt;
        let first = stream.next().await.unwrap().unwrap();
        assert!(first.output.is_some());
    }

    #[tokio::test]
    async fn test_execute_unknown_command() {
        let service = create_test_service().await;
        let request = tonic::Request::new(ExecuteServiceCommandRequest {
            command: "nonexistent".to_string(),
            args: vec![],
        });
        let response = service.execute_service_command(request).await.unwrap();
        let mut stream = response.into_inner();
        use tokio_stream::StreamExt;
        let first = stream.next().await.unwrap().unwrap();
        match first.output {
            Some(service_command_output::Output::Error(msg)) => {
                assert!(msg.contains("Unknown service command"));
            }
            other => panic!("Expected Error output, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_plugin_rpcs_unimplemented() {
        let service = create_test_service().await;
        let err = service
            .list_plugins(tonic::Request::new(ListPluginsRequest {}))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
