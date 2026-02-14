//! Tests for TUI rendering.

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]
mod tests {
    use crate::app::{App, AppMode, PendingPermission, PendingUserQuestion, QuestionOptionDisplay};
    use crate::ui::draw;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Paragraph, Wrap};

    /// Create a `TestBackend` + `Terminal` of the given size and draw the app once.
    fn draw_app(width: u16, height: u16, app: &mut App) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
    }

    /// Create an App pre-loaded with `n` numbered user messages, draw it at
    /// the given terminal size, and return `(terminal, app)`.
    fn draw_app_with_messages(width: u16, height: u16, n: usize) -> (Terminal<TestBackend>, App) {
        let mut app = App::new();
        for i in 0..n {
            app.add_user_message(format!("Message {i}"));
        }
        let terminal = draw_app(width, height, &mut app);
        (terminal, app)
    }

    #[test]
    fn render_empty_app() {
        draw_app(80, 24, &mut App::new());
    }

    #[test]
    fn render_with_messages() {
        let mut app = App::new();
        app.add_user_message("Hello".to_string());
        app.start_assistant_message();
        app.append_text("Hi there!");
        app.finish_streaming();
        draw_app(80, 24, &mut app);
    }

    #[test]
    fn scroll_pinned_to_bottom_by_default() {
        let (_terminal, app) = draw_app_with_messages(80, 24, 30);
        assert!(app.scroll_pinned);
        assert!(app.total_lines >= 30);
    }

    #[test]
    fn scroll_indicator_values_are_valid() {
        let (_terminal, mut app) = draw_app_with_messages(80, 24, 30);
        app.scroll_up(5);
        assert!(!app.scroll_pinned);
        let max_scroll = app.total_lines.saturating_sub(app.viewport_height);
        assert!(app.scroll_offset <= max_scroll);
    }

    #[test]
    fn wrapped_lines_counted_correctly() {
        let mut app = App::new();
        app.add_user_message("A".repeat(100));
        draw_app(40, 24, &mut app);
        assert!(app.total_lines >= 3);
    }

    #[test]
    fn scroll_to_bottom_resets_state() {
        let mut app = App::new();
        app.total_lines = 50;
        app.viewport_height = 20;
        app.scroll_up(10);
        assert!(!app.scroll_pinned);
        assert_eq!(app.scroll_offset, 10);
        app.scroll_to_bottom();
        assert!(app.scroll_pinned);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn input_wraps_long_text() {
        let mut app = App::new();
        app.input = "A".repeat(80);
        app.cursor_pos = 80;
        let mut terminal = draw_app(40, 24, &mut app);
        let pos = terminal.get_cursor_position().unwrap();
        assert!(pos.x < 40);
        assert!(pos.y < 24);
    }

    #[test]
    fn input_short_text_stays_single_line() {
        let mut app = App::new();
        app.input = "hello".to_string();
        app.cursor_pos = 5;
        let mut terminal = draw_app(80, 24, &mut app);
        let pos = terminal.get_cursor_position().unwrap();
        // x = 2 (border + padding) + 5 (cursor col) = 7
        assert_eq!(pos.x, 7);
    }

    #[test]
    fn input_cursor_at_wrap_boundary() {
        let mut app = App::new();
        // inner_width = 40 - 4 (borders + padding) = 36
        // Text exactly fills inner width: cursor clamped to right edge of content
        app.input = "B".repeat(36);
        app.cursor_pos = 36;
        let mut terminal = draw_app(40, 24, &mut app);
        let pos = terminal.get_cursor_position().unwrap();
        // x = min(2 + 36, 40 - 3) = min(38, 37) = 37
        assert_eq!(pos.x, 37);

        // One char past inner width wraps to next line
        app.input = "B".repeat(37);
        app.cursor_pos = 37;
        let mut terminal = draw_app(40, 24, &mut app);
        let pos = terminal.get_cursor_position().unwrap();
        // row 1, col 1 → x = 2 + 1 = 3
        assert_eq!(pos.x, 3);
    }

    #[test]
    fn input_cursor_mid_position() {
        let mut app = App::new();
        // inner_width = 36. 80 chars: line1=0-35, line2=36-71, line3=72-79
        // cursor at 40: line 2, offset 40-36=4 → col 4
        app.input = "C".repeat(80);
        app.cursor_pos = 40;
        let mut terminal = draw_app(40, 24, &mut app);
        let pos = terminal.get_cursor_position().unwrap();
        // x = 2 + 4 = 6
        assert_eq!(pos.x, 6);
    }

    #[test]
    fn input_empty_renders_without_panic() {
        let mut terminal = draw_app(80, 24, &mut App::new());
        let pos = terminal.get_cursor_position().unwrap();
        // x = 2 (border + padding) + 0 = 2
        assert_eq!(pos.x, 2);
    }

    #[test]
    fn input_height_capped_at_third_of_screen() {
        let mut app = App::new();
        app.input = "D".repeat(500);
        app.cursor_pos = 500;
        let mut terminal = draw_app(40, 24, &mut app);
        let pos = terminal.get_cursor_position().unwrap();
        assert!(pos.x < 40);
        assert!(pos.y < 24);
    }

    // -- Message spacing tests --

    #[test]
    fn empty_line_between_messages() {
        let mut app = App::new();

        // Add: User → Assistant → User
        app.add_user_message("Hello".to_string());
        app.start_assistant_message();
        app.append_text("Hi there!");
        app.finish_streaming();
        app.add_user_message("Thanks".to_string());

        draw_app(80, 40, &mut app);

        // 3 messages + 2 empty separator lines between them = 5 total lines
        assert_eq!(
            app.total_lines, 5,
            "Expected 3 message lines + 2 separator lines = 5, got {}",
            app.total_lines,
        );
    }

    #[test]
    fn no_empty_line_after_streaming_message() {
        let mut app = App::new();

        // User → Assistant (still streaming)
        app.add_user_message("Hello".to_string());
        app.start_assistant_message();
        app.append_text("thinking...");
        // NOT calling finish_streaming() — message still streaming

        draw_app(80, 40, &mut app);

        // 2 messages + 1 separator (after User, before streaming Assistant) = 3 lines
        // No trailing separator after the streaming message
        assert_eq!(
            app.total_lines, 3,
            "Expected 2 message lines + 1 separator = 3, got {}",
            app.total_lines,
        );
    }

    #[test]
    fn single_message_no_separator() {
        let mut app = App::new();

        app.add_user_message("Hello".to_string());

        draw_app(80, 40, &mut app);

        // Single message, no separator
        assert_eq!(
            app.total_lines, 1,
            "Expected 1 message line, no separators, got {}",
            app.total_lines,
        );
    }

    // -- Permission panel rendering tests --

    fn make_permission_app(mode: AppMode) -> App {
        let mut app = App::new();
        app.mode = mode;
        app.pending_permission = Some(PendingPermission {
            request_id: "r1".to_string(),
            tool_name: "Bash".to_string(),
            description: "ls -la".to_string(),
            original_input: None,
            edit_buffer: String::new(),
            edit_cursor: 0,
            deny_interrupt: false,
        });
        app
    }

    #[test]
    fn render_permission_prompt_panel() {
        draw_app(80, 24, &mut make_permission_app(AppMode::PermissionPrompt));
    }

    #[test]
    fn render_permission_prompt_with_input_preview() {
        let mut app = make_permission_app(AppMode::PermissionPrompt);
        app.pending_permission.as_mut().unwrap().original_input =
            Some(serde_json::json!({"command": "ls -la", "timeout": 30}));
        draw_app(80, 24, &mut app);
    }

    #[test]
    fn render_permission_edit_input() {
        let mut app = make_permission_app(AppMode::PermissionEditInput);
        app.pending_permission.as_mut().unwrap().edit_buffer = r#"{"command": "ls"}"#.to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 17;
        draw_app(80, 24, &mut app);
    }

    #[test]
    fn render_permission_comment_mode() {
        let mut app = make_permission_app(AppMode::PermissionComment);
        app.pending_permission.as_mut().unwrap().edit_buffer = "be careful with this".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 20;
        draw_app(80, 24, &mut app);
    }

    #[test]
    fn render_permission_deny_message_interrupt() {
        let mut app = make_permission_app(AppMode::PermissionDenyMessage);
        app.pending_permission.as_mut().unwrap().deny_interrupt = true;
        app.pending_permission.as_mut().unwrap().edit_buffer = "stop this".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 9;
        draw_app(80, 24, &mut app);
    }

    #[test]
    fn render_permission_deny_message_continue() {
        let mut app = make_permission_app(AppMode::PermissionDenyMessage);
        app.pending_permission.as_mut().unwrap().deny_interrupt = false;
        draw_app(80, 24, &mut app);
    }

    // -- Question panel rendering tests --

    /// Create an App with a pending user question and the given parameters.
    fn make_question_app(
        question_id: &str,
        question: &str,
        options: Vec<QuestionOptionDisplay>,
        multi_select: bool,
        highlight: usize,
        selected: Vec<usize>,
    ) -> App {
        let mut app = App::new();
        app.mode = AppMode::UserQuestion;
        app.pending_question = Some(PendingUserQuestion {
            question_id: question_id.to_string(),
            question: question.to_string(),
            options,
            multi_select,
            highlight,
            selected,
        });
        app
    }

    #[test]
    fn render_question_single_select() {
        let mut app = make_question_app(
            "q1",
            "Which approach?",
            vec![
                QuestionOptionDisplay {
                    label: "Option A".to_string(),
                    description: "Fast but risky".to_string(),
                },
                QuestionOptionDisplay {
                    label: "Option B".to_string(),
                    description: "Slow but safe".to_string(),
                },
            ],
            false,
            0,
            Vec::new(),
        );
        draw_app(80, 24, &mut app);
    }

    #[test]
    fn render_question_multi_select_with_selections() {
        let mut app = make_question_app(
            "q2",
            "Which features?",
            vec![
                QuestionOptionDisplay {
                    label: "Auth".to_string(),
                    description: "User auth".to_string(),
                },
                QuestionOptionDisplay {
                    label: "Cache".to_string(),
                    description: "Redis cache".to_string(),
                },
                QuestionOptionDisplay {
                    label: "Logs".to_string(),
                    description: "Logging".to_string(),
                },
            ],
            true,
            1,
            vec![0, 2],
        );
        draw_app(80, 24, &mut app);
    }

    /// Word-wrapped messages must report the same `total_lines` that ratatui
    /// actually renders. If our count is lower, `max_scroll` is too small and
    /// the user can't see the bottom of long responses.
    #[test]
    fn total_lines_matches_ratatui_word_wrap() {
        // Narrow terminal forces aggressive word wrapping
        let width = 30u16;
        let mut app = App::new();

        // Build a message with many short words — word wrapping will produce
        // MORE lines than simple ceil(char_count / width) because words can't
        // be split across line boundaries.
        let words: Vec<String> = (0..40).map(|i| format!("word{i}")).collect();
        let long_text = words.join(" ");
        app.start_assistant_message();
        app.append_text(&long_text);
        app.finish_streaming();

        draw_app(width, 40, &mut app);

        // Build the same Paragraph that draw_messages builds and ask ratatui
        // for its authoritative line count.
        let inner_width = width.saturating_sub(4); // borders + padding
        let prefix = "Claude: ";
        let content_lines: Vec<&str> = long_text.split('\n').collect();
        let mut lines: Vec<Line<'_>> = Vec::new();
        lines.push(Line::from(vec![
            Span::raw(prefix),
            Span::raw(content_lines[0]),
        ]));
        let indent = " ".repeat(prefix.len());
        for cl in content_lines.iter().skip(1) {
            lines.push(Line::from(vec![Span::raw(indent.clone()), Span::raw(*cl)]));
        }
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let expected_total = paragraph.line_count(inner_width) as u16;

        assert_eq!(
            app.total_lines, expected_total,
            "total_lines ({}) must match ratatui Paragraph::line_count ({}) for correct scrolling",
            app.total_lines, expected_total,
        );
    }

    #[test]
    fn render_question_narrow_terminal() {
        let mut app = make_question_app(
            "q3",
            "Pick one",
            vec![
                QuestionOptionDisplay {
                    label: "A".to_string(),
                    description: String::new(),
                },
                QuestionOptionDisplay {
                    label: "B".to_string(),
                    description: String::new(),
                },
            ],
            false,
            0,
            vec![0],
        );
        draw_app(40, 15, &mut app);
    }
}
