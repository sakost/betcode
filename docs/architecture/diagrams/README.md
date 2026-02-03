# BetCode Mermaid Diagrams

This directory contains Mermaid diagram replacements for ASCII art in the architecture documentation.

## Files

| File | Source Doc | Diagrams |
|------|-----------|----------|
| [OVERVIEW_DIAGRAMS.md](./OVERVIEW_DIAGRAMS.md) | OVERVIEW.md | C4 Context, Legend |
| [DAEMON_DIAGRAMS.md](./DAEMON_DIAGRAMS.md) | DAEMON.md | Architecture, Process Lifecycle, Multiplexer, Permission Bridge |
| [TOPOLOGY_DIAGRAMS.md](./TOPOLOGY_DIAGRAMS.md) | TOPOLOGY.md | High-Level Topology, Connection Modes, Daemon Lifecycle |
| [CLIENTS_DIAGRAMS.md](./CLIENTS_DIAGRAMS.md) | CLIENTS.md | Sync Engine, Queue States, gRPC Protocol, Push Flow |
| [SUBAGENTS_DIAGRAMS.md](./SUBAGENTS_DIAGRAMS.md) | SUBAGENTS.md | Orchestration, DAG Scheduler, Session Hierarchy |

## Usage

1. Open the diagram file for the document you want to update
2. Copy the Mermaid code block (including fences)
3. Replace the ASCII diagram in the source document
4. Add accessibility description above the diagram

## Standards

See [DIAGRAM_GUIDE.md](../DIAGRAM_GUIDE.md) for color coding, naming conventions, and style rules.
