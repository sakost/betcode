use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result, bail};

/// Parse `/etc/os-release` into key-value pairs.
fn parse_os_release() -> Result<HashMap<String, String>> {
    let content =
        fs::read_to_string("/etc/os-release").context("failed to read /etc/os-release")?;

    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim_matches('"');
            map.insert(key.to_string(), value.to_string());
        }
    }
    Ok(map)
}

/// Ensure the current OS is Ubuntu. Bails with a clear message otherwise.
pub fn ensure_ubuntu() -> Result<()> {
    let release = parse_os_release()?;
    let id = release.get("ID").map_or("unknown", String::as_str);
    if id != "ubuntu" {
        bail!(
            "betcode-setup currently only supports Ubuntu (detected OS: {id}). \
             Contributions for other distributions are welcome!"
        );
    }
    let version = release.get("VERSION_ID").map_or("unknown", String::as_str);
    tracing::info!("detected Ubuntu ({version})");
    Ok(())
}
