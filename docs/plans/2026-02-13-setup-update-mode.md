# Setup Update Mode Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `betcode-setup relay` idempotent — auto-detect existing installations and update in-place without clobbering config or fighting port conflicts.

**Architecture:** Add `is_update` detection based on systemd unit file existence. Thread this flag through the deploy pipeline to skip env-file overwrite, skip certbot when cert exists, and restart (not start) the running service. Also make the listen address configurable instead of hardcoding port 443.

**Tech Stack:** Rust, clap, anyhow, std::net, std::path, tracing

---

## Task 1: Add `addr` field to `RelaySetupConfig` and CLI args

**Files:**
- Modify: `crates/betcode-setup/src/config.rs`
- Modify: `crates/betcode-setup/src/relay/mod.rs` (RelayArgs + run())
- Modify: `crates/betcode-setup/src/relay/templates.rs` (systemd_unit, docker_compose)
- Test: `crates/betcode-setup/src/relay/templates.rs` (inline #[cfg(test)])

**Step 1: Write failing tests for templates**

In `crates/betcode-setup/src/relay/templates.rs`, add `#[cfg(test)] mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DeploymentMode, RelaySetupConfig};
    use std::net::SocketAddr;
    use std::path::PathBuf;

    fn test_config(addr: SocketAddr) -> RelaySetupConfig {
        RelaySetupConfig {
            domain: "relay.example.com".into(),
            jwt_secret: "a]".repeat(24),
            db_path: PathBuf::from("/var/lib/betcode/relay.db"),
            deployment_mode: DeploymentMode::Systemd,
            relay_binary_path: None,
            addr,
        }
    }

    #[test]
    fn systemd_unit_contains_addr_flag() {
        let config = test_config("0.0.0.0:8443".parse().unwrap());
        let unit = systemd_unit(&config);
        assert!(unit.contains("--addr 0.0.0.0:8443"), "unit must contain --addr flag");
    }

    #[test]
    fn systemd_unit_default_port_omits_addr() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let unit = systemd_unit(&config);
        // Default port: --addr flag should NOT appear (keep ExecStart clean)
        assert!(!unit.contains("--addr"), "default port should not emit --addr");
    }

    #[test]
    fn env_file_contains_expected_keys() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let content = env_file(&config);
        assert!(content.contains("BETCODE_JWT_SECRET="));
        assert!(content.contains("BETCODE_DB_PATH="));
    }

    #[test]
    fn docker_compose_uses_configured_port() {
        let config = test_config("0.0.0.0:8443".parse().unwrap());
        let compose = docker_compose(&config);
        assert!(compose.contains("\"8443:8443\""), "compose must map configured port");
    }

    #[test]
    fn docker_compose_default_port() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let compose = docker_compose(&config);
        assert!(compose.contains("\"443:443\""));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p betcode-setup`
Expected: FAIL — `RelaySetupConfig` doesn't have `addr` field yet.

**Step 3: Add `addr` field to config**

In `config.rs`, add to `RelaySetupConfig`:
```rust
use std::net::SocketAddr;

pub struct RelaySetupConfig {
    pub domain: String,
    pub jwt_secret: String,
    pub db_path: PathBuf,
    pub deployment_mode: DeploymentMode,
    pub relay_binary_path: Option<PathBuf>,
    pub addr: SocketAddr,
}
```

**Step 4: Add `--addr` to `RelayArgs`**

In `relay/mod.rs`, add to `RelayArgs`:
```rust
/// Listen address for the relay server
#[arg(long, default_value = "0.0.0.0:443")]
pub addr: std::net::SocketAddr,
```

And update config construction in `run()`:
```rust
let config = RelaySetupConfig {
    domain,
    jwt_secret,
    db_path,
    deployment_mode,
    relay_binary_path: ...,
    addr: args.addr,
};
```

**Step 5: Update `systemd_unit` template**

In `templates.rs`, update `systemd_unit()`:
- If `config.addr.port() != 443`, append `--addr {addr}` to ExecStart
- If port == 443, omit it (default)
- If port < 1024, keep `CAP_NET_BIND_SERVICE`; otherwise remove it

**Step 6: Update `docker_compose` template**

Use `config.addr.port()` for port mapping instead of hardcoded 443.

**Step 7: Run tests**

Run: `cargo test -p betcode-setup`
Expected: PASS

**Step 8: Commit**

```bash
git add crates/betcode-setup/
git commit -m "feat(setup): add --addr flag for configurable listen address"
```

---

## Task 2: Add `is_update` detection and thread through deploy pipeline

**Files:**
- Modify: `crates/betcode-setup/src/relay/mod.rs`
- Modify: `crates/betcode-setup/src/relay/systemd.rs`
- Test: `crates/betcode-setup/src/relay/systemd.rs` (inline #[cfg(test)])

**Step 1: Write failing test for `is_service_installed` helper**

In `systemd.rs`, add test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_service_installed_returns_false_for_missing_file() {
        assert!(!is_service_installed());
        // Or test the helper directly with a path parameter
    }
}
```

Actually, since `is_service_installed` checks a real filesystem path (`/etc/systemd/system/betcode-relay.service`), make the helper accept a path for testability:

```rust
fn service_unit_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

const SERVICE_UNIT_PATH: &str = "/etc/systemd/system/betcode-relay.service";
```

Test:
```rust
#[test]
fn service_unit_exists_returns_false_for_nonexistent_path() {
    assert!(!service_unit_exists("/tmp/does-not-exist.service"));
}

#[test]
fn service_unit_exists_returns_true_for_existing_path() {
    let dir = std::env::temp_dir().join("betcode-test-unit");
    std::fs::write(&dir, "dummy").unwrap();
    assert!(service_unit_exists(dir.to_str().unwrap()));
    std::fs::remove_file(&dir).unwrap();
}
```

**Step 2: Run tests — should fail (function doesn't exist)**

**Step 3: Implement `service_unit_exists` and update `deploy` signature**

In `systemd.rs`:
```rust
const SERVICE_UNIT_PATH: &str = "/etc/systemd/system/betcode-relay.service";

fn service_unit_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

pub fn deploy(config: &RelaySetupConfig) -> Result<()> {
    let is_update = service_unit_exists(SERVICE_UNIT_PATH);
    if is_update {
        tracing::info!("existing installation detected — running in update mode");
    }

    create_system_user()?;
    create_directories(config)?;
    write_env_file(config, is_update)?;
    write_systemd_unit(config)?;
    install_relay_binary(config)?;
    setup_certbot(config, is_update)?;
    enable_and_start(is_update)?;
    Ok(())
}
```

Note: `deploy()` signature stays the same externally — `is_update` is computed internally.

**Step 4: Run tests, verify they pass**

**Step 5: Commit**

```bash
git commit -m "feat(setup): add is_update detection in deploy pipeline"
```

---

## Task 3: Protect relay.env from overwrite on update

**Files:**
- Modify: `crates/betcode-setup/src/relay/systemd.rs` (`write_env_file`)
- Test: `crates/betcode-setup/src/relay/systemd.rs`

**Step 1: Write failing test**

```rust
#[test]
fn write_env_file_skips_existing_on_update() {
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join("relay.env");

    // Pre-populate the file with a known secret
    std::fs::write(&env_path, "BETCODE_JWT_SECRET=original-secret\n").unwrap();

    let config = test_config();
    // Call the inner function with is_update=true and custom path
    let result = write_env_file_inner(&config, true, env_path.to_str().unwrap());
    assert!(result.is_ok());

    // Verify the file was NOT overwritten
    let content = std::fs::read_to_string(&env_path).unwrap();
    assert!(content.contains("original-secret"), "env file must be preserved on update");
}

#[test]
fn write_env_file_creates_on_fresh_install() {
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join("relay.env");

    let config = test_config();
    let result = write_env_file_inner(&config, false, env_path.to_str().unwrap());
    assert!(result.is_ok());

    let content = std::fs::read_to_string(&env_path).unwrap();
    assert!(content.contains("BETCODE_JWT_SECRET="));
}
```

To make this testable, extract an inner function that accepts the path:
```rust
fn write_env_file_inner(config: &RelaySetupConfig, is_update: bool, path: &str) -> Result<()>
```

The public `write_env_file` calls it with the hardcoded path.

**Step 2: Run tests — should fail**

**Step 3: Implement**

```rust
const ENV_FILE_PATH: &str = "/etc/betcode/relay.env";

fn write_env_file(config: &RelaySetupConfig, is_update: bool) -> Result<()> {
    write_env_file_inner(config, is_update, ENV_FILE_PATH)
}

fn write_env_file_inner(config: &RelaySetupConfig, is_update: bool, path: &str) -> Result<()> {
    if is_update && Path::new(path).exists() {
        tracing::warn!(
            "existing {path} preserved — to regenerate, delete it and re-run setup"
        );
        return Ok(());
    }

    tracing::info!("writing environment file: {path}");
    let content = templates::env_file(config);
    fs::write(path, content).context("failed to write relay.env")?;

    #[cfg(not(test))]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o640))
            .context("failed to set permissions on relay.env")?;
        run_cmd("setting ownership on relay.env", "chown", &["root:betcode", path])?;
    }

    Ok(())
}
```

**Step 4: Add `tempfile` to dev-dependencies**

In `crates/betcode-setup/Cargo.toml`:
```toml
[dev-dependencies]
tempfile.workspace = true
```

**Step 5: Run tests — should pass**

**Step 6: Commit**

```bash
git commit -m "feat(setup): preserve existing relay.env on update"
```

---

## Task 4: Skip certbot on update when cert exists

**Files:**
- Modify: `crates/betcode-setup/src/relay/systemd.rs` (`setup_certbot`)
- Test: `crates/betcode-setup/src/relay/systemd.rs`

**Step 1: Write failing test**

```rust
#[test]
fn setup_certbot_skips_when_cert_exists_on_update() {
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("fullchain.pem");
    std::fs::write(&cert_path, "dummy cert").unwrap();

    let config = test_config();
    let result = should_skip_certbot(true, cert_path.to_str().unwrap());
    assert!(result, "certbot should be skipped when cert exists on update");
}

