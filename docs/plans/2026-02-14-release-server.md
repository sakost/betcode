# betcode-releases: Download Server Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a smart download redirect service at `get.betcode.dev` that detects OS/arch and redirects to GitHub Releases, plus a CI pipeline for cross-platform builds.

**Architecture:** A small axum HTTP service (`betcode-releases` crate) serves three roles: (1) browser landing page with styled download buttons, (2) install script delivery for `curl | sh`, (3) User-Agent-based redirect to the correct GitHub Release asset. All binaries live on GitHub Releases — this service stores nothing. A GitHub Actions workflow cross-compiles on tag push and uploads release assets.

**Tech Stack:** axum 0.8, tokio, reqwest (for optional latest-version lookup), GitHub Actions with `cross` for ARM Linux.

---

## Asset Naming Convention

```
{binary}-{os}-{arch}.tar.gz    # Linux, macOS
{binary}-{os}-{arch}.zip       # Windows
```

Examples:
- `betcode-linux-amd64.tar.gz`
- `betcode-daemon-darwin-arm64.tar.gz`
- `betcode-windows-amd64.zip`
- `betcode-relay-linux-arm64.tar.gz`

## Platform Matrix

| Binary         | linux-amd64 | linux-arm64 | darwin-amd64 | darwin-arm64 | windows-amd64 |
|----------------|:-----------:|:-----------:|:------------:|:------------:|:-------------:|
| betcode        | x | x | x | x | x |
| betcode-daemon | x | x | x | x | x |
| betcode-relay  | x | x |   |   |   |
| betcode-setup  | x | x |   |   |   |

## URL Routing

| URL | Browser (`Accept: text/html`) | CLI (curl/wget) |
|-----|-------------------------------|-----------------|
| `/` | Landing page | `install.sh` body |
| `/install.sh` | Install script | Install script |
| `/install.ps1` | PowerShell script | PowerShell script |
| `/{binary}` | Landing page | 302 → GitHub Release asset |
| `/version` | — | Latest version string (plain text) |

## GitHub Release URL Pattern

```
https://github.com/sakost/betcode/releases/latest/download/{binary}-{os}-{arch}.{ext}
```

No API calls needed. GitHub's `/latest/download/` endpoint handles the redirect natively.

---

## Task 1: Scaffold `betcode-releases` Crate

**Files:**
- Create: `crates/betcode-releases/Cargo.toml`
- Create: `crates/betcode-releases/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create `Cargo.toml`**

```toml
[package]
name = "betcode-releases"
description = "Download redirect server for BetCode binaries"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[[bin]]
name = "betcode-releases"
path = "src/main.rs"

[dependencies]
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap = { workspace = true, features = ["derive", "env"] }
axum = "0.8"
tower-http = { version = "0.6", features = ["cors"] }

[lints]
workspace = true
```

**Step 2: Create minimal `main.rs`**

```rust
use std::net::SocketAddr;

use clap::Parser;
use tracing::info;

#[derive(Parser)]
struct Args {
    /// Listen address
    #[arg(long, default_value = "0.0.0.0:8080", env = "LISTEN_ADDR")]
    addr: SocketAddr,

    /// GitHub repository (owner/repo)
    #[arg(long, default_value = "sakost/betcode", env = "GITHUB_REPO")]
    repo: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    info!(addr = %args.addr, "starting betcode-releases");

    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    let app = axum::Router::new();
    axum::serve(listener, app).await?;
    Ok(())
}
```

**Step 3: Add to workspace**

In root `Cargo.toml`, add `"crates/betcode-releases"` to the `members` array.

Also add to `[workspace.dependencies]`:
```toml
anyhow = "1"
```

(anyhow is already there — just verify.)

**Step 4: Verify it compiles**

Run: `cargo clippy -p betcode-releases -- -D warnings`
Expected: clean build

**Step 5: Commit**

```bash
git add crates/betcode-releases/ Cargo.toml
git commit -m "feat(releases): scaffold betcode-releases crate with axum"
```

---

## Task 2: User-Agent Parsing & Platform Detection

**Files:**
- Create: `crates/betcode-releases/src/platform.rs`
- Modify: `crates/betcode-releases/src/main.rs`

**Step 1: Write the failing test**

Create `crates/betcode-releases/src/platform.rs`:

```rust
/// Detected platform from a User-Agent string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Darwin,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Amd64,
    Arm64,
}

