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
    pub const fn ext(&self) -> &'static str {
        match self.os {
            Os::Windows => "zip",
            _ => "tar.gz",
        }
    }
}

/// Parse OS and architecture from a User-Agent header.
/// Returns `None` if detection fails.
#[allow(clippy::unnecessary_wraps)] // API returns Option for future extensibility
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
