use std::collections::HashMap;

/// The kind of agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentKind {
    ClaudeInternal,
    DaemonOrchestrated,
    TeamMember,
}

/// The current status of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Working,
    Done,
    Failed,
}

/// Information about a single agent.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub name: String,
    pub kind: AgentKind,
    pub status: AgentStatus,
    pub session_id: Option<String>,
}

/// Tracks known agents for @-prefix completion.
pub struct AgentLister {
    agents: HashMap<String, AgentInfo>,
}

impl AgentLister {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Insert or update an agent by name.
    pub fn update(&mut self, info: AgentInfo) {
        self.agents.insert(info.name.clone(), info);
    }

    /// Remove an agent by name.
    pub fn remove(&mut self, name: &str) {
        self.agents.remove(name);
    }

    /// Search for agents by name substring (case-insensitive).
    /// An empty query returns all agents up to `max_results`.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<AgentInfo> {
        let query_lower = query.to_lowercase();
        self.agents
            .values()
            .filter(|a| query.is_empty() || a.name.to_lowercase().contains(&query_lower))
            .take(max_results)
            .cloned()
            .collect()
    }
}

impl Default for AgentLister {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_info_creation() {
        let agent = AgentInfo {
            name: "researcher".to_string(),
            kind: AgentKind::ClaudeInternal,
            status: AgentStatus::Working,
            session_id: Some("sess-123".to_string()),
        };
        assert_eq!(agent.name, "researcher");
    }

    #[test]
    fn test_agent_lister_search() {
        let mut lister = AgentLister::new();
        lister.update(AgentInfo {
            name: "researcher".to_string(),
            kind: AgentKind::ClaudeInternal,
            status: AgentStatus::Working,
            session_id: None,
        });
        lister.update(AgentInfo {
            name: "team-lead".to_string(),
            kind: AgentKind::TeamMember,
            status: AgentStatus::Idle,
            session_id: None,
        });
        let results = lister.search("res", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "researcher");
    }

    #[test]
    fn test_agent_lister_empty_query_returns_all() {
        let mut lister = AgentLister::new();
        lister.update(AgentInfo {
            name: "a".to_string(),
            kind: AgentKind::ClaudeInternal,
            status: AgentStatus::Idle,
            session_id: None,
        });
        lister.update(AgentInfo {
            name: "b".to_string(),
            kind: AgentKind::TeamMember,
            status: AgentStatus::Working,
            session_id: None,
        });
        let results = lister.search("", 10);
        assert_eq!(results.len(), 2);
    }
}
