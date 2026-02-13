//! CLI worktree subcommands.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use clap::Subcommand;

use crate::connection::DaemonConnection;

/// Worktree subcommand actions.
#[derive(Subcommand, Debug)]
pub enum WorktreeAction {
    /// Create a new worktree
    Create {
        /// Worktree name (used as directory name)
        name: String,
        /// Path to the git repository root
        #[arg(short, long)]
        repo: String,
        /// Branch name to create
        #[arg(short, long)]
        branch: String,
        /// Setup script to run after creation (e.g. "npm install")
        #[arg(long)]
        setup: Option<String>,
    },
    /// List all worktrees
    List {
        /// Filter by repository path
        #[arg(short, long)]
        repo: Option<String>,
    },
    /// Show worktree details
    Get {
        /// Worktree ID
        id: String,
    },
    /// Remove a worktree
    Remove {
        /// Worktree ID
        id: String,
    },
}

/// Execute a worktree subcommand.
pub async fn run(conn: &mut DaemonConnection, action: WorktreeAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        WorktreeAction::Create {
            name,
            repo,
            branch,
            setup,
        } => {
            let detail = conn
                .create_worktree(&name, &repo, &branch, setup.as_deref())
                .await?;
            writeln!(out, "Created worktree:")?;
            write_detail(&mut out, &detail)?;
        }
        WorktreeAction::List { repo } => {
            let resp = conn.list_worktrees(repo.as_deref()).await?;
            if resp.worktrees.is_empty() {
                writeln!(out, "No worktrees found.")?;
            } else {
                writeln!(
                    out,
                    "{:<36}  {:<16}  {:<16}  {:<6}  {:<5}",
                    "ID", "NAME", "BRANCH", "DISK", "SESS"
                )?;
                for wt in &resp.worktrees {
                    writeln!(
                        out,
                        "{:<36}  {:<16}  {:<16}  {:<6}  {:<5}",
                        wt.id,
                        truncate(&wt.name, 16),
                        truncate(&wt.branch, 16),
                        if wt.exists_on_disk { "yes" } else { "no" },
                        wt.session_count,
                    )?;
                }
                writeln!(out, "\n{} worktree(s)", resp.worktrees.len())?;
            }
        }
        WorktreeAction::Get { id } => {
            let detail = conn.get_worktree(&id).await?;
            write_detail(&mut out, &detail)?;
        }
        WorktreeAction::Remove { id } => {
            let resp = conn.remove_worktree(&id).await?;
            if resp.removed {
                writeln!(out, "Worktree {id} removed.")?;
            } else {
                writeln!(out, "Worktree {id} not found.")?;
            }
        }
    }
    Ok(())
}

/// Write a worktree detail record to the given writer.
fn write_detail(w: &mut impl Write, wt: &betcode_proto::v1::WorktreeDetail) -> io::Result<()> {
    writeln!(w, "  ID:       {}", wt.id)?;
    writeln!(w, "  Name:     {}", wt.name)?;
    writeln!(w, "  Branch:   {}", wt.branch)?;
    writeln!(w, "  Path:     {}", wt.path)?;
    writeln!(w, "  Repo ID:  {}", wt.repo_id)?;
    if !wt.setup_script.is_empty() {
        writeln!(w, "  Setup:    {}", wt.setup_script)?;
    }
    writeln!(
        w,
        "  On disk:  {}",
        if wt.exists_on_disk { "yes" } else { "no" }
    )?;
    writeln!(w, "  Sessions: {}", wt.session_count)?;
    if let Some(ref ts) = wt.created_at {
        writeln!(w, "  Created:  {} (unix)", ts.seconds)?;
    }
    if let Some(ref ts) = wt.last_active {
        writeln!(w, "  Active:   {} (unix)", ts.seconds)?;
    }
    Ok(())
}

/// Truncate a string to max display characters with ellipsis.
fn truncate(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}
