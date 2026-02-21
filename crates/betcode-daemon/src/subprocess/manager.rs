//! Claude Code subprocess lifecycle manager.
//!
//! Manages spawning, monitoring, and graceful shutdown of Claude CLI processes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

/// Strategy for handling permission prompts in the subprocess.
#[derive(Debug, Clone, Default)]
pub enum PermissionStrategy {
    /// Use `--permission-prompt-tool stdio` for interactive approval via NDJSON.
    /// Note: This flag is hidden in Claude Code v2.1.6+ and the control protocol
    /// may not emit `control_request` events. Falls back to auto-approve behavior.
    #[default]
    PromptToolStdio,
    /// Use `--allowedTools` to pre-approve specific tools. No runtime prompts.
    AllowedTools(Vec<String>),
    /// Use `--dangerously-skip-permissions` to bypass all checks.
    /// Only safe in sandboxed environments with no internet access.
    SkipPermissions,
}

/// Configuration for subprocess spawning.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    /// Working directory for the Claude process.
    pub working_directory: PathBuf,
    /// Initial prompt (for headless mode).
    pub prompt: Option<String>,
    /// Session ID to resume (if any).
    pub resume_session: Option<String>,
    /// Model to use.
    pub model: Option<String>,
    /// Permission handling strategy.
    pub permission_strategy: PermissionStrategy,
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            working_directory: std::env::current_dir().unwrap_or_default(),
            prompt: None,
            resume_session: None,
            model: None,
            permission_strategy: PermissionStrategy::default(),
        }
    }
}

/// Handle to a running Claude subprocess.
#[derive(Debug)]
pub struct ProcessHandle {
    /// Unique identifier for this process.
    pub id: String,
    /// Session ID from Claude's system.init message.
    pub session_id: Option<String>,
    /// Sender for stdin commands.
    pub stdin_tx: mpsc::Sender<String>,
    /// Working directory.
    pub working_directory: PathBuf,
}

/// Subprocess manager for Claude Code processes.
pub struct SubprocessManager {
    /// Active processes keyed by process ID.
    processes: Arc<RwLock<HashMap<String, ProcessState>>>,
    /// Maximum concurrent processes.
    max_processes: usize,
    /// Path to the `claude` binary.
    claude_bin: PathBuf,
    /// Default permission strategy for new subprocesses.
    default_permission_strategy: PermissionStrategy,
    /// Timeout for graceful subprocess termination before SIGKILL.
    terminate_timeout: std::time::Duration,
}

struct ProcessState {
    child: Child,
    session_id: Option<String>,
    stdin_tx: mpsc::Sender<String>,
    working_directory: PathBuf,
}

