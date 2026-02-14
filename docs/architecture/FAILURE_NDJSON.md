# NDJSON Parse Failure Recovery

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## Failure Modes

The daemon reads Claude's stdout line by line, parsing each as JSON. Parse failures
can occur from:

| Failure | Example | Detection |
|---------|---------|-----------|
| Truncated JSON | `{"type":"assistant","mes` | JSON parse error |
| Invalid UTF-8 | Binary data in output | UTF-8 validation |
| Unknown type | `{"type":"new_feature",...}` | Schema validation |
| Missing fields | Required field absent | Schema validation |

---

## Line-Level Recovery

```
Read line from stdout
       |
       v
UTF-8 valid? --No--> Log error, skip line
       |
      Yes
       v
JSON valid? --No--> Log error, skip line
       |
      Yes
       v
Type known? --No--> Log warning, store raw, pass through
       |
      Yes
       v
Schema valid? --No--> Log warning, best-effort parse
       |
      Yes
       v
Process message normally
```

---

## Unknown Message Type Handling

Unknown message types should NOT cause failures. Claude Code may add new types
in updates. The daemon must handle them gracefully:

```rust
match msg_type {
    "system" | "assistant" | "user" | ... => handle_known(value),
    unknown => {
        tracing::warn!(message_type = %unknown, "Unknown NDJSON type, passing through");
        store_raw_message(line, session)?;
        Ok(())
    }
}
```

---

## Error Threshold

If parse errors exceed threshold, the subprocess is likely malfunctioning:

| Condition | Action |
|-----------|--------|
| 5 parse errors in 60 seconds | Kill subprocess, restart with --resume |
| Restart also fails | Mark session as CRASHED, notify clients |

---

## Sequence Gap Detection

The daemon assigns monotonic sequence numbers. Gaps indicate lost messages:

```rust
struct SequenceTracker {
    expected_next: u64,
    gaps: Vec<(u64, u64)>,  // (start, end) of missing sequences
}

impl SequenceTracker {
    fn record(&mut self, sequence: u64) {
        if sequence > self.expected_next {
            self.gaps.push((self.expected_next, sequence - 1));
            tracing::error!(expected = self.expected_next, received = sequence,
                "Sequence gap detected");
        }
        self.expected_next = sequence + 1;
    }
}
```

Gaps are logged and surfaced in observability metrics but do not halt processing.
