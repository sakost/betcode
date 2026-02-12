# CLI Setup Wizard Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `betcode-setup cli` subcommand that walks users through configuring relay URL, user registration/login, and machine setup in one interactive wizard.

**Architecture:** Shells out to the existing `betcode` CLI binary for auth and machine operations (no gRPC deps in betcode-setup). Password is never passed as a CLI argument — uses `BETCODE_PASSWORD` env var instead. Prompts via dialoguer, config written to `~/.betcode/config.json` by the `betcode` CLI itself.

**Tech Stack:** Rust, clap, dialoguer, std::process::Command

---

## Prerequisites

The `betcode` CLI auth commands currently accept `-p <password>` only as a CLI arg. We need to add `BETCODE_PASSWORD` env var support first, so the setup wizard can pass the password securely without it appearing in `ps` output.

---

### Task 1: Add `BETCODE_PASSWORD` env var support to betcode-cli auth commands

**Files:**
- Modify: `crates/betcode-cli/src/auth_cmd.rs:37-55`

**Step 1: Add `env` attribute to password args**

In `AuthAction::Register` and `AuthAction::Login`, add `env = "BETCODE_PASSWORD"` to the password field:

```rust
/// Register a new account on the relay server.
Register {
    /// Username.
    #[arg(short, long)]
    username: String,
    /// Password (or set BETCODE_PASSWORD env var).
    #[arg(short, long, env = "BETCODE_PASSWORD")]
    password: String,
    /// Email address.
    #[arg(short, long, default_value = "")]
    email: String,
},
/// Log in to a relay server.
Login {
    /// Username.
    #[arg(short, long)]
    username: String,
    /// Password (or set BETCODE_PASSWORD env var).
    #[arg(short, long, env = "BETCODE_PASSWORD")]
    password: String,
},
```

**Step 2: Verify it compiles**

Run: `cargo clippy -p betcode-cli`
Expected: clean

**Step 3: Commit**

```
feat(cli): support BETCODE_PASSWORD env var for auth commands
```

---

### Task 2: Fix TLS `with_enabled_roots()` in betcode-cli relay connections

The CLI has the same TLS bug as the daemon — `ClientTlsConfig::new()` without `with_enabled_roots()` fails on NixOS and other systems where rustls can't find native certs.

**Files:**
- Modify: `crates/betcode-cli/src/auth_cmd.rs:17-31`
- Modify: `crates/betcode-cli/src/machine_cmd.rs:71-89`
- Modify: `crates/betcode-cli/Cargo.toml` (add `tls-webpki-roots` feature to tonic)

**Step 1: Add `tls-webpki-roots` to tonic features in Cargo.toml**

Find the tonic dependency line and add `tls-webpki-roots`:

```toml
tonic = { workspace = true, features = ["tls-ring", "tls-native-roots", "tls-webpki-roots"] }
```

**Step 2: Fix `relay_channel()` in `auth_cmd.rs`**

Replace the TLS config block (lines 17-31):

```rust
async fn relay_channel(url: &str, ca_cert: Option<&Path>) -> anyhow::Result<Channel> {
    let mut endpoint = Channel::from_shared(url.to_string())?;
    if url.starts_with("https://") {
        let mut tls_config = ClientTlsConfig::new().with_enabled_roots();
        if let Some(ca_path) = ca_cert {
            let ca_pem = std::fs::read_to_string(ca_path)
                .map_err(|e| anyhow::anyhow!("Failed to read CA cert {}: {}", ca_path.display(), e))?;
            tls_config = tls_config.ca_certificate(Certificate::from_pem(ca_pem));
        }
        endpoint = endpoint
            .tls_config(tls_config)
            .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
    }
    endpoint
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to relay: {e}"))
}
```

**Step 3: Fix `connect_relay()` in `machine_cmd.rs`**

Same pattern — replace lines 71-89:

```rust
async fn connect_relay(config: &CliConfig) -> anyhow::Result<Channel> {
    let relay_url = config
        .relay_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No relay URL configured. Use --relay <url>"))?;
    let mut endpoint = Channel::from_shared(relay_url.clone())?;
    if relay_url.starts_with("https://") {
        let mut tls_config = ClientTlsConfig::new().with_enabled_roots();
        if let Some(ca_path) = &config.relay_ca_cert {
            let ca_pem = std::fs::read_to_string(ca_path)
                .map_err(|e| anyhow::anyhow!("Failed to read CA cert {}: {}", ca_path.display(), e))?;
            tls_config = tls_config.ca_certificate(Certificate::from_pem(ca_pem));
        }
        endpoint = endpoint
            .tls_config(tls_config)
            .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
    }
    endpoint
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to relay: {e}"))
}
```