#[test]
fn setup_certbot_runs_on_fresh_install() {
    let result = should_skip_certbot(false, "/nonexistent/fullchain.pem");
    assert!(!result, "certbot should run on fresh install");
}

#[test]
fn setup_certbot_runs_on_update_when_cert_missing() {
    let result = should_skip_certbot(true, "/nonexistent/fullchain.pem");
    assert!(!result, "certbot should run on update when cert is missing");
}
```

**Step 2: Run tests — should fail**

**Step 3: Implement**

Extract a pure `should_skip_certbot` function for testability:
```rust
fn should_skip_certbot(is_update: bool, cert_path: &str) -> bool {
    is_update && Path::new(cert_path).exists()
}
```

Update `setup_certbot`:
```rust
fn setup_certbot(config: &RelaySetupConfig, is_update: bool) -> Result<()> {
    let cert_path = format!("/etc/letsencrypt/live/{}/fullchain.pem", config.domain);
    if should_skip_certbot(is_update, &cert_path) {
        tracing::info!("TLS certificate already exists — skipping certbot setup");
        return Ok(());
    }
    if is_update {
        tracing::info!("TLS certificate not found — running certbot despite update mode");
    }
    // ... existing certbot logic unchanged ...
}
```

**Step 4: Run tests — should pass**

**Step 5: Commit**

```bash
git commit -m "feat(setup): skip certbot on update when cert exists"
```

---

## Task 5: Smart port check — skip when our service is active

**Files:**
- Modify: `crates/betcode-setup/src/relay/validate.rs`
- Modify: `crates/betcode-setup/src/relay/mod.rs` (pass `is_update`)
- Test: `crates/betcode-setup/src/relay/validate.rs`

**Step 1: Write failing tests**

Note: We can't easily test `systemctl is-active` in unit tests. Instead, extract a pure decision function:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_skip_port_check_when_update_and_service_active() {
        assert!(should_skip_port_check(true, true));
    }

    #[test]
    fn should_not_skip_port_check_on_fresh_install() {
        assert!(!should_skip_port_check(false, false));
        assert!(!should_skip_port_check(false, true));
    }

    #[test]
    fn should_not_skip_port_check_when_update_but_service_inactive() {
        assert!(!should_skip_port_check(true, false));
    }
}
```

