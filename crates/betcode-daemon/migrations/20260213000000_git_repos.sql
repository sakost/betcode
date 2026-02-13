-- GitRepo entity: registered repositories with per-repo worktree config
CREATE TABLE IF NOT EXISTS git_repos (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    repo_path       TEXT NOT NULL UNIQUE,
    worktree_mode   TEXT NOT NULL DEFAULT 'global'
                    CHECK (worktree_mode IN ('global', 'local', 'custom')),
    local_subfolder TEXT NOT NULL DEFAULT '.worktree',
    custom_path     TEXT,
    setup_script    TEXT,
    auto_gitignore  INTEGER NOT NULL DEFAULT 1 CHECK (auto_gitignore IN (0, 1)),
    created_at      INTEGER NOT NULL,
    last_active     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_git_repos_path ON git_repos(repo_path);

-- Recreate worktrees table with repo_id FK instead of repo_path
DROP INDEX IF EXISTS idx_worktrees_repo;
DROP TABLE IF EXISTS worktrees;

CREATE TABLE IF NOT EXISTS worktrees (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    path            TEXT NOT NULL UNIQUE,
    branch          TEXT NOT NULL,
    repo_id         TEXT NOT NULL REFERENCES git_repos(id) ON DELETE CASCADE,
    setup_script    TEXT,
    created_at      INTEGER NOT NULL,
    last_active     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_worktrees_repo_id ON worktrees(repo_id);
