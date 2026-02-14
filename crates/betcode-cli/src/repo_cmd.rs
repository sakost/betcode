//! CLI repo subcommands.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use clap::Subcommand;

use betcode_proto::v1::{GitRepoDetail, WorktreeMode};

use crate::connection::DaemonConnection;
use crate::gitlab_fmt::truncate;

/// Repo subcommand actions.
#[derive(Subcommand, Debug)]
pub enum RepoAction {
    /// Register a new git repository
    Register {
        /// Absolute path to the git repository root
        path: String,
        /// Display name (defaults to directory name)
        #[arg(long)]
        name: Option<String>,
        /// Worktree mode: global (default), local, custom
        #[arg(long, default_value = "global")]
        mode: String,
        /// Subfolder for local mode (default: .worktree)
        #[arg(long, default_value = ".worktree")]
        local_subfolder: String,
        /// Path for custom mode
        #[arg(long, default_value = "")]
        custom_path: String,
        /// Default setup script for new worktrees
        #[arg(long, default_value = "")]
        setup_script: String,
        /// Disable auto .gitignore for local subfolder
        #[arg(long)]
        no_auto_gitignore: bool,
    },
    /// Unregister a repository
    Unregister {
        /// Repository ID
        id: String,
        /// Also remove worktrees on disk
        #[arg(long)]
        remove_worktrees: bool,
    },
    /// List all registered repositories
    List {
        /// Maximum number of results
        #[arg(short, long, default_value = "0")]
        limit: u32,
        /// Number of results to skip
        #[arg(short, long, default_value = "0")]
        offset: u32,
    },
    /// Get details for a single repository
    Get {
        /// Repository ID
        id: String,
    },
    /// Update repository configuration
    Update {
        /// Repository ID
        id: String,
        /// New display name
        #[arg(long)]
        name: Option<String>,
        /// New worktree mode: global, local, custom
        #[arg(long)]
        mode: Option<String>,
        /// New subfolder for local mode
        #[arg(long)]
        local_subfolder: Option<String>,
        /// New path for custom mode
        #[arg(long)]
        custom_path: Option<String>,
        /// New default setup script
        #[arg(long)]
        setup_script: Option<String>,
        /// Enable auto .gitignore
        #[arg(long, conflicts_with = "no_auto_gitignore")]
        auto_gitignore: bool,
        /// Disable auto .gitignore
        #[arg(long)]
        no_auto_gitignore: bool,
    },
    /// Scan a directory for git repositories
    Scan {
        /// Directory to scan
        path: String,
        /// Maximum scan depth
        #[arg(long, default_value = "2")]
        max_depth: u32,
    },
}

/// Parse a worktree mode string to the proto enum i32 value.
fn parse_worktree_mode(s: &str) -> Result<i32, String> {
    match s {
        "global" => Ok(WorktreeMode::Global as i32),
        "local" => Ok(WorktreeMode::Local as i32),
        "custom" => Ok(WorktreeMode::Custom as i32),
        _ => Err(format!(
            "Invalid mode '{s}': must be 'global', 'local', or 'custom'"
        )),
    }
}

/// Format a `WorktreeMode` i32 for display.
fn worktree_mode_str(mode: i32) -> &'static str {
    match WorktreeMode::try_from(mode) {
        Ok(WorktreeMode::Global | WorktreeMode::Unspecified) => "global",
        Ok(WorktreeMode::Local) => "local",
        Ok(WorktreeMode::Custom) => "custom",
        Err(_) => "unknown",
    }
}