impl std::fmt::Display for Os {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Linux => write!(f, "linux"),
            Self::Darwin => write!(f, "darwin"),
            Self::Windows => write!(f, "windows"),
        }
    }
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Amd64 => write!(f, "amd64"),
            Self::Arm64 => write!(f, "arm64"),
        }
    }
}

impl Platform {
    /// Archive extension for this platform.
    pub fn ext(&self) -> &'static str {
        match self.os {
            Os::Windows => "zip",
            _ => "tar.gz",
        }
    }
}

/// Parse OS and architecture from a User-Agent header.
/// Returns `None` if detection fails.
pub fn detect(user_agent: &str) -> Option<Platform> {
    let ua = user_agent.to_ascii_lowercase();

    let os = if ua.contains("windows") || ua.contains("win64") || ua.contains("win32") {
        Os::Windows
    } else if ua.contains("mac") || ua.contains("darwin") {
        Os::Darwin
    } else if ua.contains("linux") {
        Os::Linux
    } else {
        // curl default UA is "curl/x.y.z" with no OS info.
        // wget is "Wget/x.y.z" — also no OS.
        // Fall back to Linux as most likely server/dev environment.
        Os::Linux
    };

    let arch = if ua.contains("aarch64") || ua.contains("arm64") {
        Arch::Arm64
    } else {
        // Default to amd64 — most common for downloads.
        Arch::Amd64
    };

    Some(Platform { os, arch })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_on_linux_amd64() {
        let p = detect("curl/8.5.0").unwrap();
        assert_eq!(p.os, Os::Linux);
        assert_eq!(p.arch, Arch::Amd64);
    }

    #[test]
    fn curl_on_macos_arm64() {
        // macOS curl includes system info
        let p = detect("curl/8.4.0 (aarch64-apple-darwin23.0) ...").unwrap();
        assert_eq!(p.os, Os::Darwin);
        assert_eq!(p.arch, Arch::Arm64);
    }

    #[test]
    fn wget_on_linux() {
        let p = detect("Wget/1.21.4 (linux-gnu)").unwrap();
        assert_eq!(p.os, Os::Linux);
        assert_eq!(p.arch, Arch::Amd64);
    }

    #[test]
    fn powershell_on_windows() {
        let p = detect("Mozilla/5.0 (Windows NT 10.0; Win64; x64) PowerShell/7.4").unwrap();
        assert_eq!(p.os, Os::Windows);
        assert_eq!(p.arch, Arch::Amd64);
    }

    #[test]
    fn chrome_on_macos() {
        let p = detect("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36").unwrap();
        assert_eq!(p.os, Os::Darwin);
        assert_eq!(p.arch, Arch::Amd64);
    }

    #[test]
    fn windows_extension() {
        let p = Platform { os: Os::Windows, arch: Arch::Amd64 };
        assert_eq!(p.ext(), "zip");
    }

    #[test]
    fn linux_extension() {
        let p = Platform { os: Os::Linux, arch: Arch::Amd64 };
        assert_eq!(p.ext(), "tar.gz");
    }
}
```

**Step 2: Add `mod platform;` to `main.rs`**

**Step 3: Run tests**

Run: `cargo test -p betcode-releases`
Expected: all 7 tests pass

**Step 4: Commit**

```bash
git add crates/betcode-releases/src/platform.rs crates/betcode-releases/src/main.rs
git commit -m "feat(releases): add User-Agent platform detection"
```

---

## Task 3: Binary Registry & Redirect URL Builder

**Files:**
- Create: `crates/betcode-releases/src/registry.rs`
- Modify: `crates/betcode-releases/src/main.rs`

**Step 1: Create `registry.rs`**

```rust
use crate::platform::{Os, Platform};

/// Known binaries and their platform availability.
const CLIENT_BINARIES: &[&str] = &["betcode", "betcode-daemon"];
const SERVER_BINARIES: &[&str] = &["betcode-relay", "betcode-setup"];

/// Check if a binary name is valid.
pub fn is_valid_binary(name: &str) -> bool {
    CLIENT_BINARIES.contains(&name) || SERVER_BINARIES.contains(&name)
}

