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
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::app::App;
use crate::app::{CompletionFetchKind, CompletionRequest};
use crate::commands::cache::CachedCommand;
use crate::connection::DaemonConnection;
use crate::ui;

/// Response from the async completion fetcher.
struct CompletionResponse {
    items: Vec<String>,
}

/// A slash-command to execute via the `CommandService`.
pub struct ServiceCommandExec {
    pub command: String,
    pub args: Vec<String>,
}

/// Terminal events forwarded from the UI reader thread.
pub enum TermEvent {
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize(u16, u16),
}

/// Spawn a one-shot task that fetches the command registry from the daemon and
/// sends the result through `tx`. Does nothing if `cmd_client` is `None`.
fn spawn_registry_fetch(
    cmd_client: Option<
        betcode_proto::v1::command_service_client::CommandServiceClient<tonic::transport::Channel>,
    >,
    auth_token: Option<String>,
    machine_id: Option<String>,
    tx: tokio::sync::mpsc::Sender<Vec<CachedCommand>>,
) {
    let Some(mut client) = cmd_client else {
        return;
    };
    tokio::spawn(async move {
        let mut request = tonic::Request::new(betcode_proto::v1::GetCommandRegistryRequest {
            // TODO(Task 7): pass actual session_id from app state
            session_id: String::new(),
        });
        crate::connection::attach_relay_metadata(
            &mut request,
            auth_token.as_deref(),
            machine_id.as_deref(),
        );
        match client.get_command_registry(request).await {
            Ok(resp) => {
                let cached: Vec<CachedCommand> = resp
                    .into_inner()
                    .commands
                    .into_iter()
                    .map(|c| {
                        let category = betcode_proto::v1::CommandCategory::try_from(c.category)
                            .map_or_else(|_| "Unknown".to_string(), |cat| format!("{cat:?}"));
                        CachedCommand {
                            name: c.name,
                            description: c.description,
                            category,
                            source: c.source,
                        }
                    })
                    .collect();
                let _ = tx.send(cached).await;
            }
            Err(e) => {
                tracing::warn!(
                    ?e,
                    "Could not fetch command registry, completions may be limited"
                );
            }
        }
    });
}

