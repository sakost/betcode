# Permission Request Timeout Redesign

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## The Problem

**The current 60-second timeout is wrong.**

A mobile user may:
- Be on a slow network (5+ seconds latency)
- Have the app backgrounded
- Need time to read and understand the request
- Switch to another app to verify the command is safe
- Be in a meeting and unable to respond immediately

**60 seconds is hostile to real usage patterns.**

---

## Proposed: 7-Day TTL with Activity-Based Refresh

```
Claude emits control_request
         |
         v
Create pending permission with 7-day base TTL
         |
         v
Forward to client(s)
         |
         v
+--------------------------------------------------+
|                 WAITING STATE                    |
|                                                  |
|  TTL refreshed on ANY client activity:           |
|  - Client connects/reconnects                    |
|  - Client sends heartbeat                        |
|  - Client views session                          |
|  - Client interacts with any session             |
|                                                  |
|  TTL: 7 days from last activity                  |
+--------------------------------------------------+
         |
    +----+----+----+
    |         |    |
    v         v    v
 Client    No      Session
 responds  activity closed
           7 days
    |         |    |
    v         v    v
 Process   Auto-  Auto-
 response  DENY   DENY
```

---

## Why This is Better

| Scenario | Old (60s) | New (7-day TTL) |
|----------|-----------|-----------------|
| User in meeting, phone in pocket | Auto-denied | Waits for user |
| App backgrounded for 2 minutes | Auto-denied | Waits for user |
| Network outage for 5 minutes | Auto-denied | Waits for user |
| User forgot about session for a week | Auto-denied | Auto-denied |
| User actively using app but slow to decide | Auto-denied | Waits for user |

---

## Activity-Based Refresh Logic

```rust
struct PendingPermission {
    request_id: String,
    session_id: String,
    tool_name: String,
    input: serde_json::Value,
    created_at: Instant,
    last_activity: Instant,  // Refreshed on ANY user activity
    base_ttl: Duration,      // 7 days
}

impl PendingPermission {
    fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > self.base_ttl
    }

    fn refresh(&mut self) {
        self.last_activity = Instant::now();
    }
}
```

---

## Push Notification Schedule

With 7-day TTL, push notifications become critical:

| Timing | Notification |
|--------|--------------|
| Immediate | "BetCode needs permission: cargo test" |
| 1 hour | "BetCode still waiting for permission" |
| 24 hours | "Permission request pending for 24 hours" |
| 6 days | "Permission will auto-deny in 24 hours" |

Users can configure notification frequency in settings.

---

## Backward Compatibility

The 7-day TTL is a default. For automated/CI scenarios, configure shorter timeouts:

```json
{
  "permission": {
    "default_ttl_seconds": 604800,
    "headless_ttl_seconds": 60,
    "refresh_on_activity": true
  }
}
```

---

## Arguments Against 60-Second Timeout

1. **Mobile networks are unreliable**: 60 seconds assumes always-on, low-latency
   connectivity that mobile users do not have.

2. **Users have lives**: People step away from their phones. A coding agent should
   not make irrevocable decisions because a human took a bathroom break.

3. **Permission decisions are important**: If Claude is asking for permission, the
   action matters. Rushing users into denials is worse than waiting.

4. **Auto-deny is destructive**: An auto-denied permission may cause the session to
   fail or produce incorrect results. The user never even saw the request.

5. **Activity tracking solves abandonment**: If the user is active, they will respond.
   If they are truly gone, the 7-day timeout catches it.
