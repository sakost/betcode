//! GitLab REST API v4 client.
//!
//! Uses reqwest to call GitLab endpoints for merge requests, pipelines, and issues.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use thiserror::Error;

use super::types::{Issue, MergeRequest, Pipeline};

/// GitLab API client errors.
#[derive(Debug, Error)]
pub enum GitLabError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("GitLab API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Configuration error: {0}")]
    Config(String),
}

/// Configuration for connecting to a GitLab instance.
#[derive(Debug, Clone)]
pub struct GitLabConfig {
    /// GitLab instance URL (e.g., "<https://gitlab.com>").
    pub base_url: String,
    /// Personal access token or OAuth token.
    pub token: String,
}

/// GitLab REST API v4 client.
#[derive(Debug)]
pub struct GitLabClient {
    http: reqwest::Client,
    base_url: String,
}

impl GitLabClient {
    /// Create a new GitLab API client.
    pub fn new(config: &GitLabConfig) -> Result<Self, GitLabError> {
        if config.base_url.is_empty() {
            return Err(GitLabError::Config("base_url is empty".into()));
        }
        if config.token.is_empty() {
            return Err(GitLabError::Config("token is empty".into()));
        }

        let mut headers = HeaderMap::new();
        let token_val = HeaderValue::from_str(&format!("Bearer {}", config.token))
            .map_err(|_| GitLabError::Config("Invalid token format".into()))?;
        headers.insert(AUTHORIZATION, token_val);

        // Ensure a TLS crypto provider is installed (reqwest uses rustls-no-provider).
        // The `Err` case just means it was already installed — safe to ignore.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        let base_url = config.base_url.trim_end_matches('/').to_string();
        Ok(Self { http, base_url })
    }

    /// Build the API v4 URL for a given path.
    pub(crate) fn api_url(&self, path: &str) -> String {
        format!("{}/api/v4{}", self.base_url, path)
    }

    /// URL-encode a project path (e.g., "group/project" -> "group%2Fproject").
    pub(crate) fn encode_project(project: &str) -> String {
        project.replace('/', "%2F")
    }

    /// Check HTTP response status, returning error for non-success codes.
    fn check_status(resp: &reqwest::Response) -> Result<(), GitLabError> {
        let status = resp.status();
        if !status.is_success() {
            return Err(GitLabError::Api {
                status: status.as_u16(),
                message: status.canonical_reason().unwrap_or("Unknown").into(),
            });
        }
        Ok(())
    }

    // =========================================================================
    // Merge Requests
    // =========================================================================

    /// List merge requests for a project.
    pub async fn list_merge_requests(
        &self,
        project: &str,
        state: Option<&str>,
        per_page: u32,
        page: u32,
    ) -> Result<Vec<MergeRequest>, GitLabError> {
        let encoded = Self::encode_project(project);
        let mut url = format!(
            "{}?per_page={}&page={}",
            self.api_url(&format!("/projects/{encoded}/merge_requests")),
            per_page,
            page
        );
        if let Some(s) = state {
            use std::fmt::Write;
            let _ = write!(url, "&state={s}");
        }
        let resp = self.http.get(&url).send().await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }

    /// Get a single merge request by IID.
    pub async fn get_merge_request(
        &self,
        project: &str,
        iid: u64,
    ) -> Result<MergeRequest, GitLabError> {
        let encoded = Self::encode_project(project);
        let url = self.api_url(&format!("/projects/{encoded}/merge_requests/{iid}"));
        let resp = self.http.get(&url).send().await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }

    // =========================================================================
    // Pipelines
    // =========================================================================

    /// List pipelines for a project.
    pub async fn list_pipelines(
        &self,
        project: &str,
        status: Option<&str>,
        per_page: u32,
        page: u32,
    ) -> Result<Vec<Pipeline>, GitLabError> {
        let encoded = Self::encode_project(project);
        let mut url = format!(
            "{}?per_page={}&page={}",
            self.api_url(&format!("/projects/{encoded}/pipelines")),
            per_page,
            page
        );
        if let Some(s) = status {
            use std::fmt::Write;
            let _ = write!(url, "&status={s}");
        }
        let resp = self.http.get(&url).send().await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }

    /// Get a single pipeline by ID.
    pub async fn get_pipeline(
        &self,
        project: &str,
        pipeline_id: u64,
    ) -> Result<Pipeline, GitLabError> {
        let encoded = Self::encode_project(project);
        let url = self.api_url(&format!("/projects/{encoded}/pipelines/{pipeline_id}"));
        let resp = self.http.get(&url).send().await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }

    // =========================================================================
    // Issues
    // =========================================================================

    /// List issues for a project.
    pub async fn list_issues(
        &self,
        project: &str,
        state: Option<&str>,
        per_page: u32,
        page: u32,
    ) -> Result<Vec<Issue>, GitLabError> {
        let encoded = Self::encode_project(project);
        let mut url = format!(
            "{}?per_page={}&page={}",
            self.api_url(&format!("/projects/{encoded}/issues")),
            per_page,
            page
        );
        if let Some(s) = state {
            use std::fmt::Write;
            let _ = write!(url, "&state={s}");
        }
        let resp = self.http.get(&url).send().await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }

    /// Get a single issue by IID.
    pub async fn get_issue(&self, project: &str, iid: u64) -> Result<Issue, GitLabError> {
        let encoded = Self::encode_project(project);
        let url = self.api_url(&format!("/projects/{encoded}/issues/{iid}"));
        let resp = self.http.get(&url).send().await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }
}
