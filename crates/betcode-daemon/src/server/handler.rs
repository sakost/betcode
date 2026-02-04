//! Agent request handler - routes gRPC requests to the relay.

use tokio::sync::mpsc;
use tonic::Status;
use tracing::{info, warn};

use betcode_proto::v1::{AgentEvent, AgentRequest, PermissionDecision};

use crate::relay::{RelaySessionConfig, SessionRelay};
use crate::session::SessionMultiplexer;
use crate::storage::Database;

/// Handle a single agent request using the relay.
#[allow(clippy::too_many_arguments)]
pub async fn handle_agent_request(
    relay: &SessionRelay,
    multiplexer: &SessionMultiplexer,
    _db: &Database,
    tx: &mpsc::Sender<Result<AgentEvent, Status>>,
    client_id: &str,
    session_id: &mut Option<String>,
    request: AgentRequest,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use betcode_proto::v1::agent_request::Request;

    match request.request {
        Some(Request::Start(start)) => {
            handle_start(relay, multiplexer, tx, client_id, session_id, start).await?;
        }
        Some(Request::Message(msg)) => {
            if let Some(ref sid) = session_id {
                info!(session_id = %sid, content_len = msg.content.len(), "User message");
                relay
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
                relay
                    .send_permission_response(sid, &perm.request_id, granted)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Some(Request::QuestionResponse(qr)) => {
            info!(question_id = %qr.question_id, "Question response (not yet routed)");
        }
        Some(Request::Cancel(cancel)) => {
            if let Some(ref sid) = session_id {
                info!(session_id = %sid, reason = %cancel.reason, "Cancel request");
                relay.cancel_session(sid).await.map_err(|e| e.to_string())?;
            }
        }
        None => {
            warn!("Received empty request");
        }
    }

    Ok(())
}

/// Handle a StartConversation request.
async fn handle_start(
    relay: &SessionRelay,
    multiplexer: &SessionMultiplexer,
    tx: &mpsc::Sender<Result<AgentEvent, Status>>,
    client_id: &str,
    session_id: &mut Option<String>,
    start: betcode_proto::v1::StartConversation,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        session_id = %start.session_id,
        model = %start.model,
        "Starting conversation"
    );

    let sid = start.session_id.clone();

    // Subscribe this client to the session's broadcast channel
    let handle = multiplexer
        .subscribe(&sid, client_id, "grpc")
        .await
        .map_err(|e| e.to_string())?;

    *session_id = Some(sid.clone());

    // Start the relay (spawns subprocess + event pipeline)
    let config = RelaySessionConfig {
        session_id: sid.clone(),
        working_directory: start.working_directory.into(),
        model: if start.model.is_empty() {
            None
        } else {
            Some(start.model)
        },
        resume_session: None,
        worktree_id: start.worktree_id,
    };

    relay
        .start_session(config)
        .await
        .map_err(|e| e.to_string())?;

    // Forward broadcast events to this client's gRPC stream
    let tx_clone = tx.clone();
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
