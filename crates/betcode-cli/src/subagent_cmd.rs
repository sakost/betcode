//! CLI subagent subcommands.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use clap::Subcommand;

use crate::connection::DaemonConnection;

/// Subagent subcommand actions.
#[derive(Subcommand, Debug)]
pub enum SubagentAction {
    /// Spawn a new subagent
    Spawn {
        /// Parent session ID
        #[arg(short, long)]
        session: String,
        /// Prompt to send to the subagent
        #[arg(short, long)]
        prompt: String,
        /// Model to use (e.g., "claude-sonnet-4")
        #[arg(short, long)]
        model: Option<String>,
        /// Maximum turns before auto-stopping
        #[arg(long)]
        max_turns: Option<i32>,
    },
    /// List subagents for a session
    List {
        /// Filter by parent session ID
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Cancel a running subagent
    Cancel {
        /// Subagent ID to cancel
        id: String,
    },
    /// Watch a subagent's event stream
    Watch {
        /// Subagent ID to watch
        id: String,
    },
}

/// Execute a subagent subcommand.
pub async fn run(conn: &mut DaemonConnection, action: SubagentAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        SubagentAction::Spawn {
            session,
            prompt,
            model,
            max_turns,
        } => {
            let resp = conn
                .spawn_subagent(&session, &prompt, model.as_deref(), max_turns.unwrap_or(0))
                .await?;
            writeln!(out, "Spawned subagent:")?;
            writeln!(out, "  Subagent ID: {}", resp.subagent_id)?;
            writeln!(out, "  Session ID:  {}", resp.session_id)?;
        }
        SubagentAction::List { session } => {
            let resp = conn.list_subagents(session.as_deref()).await?;
            if resp.subagents.is_empty() {
                writeln!(out, "No subagents found.")?;
            } else {
                writeln!(out, "{:<36}  {:<16}  {:<12}  MODEL", "ID", "NAME", "STATUS")?;
                for sa in &resp.subagents {
                    writeln!(
                        out,
                        "{:<36}  {:<16}  {:<12}  {}",
                        sa.id,
                        truncate(&sa.name, 16),
                        format_status(sa.status),
                        sa.model,
                    )?;
                }
                writeln!(out, "\n{} subagent(s)", resp.subagents.len())?;
            }
        }
        SubagentAction::Cancel { id } => {
            let resp = conn.cancel_subagent(&id).await?;
            if resp.cancelled {
                writeln!(out, "Subagent {id} cancelled.")?;
                writeln!(out, "  Final status: {}", resp.final_status)?;
            } else {
                writeln!(
                    out,
                    "Subagent {id} could not be cancelled (may have already finished)."
                )?;
            }
        }
        SubagentAction::Watch { id } => {
            writeln!(out, "Watching subagent {id}...")?;
            let mut stream = conn.watch_subagent(&id).await?;
            while let Some(event) = stream
                .message()
                .await
                .map_err(|e| anyhow::anyhow!("Stream error: {e}"))?
            {
                write_event(&mut out, &event)?;
            }
            writeln!(out, "Stream ended.")?;
        }
    }
    Ok(())
}

/// Truncate a string to a maximum display width.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

/// Format a `SubagentStatus` enum value for display.
fn format_status(status: i32) -> &'static str {
    use betcode_proto::v1::SubagentStatus;
    match SubagentStatus::try_from(status) {
        Ok(SubagentStatus::Pending) => "pending",
        Ok(SubagentStatus::Running) => "running",
        Ok(SubagentStatus::Completed) => "completed",
        Ok(SubagentStatus::Failed) => "failed",
        Ok(SubagentStatus::Cancelled) => "cancelled",
        Ok(SubagentStatus::Unspecified) | Err(_) => "unknown",
    }
}

