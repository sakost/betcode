//! GitLab output formatting helpers.

use std::io::{self, Write};

use betcode_proto::v1::{
    IssueState, MergeRequestInfo, MergeRequestState, MergeStatus, PipelineInfo, PipelineStatus,
};

pub fn write_mr_detail(w: &mut impl Write, mr: &MergeRequestInfo) -> io::Result<()> {
    writeln!(w, "  MR:       !{}", mr.iid)?;
    writeln!(w, "  Title:    {}", mr.title)?;
    writeln!(w, "  State:    {}", mr_state_str(mr.state))?;
    writeln!(w, "  Author:   {}", mr.author)?;
    writeln!(w, "  Source:   {}", mr.source_branch)?;
    writeln!(w, "  Target:   {}", mr.target_branch)?;
    if mr.draft {
        writeln!(w, "  Draft:    yes")?;
    }
    let ms = MergeStatus::try_from(mr.merge_status).unwrap_or(MergeStatus::Unspecified);
    if ms != MergeStatus::Unspecified {
        writeln!(w, "  Merge:    {:?}", ms)?;
    }
    if !mr.assignee.is_empty() {
        writeln!(w, "  Assignee: {}", mr.assignee)?;
    }
    if !mr.reviewers.is_empty() {
        writeln!(w, "  Review:   {}", mr.reviewers.join(", "))?;
    }
    if !mr.labels.is_empty() {
        writeln!(w, "  Labels:   {}", mr.labels.join(", "))?;
    }
    if !mr.milestone.is_empty() {
        writeln!(w, "  Mile:     {}", mr.milestone)?;
    }
    writeln!(w, "  URL:      {}", mr.web_url)?;
    Ok(())
}

pub fn write_pipeline_detail(w: &mut impl Write, p: &PipelineInfo) -> io::Result<()> {
    writeln!(w, "  ID:       {}", p.id)?;
    writeln!(w, "  Status:   {}", pipeline_status_str(p.status))?;
    writeln!(w, "  Ref:      {}", p.ref_name)?;
    writeln!(w, "  SHA:      {}", p.sha)?;
    if !p.source.is_empty() {
        writeln!(w, "  Source:   {}", p.source)?;
    }
    writeln!(w, "  URL:      {}", p.web_url)?;
    Ok(())
}

pub fn write_issue_detail(
    w: &mut impl Write,
    issue: &betcode_proto::v1::IssueInfo,
) -> io::Result<()> {
    writeln!(w, "  Issue:    #{}", issue.iid)?;
    writeln!(w, "  Title:    {}", issue.title)?;
    writeln!(w, "  State:    {}", issue_state_str(issue.state))?;
    writeln!(w, "  Author:   {}", issue.author)?;
    if issue.confidential {
        writeln!(w, "  Confid:   yes")?;
    }
    if !issue.assignee.is_empty() {
        writeln!(w, "  Assignee: {}", issue.assignee)?;
    }
    if !issue.labels.is_empty() {
        writeln!(w, "  Labels:   {}", issue.labels.join(", "))?;
    }
    if !issue.milestone.is_empty() {
        writeln!(w, "  Mile:     {}", issue.milestone)?;
    }
    writeln!(w, "  URL:      {}", issue.web_url)?;
    Ok(())
}

pub fn parse_mr_state(s: Option<&str>) -> i32 {
    match s {
        Some("opened") => MergeRequestState::Opened as i32,
        Some("closed") => MergeRequestState::Closed as i32,
        Some("merged") => MergeRequestState::Merged as i32,
        Some("locked") => MergeRequestState::Locked as i32,
        _ => MergeRequestState::Unspecified as i32,
    }
}

pub fn parse_pipeline_status(s: Option<&str>) -> i32 {
    match s {
        Some("running") => PipelineStatus::Running as i32,
        Some("success") => PipelineStatus::Success as i32,
        Some("failed") => PipelineStatus::Failed as i32,
        Some("canceled") => PipelineStatus::Canceled as i32,
        Some("pending") => PipelineStatus::Pending as i32,
        _ => PipelineStatus::Unspecified as i32,
    }
}

pub fn parse_issue_state(s: Option<&str>) -> i32 {
    match s {
        Some("opened") => IssueState::Opened as i32,
        Some("closed") => IssueState::Closed as i32,
        _ => IssueState::Unspecified as i32,
    }
}

pub fn mr_state_str(state: i32) -> &'static str {
    match MergeRequestState::try_from(state) {
        Ok(MergeRequestState::Opened) => "opened",
        Ok(MergeRequestState::Closed) => "closed",
        Ok(MergeRequestState::Merged) => "merged",
        Ok(MergeRequestState::Locked) => "locked",
        _ => "unknown",
    }
}

pub fn pipeline_status_str(status: i32) -> &'static str {
    match PipelineStatus::try_from(status) {
        Ok(PipelineStatus::Created) => "created",
        Ok(PipelineStatus::WaitingForResource) => "waiting",
        Ok(PipelineStatus::Preparing) => "preparing",
        Ok(PipelineStatus::Pending) => "pending",
        Ok(PipelineStatus::Running) => "running",
        Ok(PipelineStatus::Success) => "success",
        Ok(PipelineStatus::Failed) => "failed",
        Ok(PipelineStatus::Canceled) => "canceled",
        Ok(PipelineStatus::Skipped) => "skipped",
        Ok(PipelineStatus::Manual) => "manual",
        Ok(PipelineStatus::Scheduled) => "scheduled",
        _ => "unknown",
    }
}

pub fn issue_state_str(state: i32) -> &'static str {
    match IssueState::try_from(state) {
        Ok(IssueState::Opened) => "opened",
        Ok(IssueState::Closed) => "closed",
        _ => "unknown",
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}
