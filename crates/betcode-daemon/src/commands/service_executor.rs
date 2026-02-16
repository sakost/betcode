use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::CommandRegistry;
use super::cc_discovery::discover_all_cc_commands;

/// Output messages from command execution.
#[derive(Debug)]
pub enum ServiceOutput {
    Stdout(String),
    Stderr(String),
    ExitCode(i32),
    Error(String),
}

/// Executes service commands (cd, pwd, bash) with a tracked working directory.
pub struct ServiceExecutor {
    cwd: PathBuf,
}

impl ServiceExecutor {
    pub const fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Returns the current working directory as a string.
    pub fn execute_pwd(&self) -> Result<String> {
        Ok(self.cwd.display().to_string())
    }

    /// Changes the working directory, resolving relative paths against current cwd.
    pub fn execute_cd(&mut self, path: &str) -> Result<()> {
        let target = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };

        let resolved = target
            .canonicalize()
            .with_context(|| format!("Failed to resolve path: {}", target.display()))?;

        if !resolved.is_dir() {
            bail!("Not a directory: {}", resolved.display());
        }

        self.cwd = resolved;
        Ok(())
    }

    /// Reloads the command registry by re-discovering Claude Code commands and plugins.
    ///
    /// Clears existing CC-sourced and plugin commands, re-runs discovery, and adds
    /// the fresh commands back into the registry. Plugin entries are scoped to the
    /// given `session_id`.
    pub async fn execute_reload_remote(
        &self,
        registry: &mut CommandRegistry,
        session_id: &str,
    ) -> Result<String> {
        // Clear existing CC commands
        registry.clear_source("claude-code");
        registry.clear_source("user");

        // Re-discover commands
        let result = discover_all_cc_commands(&self.cwd, None);

        let count = result.commands.len();
        for cmd in result.commands {
            registry.add(cmd);
        }

        // Re-discover plugin commands from the session's working directory
        let claude_dir = self.cwd.join(".claude");
        let plugin_entries = tokio::task::spawn_blocking(move || {
            betcode_core::commands::discover_plugin_entries(&claude_dir)
        })
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(error = %err, "Plugin discovery task failed");
            Vec::new()
        });
        let plugin_count = plugin_entries.len();
        registry.update_session_plugin_entries(session_id, plugin_entries);

        let mut msg = format!("Reloaded {count} commands, {plugin_count} plugin entries");
        if !result.warnings.is_empty() {
            use std::fmt::Write;
            let _ = write!(msg, " ({} warnings)", result.warnings.len());
        }
        Ok(msg)
    }

    /// Executes a bash command, streaming stdout/stderr line-by-line via the channel.
    ///
    /// # Panics
    ///
    /// Panics if the child process stdout or stderr cannot be captured (should
    /// never happen because both are configured as `Stdio::piped()`).
    #[allow(clippy::expect_used)]
    pub async fn execute_bash(
        &self,
        cmd: &str,
        output_tx: mpsc::Sender<ServiceOutput>,
    ) -> Result<()> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn shell process")?;

        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let tx_out = output_tx.clone();
        let stdout_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_out.send(ServiceOutput::Stdout(line)).await;
            }
        });

        let tx_err = output_tx.clone();
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_err.send(ServiceOutput::Stderr(line)).await;
            }
        });

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        let status = child.wait().await.context("Failed to wait for process")?;
        let _ = output_tx
            .send(ServiceOutput::ExitCode(status.code().unwrap_or(-1)))
            .await;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_execute_pwd() {
        let dir = TempDir::new().unwrap();
        let executor = ServiceExecutor::new(dir.path().to_path_buf());
        let result = executor.execute_pwd();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_cd_valid() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut executor = ServiceExecutor::new(dir.path().to_path_buf());
        let result = executor.execute_cd("sub");
        assert!(result.is_ok());
        assert_eq!(executor.cwd(), sub.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn test_execute_cd_invalid() {
        let dir = TempDir::new().unwrap();
        let mut executor = ServiceExecutor::new(dir.path().to_path_buf());
        let result = executor.execute_cd("nonexistent");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_bash() {
        let dir = TempDir::new().unwrap();
        let executor = ServiceExecutor::new(dir.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        executor.execute_bash("echo hello", tx).await.unwrap();
        let mut found_hello = false;
        while let Some(output) = rx.recv().await {
            if let ServiceOutput::Stdout(line) = output
                && line.contains("hello")
            {
                found_hello = true;
            }
        }
        assert!(found_hello);
    }

    #[tokio::test]
    async fn test_execute_reload_remote() {
        let dir = TempDir::new().unwrap();
        // Create a user command file to be discovered
        let commands_dir = dir.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(commands_dir.join("deploy.md"), "# Deploy").unwrap();

        let executor = ServiceExecutor::new(dir.path().to_path_buf());
        let mut registry = CommandRegistry::new();

        let msg = executor
            .execute_reload_remote(&mut registry, "test-session")
            .await
            .unwrap();
        assert!(msg.contains("Reloaded"));

        // Should have CC commands + user commands + builtins
        let all = registry.get_all();
        assert!(all.iter().any(|c| c.name == "deploy"));
        assert!(all.iter().any(|c| c.name == "help"));
        // Builtins should still be there
        assert!(all.iter().any(|c| c.name == "cd"));
    }
}
