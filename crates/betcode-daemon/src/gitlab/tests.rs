//! Tests for the GitLab API client and types.

use super::client::{GitLabClient, GitLabConfig, GitLabError};
use super::types::{Issue, MergeRequest, Pipeline};

// =============================================================================
// Client construction tests
// =============================================================================

#[test]
fn empty_base_url_returns_config_error() {
    let config = GitLabConfig {
        base_url: String::new(),
        token: "tok".into(),
    };
    let err = GitLabClient::new(&config).unwrap_err();
    assert!(matches!(err, GitLabError::Config(_)));
}

#[test]
fn empty_token_returns_config_error() {
    let config = GitLabConfig {
        base_url: "https://gitlab.com".into(),
        token: String::new(),
    };
    let err = GitLabClient::new(&config).unwrap_err();
    assert!(matches!(err, GitLabError::Config(_)));
}

#[test]
fn valid_config_creates_client() {
    let config = GitLabConfig {
        base_url: "https://gitlab.com".into(),
        token: "glpat-test-token".into(),
    };
    assert!(GitLabClient::new(&config).is_ok());
}

#[test]
fn trailing_slash_stripped_from_base_url() {
    let config = GitLabConfig {
        base_url: "https://gitlab.com/".into(),
        token: "glpat-test".into(),
    };
    let client = GitLabClient::new(&config).unwrap();
    let url = client.api_url("/projects/1/merge_requests");
    assert!(url.starts_with("https://gitlab.com/api/v4"));
    assert!(!url.contains("//api"));
}

#[test]
fn api_url_constructed_correctly() {
    let config = GitLabConfig {
        base_url: "https://gitlab.com".into(),
        token: "glpat-test".into(),
    };
    let client = GitLabClient::new(&config).unwrap();
    assert_eq!(
        client.api_url("/projects/123/merge_requests"),
        "https://gitlab.com/api/v4/projects/123/merge_requests"
    );
}

#[test]
fn encode_project_handles_slashes() {
    assert_eq!(
        GitLabClient::encode_project("group/project"),
        "group%2Fproject"
    );
    assert_eq!(
        GitLabClient::encode_project("group/sub/project"),
        "group%2Fsub%2Fproject"
    );
}

#[test]
fn encode_project_no_slash_unchanged() {
    assert_eq!(GitLabClient::encode_project("12345"), "12345");
}

// =============================================================================
// Deserialization tests (MergeRequest)
// =============================================================================

#[test]
fn deserialize_merge_request_full() {
    let json = r#"{
        "id": 1,
        "iid": 42,
        "title": "Fix bug",
        "description": "Fixes the thing",
        "state": "opened",
        "source_branch": "fix/bug",
        "target_branch": "main",
        "author": {"username": "alice"},
        "labels": ["bug"],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-02T00:00:00Z",
        "web_url": "https://gitlab.com/g/p/-/merge_requests/42",
        "draft": false,
        "merge_status": "can_be_merged",
        "assignee": {"username": "bob"},
        "assignees": [{"username": "bob"}],
        "reviewers": [{"username": "carol"}],
        "milestone": {"title": "v1.0"}
    }"#;
    let mr: MergeRequest = serde_json::from_str(json).unwrap();
    assert_eq!(mr.iid, 42);
    assert_eq!(mr.title, "Fix bug");
    assert_eq!(mr.state, "opened");
    assert_eq!(mr.author.username, "alice");
    assert_eq!(mr.assignee.unwrap().username, "bob");
    assert_eq!(mr.reviewers.len(), 1);
    assert_eq!(mr.milestone.unwrap().title, "v1.0");
    assert_eq!(mr.merge_status.unwrap(), "can_be_merged");
}

#[test]
fn deserialize_merge_request_minimal() {
    let json = r#"{
        "id": 1,
        "iid": 1,
        "title": "MR",
        "state": "opened",
        "source_branch": "a",
        "target_branch": "b",
        "author": {"username": "u"},
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "web_url": "https://x.com/mr/1"
    }"#;
    let mr: MergeRequest = serde_json::from_str(json).unwrap();
    assert_eq!(mr.iid, 1);
    assert!(mr.description.is_none());
    assert!(mr.assignee.is_none());
    assert!(mr.reviewers.is_empty());
    assert!(mr.milestone.is_none());
    assert!(mr.merge_status.is_none());
    assert!(!mr.draft);
}

// =============================================================================
// Deserialization tests (Pipeline)
// =============================================================================

#[test]
fn deserialize_pipeline() {
    let json = r#"{
        "id": 100,
        "status": "success",
        "ref": "main",
        "sha": "abc123",
        "source": "push",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "web_url": "https://gitlab.com/g/p/-/pipelines/100"
    }"#;
    let p: Pipeline = serde_json::from_str(json).unwrap();
    assert_eq!(p.id, 100);
    assert_eq!(p.status, "success");
    assert_eq!(p.ref_name, "main");
    assert_eq!(p.sha, "abc123");
    assert_eq!(p.source.unwrap(), "push");
}

#[test]
fn deserialize_pipeline_minimal() {
    let json = r#"{
        "id": 1,
        "status": "pending",
        "ref": "dev",
        "sha": "def456",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "web_url": "https://x.com/p/1"
    }"#;
    let p: Pipeline = serde_json::from_str(json).unwrap();
    assert!(p.source.is_none());
}

// =============================================================================
// Deserialization tests (Issue)
// =============================================================================

#[test]
fn deserialize_issue_full() {
    let json = r#"{
        "id": 200,
        "iid": 10,
        "title": "Bug report",
        "description": "Something broke",
        "state": "opened",
        "author": {"username": "alice"},
        "labels": ["bug", "urgent"],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "web_url": "https://gitlab.com/g/p/-/issues/10",
        "confidential": true,
        "assignees": [{"username": "bob"}, {"username": "carol"}],
        "milestone": {"title": "Sprint 5"}
    }"#;
    let issue: Issue = serde_json::from_str(json).unwrap();
    assert_eq!(issue.iid, 10);
    assert!(issue.confidential);
    assert_eq!(issue.assignees.len(), 2);
    assert_eq!(issue.milestone.unwrap().title, "Sprint 5");
}

#[test]
fn deserialize_issue_minimal() {
    let json = r#"{
        "id": 1,
        "iid": 1,
        "title": "Issue",
        "state": "opened",
        "author": {"username": "u"},
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "web_url": "https://x.com/issue/1"
    }"#;
    let issue: Issue = serde_json::from_str(json).unwrap();
    assert!(issue.description.is_none());
    assert!(!issue.confidential);
    assert!(issue.assignees.is_empty());
    assert!(issue.milestone.is_none());
}

// =============================================================================
// Error display tests
// =============================================================================

#[test]
fn gitlab_error_display_api() {
    let err = GitLabError::Api {
        status: 404,
        message: "Not Found".into(),
    };
    assert_eq!(err.to_string(), "GitLab API error (404): Not Found");
}

#[test]
fn gitlab_error_display_config() {
    let err = GitLabError::Config("bad".into());
    assert_eq!(err.to_string(), "Configuration error: bad");
}
