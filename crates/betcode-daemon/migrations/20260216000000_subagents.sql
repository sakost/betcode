-- Subagent orchestration tables
-- See docs/architecture/SUBAGENTS.md for detailed documentation

-- Subagents table: tracks daemon-orchestrated Claude subprocesses
CREATE TABLE IF NOT EXISTS subagents (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    prompt TEXT NOT NULL,
    model TEXT,
    max_turns INTEGER NOT NULL DEFAULT 10,
    auto_approve INTEGER NOT NULL DEFAULT 0 CHECK (auto_approve IN (0, 1)),
    allowed_tools TEXT NOT NULL DEFAULT '[]',
    working_directory TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    exit_code INTEGER,
    result_summary TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_subagents_parent ON subagents(parent_session_id);
CREATE INDEX IF NOT EXISTS idx_subagents_status ON subagents(status)
    WHERE status IN ('pending', 'running');

-- Orchestrations table: multi-step execution plans
CREATE TABLE IF NOT EXISTS orchestrations (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    strategy TEXT NOT NULL DEFAULT 'parallel'
        CHECK (strategy IN ('parallel', 'sequential', 'dag')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_orchestrations_parent ON orchestrations(parent_session_id);

-- Orchestration steps table: individual steps within an orchestration
CREATE TABLE IF NOT EXISTS orchestration_steps (
    id TEXT PRIMARY KEY,
    orchestration_id TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
    subagent_id TEXT REFERENCES subagents(id) ON DELETE SET NULL,
    step_index INTEGER NOT NULL,
    prompt TEXT NOT NULL,
    depends_on TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'blocked')),
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_orch_steps ON orchestration_steps(orchestration_id, step_index);
