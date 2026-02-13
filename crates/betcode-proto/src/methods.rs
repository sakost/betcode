//! Named constants for gRPC method strings used across the tunnel protocol.
//!
//! These constants are derived from the protobuf service definitions and are
//! shared between the daemon (tunnel handler) and relay (proxy services) so
//! that method names stay in sync without duplicating string literals.

// ---------------------------------------------------------------------------
// AgentService
// ---------------------------------------------------------------------------

/// `AgentService/ListSessions`
pub const METHOD_LIST_SESSIONS: &str = "AgentService/ListSessions";

/// `AgentService/CompactSession`
pub const METHOD_COMPACT_SESSION: &str = "AgentService/CompactSession";

/// `AgentService/CancelTurn`
pub const METHOD_CANCEL_TURN: &str = "AgentService/CancelTurn";

/// `AgentService/RequestInputLock`
pub const METHOD_REQUEST_INPUT_LOCK: &str = "AgentService/RequestInputLock";

/// `AgentService/Converse`
pub const METHOD_CONVERSE: &str = "AgentService/Converse";

/// `AgentService/ResumeSession`
pub const METHOD_RESUME_SESSION: &str = "AgentService/ResumeSession";

/// `AgentService/ExchangeKeys`
pub const METHOD_EXCHANGE_KEYS: &str = "AgentService/ExchangeKeys";

// ---------------------------------------------------------------------------
// CommandService
// ---------------------------------------------------------------------------

/// `CommandService/GetCommandRegistry`
pub const METHOD_GET_COMMAND_REGISTRY: &str = "CommandService/GetCommandRegistry";

/// `CommandService/ListAgents`
pub const METHOD_LIST_AGENTS: &str = "CommandService/ListAgents";

/// `CommandService/ListPath`
pub const METHOD_LIST_PATH: &str = "CommandService/ListPath";

/// `CommandService/ExecuteServiceCommand`
pub const METHOD_EXECUTE_SERVICE_COMMAND: &str = "CommandService/ExecuteServiceCommand";

/// `CommandService/ListPlugins`
pub const METHOD_LIST_PLUGINS: &str = "CommandService/ListPlugins";

/// `CommandService/GetPluginStatus`
pub const METHOD_GET_PLUGIN_STATUS: &str = "CommandService/GetPluginStatus";

/// `CommandService/AddPlugin`
pub const METHOD_ADD_PLUGIN: &str = "CommandService/AddPlugin";

/// `CommandService/RemovePlugin`
pub const METHOD_REMOVE_PLUGIN: &str = "CommandService/RemovePlugin";

/// `CommandService/EnablePlugin`
pub const METHOD_ENABLE_PLUGIN: &str = "CommandService/EnablePlugin";

/// `CommandService/DisablePlugin`
pub const METHOD_DISABLE_PLUGIN: &str = "CommandService/DisablePlugin";

// ---------------------------------------------------------------------------
// GitLabService
// ---------------------------------------------------------------------------

/// `GitLabService/ListMergeRequests`
pub const METHOD_LIST_MERGE_REQUESTS: &str = "GitLabService/ListMergeRequests";

/// `GitLabService/GetMergeRequest`
pub const METHOD_GET_MERGE_REQUEST: &str = "GitLabService/GetMergeRequest";

/// `GitLabService/ListPipelines`
pub const METHOD_LIST_PIPELINES: &str = "GitLabService/ListPipelines";

/// `GitLabService/GetPipeline`
pub const METHOD_GET_PIPELINE: &str = "GitLabService/GetPipeline";

/// `GitLabService/ListIssues`
pub const METHOD_LIST_ISSUES: &str = "GitLabService/ListIssues";

/// `GitLabService/GetIssue`
pub const METHOD_GET_ISSUE: &str = "GitLabService/GetIssue";

// ---------------------------------------------------------------------------
// WorktreeService
// ---------------------------------------------------------------------------

/// `WorktreeService/CreateWorktree`
pub const METHOD_CREATE_WORKTREE: &str = "WorktreeService/CreateWorktree";

/// `WorktreeService/RemoveWorktree`
pub const METHOD_REMOVE_WORKTREE: &str = "WorktreeService/RemoveWorktree";

/// `WorktreeService/ListWorktrees`
pub const METHOD_LIST_WORKTREES: &str = "WorktreeService/ListWorktrees";

/// `WorktreeService/GetWorktree`
pub const METHOD_GET_WORKTREE: &str = "WorktreeService/GetWorktree";

// ---------------------------------------------------------------------------
// GitRepoService
// ---------------------------------------------------------------------------

/// `GitRepoService/RegisterRepo`
pub const METHOD_REGISTER_REPO: &str = "GitRepoService/RegisterRepo";

/// `GitRepoService/UnregisterRepo`
pub const METHOD_UNREGISTER_REPO: &str = "GitRepoService/UnregisterRepo";

/// `GitRepoService/ListRepos`
pub const METHOD_LIST_REPOS: &str = "GitRepoService/ListRepos";

/// `GitRepoService/GetRepo`
pub const METHOD_GET_REPO: &str = "GitRepoService/GetRepo";

/// `GitRepoService/UpdateRepo`
pub const METHOD_UPDATE_REPO: &str = "GitRepoService/UpdateRepo";

/// `GitRepoService/ScanRepos`
pub const METHOD_SCAN_REPOS: &str = "GitRepoService/ScanRepos";

// ---------------------------------------------------------------------------
// ConfigService
// ---------------------------------------------------------------------------

/// `ConfigService/GetSettings`
pub const METHOD_GET_SETTINGS: &str = "ConfigService/GetSettings";

/// `ConfigService/UpdateSettings`
pub const METHOD_UPDATE_SETTINGS: &str = "ConfigService/UpdateSettings";

/// `ConfigService/ListMcpServers`
pub const METHOD_LIST_MCP_SERVERS: &str = "ConfigService/ListMcpServers";

/// `ConfigService/GetPermissions`
pub const METHOD_GET_PERMISSIONS: &str = "ConfigService/GetPermissions";
