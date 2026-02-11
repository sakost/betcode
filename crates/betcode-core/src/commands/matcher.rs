use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// A single fuzzy match result.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub text: String,
    pub score: u32,
    pub match_positions: Vec<usize>,
}

/// Performs fzf-style fuzzy matching of `query` against `items`.
///
/// Returns up to `max_results` results sorted by score (highest first).
/// Each result includes the matched text, score, and character positions
/// that matched (for UI highlighting).
pub fn fuzzy_match(query: &str, items: &[&str], max_results: usize) -> Vec<MatchResult> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let atom = Atom::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );

    let mut buf = Vec::new();
    let mut results: Vec<MatchResult> = items
        .iter()
        .filter_map(|item| {
            let mut indices = Vec::new();
            let haystack = Utf32Str::new(item, &mut buf);
            let score = atom.indices(haystack, &mut matcher, &mut indices)?;
            Some(MatchResult {
                text: item.to_string(),
                score: score as u32,
                match_positions: indices.into_iter().map(|i| i as usize).collect(),
            })
        })
        .collect();

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(max_results);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_match_exact() {
        let items = vec!["cd", "pwd", "exit", "exit-daemon", "reload-remote"];
        let results = fuzzy_match("cd", &items, 10);
        assert_eq!(results[0].text, "cd");
    }

    #[test]
    fn test_fuzzy_match_substring() {
        let items = vec!["cd", "pwd", "exit", "exit-daemon", "reload-remote"];
        let results = fuzzy_match("rl", &items, 10);
        assert!(results.iter().any(|r| r.text == "reload-remote"));
    }

    #[test]
    fn test_fuzzy_match_fzf_style() {
        let items = vec!["reload-remote", "remove-plugin", "restart"];
        let results = fuzzy_match("rr", &items, 10);
        assert_eq!(results[0].text, "reload-remote");
    }

    #[test]
    fn test_fuzzy_match_max_results() {
        let items: Vec<String> = (0..100).map(|i| format!("item-{}", i)).collect();
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        let results = fuzzy_match("item", &refs, 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_match_result_has_positions() {
        let items = vec!["reload-remote"];
        let results = fuzzy_match("rr", &items, 10);
        assert!(!results[0].match_positions.is_empty());
    }
}
