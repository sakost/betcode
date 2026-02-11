//! Conversion helpers between GitLab API types and proto types.

use betcode_proto::v1::{
    IssueInfo, IssueState, MergeRequestInfo, MergeRequestState, MergeStatus, PipelineInfo,
    PipelineStatus,
};
use tonic::Status;

use crate::gitlab::{self, GitLabError};

/// Map `GitLabError` to tonic Status.
#[allow(clippy::needless_pass_by_value)]
pub fn to_status(err: GitLabError) -> Status {
    match &err {
        GitLabError::Api { status, .. } => match *status {
            401 => Status::unauthenticated(err.to_string()),
            403 => Status::permission_denied(err.to_string()),
            404 => Status::not_found(err.to_string()),
            _ => Status::internal(err.to_string()),
        },
        GitLabError::Config(_) => Status::failed_precondition(err.to_string()),
        GitLabError::Http(_) => Status::unavailable(err.to_string()),
    }
}

/// Parse ISO 8601 timestamp to prost Timestamp.
pub fn parse_timestamp(s: &str) -> Option<prost_types::Timestamp> {
    let secs = parse_rfc3339_seconds(s)?;
    Some(prost_types::Timestamp {
        seconds: secs,
        nanos: 0,
    })
}

/// Minimal RFC 3339 parser to unix seconds (avoids chrono dependency).
fn parse_rfc3339_seconds(s: &str) -> Option<i64> {
    let s = s.trim();
    let t_pos = s.find('T')?;
    let date_part = &s[..t_pos];
    let time_rest = &s[t_pos + 1..];

    let mut date_parts = date_part.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let time_str = time_rest
        .trim_end_matches('Z')
        .split('+')
        .next()?
        .split('-')
        .next()?;
    let time_no_frac = time_str.split('.').next()?;
    let mut time_parts = time_no_frac.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let min: i64 = time_parts.next()?.parse().ok()?;
    let sec: i64 = time_parts.next()?.parse().ok()?;

    let is_leap = |y: i64| -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 };
    let mut days = 0i64;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let month_days = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    for m in 0..(month - 1) as usize {
        days += i64::from(month_days.get(m).copied().unwrap_or(30));
    }
    days += day - 1;

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

// =============================================================================
// Enum string <-> proto conversions
// =============================================================================

pub const fn mr_state_to_str(state: MergeRequestState) -> Option<&'static str> {
    match state {
        MergeRequestState::Unspecified => None,
        MergeRequestState::Opened => Some("opened"),
        MergeRequestState::Closed => Some("closed"),
        MergeRequestState::Merged => Some("merged"),
        MergeRequestState::Locked => Some("locked"),
    }
}

pub const fn pipeline_status_to_str(status: PipelineStatus) -> Option<&'static str> {
    match status {
        PipelineStatus::Unspecified => None,
        PipelineStatus::Created => Some("created"),
        PipelineStatus::WaitingForResource => Some("waiting_for_resource"),
        PipelineStatus::Preparing => Some("preparing"),
        PipelineStatus::Pending => Some("pending"),
        PipelineStatus::Running => Some("running"),
        PipelineStatus::Success => Some("success"),
        PipelineStatus::Failed => Some("failed"),
        PipelineStatus::Canceled => Some("canceled"),
        PipelineStatus::Skipped => Some("skipped"),
        PipelineStatus::Manual => Some("manual"),
        PipelineStatus::Scheduled => Some("scheduled"),
    }
}

pub const fn issue_state_to_str(state: IssueState) -> Option<&'static str> {
    match state {
        IssueState::Unspecified => None,
        IssueState::Opened => Some("opened"),
        IssueState::Closed => Some("closed"),
    }
}

pub fn str_to_mr_state(s: &str) -> i32 {
    match s {
        "opened" => MergeRequestState::Opened as i32,
        "closed" => MergeRequestState::Closed as i32,
        "merged" => MergeRequestState::Merged as i32,
        "locked" => MergeRequestState::Locked as i32,
        _ => MergeRequestState::Unspecified as i32,
    }
}

