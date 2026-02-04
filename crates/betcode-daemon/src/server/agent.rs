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

use super::handler::handle_agent_request;
use crate::relay::SessionRelay;
use crate::session::SessionMultiplexer;
use crate::storage::Database;

/// AgentService implementation backed by SessionRelay.
pub struct AgentServiceImpl {
    db: Database,
    relay: Arc<SessionRelay>,
    multiplexer: Arc<SessionMultiplexer>,
}

impl AgentServiceImpl {
    /// Create a new AgentService.
    pub fn new(
        db: Database,
        relay: Arc<SessionRelay>,
        multiplexer: Arc<SessionMultiplexer>,
    ) -> Self {
        Self {
            db,
            relay,
            multiplexer,
        }
    }
}

type AgentEventStream = Pin<Box<dyn Stream<Item = Result<AgentEvent, Status>> + Send>>;

#[tonic::async_trait]
impl AgentService for AgentServiceImpl {
    type ConverseStream = AgentEventStream;
    type ResumeSessionStream = AgentEventStream;

    async fn converse(
        &self,
        request: Request<Streaming<AgentRequest>>,
    ) -> Result<Response<Self::ConverseStream>, Status> {
        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        let relay = Arc::clone(&self.relay);
        let multiplexer = Arc::clone(&self.multiplexer);
        let db = self.db.clone();

        tokio::spawn(async move {
            let client_id = uuid::Uuid::new_v4().to_string();
            let mut session_id: Option<String> = None;

            while let Some(result) = in_stream.next().await {
                match result {
                    Ok(req) => {
                        if let Err(e) = handle_agent_request(
                            &relay,
                            &multiplexer,
                            &db,
                            &tx,
                            &client_id,
                            &mut session_id,
                            req,
                        )
                        .await
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

            if let Some(ref sid) = session_id {
                multiplexer.unsubscribe(sid, &client_id).await;
            }
            info!(client_id, "Converse stream ended");
        });

        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(out_stream)))
    }

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
                message_count: 0,
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

    async fn resume_session(
        &self,
        request: Request<ResumeSessionRequest>,
    ) -> Result<Response<Self::ResumeSessionStream>, Status> {
        let req = request.into_inner();
        let messages = self
            .db
            .get_messages_from_sequence(&req.session_id, req.from_sequence as i64)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(128);

        tokio::spawn(async move {
            for msg in messages {
                match prost::Message::decode(msg.payload.as_bytes()) {
                    Ok(event) => {
                        if tx.send(Ok(event)).await.is_err() {
                            warn!("Resume stream receiver dropped");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(?e, "Failed to decode stored message");
                        let err_event = AgentEvent {
                            sequence: 0,
                            timestamp: None,
                            parent_tool_use_id: String::new(),
                            event: Some(betcode_proto::v1::agent_event::Event::Error(
                                betcode_proto::v1::ErrorEvent {
                                    code: "DECODE_ERROR".to_string(),
                                    message: format!("Decode error: {}", e),
                                    is_fatal: false,
                                    details: Default::default(),
                                },
                            )),
                        };
                        if tx.send(Ok(err_event)).await.is_err() {
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

    async fn compact_session(
        &self,
        _request: Request<CompactSessionRequest>,
    ) -> Result<Response<CompactSessionResponse>, Status> {
        Ok(Response::new(CompactSessionResponse {
            messages_before: 0,
            messages_after: 0,
            tokens_saved: 0,
        }))
    }

    async fn cancel_turn(
        &self,
        request: Request<CancelTurnRequest>,
    ) -> Result<Response<CancelTurnResponse>, Status> {
        let req = request.into_inner();
        let was_active = self
            .relay
            .cancel_session(&req.session_id)
            .await
            .unwrap_or(false);
        Ok(Response::new(CancelTurnResponse { was_active }))
    }

    async fn request_input_lock(
        &self,
        _request: Request<InputLockRequest>,
    ) -> Result<Response<InputLockResponse>, Status> {
        // Input lock is managed per-client within the Converse stream.
        // This unary RPC currently grants by default since the proto
        // doesn't carry a client_id for identification.
        Ok(Response::new(InputLockResponse {
            granted: true,
            previous_holder: String::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subprocess::SubprocessManager;

    #[tokio::test]
    async fn agent_service_creation() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = Arc::new(SessionRelay::new(
            subprocess_mgr,
            Arc::clone(&multiplexer),
            db.clone(),
        ));
        let _service = AgentServiceImpl::new(db, relay, multiplexer);
    }
}
