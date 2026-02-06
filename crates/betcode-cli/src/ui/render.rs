//! TUI rendering functions.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, AppMode, MessageRole};

/// Draw the full UI.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(5),    // Messages
            Constraint::Length(3), // Input
            Constraint::Length(1), // Status bar
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_messages(frame, app, chunks[1]);
    draw_input(frame, app, chunks[2]);
    draw_status_bar(frame, app, chunks[3]);

    // Draw permission dialog overlay if needed
    if app.mode == AppMode::PermissionPrompt {
        draw_permission_dialog(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let model_info = if app.model.is_empty() {
        "BetCode".to_string()
    } else {
        format!("BetCode | {}", app.model)
    };

    let session_info = app
        .session_id
        .as_deref()
        .map(|s| format!(" | Session: {}", &s[..8.min(s.len())]))
        .unwrap_or_default();

    let busy = if app.agent_busy { " [thinking...]" } else { "" };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            model_info,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(session_info),
        Span::styled(busy, Style::default().fg(Color::Yellow)),
    ]));

    frame.render_widget(header, area);
}

fn draw_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        let (prefix, color) = match msg.role {
            MessageRole::User => ("You: ", Color::Green),
            MessageRole::Assistant => ("Claude: ", Color::Blue),
            MessageRole::System => ("System: ", Color::Yellow),
            MessageRole::Tool => ("", Color::DarkGray),
        };

        let prefix_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let content_lines: Vec<&str> = msg.content.split('\n').collect();

        if content_lines.is_empty() || (content_lines.len() == 1 && content_lines[0].is_empty()) {
            let cursor = if msg.streaming { "█" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(cursor, Style::default().fg(Color::White)),
            ]));
        } else {
            let cursor = if msg.streaming && content_lines.len() == 1 {
                "█"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::raw(content_lines[0]),
                Span::styled(cursor, Style::default().fg(Color::White)),
            ]));

            let indent = " ".repeat(prefix.len());
            for (i, content_line) in content_lines.iter().enumerate().skip(1) {
                let cursor = if msg.streaming && i == content_lines.len() - 1 {
                    "█"
                } else {
                    ""
                };
                lines.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::raw(*content_line),
                    Span::styled(cursor, Style::default().fg(Color::White)),
                ]));
            }
        }
    }

    let inner_height = area.height.saturating_sub(2); // minus borders
    let inner_width = area.width.saturating_sub(2) as usize; // minus borders

    // Count wrapped visual lines using unicode display width
    let total: u16 = lines
        .iter()
        .map(|line| {
            if inner_width == 0 {
                return 1u16;
            }
            let display_width: usize = line
                .spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            // Each line takes at least 1 row; long lines wrap to ceil(width/inner_width)
            1u16.max(
                display_width
                    .saturating_add(inner_width - 1)
                    .checked_div(inner_width)
                    .unwrap_or(1) as u16,
            )
        })
        .sum();

    // Update app state so scroll methods know the bounds
    app.viewport_height = inner_height;
    app.total_lines = total;

    // Compute absolute scroll position from bottom-relative offset
    let max_scroll = total.saturating_sub(inner_height);
    let scroll = if app.scroll_pinned {
        // Auto-scroll: pin to bottom
        max_scroll
    } else {
        // Manual scroll: offset is distance from bottom
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let title = if !app.scroll_pinned {
        let position = max_scroll.saturating_sub(scroll);
        format!(
            "Conversation [scroll: {}/{}]",
            position.min(max_scroll),
            max_scroll
        )
    } else {
        "Conversation".to_string()
    };

    let messages = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(messages, area);
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let input = Paragraph::new(app.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.agent_busy {
                    "Waiting..."
                } else {
                    "Input"
                }),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(input, area);

    // Position cursor (clamp to prevent u16 overflow and stay within input area)
    let max_cursor_x = area.x.saturating_add(area.width.saturating_sub(2));
    let cursor_x = area
        .x
        .saturating_add(1)
        .saturating_add(app.cursor_pos as u16)
        .min(max_cursor_x);
    frame.set_cursor_position((cursor_x, area.y + 1));
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let usage = app.token_usage.as_ref().map_or(String::new(), |u| {
        format!(
            " | Tokens: {}in/{}out | ${:.4}",
            u.input_tokens, u.output_tokens, u.cost_usd
        )
    });

    let status = Paragraph::new(Line::from(vec![
        Span::styled(&app.status, Style::default().fg(Color::DarkGray)),
        Span::styled(usage, Style::default().fg(Color::DarkGray)),
        Span::styled(
            " | Ctrl+C: quit | Enter: send",
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    frame.render_widget(status, area);
}

fn draw_permission_dialog(frame: &mut Frame, app: &App) {
    let Some(ref perm) = app.pending_permission else {
        return;
    };

    // Center dialog
    let area = frame.area();
    let dialog_width = 60.min(area.width - 4);
    let dialog_height = 8.min(area.height - 4);
    let dialog_area = Rect::new(
        (area.width - dialog_width) / 2,
        (area.height - dialog_height) / 2,
        dialog_width,
        dialog_height,
    );

    frame.render_widget(Clear, dialog_area);

    let text = vec![
        Line::from(Span::styled(
            format!("Tool: {}", perm.tool_name),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(perm.description.as_str()),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "[Y]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Allow  "),
            Span::styled(
                "[N]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Deny  "),
            Span::styled(
                "[A]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Always"),
        ]),
    ];

    let dialog = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Permission Required")
                .style(Style::default().fg(Color::White)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(dialog, dialog_area);
}

#[cfg(test)]
mod tests {
    use super::*;
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

        // Add enough messages to exceed viewport
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

        // Now scroll up
        app.scroll_up(5);
        assert!(!app.scroll_pinned);

        // The scroll_offset should never exceed max_scroll
        let max_scroll = app.total_lines.saturating_sub(app.viewport_height);
        assert!(
            app.scroll_offset <= max_scroll,
            "scroll_offset {} should be <= max_scroll {}",
            app.scroll_offset,
            max_scroll,
        );
    }

    #[test]
    fn wrapped_lines_counted_correctly() {
        let backend = TestBackend::new(40, 24); // narrow terminal
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        // Add a message that will definitely wrap at width 40 (inner ~38)
        app.add_user_message("A".repeat(100));

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        // "You: " (5 chars) + 100 "A"s = 105 chars, inner width ~38
        // Should wrap to ceil(105/38) = 3 lines
        assert!(
            app.total_lines >= 3,
            "Expected at least 3 wrapped lines, got {}",
            app.total_lines,
        );
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
}
