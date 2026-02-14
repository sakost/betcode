/// Generate the systemd unit file for the betcode-releases download server.
///
/// When `localhost_only` is true (Caddy is fronting), the service binds to
/// `127.0.0.1:8090`; otherwise it listens on all interfaces.
pub fn systemd_unit(domain: &str, repo: &str, localhost_only: bool) -> String {
    let addr = if localhost_only {
        "127.0.0.1:8090"
    } else {
        "0.0.0.0:8090"
    };

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
  --addr {addr} \
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

/// Generate a Caddy site block for reverse-proxying the releases server.
pub fn caddyfile_site(domain: &str, acme_email: Option<&str>) -> String {
    let email_block = acme_email.map_or_else(String::new, |email| {
        format!(
            r"
    tls {{
        email {email}
    }}
"
        )
    });

    format!(
        r"{domain} {{
    encode gzip{email_block}

    reverse_proxy localhost:8090

    log {{
        output file /var/log/caddy/betcode-releases.log
    }}
}}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_localhost_only() {
        let unit = systemd_unit("get.example.com", "sakost/betcode", true);
        assert!(
            unit.contains("--addr 127.0.0.1:8090"),
            "localhost_only should bind to 127.0.0.1"
        );
        assert!(unit.contains("get.example.com"));
        assert!(unit.contains("sakost/betcode"));
    }

    #[test]
    fn systemd_unit_all_interfaces() {
        let unit = systemd_unit("get.example.com", "sakost/betcode", false);
        assert!(
            unit.contains("--addr 0.0.0.0:8090"),
            "non-localhost should bind to 0.0.0.0"
        );
    }

    #[test]
    fn systemd_unit_custom_repo() {
        let unit = systemd_unit("dl.example.com", "myorg/myrepo", true);
        assert!(unit.contains("myorg/myrepo"));
        assert!(unit.contains("dl.example.com"));
    }

    #[test]
    fn caddyfile_without_email() {
        let site = caddyfile_site("get.example.com", None);
        assert!(site.contains("get.example.com {"));
        assert!(site.contains("reverse_proxy localhost:8090"));
        assert!(site.contains("encode gzip"));
        assert!(site.contains("/var/log/caddy/betcode-releases.log"));
        assert!(!site.contains("tls"));
    }

    #[test]
    fn caddyfile_with_email() {
        let site = caddyfile_site("get.example.com", Some("admin@example.com"));
        assert!(site.contains("get.example.com {"));
        assert!(site.contains("reverse_proxy localhost:8090"));
        assert!(site.contains("email admin@example.com"));
        assert!(site.contains("tls {"));
    }
}
