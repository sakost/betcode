use crate::config::RelaySetupConfig;

/// Generate the systemd unit file content.
pub fn systemd_unit(config: &RelaySetupConfig) -> String {
    let domain = &config.domain;
    let port = config.addr.port();

    let addr_flag = if port == 443 {
        String::new()
    } else {
        format!(" \\\n  --addr {}", config.addr)
    };

    let cap_lines = if port < 1024 {
        "AmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE"
    } else {
        ""
    };

    format!(
        r"[Unit]
Description=BetCode Relay Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=betcode
Group=betcode
EnvironmentFile=/etc/betcode/relay.env
ExecStart=/usr/local/bin/betcode-relay \
  --tls-cert /etc/letsencrypt/live/{domain}/fullchain.pem \
  --tls-key /etc/letsencrypt/live/{domain}/privkey.pem{addr_flag}
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/betcode
ReadOnlyPaths=/etc/betcode /etc/letsencrypt
PrivateTmp=true
{cap_lines}

[Install]
WantedBy=multi-user.target
"
    )
}

/// Generate the environment file content.
pub fn env_file(config: &RelaySetupConfig) -> String {
    format!(
        "BETCODE_JWT_SECRET={}\nBETCODE_DB_PATH={}\n",
        config.jwt_secret,
        config.db_path.display()
    )
}

/// Generate the certbot pre-hook script (stop relay before renewal).
pub const fn certbot_pre_hook() -> &'static str {
    "#!/bin/sh\nsystemctl stop betcode-relay\n"
}

/// Generate the certbot post-hook script (start relay after renewal).
pub const fn certbot_post_hook() -> &'static str {
    "#!/bin/sh\nsystemctl start betcode-relay\n"
}

/// Generate the Dockerfile content.
pub const fn dockerfile() -> &'static str {
    r#"FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin betcode-relay && strip target/release/betcode-relay

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /usr/sbin/nologin betcode
COPY --from=builder /src/target/release/betcode-relay /usr/local/bin/
USER betcode
EXPOSE 443
ENTRYPOINT ["betcode-relay"]
"#
}

/// Generate the docker-compose.yml content.
pub fn docker_compose(config: &RelaySetupConfig) -> String {
    let domain = &config.domain;
    let port = config.addr.port();
    format!(
        r#"services:
  certbot-init:
    image: certbot/certbot:latest
    volumes:
      - certs:/etc/letsencrypt
      - certbot-www:/var/www/certbot
    command: certonly --standalone -d {domain} --agree-tos --non-interactive --email admin@{domain}
    profiles:
      - init

  relay:
    build: .
    restart: unless-stopped
    ports:
      - "{port}:{port}"
    env_file:
      - .env
    volumes:
      - data:/var/lib/betcode
      - certs:/etc/letsencrypt:ro
    command:
      - --tls-cert
      - /etc/letsencrypt/live/{domain}/fullchain.pem
      - --tls-key
      - /etc/letsencrypt/live/{domain}/privkey.pem
    depends_on:
      certbot-init:
        condition: service_completed_successfully
        required: false

volumes:
  data:
  certs:
  certbot-www:
"#
    )
}

/// Generate the .env.example content.
pub fn env_example(config: &RelaySetupConfig) -> String {
    format!(
        r"# BetCode Relay environment
BETCODE_JWT_SECRET={}
BETCODE_DB_PATH={}
",
        config.jwt_secret,
        config.db_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DeploymentMode, RelaySetupConfig};
    use std::net::SocketAddr;
    use std::path::PathBuf;

    fn test_config(addr: SocketAddr) -> RelaySetupConfig {
        RelaySetupConfig {
            domain: "relay.example.com".into(),
            jwt_secret: "a".repeat(48),
            db_path: PathBuf::from("/var/lib/betcode/relay.db"),
            deployment_mode: DeploymentMode::Systemd,
            relay_binary_path: None,
            addr,
        }
    }

    #[test]
    fn systemd_unit_custom_port_contains_addr_flag() {
        let config = test_config("0.0.0.0:8443".parse().unwrap());
        let unit = systemd_unit(&config);
        assert!(
            unit.contains("--addr 0.0.0.0:8443"),
            "unit must contain --addr flag for non-default port"
        );
    }

    #[test]
    fn systemd_unit_default_port_omits_addr() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let unit = systemd_unit(&config);
        assert!(
            !unit.contains("--addr"),
            "default port should not emit --addr"
        );
    }

    #[test]
    fn systemd_unit_privileged_port_has_cap() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let unit = systemd_unit(&config);
        assert!(unit.contains("CAP_NET_BIND_SERVICE"));
    }

    #[test]
    fn systemd_unit_unprivileged_port_no_cap() {
        let config = test_config("0.0.0.0:8443".parse().unwrap());
        let unit = systemd_unit(&config);
        assert!(
            !unit.contains("CAP_NET_BIND_SERVICE"),
            "unprivileged port should not require CAP_NET_BIND_SERVICE"
        );
    }

    #[test]
    fn systemd_unit_contains_domain() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let unit = systemd_unit(&config);
        assert!(unit.contains("relay.example.com"));
    }

    #[test]
    fn env_file_contains_expected_keys() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let content = env_file(&config);
        assert!(content.contains("BETCODE_JWT_SECRET="));
        assert!(content.contains("BETCODE_DB_PATH="));
    }

    #[test]
    fn docker_compose_uses_configured_port() {
        let config = test_config("0.0.0.0:8443".parse().unwrap());
        let compose = docker_compose(&config);
        assert!(
            compose.contains("\"8443:8443\""),
            "compose must map configured port"
        );
    }

    #[test]
    fn docker_compose_default_port() {
        let config = test_config("0.0.0.0:443".parse().unwrap());
        let compose = docker_compose(&config);
        assert!(compose.contains("\"443:443\""));
    }
}
