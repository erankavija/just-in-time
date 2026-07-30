# Product Compatibility

> **Diátaxis Type:** Reference

**Product compatibility version:** `1.0.0`

This release keeps the following supported capabilities aligned under one
product version:

- native `jit` CLI
- `jit-server` Rust server and API
- built web UI
- `@erankavija/jit-mcp-server` MCP server

The release tag adds a `v` prefix to the product version. Component manifests
and their committed lock metadata use the product version without that prefix.

The product version does not version repository data. Repository compatibility
is governed independently by `schema_version` in `.jit/index.json`, as defined
by the [storage-format versioning contract](storage-format.md#versioning).
