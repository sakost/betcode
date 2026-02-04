//! BetCode CLI
//!
//! Terminal interface for interacting with Claude Code through the daemon.
//! Provides both TUI (ratatui) and headless modes.

use std::io;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use betcode_cli::app::{App, AppMode};
use betcode_cli::connection::{ConnectionConfig, DaemonConnection};
use betcode_cli::headless::{self, HeadlessConfig};
use betcode_cli::ui;

use betcode_proto::v1::{
    AgentRequest, PermissionDecision, PermissionResponse, StartConversation, UserMessage,
};

#[derive(Parser, Debug)]
#[command(name = "betcode")]
#[command(version, about = "Claude Code multiplexer CLI", long_about = None)]
struct Cli {
    /// Prompt to send (enables headless mode if no --interactive)
    #[arg(short, long)]
    prompt: Option<String>,

    /// Session ID to resume (creates new if not specified)
    #[arg(short, long)]
    session: Option<String>,

    /// Working directory for the session
    #[arg(short = 'd', long)]
    working_dir: Option<String>,

    /// Model to use (e.g., "claude-sonnet-4")
    #[arg(short, long)]
    model: Option<String>,

    /// Daemon address
    #[arg(long, default_value = "http://127.0.0.1:50051")]
    daemon_addr: String,

    /// Auto-accept all permission prompts (headless only)
    #[arg(long)]
    yes: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Use file-based tracing for TUI mode to avoid polluting terminal
    let is_headless = cli.prompt.is_some();
    if is_headless {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode=info".into()),
            ))
            .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "betcode=warn".into()),
            ))
            .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
            .init();
    }

    info!(version = env!("CARGO_PKG_VERSION"), "Starting betcode CLI");

    // Connect to daemon
    let config = ConnectionConfig {
        addr: cli.daemon_addr.clone(),
        ..Default::default()
    };
    let mut conn = DaemonConnection::new(config);
    conn.connect().await?;

    if let Some(prompt) = cli.prompt {
        // Headless mode
        let working_dir = cli.working_dir.unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string()
        });

        let config = HeadlessConfig {
            prompt,
            session_id: cli.session,
            working_directory: working_dir,
            model: cli.model,
            auto_accept: cli.yes,
        };

        headless::run(&mut conn, config).await?;
    } else {
        // TUI mode
        run_tui(&mut conn, &cli.session, &cli.working_dir, &cli.model).await?;
    }

    Ok(())
}

/// Run the interactive TUI mode.
async fn run_tui(
    conn: &mut DaemonConnection,
    session_id: &Option<String>,
    working_dir: &Option<String>,
    model: &Option<String>,
) -> anyhow::Result<()> {
    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(&mut terminal, conn, session_id, working_dir, model).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main TUI event loop.
async fn run_tui_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    conn: &mut DaemonConnection,
    session_id: &Option<String>,
    working_dir: &Option<String>,
    model: &Option<String>,
) -> anyhow::Result<()> {
    let mut app = App::new();

    // Start conversation stream
    let (request_tx, mut event_rx) = conn.converse().await?;

    // Generate session ID
    let sid = session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let wd = working_dir.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string()
    });

    // Send StartConversation
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

    app.status = format!("Connected | Session: {}", &sid[..8.min(sid.len())]);

    loop {
        // Draw UI
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Poll for events with timeout so we can check for daemon events
        let has_terminal_event =
            tokio::task::block_in_place(|| event::poll(std::time::Duration::from_millis(50)))?;

        if has_terminal_event {
            let ev = tokio::task::block_in_place(event::read)?;
            if let Event::Key(key) = ev {
                // Ctrl+C to quit
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    app.should_quit = true;
                }
                // Handle permission prompts
                else if app.mode == AppMode::PermissionPrompt {
                    handle_permission_key(&mut app, &request_tx, key.code).await;
                }
                // Normal input mode
                else {
                    handle_input_key(&mut app, &request_tx, key.code).await;
                }
            }
        }

        // Drain daemon events (non-blocking)
        while let Ok(result) = event_rx.try_recv() {
            match result {
                Ok(event) => app.handle_event(event),
                Err(e) => {
                    error!(?e, "Daemon stream error");
                    app.status = format!("Error: {}", e.message());
                    break;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Handle a key press during a permission prompt.
async fn handle_permission_key(
    app: &mut App,
    tx: &tokio::sync::mpsc::Sender<AgentRequest>,
    code: KeyCode,
) {
    let decision = match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(PermissionDecision::AllowOnce),
        KeyCode::Char('n') | KeyCode::Char('N') => Some(PermissionDecision::Deny),
        KeyCode::Char('a') | KeyCode::Char('A') => Some(PermissionDecision::AllowSession),
        _ => None,
    };

    if let (Some(decision), Some(ref perm)) = (decision, &app.pending_permission) {
        let _ = tx
            .send(AgentRequest {
                request: Some(betcode_proto::v1::agent_request::Request::Permission(
                    PermissionResponse {
                        request_id: perm.request_id.clone(),
                        decision: decision.into(),
                    },
                )),
            })
            .await;
        app.pending_permission = None;
        app.mode = AppMode::Normal;
    }
}

/// Handle a key press in normal input mode.
async fn handle_input_key(
    app: &mut App,
    tx: &tokio::sync::mpsc::Sender<AgentRequest>,
    code: KeyCode,
) {
    match code {
        KeyCode::Enter => {
            if let Some(text) = app.submit_input() {
                let _ = tx
                    .send(AgentRequest {
                        request: Some(betcode_proto::v1::agent_request::Request::Message(
                            UserMessage {
                                content: text,
                                attachments: Vec::new(),
                            },
                        )),
                    })
                    .await;
                app.agent_busy = true;
            }
        }
        KeyCode::Char(c) => {
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += 1;
        }
        KeyCode::Backspace => {
            if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
                app.input.remove(app.cursor_pos);
            }
        }
        KeyCode::Left => {
            app.cursor_pos = app.cursor_pos.saturating_sub(1);
        }
        KeyCode::Right => {
            app.cursor_pos = (app.cursor_pos + 1).min(app.input.len());
        }
        KeyCode::Up => app.history_up(),
        KeyCode::Down => app.history_down(),
        _ => {}
    }
}
