//! Tests for `CommandProxyService`.

use tokio_stream::StreamExt;

use betcode_proto::v1::command_service_server::CommandService;
use betcode_proto::v1::{
    AddPluginRequest, AddPluginResponse, AgentInfo, CommandEntry, DisablePluginRequest,
    DisablePluginResponse, EnablePluginRequest, EnablePluginResponse, ExecuteServiceCommandRequest,
    GetCommandRegistryRequest, GetCommandRegistryResponse, GetPluginStatusRequest,
    GetPluginStatusResponse, ListAgentsRequest, ListAgentsResponse, ListPathRequest,
    ListPathResponse, ListPluginsRequest, ListPluginsResponse, PathEntry, PluginInfo,
    RemovePluginRequest, RemovePluginResponse, ServiceCommandOutput,
};

use super::CommandProxyService;
use crate::server::test_helpers::{
    assert_daemon_error, assert_no_claims_error, assert_no_machine_error, assert_offline_error,
    assert_wrong_owner_error, make_request, proxy_test_setup, spawn_responder,
    spawn_stream_responder,
};

proxy_test_setup!(CommandProxyService);

// --- Unary RPC routing ---

#[tokio::test]
async fn get_command_registry_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        GetCommandRegistryResponse {
            commands: vec![CommandEntry {
                name: "test-cmd".into(),
                description: "A test command".into(),
                category: 1,       // COMMAND_CATEGORY_SERVICE
                execution_mode: 1, // EXECUTION_MODE_LOCAL
                source: "test".into(),
                args_schema: None,
            }],
        },
    );
    let req = make_request(GetCommandRegistryRequest {}, "m1");
    let resp = svc.get_command_registry(req).await.unwrap().into_inner();
    assert_eq!(resp.commands.len(), 1);
    assert_eq!(resp.commands[0].name, "test-cmd");
    assert_eq!(resp.commands[0].description, "A test command");
}

#[tokio::test]
async fn list_agents_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListAgentsResponse {
            agents: vec![AgentInfo {
                name: "test-agent".into(),
                kind: 1,
                status: 1,
                source: "builtin".into(),
                session_id: None,
            }],
        },
    );
    let req = make_request(
        ListAgentsRequest {
            query: "test".into(),
            max_results: 10,
        },
        "m1",
    );
    let resp = svc.list_agents(req).await.unwrap().into_inner();
    assert_eq!(resp.agents.len(), 1);
    assert_eq!(resp.agents[0].name, "test-agent");
    assert_eq!(resp.agents[0].source, "builtin");
}

#[tokio::test]
async fn list_path_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListPathResponse {
            entries: vec![PathEntry {
                path: "/home/src".into(),
                kind: 2, // PATH_KIND_DIRECTORY
                size: 0,
                modified_at: 0,
            }],
        },
    );
    let req = make_request(
        ListPathRequest {
            query: "/home".into(),
            max_results: 10,
        },
        "m1",
    );
    let resp = svc.list_path(req).await.unwrap().into_inner();
    assert_eq!(resp.entries.len(), 1);
    assert_eq!(resp.entries[0].path, "/home/src");
}

// --- Plugin RPC routing ---

#[tokio::test]
async fn list_plugins_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        ListPluginsResponse {
            plugins: vec![PluginInfo {
                name: "my-plugin".into(),
                status: "running".into(),
                enabled: true,
                socket_path: "/tmp/my-plugin.sock".into(),
                command_count: 3,
                health_message: None,
                healthy: Some(true),
            }],
        },
    );
    let req = make_request(ListPluginsRequest {}, "m1");
    let resp = svc.list_plugins(req).await.unwrap().into_inner();
    assert_eq!(resp.plugins.len(), 1);
    assert_eq!(resp.plugins[0].name, "my-plugin");
    assert!(resp.plugins[0].enabled);
}

#[tokio::test]
async fn get_plugin_status_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        GetPluginStatusResponse {
            plugin: Some(PluginInfo {
                name: "checker".into(),
                status: "running".into(),
                enabled: true,
                socket_path: "/tmp/checker.sock".into(),
                command_count: 2,
                health_message: Some("OK".into()),
                healthy: Some(true),
            }),
        },
    );
    let req = make_request(
        GetPluginStatusRequest {
            name: "checker".into(),
        },
        "m1",
    );
    let resp = svc.get_plugin_status(req).await.unwrap().into_inner();
    let plugin = resp.plugin.unwrap();
    assert_eq!(plugin.name, "checker");
    assert_eq!(plugin.command_count, 2);
}

