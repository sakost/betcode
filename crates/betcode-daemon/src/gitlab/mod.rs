//! GitLab API integration.
//!
//! Provides a reqwest-based client for the GitLab REST API v4,
//! covering merge requests, pipelines, and issues.

mod client;
pub mod types;

#[cfg(test)]
mod tests;

pub use client::{GitLabClient, GitLabConfig, GitLabError};
pub use types::{Issue, MergeRequest, Pipeline};
