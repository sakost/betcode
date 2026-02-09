//! AgentService gRPC implementation.

use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{error, info, instrument, warn};

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

    #[instrument(skip(self, request), fields(rpc = "Converse"))]
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

    #[instrument(skip(self, request), fields(rpc = "ListSessions"))]
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

    #[instrument(skip(self, request), fields(rpc = "ResumeSession"))]
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
                // Payload is base64-encoded protobuf bytes
                let bytes = match base64_decode(&msg.payload) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!(error = %e, "Failed to decode base64 payload");
                        continue;
                    }
                };

                match prost::Message::decode(bytes.as_slice()) {
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

    #[instrument(skip(self, request), fields(rpc = "CompactSession"))]
    async fn compact_session(
        &self,
        request: Request<CompactSessionRequest>,
    ) -> Result<Response<CompactSessionResponse>, Status> {
        let req = request.into_inner();
        let sid = &req.session_id;

        let messages_before = self
            .db
            .count_messages(sid)
            .await
            .map_err(|e| Status::internal(e.to_string()))? as u32;

        if messages_before == 0 {
            return Ok(Response::new(CompactSessionResponse {
                messages_before: 0,
                messages_after: 0,
                tokens_saved: 0,
            }));
        }

        // Keep the most recent half of messages (at least 10)
        let max_seq = self
            .db
            .max_message_sequence(sid)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let keep_count = (messages_before / 2).max(10).min(messages_before);
        let cutoff = max_seq - keep_count as i64;

        if cutoff <= 0 {
            return Ok(Response::new(CompactSessionResponse {
                messages_before,
                messages_after: messages_before,
                tokens_saved: 0,
            }));
        }

        let deleted = self
            .db
            .delete_messages_before_sequence(sid, cutoff)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        self.db
            .update_compaction_sequence(sid, cutoff)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let messages_after = messages_before - deleted as u32;
        // Rough estimate: ~100 tokens per message on average
        let tokens_saved = deleted as u32 * 100;

        info!(
            session_id = %sid,
            messages_before,
            messages_after,
            tokens_saved,
            "Session compacted"
        );

        Ok(Response::new(CompactSessionResponse {
            messages_before,
            messages_after,
            tokens_saved,
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "CancelTurn"))]
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

    #[instrument(skip(self, request), fields(rpc = "RequestInputLock"))]
    async fn request_input_lock(
        &self,
        request: Request<InputLockRequest>,
    ) -> Result<Response<InputLockResponse>, Status> {
        // Extract client_id from metadata before consuming the request
        let client_id = request_client_id(&request)
            .unwrap_or_else(|| format!("unary-{}", uuid::Uuid::new_v4()));
        let req = request.into_inner();
        let sid = &req.session_id;

        let previous = self
            .db
            .acquire_input_lock(sid, &client_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        info!(
            session_id = %sid,
            client_id = %client_id,
            previous_holder = ?previous,
            "Input lock acquired"
        );

        Ok(Response::new(InputLockResponse {
            granted: true,
            previous_holder: previous.unwrap_or_default(),
        }))
    }
}

/// Extract client_id from gRPC request metadata.
fn request_client_id<T>(request: &Request<T>) -> Option<String> {
    request
        .metadata()
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

use betcode_core::db::base64_decode;

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
