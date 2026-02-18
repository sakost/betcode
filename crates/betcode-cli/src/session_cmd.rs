//! CLI session management subcommands.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use clap::Subcommand;

use crate::connection::DaemonConnection;
use crate::gitlab_fmt::truncate;

/// Session subcommand actions.
#[derive(Subcommand, Debug)]
pub enum SessionAction {
    /// List sessions
    List {
        /// Filter by working directory
        #[arg(short = 'd', long)]
        working_dir: Option<String>,
        /// Filter by worktree ID
        #[arg(short, long)]
        worktree: Option<String>,
        /// Maximum number of sessions to return
        #[arg(short, long, default_value_t = 50)]
        limit: u32,
        /// Offset for pagination
        #[arg(short, long, default_value_t = 0)]
        offset: u32,
    },
    /// Rename a session
    Rename {
        /// Session ID
        id: String,
        /// New name for the session
        name: String,
    },
    /// Delete a session
    Delete {
        /// Session ID
        id: String,
    },
    /// Compact a session (remove redundant messages to save tokens)
    Compact {
        /// Session ID
        id: String,
    },
    /// Cancel the current turn in a session
    Cancel {
        /// Session ID
        id: String,
    },
}

/// Return the display name for a session summary: its name if set, otherwise the model.
fn display_name(s: &betcode_proto::v1::SessionSummary) -> &str {
    if s.name.is_empty() { &s.model } else { &s.name }
}

/// Execute a session subcommand.
pub async fn run(conn: &mut DaemonConnection, action: SessionAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        SessionAction::List {
            working_dir,
            worktree,
            limit,
            offset,
        } => {
            let resp = conn
                .list_sessions_filtered(working_dir.as_deref(), worktree.as_deref(), limit, offset)
                .await?;
            if resp.sessions.is_empty() {
                writeln!(out, "No sessions found.")?;
            } else {
                writeln!(
                    out,
                    "{:<36}  {:<20}  {:<10}  {:>4}  {:>8}  PREVIEW",
                    "ID", "NAME", "STATUS", "MSGS", "COST"
                )?;
                for s in &resp.sessions {
                    writeln!(
                        out,
                        "{:<36}  {:<20}  {:<10}  {:>4}  {:>8.4}  {}",
                        s.id,
                        truncate(display_name(s), 20),
                        truncate(&s.status, 10),
                        s.message_count,
                        s.total_cost_usd,
                        truncate(&s.last_message_preview, 40),
                    )?;
                }
                writeln!(
                    out,
                    "\nShowing {}-{} of {} session(s)",
                    offset.saturating_add(1),
                    offset.saturating_add(u32::try_from(resp.sessions.len()).unwrap_or(u32::MAX),),
                    resp.total,
                )?;
            }
        }
        SessionAction::Rename { id, name } => {
            conn.rename_session(&id, &name).await?;
            writeln!(out, "Session {id} renamed to \"{name}\".")?;
        }
        SessionAction::Delete { id } => {
            let resp = conn.delete_session(&id).await?;
            if resp.deleted {
                writeln!(out, "Session {id} deleted.")?;
            } else {
                writeln!(out, "Session {id} not found.")?;
            }
        }
        SessionAction::Compact { id } => {
            let resp = conn.compact_session(&id).await?;
            writeln!(out, "Session {id} compacted.")?;
            writeln!(out, "  Messages before: {}", resp.messages_before)?;
            writeln!(out, "  Messages after:  {}", resp.messages_after)?;
            writeln!(out, "  Tokens saved:    {}", resp.tokens_saved)?;
        }
        SessionAction::Cancel { id } => {
            let resp = conn.cancel_turn(&id).await?;
            if resp.was_active {
                writeln!(out, "Turn cancelled for session {id}.")?;
            } else {
                writeln!(out, "No active turn in session {id}.")?;
            }
        }
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
        action: SessionAction,
    }

    #[test]
    fn parse_list_defaults() {
        let cli = TestCli::parse_from(["test", "list"]);
        match cli.action {
            SessionAction::List {
                working_dir,
                worktree,
                limit,
                offset,
            } => {
                assert!(working_dir.is_none());
                assert!(worktree.is_none());
                assert_eq!(limit, 50);
                assert_eq!(offset, 0);
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }

    #[test]
    fn parse_list_with_filters() {
        let cli = TestCli::parse_from([
            "test",
            "list",
            "--working-dir",
            "/tmp/project",
            "--worktree",
            "wt-1",
            "--limit",
            "10",
            "--offset",
            "5",
        ]);
        match cli.action {
            SessionAction::List {
                working_dir,
                worktree,
                limit,
                offset,
            } => {
                assert_eq!(working_dir.as_deref(), Some("/tmp/project"));
                assert_eq!(worktree.as_deref(), Some("wt-1"));
                assert_eq!(limit, 10);
                assert_eq!(offset, 5);
            }
            other => panic!("Expected List, got {other:?}"),
        }
    }

    #[test]
    fn parse_rename_command() {
        let cli = TestCli::parse_from(["test", "rename", "sess-1", "my-session"]);
        match cli.action {
            SessionAction::Rename { id, name } => {
                assert_eq!(id, "sess-1");
                assert_eq!(name, "my-session");
            }
            other => panic!("Expected Rename, got {other:?}"),
        }
    }

    #[test]
    fn parse_delete_command() {
        let cli = TestCli::parse_from(["test", "delete", "sess-1"]);
        match cli.action {
            SessionAction::Delete { id } => {
                assert_eq!(id, "sess-1");
            }
            other => panic!("Expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn parse_compact_command() {
        let cli = TestCli::parse_from(["test", "compact", "sess-1"]);
        match cli.action {
            SessionAction::Compact { id } => {
                assert_eq!(id, "sess-1");
            }
            other => panic!("Expected Compact, got {other:?}"),
        }
    }

    #[test]
    fn parse_cancel_command() {
        let cli = TestCli::parse_from(["test", "cancel", "sess-1"]);
        match cli.action {
            SessionAction::Cancel { id } => {
                assert_eq!(id, "sess-1");
            }
            other => panic!("Expected Cancel, got {other:?}"),
        }
    }

    #[test]
    fn display_name_uses_name_when_present() {
        let s = betcode_proto::v1::SessionSummary {
            name: "my-session".to_string(),
            model: "claude-sonnet-4".to_string(),
            ..Default::default()
        };
        assert_eq!(display_name(&s), "my-session");
    }

    #[test]
    fn display_name_falls_back_to_model() {
        let s = betcode_proto::v1::SessionSummary {
            name: String::new(),
            model: "claude-sonnet-4".to_string(),
            ..Default::default()
        };
        assert_eq!(display_name(&s), "claude-sonnet-4");
    }
}