**Step 2: Run tests — should fail**

**Step 3: Implement**

```rust
fn should_skip_port_check(is_update: bool, service_is_active: bool) -> bool {
    is_update && service_is_active
}

fn is_betcode_relay_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "betcode-relay"])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn check_systemd_prereqs(addr: std::net::SocketAddr, is_update: bool) -> Result<()> {
    if !command_exists("systemctl") {
        bail!("systemctl not found — systemd mode requires a systemd-based system");
    }

    if should_skip_port_check(is_update, is_betcode_relay_active()) {
        tracing::info!("betcode-relay is already active — skipping port check");
        return Ok(());
    }

    match TcpListener::bind(addr) {
        Ok(_listener) => {
            tracing::debug!("port {} is available", addr.port());
        }
        Err(e) => {
            bail!(
                "port {} is already in use ({e}). \
                 Stop the service occupying it before running setup.",
                addr.port()
            );
        }
    }

    Ok(())
}
```

**Step 4: Update `relay/mod.rs`**

Move `is_update` detection to `run()` (before validation), so both `check_systemd_prereqs` and `deploy` use it:

```rust
DeploymentMode::Systemd => {
    crate::escalate::escalate_if_needed(non_interactive)?;
    let is_update = std::path::Path::new("/etc/systemd/system/betcode-relay.service").exists();
    validate::check_systemd_prereqs(config.addr, is_update)?;
    systemd::deploy(&config, is_update)?;
}
```

