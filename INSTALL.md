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

**Best for running the API and Web UI together.** One image, built from this
repository, runs a single `jit-server` process that serves both on port 3000
against a repository bind-mounted at `/repo`.

### Build the Image

```bash
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time
docker build -t jit-server:local .
```

### Serve a Repository

Initialize the repository on the host first (`jit init --profile jit-dogfood`);
the container serves an existing one and creates nothing.

The image runs as UID:GID `10001:10001`, an identity that owns nothing on the
host, so an unmapped container reaches the mount only if the repository already
grants `10001` read, write, and execute permission. Map the repository's owner
onto the container user instead — that is what `JIT_UID` and `JIT_GID` carry:

```bash
export JIT_REPO=/path/to/your/repo
export JIT_UID=$(stat -c '%u' "$JIT_REPO")
export JIT_GID=$(stat -c '%g' "$JIT_REPO")

docker compose up -d     # from this checkout; reads the three variables above
```

`http://localhost:3000` then serves the API and the Web UI. `docker compose
down` stops it.

The equivalent without Compose:

```bash
docker run -d --name jit-server \
  --user "$JIT_UID:$JIT_GID" \
  --publish 3000:3000 \
  --volume "$JIT_REPO:/repo" \
  jit-server:local
```

[Deployment](docs/how-to/deployment.md#container-deployment-team-server) covers
the mount contract, rootless Podman, backups, and troubleshooting.

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
- **Docker or Podman**: For containerized deployment (`apt install docker.io docker-compose-v2`, or `apt install podman podman-compose`)
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

### Docker: Cannot Connect to the Server

```bash
# Check whether the service is running
docker compose ps

# Check logs; status 78 names the mount check that failed
docker compose logs jit-server
```

[Deployment troubleshooting](docs/how-to/deployment.md#troubleshooting) covers
the mount-identity failures behind status 78.

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
docker compose down          # Stop and remove the container
docker rmi jit-server:local  # Remove the image you built
```

The served repository is a host directory and outlives both.

### NPM

```bash
npm uninstall -g @erankavija/jit-mcp-server
```

### Data

```bash
# Remove JIT data (careful!)
rm -rf .jit/
```
