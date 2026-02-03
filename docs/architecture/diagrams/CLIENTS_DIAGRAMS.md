# Client Diagrams (Mermaid)

**Source**: CLIENTS.md
**Last Updated**: 2026-02-03

---

## 1. Sync Engine Flow

**Replaces**: ASCII flow at lines 224-233

**Description**: Offline sync queue processing.

```mermaid
flowchart TB
    action[User Action]
    style action fill:#06B6D4,stroke:#0891B2,color:#fff

    localdb[(Local DB)]
    style localdb fill:#10B981,stroke:#059669,color:#fff

    queue[(sync_queue)]
    style queue fill:#10B981,stroke:#059669,color:#fff

    check{Online?}
    style check fill:#F59E0B,stroke:#D97706,color:#1F2937

    grpc[Replay as gRPC]
    style grpc fill:#3B82F6,stroke:#2563EB,color:#fff

    success{Success?}
    mark[Mark synced]
    backoff[Backoff retry]
    accumulate[Queue offline]

    action --> localdb --> queue --> check
    check -->|YES| grpc --> success
    check -->|NO| accumulate --> check
    success -->|YES| mark
    success -->|NO| backoff --> grpc
```

---

## 2. Sync Queue State Machine

**Replaces**: ASCII state diagram at lines 354-362

```mermaid
stateDiagram-v2
    direction LR

    [*] --> PENDING: queued
    PENDING --> SENDING: sync attempt
    SENDING --> SENT: success
    SENDING --> BLOCKED: no lock
    SENDING --> FAILED: permanent error
    BLOCKED --> PENDING: lock acquired
    SENT --> [*]: removed
    FAILED --> [*]: removed, notified
```

---

## 3. gRPC Streaming Protocol

**Replaces**: ASCII sequence at lines 444-458

```mermaid
sequenceDiagram
    participant C as Client
    participant D as Daemon

    C->>D: StartConversation
    D-->>C: SessionInfo
    C->>D: UserMessage
    D-->>C: StatusChange(THINKING)
    D-->>C: TextDelta (streaming)
    D-->>C: ToolCallStart
    D-->>C: ToolCallResult
    D-->>C: PermissionRequest
    C->>D: PermissionResponse
    D-->>C: TextDelta
    D-->>C: UsageReport
    D-->>C: StatusChange(IDLE)
```

---

## 4. Push Notification Flow

**Replaces**: ASCII flow at lines 669-688

```mermaid
flowchart TB
    perm[Permission arrives]
    style perm fill:#F59E0B,stroke:#D97706,color:#1F2937

    check{Client has lock?}
    style check fill:#EF4444,stroke:#DC2626,color:#fff

    grpc[Forward via gRPC]
    style grpc fill:#3B82F6,stroke:#2563EB,color:#fff

    buffer[(Buffer in daemon)]
    style buffer fill:#10B981,stroke:#059669,color:#fff

    relay[Send to relay]
    push[Push notification]
    style push fill:#06B6D4,stroke:#0891B2,color:#fff

    perm --> check
    check -->|Yes| grpc
    check -->|No| buffer --> relay --> push
```
