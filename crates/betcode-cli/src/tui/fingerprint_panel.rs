//! Fingerprint verification panel for the TUI.
//!
//! Displays the daemon's fingerprint with randomart visualization when
//! connecting via relay. Supports TOFU (trust on first use) and explicit
//! verification flows.

use betcode_crypto::{FingerprintCheck, fingerprint_randomart, format_fingerprint_display};

/// State for the fingerprint verification prompt.
#[derive(Debug, Clone)]
pub struct FingerprintPrompt {
    /// Machine ID of the daemon.
    pub machine_id: String,
    /// The daemon's fingerprint (hex colon-separated).
    pub daemon_fingerprint: String,
    /// Randomart visualization of the fingerprint.
    pub randomart: String,
    /// Human-readable fingerprint display (grouped lines).
    pub fingerprint_display: String,
    /// The result of the TOFU check.
    pub check: FingerprintCheck,
    /// The user's decision (None = pending).
    pub decision: Option<FingerprintDecision>,
}

/// User's decision on fingerprint verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintDecision {
    /// Accept the fingerprint (TOFU or explicit trust).
    Accept,
    /// Reject the fingerprint and disconnect.
    Reject,
}

impl FingerprintPrompt {
    /// Create a new fingerprint prompt.
    pub fn new(machine_id: &str, daemon_fingerprint: &str, check: FingerprintCheck) -> Self {
        let randomart = fingerprint_randomart(daemon_fingerprint, machine_id);
        let fingerprint_display = format_fingerprint_display(daemon_fingerprint);
        Self {
            machine_id: machine_id.to_string(),
            daemon_fingerprint: daemon_fingerprint.to_string(),
            randomart,
            fingerprint_display,
            check,
            decision: None,
        }
    }

    /// Get the header text based on the check result.
    pub const fn header_text(&self) -> &'static str {
        match &self.check {
            FingerprintCheck::TrustOnFirstUse => "New daemon — verify the fingerprint if possible",
            FingerprintCheck::Matched => "Daemon fingerprint verified (matches known key)",
            FingerprintCheck::Mismatch { .. } => {
                "WARNING: Daemon fingerprint has CHANGED — possible MITM attack!"
            }
        }
    }

    /// Whether this prompt requires user action (mismatch needs explicit accept/reject).
    pub const fn needs_action(&self) -> bool {
        matches!(self.check, FingerprintCheck::Mismatch { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tofu_prompt_creation() {
        let prompt = FingerprintPrompt::new(
            "my-machine",
            "aa:bb:cc:dd:ee:ff:11:22:33:44:55:66:77:88:99:00:ab:cd:ef:12:34:56:78:9a:bc:de:f0:12:34:56:78:9a",
            FingerprintCheck::TrustOnFirstUse,
        );
        assert_eq!(prompt.machine_id, "my-machine");
        assert!(!prompt.randomart.is_empty());
        assert!(prompt.randomart.contains("my-machine"));
        assert!(!prompt.fingerprint_display.is_empty());
        assert!(!prompt.needs_action());
        assert!(prompt.decision.is_none());
    }

    #[test]
    fn matched_prompt_does_not_need_action() {
        let prompt = FingerprintPrompt::new("m1", "aa:bb", FingerprintCheck::Matched);
        assert!(!prompt.needs_action());
        assert!(prompt.header_text().contains("verified"));
    }

    #[test]
    fn mismatch_prompt_needs_action() {
        let prompt = FingerprintPrompt::new(
            "m1",
            "dd:ee",
            FingerprintCheck::Mismatch {
                expected: "aa:bb".into(),
                actual: "dd:ee".into(),
            },
        );
        assert!(prompt.needs_action());
        assert!(prompt.header_text().contains("WARNING"));
        assert!(prompt.header_text().contains("CHANGED"));
    }

    #[test]
    fn decision_initially_none() {
        let mut prompt = FingerprintPrompt::new("m1", "aa:bb", FingerprintCheck::TrustOnFirstUse);
        assert!(prompt.decision.is_none());
        prompt.decision = Some(FingerprintDecision::Accept);
        assert_eq!(prompt.decision, Some(FingerprintDecision::Accept));
    }
}
