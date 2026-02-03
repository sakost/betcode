# Recovery State Machines

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## Daemon Startup Recovery

```mermaid
stateDiagram-v2
    [*] --> LoadConfig
    LoadConfig --> ConfigError: Invalid config
    LoadConfig --> InitStorage: Config valid
    ConfigError --> [*]: Exit with error

    InitStorage --> CorruptionDetected: Integrity check fails
    InitStorage --> ReconcileState: Storage healthy

    CorruptionDetected --> AttemptRecovery
    AttemptRecovery --> RecoverySuccess: Recovery worked
    AttemptRecovery --> RecoveryFailed: Recovery failed
    RecoverySuccess --> ReconcileState
    RecoveryFailed --> [*]: Exit with error

    ReconcileState --> StartServers: State reconciled
    StartServers --> LocalServerFailed: Socket bind fails
    StartServers --> Running: All servers started

    LocalServerFailed --> RetryLocalServer: Retry with backoff
    RetryLocalServer --> Running: Succeeded
    RetryLocalServer --> [*]: Max retries, exit

    Running --> [*]: Shutdown signal
```

---

## Subprocess Crash Recovery

```mermaid
stateDiagram-v2
    [*] --> Running

    Running --> ExitedClean: Exit code 0
    Running --> Crashed: Exit code != 0
    Running --> Hung: No output for 5 min

    ExitedClean --> Idle: Normal turn completion

    Crashed --> CheckCrashCount
    Hung --> SendSIGTERM
    SendSIGTERM --> Killed: Process terminated
    Killed --> CheckCrashCount

    CheckCrashCount --> RestartWithResume: Count < 5 in 60s
    CheckCrashCount --> PermanentlyFailed: Count >= 5 in 60s

    RestartWithResume --> BackoffWait: Apply backoff
    BackoffWait --> Spawning
    Spawning --> Running: Subprocess started
    Spawning --> PermanentlyFailed: Spawn failed

    PermanentlyFailed --> NotifyClients: Send CRASHED status
    NotifyClients --> Idle: Await manual intervention

    Idle --> [*]
```

---

## Tunnel Reconnection Recovery

```mermaid
stateDiagram-v2
    [*] --> Disconnected

    Disconnected --> Connecting: Initiate connection
    Connecting --> Authenticating: TCP established
    Connecting --> BackoffWait: Connection failed

    Authenticating --> Connected: mTLS success
    Authenticating --> AuthFailed: Certificate rejected

    AuthFailed --> CertExpired: Cert expired
    AuthFailed --> CertRevoked: Cert revoked
    AuthFailed --> BackoffWait: Transient error

    CertExpired --> RequestRenewal: Auto-renew
    CertRevoked --> [*]: Manual re-registration

    RequestRenewal --> Connecting: New cert obtained
    RequestRenewal --> [*]: Renewal failed

    Connected --> Disconnected: Connection lost
    Connected --> HeartbeatTimeout: No response
    HeartbeatTimeout --> Disconnected: Assume dead

    BackoffWait --> Connecting: Backoff elapsed
```

Backoff schedule: 1s, 2s, 4s, 8s, 16s, 32s, 60s (max) with 20% jitter.

---

## Circuit Breaker State Machine

```mermaid
stateDiagram-v2
    [*] --> Closed

    Closed --> Closed: Success
    Closed --> Closed: Failure below threshold
    Closed --> Open: Failure threshold exceeded

    Open --> Open: Timeout not elapsed (fail fast)
    Open --> HalfOpen: Timeout elapsed

    HalfOpen --> Closed: Probe succeeds
    HalfOpen --> Open: Probe fails
```

---

## Permission Request Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Pending: control_request received

    Pending --> AutoApproved: Rule match (allow)
    Pending --> AutoDenied: Rule match (deny)
    Pending --> Forwarded: No rule match

    AutoApproved --> [*]: Write allow to Claude
    AutoDenied --> [*]: Write deny to Claude

    Forwarded --> WaitingForClient: Sent to client

    WaitingForClient --> Responded: Client sends response
    WaitingForClient --> ActivityRefresh: User activity detected
    WaitingForClient --> Expired: 7 days no activity

    ActivityRefresh --> WaitingForClient: TTL reset

    Responded --> [*]: Write response to Claude
    Expired --> [*]: Write auto-deny to Claude
```