Update `deploy()` to accept `is_update` as parameter instead of computing it internally.

**Step 5: Run tests — should pass**

**Step 6: Commit**

```bash
git commit -m "feat(setup): skip port check when betcode-relay is already active"
```

---

## Task 6: Restart instead of start on update

**Files:**
- Modify: `crates/betcode-setup/src/relay/systemd.rs` (`enable_and_start`)
- Test: `crates/betcode-setup/src/relay/systemd.rs`

**Step 1: Write failing test**

Extract a pure function for the command args decision:

```rust
#[test]
fn start_args_for_fresh_install() {
    let (desc, args) = start_command_args(false);
    assert_eq!(args, &["enable", "--now", "betcode-relay"]);
}

#[test]
fn start_args_for_update() {
    let (desc, args) = start_command_args(true);
    assert_eq!(args, &["restart", "betcode-relay"]);
}
```

**Step 2: Run tests — should fail**

**Step 3: Implement**

```rust
fn start_command_args(is_update: bool) -> (&'static str, Vec<&'static str>) {
    if is_update {
        ("restarting betcode-relay", vec!["restart", "betcode-relay"])
    } else {
        ("enabling and starting betcode-relay", vec!["enable", "--now", "betcode-relay"])
    }
}

fn enable_and_start(is_update: bool) -> Result<()> {
    let (description, args) = start_command_args(is_update);
    run_cmd(description, "systemctl", &args)?;

    let result = run_cmd("verifying service status", "systemctl", &["is-active", "betcode-relay"]);
    if result.is_err() {
        tracing::warn!("service may not have started correctly — check: journalctl -u betcode-relay");
    }
    tracing::info!("betcode-relay is deployed and running");
    Ok(())
}
```

**Step 4: Run tests — should pass**

**Step 5: Commit**

```bash
git commit -m "feat(setup): restart service on update instead of enable --now"
```

---

## Task 7: Final integration — clippy + full test run

**Step 1:** Run `cargo clippy -p betcode-setup -- -D warnings`
**Step 2:** Run `cargo test -p betcode-setup`
**Step 3:** Run `cargo clippy --workspace -- -D warnings`
**Step 4:** Run `cargo test --workspace`
**Step 5:** Fix any issues

**Step 6: Final commit**

```bash
git commit -m "chore(setup): final cleanup for update mode feature"
```

---

## Files Modified Summary

| File | Change |
|------|--------|
| `crates/betcode-setup/Cargo.toml` | Add `tempfile` dev-dependency |
| `crates/betcode-setup/src/config.rs` | Add `addr: SocketAddr` field |
| `crates/betcode-setup/src/relay/mod.rs` | Add `--addr` arg, `is_update` detection, pass through |
| `crates/betcode-setup/src/relay/templates.rs` | Conditional `--addr` in unit, port in compose, tests |
| `crates/betcode-setup/src/relay/systemd.rs` | `is_update` flag in deploy/write_env/certbot/start, tests |
| `crates/betcode-setup/src/relay/validate.rs` | Accept `addr` + `is_update`, skip port check, tests |

## Verification

1. `cargo test -p betcode-setup` — all new tests pass
2. `cargo clippy --workspace -- -D warnings` — clean
3. `cargo test --workspace` — 750+ tests pass
