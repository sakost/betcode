# Overview Diagrams (Mermaid)

**Source**: OVERVIEW.md
**Last Updated**: 2026-02-03

---

## C4 Context Diagram

**Replaces**: ASCII box diagram at line 34-62

**Description**: System context showing BetCode's relationship with external systems (Anthropic API, GitLab, MCP servers) and client applications.

```mermaid
flowchart TB
    subgraph external[EXTERNAL SYSTEMS]
        direction LR
        anthropic([Anthropic API<br/>Claude models])
        gitlab([GitLab API<br/>MRs, pipelines])
        mcp([MCP Servers<br/>spawned by CC])

        style anthropic fill:#9CA3AF,stroke:#6B7280,color:#1F2937
        style gitlab fill:#9CA3AF,stroke:#6B7280,color:#1F2937
        style mcp fill:#9CA3AF,stroke:#6B7280,color:#1F2937
    end

    subgraph betcode[BETCODE SYSTEM BOUNDARY]
        daemon[betcode-daemon<br/>Session Multiplexer<br/>Worktree Manager<br/>GitLab Client]
        style daemon fill:#3B82F6,stroke:#2563EB,color:#fff

        subgraph subprocesses[Claude Code Subprocesses]
            cc1[claude #1]
            cc2[claude #2]
            ccn[claude #n]
            style cc1 fill:#60A5FA,stroke:#3B82F6,color:#1F2937
            style cc2 fill:#60A5FA,stroke:#3B82F6,color:#1F2937
            style ccn fill:#60A5FA,stroke:#3B82F6,color:#1F2937
        end

        relay[betcode-relay<br/>gRPC router]
        style relay fill:#3B82F6,stroke:#2563EB,color:#fff

        cli[betcode-cli<br/>ratatui TUI]
        style cli fill:#06B6D4,stroke:#0891B2,color:#fff
    end

    flutter[betcode_app<br/>Flutter mobile]
    style flutter fill:#06B6D4,stroke:#0891B2,color:#fff

    %% External connections
    cc1 & cc2 & ccn -->|called by CC| anthropic
    cc1 & cc2 & ccn -->|by CC| mcp
    daemon -->|called by daemon| gitlab

    %% Internal connections
    daemon --- cc1 & cc2 & ccn
    cli -->|local socket| daemon
    daemon <-->|mTLS tunnel| relay
    flutter -->|TLS + JWT| relay
```

---

## Legend

```mermaid
flowchart LR
    subgraph Legend
        direction LR
        ext([External System])
        style ext fill:#9CA3AF,stroke:#6B7280,color:#1F2937

        core[BetCode Core]
        style core fill:#3B82F6,stroke:#2563EB,color:#fff

        client[Client App]
        style client fill:#06B6D4,stroke:#0891B2,color:#fff

        data[(Data Store)]
        style data fill:#10B981,stroke:#059669,color:#fff
    end
```
