//! Agent request handler - routes gRPC requests to the relay.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tonic::Status;
use tracing::{info, warn};

use betcode_proto::v1::{AgentEvent, AgentRequest, PermissionDecision};

use crate::relay::{RelaySessionConfig, SessionRelay};
use crate::session::SessionMultiplexer;
use crate::storage::Database;

/// Shared context for agent request handling.
pub struct HandlerContext<'a> {
    pub relay: &'a SessionRelay,
    pub multiplexer: &'a SessionMultiplexer,
    pub db: &'a Database,
    pub tx: &'a mpsc::Sender<Result<AgentEvent, Status>>,
    pub client_id: &'a str,
}

/// Handle a single agent request using the relay.
pub async fn handle_agent_request(
    ctx: &HandlerContext<'_>,
    session_id: &mut Option<String>,
    request: AgentRequest,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use betcode_proto::v1::agent_request::Request;

    match request.request {
        Some(Request::Start(start)) => {
            handle_start(ctx, session_id, start).await?;
        }
        Some(Request::Message(msg)) => {
            if let Some(ref sid) = session_id {
                info!(session_id = %sid, content_len = msg.content.len(), "User message");
                ctx.relay
                    .send_user_message(sid, &msg.content)
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                warn!("Received message before session start");
            }
        }
        Some(Request::Permission(perm)) => {
            if let Some(ref sid) = session_id {
                let granted = perm.decision == PermissionDecision::AllowOnce as i32
                    || perm.decision == PermissionDecision::AllowSession as i32;
                info!(session_id = %sid, request_id = %perm.request_id, granted, "Permission");

                // Look up original tool input from pending map.
                let original_input = if let Some(handle) = ctx.relay.get_handle(sid).await {
                    handle
                        .pending_permission_inputs
                        .write()
                        .await
                        .remove(&perm.request_id)
                        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::default()))
                } else {
                    serde_json::Value::Object(serde_json::Map::default())
                };

                ctx.relay
                    .send_permission_response(sid, &perm.request_id, granted, &original_input)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Some(Request::QuestionResponse(qr)) => {
            if let Some(ref sid) = session_id {
                info!(session_id = %sid, question_id = %qr.question_id, "Question response");
                let answers: HashMap<String, String> = qr.answers.into_iter().collect();

                // Look up the original AskUserQuestion input from the pending map.
                let original_input = if let Some(handle) = ctx.relay.get_handle(sid).await {
                    handle
                        .pending_question_inputs
                        .write()
                        .await
                        .remove(&qr.question_id)
                        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::default()))
                } else {
                    serde_json::Value::Object(serde_json::Map::default())
                };

                ctx.relay
                    .send_question_response(sid, &qr.question_id, &answers, &original_input)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                        e.to_string().into()
                    })?;
            }
        }
        Some(Request::Cancel(cancel)) => {
            if let Some(ref sid) = session_id {
                info!(session_id = %sid, reason = %cancel.reason, "Cancel request");
                ctx.relay
                    .cancel_session(sid)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Some(Request::Encrypted(_)) => {
            // EncryptedEnvelope is handled at the tunnel layer (application-layer E2E).
            // In the local gRPC path, encrypted requests should never arrive.
            warn!("Received encrypted request on local gRPC — ignoring");
        }
        None => {
            warn!("Received empty request");
        }
    }

    Ok(())
}

/// Handle a `StartConversation` request.
async fn handle_start(
    ctx: &HandlerContext<'_>,
    session_id: &mut Option<String>,
    start: betcode_proto::v1::StartConversation,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        session_id = %start.session_id,
        model = %start.model,
        "Starting conversation"
    );

    let sid = start.session_id.clone();
    let model = if start.model.is_empty() {
        "claude-sonnet-4".to_string()
    } else {
        start.model.clone()
    };
    let working_dir: std::path::PathBuf = start.working_directory.clone().into();

    // Check if session already exists in DB; create if new
    let resume_session = if let Ok(existing) = ctx.db.get_session(&sid).await {
        info!(session_id = %sid, "Resuming existing session");
        existing.claude_session_id.filter(|s| !s.is_empty())
    } else {
        // New session - create in DB
        ctx.db
            .create_session(&sid, &model, &start.working_directory)
            .await
            .map_err(|e| e.to_string())?;
        info!(session_id = %sid, "Created new session in database");
        None
    };

    // Mark session as active
    ctx.db
        .update_session_status(&sid, crate::storage::SessionStatus::Active)
        .await
        .map_err(|e| e.to_string())?;

    // Subscribe this client to the session's broadcast channel
    let handle = ctx
        .multiplexer
        .subscribe(&sid, ctx.client_id, "grpc")
        .await
        .map_err(|e| e.to_string())?;

    *session_id = Some(sid.clone());

    // Start the relay (spawns subprocess + event pipeline)
    let config = RelaySessionConfig {
        session_id: sid.clone(),
        working_directory: working_dir,
        model: Some(model),
        resume_session,
        worktree_id: start.worktree_id,
    };

    ctx.relay
        .start_session(config)
        .await
        .map_err(|e| e.to_string())?;

    // Forward broadcast events to this client's gRPC stream
    let tx_clone = ctx.tx.clone();
    let mut event_rx = handle.event_rx;
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            if tx_clone.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    Ok(())
}
