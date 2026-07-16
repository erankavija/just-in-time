# Installation Guide

This guide covers the installation methods provided in this repository.

## Table of Contents

- [Pre-built Binaries](#pre-built-binaries)
- [Docker](#docker)
- [From Source](#from-source)
- [NPM (MCP Server)](#npm-mcp-server)
- [Optional Dependencies](#optional-dependencies)

---

## Pre-built Binaries

**Recommended for most users.** Static binaries with zero dependencies.

### Download Latest Release

```bash
# Download and extract
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz

# Install to system (requires sudo)
sudo mv jit jit-server /usr/local/bin/

# Verify installation
jit --version
jit version
jit-server --version
```

### Install to User Directory (No sudo)

```bash
# Extract to ~/.local/bin
mkdir -p ~/.local/bin
tar -xzf jit-linux-x64.tar.gz -C ~/.local/bin

# Add to PATH (add to ~/.bashrc or ~/.zshrc)
export PATH="$HOME/.local/bin:$PATH"

# Verify
jit --version
jit version --json
```

### Verify Checksums

```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/checksums.txt
sha256sum -c checksums.txt
```

---

## Docker

**Best for running API + Web UI together.**

### Quick Start with Docker Compose

```bash
# Clone repository (or download docker-compose.yml)
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

# Initialize the shared data volume first — the API server refuses to start
# against an uninitialized directory. The cli service mounts the same volume
# and sets JIT_DATA_DIR=/data, so profiled init targets it.
docker-compose run --rm --entrypoint jit cli init --profile jit-dogfood

# Start all services (API + Web UI)
docker-compose up -d

# Access services
# - Web UI: http://localhost:8080
# - API: http://localhost:3000

# View logs
docker-compose logs -f

# Stop services
docker-compose down
```

### Pre-built Images (GitHub Container Registry)

```bash
# Pull component images by the rolling :main branch tag, a release-version,
# or a commit-sha tag.
docker pull ghcr.io/erankavija/just-in-time-api:main       # API server only
docker pull ghcr.io/erankavija/just-in-time-web:main       # Web UI only
docker pull ghcr.io/erankavija/just-in-time-cli:main       # CLI only
```

### Run Individual Containers

The Web UI image proxies `/api/` to the hostname `api` on its Docker network. Create a
user-defined network and give the API container that network alias before starting the Web UI.

#### API Server

```bash
docker network create jit-network

# Initialize the named volume once before starting the API server.
docker run --rm \
  --workdir /data \
  -v jit-data:/data \
  ghcr.io/erankavija/just-in-time-cli:main init --profile jit-dogfood

docker run -d \
  --name jit-api \
  --network jit-network \
  --network-alias api \
  -p 3000:3000 \
  -v jit-data:/data \
  -e JIT_DATA_DIR=/data \
  ghcr.io/erankavija/just-in-time-api:main
```

#### Web UI

```bash
docker run -d \
  --name jit-web \
  --network jit-network \
  -p 8080:80 \
  ghcr.io/erankavija/just-in-time-web:main
```

#### CLI (Interactive)

```bash
# Run CLI commands
docker run --rm \
  -v jit-data:/data \
  -e JIT_DATA_DIR=/data \
  ghcr.io/erankavija/just-in-time-cli:main \
  issue list

# Interactive shell
docker run --rm -it \
  --entrypoint sh \
  -v $(pwd):/data \
  -e JIT_DATA_DIR=/data \
  ghcr.io/erankavija/just-in-time-cli:main
```

## From Source

**For developers or if you need latest changes.**

### Prerequisites

- Rust 1.97 or newer — the workspace minimum supported version (`rust-version` in `Cargo.toml`), which CI's MSRV job builds against so a change requiring a newer compiler fails the pipeline (`.github/workflows/ci.yml`). Install via rustup: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Node.js 20+ (for MCP server and Web UI; the CI floor, `.github/workflows/ci.yml`)
- ripgrep (optional, for search: `sudo apt install ripgrep`)

### Build Rust Components

```bash
# Clone repository
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

# Build all Rust binaries (CLI, API server)
cargo build --release --workspace

# Binaries are in target/release/
./target/release/jit --version
./target/release/jit version
./target/release/jit-server --version

# Optional: Install to system
sudo cp target/release/jit /usr/local/bin/
sudo cp target/release/jit-server /usr/local/bin/
```

### Build MCP Server

```bash
# `npm test` runs `jit --schema`; use the release binary built above.
export PATH="$(pwd)/target/release:$PATH"
cd mcp-server
npm install
npm test

# Link globally (optional)
npm link
which jit-mcp-server   # confirm the linked bin is on PATH
```

### Build Web UI

```bash
cd web
npm install
npm run build

# Serve the built UI and its same-origin /api endpoint from the repository root.
cd ..
jit-server --data-dir .jit --web-dir web/dist
# Open http://localhost:3000
```

`npm run dev` starts Vite for frontend asset work, but Vite has no API proxy in this
repository. Because the UI calls `/api` on its own origin, use the same-origin command
above to exercise the UI against JIT, or configure a reverse proxy that sends `/api` to
`jit-server`.

---

## NPM (MCP Server)

**For AI agent integration via Model Context Protocol.**

The MCP server loads `jit --schema` at startup, so keep a built or installed `jit` executable
on `PATH` when launching `jit-mcp-server` from this package.

### Install from Source

```bash
cd mcp-server
npm install -g .
```

### MCP Client Configuration

Add this server definition to your MCP client's configuration:

```json
{
  "mcpServers": {
    "jit": {
      "command": "jit-mcp-server",
      "env": {
        "JIT_DATA_DIR": "/path/to/your/project/.jit"
      }
    }
  }
}
```

---

## Optional Dependencies

- **Git**: Core issue tracking is Git-optional, but advisory leases (`jit claim`) and worktree coordination need a Git repository with a resolvable `HEAD` (`apt install git`)
- **Docker**: For containerized deployment (`apt install docker.io docker-compose`)
- **Node.js** (20+): Required for the MCP server and to build or develop the Web UI (`apt install nodejs npm`); the CI floor is Node 20 (`.github/workflows/ci.yml`)
- **ripgrep**: For full-text search (`apt install ripgrep` or `yum install ripgrep`)

---

## Troubleshooting

### Command Not Found

```bash
# Check if binary is in PATH
which jit

# Add to PATH if needed
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

### Permission Denied

```bash
# Make binary executable
chmod +x jit

# Or run with sudo for system-wide install
sudo mv jit /usr/local/bin/
```

### Docker: Cannot Connect to API

```bash
# Check if services are running
docker-compose ps

# Check logs
docker-compose logs api

# Restart services
docker-compose restart
```

### Search Not Working

```bash
# Install ripgrep
sudo apt install ripgrep    # Debian/Ubuntu
sudo yum install ripgrep    # RHEL/CentOS
sudo dnf install ripgrep    # Fedora

# Verify
rg --version
```

---

## Next Steps

- Initialize with the preferred embedded workflow:
  `jit init --profile jit-dogfood`
- Read the [Repository Profiles reference](docs/reference/profiles.md) for the
  offline package contract, dry-run/apply commands, and recovery guarantees
- Read the [Quick Start](README.md#quick-start)
- See [Quickstart Tutorial](docs/tutorials/quickstart.md) for complete workflows
- Check [Deployment Guide](docs/how-to/deployment.md) for production setup

---

## Uninstallation

### Binary Installation

```bash
sudo rm /usr/local/bin/jit
sudo rm /usr/local/bin/jit-server
```

### Docker

```bash
docker-compose down -v  # Remove containers and volumes
docker rmi ghcr.io/erankavija/just-in-time:latest
```

### NPM

```bash
npm uninstall -g @erankavija/jit-mcp-server
```

### Data

```bash
# Remove JIT data (careful!)
rm -rf .jit/
```
