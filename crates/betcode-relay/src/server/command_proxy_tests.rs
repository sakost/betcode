//! Tests for CommandProxyService.

use std::collections::HashMap;
use std::sync::Arc;

use tonic::Request;

use betcode_proto::v1::command_service_server::CommandService;
use betcode_proto::v1::{
    AddPluginRequest, AddPluginResponse, AgentInfo, CommandEntry, DisablePluginRequest,
    DisablePluginResponse, EnablePluginRequest, EnablePluginResponse, FrameType,
    GetCommandRegistryRequest, GetCommandRegistryResponse, GetPluginStatusRequest,
    GetPluginStatusResponse, ListAgentsRequest, ListAgentsResponse, ListPathRequest,
    ListPathResponse, ListPluginsRequest, ListPluginsResponse, PathEntry, PluginInfo,
    RemovePluginRequest, RemovePluginResponse, TunnelFrame,
};

use super::CommandProxyService;
use crate::server::test_helpers::{
    make_request, setup_offline_router, setup_router_with_machine, spawn_error_responder,
    spawn_responder, test_claims,
};

async fn setup_with_machine(
    mid: &str,
) -> (
    CommandProxyService,
    Arc<crate::router::RequestRouter>,
    tokio::sync::mpsc::Receiver<TunnelFrame>,
) {
    let (router, rx) = setup_router_with_machine(mid).await;
    (CommandProxyService::new(Arc::clone(&router)), router, rx)
}

async fn setup_offline() -> CommandProxyService {
    let router = setup_offline_router().await;
    CommandProxyService::new(router)
}

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
                category: 1, // COMMAND_CATEGORY_SERVICE
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
    spawn_responder(
        &router,
        "m1",
        rx,
        RemovePluginResponse { removed: true },
    );
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
    let req = make_request(GetCommandRegistryRequest {}, "m-off");
    let err = svc.get_command_registry(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}

#[tokio::test]
async fn missing_machine_id_returns_invalid_argument() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(GetCommandRegistryRequest {});
    req.extensions_mut().insert(test_claims());
    let err = svc.get_command_registry(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn missing_claims_returns_internal() {
    let (svc, _router, _rx) = setup_with_machine("m1").await;
    let mut req = Request::new(GetCommandRegistryRequest {});
    req.metadata_mut()
        .insert("x-machine-id", "m1".parse().unwrap());
    let err = svc.get_command_registry(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
}

#[tokio::test]
async fn daemon_error_propagated_to_client() {
    let (svc, router, rx) = setup_with_machine("m1").await;
    spawn_error_responder(&router, "m1", rx, "daemon error");
    let req = make_request(GetCommandRegistryRequest {}, "m1");
    let err = svc.get_command_registry(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(err.message().contains("daemon error"));
}

// --- I-2: ExecuteServiceCommand streaming test (relay side) ---

use betcode_proto::v1::{
    EncryptedPayload, ExecuteServiceCommandRequest, ServiceCommandOutput, StreamPayload,
};
use tokio_stream::StreamExt;

use crate::server::test_helpers::encode_msg;

#[tokio::test]
async fn execute_service_command_streams_output() {
    let (svc, router, mut tunnel_rx) = setup_with_machine("m1").await;
    let rc = Arc::clone(&router);
    tokio::spawn(async move {
        if let Some(frame) = tunnel_rx.recv().await {
            let rid = frame.request_id.clone();
            let conn = rc.registry().get("m1").await.unwrap();
            // Send two StreamData frames with ServiceCommandOutput payloads
            for (seq, line) in ["first line", "second line"].iter().enumerate() {
                let output = ServiceCommandOutput {
                    output: Some(
                        betcode_proto::v1::service_command_output::Output::StdoutLine(
                            (*line).into(),
                        ),
                    ),
                };
                let f = TunnelFrame {
                    request_id: rid.clone(),
                    frame_type: FrameType::StreamData as i32,
                    timestamp: None,
                    payload: Some(betcode_proto::v1::tunnel_frame::Payload::StreamData(
                        StreamPayload {
                            method: String::new(),
                            encrypted: Some(EncryptedPayload {
                                ciphertext: encode_msg(&output),
                                nonce: Vec::new(),
                                ephemeral_pubkey: Vec::new(),
                            }),
                            sequence: seq as u64,
                            metadata: HashMap::new(),
                        },
                    )),
                };
                conn.send_stream_frame(&rid, f).await;
            }
            // Close the stream
            conn.complete_stream(&rid).await;
        }
    });

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
