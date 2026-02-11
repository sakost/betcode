use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// The kind of a filesystem entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathKind {
    File,
    Directory,
    Symlink,
}

/// A single indexed filesystem path.
#[derive(Debug, Clone)]
pub struct IndexedPath {
    /// Path relative to the index root.
    pub path: String,
    pub kind: PathKind,
}

/// An index of filesystem paths for completion.
pub struct FileIndex {
    entries: Vec<IndexedPath>,
    root: PathBuf,
    /// Shared entries for watching; populated by `start_watching`.
    shared_entries: Option<Arc<RwLock<Vec<IndexedPath>>>>,
    /// Kept alive to maintain the watcher; dropped when `FileIndex` is dropped.
    _watcher: Option<RecommendedWatcher>,
}

impl FileIndex {
    /// Create an empty file index.
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
            root: PathBuf::new(),
            shared_entries: None,
            _watcher: None,
        }
    }

    /// Build a file index from the given root directory.
    ///
    /// Tries `git ls-files` first, falling back to a directory walk.
    pub async fn build(root: &Path, max_entries: usize) -> Result<Self> {
        let entries = match Self::build_from_git(root, max_entries).await {
            Ok(entries) => entries,
            Err(_) => Self::build_from_walkdir(root, max_entries)?,
        };

        Ok(Self {
            entries,
            root: root.to_path_buf(),
            shared_entries: None,
            _watcher: None,
        })
    }

    /// Search for paths containing the query substring (case-insensitive).
    pub fn search(&self, query: &str, max_results: usize) -> Vec<IndexedPath> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.path.to_lowercase().contains(&query_lower))
            .take(max_results)
            .cloned()
            .collect()
    }

    /// Returns the number of indexed entries.
    pub const fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns the index root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Start watching the root directory for filesystem changes.
    ///
    /// On create events new entries are added, on remove events entries are
    /// removed, and on rename events the old path is replaced with the new one.
    /// The watcher is kept alive as long as this `FileIndex` is alive.
    pub fn start_watching(&mut self) -> Result<()> {
        if self.root.as_os_str().is_empty() {
            anyhow::bail!("Cannot watch empty root");
        }

        let shared = Arc::new(RwLock::new(self.entries.clone()));
        let shared_for_handler = Arc::clone(&shared);
        let root = self.root.clone();

        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                let event = match res {
                    Ok(event) => event,
                    Err(e) => {
                        warn!("File watcher error: {e}");
                        return;
                    }
                };

                let shared = shared_for_handler.clone();
                let root = root.clone();

                // Use blocking lock since this callback is called from a sync context.
                let mut entries = shared.blocking_write();
                match event.kind {
                    EventKind::Create(_) => {
                        for path in &event.paths {
                            if let Ok(relative) = path.strip_prefix(&root) {
                                let rel_str = relative.display().to_string();
                                if !entries.iter().any(|e| e.path == rel_str) {
                                    let kind = Self::classify_path(&root, &rel_str);
                                    debug!(path = %rel_str, "File index: added");
                                    entries.push(IndexedPath {
                                        path: rel_str,
                                        kind,
                                    });
                                }
                            }
                        }
                    }
                    EventKind::Remove(_) => {
                        for path in &event.paths {
                            if let Ok(relative) = path.strip_prefix(&root) {
                                let rel_str = relative.display().to_string();
                                debug!(path = %rel_str, "File index: removed");
                                entries.retain(|e| e.path != rel_str);
                            }
                        }
                    }
                    EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                        // Rename events: paths[0] is old, paths[1] is new (if both present).
                        if event.paths.len() == 2 {
                            let old = &event.paths[0];
                            let new = &event.paths[1];
                            if let (Ok(old_rel), Ok(new_rel)) =
                                (old.strip_prefix(&root), new.strip_prefix(&root))
                            {
                                let old_str = old_rel.display().to_string();
                                let new_str = new_rel.display().to_string();
                                debug!(from = %old_str, to = %new_str, "File index: renamed");
                                if let Some(entry) = entries.iter_mut().find(|e| e.path == old_str)
                                {
                                    entry.path.clone_from(&new_str);
                                    entry.kind = Self::classify_path(&root, &new_str);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            })?;

        watcher.watch(&self.root, RecursiveMode::Recursive)?;
        self.shared_entries = Some(shared);
        #[allow(clippy::used_underscore_binding)]
        {
            self._watcher = Some(watcher);
        }
        Ok(())
    }

    async fn build_from_git(root: &Path, max_entries: usize) -> Result<Vec<IndexedPath>> {
        // Get tracked files
        let tracked_output = Command::new("git")
            .args(["ls-files"])
            .current_dir(root)
            .output()
            .await
            .context("Failed to run git ls-files")?;

        if !tracked_output.status.success() {
            anyhow::bail!("git ls-files failed");
        }

        // Get untracked files (excluding gitignored)
        let untracked_output = Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .current_dir(root)
            .output()
            .await
            .context("Failed to run git ls-files --others")?;

        let tracked = String::from_utf8_lossy(&tracked_output.stdout);
        let untracked = String::from_utf8_lossy(&untracked_output.stdout);

        let mut entries = Vec::new();
        let mut dirs_seen = std::collections::HashSet::new();

        for line in tracked.lines().chain(untracked.lines()) {
            if entries.len() >= max_entries {
                break;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Add parent directories
            let path = Path::new(line);
            if let Some(parent) = path.parent() {
                let mut current = PathBuf::new();
                for component in parent.components() {
                    current.push(component);
                    let dir_str = current.display().to_string();
                    if dirs_seen.insert(dir_str.clone()) && entries.len() < max_entries {
                        entries.push(IndexedPath {
                            path: dir_str,
                            kind: PathKind::Directory,
                        });
                    }
                }
            }

            if entries.len() < max_entries {
                let kind = Self::classify_path(root, line);
                entries.push(IndexedPath {
                    path: line.to_string(),
                    kind,
                });
            }
        }

        Ok(entries)
    }

    fn build_from_walkdir(root: &Path, max_entries: usize) -> Result<Vec<IndexedPath>> {
        let mut entries = Vec::new();

        Self::walk_dir_recursive(root, root, max_entries, &mut entries)?;

        Ok(entries)
    }

    fn walk_dir_recursive(
        base: &Path,
        dir: &Path,
        max_entries: usize,
        entries: &mut Vec<IndexedPath>,
    ) -> Result<()> {
        let read_dir = std::fs::read_dir(dir).context("Failed to read directory")?;

        for entry in read_dir.flatten() {
            if entries.len() >= max_entries {
                return Ok(());
            }

            let path = entry.path();
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .display()
                .to_string();

            let metadata = entry.metadata()?;
            let kind = if metadata.is_symlink() {
                PathKind::Symlink
            } else if metadata.is_dir() {
                PathKind::Directory
            } else {
                PathKind::File
            };

            entries.push(IndexedPath {
                path: relative,
                kind: kind.clone(),
            });

            if kind == PathKind::Directory {
                Self::walk_dir_recursive(base, &path, max_entries, entries)?;
            }
        }

        Ok(())
    }

    fn classify_path(root: &Path, relative: &str) -> PathKind {
        let full = root.join(relative);
        std::fs::symlink_metadata(&full).map_or(PathKind::File, |metadata| {
            if metadata.is_symlink() {
                PathKind::Symlink
            } else if metadata.is_dir() {
                PathKind::Directory
            } else {
                PathKind::File
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used, clippy::used_underscore_binding)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_file_index_build() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file1.rs"), "").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        let index = FileIndex::build(dir.path(), 1000).await.unwrap();
        assert!(index
            .search("file1", 10)
            .iter()
            .any(|p| p.path.contains("file1.rs")));
        assert!(index
            .search("main", 10)
            .iter()
            .any(|p| p.path.contains("main.rs")));
    }

    #[tokio::test]
    async fn test_file_index_respects_max_entries() {
        let dir = TempDir::new().unwrap();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("file{i}.txt")), "").unwrap();
        }
        let index = FileIndex::build(dir.path(), 10).await.unwrap();
        assert!(index.entry_count() <= 10);
    }

    #[tokio::test]
    async fn test_file_index_returns_path_kind() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.rs"), "").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let index = FileIndex::build(dir.path(), 1000).await.unwrap();
        let files = index.search("file.rs", 10);
        assert_eq!(files[0].kind, PathKind::File);
    }

    #[tokio::test]
    async fn test_start_watching() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("initial.txt"), "").unwrap();
        let mut index = FileIndex::build(dir.path(), 1000).await.unwrap();
        assert!(index.start_watching().is_ok());
        // Watcher is alive; creating a file should eventually be picked up.
        // We just verify the watcher was created without error.
        assert!(index._watcher.is_some());
        assert!(index.shared_entries.is_some());
    }

    #[test]
    fn test_start_watching_empty_root_fails() {
        let mut index = FileIndex::empty();
        assert!(index.start_watching().is_err());
    }
}
