//! GitLab API v4 response types.
//!
//! Deserialization structs matching GitLab REST API JSON responses.

use serde::Deserialize;

/// GitLab user reference (subset of fields).
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabUser {
    pub username: String,
}

/// GitLab milestone reference (subset of fields).
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabMilestone {
    pub title: String,
}

/// Merge request from GitLab API v4.
#[derive(Debug, Clone, Deserialize)]
pub struct MergeRequest {
    pub id: u64,
    pub iid: u64,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub author: GitLabUser,
    #[serde(default)]
    pub labels: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub web_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merge_status: Option<String>,
    #[serde(default)]
    pub assignee: Option<GitLabUser>,
    #[serde(default)]
    pub assignees: Vec<GitLabUser>,
    #[serde(default)]
    pub reviewers: Vec<GitLabUser>,
    #[serde(default)]
    pub milestone: Option<GitLabMilestone>,
}

/// Pipeline from GitLab API v4.
#[derive(Debug, Clone, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    #[serde(default)]
    pub source: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub web_url: String,
}

/// Issue from GitLab API v4.
#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub id: u64,
    pub iid: u64,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub state: String,
    pub author: GitLabUser,
    #[serde(default)]
    pub labels: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub web_url: String,
    #[serde(default)]
    pub confidential: bool,
    #[serde(default)]
    pub assignee: Option<GitLabUser>,
    #[serde(default)]
    pub assignees: Vec<GitLabUser>,
    #[serde(default)]
    pub milestone: Option<GitLabMilestone>,
}
