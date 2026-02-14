# Executable Specification: Worktree Lifecycle

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Specification

---

## Purpose

Executable examples for BetCode's Worktree lifecycle behavior. Each scenario is
specific, testable, and covers ONE behavior.

---

## Feature: Worktree Creation

### Background

```gherkin
Background:
  Given the daemon is running
  And the main repository is at "/home/user/project"
  And the repository has branches: main, develop, feature/auth
```

### Scenario: Successful creation for existing branch

```gherkin
When CreateWorktree is called:
  | repo_path | /home/user/project |
  | branch    | feature/auth       |
  | name      | auth-work          |
Then the daemon executes:
  "git worktree add ../project-auth-work feature/auth"
And a worktree row is inserted:
  | name   | auth-work                    |
  | path   | /home/user/project-auth-work |
  | branch | feature/auth                 |
And the response contains the worktree info
```

### Scenario: Successful creation with new branch

```gherkin
When CreateWorktree is called:
  | repo_path     | /home/user/project |
  | branch        | feature/new-thing  |
  | create_branch | true               |
Then the daemon executes:
  "git worktree add -b feature/new-thing ../project-new-thing"
And a worktree row is inserted with branch "feature/new-thing"
```

### Scenario: Fails - branch already checked out

```gherkin
Given branch "develop" is currently checked out in the main repository
When CreateWorktree is called:
  | repo_path | /home/user/project |
  | branch    | develop            |
Then the response is an error:
  | code    | ALREADY_EXISTS                                                    |
  | message | Branch 'develop' is already checked out at '/home/user/project'   |
And no worktree row is inserted
And no filesystem changes occur
```

### Scenario: Fails - insufficient disk space

```gherkin
Given the filesystem has 50MB free space
And the repository requires 500MB
When CreateWorktree is called for feature/auth
Then the response is an error:
  | code    | RESOURCE_EXHAUSTED                         |
  | message | Insufficient disk space to create worktree |
  | details | {"required_bytes": 524288000, "available_bytes": 52428800} |
And no worktree row is inserted
```

### Scenario: Fails - path already exists

```gherkin
Given directory "/home/user/project-auth-work" already exists
When CreateWorktree is called with name "auth-work"
Then the response is an error:
  | code    | ALREADY_EXISTS                                     |
  | message | Path '/home/user/project-auth-work' already exists |
```

### Scenario: Fails - branch does not exist

```gherkin
When CreateWorktree is called:
  | branch        | feature/nonexistent |
  | create_branch | false               |
Then the response is an error:
  | code    | NOT_FOUND                                   |
  | message | Branch 'feature/nonexistent' does not exist |
```

### Scenario: Fails - invalid branch name

```gherkin
When CreateWorktree is called:
  | branch        | feature/bad..name |
  | create_branch | true              |
Then the response is an error:
  | code    | INVALID_ARGUMENT                                      |
  | message | Invalid branch name: 'feature/bad..name' contains '..' |
```

### Scenario: Fails - name contains path separator

```gherkin
When CreateWorktree is called with name "foo/bar"
Then the response is an error:
  | code    | INVALID_ARGUMENT                             |
  | message | Worktree name cannot contain path separators |
```

### Scenario: Atomicity - git failure rolls back

```gherkin
Given git worktree add will fail with "fatal: unable to checkout working tree"
When CreateWorktree is called
Then the response is an error:
  | code    | INTERNAL                     |
  | message | Git worktree creation failed |
  | details | {"stderr": "fatal: unable to checkout working tree"} |
And no worktree row is inserted
And any partial directory is cleaned up
```

---

## Feature: External Deletion Detection

### Background

```gherkin
Background:
  Given a worktree exists:
    | id     | wt_001                       |
    | path   | /home/user/project-auth-work |
    | branch | feature/auth                 |
```

### Scenario: Directory deleted externally

```gherkin
Given the directory "/home/user/project-auth-work" is deleted externally
When ListWorktrees is called
Then the response includes worktree "wt_001" with:
  | status  | stale                             |
  | message | Worktree directory no longer exists |
And the daemon logs at WARN level:
  "Worktree wt_001 directory missing: /home/user/project-auth-work"
```