/// Check if a binary is available on the given platform.
pub fn is_available(binary: &str, platform: &Platform) -> bool {
    if CLIENT_BINARIES.contains(&binary) {
        return true; // available on all platforms
    }
    if SERVER_BINARIES.contains(&binary) {
        return platform.os == Os::Linux;
    }
    false
}

/// Build the GitHub Release download URL for a binary + platform.
pub fn download_url(repo: &str, binary: &str, platform: &Platform) -> String {
    let ext = platform.ext();
    format!(
        "https://github.com/{repo}/releases/latest/download/{binary}-{}-{}.{ext}",
        platform.os, platform.arch
    )
}

/// List all known binary names.
pub fn all_binaries() -> Vec<&'static str> {
    let mut bins: Vec<&str> = Vec::new();
    bins.extend_from_slice(CLIENT_BINARIES);
    bins.extend_from_slice(SERVER_BINARIES);
    bins
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Arch, Os, Platform};

    #[test]
    fn valid_binaries() {
        assert!(is_valid_binary("betcode"));
        assert!(is_valid_binary("betcode-daemon"));
        assert!(is_valid_binary("betcode-relay"));
        assert!(is_valid_binary("betcode-setup"));
        assert!(!is_valid_binary("unknown"));
    }

    #[test]
    fn relay_not_available_on_macos() {
        let mac = Platform { os: Os::Darwin, arch: Arch::Arm64 };
        assert!(!is_available("betcode-relay", &mac));
        assert!(is_available("betcode", &mac));
    }

    #[test]
    fn relay_available_on_linux() {
        let linux = Platform { os: Os::Linux, arch: Arch::Amd64 };
        assert!(is_available("betcode-relay", &linux));
    }

    #[test]
    fn download_url_linux() {
        let p = Platform { os: Os::Linux, arch: Arch::Amd64 };
        let url = download_url("sakost/betcode", "betcode", &p);
        assert_eq!(
            url,
            "https://github.com/sakost/betcode/releases/latest/download/betcode-linux-amd64.tar.gz"
        );
    }

    #[test]
    fn download_url_windows() {
        let p = Platform { os: Os::Windows, arch: Arch::Amd64 };
        let url = download_url("sakost/betcode", "betcode", &p);
        assert_eq!(
            url,
            "https://github.com/sakost/betcode/releases/latest/download/betcode-windows-amd64.zip"
        );
    }
}
```

**Step 2: Add `mod registry;` to `main.rs`**

**Step 3: Run tests**

Run: `cargo test -p betcode-releases`
Expected: all 12 tests pass

**Step 4: Commit**

```bash
git add crates/betcode-releases/src/registry.rs crates/betcode-releases/src/main.rs
git commit -m "feat(releases): add binary registry and download URL builder"
```

---

## Task 4: HTTP Routes — Install Scripts & Redirects

**Files:**
- Create: `crates/betcode-releases/src/routes.rs`
- Create: `crates/betcode-releases/src/scripts/install.sh`
- Create: `crates/betcode-releases/src/scripts/install.ps1`
- Modify: `crates/betcode-releases/src/main.rs`

**Step 1: Create install.sh**

`crates/betcode-releases/src/scripts/install.sh`:

```sh
#!/bin/sh
set -eu

REPO="REPO_PLACEHOLDER"
BASE_URL="BASE_URL_PLACEHOLDER"
BINARY="${1:-betcode}"
INSTALL_DIR="${BETCODE_INSTALL_DIR:-/usr/local/bin}"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Linux*)  OS_SUFFIX="linux" ;;
  Darwin*) OS_SUFFIX="darwin" ;;
  *)       echo "Error: unsupported OS: $OS" >&2; exit 1 ;;
esac

# Detect arch
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)   ARCH_SUFFIX="amd64" ;;
  aarch64|arm64)   ARCH_SUFFIX="arm64" ;;
  *)               echo "Error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

# Server-only binaries
case "$BINARY" in
  betcode-relay|betcode-setup)
    if [ "$OS_SUFFIX" != "linux" ]; then
      echo "Error: $BINARY is only available on Linux" >&2; exit 1
    fi ;;
esac

PLATFORM="${OS_SUFFIX}-${ARCH_SUFFIX}"
URL="https://github.com/${REPO}/releases/latest/download/${BINARY}-${PLATFORM}.tar.gz"

echo "Installing ${BINARY} (${PLATFORM})..."

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "${TMP}/${BINARY}.tar.gz"
tar xzf "${TMP}/${BINARY}.tar.gz" -C "$TMP"

