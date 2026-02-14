//! Permission prompt and edit input handling.

use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::app::{App, AppMode, json_to_struct};
use betcode_proto::v1::{AgentRequest, PermissionDecision, PermissionResponse, UserMessage};

/// Handle a key press during the permission prompt (Y/A/Tab/E/N/X).
pub async fn handle_permission_key(app: &mut App, tx: &mpsc::Sender<AgentRequest>, code: KeyCode) {
    match code {
        KeyCode::Char('y' | 'Y' | '1') => {
            send_permission(app, tx, PermissionDecision::AllowOnce, None, String::new()).await;
        }
        KeyCode::Char('a' | 'A' | '2') => {
            send_permission(
                app,
                tx,
                PermissionDecision::AllowSession,
                None,
                String::new(),
            )
            .await;
        }
        KeyCode::Tab | KeyCode::Char('3') => {
            app.start_permission_edit();
        }
        KeyCode::Char('e' | 'E' | '4') => {
            app.start_permission_text(AppMode::PermissionComment, false);
        }
        KeyCode::Char('n' | 'N' | '5') => {
            app.start_permission_text(AppMode::PermissionDenyMessage, false);
        }
        KeyCode::Char('x' | 'X' | '6') => {
            app.start_permission_text(AppMode::PermissionDenyMessage, true);
        }
        KeyCode::Esc => {
            send_permission(app, tx, PermissionDecision::Deny, None, String::new()).await;
        }
        _ => {}
    }
}

/// Handle a key press during permission edit/comment/deny text input.
pub async fn handle_permission_edit_key(
    app: &mut App,
    tx: &mpsc::Sender<AgentRequest>,
    code: KeyCode,
) {
    match code {
        KeyCode::Enter => submit_permission_edit(app, tx).await,
        KeyCode::Esc => {
            app.mode = AppMode::PermissionPrompt;
        }
        KeyCode::Char(c) => {
            if let Some(ref mut perm) = app.pending_permission {
                perm.edit_buffer.insert(perm.edit_cursor, c);
                perm.edit_cursor += c.len_utf8();
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut perm) = app.pending_permission
                && perm.edit_cursor > 0
            {
                let prev = perm.edit_buffer[..perm.edit_cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
                perm.edit_buffer.remove(prev);
                perm.edit_cursor = prev;
            }
        }
        KeyCode::Left => {
            if let Some(ref mut perm) = app.pending_permission
                && perm.edit_cursor > 0
            {
                perm.edit_cursor = perm.edit_buffer[..perm.edit_cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
            }
        }
        KeyCode::Right => {
            if let Some(ref mut perm) = app.pending_permission
                && perm.edit_cursor < perm.edit_buffer.len()
            {
                perm.edit_cursor = perm.edit_buffer[perm.edit_cursor..]
                    .char_indices()
                    .nth(1)
                    .map_or(perm.edit_buffer.len(), |(i, _)| perm.edit_cursor + i);
            }
        }
        _ => {}
    }
}

/// Submit the current edit buffer based on the active permission edit mode.
async fn submit_permission_edit(app: &mut App, tx: &mpsc::Sender<AgentRequest>) {
    match app.mode {
        AppMode::PermissionEditInput => {
            let updated = app
                .pending_permission
                .as_ref()
                .and_then(|p| serde_json::from_str::<serde_json::Value>(&p.edit_buffer).ok())
                .map(|v| json_to_struct(&v));
            send_permission(
                app,
                tx,
                PermissionDecision::AllowWithEdit,
                updated,
                String::new(),
            )
            .await;
        }
        AppMode::PermissionComment => {
            let comment = app
                .pending_permission
                .as_ref()
                .map(|p| p.edit_buffer.clone())
                .unwrap_or_default();
            send_permission(app, tx, PermissionDecision::AllowOnce, None, String::new()).await;
            if !comment.is_empty() {
                let _ = tx
                    .send(AgentRequest {
                        request: Some(betcode_proto::v1::agent_request::Request::Message(
                            UserMessage {
                                content: comment,
                                attachments: Vec::new(),
                                agent_id: String::new(),
                            },
                        )),
                    })
                    .await;
            }
        }
        AppMode::PermissionDenyMessage => {
            let message = app
                .pending_permission
                .as_ref()
                .map(|p| p.edit_buffer.clone())
                .unwrap_or_default();
            let interrupt = app
                .pending_permission
                .as_ref()
                .is_none_or(|p| p.deny_interrupt);
            let decision = if interrupt {
                PermissionDecision::DenyWithInterrupt
            } else {
                PermissionDecision::DenyNoInterrupt
            };
            send_permission(app, tx, decision, None, message).await;
        }
        _ => {}
    }
}

/// Send a permission response and clean up state.
pub async fn send_permission(
    app: &mut App,
    tx: &mpsc::Sender<AgentRequest>,
    decision: PermissionDecision,
    updated_input: Option<betcode_proto::prost_types::Struct>,
    message: String,
) {
    if let Some(ref perm) = app.pending_permission {
        let _ = tx
            .send(AgentRequest {
                request: Some(betcode_proto::v1::agent_request::Request::Permission(
                    PermissionResponse {
                        request_id: perm.request_id.clone(),
                        decision: decision.into(),
                        updated_input,
                        message,
                    },
                )),
            })
            .await;
    }
    app.pending_permission = None;
    app.mode = AppMode::Normal;
}
