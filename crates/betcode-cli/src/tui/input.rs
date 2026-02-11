//! Input handling for TUI key events.

use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::app::{App, AppMode, MessageRole};
use betcode_proto::v1::AgentRequest;

use super::TermEvent;

/// Actions returned by completion key handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionAction {
    /// No completion action taken.
    None,
    /// Accept the selected completion text.
    Accept(String),
    /// Completion state was toggled/updated (no text change needed).
    Updated,
}

/// Handle a key event related to completion UI.
/// Returns a `CompletionAction` indicating what happened.
pub fn handle_completion_key(app: &mut App, key: crossterm::event::KeyEvent) -> CompletionAction {
    match key.code {
        KeyCode::Tab => {
            app.completion_state.popup_visible = !app.completion_state.popup_visible;
            CompletionAction::Updated
        }
        KeyCode::Esc if app.completion_state.popup_visible => {
            app.completion_state.popup_visible = false;
            CompletionAction::Updated
        }
        KeyCode::Up if app.completion_state.popup_visible => {
            if !app.completion_state.items.is_empty() {
                if app.completion_state.selected_index == 0 {
                    app.completion_state.selected_index = app.completion_state.items.len() - 1;
                } else {
                    app.completion_state.selected_index -= 1;
                }
                app.completion_state.adjust_scroll();
                app.completion_state.ghost_text = app
                    .completion_state
                    .items
                    .get(app.completion_state.selected_index)
                    .cloned();
            }
            CompletionAction::Updated
        }
        KeyCode::Down if app.completion_state.popup_visible => {
            if !app.completion_state.items.is_empty() {
                app.completion_state.selected_index =
                    (app.completion_state.selected_index + 1) % app.completion_state.items.len();
                app.completion_state.adjust_scroll();
                app.completion_state.ghost_text = app
                    .completion_state
                    .items
                    .get(app.completion_state.selected_index)
                    .cloned();
            }
            CompletionAction::Updated
        }
        KeyCode::Enter if app.completion_state.popup_visible => {
            let text = app
                .completion_state
                .items
                .get(app.completion_state.selected_index)
                .cloned()
                .unwrap_or_default();
            app.completion_state.popup_visible = false;
            if text.is_empty() {
                CompletionAction::None
            } else {
                CompletionAction::Accept(text)
            }
        }
        _ => CompletionAction::None,
    }
}