/// Execute a repo subcommand.
#[allow(clippy::too_many_lines)]
pub async fn run(conn: &mut DaemonConnection, action: RepoAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        RepoAction::Register {
            path,
            name,
            mode,
            local_subfolder,
            custom_path,
            setup_script,
            no_auto_gitignore,
        } => {
            let worktree_mode = parse_worktree_mode(&mode).map_err(|e| anyhow::anyhow!(e))?;
            let detail = conn
                .register_repo(
                    &path,
                    name.as_deref().unwrap_or(""),
                    worktree_mode,
                    &local_subfolder,
                    &custom_path,
                    &setup_script,
                    !no_auto_gitignore,
                )
                .await?;
            writeln!(out, "Registered repository:")?;
            write_detail(&mut out, &detail)?;
        }
        RepoAction::Unregister {
            id,
            remove_worktrees,
        } => {
            let resp = conn.unregister_repo(&id, remove_worktrees).await?;
            if resp.removed {
                writeln!(out, "Repository {id} unregistered.")?;
                if resp.worktrees_removed > 0 {
                    writeln!(out, "  Worktrees removed: {}", resp.worktrees_removed)?;
                }
            } else {
                writeln!(out, "Repository {id} not found.")?;
            }
        }
        RepoAction::List { limit, offset } => {
            let resp = conn.list_repos(limit, offset).await?;
            if resp.repos.is_empty() {
                writeln!(out, "No repositories found.")?;
            } else {
                writeln!(
                    out,
                    "{:<36}  {:<20}  {:<8}  {:<6}",
                    "ID", "NAME", "MODE", "WTs"
                )?;
                for repo in &resp.repos {
                    writeln!(
                        out,
                        "{:<36}  {:<20}  {:<8}  {:<6}",
                        repo.id,
                        truncate(&repo.name, 20),
                        worktree_mode_str(repo.worktree_mode),
                        repo.worktree_count,
                    )?;
                }
                if resp.total_count > 0 && resp.total_count as usize != resp.repos.len() {
                    writeln!(
                        out,
                        "\nShowing {} of {} repository(ies)",
                        resp.repos.len(),
                        resp.total_count
                    )?;
                } else {
                    writeln!(out, "\n{} repository(ies)", resp.repos.len())?;
                }
            }
        }
        RepoAction::Get { id } => {
            let detail = conn.get_repo(&id).await?;
            write_detail(&mut out, &detail)?;
        }
        RepoAction::Update {
            id,
            name,
            mode,
            local_subfolder,
            custom_path,
            setup_script,
            auto_gitignore,
            no_auto_gitignore,
        } => {
            let worktree_mode = mode
                .as_deref()
                .map(parse_worktree_mode)
                .transpose()
                .map_err(|e| anyhow::anyhow!(e))?;
            let auto_gitignore_opt = if auto_gitignore {
                Some(true)
            } else if no_auto_gitignore {
                Some(false)
            } else {
                None
            };
            let detail = conn
                .update_repo(
                    &id,
                    name.as_deref(),
                    worktree_mode,
                    local_subfolder.as_deref(),
                    custom_path.as_deref(),
                    setup_script.as_deref(),
                    auto_gitignore_opt,
                )
                .await?;
            writeln!(out, "Updated repository:")?;
            write_detail(&mut out, &detail)?;
        }
        RepoAction::Scan { path, max_depth } => {
            let resp = conn.scan_repos(&path, max_depth).await?;
            if resp.repos.is_empty() {
                writeln!(out, "No repositories found.")?;
            } else {
                writeln!(out, "{:<36}  {:<20}  {:<8}  PATH", "ID", "NAME", "MODE")?;
                for repo in &resp.repos {
                    writeln!(
                        out,
                        "{:<36}  {:<20}  {:<8}  {}",
                        repo.id,
                        truncate(&repo.name, 20),
                        worktree_mode_str(repo.worktree_mode),
                        repo.repo_path,
                    )?;
                }
                writeln!(out, "\n{} repository(ies) found", resp.repos.len())?;
            }
        }
    }
    Ok(())
}

/// Write a repo detail record to the given writer.
fn write_detail(w: &mut impl Write, repo: &GitRepoDetail) -> io::Result<()> {
    writeln!(w, "  ID:             {}", repo.id)?;
    writeln!(w, "  Name:           {}", repo.name)?;
    writeln!(w, "  Path:           {}", repo.repo_path)?;
    writeln!(
        w,
        "  Worktree mode:  {}",
        worktree_mode_str(repo.worktree_mode)
    )?;
    if !repo.local_subfolder.is_empty() {
        writeln!(w, "  Local subfolder: {}", repo.local_subfolder)?;
    }
    if !repo.custom_path.is_empty() {
        writeln!(w, "  Custom path:    {}", repo.custom_path)?;
    }
    if !repo.setup_script.is_empty() {
        writeln!(w, "  Setup script:   {}", repo.setup_script)?;
    }
    writeln!(
        w,
        "  Auto gitignore: {}",
        if repo.auto_gitignore { "yes" } else { "no" }
    )?;
    writeln!(w, "  Worktrees:      {}", repo.worktree_count)?;
    if let Some(ref ts) = repo.created_at {
        writeln!(w, "  Created:        {} (unix)", ts.seconds)?;
    }
    if let Some(ref ts) = repo.last_active {
        writeln!(w, "  Last active:    {} (unix)", ts.seconds)?;
    }
    Ok(())
}
