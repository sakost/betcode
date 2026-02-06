//! Headless (non-interactive) mode.
//!
//! Sends a prompt to the daemon and streams the response to stdout.

use tokio::sync::mpsc;
use tracing::{error, info};

use betcode_proto::v1::agent_event::Event;
use betcode_proto::v1::{
    AgentEvent, AgentRequest, PermissionDecision, PermissionResponse, StartConversation,
    UserMessage,
};

use crate::connection::{ConnectionError, DaemonConnection};

/// Headless mode configuration.
#[derive(Debug, Clone)]
pub struct HeadlessConfig {
    /// Prompt to send.
    pub prompt: String,
    /// Session ID (creates new if None).
    pub session_id: Option<String>,
    /// Working directory.
    pub working_directory: String,
    /// Model to use.
    pub model: Option<String>,
    /// Auto-accept all permissions.
    pub auto_accept: bool,
}

/// Run headless mode.
pub async fn run(conn: &mut DaemonConnection, config: HeadlessConfig) -> Result<(), HeadlessError> {
    // Generate session ID if not provided
    let session_id = config
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Load and display history if continuing an existing session
    match conn.resume_session(&session_id, 0).await {
        Ok(events) if !events.is_empty() => {
            eprintln!("[Resuming session {} ({} events)]", &session_id[..8.min(session_id.len())], events.len());
            for event in &events {
                print_history_event(event);
            }
            eprintln!("[--- End of history ---]");
        }
        Ok(_) => {} // No history, new session
        Err(e) => {
            // Non-fatal: session may be new
            eprintln!("[Warning: could not load history: {}]", e);
        }
    }

    let (request_tx, mut event_rx, stream_handle) =
        conn.converse().await.map_err(HeadlessError::Connection)?;

    // Send start conversation
    request_tx
        .send(AgentRequest {
            request: Some(betcode_proto::v1::agent_request::Request::Start(
                StartConversation {
                    session_id: session_id.clone(),
                    working_directory: config.working_directory,
                    model: config.model.unwrap_or_default(),
                    allowed_tools: Vec::new(),
                    plan_mode: false,
                    worktree_id: String::new(),
                    metadata: Default::default(),
                },
            )),
        })
        .await
        .map_err(|_| HeadlessError::StreamClosed)?;

    // Send user message
    request_tx
        .send(AgentRequest {
            request: Some(betcode_proto::v1::agent_request::Request::Message(
                UserMessage {
                    content: config.prompt,
                    attachments: Vec::new(),
                },
            )),
        })
        .await
        .map_err(|_| HeadlessError::StreamClosed)?;

    info!(session_id, "Headless mode started");

    // Process events until turn complete
    let result: Result<(), HeadlessError> = async {
        while let Some(result) = event_rx.recv().await {
            match result {
                Ok(event) => {
                    let done =
                        process_headless_event(event, &request_tx, config.auto_accept).await?;
                    if done {
                        break;
                    }
                }
                Err(e) => {
                    error!(?e, "Stream error");
                    return Err(HeadlessError::StreamError(e.to_string()));
                }
            }
        }
        Ok(())
    }
    .await;

    stream_handle.abort();
    result
}

/// Process a single event in headless mode. Returns true if done.
async fn process_headless_event(
    event: AgentEvent,
    request_tx: &mpsc::Sender<AgentRequest>,
    auto_accept: bool,
) -> Result<bool, HeadlessError> {
    match event.event {
        Some(Event::TextDelta(delta)) => {
            print!("{}", delta.text);
        }
        Some(Event::ToolCallStart(tool)) => {
            eprintln!("[Tool: {} - {}]", tool.tool_name, tool.description);
        }
        Some(Event::ToolCallResult(result)) => {
            if result.is_error {
                eprintln!("[Tool Error: {}]", result.output);
            }
        }
        Some(Event::PermissionRequest(perm)) => {
            if auto_accept {
                eprintln!(
                    "[Auto-accepting: {} - {}]",
                    perm.tool_name, perm.description
                );
                request_tx
                    .send(AgentRequest {
                        request: Some(betcode_proto::v1::agent_request::Request::Permission(
                            PermissionResponse {
                                request_id: perm.request_id,
                                decision: PermissionDecision::AllowOnce.into(),
                            },
                        )),
                    })
                    .await
                    .map_err(|_| HeadlessError::StreamClosed)?;
            } else {
                eprintln!(
                    "[Permission denied (use --yes to auto-accept): {} - {}]",
                    perm.tool_name, perm.description
                );
                request_tx
                    .send(AgentRequest {
                        request: Some(betcode_proto::v1::agent_request::Request::Permission(
                            PermissionResponse {
                                request_id: perm.request_id,
                                decision: PermissionDecision::Deny.into(),
                            },
                        )),
                    })
                    .await
                    .map_err(|_| HeadlessError::StreamClosed)?;
            }
        }
        Some(Event::Error(err)) => {
            eprintln!("[Error: {} - {}]", err.code, err.message);
            if err.is_fatal {
                return Err(HeadlessError::FatalError(err.message));
            }
        }
        Some(Event::TurnComplete(_)) => {
            // Final newline after streamed text output
            use std::io::Write;
            let _ = std::io::stdout().flush();
            eprintln!();
            return Ok(true);
        }
        Some(Event::SessionInfo(info)) => {
            info!(session_id = %info.session_id, model = %info.model, "Session started");
        }
        Some(Event::Usage(usage)) => {
            eprintln!(
                "[Tokens: {}in/{}out | ${:.4}]",
                usage.input_tokens, usage.output_tokens, usage.cost_usd
            );
        }
        _ => {}
    }

    Ok(false)
}

/// Print a historical event to stderr for headless context display.
fn print_history_event(event: &AgentEvent) {
    match &event.event {
        Some(Event::TextDelta(delta)) if !delta.text.is_empty() => {
            eprint!("{}", delta.text);
            if delta.is_complete {
                eprintln!();
            }
        }
        Some(Event::ToolCallStart(tool)) => {
            if tool.description.is_empty() {
                eprintln!("[Tool: {}]", tool.tool_name);
            } else {
                eprintln!("[Tool: {} - {}]", tool.tool_name, tool.description);
            }
        }
        Some(Event::ToolCallResult(result)) if result.is_error => {
            eprintln!("[Tool Error: {}]", result.output);
        }
        Some(Event::Error(err)) => {
            eprintln!("[Error: {} - {}]", err.code, err.message);
        }
        _ => {}
    }
}

/// Headless mode errors.
#[derive(Debug, thiserror::Error)]
pub enum HeadlessError {
    #[error("Connection error: {0}")]
    Connection(#[from] ConnectionError),

    #[error("Stream closed unexpectedly")]
    StreamClosed,

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("Fatal error: {0}")]
    FatalError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_config_defaults() {
        let config = HeadlessConfig {
            prompt: "hello".to_string(),
            session_id: None,
            working_directory: "/tmp".to_string(),
            model: None,
            auto_accept: false,
        };
        assert!(!config.auto_accept);
        assert!(config.session_id.is_none());
    }
}
