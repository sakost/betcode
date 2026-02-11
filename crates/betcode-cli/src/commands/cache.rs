//! Local command cache for completion.
//!
//! Stores command metadata fetched from the daemon and provides
//! fuzzy search using nucleo via betcode-core.

use betcode_core::commands::matcher::fuzzy_match;

/// A cached command entry for completion.
#[derive(Debug, Clone)]
pub struct CachedCommand {
    pub name: String,
    pub description: String,
    pub category: String,
    pub source: String,
}

/// Local cache of available commands, supporting fuzzy search.
#[derive(Debug, Default)]
pub struct CommandCache {
    entries: Vec<CachedCommand>,
}

impl CommandCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replace all cached entries.
    pub fn load(&mut self, entries: Vec<CachedCommand>) {
        self.entries = entries;
    }

    /// Look up a command by exact name.
    pub fn find_by_name(&self, name: &str) -> Option<&CachedCommand> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Return all cached entries.
    pub fn all(&self) -> &[CachedCommand] {
        &self.entries
    }

    /// Fuzzy-search cached commands by name, returning up to `max_results`.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<&CachedCommand> {
        if query.is_empty() {
            return self.entries.iter().take(max_results).collect();
        }

        let names: Vec<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
        let matches = fuzzy_match(query, &names, max_results);

        matches
            .iter()
            .filter_map(|m| self.entries.iter().find(|e| e.name == m.text))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<CachedCommand> {
        vec![
            CachedCommand {
                name: "cd".to_string(),
                description: "Change directory".to_string(),
                category: "service".to_string(),
                source: "builtin".to_string(),
            },
            CachedCommand {
                name: "pwd".to_string(),
                description: "Print working directory".to_string(),
                category: "service".to_string(),
                source: "builtin".to_string(),
            },
            CachedCommand {
                name: "reload-remote".to_string(),
                description: "Re-discover all commands".to_string(),
                category: "service".to_string(),
                source: "builtin".to_string(),
            },
            CachedCommand {
                name: "help".to_string(),
                description: "Show help".to_string(),
                category: "cc".to_string(),
                source: "claude-code".to_string(),
            },
        ]
    }

    #[test]
    fn cache_starts_empty() {
        let cache = CommandCache::new();
        assert!(cache.is_empty());
    }

    #[test]
    fn load_replaces_entries() {
        let mut cache = CommandCache::new();
        cache.load(sample_entries());
        assert!(!cache.is_empty());

        cache.load(vec![CachedCommand {
            name: "only".to_string(),
            description: String::new(),
            category: String::new(),
            source: String::new(),
        }]);
        assert_eq!(cache.search("", 10).len(), 1);
    }

    #[test]
    fn search_fuzzy_match() {
        let mut cache = CommandCache::new();
        cache.load(sample_entries());

        let results = cache.search("rr", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "reload-remote");
    }

    #[test]
    fn search_empty_query_returns_all() {
        let mut cache = CommandCache::new();
        cache.load(sample_entries());

        let results = cache.search("", 10);
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn search_respects_max_results() {
        let mut cache = CommandCache::new();
        cache.load(sample_entries());

        let results = cache.search("", 2);
        assert_eq!(results.len(), 2);
    }
}
