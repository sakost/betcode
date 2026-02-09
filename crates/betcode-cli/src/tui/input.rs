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
                    AppMode::FingerprintVerification => {
                        handle_fingerprint_key(app, key.code);
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

/// Handle a key press in fingerprint verification mode.
fn handle_fingerprint_key(app: &mut App, key: KeyCode) {
    use crate::tui::fingerprint_panel::FingerprintDecision;

    let needs_action = app
        .pending_fingerprint
        .as_ref()
        .is_some_and(|fp| fp.needs_action());

    if needs_action {
        // Mismatch: Y to accept, N/Esc to reject
        match key {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(ref mut fp) = app.pending_fingerprint {
                    fp.decision = Some(FingerprintDecision::Accept);
                }
                app.pending_fingerprint = None;
                app.mode = AppMode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(ref mut fp) = app.pending_fingerprint {
                    fp.decision = Some(FingerprintDecision::Reject);
                }
                app.should_quit = true;
            }
            _ => {} // Ignore other keys on mismatch
        }
    } else {
        // TOFU or Matched: any key continues
        app.pending_fingerprint = None;
        app.mode = AppMode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fingerprint_panel::{FingerprintDecision, FingerprintPrompt};
    use betcode_crypto::FingerprintCheck;

    fn make_app_with_mismatch() -> App {
        let mut app = App::new();
        app.mode = AppMode::FingerprintVerification;
        app.pending_fingerprint = Some(FingerprintPrompt::new(
            "m1",
            "dd:ee",
            FingerprintCheck::Mismatch {
                expected: "aa:bb".into(),
                actual: "dd:ee".into(),
            },
        ));
        app
    }

    fn make_app_with_tofu() -> App {
        let mut app = App::new();
        app.mode = AppMode::FingerprintVerification;
        app.pending_fingerprint = Some(FingerprintPrompt::new(
            "m1",
            "aa:bb",
            FingerprintCheck::TrustOnFirstUse,
        ));
        app
    }

    fn make_app_with_matched() -> App {
        let mut app = App::new();
        app.mode = AppMode::FingerprintVerification;
        app.pending_fingerprint = Some(FingerprintPrompt::new(
            "m1",
            "aa:bb",
            FingerprintCheck::Matched,
        ));
        app
    }

    #[test]
    fn fingerprint_y_accepts_mismatch() {
        let mut app = make_app_with_mismatch();
        handle_fingerprint_key(&mut app, KeyCode::Char('y'));
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.pending_fingerprint.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn fingerprint_n_rejects_mismatch() {
        let mut app = make_app_with_mismatch();
        handle_fingerprint_key(&mut app, KeyCode::Char('n'));
        assert!(app.should_quit);
    }

    #[test]
    fn fingerprint_esc_rejects_mismatch() {
        let mut app = make_app_with_mismatch();
        handle_fingerprint_key(&mut app, KeyCode::Esc);
        assert!(app.should_quit);
    }

    #[test]
    fn fingerprint_any_key_continues_tofu() {
        let mut app = make_app_with_tofu();
        handle_fingerprint_key(&mut app, KeyCode::Enter);
        assert!(app.pending_fingerprint.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn fingerprint_any_key_continues_matched() {
        let mut app = make_app_with_matched();
        handle_fingerprint_key(&mut app, KeyCode::Enter);
        assert!(app.pending_fingerprint.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn fingerprint_ignores_random_keys_on_mismatch() {
        let mut app = make_app_with_mismatch();
        handle_fingerprint_key(&mut app, KeyCode::Char('z'));
        assert_eq!(app.mode, AppMode::FingerprintVerification);
        assert!(app.pending_fingerprint.is_some());
        assert!(!app.should_quit);
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
