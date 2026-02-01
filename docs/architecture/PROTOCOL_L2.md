# Layer 2: BetCode gRPC API

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Parent**: [PROTOCOL.md](./PROTOCOL.md)

## Overview

The gRPC protocol connecting BetCode clients to the daemon (and optionally
through the relay). All proto definitions live in `proto/betcode/v1/`.

```
proto/betcode/v1/
  agent.proto       -- Core agent conversation service
  machine.proto     -- Multi-machine management
  worktree.proto    -- Git worktree management
  subagent.proto    -- Subagent orchestration (Level 2)
  gitlab.proto      -- GitLab integration
  config.proto      -- Settings and configuration
  tunnel.proto      -- Relay <-> Daemon communication
```

---

## AgentService

Primary service for AI agent interaction. Bidirectional streaming.

```protobuf
syntax = "proto3";
package betcode.v1;

service AgentService {
  rpc Converse(stream AgentRequest) returns (stream AgentEvent);
  rpc ListSessions(ListSessionsRequest) returns (ListSessionsResponse);
  rpc ResumeSession(ResumeSessionRequest) returns (stream AgentEvent);
  rpc CompactSession(CompactSessionRequest) returns (CompactSessionResponse);
  rpc CancelTurn(CancelTurnRequest) returns (CancelTurnResponse);
  rpc RequestInputLock(InputLockRequest) returns (InputLockResponse);
}
```

### AgentRequest (Client -> Agent)

```protobuf
message AgentRequest {
  oneof request {
    StartConversation start = 1;
    UserMessage message = 2;
    PermissionResponse permission = 3;
    UserQuestionResponse question_response = 4;
    CancelRequest cancel = 5;
  }
}

message StartConversation {
  string session_id = 1;             // Empty = new session
  string working_directory = 2;
  string model = 3;
  repeated string allowed_tools = 4;
  bool plan_mode = 5;
  string worktree_id = 6;
  map<string, string> metadata = 7;
}

message UserMessage {
  string content = 1;
  repeated Attachment attachments = 2;
}

message Attachment {
  string filename = 1;
  string mime_type = 2;
  bytes data = 3;
}

message PermissionResponse {
  string request_id = 1;
  PermissionDecision decision = 2;
}

enum PermissionDecision {
  PERMISSION_DECISION_UNSPECIFIED = 0;
  ALLOW_ONCE = 1;
  ALLOW_SESSION = 2;
  DENY = 3;
}

message UserQuestionResponse {
  string question_id = 1;
  map<string, string> answers = 2;    // question text -> selected answer(s)
}

message CancelRequest {
  string reason = 1;
}
```

### Converse Stream Lifecycle

The `Converse` RPC is a bidirectional stream with defined lifecycle states.

**Opening**: Client sends `StartConversation` as the first message.
If `session_id` is empty, the daemon creates a new session. If non-empty,
the daemon attaches to the existing session (no new subprocess spawned
unless the session is idle and a `UserMessage` follows).

**Active turns**: Client sends `UserMessage`, daemon streams `AgentEvent`
messages until `TurnComplete`. Multiple turns occur within a single
`Converse` stream.

**Clean close**: Either side closes. Client closes the send half when
done. Daemon closes after sending final events. The Claude subprocess
continues if a turn is in progress — events are buffered for the next
stream.

**Error close**: Daemon sends `ErrorEvent { is_fatal: true }` then
closes the stream. Non-fatal errors are sent inline without closing.

**Cancellation**: Client sends `CancelRequest` within the stream. Daemon
sends SIGINT to the Claude subprocess. Claude finishes its current
operation, emits a `result`, and the daemon sends `TurnComplete` with
`stop_reason: "cancelled"`.

**Reconnection**: If the stream drops mid-turn, the client reopens
`Converse` with the same `session_id`. The daemon replays missed events
(from the last known sequence) then continues live streaming.

### AgentEvent (Agent -> Client)

Every event carries a monotonic `sequence` for reconnection support.