/// Process a terminal event, updating app state and optionally sending gRPC requests.
pub async fn handle_term_event(app: &mut App, tx: &mpsc::Sender<AgentRequest>, event: TermEvent) {
    match event {
        TermEvent::Key(key) => {
            // Ctrl+T toggles the status panel overlay.
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('t')
            {
                app.show_status_panel = !app.show_status_panel;
                return;
            }
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
    let needs_action = app
        .pending_fingerprint
        .as_ref()
        .is_some_and(super::fingerprint_panel::FingerprintPrompt::needs_action);

    if needs_action {
        // Mismatch: Y to accept, N/Esc to reject
        match key {
            KeyCode::Char('y' | 'Y') => {
                if let Some(ref mut fp) = app.pending_fingerprint {
                    fp.decision = Some(crate::tui::fingerprint_panel::FingerprintDecision::Accept);
                }
                app.pending_fingerprint = None;
                app.mode = AppMode::Normal;
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                if let Some(ref mut fp) = app.pending_fingerprint {
                    fp.decision = Some(crate::tui::fingerprint_panel::FingerprintDecision::Reject);
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

/// Replace the current token (at cursor) with `replacement`, updating cursor position.
///
/// # Panics
///
/// Panics if `rfind` returns an index that is not a valid char boundary,
/// which is structurally impossible since `rfind(char::is_whitespace)` always
/// returns char-aligned offsets.
#[allow(clippy::expect_used)]
fn replace_token(app: &mut App, replacement: &str) {
    let mut pos = app.cursor_pos.min(app.input.len());
    // Clamp to nearest char boundary to avoid panics on multi-byte UTF-8.
    while pos > 0 && !app.input.is_char_boundary(pos) {
        pos -= 1;
    }
    let before_cursor = &app.input[..pos];

    // Find the start of the current token by scanning backwards to whitespace
    let token_start = before_cursor
        .rfind(char::is_whitespace)
        .map_or(0, |i| {
            i + before_cursor[i..]
                .chars()
                .next()
                .expect("rfind returned a valid char index")
                .len_utf8()
        });

    // Find the end of the current token by scanning forward to whitespace
    let token_end = app.input[pos..]
        .find(char::is_whitespace)
        .map_or(app.input.len(), |i| pos + i);

    app.input.replace_range(token_start..token_end, replacement);
    app.cursor_pos = token_start + replacement.len();
}

/// Clear the conversation: wipe TUI messages and start a fresh session.
async fn clear_session(app: &mut App, tx: &mpsc::Sender<AgentRequest>) {
    app.messages.clear();
    app.agent_busy = false;

    // Generate a new session ID and send a StartConversation to reset context.
    let new_sid = uuid::Uuid::new_v4().to_string();
    let wd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let model = app.model.clone();

    let send_result = tx
        .send(AgentRequest {
            request: Some(betcode_proto::v1::agent_request::Request::Start(
                betcode_proto::v1::StartConversation {
                    session_id: new_sid.clone(),
                    working_directory: wd,
                    model,
                    allowed_tools: Vec::new(),
                    plan_mode: false,
                    worktree_id: String::new(),
                    metadata: std::collections::HashMap::default(),
                },
            )),
        })
        .await;

    if send_result.is_err() {
        app.add_system_message(
            MessageRole::System,
            "Failed to reset session — agent stream closed.".to_string(),
        );
    } else {
        app.session_id = Some(new_sid.clone());
        app.status = format!("Connected | Session: {}", &new_sid[..8.min(new_sid.len())]);
        app.add_system_message(MessageRole::System, "Conversation cleared.".to_string());
    }
    app.scroll_to_bottom();
}

/// Display help message listing all available commands.
fn show_help(app: &mut App) {
    let mut lines = Vec::new();
    lines.push("BetCode CLI - Available Commands".to_string());
    lines.push(String::new());

    // Collect commands grouped by category
    let mut service_cmds = Vec::new();
    let mut cc_cmds = Vec::new();
    let mut plugin_cmds = Vec::new();

    for cmd in app.command_cache.all() {
        let entry = format!("  /{:<20} {}", cmd.name, cmd.description);
        match cmd.category.as_str() {
            "Service" => service_cmds.push(entry),
            "ClaudeCode" => cc_cmds.push(entry),
            _ => plugin_cmds.push(entry),
        }
    }

    // Always include built-in CLI commands not from the cache
    lines.push("Built-in:".to_string());
    lines.push("  /exit                 Exit the CLI".to_string());
    lines.push("  /help                 Show this help message".to_string());
    lines.push("  /clear                Clear conversation and reset context".to_string());

    if !service_cmds.is_empty() {
        lines.push(String::new());
        lines.push("Service:".to_string());
        lines.extend(service_cmds);
    }

    if !cc_cmds.is_empty() {
        lines.push(String::new());
        lines.push("Claude Code:".to_string());
        lines.extend(cc_cmds);
    }

    if !plugin_cmds.is_empty() {
        lines.push(String::new());
        lines.push("Plugins:".to_string());
        lines.extend(plugin_cmds);
    }

    lines.push(String::new());
    lines.push("Keyboard shortcuts:".to_string());
    lines.push("  Ctrl+C               Quit".to_string());
    lines.push("  Ctrl+T               Toggle status panel".to_string());
    lines.push("  Tab                  Toggle completion popup".to_string());
    lines.push("  Shift+Up/Down        Scroll messages".to_string());
    lines.push("  PageUp/PageDown      Scroll messages (page)".to_string());

    app.add_system_message(MessageRole::System, lines.join("\n"));
    app.scroll_to_bottom();
}

/// Handle a key press in normal input mode.
#[allow(clippy::too_many_lines)]
async fn handle_input_key(
    app: &mut App,
    tx: &mpsc::Sender<AgentRequest>,
    key: crossterm::event::KeyEvent,
) {
    // Check completion keys first — if they handle the event, skip normal input handling.
    let completion_action = handle_completion_key(app, key);
    match completion_action {
        CompletionAction::Accept(text) => {
            // Replace only the trigger token, preserving the prefix character(s).
            // A trailing space is appended so the cursor moves past the token
            // boundary, preventing the completion popup from reopening.
            let trigger =
                crate::completion::controller::detect_trigger(&app.input, app.cursor_pos);
            match trigger {
                Some(crate::completion::controller::CompletionTrigger::Command { .. }) => {
                    replace_token(app, &format!("/{text} "));
                }
                Some(crate::completion::controller::CompletionTrigger::Agent {
                    forced: true,
                    ..
                }) => {
                    replace_token(app, &format!("@@{text} "));
                }
                Some(
                    crate::completion::controller::CompletionTrigger::Agent {
                        forced: false,
                        ..
                    }
                    | crate::completion::controller::CompletionTrigger::File { .. },
                ) => {
                    replace_token(app, &format!("@{text} "));
                }
                Some(crate::completion::controller::CompletionTrigger::Bash { .. }) | None => {
                    replace_token(app, &format!("{text} "));
                }
            }
            app.update_completion_state();
            return;
        }
        CompletionAction::Updated => {
            return;
        }
        CompletionAction::None => {}
    }

    let shift = key
        .modifiers
        .contains(crossterm::event::KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Enter => {
            if let Some(text) = app.submit_input() {
                let trimmed = text.trim();
                if trimmed == "?" {
                    show_help(app);
                } else if let Some(cmd_body) = trimmed.strip_prefix('/') {
                    // Slash command — dispatch based on command category.
                    let mut parts = cmd_body.splitn(2, char::is_whitespace);
                    let command = parts.next().unwrap_or("").to_string();
                    let args: Vec<String> = parts
                        .next()
                        .map(|a| a.split_whitespace().map(std::string::ToString::to_string).collect())
                        .unwrap_or_default();

                    // Determine routing based on command category from cache.
                    let is_service = app
                        .command_cache
                        .find_by_name(&command)
                        .is_some_and(|c| c.category == "Service");

                    match command.as_str() {
                        "exit" => {
                            app.should_quit = true;
                        }
                        "help" => {
                            show_help(app);
                        }
                        "clear" => {
                            clear_session(app, tx).await;
                        }
                        _ if is_service => {
                            // Service commands (cd, pwd, exit-daemon, etc.)
                            // executed on the daemon via CommandService.
                            if let Some(cmd_tx) = &app.service_command_tx {
                                let _ =
                                    cmd_tx.try_send(super::ServiceCommandExec { command, args });
                            }
                        }
                        _ => {
                            // Claude Code / Plugin / unknown commands:
                            // forward as user message to the agent stream
                            // (the Claude subprocess handles them).
                            let _ = tx
                                .send(AgentRequest {
                                    request: Some(
                                        betcode_proto::v1::agent_request::Request::Message(
                                            betcode_proto::v1::UserMessage {
                                                content: trimmed.to_string(),
                                                attachments: Vec::new(),
                                            },
                                        ),
                                    ),
                                })
                                .await;
                            app.agent_busy = true;
                        }
                    }
                } else {
                    // Regular user message
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
                }
                app.scroll_to_bottom();
            }
        }
        KeyCode::Char(c) => {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += 1;
            app.update_completion_state();
        }
        KeyCode::Backspace => {
            if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
                app.input.remove(app.cursor_pos);
            }
            app.update_completion_state();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fingerprint_panel::FingerprintPrompt;
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

    fn make_completion_app(item_count: usize) -> App {
        let mut app = App::new();
        app.completion_state.items = (0..item_count).map(|i| format!("item-{i}")).collect();
        app.completion_state.popup_visible = true;
        app.completion_state.selected_index = 0;
        app.completion_state.scroll_offset = 0;
        app.completion_state.ghost_text = app.completion_state.items.first().cloned();
        app
    }

    fn down_key() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE)
    }

    fn up_key() -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn completion_scroll_adjusts_on_down() {
        let mut app = make_completion_app(20);
        // Press Down 10 times — should scroll
        for _ in 0..10 {
            handle_completion_key(&mut app, down_key());
        }
        assert_eq!(app.completion_state.selected_index, 10);
        // selected=10 should be visible: scroll_offset + 8 > 10
        assert!(app.completion_state.scroll_offset + 8 > 10);
        assert!(app.completion_state.selected_index >= app.completion_state.scroll_offset);
    }

    #[test]
    fn completion_scroll_adjusts_on_up() {
        let mut app = make_completion_app(20);
        // Move to item 12
        for _ in 0..12 {
            handle_completion_key(&mut app, down_key());
        }
        assert_eq!(app.completion_state.selected_index, 12);
        let scroll_at_12 = app.completion_state.scroll_offset;

        // Now press Up back to 0
        for _ in 0..12 {
            handle_completion_key(&mut app, up_key());
        }
        assert_eq!(app.completion_state.selected_index, 0);
        assert_eq!(app.completion_state.scroll_offset, 0);
        assert!(scroll_at_12 > 0, "Should have scrolled down at item 12");
    }

    #[test]
    fn completion_scroll_wraps_down_resets_offset() {
        let mut app = make_completion_app(20);
        // Move to last item then wrap
        for _ in 0..20 {
            handle_completion_key(&mut app, down_key());
        }
        // Should wrap to 0
        assert_eq!(app.completion_state.selected_index, 0);
        assert_eq!(app.completion_state.scroll_offset, 0);
    }

    #[test]
    fn completion_scroll_wraps_up_jumps_to_end() {
        let mut app = make_completion_app(20);
        // At index 0, press Up → wraps to 19
        handle_completion_key(&mut app, up_key());
        assert_eq!(app.completion_state.selected_index, 19);
        assert_eq!(app.completion_state.scroll_offset, 12); // 19+1-8=12
    }

    #[tokio::test]
    async fn question_mark_shows_help() {
        let mut app = App::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);

        // Type "?" and press Enter
        app.input = "?".to_string();
        app.cursor_pos = 1;
        let enter =
            crossterm::event::KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        handle_input_key(&mut app, &tx, enter).await;

        // submit_input adds the user message, then show_help adds the help message
        assert!(
            app.messages.len() >= 2,
            "? should produce user msg + help message"
        );
        assert!(
            app.messages
                .last()
                .unwrap()
                .content
                .contains("Available Commands"),
            "Last message should be the help output"
        );
    }
}
