use crate::config::RelaySetupConfig;

/// Generate the systemd unit file content.
pub fn systemd_unit(config: &RelaySetupConfig) -> String {
    let domain = &config.domain;
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
  --tls-key /etc/letsencrypt/live/{domain}/privkey.pem
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/betcode
ReadOnlyPaths=/etc/betcode /etc/letsencrypt
PrivateTmp=true
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE

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
      - "443:443"
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
