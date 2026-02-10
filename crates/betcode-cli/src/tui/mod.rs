//! Two-thread TUI orchestration.
//!
//! Terminal I/O runs on a dedicated OS thread; all async/gRPC work stays on the
//! tokio runtime. Communication via `tokio::sync::mpsc` channels.

pub mod fingerprint_panel;
mod input;
mod permission_input;
#[cfg(test)]
mod permission_tests;
mod question_input;
#[cfg(test)]
mod question_tests;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use crate::app::App;
use crate::app::{CompletionFetchKind, CompletionRequest};
use crate::commands::cache::CachedCommand;
use crate::connection::DaemonConnection;
use crate::ui;
use betcode_crypto::FingerprintCheck;
use betcode_proto::v1::{AgentRequest, StartConversation};

/// Response from the async completion fetcher.
struct CompletionResponse {
    items: Vec<String>,
}

/// Terminal events forwarded from the UI reader thread.
pub enum TermEvent {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
}

/// Run the interactive TUI mode.
///
/// Establishes the gRPC stream, enters raw mode, spawns a dedicated terminal
/// reader thread, and runs the main `select!` loop until the user quits.
pub async fn run(
    conn: &mut DaemonConnection,
    session_id: &Option<String>,
    working_dir: &Option<String>,
    model: &Option<String>,
) -> anyhow::Result<()> {
    // 0. Key exchange for relay connections (before entering raw mode so
    //    Ctrl+C works during the handshake and fingerprint errors are visible).
    if conn.is_relay() {
        let machine_id = conn.machine_id().unwrap_or("unknown").to_string();
        let (_daemon_fp, fp_check) = conn.exchange_keys(&machine_id).await?;
        match fp_check {
            FingerprintCheck::TrustOnFirstUse | FingerprintCheck::Matched => {
                // Proceed — fingerprint accepted
            }
            FingerprintCheck::Mismatch { expected, actual } => {
                return Err(anyhow::anyhow!(
                    "Daemon fingerprint mismatch!\n  Expected: {}\n  Actual:   {}\n\
                     This could indicate a MITM attack. Connection aborted.",
                    expected,
                    actual
                ));
            }
        }
    }

    // 1. Establish gRPC stream BEFORE entering raw mode so Ctrl+C works
    //    during the (potentially slow) handshake.
    let (request_tx, mut event_rx, stream_handle) = conn.converse().await?;

    let sid = session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let wd = working_dir.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string()
    });

    request_tx
        .send(AgentRequest {
            request: Some(betcode_proto::v1::agent_request::Request::Start(
                StartConversation {
                    session_id: sid.clone(),
                    working_directory: wd,
                    model: model.clone().unwrap_or_default(),
                    allowed_tools: Vec::new(),
                    plan_mode: false,
                    worktree_id: String::new(),
                    metadata: Default::default(),
                },
            )),
        })
        .await?;

    // 2. Enter raw mode, create terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 3. Channels + cancellation token
    let cancel = CancellationToken::new();
    let (term_tx, mut term_rx) = tokio::sync::mpsc::channel::<TermEvent>(64);

    // 4. Spawn dedicated OS thread for crossterm::event::read()
    let cancel_clone = cancel.clone();
    let ui_thread = std::thread::spawn(move || {
        loop {
            if cancel_clone.is_cancelled() {
                break;
            }
            // Poll with 50ms timeout so we can check cancellation
            if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        // Filter out Release events (Windows emits Press + Release per keystroke)
                        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                            continue;
                        }
                        if term_tx.blocking_send(TermEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Resize(w, h)) => {
                        if term_tx.blocking_send(TermEvent::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    // 5. Load conversation history if resuming an existing session
    let mut app = App::new();
    if session_id.is_some() {
        app.status = "Loading history...".to_string();
        // Draw once to show the loading status
        terminal.draw(|f| ui::draw(f, &mut app))?;

        match conn.resume_session(&sid, 0).await {
            Ok(events) => {
                let count = events.len();
                for event in events {
                    app.load_history_event(event);
                }
                app.finish_history_load();
                if count > 0 {
                    app.scroll_to_bottom();
                }
            }
            Err(e) => {
                // Non-fatal: session may be new or history unavailable
                tracing::warn!(?e, "Could not load session history");
            }
        }
    }
    app.status = format!("Connected | Session: {}", &sid[..8.min(sid.len())]);

    // 6. Fetch command registry and load into local cache
    match conn.get_command_registry().await {
        Ok(registry) => {
            let cached: Vec<CachedCommand> = registry
                .commands
                .into_iter()
                .map(|c| CachedCommand {
                    name: c.name,
                    description: c.description,
                    category: format!("{:?}", c.category),
                    source: c.source,
                })
                .collect();
            app.command_cache.load(cached);
        }
        Err(e) => {
            warn!(
                ?e,
                "Could not fetch command registry, completions may be limited"
            );
        }
    }

    // 7. Spawn async completion fetcher task
    let (completion_req_tx, mut completion_req_rx) =
        tokio::sync::mpsc::channel::<CompletionRequest>(16);
    let (completion_resp_tx, mut completion_resp_rx) =
        tokio::sync::mpsc::channel::<CompletionResponse>(16);
    app.completion_request_tx = Some(completion_req_tx);

    let completion_handle = conn.command_service_client().map(|mut cmd_client| {
        tokio::spawn(async move {
            let debounce = Duration::from_millis(100);
            let mut last_request_time = Instant::now() - debounce;

            while let Some(req) = completion_req_rx.recv().await {
                // Debounce: drain any newer requests that arrived
                let mut latest = req;
                while let Ok(newer) = completion_req_rx.try_recv() {
                    latest = newer;
                }

                // Wait for debounce period since last request
                let elapsed = last_request_time.elapsed();
                if elapsed < debounce {
                    tokio::time::sleep(debounce - elapsed).await;
                    // Drain again after sleep
                    while let Ok(newer) = completion_req_rx.try_recv() {
                        latest = newer;
                    }
                }
                last_request_time = Instant::now();

                let items = match latest.kind {
                    CompletionFetchKind::Agents => {
                        let request = tonic::Request::new(betcode_proto::v1::ListAgentsRequest {
                            query: latest.query,
                            max_results: 8,
                        });
                        match cmd_client.list_agents(request).await {
                            Ok(resp) => resp
                                .into_inner()
                                .agents
                                .into_iter()
                                .map(|a| a.name)
                                .collect(),
                            Err(_) => Vec::new(),
                        }
                    }
                    CompletionFetchKind::Files => {
                        let request = tonic::Request::new(betcode_proto::v1::ListPathRequest {
                            query: latest.query,
                            max_results: 8,
                        });
                        match cmd_client.list_path(request).await {
                            Ok(resp) => resp
                                .into_inner()
                                .entries
                                .into_iter()
                                .map(|p| p.path)
                                .collect(),
                            Err(_) => Vec::new(),
                        }
                    }
                };

                if completion_resp_tx
                    .send(CompletionResponse { items })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    });

    let mut tick = tokio::time::interval(Duration::from_millis(50));

    let result: anyhow::Result<()> = loop {
        tokio::select! {
            _ = tick.tick() => {
                terminal.draw(|f| ui::draw(f, &mut app))?;
            }
            Some(term_event) = term_rx.recv() => {
                input::handle_term_event(&mut app, &request_tx, term_event).await;
            }
            Some(resp) = completion_resp_rx.recv() => {
                app.completion_state.items = resp.items;
                app.completion_state.selected_index = 0;
                app.completion_state.ghost_text =
                    app.completion_state.items.first().cloned();
            }
            Some(grpc_result) = event_rx.recv() => {
                match grpc_result {
                    Ok(event) => app.handle_event(event),
                    Err(e) => {
                        error!(?e, "Daemon stream error");
                        let msg = e.message();
                        app.status = if msg.contains("broken pipe")
                            || msg.contains("connection reset")
                            || msg.contains("h2 protocol error")
                        {
                            "Disconnected: daemon connection lost".to_string()
                        } else {
                            format!("Error: {}", msg)
                        };
                        app.agent_busy = false;
                    }
                }
            }
        }
        if app.should_quit {
            break Ok(());
        }
    };

    // 8. Shutdown: signal UI thread to stop, clean up gRPC resources
    cancel.cancel();
    let _ = ui_thread.join(); // fast — <50ms due to poll timeout
    drop(request_tx);
    drop(event_rx);
    drop(completion_resp_rx);
    if let Some(h) = completion_handle {
        h.abort();
    }
    stream_handle.abort();

    // 9. Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    if let Err(ref e) = result {
        eprintln!("Error: {e}");
    }

    // Safety net — gRPC channel drop can still block for HTTP/2 GOAWAY
    std::process::exit(if result.is_ok() { 0 } else { 1 })
}
