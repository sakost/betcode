use std::fmt::Write;

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
            format!(
                "{binary} is not available on {}-{}",
                platform.os, platform.arch
            ),
        )
            .into_response();
    }

    let url = registry::download_url(&state.repo, &binary, &platform);
    Redirect::temporary(&url).into_response()
}

/// Build a download link cell, or an em-dash if the binary is not available.
fn platform_cell(repo: &str, bin: &str, os: platform::Os, arch: platform::Arch, label: &str) -> String {
    let p = platform::Platform { os, arch };
    if registry::is_available(bin, &p) {
        let ext = p.ext();
        format!(
            r#"<a href="https://github.com/{repo}/releases/latest/download/{bin}-{os}-{arch}.{ext}">{label}</a>"#,
        )
    } else {
        "\u{2014}".to_string()
    }
}

/// Build the `<tbody>` rows for the download table.
fn binary_table_rows(repo: &str) -> String {
    let mut rows = String::new();
    for bin in registry::all_binaries() {
        let _ = write!(
            rows,
            r#"<tr>
  <td><code>{bin}</code></td>
  <td><a href="https://github.com/{repo}/releases/latest/download/{bin}-linux-amd64.tar.gz">Linux x64</a></td>
  <td><a href="https://github.com/{repo}/releases/latest/download/{bin}-linux-arm64.tar.gz">Linux ARM64</a></td>
  <td>{darwin_amd64}</td>
  <td>{darwin_arm64}</td>
  <td>{windows}</td>
</tr>"#,
            darwin_amd64 = platform_cell(repo, bin, platform::Os::Darwin, platform::Arch::Amd64, "macOS x64"),
            darwin_arm64 = platform_cell(repo, bin, platform::Os::Darwin, platform::Arch::Arm64, "macOS ARM64"),
            windows = platform_cell(repo, bin, platform::Os::Windows, platform::Arch::Amd64, "Windows x64"),
        );
    }
    rows
}

/// Generate a styled HTML landing page.
fn landing_page(state: &AppState) -> String {
    let binary_rows = binary_table_rows(&state.repo);

    format!(
        r#"<!DOCTYPE html>
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
