//! Tests for GitLab gRPC service conversions and helpers.

use super::gitlab_convert::*;
use crate::gitlab;
use betcode_proto::v1::{IssueState, MergeRequestState, MergeStatus, PipelineStatus};

// =============================================================================
// Timestamp parsing
// =============================================================================

#[test]
fn parse_timestamp_iso8601_z() {
    let ts = parse_timestamp("2026-01-15T10:30:00Z").unwrap();
    assert!(ts.seconds > 0);
    assert_eq!(ts.nanos, 0);
}

#[test]
fn parse_timestamp_with_fractional() {
    let ts = parse_timestamp("2026-01-15T10:30:00.123Z").unwrap();
    assert!(ts.seconds > 0);
}

#[test]
fn parse_timestamp_invalid_returns_none() {
    assert!(parse_timestamp("not-a-date").is_none());
    assert!(parse_timestamp("").is_none());
}

// =============================================================================
// Enum roundtrips
// =============================================================================

#[test]
fn mr_state_roundtrip() {
    assert_eq!(mr_state_to_str(MergeRequestState::Opened), Some("opened"));
    assert_eq!(mr_state_to_str(MergeRequestState::Unspecified), None);
    assert_eq!(str_to_mr_state("opened"), MergeRequestState::Opened as i32);
    assert_eq!(
        str_to_mr_state("unknown"),
        MergeRequestState::Unspecified as i32
    );
}

#[test]
fn pipeline_status_roundtrip() {
    assert_eq!(
        pipeline_status_to_str(PipelineStatus::Success),
        Some("success")
    );
    assert_eq!(pipeline_status_to_str(PipelineStatus::Unspecified), None);
    assert_eq!(
        str_to_pipeline_status("running"),
        PipelineStatus::Running as i32
    );
}

#[test]
fn issue_state_roundtrip() {
    assert_eq!(issue_state_to_str(IssueState::Opened), Some("opened"));
    assert_eq!(issue_state_to_str(IssueState::Unspecified), None);
    assert_eq!(str_to_issue_state("closed"), IssueState::Closed as i32);
}

#[test]
fn merge_status_mapping() {
    assert_eq!(
        str_to_merge_status("can_be_merged"),
        MergeStatus::CanBeMerged as i32
    );
    assert_eq!(
        str_to_merge_status("cannot_be_merged"),
        MergeStatus::CannotBeMerged as i32
    );
    assert_eq!(
        str_to_merge_status("unknown"),
        MergeStatus::Unspecified as i32
    );
}

// =============================================================================
// Error mapping
// =============================================================================

#[test]
fn to_status_maps_401_to_unauthenticated() {
    let err = gitlab::GitLabError::Api {
        status: 401,
        message: "Unauthorized".into(),
    };
    assert_eq!(to_status(err).code(), tonic::Code::Unauthenticated);
}

#[test]
fn to_status_maps_403_to_permission_denied() {
    let err = gitlab::GitLabError::Api {
        status: 403,
        message: "Forbidden".into(),
    };
    assert_eq!(to_status(err).code(), tonic::Code::PermissionDenied);
}

#[test]
fn to_status_maps_404_to_not_found() {
    let err = gitlab::GitLabError::Api {
        status: 404,
        message: "Not Found".into(),
    };
    assert_eq!(to_status(err).code(), tonic::Code::NotFound);
}

#[test]
fn to_status_maps_config_to_failed_precondition() {
    let err = gitlab::GitLabError::Config("missing".into());
    assert_eq!(to_status(err).code(), tonic::Code::FailedPrecondition);
}

// =============================================================================
// Type conversions
// =============================================================================

#[test]
fn to_mr_info_converts_full_mr() {
    let mr = gitlab::MergeRequest {
        id: 1,
        iid: 42,
        title: "Test".into(),
        description: Some("desc".into()),
        state: "opened".into(),
        source_branch: "feat".into(),
        target_branch: "main".into(),
        author: gitlab::types::GitLabUser {
            username: "alice".into(),
        },
        labels: vec!["bug".into()],
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-02T00:00:00Z".into(),
        web_url: "https://x.com/mr/42".into(),
        draft: true,
        merge_status: Some("can_be_merged".into()),
        assignee: Some(gitlab::types::GitLabUser {
            username: "bob".into(),
        }),
        assignees: vec![gitlab::types::GitLabUser {
            username: "bob".into(),
        }],
        reviewers: vec![gitlab::types::GitLabUser {
            username: "carol".into(),
        }],
        milestone: Some(gitlab::types::GitLabMilestone { title: "v1".into() }),
    };
    let info = to_mr_info(mr);
    assert_eq!(info.iid, 42);
    assert_eq!(info.author, "alice");
    assert_eq!(info.assignee, "bob");
    assert_eq!(info.reviewers, vec!["carol"]);
    assert_eq!(info.milestone, "v1");
    assert!(info.draft);
    assert_eq!(info.merge_status, MergeStatus::CanBeMerged as i32);
}

#[test]
fn to_pipeline_info_converts() {
    let p = gitlab::Pipeline {
        id: 100,
        status: "success".into(),
        ref_name: "main".into(),
        sha: "abc".into(),
        source: Some("push".into()),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        web_url: "https://x.com/p/100".into(),
    };
    let info = to_pipeline_info(p);
    assert_eq!(info.id, 100);
    assert_eq!(info.status, PipelineStatus::Success as i32);
    assert_eq!(info.source, "push");
}

#[test]
fn to_issue_info_converts() {
    let issue = gitlab::Issue {
        id: 200,
        iid: 10,
        title: "Bug".into(),
        description: Some("desc".into()),
        state: "opened".into(),
        author: gitlab::types::GitLabUser {
            username: "alice".into(),
        },
        labels: vec!["bug".into()],
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        web_url: "https://x.com/i/10".into(),
        confidential: true,
        assignee: None,
        assignees: vec![gitlab::types::GitLabUser {
            username: "bob".into(),
        }],
        milestone: Some(gitlab::types::GitLabMilestone {
            title: "Sprint 5".into(),
        }),
    };
    let info = to_issue_info(issue);
    assert_eq!(info.iid, 10);
    assert!(info.confidential);
    assert_eq!(info.assignees, vec!["bob"]);
    assert_eq!(info.milestone, "Sprint 5");
}