```protobuf
message AgentEvent {
  uint64 sequence = 1;
  google.protobuf.Timestamp timestamp = 2;
  string parent_tool_use_id = 3;  // Non-empty when from Claude-internal subagent (Task tool)
  oneof event {
    TextDelta text_delta = 10;
    ToolCallStart tool_call_start = 11;
    ToolCallResult tool_call_result = 12;
    PermissionRequest permission_request = 13;
    UserQuestion user_question = 14;
    TodoUpdate todo_update = 15;
    StatusChange status_change = 16;
    SessionInfo session_info = 17;
    ErrorEvent error = 18;
    UsageReport usage = 19;
    PlanModeChange plan_mode = 20;
    TurnComplete turn_complete = 21;
  }
}

message TextDelta { string text = 1; bool is_complete = 2; }

message ToolCallStart {
  string tool_id = 1;
  string tool_name = 2;
  google.protobuf.Struct input = 3;
  string description = 4;
}

message ToolCallResult {
  string tool_id = 1;
  string output = 2;
  bool is_error = 3;
  uint32 duration_ms = 4;
}

message PermissionRequest {
  string request_id = 1;
  string tool_name = 2;
  string description = 3;
  google.protobuf.Struct input = 4;
}

message UserQuestion {
  string question_id = 1;
  string question = 2;
  repeated QuestionOption options = 3;
  bool multi_select = 4;
}

message QuestionOption {
  string value = 1;
  string label = 2;
  string description = 3;
}

message TodoUpdate { repeated TodoItem items = 1; }
message TodoItem {
  string id = 1; string subject = 2; string description = 3;
  string active_form = 4; TodoStatus status = 5;
}
enum TodoStatus {
  TODO_STATUS_UNSPECIFIED = 0; PENDING = 1; IN_PROGRESS = 2; COMPLETED = 3;
}

message StatusChange { AgentStatus status = 1; string message = 2; }
enum AgentStatus {
  AGENT_STATUS_UNSPECIFIED = 0; THINKING = 1; EXECUTING_TOOL = 2;
  WAITING_FOR_USER = 3; IDLE = 4; COMPACTING = 5; ERROR = 6;
}

message SessionInfo {
  string session_id = 1; string model = 2; string working_directory = 3;
  string worktree_id = 4; uint64 message_count = 5; bool is_resumed = 6;
}

message ErrorEvent { string code = 1; string message = 2; bool is_fatal = 3; }

message UsageReport {
  uint32 input_tokens = 1; uint32 output_tokens = 2;
  uint32 cache_read_tokens = 3; uint32 cache_creation_tokens = 4;
  string model = 5; double cost_usd = 6; uint32 duration_ms = 7;
}

message PlanModeChange { bool active = 1; string plan = 2; }
message TurnComplete { string stop_reason = 1; }
```

### Session Management Messages

```protobuf
message ListSessionsRequest {
  string working_directory = 1; string worktree_id = 2;
  uint32 limit = 3; uint32 offset = 4;
}
message ListSessionsResponse { repeated SessionSummary sessions = 1; uint32 total = 2; }
message SessionSummary {
  string id = 1; string model = 2; string working_directory = 3;
  string worktree_id = 4; string status = 5; uint32 message_count = 6;
  uint32 total_input_tokens = 7; uint32 total_output_tokens = 8;
  double total_cost_usd = 9; google.protobuf.Timestamp created_at = 10;
  google.protobuf.Timestamp updated_at = 11; string last_message_preview = 12;
}
message ResumeSessionRequest { string session_id = 1; uint64 from_sequence = 2; }
message CompactSessionRequest { string session_id = 1; }
message CompactSessionResponse {
  uint32 messages_before = 1; uint32 messages_after = 2; uint32 tokens_saved = 3;
}
message CancelTurnRequest { string session_id = 1; }
message CancelTurnResponse { bool was_active = 1; }

message InputLockRequest { string session_id = 1; }
message InputLockResponse {
  bool granted = 1;
  string previous_holder = 2;  // client_id that released
}
```

**Session resume pattern**: `ResumeSession` replays historical events as a
server stream. To send new messages on a resumed session, the client opens
a `Converse` stream with `StartConversation { session_id: "<existing>" }`.
The daemon detects the existing session and attaches the stream without
spawning a new subprocess. Both RPCs can be active simultaneously: the
`ResumeSession` stream delivers replay events while the `Converse` stream
handles bidirectional interaction.

---

## WorktreeService

```protobuf
service WorktreeService {
  rpc ListWorktrees(ListWorktreesRequest) returns (ListWorktreesResponse);
  rpc CreateWorktree(CreateWorktreeRequest) returns (WorktreeInfo);
  rpc SwitchWorktree(SwitchWorktreeRequest) returns (SwitchWorktreeResponse);
  rpc RemoveWorktree(RemoveWorktreeRequest) returns (RemoveWorktreeResponse);
}

message ListWorktreesRequest { string repo_path = 1; }
message ListWorktreesResponse { repeated WorktreeInfo worktrees = 1; }
message WorktreeInfo {
  string id = 1; string name = 2; string path = 3; string branch = 4;
  google.protobuf.Timestamp created_at = 5; string active_session_id = 6;
}
message CreateWorktreeRequest {
  string repo_path = 1; string branch = 2; string name = 3;
  string path = 4; bool create_branch = 5;
}
message SwitchWorktreeRequest { string worktree_id = 1; }
message SwitchWorktreeResponse { WorktreeInfo worktree = 1; string previous_worktree_id = 2; }
message RemoveWorktreeRequest { string worktree_id = 1; bool force = 2; }
message RemoveWorktreeResponse { bool removed = 1; uint32 sessions_closed = 2; }
```

