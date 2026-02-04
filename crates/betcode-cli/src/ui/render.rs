//! TUI rendering functions.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, AppMode, MessageRole};

/// Draw the full UI.
pub fn draw(frame: &mut Frame, app: &App) {
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

fn draw_messages(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .messages
        .iter()
        .map(|msg| {
            let (prefix, color) = match msg.role {
                MessageRole::User => ("You: ", Color::Green),
                MessageRole::Assistant => ("Claude: ", Color::Blue),
                MessageRole::System => ("System: ", Color::Yellow),
                MessageRole::Tool => ("", Color::DarkGray),
            };

            let cursor = if msg.streaming { "█" } else { "" };
            let line = Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(&msg.content),
                Span::styled(cursor, Style::default().fg(Color::White)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let messages =
        List::new(items).block(Block::default().borders(Borders::ALL).title("Conversation"));

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

    // Position cursor
    frame.set_cursor_position((area.x + 1 + app.cursor_pos as u16, area.y + 1));
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
        let app = App::new();

        terminal.draw(|frame| draw(frame, &app)).unwrap();
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

        terminal.draw(|frame| draw(frame, &app)).unwrap();
    }
}