**Step 4: Verify**

Run: `cargo clippy -p betcode-cli`
Expected: clean

**Step 5: Commit**

```
fix(cli): use with_enabled_roots() for TLS relay connections
```

---

### Task 3: Add CLI-specific prompts to `prompt.rs`

**Files:**
- Modify: `crates/betcode-setup/src/prompt.rs`

**Step 1: Add new prompt functions**

Append these functions to the end of `prompt.rs`:

```rust
/// Prompt for a relay URL.
pub fn prompt_relay_url(non_interactive: bool, default: &str) -> Result<String> {
    if non_interactive {
        return Ok(default.to_string());
    }
    let url: String = Input::new()
        .with_prompt("Relay URL")
        .default(default.to_string())
        .interact_text()?;
    Ok(url)
}

/// Prompt for a username.
pub fn prompt_username(non_interactive: bool) -> Result<String> {
    if non_interactive {
        anyhow::bail!("--username is required in non-interactive mode");
    }
    let username: String = Input::new()
        .with_prompt("Username")
        .interact_text()?;
    Ok(username)
}

/// Prompt for a password (hidden input).
pub fn prompt_password(non_interactive: bool) -> Result<String> {
    if non_interactive {
        // In non-interactive mode, read from BETCODE_PASSWORD env var
        std::env::var("BETCODE_PASSWORD").map_err(|_| {
            anyhow::anyhow!(
                "BETCODE_PASSWORD env var is required in non-interactive mode"
            )
        })
    } else {
        let password: String = Password::new()
            .with_prompt("Password")
            .interact()?;
        Ok(password)
    }
}

/// Prompt for register vs login.
pub fn prompt_auth_action(non_interactive: bool) -> Result<bool> {
    if non_interactive {
        // Default to login in non-interactive mode
        return Ok(true);
    }
    let items = &["Login (existing account)", "Register (new account)"];
    let selection = Select::new()
        .with_prompt("Do you have an account on this relay?")
        .items(items)
        .default(0)
        .interact()?;
    // returns true for login, false for register
    Ok(selection == 0)
}

/// Prompt for a machine name.
pub fn prompt_machine_name(non_interactive: bool, default: &str) -> Result<String> {
    if non_interactive {
        return Ok(default.to_string());
    }
    let name: String = Input::new()
        .with_prompt("Machine name for this computer")
        .default(default.to_string())
        .interact_text()?;
    Ok(name)
}

/// Prompt user to select from a list of items. Returns the index.
pub fn prompt_select(prompt_text: &str, items: &[String]) -> Result<usize> {
    let selection = Select::new()
        .with_prompt(prompt_text)
        .items(items)
        .default(0)
        .interact()?;
    Ok(selection)
}
```

**Step 2: Verify**

Run: `cargo clippy -p betcode-setup`
Expected: clean

**Step 3: Commit**

```
feat(setup): add CLI wizard prompt helpers
```

---

### Task 4: Create `cli/mod.rs` — CLI wizard orchestrator with `CliArgs`

**Files:**
- Create: `crates/betcode-setup/src/cli/mod.rs`
- Modify: `crates/betcode-setup/src/lib.rs` (add `pub mod cli;`)
- Modify: `crates/betcode-setup/src/main.rs` (add `Cli` variant to `Commands`)

**Step 1: Create `cli/mod.rs`**

```rust
mod wizard;

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

/// Arguments for the `cli` subcommand.
#[derive(Debug, Args)]
pub struct CliArgs {
    /// Relay URL (e.g. https://relay.example.com:443)
    #[arg(long)]
    pub relay: Option<String>,

    /// Path to the betcode CLI binary (default: search PATH)
    #[arg(long)]
    pub cli_binary: Option<PathBuf>,

    /// Username for authentication
    #[arg(long)]
    pub username: Option<String>,

    /// Machine name for this computer
    #[arg(long)]
    pub machine_name: Option<String>,

    /// Register a new account (instead of login)
    #[arg(long, conflicts_with = "login")]
    pub register: bool,

    /// Login to existing account (instead of register)
    #[arg(long, conflicts_with = "register")]
    pub login: bool,
}

/// Run the CLI setup wizard.
pub fn run(args: CliArgs, non_interactive: bool) -> Result<()> {
    let betcode = wizard::find_betcode_binary(args.cli_binary.as_deref())?;
    wizard::run_wizard(&betcode, &args, non_interactive)
}
```

