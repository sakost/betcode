# Network Topology & Relay Architecture

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

## System Context

BetCode is a distributed system composed of four principal components:

- **Daemon** (per machine) -- spawns Claude Code subprocesses, serves gRPC.
  Runs on every developer machine that participates in the network.
- **Relay** (public internet) -- lightweight gRPC router and auth gateway.
  A single shared service that connects clients to daemons across the internet.
- **CLI** (local) -- Rust ratatui TUI client. Connects to the local daemon
  directly or to remote daemons through the relay.
- **Flutter App** (mobile/web) -- remote client. Always connects through the
  relay since it runs on a separate device.

Each component is independently deployable. The relay has zero knowledge of AI
workloads. Daemons are fully autonomous and need no coordination with each
other.

## High-Level Topology

```
                    ┌─────────────────────────────┐
                    │        RELAY SERVER          │
                    │   (Rust, public internet)    │
                    │                              │
                    │  Connection Registry         │
                    │  Auth Gateway (JWT + mTLS)   │
                    │  gRPC Router / Forwarder     │
                    │  Message Buffer (SQLite)     │
                    └──────┬──────────────┬────────┘
                           │              │
              TLS+JWT      │              │  mTLS (reverse tunnel)
                           │              │
            ┌──────────────┘              └──────────────────┐
            │                                                │
  ┌─────────▼──────────┐                        ┌────────────▼───────────┐
  │   CLIENT LAYER     │                        │  DAEMON (per machine)  │
  │                    │                        │                        │
  │  Flutter App       │   Multiple daemons     │  Claude Subprocess Mgr │
  │  (mobile/web)      │   on different         │  Session Multiplexer   │
  │                    │   machines             │  Worktree Manager      │
  │  CLI Client        │                        │  Config Resolver       │
  │  (local only)      │                        │  Session Store (SQLite)│
  └────────────────────┘                        └────────────────────────┘
```

The relay sits at the center of the star topology. Every daemon maintains a
persistent outbound tunnel to the relay. Every remote client connects inbound
to the relay. The relay matches client requests to daemon tunnels by
`machine_id`.

---

## Connection Modes

BetCode supports four connection modes, determined at connection time based on
network topology and client configuration.

### Mode 1: Local CLI (lowest latency)

```
CLI ──> Unix socket / named pipe ──> Daemon (same machine)
```

No network traversal, no TLS overhead. OS filesystem permissions on the socket
provide access control. This is the default when the CLI detects a local
daemon. Latency: sub-millisecond IPC.

### Mode 2: Mobile via Relay

```
Flutter ──TLS+JWT──> Relay ──mTLS tunnel──> Daemon
```

Primary mobile use case. Relay validates the JWT, resolves the target
`machine_id`, and forwards through the daemon's reverse tunnel.

### Mode 3: Cross-Machine

```
CLI/Flutter ──TLS+JWT──> Relay ──mTLS tunnel──> Target Daemon
```

Any client can target any daemon the user owns. The target machine is specified
by `machine_id` in request metadata. Uses the same relay infrastructure as
Mode 2.

### Mode 4: Direct LAN

```
CLI/Flutter ──mTLS──> Daemon (same network)
```

Skip the relay when on the same local network. Client discovers daemon via
mDNS or explicit configuration. Requires the daemon's mTLS server certificate
to be trusted by the client.

### Connection Mode Summary

| Mode | Path | Auth | Latency | Use Case |
|------|------|------|---------|----------|
| Local CLI | socket/pipe | OS perms | <1ms | Same machine dev |
| Mobile via Relay | TLS+JWT + mTLS | JWT + mTLS | ~50-100ms | Remote mobile |
| Cross-Machine | TLS+JWT + mTLS | JWT + mTLS | ~50-100ms | Multi-machine |
| Direct LAN | mTLS | mTLS | ~1-5ms | Same network |

---

## Relay Architecture

The relay is a **pure router**. It carries zero AI workload, runs no Claude
Code processes, and holds no agent logic.

### Responsibilities

1. **Connection Registry** -- map of `machine_id` to active tunnel stream.
2. **Request Routing** -- forward client requests to the correct daemon tunnel.
3. **Message Buffering** -- store requests for offline daemons (SQLite, 24h).
4. **Authentication** -- validate client JWTs and daemon mTLS certificates.

The relay does NOT run AI models, store session state, inspect request content,
or coordinate between daemons.

### Reverse Tunnel Protocol

The daemon initiates a persistent bidirectional gRPC stream to the relay,
inverting the usual client-server direction so daemons behind NAT/firewalls
need not expose ports.

**Establishment:**
1. Daemon starts and dials relay with mTLS client certificate.
2. Opens `TunnelService.OpenTunnel` (bidirectional gRPC stream).
3. Relay validates mTLS cert, extracts `machine_id` from cert CN.
4. Relay registers `(machine_id -> stream)` in connection registry.

