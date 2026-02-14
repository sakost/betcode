# Topology Diagrams (Mermaid)

**Source**: TOPOLOGY.md
**Last Updated**: 2026-02-14

---

## 1. High-Level Topology

**Replaces**: ASCII topology at line 26-50

**Description**: Star topology with relay at center.

```mermaid
flowchart TB
    subgraph relay_box[RELAY SERVER - Rust, public internet]
        registry[Connection Registry]
        auth[Auth Gateway<br/>JWT + mTLS]
        router[gRPC Router]
        buffer[(Message Buffer)]

        style registry fill:#3B82F6,stroke:#2563EB,color:#fff
        style auth fill:#EF4444,stroke:#DC2626,color:#fff
        style router fill:#3B82F6,stroke:#2563EB,color:#fff
        style buffer fill:#10B981,stroke:#059669,color:#fff
    end

    subgraph clients[CLIENT LAYER]
        flutter[Flutter App]
        cli[CLI Client]
        style flutter fill:#06B6D4,stroke:#0891B2,color:#fff
        style cli fill:#06B6D4,stroke:#0891B2,color:#fff
    end

    subgraph daemon_box[DAEMON - per machine]
        subprocess[Claude Subprocess Mgr]
        mux[Session Multiplexer]
        store[(Session Store)]

        style subprocess fill:#3B82F6,stroke:#2563EB,color:#fff
        style mux fill:#3B82F6,stroke:#2563EB,color:#fff
        style store fill:#10B981,stroke:#059669,color:#fff
    end

    flutter -->|TLS+JWT| auth
    auth --> router
    router <-->|mTLS tunnel| subprocess
    cli -.->|local socket| subprocess
```

---

## 2. Connection Modes

**Replaces**: ASCII diagrams at lines 66-97

**Description**: Four connection modes with different network paths.

```mermaid
flowchart LR
    subgraph mode1[Mode 1: Local CLI - sub-ms latency]
        cli1[CLI] -->|Unix socket / pipe| daemon1[Daemon]
        style cli1 fill:#06B6D4,stroke:#0891B2,color:#fff
        style daemon1 fill:#3B82F6,stroke:#2563EB,color:#fff
    end

    subgraph mode2[Mode 2: Mobile via Relay - 50-100ms]
        flutter2[Flutter] -->|TLS+JWT| relay2[Relay]
        relay2 -->|mTLS| daemon2[Daemon]
        style flutter2 fill:#06B6D4,stroke:#0891B2,color:#fff
        style relay2 fill:#9CA3AF,stroke:#6B7280,color:#1F2937
        style daemon2 fill:#3B82F6,stroke:#2563EB,color:#fff
    end

    subgraph mode3[Mode 3: Cross-Machine - 50-100ms]
        cli3[CLI] -->|TLS+JWT| relay3[Relay]
        relay3 -->|mTLS| daemon3[Daemon]
        style cli3 fill:#06B6D4,stroke:#0891B2,color:#fff
        style relay3 fill:#9CA3AF,stroke:#6B7280,color:#1F2937
        style daemon3 fill:#3B82F6,stroke:#2563EB,color:#fff
    end

    subgraph mode4[Mode 4: Direct LAN - 1-5ms]
        cli4[CLI] -->|mTLS| daemon4[Daemon]
        style cli4 fill:#06B6D4,stroke:#0891B2,color:#fff
        style daemon4 fill:#3B82F6,stroke:#2563EB,color:#fff
    end
```

---

## 3. Daemon Lifecycle

**Replaces**: ASCII diagram at lines 259-282

**Description**: Startup and shutdown sequences.

```mermaid
stateDiagram-v2
    direction TB

    [*] --> Loading: START

    state Loading {
        [*] --> LoadConfig
        LoadConfig --> InitSQLite
        InitSQLite --> RestoreSessions
    }

    Loading --> Starting: config loaded

    state Starting {
        [*] --> LocalServer
        LocalServer --> RelayTunnel
        RelayTunnel --> HealthMonitor
    }

    Starting --> Running: servers ready

    state Running {
        [*] --> Accepting
        Accepting --> Spawning: on demand
        Spawning --> Accepting
    }

    Running --> Shutdown: SIGTERM/SIGINT

    state Shutdown {
        [*] --> StopAccepting
        StopAccepting --> TerminateProcesses
        TerminateProcesses --> FlushSQLite
        FlushSQLite --> CloseConnections
    }

    Shutdown --> [*]: EXIT
```
