FROM rust:1-bookworm AS builder
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
