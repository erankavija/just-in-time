# Installation Guide

How to install JIT from the archive a release publishes, and how to build it
from a source checkout. Two adjacent workflows have their own homes:
[Deployment](docs/how-to/deployment.md) serves the API and web UI from a
container, and [MCP Integration](docs/how-to/mcp-integration.md) installs the
MCP server for an agent client.

## Table of Contents

- [Pre-built Binaries](#pre-built-binaries)
- [Build From a Source Checkout](#build-from-a-source-checkout)
- [Optional Dependencies](#optional-dependencies)
- [Troubleshooting](#troubleshooting)
- [Uninstallation](#uninstallation)
- [Next Steps](#next-steps)

---

## Pre-built Binaries

**Recommended for most users.** Statically linked binaries with no runtime
dependencies, published as a GitHub release asset. No package manager and no
container registry carries them; [what a release
publishes](docs/reference/release-policy.md#what-a-release-publishes) lists the
complete asset set.

The archive is flat and carries four files: the `jit` CLI, the `jit-server`
binary, and both license texts.

### Download and Verify

```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
wget https://github.com/erankavija/just-in-time/releases/latest/download/checksums.txt

sha256sum --check --ignore-missing checksums.txt
tar -xzf jit-linux-x64.tar.gz
```

The checksum file covers every archive the release carries, so
`--ignore-missing` verifies the one just downloaded and skips the rest.

### Install to System (requires sudo)

```bash
sudo mv jit jit-server /usr/local/bin/
```

### Install to a User Directory (no sudo)

```bash
mkdir -p ~/.local/bin
mv jit jit-server ~/.local/bin/

# Add to PATH (add to ~/.bashrc or ~/.zshrc)
export PATH="$HOME/.local/bin:$PATH"
```

### Confirm the Installation

```bash
jit --version
jit version --json
jit-server --version
```

`jit version --json` reports the product version, the
`x86_64-unknown-linux-musl` target the archive is built for, and the `release`
build profile. The release smoke test asserts those same three fields against
the extracted archive before the release is published
(`.github/workflows/release-artifacts.yml`).

---

## Build From a Source Checkout

**For contributors and for running unreleased changes.** A source build
produces the same two binaries the archive carries, from the working tree
rather than from a published asset.

### Prerequisites

- Rust at the workspace minimum supported version or newer — `rust-version`
  under `[workspace.package]` in `Cargo.toml` declares it, and [the supported
  Rust version](docs/reference/release-policy.md#supported-rust-version) covers
  how it is derived, kept within one minor release of current stable, and
  enforced. Install via rustup:
  `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Node.js to build the web bundle `jit-server` embeds — see [Optional
  Dependencies](#optional-dependencies)

### Build the Binaries

```bash
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

cargo build --locked --release --bin jit --bin jit-server

./target/release/jit --version
./target/release/jit-server --version
```

`--locked` builds the committed dependency set, which is how the published
archive is built (`.github/workflows/release-artifacts.yml`).

`jit-server` embeds whatever `web/dist/` holds when it is compiled and falls
back to an empty asset stub when that directory is absent — a server that
answers its API and serves no UI. [Building the web
UI](docs/how-to/deployment.md#building-the-web-ui) covers producing that bundle
before the server is compiled.

### Install With Build Provenance

```bash
./scripts/install-jit.sh
```

The wrapper records the source commit in the installed binary, so jit's
stale-binary guard can tell whether the binary matches the repository it
validates. Plain `cargo install --path crates/jit` also installs the CLI, but
produces a binary of unknown provenance, which the guard treats as
unverifiable.

---

## Optional Dependencies

- **Git**: Core issue tracking is Git-optional; advisory leases (`jit claim`)
  and worktree coordination need a Git repository with a resolvable `HEAD`
  (`apt install git`)
- **Node.js 20 or newer**: Required to build the web bundle and to run the MCP
  server (`apt install nodejs npm`); the floor is the `node-version: '20'` the
  CI jobs install (`.github/workflows/ci.yml`)
- **ripgrep**: For full-text search (`apt install ripgrep` or
  `yum install ripgrep`)
- **Docker or Podman**: For the containerized
  [deployment](docs/how-to/deployment.md#container-deployment)
  (`apt install docker.io docker-compose-v2`, or
  `apt install podman podman-compose`)

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

### Search Not Working

```bash
# Install ripgrep
sudo apt install ripgrep    # Debian/Ubuntu
sudo yum install ripgrep    # RHEL/CentOS
sudo dnf install ripgrep    # Fedora

# Verify
rg --version
```

Container startup failures are covered by [deployment
troubleshooting](docs/how-to/deployment.md#troubleshooting), and MCP client
startup failures by [MCP
troubleshooting](docs/how-to/mcp-integration.md#troubleshooting).

---

## Uninstallation

### Binaries

```bash
sudo rm /usr/local/bin/jit
sudo rm /usr/local/bin/jit-server
```

A user-directory install is removed the same way from `~/.local/bin`.

### Data

```bash
# Remove JIT data (careful!)
rm -rf .jit/
```

Removing a [container deployment](docs/how-to/deployment.md#container-deployment)
and an [installed MCP server](docs/how-to/mcp-integration.md) is covered by
their own guides.

---

## Next Steps

- Initialize with the preferred embedded workflow:
  `jit init --profile jit-dogfood`
- Read the [Repository Profiles reference](docs/reference/profiles.md) for the
  offline package contract, dry-run/apply commands, and recovery guarantees
- Read the [Quick Start](README.md#quick-start)
- See [Quickstart Tutorial](docs/tutorials/quickstart.md) for complete workflows
- Check the [Deployment Guide](docs/how-to/deployment.md) for serving the API
  and web UI
