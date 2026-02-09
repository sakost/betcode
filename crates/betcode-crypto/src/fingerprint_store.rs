//! Trust-on-first-use (TOFU) fingerprint store.
//!
//! Persists verified daemon fingerprints so subsequent connections can
//! detect man-in-the-middle attacks by comparing against previously
//! seen fingerprints.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CryptoError;
use crate::exchange::constant_time_str_eq;

/// A stored fingerprint entry for a known daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDaemon {
    /// The machine ID of the daemon.
    pub machine_id: String,
    /// Hex colon-separated fingerprint of the daemon's identity public key.
    pub fingerprint: String,
    /// When this fingerprint was first seen (Unix timestamp).
    pub first_seen: i64,
    /// When this fingerprint was last verified (Unix timestamp).
    pub last_seen: i64,
    /// Whether the user has explicitly verified this fingerprint.
    pub verified: bool,
}

/// Persistent store of known daemon fingerprints.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FingerprintStore {
    /// Map from machine_id to known daemon entry.
    pub daemons: HashMap<String, KnownDaemon>,
}

/// Result of checking a daemon's fingerprint against the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FingerprintCheck {
    /// First time seeing this daemon — TOFU accepted.
    TrustOnFirstUse,
    /// Fingerprint matches a previously seen daemon.
    Matched,
    /// Fingerprint does NOT match — possible MITM attack.
    Mismatch { expected: String, actual: String },
}

impl FingerprintStore {
    /// Load the store from a JSON file. Returns default if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, CryptoError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path)?;
        serde_json::from_str(&data).map_err(|e| {
            CryptoError::SerializationError(format!("Failed to parse fingerprint store: {}", e))
        })
    }

    /// Save the store to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), CryptoError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            CryptoError::SerializationError(format!("Failed to serialize fingerprint store: {}", e))
        })?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Check a daemon's fingerprint against the store.
    ///
    /// Uses constant-time comparison to prevent timing side-channel attacks.
    pub fn check(&self, machine_id: &str, fingerprint: &str) -> FingerprintCheck {
        match self.daemons.get(machine_id) {
            None => FingerprintCheck::TrustOnFirstUse,
            Some(known) if constant_time_str_eq(&known.fingerprint, fingerprint) => {
                FingerprintCheck::Matched
            }
            Some(known) => FingerprintCheck::Mismatch {
                expected: known.fingerprint.clone(),
                actual: fingerprint.to_string(),
            },
        }
    }

    /// Record a daemon fingerprint (TOFU or update last_seen).
    pub fn record(&mut self, machine_id: &str, fingerprint: &str, now: i64) {
        let entry = self
            .daemons
            .entry(machine_id.to_string())
            .or_insert_with(|| KnownDaemon {
                machine_id: machine_id.to_string(),
                fingerprint: fingerprint.to_string(),
                first_seen: now,
                last_seen: now,
                verified: false,
            });
        entry.last_seen = now;
    }

    /// Mark a daemon's fingerprint as explicitly verified by the user.
    pub fn mark_verified(&mut self, machine_id: &str) {
        if let Some(entry) = self.daemons.get_mut(machine_id) {
            entry.verified = true;
        }
    }

    /// Update a daemon's fingerprint (after user confirms the change).
    pub fn update_fingerprint(&mut self, machine_id: &str, new_fingerprint: &str, now: i64) {
        if let Some(entry) = self.daemons.get_mut(machine_id) {
            entry.fingerprint = new_fingerprint.to_string();
            entry.first_seen = now;
            entry.last_seen = now;
            entry.verified = false;
        }
    }

    /// Remove a daemon from the store.
    pub fn remove(&mut self, machine_id: &str) -> bool {
        self.daemons.remove(machine_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_returns_tofu() {
        let store = FingerprintStore::default();
        assert_eq!(
            store.check("m1", "aa:bb:cc"),
            FingerprintCheck::TrustOnFirstUse
        );
    }

    #[test]
    fn record_and_match() {
        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        assert_eq!(store.check("m1", "aa:bb:cc"), FingerprintCheck::Matched);
    }

    #[test]
    fn mismatch_detected() {
        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        assert_eq!(
            store.check("m1", "dd:ee:ff"),
            FingerprintCheck::Mismatch {
                expected: "aa:bb:cc".into(),
                actual: "dd:ee:ff".into(),
            }
        );
    }

    #[test]
    fn mark_verified_sets_flag() {
        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        assert!(!store.daemons["m1"].verified);
        store.mark_verified("m1");
        assert!(store.daemons["m1"].verified);
    }

    #[test]
    fn update_fingerprint_resets_verified() {
        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        store.mark_verified("m1");
        store.update_fingerprint("m1", "dd:ee:ff", 2000);
        assert_eq!(store.daemons["m1"].fingerprint, "dd:ee:ff");
        assert!(!store.daemons["m1"].verified);
        assert_eq!(store.daemons["m1"].first_seen, 2000);
    }

    #[test]
    fn remove_daemon() {
        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        assert!(store.remove("m1"));
        assert!(!store.remove("m1"));
        assert_eq!(
            store.check("m1", "aa:bb:cc"),
            FingerprintCheck::TrustOnFirstUse
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("betcode-fp-test-{}", rand::random::<u64>()));
        let path = dir.join("known_daemons.json");

        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        store.record("m2", "dd:ee:ff", 2000);
        store.mark_verified("m1");
        store.save(&path).unwrap();

        let loaded = FingerprintStore::load(&path).unwrap();
        assert_eq!(loaded.daemons.len(), 2);
        assert!(loaded.daemons["m1"].verified);
        assert!(!loaded.daemons["m2"].verified);
        assert_eq!(loaded.daemons["m1"].fingerprint, "aa:bb:cc");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let store = FingerprintStore::load(Path::new("/nonexistent/path.json")).unwrap();
        assert!(store.daemons.is_empty());
    }

    #[test]
    fn record_updates_last_seen() {
        let mut store = FingerprintStore::default();
        store.record("m1", "aa:bb:cc", 1000);
        assert_eq!(store.daemons["m1"].last_seen, 1000);
        store.record("m1", "aa:bb:cc", 2000);
        assert_eq!(store.daemons["m1"].last_seen, 2000);
        assert_eq!(store.daemons["m1"].first_seen, 1000);
    }

    #[test]
    fn load_corrupted_json_returns_error() {
        let dir = std::env::temp_dir().join(format!("betcode-fp-test-{}", rand::random::<u64>()));
        let path = dir.join("known_daemons.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{ not valid json !!!").unwrap();

        let result = FingerprintStore::load(&path);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CryptoError::SerializationError(_)
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mark_verified_noop_for_unknown_machine() {
        let mut store = FingerprintStore::default();
        // Should not panic or error for unknown machine_id
        store.mark_verified("nonexistent");
        assert!(store.daemons.is_empty());
    }

    #[test]
    fn update_fingerprint_noop_for_unknown_machine() {
        let mut store = FingerprintStore::default();
        // Should not panic or error for unknown machine_id
        store.update_fingerprint("nonexistent", "aa:bb:cc", 1000);
        assert!(store.daemons.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn save_to_readonly_dir_fails() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("betcode-fp-ro-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        // Make directory read-only
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o444)).unwrap();

        let path = dir.join("subdir").join("known_daemons.json");
        let store = FingerprintStore::default();
        let result = store.save(&path);
        assert!(result.is_err());

        // Restore permissions for cleanup
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
