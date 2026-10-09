# syntax=docker/dockerfile:1
FROM debian:bookworm-slim AS downloads
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# Digests come from the official rust-v0.161.0 GitHub release asset metadata.
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-musl; checksum=b1efb95097660d7f2e5a3887618a23f2ea1b0d548078bf92b0f7a5d229a0cef2 ;; \
      arm64) target=aarch64-unknown-linux-musl; checksum=5c90bf4be3c97fc62fcafb28ec2335ee318e6ecb332e1e8ee3d2c63814a5a26a ;; \
      *) printf '%s\n' 'Only linux/amd64 and linux/arm64 are supported.' >&2; exit 1 ;; \
    esac; \
    curl --fail --silent --show-error --location \
      "https://github.com/openai/codex/releases/download/rust-v0.161.0/codex-${target}.tar.gz" \
      --output /tmp/codex.tar.gz; \
    printf '%s  %s\n' "$checksum" /tmp/codex.tar.gz | sha256sum --check --status; \
    tar -xzf /tmp/codex.tar.gz -C /tmp; \
    install -m 0755 "/tmp/codex-${target}" /usr/local/bin/codex; \
    rm -f /tmp/codex.tar.gz

FROM rust:1.99.0-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release --bin quota-server

FROM oven/bun:1.3.14 AS web
WORKDIR /web
COPY web ./
RUN bun install --frozen-lockfile && bun run build

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=downloads /usr/local/bin/codex /usr/local/bin/codex
ENV HOME=/auth/home CODEX_HOME=/auth/codex CLAUDE_CONFIG_DIR=/auth/claude
USER 1000:1000
WORKDIR /tmp

FROM runtime AS server
COPY --from=build /build/target/release/quota-server /usr/local/bin/quota-server
COPY --from=web /web/dist /app/web
ENTRYPOINT ["/usr/local/bin/quota-server"]

FROM downloads AS claude-download
ARG TARGETARCH
# Digests come from the official 2.1.293 manifest.json (native GNU/Linux).
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) platform=linux-x64; checksum=8968405e26db478af44eabc4635ab5ca557057b702a54460a59c13e1b253e978 ;; \
      arm64) platform=linux-arm64; checksum=a43629e888f0a7d96c5e8de62abf44852433a7ff2481574688db3e5b6399491f ;; \
      *) exit 1 ;; \
    esac; \
    curl --fail --silent --show-error --location \
      "https://downloads.claude.ai/claude-code-releases/2.1.293/${platform}/claude" \
      --output /usr/local/bin/claude; \
    printf '%s  %s\n' "$checksum" /usr/local/bin/claude | sha256sum --check --status; \
    chmod 0755 /usr/local/bin/claude

FROM runtime AS auth
USER root
RUN apt-get update && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=claude-download /usr/local/bin/claude /usr/local/bin/claude
COPY --chmod=0755 deploy/auth-entrypoint.sh /usr/local/bin/quota-auth
COPY deploy/check-claude-status.py /usr/local/lib/check-claude-status.py
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/quota-auth"]