if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
else
  echo "Installing to ${INSTALL_DIR} (requires sudo)..."
  sudo mv "${TMP}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
fi

chmod +x "${INSTALL_DIR}/${BINARY}"
echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
"${INSTALL_DIR}/${BINARY}" --version 2>/dev/null || true
```

**Step 2: Create install.ps1**

`crates/betcode-releases/src/scripts/install.ps1`:

```powershell
#Requires -Version 5.1
param(
    [string]$Binary = "betcode",
    [string]$InstallDir = "$env:USERPROFILE\.betcode\bin"
)

$ErrorActionPreference = "Stop"
$Repo = "REPO_PLACEHOLDER"

# Only client binaries on Windows
if ($Binary -in @("betcode-relay", "betcode-setup")) {
    Write-Error "$Binary is only available on Linux"
    exit 1
}

$Arch = "amd64"
$Platform = "windows-$Arch"
$Url = "https://github.com/$Repo/releases/latest/download/$Binary-$Platform.zip"

Write-Host "Installing $Binary ($Platform)..."

$TmpDir = Join-Path $env:TEMP "betcode-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TmpDir "$Binary.zip"
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $ExePath = Join-Path $TmpDir "$Binary.exe"
    $Dest = Join-Path $InstallDir "$Binary.exe"
    Move-Item -Path $ExePath -Destination $Dest -Force

    # Add to PATH if not already there
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$UserPath;$InstallDir", "User")
        Write-Host "Added $InstallDir to PATH (restart terminal to use)"
    }

    Write-Host "Installed $Binary to $Dest"
} finally {
    Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
```

**Step 3: Create routes.rs**

```rust
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::platform;
use crate::registry;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub repo: String,
    pub base_url: String,
}

const INSTALL_SH: &str = include_str!("scripts/install.sh");
const INSTALL_PS1: &str = include_str!("scripts/install.ps1");

/// Returns true if the request looks like a browser (wants HTML).
fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

/// `GET /` — landing page for browsers, install.sh for CLI.
pub async fn root(headers: HeaderMap, State(state): State<AppState>) -> Response {
    if wants_html(&headers) {
        return Html(landing_page(&state)).into_response();
    }
    // CLI: serve install script with repo placeholder replaced
    let script = INSTALL_SH
        .replace("REPO_PLACEHOLDER", &state.repo)
        .replace("BASE_URL_PLACEHOLDER", &state.base_url);
    (
        StatusCode::OK,
        [("content-type", "text/x-shellscript")],
        script,
    )
        .into_response()
}

/// `GET /install.sh`
pub async fn install_sh(State(state): State<AppState>) -> impl IntoResponse {
    let script = INSTALL_SH
        .replace("REPO_PLACEHOLDER", &state.repo)
        .replace("BASE_URL_PLACEHOLDER", &state.base_url);
    ([("content-type", "text/x-shellscript")], script)
}

/// `GET /install.ps1`
pub async fn install_ps1(State(state): State<AppState>) -> impl IntoResponse {
    let script = INSTALL_PS1.replace("REPO_PLACEHOLDER", &state.repo);
    ([("content-type", "text/plain")], script)
}

/// `GET /:binary` — redirect CLI to GitHub, landing page for browser.
pub async fn binary_download(
    Path(binary): Path<String>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if !registry::is_valid_binary(&binary) {
        return StatusCode::NOT_FOUND.into_response();
    }

    if wants_html(&headers) {
        return Html(landing_page(&state)).into_response();
    }

    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let Some(platform) = platform::detect(ua) else {
        return (StatusCode::BAD_REQUEST, "Could not detect platform").into_response();
    };

    if !registry::is_available(&binary, &platform) {
        return (
            StatusCode::NOT_FOUND,
            format!("{binary} is not available on {}-{}", platform.os, platform.arch),
        )
            .into_response();
    }

    let url = registry::download_url(&state.repo, &binary, &platform);
    Redirect::temporary(&url).into_response()
}

