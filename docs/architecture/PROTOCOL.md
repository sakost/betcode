# BetCode Communication Protocols

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

## Overview

BetCode uses a two-layer protocol architecture. Layer 1 is the Claude Code SDK
control protocol -- a bidirectional NDJSON pipe between the daemon and the Claude
subprocess. Layer 2 is the BetCode gRPC API that connects clients (Flutter, CLI)
to the daemon, either directly or through a relay. The daemon acts as the bridge
between these two layers.

```
+-----------+       gRPC (Layer 2)       +---------+   NDJSON stdin/stdout (L1)   +--------+
|  Client   | <========================> | Daemon  | <==========================> | Claude |
| (Flutter, |   bidirectional stream     |  (Rust) |   line-delimited JSON        |  Code  |
|  CLI)     |                            |         |                              | (Node) |
+-----------+                            +---------+                              +--------+
                                              |
                                              | mTLS tunnel (Layer 2)
                                         +---------+
                                         |  Relay  |
                                         +---------+
```

## Protocol Documents

| Document | Description |
|----------|-------------|
| [PROTOCOL_L1.md](./PROTOCOL_L1.md) | Layer 1: Claude Code SDK Control Protocol (NDJSON) |
| [PROTOCOL_L2.md](./PROTOCOL_L2.md) | Layer 2: BetCode gRPC API (protobuf definitions) |
| [PROTOCOL_BRIDGE.md](./PROTOCOL_BRIDGE.md) | Protocol bridge, streaming pattern, reconnection |

## Quick Reference

### Layer 1: Claude Code SDK Control Protocol (Daemon <-> Claude)

NDJSON over stdin/stdout. The daemon spawns Claude Code as a subprocess.

**Claude -> Daemon (stdout):**

| Type | When | Purpose |
|------|------|---------|
| `system` | Session start | Session ID, tools, model, cwd |
| `assistant` | Complete response | Full content array (text + tool_use blocks) |
| `user` | Tool result echo | Confirms tool results incorporated |
| `stream_event` | Token-by-token | Partial deltas for real-time streaming |
| `control_request` | Permission/input needed | Tool approval or user question |
| `result` | Session end | Duration, cost, token usage |

**Daemon -> Claude (stdin):**

| Type | When | Purpose |
|------|------|---------|
| `control_response` | Answer to request | Allow/deny with optional input modification |
| `user` | New prompt | Continue multi-turn conversation |

### Layer 2: BetCode gRPC API (Clients <-> Daemon <-> Relay)

Six gRPC services defined in `proto/betcode/v1/`:

| Service | Proto File | Purpose |
|---------|-----------|---------|
| `AgentService` | `agent.proto` | Core conversation (bidirectional streaming) |
| `WorktreeService` | `worktree.proto` | Git worktree lifecycle |
| `MachineService` | `machine.proto` | Multi-machine management |
| `TunnelService` | `tunnel.proto` | Relay <-> daemon communication |
| `ConfigService` | `config.proto` | Settings and permissions |
| `GitLabService` | `gitlab.proto` | GitLab integration |

### Protocol Bridge

The daemon translates between layers:

```
Client (gRPC AgentRequest) --> Daemon --> Claude stdin (NDJSON)
Claude stdout (NDJSON)     --> Daemon --> Client (gRPC AgentEvent stream)
```

Key mechanisms:
- **Sequence numbers**: Every `AgentEvent` has a monotonic `sequence` for reconnection.
- **Permission engine**: Auto-resolves `control_request` or forwards to client.
- **Reconnection**: Client tracks last sequence; on reconnect, replay from SQLite.

## Cross-Reference

| Topic | Document |
|-------|----------|
| System topology | [TOPOLOGY.md](./TOPOLOGY.md) |
| Daemon internals | [DAEMON.md](./DAEMON.md) |
| SQLite schemas | [SCHEMAS.md](./SCHEMAS.md) |
| Client architecture | [CLIENTS.md](./CLIENTS.md) |
| Security and auth | [SECURITY.md](./SECURITY.md) |
| System overview | [OVERVIEW.md](./OVERVIEW.md) |
