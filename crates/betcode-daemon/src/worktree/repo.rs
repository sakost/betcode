//! Git repository domain model with worktree path resolution.

use std::path::{Path, PathBuf};

use crate::storage::GitRepoRow;

/// Worktree storage mode for a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeMode {
    /// Worktrees stored under global base dir: `<global_base>/<repo_name>/<id>/`
    Global,
    /// Worktrees stored inside repo: `<repo_path>/<subfolder>/<id>/`
    Local,
    /// Worktrees stored at arbitrary path: `<custom_path>/<repo_name>/<id>/`
    Custom(PathBuf),
}

/// Git repository domain model.
#[derive(Debug, Clone)]
pub struct GitRepo {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub worktree_mode: WorktreeMode,
    pub local_subfolder: PathBuf,
    pub setup_script: Option<String>,
    pub auto_gitignore: bool,
    pub created_at: i64,
    pub last_active: i64,
}

impl GitRepo {
    /// Extract the repository directory name (last path component).
    /// Falls back to "unknown" for edge cases like `/`.
    pub fn repo_name(&self) -> &str {
        self.repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    }

    /// Compute the base directory for worktrees of this repo.
    ///
    /// Individual worktrees are placed in `<base_dir>/<worktree_id>/`.
    pub fn worktree_base_dir(&self, global_base: &Path) -> PathBuf {
        match &self.worktree_mode {
            WorktreeMode::Global => global_base.join(self.repo_name()),
            WorktreeMode::Local => self.repo_path.join(&self.local_subfolder),
            WorktreeMode::Custom(path) => path.join(self.repo_name()),
        }
    }
}

impl From<GitRepoRow> for GitRepo {
    fn from(row: GitRepoRow) -> Self {
        let worktree_mode = match row.worktree_mode.as_str() {
            "local" => WorktreeMode::Local,
            "custom" => WorktreeMode::Custom(
                row.custom_path
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/tmp/betcode-worktrees")),
            ),
            _ => WorktreeMode::Global,
        };

        Self {
            id: row.id,
            name: row.name,
            repo_path: PathBuf::from(row.repo_path),
            worktree_mode,
            local_subfolder: PathBuf::from(row.local_subfolder),
            setup_script: row.setup_script,
            auto_gitignore: row.auto_gitignore != 0,
            created_at: row.created_at,
            last_active: row.last_active,
        }
    }
}

impl GitRepo {
    /// Convert back to DB fields for insert/update.
    pub fn worktree_mode_str(&self) -> &'static str {
        match &self.worktree_mode {
            WorktreeMode::Global => "global",
            WorktreeMode::Local => "local",
            WorktreeMode::Custom(_) => "custom",
        }
    }

    /// Extract the custom path (only meaningful for Custom mode).
    pub fn custom_path_str(&self) -> Option<String> {
        match &self.worktree_mode {
            WorktreeMode::Custom(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_mode_uses_global_base() {
        let repo = GitRepo {
            id: "r1".into(),
            name: "myrepo".into(),
            repo_path: PathBuf::from("/home/user/projects/myrepo"),
            worktree_mode: WorktreeMode::Global,
            local_subfolder: PathBuf::from(".worktree"),
            setup_script: None,
            auto_gitignore: true,
            created_at: 0,
            last_active: 0,
        };
        let base = repo.worktree_base_dir(Path::new("/home/user/.betcode/worktrees"));
        assert_eq!(base, PathBuf::from("/home/user/.betcode/worktrees/myrepo"));
    }

    #[test]
    fn local_mode_uses_repo_subfolder() {
        let repo = GitRepo {
            id: "r2".into(),
            name: "myrepo".into(),
            repo_path: PathBuf::from("/home/user/projects/myrepo"),
            worktree_mode: WorktreeMode::Local,
            local_subfolder: PathBuf::from(".worktree"),
            setup_script: None,
            auto_gitignore: true,
            created_at: 0,
            last_active: 0,
        };
        let base = repo.worktree_base_dir(Path::new("/ignored"));
        assert_eq!(
            base,
            PathBuf::from("/home/user/projects/myrepo/.worktree")
        );
    }

    #[test]
    fn custom_mode_uses_custom_path() {
        let repo = GitRepo {
            id: "r3".into(),
            name: "myrepo".into(),
            repo_path: PathBuf::from("/home/user/projects/myrepo"),
            worktree_mode: WorktreeMode::Custom(PathBuf::from("/mnt/fast-ssd/worktrees")),
            local_subfolder: PathBuf::from(".worktree"),
            setup_script: None,
            auto_gitignore: false,
            created_at: 0,
            last_active: 0,
        };
        let base = repo.worktree_base_dir(Path::new("/ignored"));
        assert_eq!(
            base,
            PathBuf::from("/mnt/fast-ssd/worktrees/myrepo")
        );
    }

    #[test]
    fn repo_name_fallback_for_root() {
        let repo = GitRepo {
            id: "r4".into(),
            name: "root".into(),
            repo_path: PathBuf::from("/"),
            worktree_mode: WorktreeMode::Global,
            local_subfolder: PathBuf::from(".worktree"),
            setup_script: None,
            auto_gitignore: true,
            created_at: 0,
            last_active: 0,
        };
        assert_eq!(repo.repo_name(), "unknown");
    }

    #[test]
    fn from_row_global_mode() {
        let row = GitRepoRow {
            id: "r1".into(),
            name: "myrepo".into(),
            repo_path: "/path/to/repo".into(),
            worktree_mode: "global".into(),
            local_subfolder: ".worktree".into(),
            custom_path: None,
            setup_script: None,
            auto_gitignore: 1,
            created_at: 100,
            last_active: 200,
        };
        let repo = GitRepo::from(row);
        assert_eq!(repo.worktree_mode, WorktreeMode::Global);
        assert!(repo.auto_gitignore);
    }

    #[test]
    fn from_row_custom_mode() {
        let row = GitRepoRow {
            id: "r2".into(),
            name: "myrepo".into(),
            repo_path: "/path/to/repo".into(),
            worktree_mode: "custom".into(),
            local_subfolder: ".worktree".into(),
            custom_path: Some("/custom/base".into()),
            setup_script: Some("npm install".into()),
            auto_gitignore: 0,
            created_at: 100,
            last_active: 200,
        };
        let repo = GitRepo::from(row);
        assert_eq!(
            repo.worktree_mode,
            WorktreeMode::Custom(PathBuf::from("/custom/base"))
        );
        assert!(!repo.auto_gitignore);
        assert_eq!(repo.setup_script.as_deref(), Some("npm install"));
    }

    #[test]
    fn mode_str_roundtrip() {
        let repo = GitRepo {
            id: "r5".into(),
            name: "test".into(),
            repo_path: PathBuf::from("/repo"),
            worktree_mode: WorktreeMode::Local,
            local_subfolder: PathBuf::from(".wt"),
            setup_script: None,
            auto_gitignore: true,
            created_at: 0,
            last_active: 0,
        };
        assert_eq!(repo.worktree_mode_str(), "local");
        assert!(repo.custom_path_str().is_none());
    }
}
