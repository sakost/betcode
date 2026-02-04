//! AgentService gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{error, info, warn};

use betcode_proto::v1::{
    agent_service_server::AgentService, AgentEvent, AgentRequest, CancelTurnRequest,
    CancelTurnResponse, CompactSessionRequest, CompactSessionResponse, InputLockRequest,
    InputLockResponse, ListSessionsRequest, ListSessionsResponse, ResumeSessionRequest,
    SessionSummary,
};

use crate::storage::Database;
use crate::subprocess::SubprocessManager;

/// AgentService implementation.
pub struct AgentServiceImpl {
    db: Database,
    subprocess_manager: Arc<SubprocessManager>,
}

impl AgentServiceImpl {
    /// Create a new AgentService.
    pub fn new(db: Database, subprocess_manager: SubprocessManager) -> Self {
        Self {
            db,
            subprocess_manager: Arc::new(subprocess_manager),
        }
    }
}

type AgentEventStream = Pin<Box<dyn Stream<Item = Result<AgentEvent, Status>> + Send>>;

#[tonic::async_trait]
impl AgentService for AgentServiceImpl {
    type ConverseStream = AgentEventStream;
    type ResumeSessionStream = AgentEventStream;

    /// Bidirectional streaming conversation with Claude.
    async fn converse(
        &self,
        request: Request<Streaming<AgentRequest>>,
    ) -> Result<Response<Self::ConverseStream>, Status> {
        let mut in_stream = request.into_inner();

        // Channel for outgoing events
        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        let db = self.db.clone();
        let subprocess_manager = Arc::clone(&self.subprocess_manager);

        // Spawn task to handle the conversation
        tokio::spawn(async move {
            while let Some(result) = in_stream.next().await {
                match result {
                    Ok(agent_request) => {
                        // Handle request based on type
                        if let Err(e) =
                            handle_agent_request(&db, &subprocess_manager, &tx, agent_request).await
                        {
                            error!(?e, "Error handling agent request");
                            let _ = tx.send(Err(Status::internal(e.to_string()))).await;
                            break;
                        }
                    }
                    Err(e) => {
                        error!(?e, "Stream error");
                        break;
                    }
                }
            }
            info!("Converse stream ended");
        });

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

    /// List all sessions.
    async fn list_sessions(
        &self,
        request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let req = request.into_inner();

        let working_dir = if req.working_directory.is_empty() {
            None
        } else {
            Some(req.working_directory.as_str())
        };

        let limit = if req.limit == 0 { 50 } else { req.limit };

        let sessions = self
            .db
            .list_sessions(working_dir, limit, req.offset)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let summaries: Vec<SessionSummary> = sessions
            .into_iter()
            .map(|s| SessionSummary {
                id: s.id,
                model: s.model,
                working_directory: s.working_directory,
                worktree_id: s.worktree_id.unwrap_or_default(),
                status: s.status,
                message_count: 0, // TODO: count from messages table
                total_input_tokens: s.total_input_tokens as u32,
                total_output_tokens: s.total_output_tokens as u32,
                total_cost_usd: s.total_cost_usd,
                created_at: Some(prost_types::Timestamp {
                    seconds: s.created_at,
                    nanos: 0,
                }),
                updated_at: Some(prost_types::Timestamp {
                    seconds: s.updated_at,
                    nanos: 0,
                }),
                last_message_preview: s.last_message_preview.unwrap_or_default(),
            })
            .collect();

        let total = summaries.len() as u32;

        Ok(Response::new(ListSessionsResponse {
            sessions: summaries,
            total,
        }))
    }

    /// Resume a session and replay events from a sequence number.
    async fn resume_session(
        &self,
        request: Request<ResumeSessionRequest>,
    ) -> Result<Response<Self::ResumeSessionStream>, Status> {
        let req = request.into_inner();

        // Get messages from the database starting at sequence
        let messages = self
            .db
            .get_messages_from_sequence(&req.session_id, req.from_sequence as i64)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        // Replay stored messages as events
        // Note: Messages are stored as prost-encoded bytes, not JSON
        tokio::spawn(async move {
            for msg in messages {
                // Decode stored payload back to AgentEvent using prost
                match prost::Message::decode(msg.payload.as_bytes()) {
                    Ok(event) => {
                        if tx.send(Ok(event)).await.is_err() {
                            warn!("Resume stream receiver dropped");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(?e, "Failed to decode stored message");
                        let error_event = AgentEvent {
                            sequence: 0,
                            timestamp: None,
                            parent_tool_use_id: String::new(),
                            event: Some(betcode_proto::v1::agent_event::Event::Error(
                                betcode_proto::v1::ErrorEvent {
                                    code: "DECODE_ERROR".to_string(),
                                    message: format!("Failed to decode stored message: {}", e),
                                    is_fatal: false,
                                    details: Default::default(),
                                },
                            )),
                        };
                        if tx.send(Ok(error_event)).await.is_err() {
                            break;
                        }
                    }
                }
            }
            info!(session_id = %req.session_id, "Resume replay completed");
        });

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

    /// Trigger context compaction for a session.
    async fn compact_session(
        &self,
        _request: Request<CompactSessionRequest>,
    ) -> Result<Response<CompactSessionResponse>, Status> {
        // TODO: Implement compaction - send /compact command to Claude
        Ok(Response::new(CompactSessionResponse {
            messages_before: 0,
            messages_after: 0,
            tokens_saved: 0,
        }))
    }

    /// Cancel the current turn in a session.
    async fn cancel_turn(
        &self,
        request: Request<CancelTurnRequest>,
    ) -> Result<Response<CancelTurnResponse>, Status> {
        let req = request.into_inner();

        // Try to terminate the subprocess for this session
        let was_active = self
            .subprocess_manager
            .terminate(&req.session_id)
            .await
            .is_ok();

        Ok(Response::new(CancelTurnResponse { was_active }))
    }

    /// Request exclusive input lock for a session.
    async fn request_input_lock(
        &self,
        _request: Request<InputLockRequest>,
    ) -> Result<Response<InputLockResponse>, Status> {
        // TODO: Implement input lock management
        Ok(Response::new(InputLockResponse {
            granted: true,
            previous_holder: String::new(),
        }))
    }
}

/// Handle a single agent request.
async fn handle_agent_request(
    _db: &Database,
    _subprocess_manager: &SubprocessManager,
    tx: &mpsc::Sender<Result<AgentEvent, Status>>,
    request: AgentRequest,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use betcode_proto::v1::agent_request::Request;

    match request.request {
        Some(Request::Start(start)) => {
            info!(
                session_id = %start.session_id,
                model = %start.model,
                "Starting conversation"
            );

            // TODO: Full implementation in Session Multiplexer task
            // 1. Create or get session
            // 2. Spawn subprocess if needed
            // 3. Set up event forwarding

            // For now, send a session info event
            let event = AgentEvent {
                sequence: 1,
                timestamp: Some(prost_types::Timestamp {
                    seconds: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                    nanos: 0,
                }),
                parent_tool_use_id: String::new(),
                event: Some(betcode_proto::v1::agent_event::Event::SessionInfo(
                    betcode_proto::v1::SessionInfo {
                        session_id: start.session_id,
                        model: start.model,
                        working_directory: start.working_directory,
                        worktree_id: start.worktree_id,
                        message_count: 0,
                        is_resumed: false,
                        is_compacted: false,
                        context_usage_percent: 0.0,
                    },
                )),
            };

            tx.send(Ok(event)).await?;
        }
        Some(Request::Message(msg)) => {
            info!(content_len = msg.content.len(), "Received user message");
            // TODO: Forward to subprocess via stdin
        }
        Some(Request::Permission(perm)) => {
            info!(request_id = %perm.request_id, "Received permission response");
            // TODO: Forward to subprocess
        }
        Some(Request::QuestionResponse(qr)) => {
            info!(question_id = %qr.question_id, "Received question response");
            // TODO: Forward to subprocess
        }
        Some(Request::Cancel(cancel)) => {
            info!(reason = %cancel.reason, "Received cancel request");
            // TODO: Cancel current operation
        }
        None => {
            warn!("Received empty request");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn agent_service_creation() {
        let db = Database::open_in_memory().await.unwrap();
        let manager = SubprocessManager::new(5);
        let _service = AgentServiceImpl::new(db, manager);
    }
}