**Step 2: Add `pub mod cli;` to `lib.rs`**

Add to `crates/betcode-setup/src/lib.rs`:

```rust
pub mod cli;
pub mod cmd;
pub mod config;
pub mod escalate;
pub mod os;
pub mod prompt;
pub mod relay;
```

**Step 3: Add `Cli` variant to `Commands` in `main.rs`**

Update `main.rs` to import and dispatch:

```rust
use betcode_setup::cli::CliArgs;
use betcode_setup::relay::RelayArgs;

// ...

#[derive(Debug, Subcommand)]
enum Commands {
    /// Set up the betcode relay server
    Relay(RelayArgs),
    /// Set up the betcode CLI for relay access
    Cli(CliArgs),
}

// In main():
match cli.command {
    Commands::Relay(args) => betcode_setup::relay::run(args, cli.non_interactive)?,
    Commands::Cli(args) => betcode_setup::cli::run(args, cli.non_interactive)?,
}
```

**Step 4: Verify it compiles (wizard.rs doesn't exist yet — create a stub)**

Create `crates/betcode-setup/src/cli/wizard.rs` with stubs:

```rust
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::cmd::command_exists;

use super::CliArgs;

/// Locate the betcode CLI binary.
pub fn find_betcode_binary(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        let resolved = std::fs::canonicalize(path)
            .with_context(|| format!("betcode binary not found: {}", path.display()))?;
        return Ok(resolved);
    }
    if command_exists("betcode") {
        return Ok(PathBuf::from("betcode"));
    }
    bail!(
        "betcode CLI not found on PATH. Install it first, or use --cli-binary <path>"
    );
}

/// Run the interactive CLI setup wizard.
pub fn run_wizard(_betcode: &Path, _args: &CliArgs, _non_interactive: bool) -> Result<()> {
    anyhow::bail!("CLI wizard not yet implemented")
}
```

Add the missing import:

```rust
use anyhow::{bail, Context, Result};
```

Run: `cargo clippy -p betcode-setup`
Expected: clean

**Step 5: Commit**

```
feat(setup): add cli subcommand skeleton with CliArgs
```

---

### Task 5: Implement `cli/wizard.rs` — full wizard logic

**Files:**
- Modify: `crates/betcode-setup/src/cli/wizard.rs`

**Step 1: Implement the full wizard**

Replace the stub `run_wizard` with the full implementation:

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::cmd::command_exists;
use crate::prompt;

use super::CliArgs;

/// Locate the betcode CLI binary.
pub fn find_betcode_binary(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        let resolved = std::fs::canonicalize(path)
            .with_context(|| format!("betcode binary not found: {}", path.display()))?;
        return Ok(resolved);
    }
    if command_exists("betcode") {
        return Ok(PathBuf::from("betcode"));
    }
    bail!(
        "betcode CLI not found on PATH. Install it first, or use --cli-binary <path>"
    );
}

/// Run the interactive CLI setup wizard.
pub fn run_wizard(betcode: &Path, args: &CliArgs, non_interactive: bool) -> Result<()> {
    // Step 1: Relay URL
    let relay_url = match &args.relay {
        Some(url) => url.clone(),
        None => prompt::prompt_relay_url(non_interactive, "https://relay.example.com:443")?,
    };
    tracing::info!("relay URL: {relay_url}");

    // Step 2: Auth — register or login
    let username = match &args.username {
        Some(u) => u.clone(),
        None => prompt::prompt_username(non_interactive)?,
    };
    let password = prompt::prompt_password(non_interactive)?;

    let is_login = if args.login {
        true
    } else if args.register {
        false
    } else {
        prompt::prompt_auth_action(non_interactive)?
    };

    if is_login {
        run_betcode_auth(betcode, &relay_url, "login", &username, &password)?;
    } else {
        run_betcode_auth(betcode, &relay_url, "register", &username, &password)?;
    }

    // Step 3: Machine setup
    let machine_name = match &args.machine_name {
        Some(n) => n.clone(),
        None => {
            let hostname = gethostname();
            prompt::prompt_machine_name(non_interactive, &hostname)?
        }
    };

    let machine_id = run_betcode_machine_register(betcode, &relay_url, &machine_name)?;
    run_betcode_machine_switch(betcode, &machine_id)?;

    #[allow(clippy::print_stdout)]
    {
        println!();
        println!("CLI setup complete!");
        println!();
        println!("  Relay:   {relay_url}");
        println!("  User:    {username}");
        println!("  Machine: {machine_name} ({machine_id})");
        println!();
        println!("You can now use betcode in relay mode. Try:");
        println!("  betcode auth status");
    }

    Ok(())
}

