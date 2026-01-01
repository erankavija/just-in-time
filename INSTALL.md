# Installation Guide

This guide covers all methods to install JIT Issue Tracker on Linux systems.

## Table of Contents

- [Pre-built Binaries](#pre-built-binaries)
- [Docker](#docker)
- [From Source](#from-source)
- [NPM (MCP Server)](#npm-mcp-server)
- [System Requirements](#system-requirements)

---

## Pre-built Binaries

**Recommended for most users.** Static binaries with zero dependencies.

### Download Latest Release

```bash
# Download and extract
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz

# Install to system (requires sudo)
sudo mv jit jit-server jit-dispatch /usr/local/bin/

# Verify installation
jit --version
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
# Pull latest images
docker pull ghcr.io/erankavija/just-in-time:latest         # All-in-one
docker pull ghcr.io/erankavija/just-in-time-api:latest     # API server only
docker pull ghcr.io/erankavija/just-in-time-web:latest     # Web UI only
docker pull ghcr.io/erankavija/just-in-time-cli:latest     # CLI only
```

### Run Individual Containers

#### API Server

```bash
docker run -d \
  --name jit-api \
  -p 3000:3000 \
  -v jit-data:/data \
  -e JIT_DATA_DIR=/data \
  ghcr.io/erankavija/just-in-time-api:latest
```

#### Web UI

```bash
docker run -d \
  --name jit-web \
  -p 8080:80 \
  ghcr.io/erankavija/just-in-time-web:latest
```

#### CLI (Interactive)

```bash
# Run CLI commands
docker run --rm \
  -v jit-data:/data \
  -e JIT_DATA_DIR=/data \
  ghcr.io/erankavija/just-in-time-cli:latest \
  issue list

# Interactive shell
docker run --rm -it \
  -v $(pwd):/data \
  -e JIT_DATA_DIR=/data \
  ghcr.io/erankavija/just-in-time-cli:latest \
  sh
```

### All-in-One Container

```bash
# Run API + Web UI in single container
docker run -d \
  --name jit-all \
  -p 3000:3000 \
  -p 8080:80 \
  -v jit-data:/data \
  ghcr.io/erankavija/just-in-time:latest
```

---

## From Source

**For developers or if you need latest changes.**

### Prerequisites

- Rust 1.80+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Node.js 20+ (for MCP server and Web UI)
- ripgrep (optional, for search: `sudo apt install ripgrep`)

### Build Rust Components

```bash
# Clone repository
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

# Build all Rust binaries (CLI, API server, dispatch)
cargo build --release --workspace

# Binaries are in target/release/
./target/release/jit --version
./target/release/jit-server --version
./target/release/jit-dispatch --version

# Optional: Install to system
sudo cp target/release/jit /usr/local/bin/
sudo cp target/release/jit-server /usr/local/bin/
sudo cp target/release/jit-dispatch /usr/local/bin/
```

### Build MCP Server

```bash
cd mcp-server
npm install
npm test

# Link globally (optional)
npm link
jit-mcp-server --version
```

### Build Web UI

```bash
cd web
npm install
npm run build

# Development server
npm run dev
# Access at http://localhost:5173

# Production build is in dist/
# Serve with any static file server
```

---

## NPM (MCP Server)

**For AI agent integration via Model Context Protocol.**

### Install from NPM (After Publishing)

```bash
npm install -g @erankavija/jit-mcp-server
jit-mcp-server
```

### Install from Source

```bash
cd mcp-server
npm install -g .
```

### Usage with Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

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

## System Requirements

### Minimum

- **OS**: Linux (Ubuntu 20.04+, Debian 11+, RHEL 8+, or equivalent)
- **Architecture**: x86_64 (amd64)
- **RAM**: 256 MB
- **Disk**: 50 MB for binaries, ~10 MB per 1000 issues

### Recommended

- **RAM**: 512 MB+ (for API server with multiple concurrent clients)
- **Disk**: SSD for better I/O performance
- **ripgrep**: For full-text search (`apt install ripgrep` or `yum install ripgrep`)

### Optional Dependencies

- **Git**: For document version tracking (`apt install git`)
- **Docker**: For containerized deployment (`apt install docker.io docker-compose`)
- **Node.js**: Only needed for MCP server (`apt install nodejs npm`)

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

- Initialize a repository: `jit init`
- Read the [Quick Start](README.md#quick-start)
- See [Quickstart Tutorial](docs/tutorials/quickstart.md) for complete workflows
- Check [DEPLOYMENT.md](DEPLOYMENT.md) for production setup

---

## Uninstallation

### Binary Installation

```bash
sudo rm /usr/local/bin/jit
sudo rm /usr/local/bin/jit-server
sudo rm /usr/local/bin/jit-dispatch
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
