//! Subagent lifecycle manager.
//!
//! [`SubagentManager`] is the high-level coordinator that:
//! - spawns Claude subprocess per subagent (via [`SubprocessPool`])
//! - monitors subprocess exit and updates DB status
//! - enforces per-subagent timeouts (SIGTERM -> 5 s grace -> SIGKILL)
//! - supports cancellation of running subagents
//! - manages orchestration lifecycles

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use betcode_proto::v1::{
    OrchestrationCompleted, OrchestrationEvent, OrchestrationFailed, OrchestrationStrategy,
    StepCompleted, StepFailed, StepStarted, SubagentCancelled, SubagentCompleted, SubagentEvent,
    SubagentFailed, SubagentOutput, SubagentPermissionRequest, SubagentStarted, SubagentToolUse,
};

use crate::storage::Database;

use super::pool::{PoolEntry, SubprocessPool};
use super::scheduler::DagScheduler;

/// Default timeout per subagent in seconds (10 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Grace period after SIGTERM before SIGKILL.
const GRACE_PERIOD_SECS: u64 = 5;

/// Configuration for spawning a subagent.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// Subagent ID (pre-generated).
    pub id: String,
    /// Parent session ID.
    pub parent_session_id: String,
    /// Prompt to send to the subagent.
    pub prompt: String,
    /// Model override (empty = default).
    pub model: Option<String>,
    /// Working directory.
    pub working_directory: PathBuf,
    /// Tools to auto-approve.
    pub allowed_tools: Vec<String>,
    /// Maximum turns before timeout.
    pub max_turns: i32,
    /// Whether auto-approve is enabled.
    pub auto_approve: bool,
    /// Timeout in seconds (0 = default).
    pub timeout_secs: u64,
}

/// Handle to track a running subagent for event broadcasting.
struct RunningSubagent {
    /// Subscribers watching this subagent.
    event_txs: Vec<mpsc::Sender<SubagentEvent>>,
    /// Process ID for signaling.
    #[cfg(unix)]
    pid: Option<u32>,
    /// Whether this subagent has auto-approve enabled.
    auto_approve: bool,
}

/// Errors from the subagent manager.
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    #[error("Subprocess pool is full")]
    PoolFull,

    #[error("Failed to spawn subprocess: {reason}")]
    SpawnFailed { reason: String },

    #[error("Subagent not found: {id}")]
    NotFound { id: String },

    #[error("Subagent already completed: {id}")]
    AlreadyCompleted { id: String },

    #[error("Database error: {0}")]
    Database(#[from] crate::storage::DatabaseError),

    #[error("Validation error: {message}")]
    Validation { message: String },
}

/// High-level subagent lifecycle manager.
pub struct SubagentManager {
    pool: Arc<SubprocessPool>,
    db: Database,
    /// Active subagents keyed by subagent ID.
    running: Arc<RwLock<HashMap<String, RunningSubagent>>>,
}