/// Run the interactive TUI mode.
///
/// Establishes the gRPC stream, enters raw mode, spawns a dedicated terminal
/// reader thread, and runs the main `select!` loop until the user quits.
///
/// # Panics
///
/// Panics if the current working directory cannot be accessed.
#[allow(
    clippy::too_many_lines,
    clippy::expect_used,
    clippy::option_if_let_else
)]
pub async fn run(
    conn: &mut DaemonConnection,
    session_id: &Option<String>,
    working_dir: &Option<String>,
    model: &Option<String>,
) -> anyhow::Result<()> {
    // 0. Key exchange for relay connections (before entering raw mode so
    //    Ctrl+C works during the handshake and fingerprint errors are visible).
    conn.ensure_relay_key_exchange().await?;

    // 1. Establish gRPC stream BEFORE entering raw mode so Ctrl+C works
    //    during the (potentially slow) handshake.
    let (request_tx, mut event_rx, stream_handle) = conn.converse().await?;

    let sid = session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let wd = working_dir.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .expect("current directory must be accessible")
            .to_string_lossy()
            .to_string()
    });

    request_tx
        .send(crate::connection::start_conversation_request(
            sid.clone(),
            wd,
            model.clone().unwrap_or_default(),
        ))
        .await?;

    // 2. Enter raw mode, create terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
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
                    Ok(Event::Mouse(mouse)) => {
                        if term_tx.blocking_send(TermEvent::Mouse(mouse)).is_err() {
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
    if conn.is_relay() {
        app.connection_type = "relay".to_string();
    }
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

    // 6. Fetch command registry in background (non-blocking so UI starts immediately)
    let (cmd_registry_tx, mut cmd_registry_rx) =
        tokio::sync::mpsc::channel::<Vec<CachedCommand>>(4);
    // Keep connection details for re-fetching the registry on SessionInfo events.
    let registry_cmd_client = conn.command_service_client();
    let registry_auth_token = conn.auth_token().cloned();
    let registry_machine_id = conn.machine_id().map(std::string::ToString::to_string);
    spawn_registry_fetch(
        registry_cmd_client.clone(),
        registry_auth_token.clone(),
        registry_machine_id.clone(),
        cmd_registry_tx.clone(),
    );

    // 7. Spawn async completion fetcher task
    let (completion_req_tx, mut completion_req_rx) =
        tokio::sync::mpsc::channel::<CompletionRequest>(16);
    let (completion_resp_tx, mut completion_resp_rx) =
        tokio::sync::mpsc::channel::<CompletionResponse>(16);
    app.completion_request_tx = Some(completion_req_tx);

    let completion_handle = conn.command_service_client().map(|mut cmd_client| {
        let comp_auth_token = conn.auth_token().cloned();
        let comp_machine_id = conn.machine_id().map(std::string::ToString::to_string);
        tokio::spawn(async move {
            let debounce = Duration::from_millis(100);
            let mut last_request_time = Instant::now()
                .checked_sub(debounce)
                .unwrap_or_else(Instant::now);

            while let Some(req) = completion_req_rx.recv().await {
                // Debounce: drain any newer requests that arrived
                let mut latest = req;
                while let Ok(newer) = completion_req_rx.try_recv() {
                    latest = newer;
                }

                // Wait for debounce period since last request
                let elapsed = last_request_time.elapsed();
                if elapsed < debounce {
                    tokio::time::sleep(debounce.checked_sub(elapsed).unwrap_or(Duration::ZERO))
                        .await;
                    // Drain again after sleep
                    while let Ok(newer) = completion_req_rx.try_recv() {
                        latest = newer;
                    }
                }
                last_request_time = Instant::now();

                let items = match latest.kind {
                    CompletionFetchKind::Agents => {
                        let mut request =
                            tonic::Request::new(betcode_proto::v1::ListAgentsRequest {
                                query: latest.query,
                                max_results: 50,
                            });
                        crate::connection::attach_relay_metadata(
                            &mut request,
                            comp_auth_token.as_deref(),
                            comp_machine_id.as_deref(),
                        );
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
                        let mut request = tonic::Request::new(betcode_proto::v1::ListPathRequest {
                            query: latest.query,
                            max_results: 50,
                        });
                        crate::connection::attach_relay_metadata(
                            &mut request,
                            comp_auth_token.as_deref(),
                            comp_machine_id.as_deref(),
                        );
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
                    CompletionFetchKind::Mixed => {
                        // Fetch both agents and files in parallel, combine results.
                        let mut agent_req =
                            tonic::Request::new(betcode_proto::v1::ListAgentsRequest {
                                query: latest.query.clone(),
                                max_results: 50,
                            });
                        crate::connection::attach_relay_metadata(
                            &mut agent_req,
                            comp_auth_token.as_deref(),
                            comp_machine_id.as_deref(),
                        );
                        let mut file_req =
                            tonic::Request::new(betcode_proto::v1::ListPathRequest {
                                query: latest.query,
                                max_results: 50,
                            });
                        crate::connection::attach_relay_metadata(
                            &mut file_req,
                            comp_auth_token.as_deref(),
                            comp_machine_id.as_deref(),
                        );

                        // Cannot tokio::join! two calls on the same &mut client,
                        // so call sequentially.
                        let agents: Vec<String> = match cmd_client.list_agents(agent_req).await {
                            Ok(resp) => resp
                                .into_inner()
                                .agents
                                .into_iter()
                                .map(|a| a.name)
                                .collect(),
                            Err(_) => Vec::new(),
                        };
                        let files: Vec<String> = match cmd_client.list_path(file_req).await {
                            Ok(resp) => resp
                                .into_inner()
                                .entries
                                .into_iter()
                                .map(|p| p.path)
                                .collect(),
                            Err(_) => Vec::new(),
                        };

                        // Agents first, then files
                        let mut combined = agents;
                        combined.extend(files);
                        combined
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

    // 8. Service command execution channel
    let (svc_cmd_tx, mut svc_cmd_rx) = tokio::sync::mpsc::channel::<ServiceCommandExec>(16);
    let (svc_cmd_result_tx, mut svc_cmd_result_rx) = tokio::sync::mpsc::channel::<String>(64);
    app.service_command_tx = Some(svc_cmd_tx);

    let svc_cmd_handle = conn.command_service_client().map(|mut cmd_client| {
        let svc_auth_token = conn.auth_token().cloned();
        let svc_machine_id = conn.machine_id().map(std::string::ToString::to_string);
        tokio::spawn(async move {
            while let Some(exec) = svc_cmd_rx.recv().await {
                let mut request =
                    tonic::Request::new(betcode_proto::v1::ExecuteServiceCommandRequest {
                        command: exec.command.clone(),
                        args: exec.args,
                        // TODO(Task 7): pass actual session_id from app state
                        session_id: String::new(),
                    });
                crate::connection::attach_relay_metadata(
                    &mut request,
                    svc_auth_token.as_deref(),
                    svc_machine_id.as_deref(),
                );
                match cmd_client.execute_service_command(request).await {
                    Ok(resp) => {
                        use betcode_proto::v1::service_command_output::Output;
                        use tokio_stream::StreamExt;
                        let mut stream = resp.into_inner();
                        while let Some(Ok(output)) = stream.next().await {
                            let line = match output.output {
                                Some(Output::StdoutLine(s)) => s,
                                Some(Output::StderrLine(s)) => format!("[stderr] {s}"),
                                Some(Output::ExitCode(c)) => format!("[exit code: {c}]"),
                                Some(Output::Error(e)) => format!("[error] {e}"),
                                None => continue,
                            };
                            if svc_cmd_result_tx.send(line).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = svc_cmd_result_tx
                            .send(format!("[error] {}", e.message()))
                            .await;
                    }
                }
            }
        })
    });

    let mut tick = tokio::time::interval(Duration::from_millis(50));

    let result: anyhow::Result<()> = loop {
        tokio::select! {
            _ = tick.tick() => {
                app.spinner_tick = app.spinner_tick.wrapping_add(1);
                terminal.draw(|f| ui::draw(f, &mut app))?;
            }
            Some(term_event) = term_rx.recv() => {
                input::handle_term_event(&mut app, &request_tx, term_event).await;
            }
            Some(cached) = cmd_registry_rx.recv() => {
                app.command_cache.load(cached);
            }
            Some(resp) = completion_resp_rx.recv() => {
                app.completion_state.items = resp.items;
                app.completion_state.selected_index = 0;
                app.completion_state.scroll_offset = 0;
                app.completion_state.ghost_text =
                    app.completion_state.items.first().cloned();
                app.completion_state.popup_visible =
                    !app.completion_state.items.is_empty();
            }
            Some(line) = svc_cmd_result_rx.recv() => {
                app.add_system_message(crate::app::MessageRole::System, line);
                app.scroll_to_bottom();
            }
            Some(grpc_result) = event_rx.recv() => {
                match grpc_result {
                    Ok(event) => {
                        // Re-fetch the command registry when a SessionInfo event
                        // arrives — MCP tools may have been merged into the
                        // registry during session initialisation.
                        if matches!(
                            event.event,
                            Some(betcode_proto::v1::agent_event::Event::SessionInfo(_))
                        ) {
                            spawn_registry_fetch(
                                registry_cmd_client.clone(),
                                registry_auth_token.clone(),
                                registry_machine_id.clone(),
                                cmd_registry_tx.clone(),
                            );
                        }
                        app.handle_event(event);
                    }
                    Err(e) => {
                        error!(?e, "Daemon stream error");
                        let msg = e.message();
                        app.status = if msg.contains("broken pipe")
                            || msg.contains("connection reset")
                            || msg.contains("h2 protocol error")
                        {
                            "Disconnected: daemon connection lost".to_string()
                        } else {
                            format!("Error: {msg}")
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

    // 9. Restore terminal first so the user gets their shell back immediately
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = terminal.show_cursor();

    if let Err(ref e) = result {
        tracing::error!(%e, "TUI error");
    }

    // 10. Shutdown: signal UI thread to stop, clean up gRPC resources
    cancel.cancel();
    let _ = ui_thread.join(); // fast — <50ms due to poll timeout

    // Drop the request sender to close the client side of the bidi stream.
    // This signals the server/relay that we're done.
    drop(request_tx);

    // Give the stream handle a brief window to close gracefully (reads the
    // server's stream-end), then abort if it hasn't finished.
    let _ = tokio::time::timeout(Duration::from_millis(500), stream_handle).await;

    drop(event_rx);
    drop(completion_resp_rx);
    drop(svc_cmd_result_rx);
    if let Some(h) = completion_handle {
        h.abort();
    }
    if let Some(h) = svc_cmd_handle {
        h.abort();
    }

    result
}