#[tokio::test]
async fn add_plugin_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        AddPluginResponse {
            plugin: Some(PluginInfo {
                name: "new-plugin".into(),
                status: "running".into(),
                enabled: true,
                socket_path: "/tmp/new-plugin.sock".into(),
                command_count: 0,
                health_message: None,
                healthy: Some(true),
            }),
        },
    );
    let req = make_request(
        AddPluginRequest {
            name: "new-plugin".into(),
            socket_path: "/tmp/new-plugin.sock".into(),
        },
        "m1",
    );
    let resp = svc.add_plugin(req).await.unwrap().into_inner();
    let plugin = resp.plugin.unwrap();
    assert_eq!(plugin.name, "new-plugin");
    assert_eq!(plugin.socket_path, "/tmp/new-plugin.sock");
}

#[tokio::test]
async fn remove_plugin_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(&router, "m1", rx, RemovePluginResponse { removed: true });
    let req = make_request(
        RemovePluginRequest {
            name: "old-plugin".into(),
        },
        "m1",
    );
    let resp = svc.remove_plugin(req).await.unwrap().into_inner();
    assert!(resp.removed);
}

#[tokio::test]
async fn enable_plugin_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        EnablePluginResponse {
            plugin: Some(PluginInfo {
                name: "disabled-plugin".into(),
                status: "running".into(),
                enabled: true,
                socket_path: "/tmp/disabled-plugin.sock".into(),
                command_count: 1,
                health_message: None,
                healthy: Some(true),
            }),
        },
    );
    let req = make_request(
        EnablePluginRequest {
            name: "disabled-plugin".into(),
        },
        "m1",
    );
    let resp = svc.enable_plugin(req).await.unwrap().into_inner();
    let plugin = resp.plugin.unwrap();
    assert_eq!(plugin.name, "disabled-plugin");
    assert!(plugin.enabled);
}

#[tokio::test]
async fn disable_plugin_routes_to_machine() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_responder(
        &router,
        "m1",
        rx,
        DisablePluginResponse {
            plugin: Some(PluginInfo {
                name: "active-plugin".into(),
                status: "stopped".into(),
                enabled: false,
                socket_path: "/tmp/active-plugin.sock".into(),
                command_count: 5,
                health_message: None,
                healthy: Some(false),
            }),
        },
    );
    let req = make_request(
        DisablePluginRequest {
            name: "active-plugin".into(),
        },
        "m1",
    );
    let resp = svc.disable_plugin(req).await.unwrap().into_inner();
    let plugin = resp.plugin.unwrap();
    assert_eq!(plugin.name, "active-plugin");
    assert!(!plugin.enabled);
}

// --- Error handling ---

#[tokio::test]
async fn machine_offline_returns_unavailable() {
    let svc = setup_offline().await;
    assert_offline_error!(svc, get_command_registry, GetCommandRegistryRequest {});
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_machine_error!(svc, get_command_registry, GetCommandRegistryRequest {});
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_no_claims_error!(svc, get_command_registry, GetCommandRegistryRequest {});
}

#[tokio::test]
async fn wrong_owner_returns_permission_denied() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    assert_wrong_owner_error!(svc, get_command_registry, GetCommandRegistryRequest {});
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    assert_daemon_error!(
        svc,
        get_command_registry,
        GetCommandRegistryRequest {},
        router,
        rx,
        "daemon error"
    );
}

// --- I-2: ExecuteServiceCommand streaming test (relay side) ---

#[tokio::test]
async fn execute_service_command_streams_output() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    let outputs: Vec<ServiceCommandOutput> = ["first line", "second line"]
        .iter()
        .map(|line| ServiceCommandOutput {
            output: Some(
                betcode_proto::v1::service_command_output::Output::StdoutLine((*line).into()),
            ),
        })
        .collect();
    spawn_stream_responder(&router, "m1", rx, outputs);

    let req = make_request(
        ExecuteServiceCommandRequest {
            command: "test-cmd".into(),
            args: vec![],
        },
        "m1",
    );
    let resp = svc.execute_service_command(req).await.unwrap();
    let mut stream = resp.into_inner();
    let mut outputs = vec![];
    while let Some(result) = stream.next().await {
        outputs.push(result.unwrap());
    }
    assert_eq!(outputs.len(), 2);
    assert_eq!(
        outputs[0].output,
        Some(betcode_proto::v1::service_command_output::Output::StdoutLine("first line".into()))
    );
    assert_eq!(
        outputs[1].output,
        Some(betcode_proto::v1::service_command_output::Output::StdoutLine("second line".into()))
    );
}