pub fn str_to_pipeline_status(s: &str) -> i32 {
    match s {
        "created" => PipelineStatus::Created as i32,
        "waiting_for_resource" => PipelineStatus::WaitingForResource as i32,
        "preparing" => PipelineStatus::Preparing as i32,
        "pending" => PipelineStatus::Pending as i32,
        "running" => PipelineStatus::Running as i32,
        "success" => PipelineStatus::Success as i32,
        "failed" => PipelineStatus::Failed as i32,
        "canceled" => PipelineStatus::Canceled as i32,
        "skipped" => PipelineStatus::Skipped as i32,
        "manual" => PipelineStatus::Manual as i32,
        "scheduled" => PipelineStatus::Scheduled as i32,
        _ => PipelineStatus::Unspecified as i32,
    }
}

pub fn str_to_merge_status(s: &str) -> i32 {
    match s {
        "can_be_merged" => MergeStatus::CanBeMerged as i32,
        "cannot_be_merged" => MergeStatus::CannotBeMerged as i32,
        "checking" => MergeStatus::Checking as i32,
        "unchecked" => MergeStatus::Unchecked as i32,
        _ => MergeStatus::Unspecified as i32,
    }
}

pub fn str_to_issue_state(s: &str) -> i32 {
    match s {
        "opened" => IssueState::Opened as i32,
        "closed" => IssueState::Closed as i32,
        _ => IssueState::Unspecified as i32,
    }
}

// =============================================================================
// Type conversions: GitLab REST -> Proto
// =============================================================================

pub fn to_mr_info(mr: gitlab::MergeRequest) -> MergeRequestInfo {
    MergeRequestInfo {
        id: mr.id,
        iid: mr.iid,
        title: mr.title,
        description: mr.description.unwrap_or_default(),
        state: str_to_mr_state(&mr.state),
        source_branch: mr.source_branch,
        target_branch: mr.target_branch,
        author: mr.author.username,
        labels: mr.labels,
        created_at: parse_timestamp(&mr.created_at),
        updated_at: parse_timestamp(&mr.updated_at),
        web_url: mr.web_url,
        draft: mr.draft,
        merge_status: mr
            .merge_status
            .as_deref()
            .map_or(0, str_to_merge_status),
        assignee: mr.assignee.map(|u| u.username).unwrap_or_default(),
        assignees: mr.assignees.into_iter().map(|u| u.username).collect(),
        reviewers: mr.reviewers.into_iter().map(|u| u.username).collect(),
        milestone: mr.milestone.map(|m| m.title).unwrap_or_default(),
    }
}

pub fn to_pipeline_info(p: gitlab::Pipeline) -> PipelineInfo {
    PipelineInfo {
        id: p.id,
        status: str_to_pipeline_status(&p.status),
        ref_name: p.ref_name,
        sha: p.sha,
        source: p.source.unwrap_or_default(),
        created_at: parse_timestamp(&p.created_at),
        updated_at: parse_timestamp(&p.updated_at),
        web_url: p.web_url,
    }
}

pub fn to_issue_info(issue: gitlab::Issue) -> IssueInfo {
    IssueInfo {
        id: issue.id,
        iid: issue.iid,
        title: issue.title,
        description: issue.description.unwrap_or_default(),
        state: str_to_issue_state(&issue.state),
        author: issue.author.username,
        labels: issue.labels,
        created_at: parse_timestamp(&issue.created_at),
        updated_at: parse_timestamp(&issue.updated_at),
        web_url: issue.web_url,
        confidential: issue.confidential,
        assignee: issue.assignee.map(|u| u.username).unwrap_or_default(),
        assignees: issue.assignees.into_iter().map(|u| u.username).collect(),
        milestone: issue.milestone.map(|m| m.title).unwrap_or_default(),
    }
}
