//! Two-thread TUI orchestration.
//!
//! Terminal I/O runs on a dedicated OS thread; all async/gRPC work stays on the
//! tokio runtime. Communication via `tokio::sync::mpsc` channels.

mod input;

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::app::App;
use crate::connection::DaemonConnection;
use crate::ui;
use betcode_proto::v1::{AgentRequest, StartConversation};

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
    let mut tick = tokio::time::interval(Duration::from_millis(50));

    let result: anyhow::Result<()> = loop {
        tokio::select! {
            _ = tick.tick() => {
                terminal.draw(|f| ui::draw(f, &mut app))?;
            }
            Some(term_event) = term_rx.recv() => {
                input::handle_term_event(&mut app, &request_tx, term_event).await;
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

    // 6. Shutdown: signal UI thread to stop, clean up gRPC resources
    cancel.cancel();
    let _ = ui_thread.join(); // fast — <50ms due to poll timeout
    drop(request_tx);
    drop(event_rx);
    stream_handle.abort();

    // 7. Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    if let Err(ref e) = result {
        eprintln!("Error: {e}");
    }

    // Safety net — gRPC channel drop can still block for HTTP/2 GOAWAY
    std::process::exit(if result.is_ok() { 0 } else { 1 })
}