**Request Flow:**
1. Client sends gRPC request to relay targeting `machine_id`.
2. Relay wraps request in `TunnelFrame`, sends through tunnel stream.
3. Daemon unwraps, processes, wraps response in `TunnelFrame`.
4. Relay unwraps response `TunnelFrame`, forwards to client.

```
[Flutter Client] ──TLS+JWT──> [Relay] <──mTLS tunnel── [Daemon]
                                 │
                           Routes request
                           through tunnel
```

### Peer-to-Peer Model

Each machine runs its own independent daemon. There is no central coordinator
for AI work. The relay is only a connectivity bridge.

```
Machine A (laptop)  ──tunnel──> Relay <──tunnel── Machine B (desktop)
Machine C (server)  ──tunnel──>   ^
                                  │
                            Flutter / CLI
                            can target any machine
```

A user with three machines has three independent daemons, each with its own
sessions, worktrees, and configuration. The client selects which machine to
interact with by specifying `machine_id`.

### Message Buffering (Mobile-First Design)

When a daemon goes offline (laptop lid closed, network interruption):

1. Relay detects no active tunnel for the target `machine_id`.
2. Stores the request in SQLite with configurable TTL (default 7 days).
3. Sends `StatusChange { status: DAEMON_OFFLINE }` to the client.
4. On daemon reconnect: buffered messages delivered in priority order, then FIFO.
5. Sends `StatusChange { status: DAEMON_ONLINE }` to connected clients.
6. Messages older than TTL are purged by a background sweep (1h cadence).

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Default TTL | 7 days | Covers weekend disconnects, vacations |
| Min TTL | 1 hour | Prevents accidental data loss |
| Max TTL | 30 days | Balances storage vs. usefulness |
| Max buffer per daemon | 1000 messages | Prevent unbounded growth |
| Max message size | 1 MB | Matches gRPC default max |
| Purge interval | 1 hour | Background cleanup cadence |

**Why 7 days instead of 24 hours:**

| Scenario | 24h TTL result | 7d TTL result |
|----------|---------------|---------------|
| Overnight disconnect | OK | OK |
| Weekend without laptop | LOST | OK |
| Week-long vacation | LOST | OK |
| Hardware failure (repair shop) | LOST | Likely OK |
| Forgot to open laptop for 2 weeks | LOST | LOST (acceptable) |

Storage cost for 1000 messages at 1MB max each: 1GB worst case per daemon.
Realistic average: ~50MB per daemon. This is negligible for a relay server.

**Priority Queue for Buffered Messages:**

Buffered messages are delivered in priority order on daemon reconnect:

| Priority | Message Type | Rationale |
|----------|--------------|-----------|
| 0 (highest) | Permission responses | Unblocks waiting agent |
| 1 | Cancel requests | Time-sensitive user intent |
| 2 | User messages | Primary interaction |
| 3 | Session control | Can wait |
| 4 (lowest) | Heartbeats, status | Background sync |

### Relay Restart Recovery

The relay is designed as a near-stateless router. On restart:

1. **Connection registry**: rebuilt from scratch as daemons reconnect.
   No persistent registry needed — daemons detect tunnel failure and
   reconnect with exponential backoff (1s-60s).
2. **Message buffer**: persisted in SQLite, survives relay restart.
   On startup, the relay loads `message_buffer` rows with
   `delivered = 0 AND expires_at > now()`.
3. **Active streams**: all client and daemon gRPC streams are severed.
   Clients reconnect via their own backoff logic. In-flight requests
   that were being forwarded are lost — clients retry based on timeout.
4. **JWT validation**: stateless (signature verification). The relay
   can validate tokens immediately after restart without loading state.
   Revocation checks require the `tokens` table (loaded from SQLite).

**Buffer overflow protection**: When `message_buffer` exceeds the
per-machine cap (1000 messages), the relay rejects new buffered requests
with `RESOURCE_EXHAUSTED` and includes the cap in the error detail.
Clients receive `StatusChange { status: BUFFER_FULL }` and should
inform the user that the target machine has been offline too long.

### Relay Observability