---

## MachineService

```protobuf
service MachineService {
  rpc ListMachines(ListMachinesRequest) returns (ListMachinesResponse);
  rpc GetMachine(GetMachineRequest) returns (Machine);
  rpc SwitchMachine(SwitchMachineRequest) returns (SwitchMachineResponse);
}

message ListMachinesRequest {}
message ListMachinesResponse { repeated Machine machines = 1; }
message GetMachineRequest { string machine_id = 1; }
message Machine {
  string id = 1; string name = 2; string hostname = 3;
  MachineStatus status = 4; repeated string capabilities = 5;
  google.protobuf.Timestamp last_seen = 6; repeated WorktreeInfo worktrees = 7;
  MachineResources resources = 8;
}
enum MachineStatus {
  MACHINE_STATUS_UNSPECIFIED = 0; ONLINE = 1; OFFLINE = 2; CONNECTING = 3;
}
message MachineResources {
  string os = 1; uint32 cpu_cores = 2; uint64 memory_bytes = 3; uint64 disk_free_bytes = 4;
}
message SwitchMachineRequest { string machine_id = 1; }
message SwitchMachineResponse { Machine machine = 1; string previous_machine_id = 2; }
```

---

## TunnelService (Relay <-> Daemon)

Persistent bidirectional stream. Daemon initiates on startup with mTLS.

```protobuf
service TunnelService {
  rpc OpenTunnel(stream TunnelFrame) returns (stream TunnelFrame);
  rpc Register(RegisterRequest) returns (RegisterResponse);
  rpc Heartbeat(HeartbeatRequest) returns (HeartbeatResponse);
}

message TunnelFrame {
  string request_id = 1;
  uint64 sequence = 2;
  string service_name = 8;   // e.g. "AgentService", for routing
  string method_name = 9;    // e.g. "Converse", for routing
  oneof payload {
    bytes grpc_request = 3;
    bytes grpc_response = 4;
    TunnelHeartbeat heartbeat = 5;
    ErrorFrame error = 6;
    StreamFrame stream = 7;
  }
}
message StreamFrame { bytes data = 1; bool end_of_stream = 2; }
message ErrorFrame { string code = 1; string message = 2; }
message TunnelHeartbeat { google.protobuf.Timestamp sent_at = 1; }

message RegisterRequest {
  string machine_id = 1; string machine_name = 2;
  repeated string capabilities = 3; MachineInfo info = 4;
}
message MachineInfo {
  string os = 1; string arch = 2; string version = 3;
  uint32 cpu_cores = 4; uint64 memory_bytes = 5;
}
message RegisterResponse {
  bool accepted = 1; string message = 2; uint32 heartbeat_interval_secs = 3;
}
message HeartbeatRequest {
  string machine_id = 1; google.protobuf.Timestamp sent_at = 2; MachineMetrics metrics = 3;
}
message MachineMetrics { float cpu_usage_percent = 1; uint64 memory_used_bytes = 2; uint32 active_sessions = 3; }
message HeartbeatResponse { google.protobuf.Timestamp received_at = 1; uint32 buffered_messages = 2; }
```

---

## ConfigService

```protobuf
service ConfigService {
  rpc GetSettings(GetSettingsRequest) returns (Settings);
  rpc UpdateSettings(UpdateSettingsRequest) returns (Settings);
  rpc ListMcpServers(ListMcpServersRequest) returns (ListMcpServersResponse);
  rpc GetPermissions(GetPermissionsRequest) returns (PermissionRules);
}
```

## GitLabService

```protobuf
service GitLabService {
  rpc ListMergeRequests(ListMrsRequest) returns (ListMrsResponse);
  rpc GetMergeRequest(GetMrRequest) returns (MergeRequestInfo);
  rpc ListPipelines(ListPipelinesRequest) returns (ListPipelinesResponse);
  rpc GetPipelineJobs(GetJobsRequest) returns (GetJobsResponse);
  rpc GetJobLog(GetJobLogRequest) returns (GetJobLogResponse);
  rpc ListIssues(ListIssuesRequest) returns (ListIssuesResponse);
}
```

Full ConfigService and GitLabService message definitions will be added
as those services are implemented (Phase 4).