/// Generate a styled HTML landing page.
fn landing_page(state: &AppState) -> String {
    let mut binary_rows = String::new();
    for bin in registry::all_binaries() {
        binary_rows.push_str(&format!(
            r#"<tr>
  <td><code>{bin}</code></td>
  <td><a href="https://github.com/{repo}/releases/latest/download/{bin}-linux-amd64.tar.gz">Linux x64</a></td>
  <td><a href="https://github.com/{repo}/releases/latest/download/{bin}-linux-arm64.tar.gz">Linux ARM64</a></td>
  <td>{darwin_amd64}</td>
  <td>{darwin_arm64}</td>
  <td>{windows}</td>
</tr>"#,
            repo = state.repo,
            darwin_amd64 = if registry::is_available(bin, &crate::platform::Platform {
                os: crate::platform::Os::Darwin,
                arch: crate::platform::Arch::Amd64,
            }) {
                format!(r#"<a href="https://github.com/{}/releases/latest/download/{bin}-darwin-amd64.tar.gz">macOS x64</a>"#, state.repo)
            } else {
                "—".to_string()
            },
            darwin_arm64 = if registry::is_available(bin, &crate::platform::Platform {
                os: crate::platform::Os::Darwin,
                arch: crate::platform::Arch::Arm64,
            }) {
                format!(r#"<a href="https://github.com/{}/releases/latest/download/{bin}-darwin-arm64.tar.gz">macOS ARM64</a>"#, state.repo)
            } else {
                "—".to_string()
            },
            windows = if registry::is_available(bin, &crate::platform::Platform {
                os: crate::platform::Os::Windows,
                arch: crate::platform::Arch::Amd64,
            }) {
                format!(r#"<a href="https://github.com/{}/releases/latest/download/{bin}-windows-amd64.zip">Windows x64</a>"#, state.repo)
            } else {
                "—".to_string()
            },
        ));
    }

    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BetCode — Download</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
         color: #e0e0e0; background: #0d1117; line-height: 1.6; }}
  .container {{ max-width: 800px; margin: 0 auto; padding: 2rem 1rem; }}
  h1 {{ font-size: 2rem; margin-bottom: 0.5rem; color: #f0f0f0; }}
  h2 {{ font-size: 1.2rem; margin: 2rem 0 0.75rem; color: #c0c0c0; }}
  p {{ margin-bottom: 1rem; color: #a0a0a0; }}
  code {{ background: #161b22; padding: 0.15em 0.4em; border-radius: 4px; font-size: 0.9em; }}
  pre {{ background: #161b22; padding: 1rem; border-radius: 8px; overflow-x: auto;
         margin-bottom: 1.5rem; border: 1px solid #30363d; }}
  pre code {{ background: none; padding: 0; }}
  table {{ width: 100%; border-collapse: collapse; margin-bottom: 1.5rem; }}
  th, td {{ padding: 0.5rem 0.75rem; text-align: left; border-bottom: 1px solid #21262d; }}
  th {{ color: #8b949e; font-weight: 600; font-size: 0.85em; text-transform: uppercase; }}
  a {{ color: #58a6ff; text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  .hero {{ text-align: center; padding: 2rem 0; }}
  .hero p {{ font-size: 1.1rem; }}
  .install-box {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px;
                  padding: 1.5rem; margin: 1.5rem 0; }}
</style>
</head>
<body>
<div class="container">
  <div class="hero">
    <h1>BetCode</h1>
    <p>Install the CLI tools for your platform</p>
  </div>

  <div class="install-box">
    <h2>Quick Install</h2>
    <p><strong>Linux / macOS:</strong></p>
    <pre><code>curl -fsSL https://{base_url} | sh</code></pre>
    <p><strong>Windows (PowerShell):</strong></p>
    <pre><code>irm https://{base_url}/install.ps1 | iex</code></pre>
  </div>

  <h2>Manual Downloads</h2>
  <table>
    <thead>
      <tr><th>Binary</th><th>Linux x64</th><th>Linux ARM64</th><th>macOS x64</th><th>macOS ARM64</th><th>Windows x64</th></tr>
    </thead>
    <tbody>
      {binary_rows}
    </tbody>
  </table>

  <h2>Install a Specific Binary</h2>
  <pre><code># Install relay (Linux only)
curl -fsSL https://{base_url} | sh -s betcode-relay

# Install daemon
curl -fsSL https://{base_url} | sh -s betcode-daemon</code></pre>
</div>
</body>
</html>"#,
        base_url = state.base_url,
    )
}
```

**Step 4: Wire routes into main.rs**

```rust
use std::net::SocketAddr;

use clap::Parser;
use tracing::info;

mod platform;
mod registry;
mod routes;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8080", env = "LISTEN_ADDR")]
    addr: SocketAddr,

    #[arg(long, default_value = "sakost/betcode", env = "GITHUB_REPO")]
    repo: String,

    #[arg(long, default_value = "get.betcode.dev", env = "BASE_URL")]
    base_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let state = routes::AppState {
        repo: args.repo,
        base_url: args.base_url,
    };

    let app = axum::Router::new()
        .route("/", axum::routing::get(routes::root))
        .route("/install.sh", axum::routing::get(routes::install_sh))
        .route("/install.ps1", axum::routing::get(routes::install_ps1))
        .route("/{binary}", axum::routing::get(routes::binary_download))
        .with_state(state);

    info!(addr = %args.addr, "starting betcode-releases");
    let listener = tokio::net::TcpListener::bind(args.addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

**Step 5: Run clippy and tests**

Run: `cargo clippy -p betcode-releases -- -D warnings && cargo test -p betcode-releases`
Expected: clean build, all tests pass

**Step 6: Commit**

```bash
git add crates/betcode-releases/
git commit -m "feat(releases): add HTTP routes, install scripts, and landing page"
```

---

## Task 5: GitHub Actions Release Workflow

**Files:**
- Create: `.github/workflows/release.yml`

**Step 1: Create the workflow**

`.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          # Linux AMD64
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
            suffix: linux-amd64
            bins: "betcode betcode-daemon betcode-relay betcode-setup betcode-releases"
          # Linux ARM64
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-latest
            suffix: linux-arm64
            bins: "betcode betcode-daemon betcode-relay betcode-setup betcode-releases"
            cross: true
          # macOS AMD64
          - target: x86_64-apple-darwin
            os: macos-13
            suffix: darwin-amd64
            bins: "betcode betcode-daemon"
          # macOS ARM64
          - target: aarch64-apple-darwin
            os: macos-latest
            suffix: darwin-arm64
            bins: "betcode betcode-daemon"
          # Windows AMD64
          - target: x86_64-pc-windows-msvc
            os: windows-latest
            suffix: windows-amd64
            bins: "betcode betcode-daemon"

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: true

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.target }}

      # Install cross for Linux ARM64
      - name: Install cross
        if: matrix.cross
        run: cargo install cross

      # Install protoc
      - name: Install protoc (Linux)
        if: runner.os == 'Linux'
        run: sudo apt-get update && sudo apt-get install -y protobuf-compiler

      - name: Install protoc (macOS)
        if: runner.os == 'macOS'
        run: brew install protobuf

      - name: Install protoc (Windows)
        if: runner.os == 'Windows'
        run: choco install protoc

      # Build
      - name: Build (cross)
        if: matrix.cross
        run: cross build --release --target ${{ matrix.target }}

      - name: Build (native)
        if: "!matrix.cross"
        run: cargo build --release --target ${{ matrix.target }}

      # Package
      - name: Package (Unix)
        if: runner.os != 'Windows'
        shell: bash
        run: |
          cd target/${{ matrix.target }}/release
          for bin in ${{ matrix.bins }}; do
            if [ -f "$bin" ]; then
              tar czf "${bin}-${{ matrix.suffix }}.tar.gz" "$bin"
              echo "Packaged: ${bin}-${{ matrix.suffix }}.tar.gz"
            fi
          done

      - name: Package (Windows)
        if: runner.os == 'Windows'
        shell: pwsh
        run: |
          cd target/${{ matrix.target }}/release
          foreach ($bin in "${{ matrix.bins }}".Split(" ")) {
            if (Test-Path "$bin.exe") {
              Compress-Archive -Path "$bin.exe" -DestinationPath "$bin-${{ matrix.suffix }}.zip"
              Write-Host "Packaged: $bin-${{ matrix.suffix }}.zip"
            }
          }

      - uses: actions/upload-artifact@v4
        with:
          name: release-${{ matrix.suffix }}
          path: |
            target/${{ matrix.target }}/release/*.tar.gz
            target/${{ matrix.target }}/release/*.zip

  release:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          pattern: release-*
          merge-multiple: true

      - name: List assets
        run: ls -la *.tar.gz *.zip 2>/dev/null || true

      - uses: softprops/action-gh-release@v2
        with:
          files: |
            *.tar.gz
            *.zip
          generate_release_notes: true
```

**Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"` (or just check it looks right)

**Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add cross-platform release workflow for tagged builds"
```

---

## Task 6: Deploy `betcode-releases` via `betcode-setup`

**Files:**
- Modify: `crates/betcode-setup/src/relay/templates.rs` (add releases unit template)
- Modify: `crates/betcode-setup/src/relay/systemd.rs` (add releases deploy)
- Modify: `crates/betcode-setup/src/relay/mod.rs` (add `--releases-domain` arg)

This task adds a `--releases-domain` flag to `betcode-setup relay` that also deploys the `betcode-releases` service on the same VPS. It runs on port 8090 behind Caddy/nginx reverse proxy, or directly with its own certbot cert.

**Step 1: Add systemd unit template for releases**

In `templates.rs`, add:

```rust
pub fn releases_systemd_unit(domain: &str, repo: &str) -> String {
    format!(
        r"[Unit]
Description=BetCode Release Download Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=betcode
Group=betcode
ExecStart=/usr/local/bin/betcode-releases \
  --addr 0.0.0.0:8090 \
  --repo {repo} \
  --base-url {domain}
Restart=on-failure
RestartSec=5

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
"
    )
}
```

**Step 2: Add deploy function in systemd.rs**

Add a `deploy_releases` function that:
1. Writes the systemd unit to `/etc/systemd/system/betcode-releases.service`
2. Installs the `betcode-releases` binary (same pattern as relay)
3. Sets up certbot for the releases domain (reuse existing `setup_certbot` pattern)
4. Enables and starts the service

**Step 3: Wire into relay setup**

Add `--releases-domain` optional arg to `RelayArgs`. When provided, also deploy the releases service.

**Step 4: Test and commit**

Run: `cargo clippy -p betcode-setup -- -D warnings && cargo test -p betcode-setup --lib`

```bash
git add crates/betcode-setup/
git commit -m "feat(setup): add betcode-releases deployment support"
```

---

## Task 7: Integration Test

**Files:**
- Create: `crates/betcode-releases/tests/integration.rs`

**Step 1: Write integration test**

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn app() -> axum::Router {
    // Import the same router setup from main, or extract it
    todo!("extract router builder from main.rs into a pub fn")
}

#[tokio::test]
async fn root_browser_returns_html() {
    let app = app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Check content-type is HTML
}

#[tokio::test]
async fn root_curl_returns_shell_script() {
    let app = app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header("user-agent", "curl/8.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Check body contains "#!/bin/sh"
}

#[tokio::test]
async fn binary_redirect_linux() {
    let app = app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/betcode")
                .header("user-agent", "curl/8.0 (x86_64-linux-gnu)")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.contains("betcode-linux-amd64.tar.gz"));
}

#[tokio::test]
async fn unknown_binary_returns_404() {
    let app = app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/unknown-binary")
                .header("user-agent", "curl/8.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn relay_on_macos_returns_404() {
    let app = app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/betcode-relay")
                .header("user-agent", "curl/8.0 (aarch64-apple-darwin)")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

To make this work, extract the router construction from `main.rs` into a `pub fn build_router(state: AppState) -> Router` that both `main` and tests can use.

**Step 2: Run tests**

Run: `cargo test -p betcode-releases`
Expected: all tests pass

**Step 3: Commit**

```bash
git add crates/betcode-releases/
git commit -m "test(releases): add integration tests for routing and redirects"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Scaffold crate | `Cargo.toml`, `main.rs` |
| 2 | Platform detection | `platform.rs` (7 tests) |
| 3 | Binary registry | `registry.rs` (5 tests) |
| 4 | HTTP routes + install scripts + landing page | `routes.rs`, `install.sh`, `install.ps1` |
| 5 | CI release workflow | `release.yml` |
| 6 | Setup deployment | `betcode-setup` changes |
| 7 | Integration tests | `tests/integration.rs` |

**End-to-end flow:**
1. `git tag v0.1.0 && git push --tags` → CI builds 5 targets, uploads to GitHub Releases
2. User runs `curl -fsSL https://get.betcode.dev | sh` → gets install.sh → downloads correct binary
3. User opens `https://get.betcode.dev` in browser → sees styled download page
4. `curl -LO https://get.betcode.dev/betcode` → 302 redirect to correct GitHub asset