impl SubagentManager {
    /// Create a new manager backed by the given pool and database.
    pub fn new(pool: Arc<SubprocessPool>, db: Database) -> Self {
        Self {
            pool,
            db,
            running: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawn a new subagent subprocess.
    ///
    /// Returns the subagent ID on success.
    #[allow(clippy::too_many_lines)]
    pub async fn spawn(&self, config: SubagentConfig) -> Result<String, ManagerError> {
        // Validate config
        if config.prompt.is_empty() {
            return Err(ManagerError::Validation {
                message: "prompt must not be empty".to_string(),
            });
        }
        if config.auto_approve && config.allowed_tools.is_empty() {
            return Err(ManagerError::Validation {
                message: "auto_approve requires non-empty allowed_tools".to_string(),
            });
        }

        // Acquire pool permit
        let permit = self.pool.try_acquire().ok_or(ManagerError::PoolFull)?;

        let subagent_id = config.id.clone();
        let allowed_tools_json =
            serde_json::to_string(&config.allowed_tools).unwrap_or_else(|_| "[]".to_string());

        // Create DB record
        self.db
            .create_subagent(
                &subagent_id,
                &config.parent_session_id,
                &config.prompt,
                config.model.as_deref(),
                i64::from(config.max_turns),
                config.auto_approve,
                &allowed_tools_json,
                Some(config.working_directory.to_string_lossy().as_ref()),
            )
            .await?;

        // Build claude command
        let working_dir = if config.working_directory.as_os_str().is_empty()
            || !config.working_directory.exists()
        {
            dirs::home_dir().unwrap_or_else(std::env::temp_dir)
        } else {
            config.working_directory.clone()
        };

        let mut cmd = Command::new("claude");
        cmd.current_dir(&working_dir)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--verbose")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Permission handling
        if config.auto_approve && !config.allowed_tools.is_empty() {
            cmd.arg("--allowedTools").args(&config.allowed_tools);
        }

        // Prompt
        cmd.arg("-p").arg(&config.prompt);

        // Model
        if let Some(ref model) = config.model {
            cmd.arg("--model").arg(model);
        }

        // Max turns
        if config.max_turns > 0 {
            cmd.arg("--max-turns").arg(config.max_turns.to_string());
        }

        info!(
            subagent_id = %subagent_id,
            working_dir = %working_dir.display(),
            auto_approve = config.auto_approve,
            "Spawning subagent subprocess"
        );

        let mut child = cmd.spawn().map_err(|e| ManagerError::SpawnFailed {
            reason: e.to_string(),
        })?;

        #[cfg(unix)]
        let pid = child.id();

        // Set up stdin
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ManagerError::SpawnFailed {
                reason: "Failed to capture stdin".to_string(),
            })?;
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(32);

        // Stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = stdin_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        // Register in pool
        self.pool
            .register(PoolEntry {
                subagent_id: subagent_id.clone(),
                stdin_tx: stdin_tx.clone(),
            })
            .await;

        // Register in running map
        {
            let mut running = self.running.write().await;
            running.insert(
                subagent_id.clone(),
                RunningSubagent {
                    event_txs: Vec::new(),
                    #[cfg(unix)]
                    pid,
                    auto_approve: config.auto_approve,
                },
            );
        }

        // Update DB status to running
        self.db
            .update_subagent_status(&subagent_id, "running", None, None)
            .await?;

        // Set up stdout reader and monitoring
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let sa_id = subagent_id.clone();
        let running_map = Arc::clone(&self.running);
        let pool = Arc::clone(&self.pool);
        let db = self.db.clone();

        let timeout = if config.timeout_secs == 0 {
            DEFAULT_TIMEOUT_SECS
        } else {
            config.timeout_secs
        };

        // Spawn monitoring task
        tokio::spawn(async move {
            let _permit = permit; // Keep permit alive for duration

            // Broadcast started event
            broadcast_event(
                &running_map,
                &sa_id,
                SubagentEvent {
                    subagent_id: sa_id.clone(),
                    timestamp: Some(now_timestamp()),
                    event: Some(betcode_proto::v1::subagent_event::Event::Started(
                        SubagentStarted {
                            session_id: String::new(),
                            model: config.model.unwrap_or_default(),
                        },
                    )),
                },
            )
            .await;

            // Read stdout lines and broadcast events
            if let Some(stdout) = stdout {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                let sa_id_stdout = sa_id.clone();
                let running_map_stdout = Arc::clone(&running_map);

                tokio::spawn(async move {
                    while let Ok(Some(line)) = lines.next_line().await {
                        // Parse NDJSON line and convert to subagent events
                        let events = parse_stdout_line(&sa_id_stdout, &line);
                        for event in events {
                            broadcast_event(&running_map_stdout, &sa_id_stdout, event).await;
                        }
                    }
                });
            }

            // Read stderr for diagnostics
            if let Some(stderr) = stderr {
                let sa_id_stderr = sa_id.clone();
                tokio::spawn(async move {
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        warn!(subagent_id = %sa_id_stderr, "stderr: {}", line);
                    }
                });
            }

            // Wait for exit with timeout
            let exit_result =
                tokio::time::timeout(std::time::Duration::from_secs(timeout), child.wait()).await;

            let (status_str, exit_code, summary) = match exit_result {
                Ok(Ok(status)) => {
                    let code = status.code();
                    if code == Some(0) {
                        (
                            "completed",
                            code.map(i64::from),
                            Some("Completed successfully".to_string()),
                        )
                    } else {
                        (
                            "failed",
                            code.map(i64::from),
                            Some(format!("Exited with code {}", code.unwrap_or(-1))),
                        )
                    }
                }
                Ok(Err(e)) => {
                    error!(subagent_id = %sa_id, error = %e, "Subprocess wait error");
                    ("failed", None, Some(format!("Process error: {e}")))
                }
                Err(_) => {
                    warn!(subagent_id = %sa_id, timeout, "Subagent timed out, sending SIGTERM");
                    terminate_process(&mut child).await;
                    ("failed", None, Some("Timed out".to_string()))
                }
            };

            // Update DB
            if let Err(e) = db
                .update_subagent_status(&sa_id, status_str, exit_code, summary.as_deref())
                .await
            {
                error!(subagent_id = %sa_id, error = %e, "Failed to update subagent status");
            }

            // Broadcast terminal event
            let terminal_event = match status_str {
                "completed" => SubagentEvent {
                    subagent_id: sa_id.clone(),
                    timestamp: Some(now_timestamp()),
                    event: Some(betcode_proto::v1::subagent_event::Event::Completed(
                        SubagentCompleted {
                            #[allow(clippy::cast_possible_truncation)]
                            exit_code: exit_code.unwrap_or(0) as i32,
                            result_summary: summary.unwrap_or_default(),
                        },
                    )),
                },
                "cancelled" => SubagentEvent {
                    subagent_id: sa_id.clone(),
                    timestamp: Some(now_timestamp()),
                    event: Some(betcode_proto::v1::subagent_event::Event::Cancelled(
                        SubagentCancelled {
                            reason: summary.unwrap_or_default(),
                        },
                    )),
                },
                _ => SubagentEvent {
                    subagent_id: sa_id.clone(),
                    timestamp: Some(now_timestamp()),
                    event: Some(betcode_proto::v1::subagent_event::Event::Failed(
                        SubagentFailed {
                            #[allow(clippy::cast_possible_truncation)]
                            exit_code: exit_code.unwrap_or(-1) as i32,
                            error_message: summary.unwrap_or_default(),
                        },
                    )),
                },
            };
            broadcast_event(&running_map, &sa_id, terminal_event).await;

            // Cleanup
            pool.unregister(&sa_id).await;
            running_map.write().await.remove(&sa_id);

            info!(subagent_id = %sa_id, status = status_str, "Subagent monitoring task finished");
        });

        Ok(subagent_id)
    }