/// Write a subagent event to the output.
fn write_event(w: &mut impl Write, event: &betcode_proto::v1::SubagentEvent) -> io::Result<()> {
    use betcode_proto::v1::subagent_event::Event;
    match &event.event {
        Some(Event::Started(s)) => {
            writeln!(w, "[started] session={} model={}", s.session_id, s.model)?;
        }
        Some(Event::Output(o)) => {
            write!(w, "{}", o.text)?;
            if o.is_complete {
                writeln!(w)?;
            }
        }
        Some(Event::ToolUse(t)) => {
            writeln!(w, "[tool] {} - {}", t.tool_name, t.description)?;
        }
        Some(Event::PermissionRequest(p)) => {
            writeln!(
                w,
                "[permission] {} - {} (id: {})",
                p.tool_name, p.description, p.request_id
            )?;
        }
        Some(Event::Completed(c)) => {
            writeln!(
                w,
                "[completed] exit_code={} summary={}",
                c.exit_code, c.result_summary
            )?;
        }
        Some(Event::Failed(f)) => {
            writeln!(
                w,
                "[failed] exit_code={} error={}",
                f.exit_code, f.error_message
            )?;
        }
        Some(Event::Cancelled(c)) => {
            writeln!(w, "[cancelled] reason={}", c.reason)?;
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Test wrapper to parse CLI arguments.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(subcommand)]
        action: SubagentAction,
    }

    #[test]
    fn parse_spawn_command() {
        let cli = TestCli::parse_from([
            "test",
            "spawn",
            "--session",
            "s1",
            "--prompt",
            "do something",
        ]);
        match cli.action {
            SubagentAction::Spawn {
                session,
                prompt,
                model,
                max_turns,
            } => {
                assert_eq!(session, "s1");
                assert_eq!(prompt, "do something");
                assert!(model.is_none());
                assert!(max_turns.is_none());
            }
            other => panic!("Expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn parse_spawn_with_all_options() {
        let cli = TestCli::parse_from([
            "test",
            "spawn",
            "--session",
            "s1",
            "--prompt",
            "write tests",
            "--model",
            "claude-sonnet-4",
            "--max-turns",
            "5",
        ]);
        match cli.action {
            SubagentAction::Spawn {
                session,
                prompt,
                model,
                max_turns,
            } => {
                assert_eq!(session, "s1");
                assert_eq!(prompt, "write tests");
                assert_eq!(model.as_deref(), Some("claude-sonnet-4"));
                assert_eq!(max_turns, Some(5));
            }
            other => panic!("Expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn parse_list_command() {
        let cli = TestCli::parse_from(["test", "list"]);
        match cli.action {
            SubagentAction::List { session } => {
                assert!(session.is_none());
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }

    #[test]
    fn parse_list_with_session() {
        let cli = TestCli::parse_from(["test", "list", "--session", "s1"]);
        match cli.action {
            SubagentAction::List { session } => {
                assert_eq!(session.as_deref(), Some("s1"));
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }

    #[test]
    fn parse_cancel_command() {
        let cli = TestCli::parse_from(["test", "cancel", "sa-123"]);
        match cli.action {
            SubagentAction::Cancel { id } => {
                assert_eq!(id, "sa-123");
            }
            other => panic!("Expected Cancel, got {other:?}"),
        }
    }

    #[test]
    fn parse_watch_command() {
        let cli = TestCli::parse_from(["test", "watch", "sa-456"]);
        match cli.action {
            SubagentAction::Watch { id } => {
                assert_eq!(id, "sa-456");
            }
            other => panic!("Expected Watch, got {other:?}"),
        }
    }

    #[test]
    fn format_status_values() {
        assert_eq!(format_status(0), "unknown");
        assert_eq!(format_status(1), "pending");
        assert_eq!(format_status(2), "running");
        assert_eq!(format_status(3), "completed");
        assert_eq!(format_status(4), "failed");
        assert_eq!(format_status(5), "cancelled");
        assert_eq!(format_status(99), "unknown");
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate("abcdefghij", 6), "abc...");
    }
}
