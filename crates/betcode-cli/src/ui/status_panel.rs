//! Session status panel overlay (Ctrl+I).
//!
//! Renders a centered bordered panel showing session diagnostics:
//! working directory, session ID, connection type, model, agents, etc.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Information displayed in the session status panel.
#[derive(Debug, Clone)]
pub struct SessionStatusInfo {
    pub cwd: String,
    pub session_id: String,
    pub connection: String,
    pub model: String,
    pub active_agents: usize,
    pub pending_permissions: usize,
    pub worktree: Option<String>,
    pub uptime_secs: u64,
}

/// Render the session status panel as a centered overlay.
pub fn render_status_panel(frame: &mut Frame, area: Rect, info: &SessionStatusInfo) {
    let panel_width = 50u16.min(area.width.saturating_sub(4));
    let panel_height = 14u16.min(area.height.saturating_sub(2));

    let panel_area = centered_rect(panel_width, panel_height, area);

    // Clear the area behind the panel
    frame.render_widget(Clear, panel_area);

    let label_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let value_style = Style::default().fg(Color::White);

    let agents_str = info.active_agents.to_string();
    let perms_str = info.pending_permissions.to_string();
    let uptime_str = format_uptime(info.uptime_secs);

    let mut lines = vec![
        labeled_line("CWD:", &info.cwd, label_style, value_style),
        labeled_line("Session:", &info.session_id, label_style, value_style),
        labeled_line("Connection:", &info.connection, label_style, value_style),
        labeled_line("Model:", &info.model, label_style, value_style),
        labeled_line("Active Agents:", &agents_str, label_style, value_style),
        labeled_line("Pending Perms:", &perms_str, label_style, value_style),
    ];

    if let Some(ref wt) = info.worktree {
        lines.push(labeled_line("Worktree:", wt, label_style, value_style));
    }

    lines.push(labeled_line(
        "Uptime:",
        &uptime_str,
        label_style,
        value_style,
    ));

    let panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Session Status")
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(panel, panel_area);
}

fn labeled_line<'a>(
    label: &'a str,
    value: &'a str,
    label_style: Style,
    value_style: Style,
) -> Line<'a> {
    Line::from(vec![
        Span::styled(label, label_style),
        Span::raw(" "),
        Span::styled(value, value_style),
    ])
}

fn format_uptime(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{}h {}m {}s", hours, minutes, seconds)
}

/// Compute a centered rectangle within the given area.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    horizontal[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_status_panel_render() {
        let info = SessionStatusInfo {
            cwd: "/home/user/project".to_string(),
            session_id: "sess-abc123".to_string(),
            connection: "local".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            active_agents: 2,
            pending_permissions: 0,
            worktree: Some("feature/auth".to_string()),
            uptime_secs: 3600,
        };
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_status_panel(f, f.area(), &info);
            })
            .unwrap();
        // Just verify it doesn't panic - detailed content checking is brittle
    }

    #[test]
    fn test_format_uptime() {
        assert_eq!(format_uptime(0), "0h 0m 0s");
        assert_eq!(format_uptime(61), "0h 1m 1s");
        assert_eq!(format_uptime(3600), "1h 0m 0s");
        assert_eq!(format_uptime(3661), "1h 1m 1s");
    }

    #[test]
    fn test_status_panel_without_worktree() {
        let info = SessionStatusInfo {
            cwd: "/tmp".to_string(),
            session_id: "s1".to_string(),
            connection: "relay".to_string(),
            model: "claude-opus-4".to_string(),
            active_agents: 0,
            pending_permissions: 1,
            worktree: None,
            uptime_secs: 120,
        };
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_status_panel(f, f.area(), &info);
            })
            .unwrap();
    }
}
