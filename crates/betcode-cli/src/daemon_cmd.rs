//! CLI subcommands for daemon management.
//!
//! Provides `betcode daemon rotate-cert` for forcing immediate certificate
//! rotation.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

/// Daemon management subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum DaemonAction {
    /// Force immediate mTLS certificate rotation.
    ///
    /// Regenerates the client certificate and private key, writing new files
    /// to `$HOME/.betcode/certs/`. The daemon will use the new certificate
    /// on its next relay connection.
    RotateCert {
        /// Path to the certificate directory (default: `$HOME/.betcode/certs/`)
        #[arg(long)]
        certs_dir: Option<PathBuf>,

        /// Machine ID override (uses existing metadata if available)
        #[arg(long)]
        machine_id: Option<String>,
    },
}

/// Default cert directory name under home.
const CERTS_SUBDIR: &str = ".betcode/certs";
/// Metadata filename (must match `betcode-setup`'s `cert_provisioning`).
const METADATA_FILENAME: &str = "cert-metadata.json";
/// Default validity period for rotated certificates (365 days).
const DEFAULT_VALIDITY_DAYS: u64 = 365;

/// Certificate metadata (same format as betcode-setup and betcode-daemon).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CertMetadata {
    machine_id: String,
    generated_at_secs: u64,
    validity_days: u64,
}

/// Resolve the default certificate directory.
fn default_certs_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(CERTS_SUBDIR))
}

/// Read certificate metadata from the certs directory.
fn read_metadata(certs_dir: &Path) -> Option<CertMetadata> {
    let path = certs_dir.join(METADATA_FILENAME);
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write certificate metadata JSON file.
fn write_metadata(certs_dir: &Path, metadata: &CertMetadata) -> Result<()> {
    let path = certs_dir.join(METADATA_FILENAME);
    let json = serde_json::to_string_pretty(metadata)
        .context("Failed to serialize cert metadata to JSON")?;
    fs::write(&path, json)
        .with_context(|| format!("Failed to write cert metadata: {}", path.display()))
}

/// Execute the `daemon` subcommand.
pub fn run(action: DaemonAction) -> Result<()> {
    match action {
        DaemonAction::RotateCert {
            certs_dir,
            machine_id,
        } => run_rotate_cert(certs_dir, machine_id),
    }
}

/// Force-rotate the mTLS client certificate.
#[allow(clippy::print_stdout, clippy::print_stderr)]
fn run_rotate_cert(
    certs_dir_override: Option<PathBuf>,
    machine_id_override: Option<String>,
) -> Result<()> {
    let certs_dir = match certs_dir_override {
        Some(dir) => dir,
        None => default_certs_dir()?,
    };

    if !certs_dir.exists() {
        anyhow::bail!(
            "Certificate directory {} does not exist. Run `betcode-setup daemon` first.",
            certs_dir.display()
        );
    }

    let machine_id = machine_id_override
        .or_else(|| read_metadata(&certs_dir).map(|m| m.machine_id))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No machine ID found. Provide --machine-id or run `betcode-setup daemon` first."
            )
        })?;

    println!("Rotating certificate for machine: {machine_id}");
    println!("  Certs dir: {}", certs_dir.display());

    let ca = betcode_crypto::certs::generate_ca("BetCode")
        .map_err(|e| anyhow::anyhow!("CA generation failed: {e}"))?;

    let bundle = betcode_crypto::certs::generate_client_cert(&ca, &machine_id)
        .map_err(|e| anyhow::anyhow!("Client cert generation failed: {e}"))?;

    fs::write(certs_dir.join("client.pem"), &bundle.cert_pem)
        .context("Failed to write client certificate")?;
    fs::write(certs_dir.join("client-key.pem"), &bundle.key_pem)
        .context("Failed to write client key")?;
    fs::write(certs_dir.join("ca.pem"), &bundle.ca_cert_pem)
        .context("Failed to write CA certificate")?;

    // Restrict key file permissions on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(certs_dir.join("client-key.pem"), perms)
            .context("Failed to set key file permissions")?;
    }

    let metadata = CertMetadata {
        machine_id,
        generated_at_secs: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        validity_days: DEFAULT_VALIDITY_DAYS,
    };
    write_metadata(&certs_dir, &metadata)?;

    println!();
    println!("Certificate rotated successfully.");
    println!("  Client cert: {}", certs_dir.join("client.pem").display());
    println!(
        "  Client key:  {}",
        certs_dir.join("client-key.pem").display()
    );
    println!("  CA cert:     {}", certs_dir.join("ca.pem").display());
    println!();
    println!("The daemon will use the new certificate on its next relay connection.");
    println!("Restart the daemon to apply immediately:");
    println!("  systemctl --user restart betcode-daemon");

    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn rotate_cert_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");
        fs::create_dir_all(&certs_dir).unwrap();

        // Write initial metadata so machine_id can be discovered
        let meta = CertMetadata {
            machine_id: "cli-test-machine".to_string(),
            generated_at_secs: 1000,
            validity_days: 365,
        };
        write_metadata(&certs_dir, &meta).unwrap();

        run_rotate_cert(Some(certs_dir.clone()), None).unwrap();

        assert!(certs_dir.join("client.pem").exists());
        assert!(certs_dir.join("client-key.pem").exists());
        assert!(certs_dir.join("ca.pem").exists());

        // Metadata should be updated
        let new_meta = read_metadata(&certs_dir).unwrap();
        assert_eq!(new_meta.machine_id, "cli-test-machine");
        assert!(new_meta.generated_at_secs > meta.generated_at_secs);
    }

    #[test]
    fn rotate_cert_with_machine_id_override() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");
        fs::create_dir_all(&certs_dir).unwrap();

        run_rotate_cert(
            Some(certs_dir.clone()),
            Some("override-machine".to_string()),
        )
        .unwrap();

        let meta = read_metadata(&certs_dir).unwrap();
        assert_eq!(meta.machine_id, "override-machine");
    }

    #[test]
    fn rotate_cert_fails_for_missing_dir() {
        let result = run_rotate_cert(Some(PathBuf::from("/nonexistent/certs")), None);
        assert!(result.is_err());
    }

    #[test]
    fn rotate_cert_fails_without_machine_id() {
        let dir = tempfile::tempdir().unwrap();
        let certs_dir = dir.path().join("certs");
        fs::create_dir_all(&certs_dir).unwrap();
        // No metadata file, no --machine-id
        let result = run_rotate_cert(Some(certs_dir), None);
        assert!(result.is_err());
    }
}
