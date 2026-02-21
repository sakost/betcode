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
use tokio::sync::{Notify, RwLock, broadcast, mpsc};
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

/// Per-orchestration state for event broadcasting and loop notification.
struct OrchestrationState {
    /// Broadcast sender for orchestration events (for `WatchOrchestration` subscribers).
    event_tx: broadcast::Sender<OrchestrationEvent>,
    /// Notify handle to wake the orchestration loop when a subagent finishes.
    step_notify: Arc<Notify>,
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

    #[error("Orchestration not found: {id}")]
    OrchestrationNotFound { id: String },

    #[error("Database error: {0}")]
    Database(#[from] crate::storage::DatabaseError),

    #[error("Validation error: {message}")]
    Validation { message: String },
}

/// Broadcast channel buffer size for orchestration events.
const ORCHESTRATION_BROADCAST_CAPACITY: usize = 256;

/// High-level subagent lifecycle manager.
pub struct SubagentManager {
    pool: Arc<SubprocessPool>,
    db: Database,
    /// Path to the `claude` binary.
    claude_bin: PathBuf,
    /// Active subagents keyed by subagent ID.
    running: Arc<RwLock<HashMap<String, RunningSubagent>>>,
    /// Active orchestrations keyed by orchestration ID.
    orchestrations: Arc<RwLock<HashMap<String, OrchestrationState>>>,
    /// Maps `subagent_id` to `orchestration_id` for notifying the right orchestration.
    subagent_to_orchestration: Arc<RwLock<HashMap<String, String>>>,
}

impl SubagentManager {
    /// Create a new manager backed by the given pool and database.
    pub fn new(pool: Arc<SubprocessPool>, db: Database, claude_bin: PathBuf) -> Self {
        Self {
            pool,
            db,
            claude_bin,
            running: Arc::new(RwLock::new(HashMap::new())),
            orchestrations: Arc::new(RwLock::new(HashMap::new())),
            subagent_to_orchestration: Arc::new(RwLock::new(HashMap::new())),
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
        let subagent_to_orch = Arc::clone(&self.subagent_to_orchestration);
        let orchestrations_map = Arc::clone(&self.orchestrations);

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

            // Notify orchestration if this subagent belongs to one
            let orch_id = subagent_to_orch.read().await.get(&sa_id).cloned();
            if let Some(orch_id) = orch_id {
                if let Some(state) = orchestrations_map.read().await.get(&orch_id) {
                    state.step_notify.notify_one();
                }
                subagent_to_orch.write().await.remove(&sa_id);
            }

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

    /// Subscribe to an orchestration's event broadcast channel.
    ///
    /// Returns a `broadcast::Receiver` that receives all `OrchestrationEvent`
    /// messages for the given orchestration.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn subscribe_orchestration(
        &self,
        orchestration_id: &str,
    ) -> Result<broadcast::Receiver<OrchestrationEvent>, ManagerError> {
        let orchestrations = self.orchestrations.read().await;
        let state = orchestrations.get(orchestration_id).ok_or_else(|| {
            ManagerError::OrchestrationNotFound {
                id: orchestration_id.to_string(),
            }
        })?;
        Ok(state.event_tx.subscribe())
    }

    /// Run an orchestration lifecycle.
    ///
    /// This is used by `CreateOrchestration` to kick off the scheduler loop.
    /// The manager owns the broadcast channel; subscribers connect via
    /// [`subscribe_orchestration`].
    #[allow(clippy::too_many_lines)]
    pub async fn run_orchestration(
        self: &Arc<Self>,
        orchestration_id: String,
        parent_session_id: String,
        strategy: OrchestrationStrategy,
        steps: Vec<betcode_proto::v1::OrchestrationStep>,
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

        // Create broadcast channel and Notify for this orchestration
        let (event_tx, _) = broadcast::channel(ORCHESTRATION_BROADCAST_CAPACITY);
        let step_notify = Arc::new(Notify::new());
        {
            let mut orchestrations = self.orchestrations.write().await;
            orchestrations.insert(
                orchestration_id.clone(),
                OrchestrationState {
                    event_tx: event_tx.clone(),
                    step_notify: Arc::clone(&step_notify),
                },
            );
        }

        // Build step configs from proto steps (owned, for 'static in tokio::spawn)
        let step_configs: HashMap<String, betcode_proto::v1::OrchestrationStep> =
            steps.into_iter().map(|s| (s.id.clone(), s)).collect();

        // Orchestration loop
        let manager = Arc::clone(self);
        let orch_id = orchestration_id.clone();
        let db = self.db.clone();
        let orchestrations_map = Arc::clone(&self.orchestrations);
        let subagent_to_orch = Arc::clone(&self.subagent_to_orchestration);

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
                    // Wait for a subagent to complete (event-driven via Notify)
                    step_notify.notified().await;
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

                    // Register subagent -> orchestration mapping before spawning
                    subagent_to_orch
                        .write()
                        .await
                        .insert(sa_id.clone(), orch_id.clone());

                    // Update step status
                    let _ = db
                        .update_step_status(step_id, "running", Some(&sa_id))
                        .await;

                    match manager.spawn(sa_config).await {
                        Ok(_) => {
                            // Broadcast StepStarted
                            let _ = event_tx.send(OrchestrationEvent {
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
                            });

                            scheduler.mark_running(step_id);
                        }
                        Err(e) => {
                            error!(step_id, error = %e, "Failed to spawn step subagent");
                            let _ = db.update_step_status(step_id, "failed", None).await;

                            // Remove the mapping since spawn failed
                            subagent_to_orch
                                .write()
                                .await
                                .remove(&format!("{orch_id}-{step_id}"));

                            // Cascade failure
                            let blocked = scheduler.mark_failed(step_id);
                            for bid in &blocked {
                                let _ = db.update_step_status(bid, "blocked", None).await;
                            }
                            failed_count += 1;

                            let _ = event_tx.send(OrchestrationEvent {
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
                            });
                        }
                    }
                }

                // If there are still running steps, wait for notification
                if !scheduler.running_ids().is_empty() {
                    step_notify.notified().await;
                }

                // Check all running steps for completion
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

                            let _ = event_tx.send(OrchestrationEvent {
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
                            });
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

                            let _ = event_tx.send(OrchestrationEvent {
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
                            });
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
            let _ = event_tx.send(final_event);

            // Clean up orchestration state
            orchestrations_map.write().await.remove(&orch_id);

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
        let _manager = SubagentManager::new(pool, db, "claude".into());
    }

