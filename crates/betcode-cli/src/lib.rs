//! `BetCode` CLI Library
//!
//! Terminal interface for interacting with Claude Code through the daemon.
//! Provides both TUI (ratatui) and headless modes.

pub mod app;
pub mod auth_cmd;
pub mod commands;
pub mod completion;
pub mod config;
pub mod connection;
pub mod daemon_cmd;
pub mod gitlab_cmd;
pub mod gitlab_fmt;
pub mod headless;
pub mod machine_cmd;
pub mod relay;
pub mod repo_cmd;
pub mod subagent_cmd;
pub mod tui;
pub mod ui;
pub mod worktree_cmd;
