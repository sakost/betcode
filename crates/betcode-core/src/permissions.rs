//! Permission rule matching engine.
//!
//! Evaluates tool permission requests against configured rules.
//! Rules are matched in priority order (first match wins).

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Permission rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Rule identifier.
    pub id: String,
    /// Tool name pattern (supports glob: "Bash", "mcp__*", "*").
    pub tool_pattern: String,
    /// Path pattern for file operations (supports glob).
    #[serde(default)]
    pub path_pattern: Option<String>,
    /// Action to take when matched.
    pub action: PermissionAction,
    /// Priority (lower = higher priority).
    #[serde(default)]
    pub priority: u32,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Source of this rule.
    #[serde(default)]
    pub source: RuleSource,
}

/// Permission action to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    /// Allow without prompting.
    Allow,
    /// Deny without prompting.
    Deny,
    /// Ask the user for each invocation.
    #[default]
    Ask,
    /// Ask once per session.
    AskSession,
}

/// Source of a permission rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RuleSource {
    /// Built-in default rules.
    #[default]
    Builtin,
    /// Global config rules.
    Global,
    /// Project-specific rules.
    Project,
    /// Session runtime grants.
    Session,
}

/// Permission engine for evaluating tool requests.
#[derive(Debug, Default)]
pub struct PermissionEngine {
    rules: Vec<PermissionRule>,
}

impl PermissionEngine {
    /// Create a new permission engine with default rules.
    pub fn new() -> Self {
        Self {
            rules: default_rules(),
        }
    }

    /// Create an engine with custom rules.
    pub fn with_rules(mut rules: Vec<PermissionRule>) -> Self {
        rules.sort_by_key(|r| r.priority);
        Self { rules }
    }

    /// Add rules from a source (merges with existing).
    pub fn add_rules(&mut self, rules: Vec<PermissionRule>) {
        self.rules.extend(rules);
        self.rules.sort_by_key(|r| r.priority);
    }

    /// Evaluate a tool request against rules.
    pub fn evaluate(&self, tool_name: &str, path: Option<&Path>) -> PermissionDecision {
        for rule in &self.rules {
            if matches_tool(&rule.tool_pattern, tool_name) {
                if let Some(path_pattern) = &rule.path_pattern {
                    if let Some(p) = path {
                        if !matches_path(path_pattern, p) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                return PermissionDecision {
                    action: rule.action,
                    rule_id: Some(rule.id.clone()),
                    reason: rule.description.clone(),
                };
            }
        }
        PermissionDecision::default()
    }

    /// Get all rules.
    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }
}

/// Result of permission evaluation.
#[derive(Debug, Clone, Default)]
pub struct PermissionDecision {
    /// The action to take.
    pub action: PermissionAction,
    /// ID of the rule that matched (if any).
    pub rule_id: Option<String>,
    /// Reason for the decision.
    pub reason: Option<String>,
}

/// Check if a tool name matches a pattern.
fn matches_tool(pattern: &str, tool_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return tool_name.starts_with(prefix);
    }
    pattern == tool_name
}

/// Check if a path matches a glob pattern.
fn matches_path(pattern: &str, path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    if pattern == "*" || pattern == "**" {
        return true;
    }
    if pattern.ends_with("/**") {
        let prefix = &pattern[..pattern.len() - 3];
        return path_str.starts_with(prefix);
    }
    if pattern.ends_with("/*") {
        let prefix = &pattern[..pattern.len() - 2];
        if let Some(parent) = path.parent() {
            return parent.to_string_lossy() == prefix;
        }
    }
    path_str == pattern
}

/// Built-in default permission rules.
fn default_rules() -> Vec<PermissionRule> {
    vec![
        PermissionRule {
            id: "builtin-read-allow".to_string(),
            tool_pattern: "Read".to_string(),
            path_pattern: None,
            action: PermissionAction::Allow,
            priority: 100,
            description: Some("Allow reading files".to_string()),
            source: RuleSource::Builtin,
        },
        PermissionRule {
            id: "builtin-glob-allow".to_string(),
            tool_pattern: "Glob".to_string(),
            path_pattern: None,
            action: PermissionAction::Allow,
            priority: 100,
            description: Some("Allow file globbing".to_string()),
            source: RuleSource::Builtin,
        },
        PermissionRule {
            id: "builtin-grep-allow".to_string(),
            tool_pattern: "Grep".to_string(),
            path_pattern: None,
            action: PermissionAction::Allow,
            priority: 100,
            description: Some("Allow grep search".to_string()),
            source: RuleSource::Builtin,
        },
        PermissionRule {
            id: "builtin-bash-ask".to_string(),
            tool_pattern: "Bash".to_string(),
            path_pattern: None,
            action: PermissionAction::Ask,
            priority: 200,
            description: Some("Ask for bash commands".to_string()),
            source: RuleSource::Builtin,
        },
        PermissionRule {
            id: "builtin-write-ask".to_string(),
            tool_pattern: "Write".to_string(),
            path_pattern: None,
            action: PermissionAction::Ask,
            priority: 200,
            description: Some("Ask for file writes".to_string()),
            source: RuleSource::Builtin,
        },
        PermissionRule {
            id: "builtin-edit-ask".to_string(),
            tool_pattern: "Edit".to_string(),
            path_pattern: None,
            action: PermissionAction::Ask,
            priority: 200,
            description: Some("Ask for file edits".to_string()),
            source: RuleSource::Builtin,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_tool() {
        assert!(matches_tool("Bash", "Bash"));
        assert!(!matches_tool("Bash", "Read"));
    }

    #[test]
    fn matches_wildcard_tool() {
        assert!(matches_tool("mcp__*", "mcp__github__create_pr"));
        assert!(!matches_tool("mcp__*", "Bash"));
    }

    #[test]
    fn matches_star_all() {
        assert!(matches_tool("*", "anything"));
    }

    #[test]
    fn engine_allows_read_by_default() {
        let engine = PermissionEngine::new();
        let decision = engine.evaluate("Read", None);
        assert_eq!(decision.action, PermissionAction::Allow);
    }

    #[test]
    fn engine_asks_for_bash_by_default() {
        let engine = PermissionEngine::new();
        let decision = engine.evaluate("Bash", None);
        assert_eq!(decision.action, PermissionAction::Ask);
    }
}
