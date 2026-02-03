# Subagent Diagrams (Mermaid)

**Source**: SUBAGENTS.md
**Last Updated**: 2026-02-03

---

## 1. Orchestration Architecture

**Replaces**: ASCII diagram at lines 13-24

**Description**: External orchestrator connecting to daemon subprocess pool.

```mermaid
flowchart TB
    orch([External Orchestrator])
    style orch fill:#9CA3AF,stroke:#6B7280,color:#1F2937

    subgraph daemon[betcode-daemon]
        service[SubagentService]
        pool[Subprocess Pool]
        scheduler[DAG Scheduler]

        style service fill:#3B82F6,stroke:#2563EB,color:#fff
        style pool fill:#3B82F6,stroke:#2563EB,color:#fff
        style scheduler fill:#3B82F6,stroke:#2563EB,color:#fff

        c1[claude #1<br/>worktree:A]
        c2[claude #2<br/>worktree:B]
        c3[claude #3<br/>worktree:C]

        style c1 fill:#60A5FA,stroke:#3B82F6,color:#1F2937
        style c2 fill:#60A5FA,stroke:#3B82F6,color:#1F2937
        style c3 fill:#60A5FA,stroke:#3B82F6,color:#1F2937
    end

    orch -->|gRPC| service
    service --> pool
    service --> scheduler
    pool --- c1 & c2 & c3
```

---

## 2. DAG Scheduler Example

**Replaces**: ASCII DAG at lines 200-209

**Description**: Task dependencies and parallel execution.

```mermaid
flowchart LR
    analyze[analyze]
    style analyze fill:#3B82F6,stroke:#2563EB,color:#fff

    backend[backend]
    frontend[frontend]
    docs[docs]
    style backend fill:#10B981,stroke:#059669,color:#fff
    style frontend fill:#10B981,stroke:#059669,color:#fff
    style docs fill:#10B981,stroke:#059669,color:#fff

    integration[integration-tests]
    style integration fill:#F59E0B,stroke:#D97706,color:#1F2937

    analyze --> backend --> integration
    analyze --> frontend --> integration
    analyze --> docs
```

**Execution Timeline:**
- t0: spawn analyze
- t1: complete -> spawn backend, frontend, docs in parallel
- t2: backend + frontend complete -> spawn integration-tests
- t3: all complete

---

## 3. Session Hierarchy

**Replaces**: ASCII hierarchy at lines 411-419

```mermaid
flowchart TB
    parent[Parent Session<br/>interactive]
    style parent fill:#3B82F6,stroke:#2563EB,color:#fff

    subA[Subagent A - backend<br/>feature/auth-backend]
    subB[Subagent B - frontend<br/>feature/auth-frontend]
    subC[Subagent C - tests<br/>feature/auth-tests]
    subD[Subagent D - docs<br/>same worktree]

    style subA fill:#60A5FA,stroke:#3B82F6,color:#1F2937
    style subB fill:#60A5FA,stroke:#3B82F6,color:#1F2937
    style subC fill:#60A5FA,stroke:#3B82F6,color:#1F2937
    style subD fill:#60A5FA,stroke:#3B82F6,color:#1F2937

    parent --> subA
    parent --> subB
    parent --> subC
    parent --> subD
```
