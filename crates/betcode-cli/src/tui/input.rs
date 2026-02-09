//! Input handling for TUI key events.

use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::app::{App, AppMode};
use betcode_proto::v1::AgentRequest;

use super::TermEvent;

/// Process a terminal event, updating app state and optionally sending gRPC requests.
pub async fn handle_term_event(app: &mut App, tx: &mpsc::Sender<AgentRequest>, event: TermEvent) {
    match event {
        TermEvent::Key(key) => {
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('c')
            {
                app.should_quit = true;
            } else {
                match app.mode {
                    AppMode::PermissionPrompt => {
                        super::permission_input::handle_permission_key(app, tx, key.code).await;
                    }
                    AppMode::PermissionEditInput
                    | AppMode::PermissionComment
                    | AppMode::PermissionDenyMessage => {
                        super::permission_input::handle_permission_edit_key(app, tx, key.code)
                            .await;
                    }
                    AppMode::UserQuestion => {
                        super::question_input::handle_question_key(app, tx, key.code).await;
                    }
                    AppMode::Normal | AppMode::SessionList => {
                        handle_input_key(app, tx, key).await;
                    }
                }
            }
        }
        TermEvent::Resize(_, _) => { /* terminal auto-handles resize on next draw */ }
    }
}

/// Handle a key press in normal input mode.
async fn handle_input_key(
    app: &mut App,
    tx: &mpsc::Sender<AgentRequest>,
    key: crossterm::event::KeyEvent,
) {
    let shift = key
        .modifiers
        .contains(crossterm::event::KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Enter => {
            if let Some(text) = app.submit_input() {
                let _ = tx
                    .send(AgentRequest {
                        request: Some(betcode_proto::v1::agent_request::Request::Message(
                            betcode_proto::v1::UserMessage {
                                content: text,
                                attachments: Vec::new(),
                            },
                        )),
                    })
                    .await;
                app.agent_busy = true;
                app.scroll_to_bottom();
            }
        }
        KeyCode::Char(c) => {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += 1;
        }
        KeyCode::Backspace => {
            if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
                app.input.remove(app.cursor_pos);
            }
        }
        KeyCode::Left => {
            app.cursor_pos = app.cursor_pos.saturating_sub(1);
        }
        KeyCode::Right => {
            app.cursor_pos = (app.cursor_pos + 1).min(app.input.len());
        }
        KeyCode::Up if shift => app.scroll_up(1),
        KeyCode::Down if shift => app.scroll_down(1),
        KeyCode::Up => app.history_up(),
        KeyCode::Down => app.history_down(),
        KeyCode::PageUp => app.scroll_up(app.viewport_height.max(1)),
        KeyCode::PageDown => app.scroll_down(app.viewport_height.max(1)),
        KeyCode::End if shift => app.scroll_to_bottom(),
        _ => {}
    }
}
