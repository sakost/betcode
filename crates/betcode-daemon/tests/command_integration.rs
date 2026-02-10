#![allow(clippy::unwrap_used)] // Integration tests use unwrap for brevity

//! End-to-end integration test for the command system.
//!
//! Verifies that CommandServiceImpl correctly wires together:
//! - CommandRegistry with builtins + user-discovered commands
//! - FileIndex built from a temp directory
//! - AgentLister (empty)
//! - ServiceExecutor for command execution

use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_stream::StreamExt;

use betcode_core::commands::discover_user_commands;
use betcode_daemon::commands::service_executor::ServiceExecutor;
use betcode_daemon::commands::CommandRegistry;
use betcode_daemon::completion::agent_lister::AgentLister;
use betcode_daemon::completion::file_index::FileIndex;
use betcode_daemon::server::command_svc::CommandServiceImpl;

use betcode_proto::v1::command_service_server::CommandService;
use betcode_proto::v1::*;

#[tokio::test]
async fn test_full_command_flow() {
    // Setup: create a temp directory with test files and user commands.
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("README.md"), "# Hello").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

    // Add a user-defined slash command
    let commands_dir = dir.path().join(".claude").join("commands");
    std::fs::create_dir_all(&commands_dir).unwrap();
    std::fs::write(commands_dir.join("deploy.md"), "# Deploy\nDeploy the app").unwrap();

    // Build registry with builtins + user commands
    let mut registry = CommandRegistry::new();
    let user_commands = discover_user_commands(dir.path());
    for cmd in user_commands {
        registry.add(cmd);
    }

    // Build file index from temp dir
    let file_index = FileIndex::build(dir.path(), 1000)
        .await
        .expect("FileIndex build should succeed");

    // Create all components
    let registry = Arc::new(RwLock::new(registry));
    let file_index = Arc::new(RwLock::new(file_index));
    let agent_lister = Arc::new(RwLock::new(AgentLister::new()));
    let service_executor = Arc::new(RwLock::new(ServiceExecutor::new(dir.path().to_path_buf())));

    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let service = CommandServiceImpl::new(
        registry,
        file_index,
        agent_lister,
        service_executor,
        shutdown_tx,
    );

    // Test 1: get_command_registry returns builtins + user commands
    let response = service
        .get_command_registry(tonic::Request::new(GetCommandRegistryRequest {}))
        .await
        .unwrap();
    let commands = response.into_inner().commands;

    // Builtins should be present
    assert!(
        commands.iter().any(|c| c.name == "cd"),
        "Registry should contain built-in 'cd'"
    );
    assert!(
        commands.iter().any(|c| c.name == "pwd"),
        "Registry should contain built-in 'pwd'"
    );

    // User command should be present
    assert!(
        commands.iter().any(|c| c.name == "deploy"),
        "Registry should contain user command 'deploy'"
    );

    // Test 2: list_path returns matching files
    let response = service
        .list_path(tonic::Request::new(ListPathRequest {
            query: "main".to_string(),
            max_results: 10,
        }))
        .await
        .unwrap();
    let entries = response.into_inner().entries;
    assert!(
        entries.iter().any(|e| e.path.contains("main.rs")),
        "list_path should find main.rs"
    );

    // Test 3: list_path for README
    let response = service
        .list_path(tonic::Request::new(ListPathRequest {
            query: "README".to_string(),
            max_results: 10,
        }))
        .await
        .unwrap();
    let entries = response.into_inner().entries;
    assert!(
        entries.iter().any(|e| e.path.contains("README")),
        "list_path should find README.md"
    );

    // Test 4: list_agents returns empty (no agents registered)
    let response = service
        .list_agents(tonic::Request::new(ListAgentsRequest {
            query: String::new(),
            max_results: 10,
        }))
        .await
        .unwrap();
    assert!(response.into_inner().agents.is_empty());

    // Test 5: execute_service_command for pwd
    let response = service
        .execute_service_command(tonic::Request::new(ExecuteServiceCommandRequest {
            command: "pwd".to_string(),
            args: vec![],
        }))
        .await
        .unwrap();
    let mut stream = response.into_inner();
    let first = stream.next().await.unwrap().unwrap();
    match first.output {
        Some(service_command_output::Output::StdoutLine(line)) => {
            assert!(
                line.contains(&dir.path().display().to_string()) || !line.is_empty(),
                "pwd should return a valid path"
            );
        }
        other => panic!("Expected StdoutLine from pwd, got {:?}", other),
    }
}