The relay emits metrics via the same OpenTelemetry infrastructure as the daemon
([DAEMON.md](./DAEMON.md#observability)).

#### Relay Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `betcode_relay_tunnels_active` | Gauge | Currently connected daemon tunnels |
| `betcode_relay_clients_connected` | Gauge | Currently connected client sessions |
| `betcode_relay_requests_total` | Counter | Total requests routed, by `method`, `status` |
| `betcode_relay_request_duration_seconds` | Histogram | End-to-end routing latency |
| `betcode_relay_buffer_size` | Gauge | Message buffer row count, by `machine_id` |
| `betcode_relay_buffer_bytes` | Gauge | Message buffer total size in bytes |
| `betcode_relay_auth_failures_total` | Counter | Authentication failures, by `reason` |
| `betcode_relay_push_notifications_total` | Counter | Push notifications sent, by `platform`, `status` |
| `betcode_relay_tunnel_reconnects_total` | Counter | Daemon tunnel reconnection count |

#### Structured Logging

Same format as daemon (JSON lines, tracing crate):

| Level | Content |
|-------|---------|
| ERROR | Auth failures, database errors, push delivery failures |
| WARN | Tunnel disconnections, buffer overflow, rate limit hits |
| INFO | Tunnel connections, client connections, push sends |
| DEBUG | Request routing, buffer operations |
| TRACE | gRPC frame details, JWT claims |

#### Prometheus Endpoint

Optional `/metrics` HTTP endpoint (disabled by default, enable via config):

```
betcode_relay_tunnels_active 42
betcode_relay_clients_connected 156
betcode_relay_buffer_size{machine_id="*"} 1234
```

---

## Daemon Lifecycle

### Startup Sequence

```
START
  -> Load config (settings.json, resolve hierarchy)
  -> Init SQLite (migrations, restore sessions, validate schema)
  -> Start local gRPC server (Unix socket or named pipe)
  -> Connect to relay (mTLS cert, open tunnel, confirm registration)
RUNNING
  -> Accept local and tunneled connections
  -> Spawn and manage Claude subprocesses
  -> Multiplex sessions across clients
  -> Manage git worktrees for isolation
```

### Shutdown Sequence

```
SHUTDOWN signal (SIGTERM / SIGINT / service stop)
  -> Stop accepting new connections, drain in-flight (5s timeout)
  -> Graceful stop Claude processes (interrupt, 10s wait, force kill)
  -> Persist session state to SQLite
  -> Close relay tunnel (send TunnelClose, deregister machine_id)
  -> Close local server (unbind socket, SQLite WAL checkpoint)
EXIT
```

### Health and Reconnection

- **Keepalive**: gRPC HTTP/2 PING frames every 30 seconds.
- **Reconnect**: exponential backoff (1s, 2s, 4s, ... max 60s).
- **Health endpoint**: local gRPC `HealthService` for process managers.

---

## Network Security Layers

Full details in [SECURITY.md](./SECURITY.md).

| Connection | Client Auth | Server Auth |
|------------|------------|-------------|
| CLI to local daemon | OS socket perms | N/A (local) |
| Flutter to relay | JWT (Bearer) | TLS server cert |
| Daemon to relay | mTLS client cert | TLS server cert |
| Direct LAN | mTLS client cert | mTLS server cert |

### Certificate Hierarchy

```
BetCode Root CA
  +-- Relay Server Certificate  (CN: relay.betcode.example.com)
  +-- Daemon Client Certificates (CN: machine_id, one per machine)
  +-- Client Certificates        (CN: user_id, optional for LAN mode)
```

---

## Scalability Considerations

### Relay Scaling

The relay is stateless aside from the connection registry and message buffer.

- **Vertical**: single instance handles thousands of concurrent tunnels
  (tokio async, minimal per-connection memory).
- **Horizontal**: multiple instances behind a load balancer with shared
  registry (Redis or distributed SQLite). Sticky sessions by `machine_id`
  hash.

### Daemon Scaling

Each daemon is independent. Scaling is per-machine:

- **Subprocess limits**: configurable max concurrent Claude processes.
- **Session multiplexing**: multiple clients share one daemon.
- **Worktree isolation**: each session gets its own git worktree.

### Latency Budget

| Segment | Target | Notes |
|---------|--------|-------|
| Client to relay | <50ms | Cached TLS handshake |
| Relay routing | <1ms | In-memory registry lookup |
| Relay to daemon | <50ms | Persistent stream, no handshake |
| Daemon processing | Variable | Depends on Claude API response |
| **Total overhead** | **<100ms** | **Excluding AI processing** |

---

## Related Documents

| Document | Description |
|----------|-------------|
| [OVERVIEW.md](./OVERVIEW.md) | System overview, tech stack, workspace |
| [DAEMON.md](./DAEMON.md) | Daemon internals, subprocess management |
| [PROTOCOL.md](./PROTOCOL.md) | gRPC API, Claude SDK protocol |
| [SECURITY.md](./SECURITY.md) | Auth, authorization, sandboxing |
| [SCHEMAS.md](./SCHEMAS.md) | SQLite schemas for daemon, relay, client |
| [CLIENTS.md](./CLIENTS.md) | Flutter app and CLI client architecture |
| [SUBAGENTS.md](./SUBAGENTS.md) | Multi-agent orchestration |