### Scenario: Switch to deleted worktree fails

```gherkin
Given the directory "/home/user/project-auth-work" is deleted externally
When SwitchWorktree is called for wt_001
Then the response is an error:
  | code    | FAILED_PRECONDITION                 |
  | message | Worktree directory no longer exists |
  | details | {"path": "/home/user/project-auth-work"} |
```

### Scenario: Session spawn fails for missing worktree

```gherkin
Given the directory is deleted externally
And session "sess_002" is associated with worktree "wt_001"
When a client sends a UserMessage to session "sess_002"
Then the response is an error:
  | code    | FAILED_PRECONDITION                                    |
  | message | Cannot spawn Claude: worktree directory does not exist |
And no Claude subprocess is spawned
```

### Scenario: Startup reconciliation marks stale

```gherkin
Given the daemon was stopped
And the worktree directory was deleted while daemon was stopped
When the daemon starts
Then the worktree is marked as stale (not deleted)
And the daemon logs at WARN level about missing directory
```

### Scenario: RemoveWorktree cleans up stale worktree

```gherkin
Given worktree "wt_001" is marked as stale
When RemoveWorktree is called with force=true
Then the worktree row is deleted
And the daemon does NOT run "git worktree remove" (directory already gone)
And the response contains removed=true
```

### Scenario: Git worktree prune on stale detection

```gherkin
Given worktree "wt_001" directory is missing
When the daemon detects the stale worktree
Then the daemon executes "git worktree prune"
```

---

## Feature: Setup Script Execution

### Scenario: Setup script succeeds

```gherkin
Given the settings contain:
  {"worktree": {"setup_commands": ["npm install", "npm run build"]}}
When CreateWorktree is called
Then the worktree is created
And the daemon executes in sequence with cwd=worktree_path:
  | command       | expected_exit_code |
  | npm install   | 0                  |
  | npm run build | 0                  |
And the response shows success
```

### Scenario: Setup script fails - worktree still created

```gherkin
Given the settings contain setup_commands: ["npm install"]
And "npm install" will fail with exit code 1
When CreateWorktree is called
Then the worktree is created (git worktree add succeeded)
And the worktree row is inserted
And the response contains:
  | setup_status | failed                         |
  | setup_error  | npm install exited with code 1 |
And the daemon logs at WARN level
```

### Scenario: Setup script times out

```gherkin
Given the settings contain:
  {"worktree": {"setup_commands": ["npm install"], "setup_timeout_secs": 60}}
And "npm install" will hang indefinitely
When CreateWorktree is called
Then after 60 seconds, the setup command is killed
And the response contains:
  | setup_status | timeout                                        |
  | setup_error  | Setup command 'npm install' timed out after 60s |
```

### Scenario: Custom setup script from .betcode directory

```gherkin
Given file ".betcode/worktree-setup.sh" exists in the repository
When CreateWorktree is called
Then the daemon executes the setup script with cwd=worktree_path
```

### Scenario: No setup configured

```gherkin
Given no setup_commands in settings
And no .betcode/worktree-setup.sh file
When CreateWorktree is called
Then the worktree is created
And no setup commands are executed
And the response contains setup_status="none"
```

### Scenario: Setup script environment

```gherkin
Given setup_commands: ["env"]
When CreateWorktree is called
Then the setup script receives:
  | variable        | value               |
  | PATH            | <inherited>         |
  | HOME            | <inherited>         |
  | WORKTREE_PATH   | <worktree_path>     |
  | WORKTREE_BRANCH | <branch_name>       |
And the setup script does NOT receive ANTHROPIC_API_KEY
```

---

## Undefined Behavior

1. **Concurrent creation for same branch** - Order and success undefined
2. **Directory modified externally while session active** - Results undefined
3. **Path exceeds filesystem limits** - Platform-specific behavior

---

## Related Documents

- [DAEMON.md](./DAEMON.md) - Worktree orchestration implementation
- [SCHEMAS.md](./SCHEMAS.md) - worktrees table definition
- [TESTING_PERMISSIONS.md](./TESTING_PERMISSIONS.md) - Permission test scenarios
