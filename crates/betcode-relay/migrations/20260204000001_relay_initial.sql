-- Users table: relay authentication accounts
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username);

-- Tokens table: refresh token tracking
CREATE TABLE IF NOT EXISTS tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tokens_user ON tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_tokens_hash ON tokens(token_hash);

-- Machines table: registered daemon machines
CREATE TABLE IF NOT EXISTS machines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'offline'
        CHECK (status IN ('online', 'offline')),
    registered_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_machines_owner ON machines(owner_id);
CREATE INDEX IF NOT EXISTS idx_machines_status ON machines(status) WHERE status = 'online';

-- Message buffer table: buffered requests for offline machines
CREATE TABLE IF NOT EXISTS message_buffer (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    method TEXT NOT NULL,
    payload BLOB NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    priority INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_buffer_machine ON message_buffer(machine_id, priority DESC, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_buffer_expires ON message_buffer(expires_at);

-- Certificates table: TLS certificate tracking
CREATE TABLE IF NOT EXISTS certificates (
    id TEXT PRIMARY KEY,
    machine_id TEXT REFERENCES machines(id) ON DELETE CASCADE,
    subject_cn TEXT NOT NULL,
    serial_number TEXT NOT NULL,
    not_before INTEGER NOT NULL,
    not_after INTEGER NOT NULL,
    pem_cert TEXT NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_certs_machine ON certificates(machine_id);
CREATE INDEX IF NOT EXISTS idx_certs_serial ON certificates(serial_number);
