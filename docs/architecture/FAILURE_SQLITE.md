# SQLite Corruption Handling

**Version**: 0.2.0
**Last Updated**: 2026-02-03
**Parent**: [FAILURE_MODES.md](./FAILURE_MODES.md)

---

## Corruption Sources

SQLite is remarkably robust, but corruption can occur from:
- Hardware failure (disk errors, power loss during write)
- Software bugs (writing to closed database)
- File system issues (NFS, network drives)
- Concurrent access violations

---

## Detection

### Integrity Check on Startup

```sql
PRAGMA quick_check;        -- Fast, run every startup (<100ms)
PRAGMA integrity_check;    -- Thorough, run weekly or on suspected corruption
PRAGMA foreign_key_check;  -- Referential integrity
```

### Continuous Monitoring

| Signal | Threshold | Action |
|--------|-----------|--------|
| SQLite error rate | >1% of operations | Alert, investigate |
| `SQLITE_CORRUPT` error | Any occurrence | Immediate recovery |
| `SQLITE_IOERR` error | >3 in 60 seconds | Check disk health |
| WAL file size | >100MB | Checkpoint, investigate |

---

## Recovery Procedure

```
Corruption Detected
       |
       v
Close all connections
       |
       v
Copy corrupted file for forensics
(daemon.db -> daemon.db.corrupt.{timestamp})
       |
       v
Attempt sqlite3 .recover
       |
   +---+---+
   |       |
Success  Failure
   |       |
   v       v
Resume   Check for backup
         |
    +----+----+
    |         |
 Exists    No backup
    |         |
    v         v
 Restore   Reinitialize
 from      (data loss)
 backup        |
    |         |
    +----+----+
         |
         v
   Notify user of
   recovery status
```

---

## Backup Strategy

| Backup Type | Frequency | Retention | Location |
|-------------|-----------|-----------|----------|
| WAL checkpoint | Every 5 minutes | N/A | Same directory |
| Hot backup | Every 6 hours | 7 days | `$config_dir/backups/` |
| Pre-migration | Before schema changes | 30 days | `$config_dir/backups/` |

Hot backup uses SQLite's online backup API (`sqlite3_backup_*`) to create
consistent snapshots without blocking writes.

---

## User Communication

**On Successful Recovery**:
```
Session history restored from backup (6 hours old).
Some recent messages may be missing.
```

**On Reinitialization**:
```
Database corruption detected and could not be repaired.
Session history has been reset.
Your settings and worktree configuration have been preserved.
```

---

## Implementation Notes

```rust
enum CorruptionRecovery {
    AttemptRepair,
    RestoreBackup { backup_path: PathBuf, backup_age: Duration },
    Reinitialize,
    ManualIntervention { reason: String },
}

fn handle_corruption(db_path: &Path) -> CorruptionRecovery {
    // 1. Close all connections
    // 2. Copy corrupted file for forensics
    // 3. Attempt sqlite3 .recover
    // 4. Fall back to backup if recovery fails
    // 5. Reinitialize if no backup available
}
```