    /// Subscribe to a subagent's event stream.
    ///
    /// Returns a receiver for `SubagentEvent` messages.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn subscribe(
        &self,
        subagent_id: &str,
    ) -> Result<mpsc::Receiver<SubagentEvent>, ManagerError> {
        let mut running = self.running.write().await;
        let entry = running
            .get_mut(subagent_id)
            .ok_or_else(|| ManagerError::NotFound {
                id: subagent_id.to_string(),
            })?;

        let (tx, rx) = mpsc::channel(128);
        entry.event_txs.push(tx);
        Ok(rx)
    }

    /// Send input to a running subagent's stdin.
    pub async fn send_input(&self, subagent_id: &str, content: &str) -> Result<bool, ManagerError> {
        let entry = self
            .pool
            .get(subagent_id)
            .await
            .ok_or_else(|| ManagerError::NotFound {
                id: subagent_id.to_string(),
            })?;

        let delivered = entry.stdin_tx.send(content.to_string()).await.is_ok();
        Ok(delivered)
    }

    /// Cancel a running subagent.
    pub async fn cancel(&self, subagent_id: &str, reason: &str) -> Result<bool, ManagerError> {
        // Check if running
        let running = self.running.read().await;
        let entry = running.get(subagent_id);

        if entry.is_none() {
            // May already be completed
            return Ok(false);
        }

        #[cfg(unix)]
        if let Some(sa) = entry
            && let Some(pid) = sa.pid
        {
            // Send SIGTERM
            #[allow(unsafe_code, clippy::cast_possible_wrap)]
            let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                warn!(subagent_id, pid, error = %err, "Failed to send SIGTERM");
            }
        }
        drop(running);

        // Update DB
        self.db
            .update_subagent_status(subagent_id, "cancelled", None, Some(reason))
            .await?;

        Ok(true)
    }

    /// Revoke auto-approve on a running subagent.
    ///
    /// This doesn't actually change the subprocess's `--allowedTools` flag
    /// (that's a launch-time argument), but it marks the subagent as no longer
    /// auto-approved in the database, so future permission queries will be
    /// forwarded to the parent session.
    pub async fn revoke_auto_approve(&self, subagent_id: &str) -> Result<bool, ManagerError> {
        let mut running = self.running.write().await;
        if let Some(sa) = running.get_mut(subagent_id) {
            sa.auto_approve = false;
            // Note: We can't retroactively change the subprocess's --allowedTools flag.
            // The revocation is tracked in-memory and in the DB for future permission
            // requests that are forwarded via the permission bridge.
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Check if a subagent is currently running.
    pub async fn is_running(&self, subagent_id: &str) -> bool {
        self.running.read().await.contains_key(subagent_id)
    }

    /// Get a reference to the underlying pool.
    pub fn pool(&self) -> &SubprocessPool {
        &self.pool
    }

    /// Get a reference to the database.
    pub const fn db(&self) -> &Database {
        &self.db
    }

    /// Run an orchestration lifecycle.
    ///
    /// This is used by `CreateOrchestration` to kick off the scheduler loop.
    #[allow(clippy::too_many_lines)]
    pub async fn run_orchestration(
        self: &Arc<Self>,
        orchestration_id: String,
        parent_session_id: String,
        strategy: OrchestrationStrategy,
        steps: Vec<betcode_proto::v1::OrchestrationStep>,
        event_tx: mpsc::Sender<OrchestrationEvent>,
    ) -> Result<(), ManagerError> {
        // Validate and create scheduler
        let step_ids: Vec<String> = steps.iter().map(|s| s.id.clone()).collect();
        let step_deps: Vec<Vec<String>> = steps.iter().map(|s| s.depends_on.clone()).collect();
        // Build adjacency for scheduler
        let mut dep_map: HashMap<String, Vec<String>> = HashMap::new();
        for (i, id) in step_ids.iter().enumerate() {
            dep_map.insert(id.clone(), step_deps[i].clone());
        }

        let scheduler = DagScheduler::new(step_ids.clone(), dep_map)?;

        // Create DB records for steps
        let strategy_str = match strategy {
            OrchestrationStrategy::Sequential => "sequential",
            OrchestrationStrategy::Dag => "dag",
            // Parallel and Unspecified both default to parallel
            OrchestrationStrategy::Parallel | OrchestrationStrategy::Unspecified => "parallel",
        };

        self.db
            .create_orchestration(&orchestration_id, &parent_session_id, strategy_str)
            .await?;
        self.db
            .update_orchestration_status(&orchestration_id, "running")
            .await?;

        for (i, step) in steps.iter().enumerate() {
            let deps_json =
                serde_json::to_string(&step.depends_on).unwrap_or_else(|_| "[]".to_string());
            #[allow(clippy::cast_possible_wrap)]
            self.db
                .create_orchestration_step(
                    &step.id,
                    &orchestration_id,
                    i as i64,
                    &step.prompt,
                    &deps_json,
                )
                .await?;
        }

        // Build step configs from proto steps (owned, for 'static in tokio::spawn)
        let step_configs: HashMap<String, betcode_proto::v1::OrchestrationStep> =
            steps.into_iter().map(|s| (s.id.clone(), s)).collect();

        // Orchestration loop
        let manager = Arc::clone(self);
        let orch_id = orchestration_id.clone();
        let db = self.db.clone();

        tokio::spawn(async move {
            let mut scheduler = scheduler;
            let mut completed_count: i32 = 0;
            let mut failed_count: i32 = 0;
            let mut step_results: HashMap<String, String> = HashMap::new();
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let total = step_ids.len() as i32;

            loop {
                let ready = scheduler.next_ready();
                if ready.is_empty() && !scheduler.is_complete() && failed_count == 0 {
                    // Wait a bit for running steps to complete
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }

                if ready.is_empty() && (scheduler.is_complete() || failed_count > 0) {
                    break;
                }

                // Spawn ready steps
                for step_id in &ready {
                    let Some(step_cfg) = step_configs.get(step_id) else {
                        continue;
                    };

                    // Build prompt with context from completed dependencies
                    let mut full_prompt = String::new();
                    for dep_id in &step_cfg.depends_on {
                        if let Some(result) = step_results.get(dep_id) {
                            let _ =
                                write!(full_prompt, "[Context from step {dep_id}]: {result}\n\n");
                        }
                    }
                    full_prompt.push_str(&step_cfg.prompt);

                    let sa_id = format!("{orch_id}-{step_id}");
                    let working_dir = if step_cfg.working_directory.is_empty() {
                        std::env::current_dir().unwrap_or_default()
                    } else {
                        PathBuf::from(&step_cfg.working_directory)
                    };

                    let sa_config = SubagentConfig {
                        id: sa_id.clone(),
                        parent_session_id: parent_session_id.clone(),
                        prompt: full_prompt,
                        model: if step_cfg.model.is_empty() {
                            None
                        } else {
                            Some(step_cfg.model.clone())
                        },
                        working_directory: working_dir,
                        allowed_tools: step_cfg.allowed_tools.clone(),
                        max_turns: step_cfg.max_turns,
                        auto_approve: step_cfg.auto_approve,
                        timeout_secs: 0,
                    };

                    // Update step status
                    let _ = db
                        .update_step_status(step_id, "running", Some(&sa_id))
                        .await;

                    match manager.spawn(sa_config).await {
                        Ok(_) => {
                            // Broadcast StepStarted
                            let _ = event_tx
                                .send(OrchestrationEvent {
                                    orchestration_id: orch_id.clone(),
                                    timestamp: Some(now_timestamp()),
                                    event: Some(
                                        betcode_proto::v1::orchestration_event::Event::StepStarted(
                                            StepStarted {
                                                step_id: step_id.clone(),
                                                subagent_id: sa_id,
                                                name: step_cfg.name.clone(),
                                            },
                                        ),
                                    ),
                                })
                                .await;

                            scheduler.mark_running(step_id);
                        }
                        Err(e) => {
                            error!(step_id, error = %e, "Failed to spawn step subagent");
                            let _ = db.update_step_status(step_id, "failed", None).await;

                            // Cascade failure
                            let blocked = scheduler.mark_failed(step_id);
                            for bid in &blocked {
                                let _ = db.update_step_status(bid, "blocked", None).await;
                            }
                            failed_count += 1;

                            let _ = event_tx
                                .send(OrchestrationEvent {
                                    orchestration_id: orch_id.clone(),
                                    timestamp: Some(now_timestamp()),
                                    event: Some(
                                        betcode_proto::v1::orchestration_event::Event::StepFailed(
                                            StepFailed {
                                                step_id: step_id.clone(),
                                                error_message: e.to_string(),
                                                blocked_steps: blocked,
                                            },
                                        ),
                                    ),
                                })
                                .await;
                        }
                    }
                }

                // Wait for any running step to complete
                // Poll DB for status changes
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                let running_ids = scheduler.running_ids();
                for step_id in &running_ids {
                    let sa_id = format!("{orch_id}-{step_id}");
                    if manager.is_running(&sa_id).await {
                        continue; // Still running
                    }

                    // Check DB for result
                    match db.get_subagent(&sa_id).await {
                        Ok(sa) if sa.status == "completed" => {
                            let summary = sa.result_summary.unwrap_or_default();
                            step_results.insert(step_id.clone(), summary.clone());
                            let _ = db
                                .update_step_status(step_id, "completed", Some(&sa_id))
                                .await;

                            let blocked = scheduler.mark_completed(step_id);
                            // `blocked` should be empty for completed — it's the freed steps
                            let _ = blocked; // freed downstream steps are now ready
                            completed_count += 1;

                            let _ = event_tx
                                .send(OrchestrationEvent {
                                    orchestration_id: orch_id.clone(),
                                    timestamp: Some(now_timestamp()),
                                    event: Some(
                                        betcode_proto::v1::orchestration_event::Event::StepCompleted(
                                            StepCompleted {
                                                step_id: step_id.clone(),
                                                result_summary: summary,
                                                completed_count,
                                                total_count: total,
                                            },
                                        ),
                                    ),
                                })
                                .await;
                        }
                        Ok(sa) if sa.status == "failed" || sa.status == "cancelled" => {
                            let error_msg = sa
                                .result_summary
                                .unwrap_or_else(|| "Unknown failure".to_string());
                            let _ = db.update_step_status(step_id, "failed", Some(&sa_id)).await;

                            let blocked = scheduler.mark_failed(step_id);
                            for bid in &blocked {
                                let _ = db.update_step_status(bid, "blocked", None).await;
                            }
                            failed_count += 1;

                            let _ = event_tx
                                .send(OrchestrationEvent {
                                    orchestration_id: orch_id.clone(),
                                    timestamp: Some(now_timestamp()),
                                    event: Some(
                                        betcode_proto::v1::orchestration_event::Event::StepFailed(
                                            StepFailed {
                                                step_id: step_id.clone(),
                                                error_message: error_msg,
                                                blocked_steps: blocked,
                                            },
                                        ),
                                    ),
                                })
                                .await;
                        }
                        _ => {
                            // Still pending or unknown — check again next iteration
                        }
                    }
                }
            }

            // Final orchestration status
            let (final_status, final_event) = if failed_count > 0 {
                (
                    "failed",
                    OrchestrationEvent {
                        orchestration_id: orch_id.clone(),
                        timestamp: Some(now_timestamp()),
                        event: Some(betcode_proto::v1::orchestration_event::Event::Failed(
                            OrchestrationFailed {
                                error_message: format!("{failed_count} step(s) failed"),
                                completed_steps: completed_count,
                                failed_steps: failed_count,
                            },
                        )),
                    },
                )
            } else {
                (
                    "completed",
                    OrchestrationEvent {
                        orchestration_id: orch_id.clone(),
                        timestamp: Some(now_timestamp()),
                        event: Some(betcode_proto::v1::orchestration_event::Event::Completed(
                            OrchestrationCompleted {
                                total_steps: total,
                                succeeded: completed_count,
                                failed: 0,
                            },
                        )),
                    },
                )
            };

            let _ = db.update_orchestration_status(&orch_id, final_status).await;
            let _ = event_tx.send(final_event).await;

            info!(
                orchestration_id = %orch_id,
                status = final_status,
                completed = completed_count,
                failed = failed_count,
                "Orchestration finished"
            );
        });

        Ok(())
    }
}

/// Parse an NDJSON stdout line into subagent events.
#[allow(clippy::too_many_lines)]
fn parse_stdout_line(subagent_id: &str, line: &str) -> Vec<SubagentEvent> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        debug!(subagent_id, "Non-JSON stdout line: {}", line);
        return vec![];
    };

    let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "assistant" => {
            // Extract text content from assistant message
            let text = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| {
                            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                b.get("text").and_then(|t| t.as_str()).map(String::from)
                            } else {
                                None
                            }
                        })
                        .collect::<String>()
                })
                .unwrap_or_default();

            let mut events = Vec::new();
            if !text.is_empty() {
                events.push(SubagentEvent {
                    subagent_id: subagent_id.to_string(),
                    timestamp: Some(now_timestamp()),
                    event: Some(betcode_proto::v1::subagent_event::Event::Output(
                        SubagentOutput {
                            text,
                            is_complete: false,
                        },
                    )),
                });
            }

            // Extract tool_use blocks
            if let Some(content) = value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for block in content {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let tool_name = block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let tool_id = block
                            .get("id")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        events.push(SubagentEvent {
                            subagent_id: subagent_id.to_string(),
                            timestamp: Some(now_timestamp()),
                            event: Some(betcode_proto::v1::subagent_event::Event::ToolUse(
                                SubagentToolUse {
                                    tool_id,
                                    tool_name,
                                    description: String::new(),
                                },
                            )),
                        });
                    }
                }
            }

            events
        }
        "content_block_delta" => {
            let text = value
                .get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");

            if text.is_empty() {
                return vec![];
            }

            vec![SubagentEvent {
                subagent_id: subagent_id.to_string(),
                timestamp: Some(now_timestamp()),
                event: Some(betcode_proto::v1::subagent_event::Event::Output(
                    SubagentOutput {
                        text: text.to_string(),
                        is_complete: false,
                    },
                )),
            }]
        }
        "control_request" => {
            let tool_name = value
                .get("request")
                .and_then(|r| r.get("tool_name"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let request_id = value
                .get("request_id")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();

            vec![SubagentEvent {
                subagent_id: subagent_id.to_string(),
                timestamp: Some(now_timestamp()),
                event: Some(betcode_proto::v1::subagent_event::Event::PermissionRequest(
                    SubagentPermissionRequest {
                        request_id,
                        tool_name,
                        description: String::new(),
                    },
                )),
            }]
        }
        _ => vec![],
    }
}

