use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

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
}

impl FileIndex {
    /// Create an empty file index.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            root: PathBuf::new(),
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
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns the index root path.
    pub fn root(&self) -> &Path {
        &self.root
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
        if let Ok(metadata) = std::fs::symlink_metadata(&full) {
            if metadata.is_symlink() {
                PathKind::Symlink
            } else if metadata.is_dir() {
                PathKind::Directory
            } else {
                PathKind::File
            }
        } else {
            PathKind::File
        }
    }
}

#[cfg(test)]
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
            std::fs::write(dir.path().join(format!("file{}.txt", i)), "").unwrap();
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
}
