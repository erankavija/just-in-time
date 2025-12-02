# JIT MCP Server

Model Context Protocol server for the Just-In-Time issue tracker.

## Overview

This MCP server wraps the `jit` CLI to provide MCP tools for AI agents like Claude. It dynamically generates tools from the JIT schema, ensuring the MCP interface stays synchronized with the CLI.

## Features

- **29 MCP tools** automatically generated from JIT schema
- **Type-safe** input validation using JSON Schema
- **Zero-maintenance** - tools update automatically when CLI changes
- **Full coverage** - all CLI commands exposed as MCP tools

## Installation

```bash
cd mcp-server
npm install
```

## Usage

### As MCP Server

The server communicates over stdio using the Model Context Protocol:

```bash
node index.js
```

### With Claude Desktop

Add to your Claude Desktop configuration (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

```json
{
  "mcpServers": {
    "jit": {
      "command": "node",
      "args": ["/path/to/just-in-time/mcp-server/index.js"]
    }
  }
}
```

## Available Tools

The server exposes 29 tools organized by command:

### Issue Management
- `jit_issue_create` - Create a new issue
- `jit_issue_list` - List issues with filters
- `jit_issue_show` - Show issue details
- `jit_issue_search` - Search issues by text
- `jit_issue_update` - Update an issue
- `jit_issue_delete` - Delete an issue
- `jit_issue_claim` - Claim an issue
- `jit_issue_unclaim` - Unclaim an issue

### Dependency Management
- `jit_dep_add` - Add dependency between issues
- `jit_dep_rm` - Remove dependency

### Gate Management
- `jit_gate_add` - Add gate to issue
- `jit_gate_pass` - Mark gate as passed
- `jit_gate_fail` - Mark gate as failed

### Registry Management
- `jit_registry_add` - Register new gate definition
- `jit_registry_list` - List registered gates
- `jit_registry_show` - Show gate definition

### Events
- `jit_events_tail` - Show recent events

### Graph Operations
- `jit_graph_show` - Show dependency graph
- `jit_graph_roots` - Show root issues
- `jit_graph_downstream` - Show downstream issues
- `jit_graph_export` - Export graph (dot/mermaid)

### Query Operations
- `jit_query_ready` - Query ready issues
- `jit_query_blocked` - Query blocked issues
- `jit_query_assignee` - Query by assignee
- `jit_query_state` - Query by state
- `jit_query_priority` - Query by priority

### System
- `jit_init` - Initialize tracker
- `jit_status` - Show status
- `jit_validate` - Validate repository

## Example Usage (via MCP)

When used with an MCP client like Claude:

```
User: Create a high-priority issue for implementing authentication
Claude: [calls jit_issue_create with title="Implement authentication", priority="high"]

User: Show me all ready issues
Claude: [calls jit_query_ready]

User: Add a dependency - the auth issue depends on the database setup
Claude: [calls jit_dep_add with from="AUTH_ID", to="DB_ID"]
```

## Implementation Details

### Dynamic Tool Generation

The server loads `jit-schema.json` at startup and generates MCP tools dynamically:

```javascript
function generateTools() {
  const tools = [];
  
  for (const [cmdName, cmd] of Object.entries(jitSchema.commands)) {
    if (cmd.subcommands) {
      for (const [subName, subCmd] of Object.entries(cmd.subcommands)) {
        tools.push(generateToolFromCommand(subName, subCmd, cmdName));
      }
    } else {
      tools.push(generateToolFromCommand(cmdName, cmd));
    }
  }
  
  return tools;
}
```

### CLI Execution

Each tool call is mapped to a `jit` CLI command with `--json` flag:

```javascript
async function executeTool(name, args) {
  // jit_issue_create -> jit issue create --title "..." --json
  const cliArgs = buildCliCommand(name, args);
  return await runJitCommand(cliArgs);
}
```

### Error Handling

Errors from the CLI are parsed and returned in MCP format:

```javascript
try {
  const result = await executeTool(name, args);
  return { content: [{ type: "text", text: JSON.stringify(result) }] };
} catch (error) {
  return { 
    content: [{ type: "text", text: `Error: ${error.message}` }],
    isError: true 
  };
}
```

## Development

### Updating Tools

Tools are automatically synchronized with the CLI schema. To refresh:

```bash
cd ..
./target/release/jit --schema > mcp-server/jit-schema.json
```

### Testing

Test the server manually:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | node index.js
```

## Architecture

```
┌─────────────┐
│ AI Agent    │  (Claude Desktop, etc.)
│             │
└──────┬──────┘
       │ MCP Protocol (JSON-RPC over stdio)
       │
┌──────▼──────────┐
│  index.js       │  MCP Server
│                 │  - Loads jit-schema.json
│                 │  - Generates 29 tools dynamically
│                 │  - Handles tool calls
└──────┬──────────┘
       │ Shell execution
       │
┌──────▼──────────┐
│   jit CLI       │  (with --json flag)
│                 │
└─────────────────┘
```

## Version

MCP Server Version: 0.1.0
JIT CLI Version: 0.2.0

## License

MIT OR Apache-2.0 (matches parent project)

## See Also

- [JIT CLI Documentation](../README.md)
- [JSON Schema API](../docs/json-schema-api.md)
- [CLI & MCP Strategy](../docs/cli-and-mcp-strategy.md)
- [Model Context Protocol](https://modelcontextprotocol.io/)
