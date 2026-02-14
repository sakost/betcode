//! `AgentService` gRPC implementation.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{error, info, instrument, warn};

use betcode_proto::v1::{
    agent_service_server::AgentService, AgentEvent, AgentRequest, CancelTurnRequest,
    CancelTurnResponse, ClearSessionGrantsRequest, ClearSessionGrantsResponse,
    CompactSessionRequest, CompactSessionResponse, InputLockRequest, InputLockResponse,
    KeyExchangeRequest, KeyExchangeResponse, ListSessionGrantsRequest, ListSessionGrantsResponse,
    ListSessionsRequest, ListSessionsResponse, RenameSessionRequest, RenameSessionResponse,
    ResumeSessionRequest, SessionGrantEntry, SessionSummary, SetSessionGrantRequest,
    SetSessionGrantResponse,
};

use super::handler::{handle_agent_request, HandlerContext};
use crate::relay::SessionRelay;
use crate::session::SessionMultiplexer;
use crate::storage::{Database, DatabaseError};

/// `AgentService` implementation backed by `SessionRelay`.
pub struct AgentServiceImpl {
    db: Database,
    relay: Arc<SessionRelay>,
    multiplexer: Arc<SessionMultiplexer>,
}

impl AgentServiceImpl {
    /// Create a new `AgentService`.
    pub const fn new(
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
                        let handler_ctx = HandlerContext {
                            relay: &relay,
                            multiplexer: &multiplexer,
                            db: &db,
                            tx: &tx,
                            client_id: &client_id,
                        };
                        if let Err(e) =
                            handle_agent_request(&handler_ctx, &mut session_id, req).await
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
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                total_input_tokens: s.total_input_tokens as u32,
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
                name: s.name,
            })
            .collect();

        #[allow(clippy::cast_possible_truncation)]
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
        #[allow(clippy::cast_possible_wrap)]
        let from_seq = req.from_sequence as i64;
        let messages = self
            .db
            .get_messages_from_sequence(&req.session_id, from_seq)
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
                                    message: format!("Decode error: {e}"),
                                    is_fatal: false,
                                    details: HashMap::default(),
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

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
        let cutoff = max_seq - i64::from(keep_count);

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

        #[allow(clippy::cast_possible_truncation)]
        let messages_after = messages_before - deleted as u32;
        // Rough estimate: ~100 tokens per message on average
        #[allow(clippy::cast_possible_truncation)]
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

    #[instrument(skip(self, _request), fields(rpc = "ExchangeKeys"))]
    async fn exchange_keys(
        &self,
        _request: Request<KeyExchangeRequest>,
    ) -> Result<Response<KeyExchangeResponse>, Status> {
        // Key exchange is only meaningful for remote (tunneled) connections.
        // Direct local gRPC connections don't need E2E encryption.
        Err(Status::unimplemented(
            "ExchangeKeys is only supported over tunnel connections",
        ))
    }

    #[instrument(skip(self, request), fields(rpc = "ListSessionGrants"))]
    #[allow(clippy::significant_drop_tightening)]
    async fn list_session_grants(
        &self,
        request: Request<ListSessionGrantsRequest>,
    ) -> Result<Response<ListSessionGrantsResponse>, Status> {
        let req = request.into_inner();
        let handle = self
            .relay
            .get_handle(&req.session_id)
            .await
            .ok_or_else(|| Status::not_found(format!("Session {} not active", req.session_id)))?;

        let grants = handle.session_grants.read().await;
        let entries: Vec<SessionGrantEntry> = grants
            .iter()
            .map(|(tool_name, granted)| SessionGrantEntry {
                tool_name: tool_name.clone(),
                granted: *granted,
            })
            .collect();
        drop(grants);

        Ok(Response::new(ListSessionGrantsResponse { grants: entries }))
    }

    #[instrument(skip(self, request), fields(rpc = "ClearSessionGrants"))]
    #[allow(clippy::significant_drop_tightening)]
    async fn clear_session_grants(
        &self,
        request: Request<ClearSessionGrantsRequest>,
    ) -> Result<Response<ClearSessionGrantsResponse>, Status> {
        let req = request.into_inner();
        let handle = self
            .relay
            .get_handle(&req.session_id)
            .await
            .ok_or_else(|| Status::not_found(format!("Session {} not active", req.session_id)))?;

        let mut grants = handle.session_grants.write().await;
        if req.tool_name.is_empty() {
            grants.clear();
            drop(grants);
            info!(session_id = %req.session_id, "Cleared all session grants");
        } else {
            grants.remove(&req.tool_name);
            drop(grants);
            info!(session_id = %req.session_id, tool_name = %req.tool_name, "Cleared session grant");
        }

        Ok(Response::new(ClearSessionGrantsResponse {}))
    }

    #[instrument(skip(self, request), fields(rpc = "SetSessionGrant"))]
    async fn set_session_grant(
        &self,
        request: Request<SetSessionGrantRequest>,
    ) -> Result<Response<SetSessionGrantResponse>, Status> {
        let req = request.into_inner();
        if req.tool_name.is_empty() {
            return Err(Status::invalid_argument("tool_name must not be empty"));
        }
        let handle = self
            .relay
            .get_handle(&req.session_id)
            .await
            .ok_or_else(|| Status::not_found(format!("Session {} not active", req.session_id)))?;

        handle
            .session_grants
            .write()
            .await
            .insert(req.tool_name.clone(), req.granted);

        info!(
            session_id = %req.session_id,
            tool_name = %req.tool_name,
            granted = req.granted,
            "Set session grant"
        );

        Ok(Response::new(SetSessionGrantResponse {}))
    }

    #[instrument(skip(self, request), fields(rpc = "RenameSession"))]
    async fn rename_session(
        &self,
        request: Request<RenameSessionRequest>,
    ) -> Result<Response<RenameSessionResponse>, Status> {
        let req = request.into_inner();
        self.db
            .update_session_name(&req.session_id, &req.name)
            .await
            .map_err(|e| match e {
                DatabaseError::NotFound(_) => Status::not_found(e.to_string()),
                _ => Status::internal(e.to_string()),
            })?;

        info!(session_id = %req.session_id, name = %req.name, "Session renamed");

        Ok(Response::new(RenameSessionResponse {}))
    }
}

/// Extract `client_id` from gRPC request metadata.
fn request_client_id<T>(request: &Request<T>) -> Option<String> {
    request
        .metadata()
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string)
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

    #[tokio::test]
    async fn exchange_keys_returns_unimplemented() {
        let db = Database::open_in_memory().await.unwrap();
        let subprocess_mgr = Arc::new(SubprocessManager::new(5));
        let multiplexer = Arc::new(SessionMultiplexer::with_defaults());
        let relay = Arc::new(SessionRelay::new(
            subprocess_mgr,
            Arc::clone(&multiplexer),
            db.clone(),
        ));
        let service = AgentServiceImpl::new(db, relay, multiplexer);

        let req = Request::new(KeyExchangeRequest {
            machine_id: "m1".into(),
            identity_pubkey: Vec::new(),
            fingerprint: String::new(),
            ephemeral_pubkey: vec![0u8; 32],
        });
        let err = service.exchange_keys(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
