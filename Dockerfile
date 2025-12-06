# Multi-stage Dockerfile for JIT Issue Tracker (Linux only)
# Builds CLI, API server, MCP server, and Web UI in one image

# Stage 1: Build Rust binaries
FROM rust:alpine as rust-builder

WORKDIR /build

# Install build dependencies
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static

# Copy Cargo workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Build all Rust binaries (CLI, server, dispatch)
RUN cargo build --release --workspace

# Strip binaries to reduce size
RUN strip target/release/jit && \
    strip target/release/jit-server && \
    strip target/release/jit-dispatch

# Stage 2: Build Web UI
FROM node:20-slim as web-builder

WORKDIR /build

# Copy web UI files
COPY web/package*.json ./
RUN npm ci

COPY web/ ./
RUN npm run build

# Stage 3: Runtime image
FROM node:20-slim

LABEL org.opencontainers.image.title="JIT Issue Tracker"
LABEL org.opencontainers.image.description="CLI-first issue tracker for AI agents"
LABEL org.opencontainers.image.source="https://github.com/vkaskivuo/just-in-time"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y nginx ripgrep curl && \
    rm -rf /var/lib/apt/lists/*

# Copy Rust binaries from builder
COPY --from=rust-builder /build/target/release/jit /usr/local/bin/
COPY --from=rust-builder /build/target/release/jit-server /usr/local/bin/
COPY --from=rust-builder /build/target/release/jit-dispatch /usr/local/bin/

# Copy MCP server
COPY mcp-server/package*.json ./mcp-server/
RUN cd mcp-server && npm ci --production

COPY mcp-server/ ./mcp-server/

# Copy Web UI build
COPY --from=web-builder /build/dist /var/www/html

# Copy nginx configuration
COPY docker/nginx.conf /etc/nginx/nginx.conf

# Create data directory
RUN mkdir -p /data && chmod 777 /data

# Environment variables
ENV JIT_DATA_DIR=/data
ENV NODE_PATH=/app/mcp-server/node_modules

# Expose ports
EXPOSE 3000 80

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:3000/api/health || exit 1

# Start script
COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
CMD ["all"]
