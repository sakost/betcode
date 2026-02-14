# Daemon Diagrams (Mermaid)

**Source**: DAEMON.md
**Last Updated**: 2026-02-14

---

## 1. Daemon Architecture

**Replaces**: ASCII diagram at line 18-27

**Description**: Three-tier architecture showing clients, daemon, and Claude subprocess.

```mermaid
flowchart LR
    subgraph clients[CLIENT LAYER]
        flutter[Flutter/CLI<br/>client]
        style flutter fill:#06B6D4,stroke:#0891B2,color:#fff
    end

    subgraph daemon_box[DAEMON]
        daemon[betcode-daemon<br/>Rust, tonic]
        style daemon fill:#3B82F6,stroke:#2563EB,color:#fff
    end

    subgraph subprocess[SUBPROCESS]
        claude[claude CLI<br/>subprocess]
        style claude fill:#60A5FA,stroke:#3B82F6,color:#1F2937
    end

    flutter <-->|gRPC| daemon
    daemon <-->|stdio NDJSON| claude
```

---

## 2. Process Lifecycle State Diagram

**Replaces**: ASCII state diagram at line 88-99

**Description**: Claude subprocess lifecycle states.

```mermaid
stateDiagram-v2
    direction LR

    [*] --> IDLE: daemon starts

    IDLE --> SPAWNING: client sends message
    SPAWNING --> RUNNING: process started

    RUNNING --> RUNNING: stdout NDJSON to multiplexer
    RUNNING --> EXITED_OK: exit 0 (turn complete)
    RUNNING --> CRASHED: exit != 0

    EXITED_OK --> IDLE: ready for next message

    CRASHED --> RESTARTING: backoff timer
    RESTARTING --> RUNNING: respawn with --resume
    RESTARTING --> FAILED: 5 crashes in 60s

    FAILED --> [*]: notify clients CRASHED
```

---

## 3. Multiplexer Flow

**Replaces**: ASCII flow at line 171-176

**Description**: NDJSON parsing and broadcast to clients.

```mermaid
flowchart LR
    claude[claude stdout]
    style claude fill:#60A5FA,stroke:#3B82F6,color:#1F2937

    parser[protocol.rs<br/>parse NDJSON]
    style parser fill:#3B82F6,stroke:#2563EB,color:#fff

    mux[multiplexer.rs<br/>fan-out]
    style mux fill:#3B82F6,stroke:#2563EB,color:#fff

    db[(SQLite<br/>store.rs)]
    style db fill:#10B981,stroke:#059669,color:#fff

    client1[Client 1]
    client2[Client 2]
    clientn[Client N]
    style client1 fill:#06B6D4,stroke:#0891B2,color:#fff
    style client2 fill:#06B6D4,stroke:#0891B2,color:#fff
    style clientn fill:#06B6D4,stroke:#0891B2,color:#fff

    claude --> parser
    parser --> mux
    mux --> db
    mux --> client1
    mux --> client2
    mux --> clientn
```

---

## 4. Permission Bridge Flow

**Replaces**: ASCII flow at line 231-266

**Description**: Permission request handling with reconnection replay.

```mermaid
sequenceDiagram
    participant CC as Claude Code
    participant D as Daemon
    participant PM as Pending Map
    participant C as Client

    CC->>D: control_request (request_id)
    D->>PM: Add to pending_permissions

    alt Auto-approve rule matches
        D->>CC: control_response (allow)
        D->>PM: Remove from map
    else Forward to client
        D->>C: PermissionRequest
        Note over D,C: Client may disconnect here

        alt Client responds
            C->>D: PermissionResponse
            D->>PM: Check request_id exists
            D->>CC: control_response
            D->>PM: Remove from map
        else Tunnel drops
            Note over D: Request stays in pending map
            C-->>D: Reconnect
            D->>C: Replay PermissionRequest
            C->>D: PermissionResponse
            D->>CC: control_response
        end
    end
```