impl SubprocessManager {
    /// Create a new subprocess manager.
    pub fn new(max_processes: usize, claude_bin: PathBuf) -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            max_processes,
            claude_bin,
            default_permission_strategy: PermissionStrategy::default(),
            terminate_timeout: std::time::Duration::from_secs(5),
        }
    }

    /// Create a new subprocess manager with full configuration.
    pub fn with_options(
        max_processes: usize,
        claude_bin: PathBuf,
        default_permission_strategy: PermissionStrategy,
        terminate_timeout_secs: u64,
    ) -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            max_processes,
            claude_bin,
            default_permission_strategy,
            terminate_timeout: std::time::Duration::from_secs(terminate_timeout_secs),
        }
    }

    /// Get the default permission strategy.
    pub const fn default_permission_strategy(&self) -> &PermissionStrategy {
        &self.default_permission_strategy
    }

    /// Spawn a new Claude subprocess.
    #[allow(clippy::too_many_lines)]
    pub async fn spawn(
        &self,
        config: SpawnConfig,
        stdout_tx: mpsc::Sender<String>,
    ) -> Result<ProcessHandle, SubprocessError> {
        // Check pool capacity
        let processes = self.processes.read().await;
        if processes.len() >= self.max_processes {
            return Err(SubprocessError::PoolExhausted {
                current: processes.len(),
                max: self.max_processes,
            });
        }
        drop(processes);

        // Build command
        let working_dir = if config.working_directory.as_os_str().is_empty()
            || !config.working_directory.exists()
        {
            let fallback = dirs::home_dir().unwrap_or_else(|| {
                warn!(
                    "dirs::home_dir() returned None; falling back to temp_dir for working directory"
                );
                std::env::temp_dir()
            });
            warn!(
                requested = %config.working_directory.display(),
                fallback = %fallback.display(),
                "Working directory missing or empty, using fallback"
            );
            fallback
        } else {
            config.working_directory.clone()
        };
        let mut cmd = Command::new(&self.claude_bin);
        cmd.current_dir(&working_dir)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--verbose")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Ensure essential env vars are available to the subprocess even
        // when running under systemd with stripped environment.
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", &home);
        }
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", &path);
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            cmd.env("ANTHROPIC_API_KEY", &key);
        }

        match &config.permission_strategy {
            PermissionStrategy::PromptToolStdio => {
                cmd.arg("--permission-prompt-tool").arg("stdio");
            }
            PermissionStrategy::AllowedTools(tools) => {
                if !tools.is_empty() {
                    cmd.arg("--allowedTools").args(tools);
                }
            }
            PermissionStrategy::SkipPermissions => {
                cmd.arg("--dangerously-skip-permissions");
            }
        }

        if let Some(ref prompt) = config.prompt {
            cmd.arg("-p").arg(prompt);
            // --include-partial-messages requires -p (--print mode)
            cmd.arg("--include-partial-messages");
        }

        if let Some(ref session) = config.resume_session {
            cmd.arg("--resume").arg(session);
        }

        if let Some(ref model) = config.model {
            cmd.arg("--model").arg(model);
        }

        // Spawn process
        info!(
            working_dir = %working_dir.display(),
            has_prompt = config.prompt.is_some(),
            resume_session = ?config.resume_session,
            model = ?config.model,
            "Spawning claude subprocess"
        );
        let mut child = cmd.spawn().map_err(|e| SubprocessError::SpawnFailed {
            reason: e.to_string(),
        })?;

        let process_id = uuid::Uuid::new_v4().to_string();

        // Set up stdin channel
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SubprocessError::SpawnFailed {
                reason: "Failed to capture stdin".to_string(),
            })?;

        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(32);

        // Spawn stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = stdin_rx.recv().await {
                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    error!("Failed to write to stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    error!("Failed to write newline: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    error!("Failed to flush stdin: {}", e);
                    break;
                }
            }
        });

        // Set up stdout reader
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SubprocessError::SpawnFailed {
                reason: "Failed to capture stdout".to_string(),
            })?;

        let pid = process_id.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                debug!(process_id = %pid, "stdout: {}", line);
                if stdout_tx.send(line).await.is_err() {
                    warn!(process_id = %pid, "stdout channel closed");
                    break;
                }
            }
            info!(process_id = %pid, "stdout reader finished");
        });

        // Set up stderr reader for diagnostics
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            let pid_err = process_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(process_id = %pid_err, "stderr: {}", line);
                }
                debug!(process_id = %pid_err, "stderr reader finished");
            });
        }

        // Store process state
        let handle = ProcessHandle {
            id: process_id.clone(),
            session_id: None,
            stdin_tx: stdin_tx.clone(),
            working_directory: config.working_directory.clone(),
        };

        let state = ProcessState {
            child,
            session_id: None,
            stdin_tx,
            working_directory: config.working_directory,
        };

        self.processes.write().await.insert(process_id, state);

        Ok(handle)
    }

    /// Send a command to a process's stdin.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn send(&self, process_id: &str, message: &str) -> Result<(), SubprocessError> {
        let processes = self.processes.read().await;
        let state = processes
            .get(process_id)
            .ok_or_else(|| SubprocessError::ProcessNotFound {
                id: process_id.to_string(),
            })?;

        state
            .stdin_tx
            .send(message.to_string())
            .await
            .map_err(|_| SubprocessError::ProcessExited {
                id: process_id.to_string(),
            })
    }

    /// Terminate a process gracefully.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn terminate(&self, process_id: &str) -> Result<(), SubprocessError> {
        let mut processes = self.processes.write().await;
        let mut state =
            processes
                .remove(process_id)
                .ok_or_else(|| SubprocessError::ProcessNotFound {
                    id: process_id.to_string(),
                })?;

        debug!(
            process_id,
            working_dir = %state.working_directory.display(),
            "Terminating subprocess"
        );

        // Try graceful shutdown first
        #[cfg(unix)]
        {
            if let Some(pid) = state.child.id() {
                // SAFETY: pid is a valid process ID obtained from our own Child handle.
                // kill(2) with SIGINT is safe to call on any owned subprocess.
                #[allow(unsafe_code)]
                #[allow(clippy::cast_possible_wrap)]
                let ret = unsafe { libc::kill(pid as i32, libc::SIGINT) };
                if ret != 0 {
                    let err = std::io::Error::last_os_error();
                    warn!(process_id, pid, error = %err, "Failed to send SIGINT");
                }
            }
        }

        // Wait with timeout
        match tokio::time::timeout(self.terminate_timeout, state.child.wait()).await {
            Ok(Ok(status)) => {
                info!(process_id, ?status, "Process exited gracefully");
                Ok(())
            }
            Ok(Err(e)) => {
                warn!(process_id, error = %e, "Error waiting for process");
                state.child.kill().await.ok();
                Ok(())
            }
            Err(_) => {
                warn!(process_id, "Timeout waiting for graceful shutdown, killing");
                state.child.kill().await.ok();
                Ok(())
            }
        }
    }

    /// Get count of active processes.
    pub async fn active_count(&self) -> usize {
        self.processes.read().await.len()
    }

    /// Get the maximum process pool capacity.
    pub const fn capacity(&self) -> usize {
        self.max_processes
    }

    /// Update session ID for a process.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn set_session_id(
        &self,
        process_id: &str,
        session_id: String,
    ) -> Result<(), SubprocessError> {
        let mut processes = self.processes.write().await;
        let state =
            processes
                .get_mut(process_id)
                .ok_or_else(|| SubprocessError::ProcessNotFound {
                    id: process_id.to_string(),
                })?;
        state.session_id = Some(session_id);
        Ok(())
    }
}

/// Errors from subprocess operations.
#[derive(Debug, thiserror::Error)]
pub enum SubprocessError {
    #[error("Subprocess pool exhausted ({current}/{max})")]
    PoolExhausted { current: usize, max: usize },

    #[error("Failed to spawn subprocess: {reason}")]
    SpawnFailed { reason: String },

    #[error("Process not found: {id}")]
    ProcessNotFound { id: String },

    #[error("Process already exited: {id}")]
    ProcessExited { id: String },
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn manager_respects_pool_limit() {
        let manager = SubprocessManager::new(2, "claude".into());
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn spawn_config_defaults() {
        let config = SpawnConfig::default();
        assert!(config.prompt.is_none());
        assert!(config.resume_session.is_none());
    }
}
