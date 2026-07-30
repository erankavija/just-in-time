# syntax=docker/dockerfile:1

FROM node:22-alpine AS web-builder

WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.97-bookworm AS server-builder

RUN apt-get update \
  && apt-get install -y --no-install-recommends libssl-dev pkg-config \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY profiles/ ./profiles/
COPY scripts/ ./scripts/
COPY --from=web-builder /build/web/dist ./web/dist/
RUN cargo build --locked --release -p jit-server --bin jit-server

FROM debian:bookworm-slim

LABEL org.opencontainers.image.title="JIT Issue Tracker"
LABEL org.opencontainers.image.description="Repository-mounted JIT HTTP server and Web UI"
LABEL org.opencontainers.image.source="https://github.com/erankavija/just-in-time"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates ripgrep wget \
  && printf '%s\n' '--hidden' > /etc/jit-ripgreprc \
  && rm -rf /var/lib/apt/lists/*
COPY --from=server-builder /build/target/release/jit-server /usr/local/bin/jit-server

ENV RIPGREP_CONFIG_PATH=/etc/jit-ripgreprc

WORKDIR /repo
EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD ["wget", "-q", "-O", "/dev/null", "http://127.0.0.1:3000/api/health"]

USER 10001:10001
ENTRYPOINT ["/bin/sh", "-eu", "-c", "test -d /repo || { echo 'error: /repo must be a mounted repository directory' >&2; exit 78; }; test -r /repo && test -w /repo && test -x /repo || { echo 'error: /repo must be readable, writable, and searchable by the container identity' >&2; exit 78; }; test -d /repo/.jit || { echo 'error: /repo/.jit must exist; initialize the repository on the host' >&2; exit 78; }; test -r /repo/.jit && test -w /repo/.jit && test -x /repo/.jit || { echo 'error: /repo/.jit must be readable, writable, and searchable by the container identity' >&2; exit 78; }; exec /usr/local/bin/jit-server --data-dir /repo/.jit \"$@\"", "jit-server"]