/// Run `betcode auth login` or `betcode auth register`.
fn run_betcode_auth(
    betcode: &Path,
    relay_url: &str,
    action: &str,
    username: &str,
    password: &str,
) -> Result<()> {
    tracing::info!("{action}ing as {username}");

    let output = Command::new(betcode)
        .args(["--relay", relay_url, "auth", action, "-u", username])
        .env("BETCODE_PASSWORD", password)
        .output()
        .with_context(|| format!("failed to run betcode auth {action}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("auth {action} failed: {stdout}{stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::info!("{}", stdout.trim());
    Ok(())
}

/// Run `betcode machine register` and extract the machine ID from output.
fn run_betcode_machine_register(
    betcode: &Path,
    relay_url: &str,
    name: &str,
) -> Result<String> {
    tracing::info!("registering machine: {name}");

    let output = Command::new(betcode)
        .args(["--relay", relay_url, "machine", "register", "--name", name])
        .output()
        .with_context(|| "failed to run betcode machine register")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("machine register failed: {stdout}{stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse "  ID:   <uuid>" from output
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(id) = trimmed.strip_prefix("ID:") {
            return Ok(id.trim().to_string());
        }
    }

    bail!("could not parse machine ID from output:\n{stdout}");
}

/// Run `betcode machine switch <id>`.
fn run_betcode_machine_switch(betcode: &Path, machine_id: &str) -> Result<()> {
    tracing::info!("switching to machine: {machine_id}");

    let output = Command::new(betcode)
        .args(["machine", "switch", machine_id])
        .output()
        .with_context(|| "failed to run betcode machine switch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("machine switch failed: {stderr}");
    }
    Ok(())
}

/// Get the system hostname for the default machine name.
fn gethostname() -> String {
    crate::cmd::run_cmd_output("hostname", &[])
        .unwrap_or_else(|_| "my-machine".to_string())
}
```

**Step 2: Verify**

Run: `cargo clippy -p betcode-setup`
Expected: clean

**Step 3: Manual test**

Run: `cargo run -p betcode-setup -- cli --help`
Expected output showing all flags.

**Step 4: Commit**

```
feat(setup): implement CLI setup wizard with auth and machine registration
```

---

### Task 6: Remove `ensure_ubuntu()` gate for CLI subcommand

The `main.rs` currently calls `os::ensure_ubuntu()` before dispatching. The CLI wizard doesn't need to run on Ubuntu — it should work on any platform. Move the Ubuntu check into the relay subcommand only.

**Files:**
- Modify: `crates/betcode-setup/src/main.rs`

**Step 1: Move Ubuntu check into relay branch**

```rust
fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Relay(args) => {
            betcode_setup::os::ensure_ubuntu()?;
            betcode_setup::relay::run(args, cli.non_interactive)?;
        }
        Commands::Cli(args) => betcode_setup::cli::run(args, cli.non_interactive)?,
    }

    Ok(())
}
```

**Step 2: Verify**

Run: `cargo clippy -p betcode-setup`
Expected: clean

**Step 3: Commit**

```
fix(setup): only require Ubuntu for relay subcommand, not cli
```

---

## Implementation Order

| Task | What | Depends on |
|------|------|------------|
| 1 | `BETCODE_PASSWORD` env var in betcode-cli | - |
| 2 | Fix TLS `with_enabled_roots()` in betcode-cli | - |
| 3 | Prompt helpers in betcode-setup | - |
| 4 | CLI subcommand skeleton (`CliArgs`, `mod.rs`) | 3 |
| 5 | Full wizard implementation (`wizard.rs`) | 1, 4 |
| 6 | Remove Ubuntu gate for CLI subcommand | 4 |

Tasks 1, 2, and 3 are independent and can be done in parallel.

## Verification

1. `cargo clippy -p betcode-cli` — clean
2. `cargo clippy -p betcode-setup` — clean
3. `cargo run -p betcode-setup -- cli --help` — shows all options
4. `cargo run -p betcode-setup -- cli --relay https://relay.ai.sakost.dev:443` — runs the interactive wizard
5. `betcode auth status` — shows logged-in state after wizard completes
