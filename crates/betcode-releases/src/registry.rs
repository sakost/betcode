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
