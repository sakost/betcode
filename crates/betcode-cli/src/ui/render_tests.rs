//! Tests for TUI rendering.

#[cfg(test)]
mod tests {
    use crate::app::{App, AppMode, PendingPermission, PendingUserQuestion, QuestionOptionDisplay};
    use crate::ui::draw;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn render_empty_app() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_with_messages() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.add_user_message("Hello".to_string());
        app.start_assistant_message();
        app.append_text("Hi there!");
        app.finish_streaming();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn scroll_pinned_to_bottom_by_default() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        for i in 0..30 {
            app.add_user_message(format!("Message {}", i));
        }
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        assert!(app.scroll_pinned);
        assert!(app.total_lines >= 30);
    }

    #[test]
    fn scroll_indicator_values_are_valid() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        for i in 0..30 {
            app.add_user_message(format!("Message {}", i));
        }
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        app.scroll_up(5);
        assert!(!app.scroll_pinned);
        let max_scroll = app.total_lines.saturating_sub(app.viewport_height);
        assert!(app.scroll_offset <= max_scroll);
    }

    #[test]
    fn wrapped_lines_counted_correctly() {
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.add_user_message("A".repeat(100));
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
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
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.input = "A".repeat(80);
        app.cursor_pos = 80;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert!(pos.x < 40);
        assert!(pos.y < 24);
    }

    #[test]
    fn input_short_text_stays_single_line() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.input = "hello".to_string();
        app.cursor_pos = 5;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(pos.x, 6);
    }

    #[test]
    fn input_cursor_at_wrap_boundary() {
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.input = "B".repeat(38);
        app.cursor_pos = 38;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(pos.x, 38);

        app.input = "B".repeat(39);
        app.cursor_pos = 39;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(pos.x, 2);
    }

    #[test]
    fn input_cursor_mid_position() {
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.input = "C".repeat(80);
        app.cursor_pos = 40;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(pos.x, 3);
    }

    #[test]
    fn input_empty_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(pos.x, 1);
    }

    #[test]
    fn input_height_capped_at_third_of_screen() {
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.input = "D".repeat(500);
        app.cursor_pos = 500;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let pos = terminal.get_cursor_position().unwrap();
        assert!(pos.x < 40);
        assert!(pos.y < 24);
    }

    // -- Message spacing tests --

    #[test]
    fn empty_line_between_messages() {
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        // Add: User → Assistant → User
        app.add_user_message("Hello".to_string());
        app.start_assistant_message();
        app.append_text("Hi there!");
        app.finish_streaming();
        app.add_user_message("Thanks".to_string());

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        // 3 messages + 2 empty separator lines between them = 5 total lines
        assert_eq!(
            app.total_lines, 5,
            "Expected 3 message lines + 2 separator lines = 5, got {}",
            app.total_lines,
        );
    }

    #[test]
    fn no_empty_line_after_streaming_message() {
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        // User → Assistant (still streaming)
        app.add_user_message("Hello".to_string());
        app.start_assistant_message();
        app.append_text("thinking...");
        // NOT calling finish_streaming() — message still streaming

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

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
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        app.add_user_message("Hello".to_string());

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

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
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_permission_app(AppMode::PermissionPrompt);
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_permission_prompt_with_input_preview() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_permission_app(AppMode::PermissionPrompt);
        app.pending_permission.as_mut().unwrap().original_input =
            Some(serde_json::json!({"command": "ls -la", "timeout": 30}));
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_permission_edit_input() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_permission_app(AppMode::PermissionEditInput);
        app.pending_permission.as_mut().unwrap().edit_buffer = r#"{"command": "ls"}"#.to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 17;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_permission_comment_mode() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_permission_app(AppMode::PermissionComment);
        app.pending_permission.as_mut().unwrap().edit_buffer = "be careful with this".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 20;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_permission_deny_message_interrupt() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_permission_app(AppMode::PermissionDenyMessage);
        app.pending_permission.as_mut().unwrap().deny_interrupt = true;
        app.pending_permission.as_mut().unwrap().edit_buffer = "stop this".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 9;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_permission_deny_message_continue() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_permission_app(AppMode::PermissionDenyMessage);
        app.pending_permission.as_mut().unwrap().deny_interrupt = false;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    // -- Question panel rendering tests --

    #[test]
    fn render_question_single_select() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.mode = AppMode::UserQuestion;
        app.pending_question = Some(PendingUserQuestion {
            question_id: "q1".to_string(),
            question: "Which approach?".to_string(),
            options: vec![
                QuestionOptionDisplay {
                    label: "Option A".to_string(),
                    description: "Fast but risky".to_string(),
                },
                QuestionOptionDisplay {
                    label: "Option B".to_string(),
                    description: "Slow but safe".to_string(),
                },
            ],
            multi_select: false,
            highlight: 0,
            selected: Vec::new(),
        });
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn render_question_multi_select_with_selections() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.mode = AppMode::UserQuestion;
        app.pending_question = Some(PendingUserQuestion {
            question_id: "q2".to_string(),
            question: "Which features?".to_string(),
            options: vec![
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
            multi_select: true,
            highlight: 1,
            selected: vec![0, 2],
        });
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    /// Word-wrapped messages must report the same `total_lines` that ratatui
    /// actually renders. If our count is lower, `max_scroll` is too small and
    /// the user can't see the bottom of long responses.
    #[test]
    fn total_lines_matches_ratatui_word_wrap() {
        // Narrow terminal forces aggressive word wrapping
        let width = 30u16;
        let backend = TestBackend::new(width, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        // Build a message with many short words — word wrapping will produce
        // MORE lines than simple ceil(char_count / width) because words can't
        // be split across line boundaries.
        let words: Vec<String> = (0..40).map(|i| format!("word{}", i)).collect();
        let long_text = words.join(" ");
        app.start_assistant_message();
        app.append_text(&long_text);
        app.finish_streaming();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        // Build the same Paragraph that draw_messages builds and ask ratatui
        // for its authoritative line count.
        let inner_width = width.saturating_sub(2); // borders
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Paragraph, Wrap};
        let prefix = "Claude: ";
        let content_lines: Vec<&str> = long_text.split('\n').collect();
        let mut lines: Vec<Line> = Vec::new();
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
        let backend = TestBackend::new(40, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.mode = AppMode::UserQuestion;
        app.pending_question = Some(PendingUserQuestion {
            question_id: "q3".to_string(),
            question: "Pick one".to_string(),
            options: vec![
                QuestionOptionDisplay {
                    label: "A".to_string(),
                    description: String::new(),
                },
                QuestionOptionDisplay {
                    label: "B".to_string(),
                    description: String::new(),
                },
            ],
            multi_select: false,
            highlight: 0,
            selected: vec![0],
        });
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }
}
