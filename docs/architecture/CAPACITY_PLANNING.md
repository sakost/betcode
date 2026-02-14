# Capacity Planning

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Specification
**Parent**: [NON_FUNCTIONAL.md](./NON_FUNCTIONAL.md)

## Overview

This document provides formulas and guidelines for capacity planning across BetCode
components. Use these calculations to size infrastructure and predict resource needs.

---

## Memory Estimation

### Daemon Memory Formula

```
base_memory = 50 MB
per_session = 5 MB (buffers, state)
per_client = 1 MB (gRPC stream state)
per_subprocess = 20 MB (overhead, excludes Claude's own memory)

total = base + (sessions * 5) + (clients * 1) + (subprocesses * 20) MB
```

**Example**: 20 sessions, 50 clients, 5 subprocesses
```
= 50 + (20 * 5) + (50 * 1) + (5 * 20)
= 50 + 100 + 50 + 100 = 300 MB daemon overhead
```

### Relay Memory Formula

```
base_memory = 100 MB
per_tunnel = 0.05 MB (stream state, cert cache)
per_client = 0.02 MB (JWT cache, connection state)
per_buffered_msg = 0.01 MB (average)

total = base + (tunnels * 0.05) + (clients * 0.02) + (buffer * 0.01) MB
```

**Example**: 5000 tunnels, 20000 clients, 10000 buffered messages
```
= 100 + 250 + 400 + 100 = 850 MB
```

---

## Storage Growth

### SQLite Database Growth

```
messages_per_session_per_hour = 200 (average)
avg_message_size = 5 KB
session_hours_per_day = 8

daily_growth = sessions * 200 * 5 KB * 8 = sessions * 8 MB
monthly_growth = daily * 22 workdays = sessions * 176 MB
```

**Example**: 10 active sessions
```
Monthly growth: 10 * 176 MB = 1.76 GB (before compaction)
```

### Compaction Impact

Session compaction reduces storage by 60-80% typically:
```
post_compaction = pre_compaction * 0.3
```

---

## Horizontal Scaling

### Relay Scaling Formula

```
relays_needed = ceil(max(
    tunnels / 10000,
    clients / 50000,
    rps / 100000
))
```

**Example**: 25000 tunnels, 80000 clients, 150000 rps
```
= max(ceil(2.5), ceil(1.6), ceil(1.5)) = 3 instances
```

### Load Balancer Configuration

```
                    Load Balancer
                    (hash by machine_id)
                          |
          +---------------+---------------+
          |               |               |
      Relay-1         Relay-2         Relay-3
```

Sticky sessions by `machine_id` hash ensure tunnel affinity.

---

## Vertical Scaling Guide

| User Scale | Daemon Spec | Relay Spec |
|------------|-------------|------------|
| 1-10 | 2 CPU, 2 GB RAM | Not needed (local only) |
| 10-100 | 4 CPU, 4 GB RAM | 2 CPU, 2 GB RAM |
| 100-1000 | 8 CPU, 8 GB RAM | 4 CPU, 4 GB RAM |
| 1000+ | Multiple machines | 3+ instances, 8 CPU each |

---

## Performance Budget Allocation

### Latency Budget (200ms P95 total)

| Component | Budget | Notes |
|-----------|--------|-------|
| Client processing | 20ms | UI responsiveness |
| Network RTT (2x) | 80ms | 40ms one-way |
| Relay routing | 10ms | Pure forwarding |
| Daemon processing | 50ms | Permission + broadcast |
| Subprocess overhead | 40ms | NDJSON I/O |

### Memory Budget (512 MB daemon default)

| Component | Budget |
|-----------|--------|
| Base runtime | 50 MB |
| Session state (20) | 100 MB |
| Client buffers (50) | 50 MB |
| Subprocess overhead (5) | 100 MB |
| SQLite cache | 100 MB |
| Headroom | 112 MB |

---

## Scaling Thresholds

| Metric | Warning (scale soon) | Critical (scale now) |
|--------|----------------------|----------------------|
| Sessions | 80% of max | 95% of max |
| Memory | 70% of limit | 85% of limit |
| CPU | 70% sustained | 90% sustained |
| Tunnels | 70% of max | 90% of max |
| Buffer size | 70% capacity | 90% capacity |
| SQLite size | 70% of limit | 90% of limit |

---

## Network Bandwidth

| Traffic Type | Bandwidth |
|--------------|-----------|
| Idle keepalive | 1 KB/min |
| Active streaming | 100 KB/s |
| Large file read | 1 MB/s burst |
| History replay | 500 KB/s |

---

## Related Documents

| Document | Description |
|----------|-------------|
| [NON_FUNCTIONAL.md](./NON_FUNCTIONAL.md) | SLOs, latency targets |
| [DAEMON.md](./DAEMON.md) | Resource configuration |
| [TOPOLOGY.md](./TOPOLOGY.md) | Relay architecture |
