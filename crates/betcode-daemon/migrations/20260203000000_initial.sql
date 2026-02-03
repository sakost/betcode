-- Initial schema for BetCode daemon database
-- See docs/architecture/SCHEMAS.md for detailed documentation

-- Enable foreign keys and WAL mode
PRAGMA foreign_keys = ON;

-- Sessions table: tracks Claude subprocess sessions
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    claude_session_id TEXT,
    worktree_id TEXT,
    status TEXT NOT NULL DEFAULT 'idle'
        CHECK (status IN ('idle', 'active', 'completed', 'error')),
    model TEXT NOT NULL,
    working_directory TEXT NOT NULL,
    input_lock_client TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    total_input_tokens INTEGER DEFAULT 0,
    total_output_tokens INTEGER DEFAULT 0,
    total_cost_usd REAL DEFAULT 0.0,
    last_message_preview TEXT,
    compaction_sequence INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_sessions_worktree ON sessions(worktree_id);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);

-- Messages table: stores NDJSON lines from Claude stdout
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    message_type TEXT NOT NULL
        CHECK (message_type IN (
            'system', 'assistant', 'user', 'result',
            'stream_event', 'control_request', 'control_response'
        )),
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_session_seq ON messages(session_id, sequence);

-- Worktrees table: tracks git worktrees
CREATE TABLE IF NOT EXISTS worktrees (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    branch TEXT NOT NULL,
    repo_path TEXT NOT NULL,
    setup_script TEXT,
    created_at INTEGER NOT NULL,
    last_active INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_worktrees_repo ON worktrees(repo_path);

-- Permission grants table: runtime permission decisions
CREATE TABLE IF NOT EXISTS permission_grants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    pattern TEXT,
    action TEXT NOT NULL CHECK (action IN ('allow', 'deny')),
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_grants_session_tool ON permission_grants(session_id, tool_name);

-- Connected clients table: tracks active gRPC connections
CREATE TABLE IF NOT EXISTS connected_clients (
    client_id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    client_type TEXT NOT NULL CHECK (client_type IN ('cli', 'flutter', 'headless')),
    has_input_lock INTEGER NOT NULL DEFAULT 0 CHECK (has_input_lock IN (0, 1)),
    connected_at INTEGER NOT NULL,
    last_heartbeat INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_clients_session ON connected_clients(session_id);
CREATE INDEX IF NOT EXISTS idx_clients_heartbeat ON connected_clients(last_heartbeat);

-- Todos table: task items from TodoWrite tool
CREATE TABLE IF NOT EXISTS todos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    description TEXT,
    active_form TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'in_progress', 'completed')),
    sequence INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_todos_session ON todos(session_id, sequence);
