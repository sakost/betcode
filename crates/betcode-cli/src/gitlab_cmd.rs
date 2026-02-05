//! GitLab subcommands: mr, pipeline, issue.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use crate::connection::DaemonConnection;
use crate::gitlab_fmt::*;

/// GitLab subcommand actions.
#[derive(clap::Subcommand, Debug)]
pub enum GitLabAction {
    /// Merge request operations.
    Mr {
        #[command(subcommand)]
        action: MrAction,
    },
    /// Pipeline operations.
    Pipeline {
        #[command(subcommand)]
        action: PipelineAction,
    },
    /// Issue operations.
    Issue {
        #[command(subcommand)]
        action: IssueAction,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum MrAction {
    /// List merge requests.
    List {
        /// GitLab project path (e.g., "group/project").
        project: String,
        /// Filter by state: opened, closed, merged, locked.
        #[arg(short, long)]
        state: Option<String>,
        /// Maximum results.
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Get a single merge request.
    Get {
        /// GitLab project path.
        project: String,
        /// Merge request IID.
        iid: u64,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum PipelineAction {
    /// List pipelines.
    List {
        /// GitLab project path.
        project: String,
        /// Filter by status: running, success, failed, etc.
        #[arg(short, long)]
        status: Option<String>,
        /// Maximum results.
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Get a single pipeline.
    Get {
        /// GitLab project path.
        project: String,
        /// Pipeline ID.
        id: u64,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum IssueAction {
    /// List issues.
    List {
        /// GitLab project path.
        project: String,
        /// Filter by state: opened, closed.
        #[arg(short, long)]
        state: Option<String>,
        /// Maximum results.
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Get a single issue.
    Get {
        /// GitLab project path.
        project: String,
        /// Issue IID.
        iid: u64,
    },
}

/// Execute a gitlab subcommand.
pub async fn run(conn: &mut DaemonConnection, action: GitLabAction) -> anyhow::Result<()> {
    match action {
        GitLabAction::Mr { action } => run_mr(conn, action).await,
        GitLabAction::Pipeline { action } => run_pipeline(conn, action).await,
        GitLabAction::Issue { action } => run_issue(conn, action).await,
    }
}

async fn run_mr(conn: &mut DaemonConnection, action: MrAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        MrAction::List {
            project,
            state,
            limit,
        } => {
            let state_filter = parse_mr_state(state.as_deref());
            let resp = conn
                .list_merge_requests(&project, state_filter, limit)
                .await?;
            if resp.merge_requests.is_empty() {
                writeln!(out, "No merge requests found.")?;
            } else {
                writeln!(
                    out,
                    "{:<6} {:<50} {:<10} {:<8}",
                    "IID", "TITLE", "STATE", "AUTHOR"
                )?;
                for mr in &resp.merge_requests {
                    writeln!(
                        out,
                        "{:<6} {:<50} {:<10} {:<8}",
                        format!("!{}", mr.iid),
                        truncate(&mr.title, 50),
                        mr_state_str(mr.state),
                        truncate(&mr.author, 8),
                    )?;
                }
                writeln!(out, "\n{} merge request(s)", resp.merge_requests.len())?;
            }
        }
        MrAction::Get { project, iid } => {
            let resp = conn.get_merge_request(&project, iid).await?;
            match resp.merge_request {
                Some(mr) => write_mr_detail(&mut out, &mr)?,
                None => writeln!(out, "Merge request !{} not found.", iid)?,
            }
        }
    }
    Ok(())
}

async fn run_pipeline(conn: &mut DaemonConnection, action: PipelineAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        PipelineAction::List {
            project,
            status,
            limit,
        } => {
            let status_filter = parse_pipeline_status(status.as_deref());
            let resp = conn.list_pipelines(&project, status_filter, limit).await?;
            if resp.pipelines.is_empty() {
                writeln!(out, "No pipelines found.")?;
            } else {
                writeln!(
                    out,
                    "{:<10} {:<12} {:<20} {:<10}",
                    "ID", "STATUS", "REF", "SHA"
                )?;
                for p in &resp.pipelines {
                    writeln!(
                        out,
                        "{:<10} {:<12} {:<20} {:<10}",
                        p.id,
                        pipeline_status_str(p.status),
                        truncate(&p.ref_name, 20),
                        truncate(&p.sha, 10),
                    )?;
                }
                writeln!(out, "\n{} pipeline(s)", resp.pipelines.len())?;
            }
        }
        PipelineAction::Get { project, id } => {
            let resp = conn.get_pipeline(&project, id).await?;
            match resp.pipeline {
                Some(p) => write_pipeline_detail(&mut out, &p)?,
                None => writeln!(out, "Pipeline {} not found.", id)?,
            }
        }
    }
    Ok(())
}

async fn run_issue(conn: &mut DaemonConnection, action: IssueAction) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match action {
        IssueAction::List {
            project,
            state,
            limit,
        } => {
            let state_filter = parse_issue_state(state.as_deref());
            let resp = conn.list_issues(&project, state_filter, limit).await?;
            if resp.issues.is_empty() {
                writeln!(out, "No issues found.")?;
            } else {
                writeln!(
                    out,
                    "{:<6} {:<50} {:<10} {:<8}",
                    "IID", "TITLE", "STATE", "AUTHOR"
                )?;
                for issue in &resp.issues {
                    writeln!(
                        out,
                        "{:<6} {:<50} {:<10} {:<8}",
                        format!("#{}", issue.iid),
                        truncate(&issue.title, 50),
                        issue_state_str(issue.state),
                        truncate(&issue.author, 8),
                    )?;
                }
                writeln!(out, "\n{} issue(s)", resp.issues.len())?;
            }
        }
        IssueAction::Get { project, iid } => {
            let resp = conn.get_issue(&project, iid).await?;
            match resp.issue {
                Some(issue) => write_issue_detail(&mut out, &issue)?,
                None => writeln!(out, "Issue #{} not found.", iid)?,
            }
        }
    }
    Ok(())
}
