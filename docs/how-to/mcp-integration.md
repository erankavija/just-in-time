# Integrating JIT With an MCP Client

> **Diátaxis Type:** How-To Guide

The JIT MCP server exposes the CLI to any Model Context Protocol client, so an
agent drives issues, dependencies, and gates as MCP tools instead of shelling
out. This guide installs the published server and starts it from a client.

The server is published as a release asset, not as a package-registry version:
the release carries the tarball `npm pack` produces from the package manifest
(`.github/workflows/release-artifacts.yml`), and adopters install that file.
[What a release publishes](../reference/release-policy.md#what-a-release-publishes)
lists it beside the other assets.

## Prerequisites

- **The `jit` CLI on `PATH`.** The server runs `jit --schema` at startup and
  generates its tool surface from the answer, so it fails fast when the binary
  is absent. [Install it](../../INSTALL.md#pre-built-binaries) first.
- **Node.js**, at the version in [optional
  dependencies](../../INSTALL.md#optional-dependencies).

## Install the Released Tarball

`npm pack` names the tarball after the package manifest, so the release asset
is `erankavija-jit-mcp-server-<version>.tgz`, where `<version>` is the product
version [the compatibility record](../reference/compatibility.md) declares.
Download that asset from the release and install it globally:

```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/erankavija-jit-mcp-server-<version>.tgz
npm install -g ./erankavija-jit-mcp-server-<version>.tgz
```

The package installs one executable, `jit-mcp-server`, which speaks the
protocol over stdio. Asking it for its tool listing confirms both the
installation and the `jit` binary it wraps:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jit-mcp-server
```

## Start It From a Client

An MCP client launches the executable itself; nothing listens on a port. Add
the server to the client's configuration, naming the repository whose `.jit/`
directory the tools operate on:

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

`JIT_DATA_DIR` names the data directory itself and takes precedence over
repository discovery, which is what pins the tools to one repository regardless
of the working directory the client launches from. Without it, each command
discovers the repository by walking up from that working directory.

A client with its own registration command takes the same executable, for
example `copilot mcp add jit -- jit-mcp-server` for the GitHub Copilot CLI,
which records it in that client's own configuration file.

## What the Client Sees

Every leaf command in `jit --schema` becomes a tool named `jit_<command_path>`,
so `jit doc assets list` is `jit_doc_assets_list`. `tools/list` advertises a
curated subset — the commands an autonomous agent drives to find work, claim
it, inspect it, transition it, and check gates — while every generated tool
stays callable by name. [The MCP tools
reference](../reference/cli-commands.md#mcp-tools-reference) covers that
surface and its response envelopes.

## Upgrading

The server generates its tools from the schema it loads at startup, so an
upgraded `jit` reaches the client only after the server restarts. Replace both
artifacts together — [the compatibility
record](../reference/compatibility.md#upgrade-expectations) states what an
upgrade requires — and restart the MCP client.

## Troubleshooting

### `jit: command not found`

The server launches `jit` directly, inheriting the MCP host process
environment. A client started from a desktop launcher often has a different
`PATH` than an interactive shell, so confirm the binary is reachable from the
environment the client runs in:

```bash
which jit
```

Install it to a directory on that `PATH`, or extend the client's environment in
the same configuration block that declares the server.

### The client lists tools that the CLI does not have

The advertised surface is whatever `jit --schema` reported when the server
started. Restart the MCP client after installing a different `jit`.

### Commands act on the wrong repository

Set `JIT_DATA_DIR` in the server's configuration block. Repository discovery
otherwise depends on the client's working directory.

## See Also

- [Installation Guide](../../INSTALL.md) - Installing the CLI the server wraps
- [MCP Tools Reference](../reference/cli-commands.md#mcp-tools-reference) - The
  generated tool surface
- [Multi-Agent Coordination](multi-agent-coordination.md) - Coordinating
  several agents on one repository
