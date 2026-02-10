use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

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
    pub fn new(cwd: PathBuf) -> Self {
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

    /// Executes a bash command, streaming stdout/stderr line-by-line via the channel.
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

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

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
            if let ServiceOutput::Stdout(line) = output {
                if line.contains("hello") {
                    found_hello = true;
                }
            }
        }
        assert!(found_hello);
    }
}