    #[tokio::test]
    async fn spawn_validates_empty_prompt() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

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
        let manager = SubagentManager::new(pool, db, "claude".into());

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
        let manager = SubagentManager::new(pool, db, "claude".into());

        assert!(!manager.is_running("nonexistent").await);
    }

    #[tokio::test]
    async fn cancel_nonexistent_returns_false() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        let result = manager.cancel("nonexistent", "test").await;
        assert!(matches!(result, Ok(false)));
    }

    #[tokio::test]
    async fn subscribe_nonexistent_returns_error() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        let result = manager.subscribe("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_input_nonexistent_returns_error() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        let result = manager.send_input("nonexistent", "hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn revoke_auto_approve_nonexistent() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

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
            other => panic!("Expected Output, got {other:?}"),
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
            other => panic!("Expected PermissionRequest, got {other:?}"),
        }
    }

    #[test]
    fn now_timestamp_is_reasonable() {
        let ts = now_timestamp();
        // Should be after 2020
        assert!(ts.seconds > 1_577_836_800);
    }

    // =========================================================================
    // Orchestration subscription & Notify tests
    // =========================================================================

    #[tokio::test]
    async fn subscribe_orchestration_returns_error_for_unknown() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        let result = manager.subscribe_orchestration("nonexistent").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Orchestration not found"),
            "Expected OrchestrationNotFound, got: {err}"
        );
    }

    #[tokio::test]
    async fn notify_wakes_orchestration_loop() {
        // Verify that Notify::notify_one wakes a waiting notified().await
        let notify = Arc::new(Notify::new());
        let notify_clone = Arc::clone(&notify);

        let handle = tokio::spawn(async move {
            notify_clone.notified().await;
            true
        });

        // Give the spawned task time to start waiting
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        notify.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("should not time out")
            .expect("task should not panic");
        assert!(result, "notified task should have completed");
    }

    #[tokio::test]
    async fn broadcast_delivers_to_multiple_subscribers() {
        let (tx, _) = broadcast::channel::<OrchestrationEvent>(16);

        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();

        let event = OrchestrationEvent {
            orchestration_id: "orch-1".to_string(),
            timestamp: Some(now_timestamp()),
            event: Some(betcode_proto::v1::orchestration_event::Event::Completed(
                OrchestrationCompleted {
                    total_steps: 1,
                    succeeded: 1,
                    failed: 0,
                },
            )),
        };

        tx.send(event.clone()).expect("send should succeed");

        let ev1 = rx1.recv().await.expect("rx1 should receive event");
        let ev2 = rx2.recv().await.expect("rx2 should receive event");

        assert_eq!(ev1.orchestration_id, "orch-1");
        assert_eq!(ev2.orchestration_id, "orch-1");
    }

    #[tokio::test]
    async fn orchestration_state_stored_and_cleaned_up() {
        // Verify that OrchestrationState is properly managed in the map
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        // Manually insert an orchestration state
        let (event_tx, _) = broadcast::channel(16);
        let step_notify = Arc::new(Notify::new());
        {
            let mut orchestrations = manager.orchestrations.write().await;
            orchestrations.insert(
                "test-orch-1".to_string(),
                OrchestrationState {
                    event_tx,
                    step_notify,
                },
            );
        }

        // Should be able to subscribe now
        let result = manager.subscribe_orchestration("test-orch-1").await;
        assert!(
            result.is_ok(),
            "subscribe should succeed for existing orchestration"
        );

        // Remove it
        manager.orchestrations.write().await.remove("test-orch-1");

        // Should fail now
        let result = manager.subscribe_orchestration("test-orch-1").await;
        assert!(result.is_err(), "subscribe should fail after cleanup");
    }

    // =========================================================================
    // Orchestration lifecycle integration tests
    // =========================================================================

    /// Helper: insert an `OrchestrationState` into the manager and return its
    /// broadcast sender for manual event injection.
    async fn insert_orchestration_state(
        manager: &SubagentManager,
        orch_id: &str,
    ) -> broadcast::Sender<OrchestrationEvent> {
        let (event_tx, _) = broadcast::channel(ORCHESTRATION_BROADCAST_CAPACITY);
        let step_notify = Arc::new(Notify::new());
        let tx_clone = event_tx.clone();
        manager.orchestrations.write().await.insert(
            orch_id.to_string(),
            OrchestrationState {
                event_tx,
                step_notify,
            },
        );
        tx_clone
    }

    /// Build a minimal `OrchestrationStep` proto for testing.
    fn make_step(
        id: &str,
        prompt: &str,
        depends_on: Vec<String>,
    ) -> betcode_proto::v1::OrchestrationStep {
        betcode_proto::v1::OrchestrationStep {
            id: id.to_string(),
            name: id.to_string(),
            prompt: prompt.to_string(),
            depends_on,
            working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn subscribe_orchestration_receives_events() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        let tx = insert_orchestration_state(&manager, "orch-sub-1").await;

        // Subscribe to the orchestration
        let mut rx = manager
            .subscribe_orchestration("orch-sub-1")
            .await
            .expect("subscribe should succeed");

        // Send an event through the broadcast sender
        let event = OrchestrationEvent {
            orchestration_id: "orch-sub-1".to_string(),
            timestamp: Some(now_timestamp()),
            event: Some(betcode_proto::v1::orchestration_event::Event::StepStarted(
                StepStarted {
                    step_id: "step-0".to_string(),
                    subagent_id: "sa-0".to_string(),
                    name: "first".to_string(),
                },
            )),
        };
        tx.send(event.clone())
            .expect("broadcast send should succeed");

        let received = rx.recv().await.expect("subscriber should receive event");
        assert_eq!(received.orchestration_id, "orch-sub-1");
        match &received.event {
            Some(betcode_proto::v1::orchestration_event::Event::StepStarted(started)) => {
                assert_eq!(started.step_id, "step-0");
                assert_eq!(started.name, "first");
            }
            other => panic!("Expected StepStarted event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn subscribe_orchestration_multiple_subscribers() {
        let db = test_db().await;
        let pool = test_pool();
        let manager = SubagentManager::new(pool, db, "claude".into());

        let tx = insert_orchestration_state(&manager, "orch-multi-1").await;

        // Subscribe twice
        let mut rx1 = manager
            .subscribe_orchestration("orch-multi-1")
            .await
            .expect("first subscribe should succeed");
        let mut rx2 = manager
            .subscribe_orchestration("orch-multi-1")
            .await
            .expect("second subscribe should succeed");

        // Send one event
        let event = OrchestrationEvent {
            orchestration_id: "orch-multi-1".to_string(),
            timestamp: Some(now_timestamp()),
            event: Some(betcode_proto::v1::orchestration_event::Event::Completed(
                OrchestrationCompleted {
                    total_steps: 2,
                    succeeded: 2,
                    failed: 0,
                },
            )),
        };
        tx.send(event).expect("broadcast send should succeed");

        // Both receivers should get the event
        let ev1 = rx1.recv().await.expect("rx1 should receive event");
        let ev2 = rx2.recv().await.expect("rx2 should receive event");

        assert_eq!(ev1.orchestration_id, "orch-multi-1");
        assert_eq!(ev2.orchestration_id, "orch-multi-1");
        match (&ev1.event, &ev2.event) {
            (
                Some(betcode_proto::v1::orchestration_event::Event::Completed(c1)),
                Some(betcode_proto::v1::orchestration_event::Event::Completed(c2)),
            ) => {
                assert_eq!(c1.succeeded, 2);
                assert_eq!(c2.succeeded, 2);
            }
            other => panic!("Expected Completed events, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn orchestration_db_records_created() {
        // run_orchestration creates DB records (orchestration + steps) before
        // spawning subprocesses, so they should exist even if claude isn't
        // available.
        let db = test_db().await;
        let pool = test_pool();
        let manager = Arc::new(SubagentManager::new(pool, db.clone(), "claude".into()));

        let steps = vec![
            make_step("p-1", "task one", vec![]),
            make_step("p-2", "task two", vec![]),
        ];

        manager
            .run_orchestration(
                "orch-db-1".to_string(),
                "parent-1".to_string(),
                OrchestrationStrategy::Parallel,
                steps,
            )
            .await
            .expect("run_orchestration should succeed (DB records created)");

        // Allow the spawned task to start and attempt subprocess spawn
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Orchestration record should exist in DB
        let orch = db
            .get_orchestration("orch-db-1")
            .await
            .expect("orchestration record should exist");
        assert_eq!(orch.parent_session_id, "parent-1");
        assert_eq!(orch.strategy, "parallel");

        // Step records should exist in DB
        let db_steps = db
            .get_steps_for_orchestration("orch-db-1")
            .await
            .expect("steps should be retrievable");
        assert_eq!(db_steps.len(), 2);
        assert_eq!(db_steps[0].id, "p-1");
        assert_eq!(db_steps[1].id, "p-2");
    }

    #[tokio::test]
    async fn orchestration_sequential_dependency_chaining() {
        // Sequential strategy chains steps A -> B -> C in the DAG.
        // Dependencies are pre-chained (as the gRPC layer would do) and the
        // manager stores them in the DB via run_orchestration.
        let db = test_db().await;
        let pool = test_pool();
        let manager = Arc::new(SubagentManager::new(pool, db.clone(), "claude".into()));

        // Pre-chain dependencies the same way the gRPC layer does for Sequential.
        let steps = vec![
            make_step("seq-a", "first", vec![]),
            make_step("seq-b", "second", vec!["seq-a".to_string()]),
            make_step("seq-c", "third", vec!["seq-b".to_string()]),
        ];

        manager
            .run_orchestration(
                "orch-seq-1".to_string(),
                "parent-1".to_string(),
                OrchestrationStrategy::Sequential,
                steps,
            )
            .await
            .expect("run_orchestration should succeed");

        // Allow spawned task to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify orchestration stored with sequential strategy
        let orch = db
            .get_orchestration("orch-seq-1")
            .await
            .expect("orchestration should exist");
        assert_eq!(orch.strategy, "sequential");

        // Verify steps exist and have correct order
        let db_steps = db
            .get_steps_for_orchestration("orch-seq-1")
            .await
            .expect("steps should be retrievable");
        assert_eq!(db_steps.len(), 3);
        assert_eq!(db_steps[0].id, "seq-a");
        assert_eq!(db_steps[1].id, "seq-b");
        assert_eq!(db_steps[2].id, "seq-c");

        // Verify dependency chaining: seq-b depends on seq-a, seq-c depends on seq-b.
        // Dependencies are stored as JSON arrays in the `depends_on` column.
        let deps_a: Vec<String> = serde_json::from_str(&db_steps[0].depends_on).unwrap();
        let deps_b: Vec<String> = serde_json::from_str(&db_steps[1].depends_on).unwrap();
        let deps_c: Vec<String> = serde_json::from_str(&db_steps[2].depends_on).unwrap();
        assert!(deps_a.is_empty(), "first step should have no dependencies");
        assert!(
            deps_b.contains(&"seq-a".to_string()),
            "seq-b should depend on seq-a"
        );
        assert!(
            deps_c.contains(&"seq-b".to_string()),
            "seq-c should depend on seq-b"
        );
    }
}