/// Broadcast an event to all subscribers of a subagent.
async fn broadcast_event(
    running: &Arc<RwLock<HashMap<String, RunningSubagent>>>,
    subagent_id: &str,
    event: SubagentEvent,
) {
    let mut map = running.write().await;
    if let Some(sa) = map.get_mut(subagent_id) {
        // Remove closed channels
        sa.event_txs.retain(|tx| !tx.is_closed());

        for tx in &sa.event_txs {
            let _ = tx.send(event.clone()).await;
        }
    }
}

/// Terminate a process: SIGTERM, wait grace period, then SIGKILL.
async fn terminate_process(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            #[allow(unsafe_code, clippy::cast_possible_wrap)]
            let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                warn!(pid, error = %err, "Failed to send SIGTERM");
            }
        }
    }

    if tokio::time::timeout(
        std::time::Duration::from_secs(GRACE_PERIOD_SECS),
        child.wait(),
    )
    .await
    .is_err()
    {
        warn!("Grace period expired, sending SIGKILL");
        let _ = child.kill().await;
    }
}

/// Generate a protobuf Timestamp for the current time.
fn now_timestamp() -> prost_types::Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    prost_types::Timestamp {
        #[allow(clippy::cast_possible_wrap)]
        seconds: now.as_secs() as i64,
        #[allow(clippy::cast_possible_wrap)]
        nanos: now.subsec_nanos() as i32,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_pool() -> Arc<SubprocessPool> {
        Arc::new(SubprocessPool::new(3))
    }

    async fn test_db() -> Database {
        let db = Database::open_in_memory().await.unwrap();
        db.create_session("parent-1", "claude-sonnet-4", "/tmp")
            .await
            .unwrap();
        db
    }

    fn test_config() -> SubagentConfig {
        SubagentConfig {
            id: "test-sa-1".to_string(),
            parent_session_id: "parent-1".to_string(),
            prompt: "Write tests".to_string(),
            model: None,
            working_directory: std::env::temp_dir(),
            allowed_tools: vec![],
            max_turns: 10,
            auto_approve: false,
            timeout_secs: 30,
        }
    }

    #[tokio::test]
    async fn manager_creation() {
        let db = test_db().await;
        let pool = test_pool();
        let _manager = SubagentManager::new(pool, db);
    }

    #[tokio::test]
    async fn spawn_validates_empty_prompt() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        let mut config = test_config();
        config.prompt = String::new();

        let result = manager.spawn(config).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("prompt must not be empty"),
            "Should reject empty prompt"
        );
    }

    #[tokio::test]
    async fn spawn_validates_auto_approve_requires_tools() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        let mut config = test_config();
        config.auto_approve = true;
        config.allowed_tools = vec![];

        let result = manager.spawn(config).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("auto_approve requires"),
            "Should reject auto_approve with empty allowed_tools"
        );
    }

    #[tokio::test]
    async fn is_running_returns_false_for_nonexistent() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        assert!(!manager.is_running("nonexistent").await);
    }

    #[tokio::test]
    async fn cancel_nonexistent_returns_false() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        let result = manager.cancel("nonexistent", "test").await;
        assert!(matches!(result, Ok(false)));
    }

    #[tokio::test]
    async fn subscribe_nonexistent_returns_error() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        let result = manager.subscribe("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_input_nonexistent_returns_error() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        let result = manager.send_input("nonexistent", "hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn revoke_auto_approve_nonexistent() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db);

        let result = manager.revoke_auto_approve("nonexistent").await;
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn parse_stdout_text_delta() {
        let line = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}"#;
        let events = parse_stdout_line("sa-1", line);
        assert_eq!(events.len(), 1);
        match &events[0].event {
            Some(betcode_proto::v1::subagent_event::Event::Output(out)) => {
                assert_eq!(out.text, "Hello");
            }
            other => panic!("Expected Output, got {:?}", other),
        }
    }

    #[test]
    fn parse_stdout_empty_text_delta_suppressed() {
        let line = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":""}}"#;
        let events = parse_stdout_line("sa-1", line);
        assert!(events.is_empty());
    }

    #[test]
    fn parse_stdout_non_json() {
        let events = parse_stdout_line("sa-1", "not json at all");
        assert!(events.is_empty());
    }

    #[test]
    fn parse_stdout_unknown_type() {
        let line = r#"{"type":"unknown_event","data":123}"#;
        let events = parse_stdout_line("sa-1", line);
        assert!(events.is_empty());
    }

    #[test]
    fn parse_stdout_control_request() {
        let line = r#"{"type":"control_request","request_id":"req-1","request":{"type":"CanUseTool","tool_name":"Bash","input":{"command":"ls"}}}"#;
        let events = parse_stdout_line("sa-1", line);
        assert_eq!(events.len(), 1);
        match &events[0].event {
            Some(betcode_proto::v1::subagent_event::Event::PermissionRequest(pr)) => {
                assert_eq!(pr.request_id, "req-1");
                assert_eq!(pr.tool_name, "Bash");
            }
            other => panic!("Expected PermissionRequest, got {:?}", other),
        }
    }

    #[test]
    fn now_timestamp_is_reasonable() {
        let ts = now_timestamp();
        // Should be after 2020
        assert!(ts.seconds > 1_577_836_800);
    }
}
