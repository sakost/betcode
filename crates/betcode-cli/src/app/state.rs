//! Application state and types.

use std::collections::VecDeque;

use betcode_proto::v1::AgentEvent;

/// Application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    PermissionPrompt,
    SessionList,
}

/// A displayable message in the conversation.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
    pub streaming: bool,
}

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Pending permission request shown as dialog.
#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
}

/// Token usage info for status bar.
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
}

/// TUI application state.
pub struct App {
    pub mode: AppMode,
    pub session_id: Option<String>,
    pub model: String,
    pub messages: Vec<DisplayMessage>,
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: VecDeque<String>,
    pub history_index: Option<usize>,
    pub scroll_offset: u16,
    pub should_quit: bool,
    pub status: String,
    pub token_usage: Option<TokenUsage>,
    pub pending_permission: Option<PendingPermission>,
    pub agent_busy: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: AppMode::Normal,
            session_id: None,
            model: String::new(),
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            input_history: VecDeque::with_capacity(100),
            history_index: None,
            scroll_offset: 0,
            should_quit: false,
            status: "Connecting...".to_string(),
            token_usage: None,
            pending_permission: None,
            agent_busy: false,
        }
    }

    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(DisplayMessage {
            role: MessageRole::User,
            content,
            streaming: false,
        });
    }

    pub fn start_assistant_message(&mut self) {
        self.messages.push(DisplayMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            streaming: true,
        });
    }

    pub fn append_text(&mut self, text: &str) {
        if let Some(msg) = self.messages.last_mut() {
            if msg.streaming {
                msg.content.push_str(text);
            }
        }
    }

    pub fn finish_streaming(&mut self) {
        if let Some(msg) = self.messages.last_mut() {
            msg.streaming = false;
        }
    }

    pub fn add_system_message(&mut self, role: MessageRole, content: String) {
        self.messages.push(DisplayMessage {
            role,
            content,
            streaming: false,
        });
    }

    /// Process an incoming agent event.
    pub fn handle_event(&mut self, event: AgentEvent) {
        use betcode_proto::v1::agent_event::Event;

        match event.event {
            Some(Event::TextDelta(delta)) => {
                // Skip empty text deltas to avoid blank "Claude:" lines
                if delta.text.is_empty() && !delta.is_complete {
                    return;
                }
                if !delta.text.is_empty() {
                    if self.messages.last().is_none_or(|m| !m.streaming) {
                        self.start_assistant_message();
                    }
                    self.append_text(&delta.text);
                }
                if delta.is_complete {
                    self.finish_streaming();
                }
            }
            Some(Event::ToolCallStart(tool)) => {
                // Finish any open streaming message before tool output
                self.finish_streaming();
                let msg = if tool.description.is_empty() {
                    format!("[Tool: {}]", tool.tool_name)
                } else {
                    format!("[Tool: {} - {}]", tool.tool_name, tool.description)
                };
                self.add_system_message(MessageRole::Tool, msg);
            }
            Some(Event::ToolCallResult(result)) => {
                let status = if result.is_error { "ERROR" } else { "OK" };
                let preview = if result.output.len() > 200 {
                    format!("{}...", &result.output[..200])
                } else {
                    result.output.clone()
                };
                let msg = format!("[Tool Result ({}): {}]", status, preview);
                self.add_system_message(MessageRole::Tool, msg);
            }
            Some(Event::PermissionRequest(perm)) => {
                self.mode = AppMode::PermissionPrompt;
                self.pending_permission = Some(PendingPermission {
                    request_id: perm.request_id,
                    tool_name: perm.tool_name,
                    description: perm.description,
                });
            }
            Some(Event::SessionInfo(info)) => {
                self.session_id = Some(info.session_id.clone());
                self.model = info.model.clone();
                self.status = format!("Session: {} | Model: {}", info.session_id, info.model);
            }
            Some(Event::StatusChange(sc)) => {
                // Only update status text if the message is non-empty,
                // otherwise we'd blank the session/model info from SessionInfo.
                if !sc.message.is_empty() {
                    self.status = sc.message;
                }
                self.agent_busy = sc.status == 1;
            }
            Some(Event::Usage(usage)) => {
                self.token_usage = Some(TokenUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cost_usd: usage.cost_usd,
                });
            }
            Some(Event::TurnComplete(_)) => {
                self.finish_streaming();
                self.agent_busy = false;
            }
            Some(Event::Error(err)) => {
                let msg = format!("[Error: {} - {}]", err.code, err.message);
                self.add_system_message(MessageRole::System, msg);
                if err.is_fatal {
                    self.status = format!("Fatal error: {}", err.message);
                }
            }
            _ => {}
        }
    }

    /// Submit the current input.
    pub fn submit_input(&mut self) -> Option<String> {
        if self.input.is_empty() {
            return None;
        }
        let text = self.input.clone();
        self.input_history.push_front(text.clone());
        if self.input_history.len() > 100 {
            self.input_history.pop_back();
        }
        self.input.clear();
        self.cursor_pos = 0;
        self.history_index = None;
        self.add_user_message(text.clone());
        Some(text)
    }

    /// Navigate input history (up).
    pub fn history_up(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => 0,
            Some(i) => (i + 1).min(self.input_history.len().saturating_sub(1)),
        };
        self.history_index = Some(idx);
        self.input = self.input_history[idx].clone();
        self.cursor_pos = self.input.len();
    }

    /// Navigate input history (down).
    pub fn history_down(&mut self) {
        match self.history_index {
            None => {}
            Some(0) => {
                self.history_index = None;
                self.input.clear();
                self.cursor_pos = 0;
            }
            Some(i) => {
                let idx = i - 1;
                self.history_index = Some(idx);
                self.input = self.input_history[idx].clone();
                self.cursor_pos = self.input.len();
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_app_state() {
        let app = App::new();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(!app.should_quit);
        assert!(!app.agent_busy);
        assert!(app.messages.is_empty());
    }

    #[test]
    fn submit_input_adds_message() {
        let mut app = App::new();
        app.input = "Hello".to_string();
        app.cursor_pos = 5;

        let text = app.submit_input();
        assert_eq!(text, Some("Hello".to_string()));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::User);
        assert!(app.input.is_empty());
    }

    #[test]
    fn streaming_text_appends() {
        let mut app = App::new();
        app.start_assistant_message();
        app.append_text("Hello ");
        app.append_text("world");
        app.finish_streaming();

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello world");
        assert!(!app.messages[0].streaming);
    }

    #[test]
    fn input_history_navigation() {
        let mut app = App::new();
        app.input = "first".to_string();
        app.submit_input();
        app.input = "second".to_string();
        app.submit_input();

        app.history_up();
        assert_eq!(app.input, "second");
        app.history_up();
        assert_eq!(app.input, "first");
        app.history_down();
        assert_eq!(app.input, "second");
        app.history_down();
        assert!(app.input.is_empty());
    }

    #[test]
    fn empty_submit_returns_none() {
        let mut app = App::new();
        assert_eq!(app.submit_input(), None);
    }

    fn make_event(event: betcode_proto::v1::agent_event::Event) -> AgentEvent {
        AgentEvent {
            sequence: 1,
            timestamp: None,
            parent_tool_use_id: String::new(),
            event: Some(event),
        }
    }

    #[test]
    fn empty_text_delta_does_not_create_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: String::new(),
            is_complete: false,
        })));
        assert!(app.messages.is_empty(), "Empty text delta should not create a message");
    }

    #[test]
    fn text_delta_creates_and_appends() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Hello ".to_string(),
            is_complete: false,
        })));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello ");
        assert!(app.messages[0].streaming);

        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "world".to_string(),
            is_complete: false,
        })));
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello world");
    }

    #[test]
    fn tool_call_start_finishes_streaming_and_adds_tool_msg() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // Start streaming text
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Let me check.".to_string(),
            is_complete: false,
        })));
        assert!(app.messages[0].streaming);

        // Tool call arrives — should finish streaming first
        app.handle_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Bash".to_string(),
            input: None,
            description: "ls -la".to_string(),
        })));
        assert_eq!(app.messages.len(), 2);
        assert!(!app.messages[0].streaming); // streaming finished
        assert_eq!(app.messages[0].role, MessageRole::Assistant);
        assert_eq!(app.messages[1].role, MessageRole::Tool);
        assert!(app.messages[1].content.contains("Bash"));
        assert!(app.messages[1].content.contains("ls -la"));
    }

    #[test]
    fn tool_call_start_empty_description_no_dash() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.handle_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Read".to_string(),
            input: None,
            description: String::new(),
        })));
        assert_eq!(app.messages[0].content, "[Tool: Read]");
    }

    #[test]
    fn status_change_empty_message_preserves_status() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.status = "Session: abc | Model: claude".to_string();

        app.handle_event(make_event(Event::StatusChange(betcode_proto::v1::StatusChange {
            status: 1, // Thinking
            message: String::new(),
        })));
        assert_eq!(app.status, "Session: abc | Model: claude");
        assert!(app.agent_busy);
    }

    #[test]
    fn status_change_with_message_updates_status() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();
        app.status = "old status".to_string();

        app.handle_event(make_event(Event::StatusChange(betcode_proto::v1::StatusChange {
            status: 0,
            message: "new status".to_string(),
        })));
        assert_eq!(app.status, "new status");
        assert!(!app.agent_busy);
    }

    #[test]
    fn text_after_tool_creates_new_assistant_message() {
        use betcode_proto::v1::agent_event::Event;
        let mut app = App::new();

        // Text → tool → text should create separate assistant messages
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "First".to_string(),
            is_complete: false,
        })));
        app.handle_event(make_event(Event::ToolCallStart(betcode_proto::v1::ToolCallStart {
            tool_id: "t1".to_string(),
            tool_name: "Bash".to_string(),
            input: None,
            description: "ls".to_string(),
        })));
        app.handle_event(make_event(Event::TextDelta(betcode_proto::v1::TextDelta {
            text: "Second".to_string(),
            is_complete: false,
        })));

        assert_eq!(app.messages.len(), 3);
        assert_eq!(app.messages[0].content, "First");
        assert_eq!(app.messages[0].role, MessageRole::Assistant);
        assert_eq!(app.messages[1].role, MessageRole::Tool);
        assert_eq!(app.messages[2].content, "Second");
        assert_eq!(app.messages[2].role, MessageRole::Assistant);
    }
}
