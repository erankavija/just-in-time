# JIT MCP Server

Model Context Protocol server for the Just-In-Time issue tracker.

## Overview

This MCP server wraps the `jit` CLI to provide MCP tools for AI agents like Claude. It dynamically generates tools from the JIT schema, ensuring the MCP interface stays synchronized with the CLI.

## Features

- **60+ MCP tools** automatically generated from JIT schema
- **Nested subcommand support** - handles multi-level commands like `doc.assets.list`
- **Type-safe** input validation using Zod
- **Runtime schema loading** - prefers live schema from `jit --schema` with fallback to bundled schema
- **Structured error responses** - consistent JSON envelope with error codes
- **Operational hardening** - timeouts (30s) and concurrency limits (10 concurrent commands)
- **Modular architecture** - clean separation of concerns for maintainability
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

### With GitHub Copilot CLI

**Note**: MCP support in GitHub Copilot CLI requires the `jit` binary in your PATH.

1. Build the CLI and add to PATH:
   ```bash
   cd /path/to/just-in-time
   cargo build --release
   export PATH="$(pwd)/target/release:$PATH"
   ```

2. Configure the MCP server (location depends on your Copilot version):

   **Option A**: Create `~/.config/github-copilot/mcp-servers.json`:
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

   **Option B**: Or use `~/.config/github-copilot/agents.json`:
   ```json
   {
     "agents": {
       "jit": {
         "name": "JIT Issue Tracker",
         "description": "Just-In-Time issue tracker with dependency graphs",
         "mcp": {
           "command": "node",
           "args": ["/path/to/just-in-time/mcp-server/index.js"]
         }
       }
     }
   }
   ```

3. Verify configuration:
   ```bash
   gh copilot suggest "Use jit to create a new high-priority issue"
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

The server exposes 60+ tools organized by command, including nested subcommands:

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

### Document Management
- `jit_doc_add` - Add document reference to issue
- `jit_doc_list` - List document references
- `jit_doc_remove` - Remove document reference
- `jit_doc_show` - Show document content

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

### Architecture

The server is modularized into focused components:

```
mcp-server/
├── index.js                    # MCP server entry point
└── lib/
    ├── schema-loader.js        # Runtime schema loading with fallback
    ├── tool-generator.js       # Recursive tool generation
    ├── validator.js            # Zod-based input validation
    ├── cli-executor.js         # CLI execution with timeouts
    └── concurrency.js          # Concurrency limiter
```

### Schema Loading

The server prefers loading the schema from the runtime CLI to ensure synchronization:

```javascript
// Try loading from `jit --schema` first
const cliSchema = await loadSchemaFromCli();
const bundledSchema = loadSchemaFromFile(schemaPath);

if (!cliSchema) {
  warnings.push("Could not load schema from jit CLI. Using bundled schema.");
  return { schema: bundledSchema, warnings };
}

// Check version mismatch
if (cliSchema.version !== bundledSchema.version) {
  warnings.push(`Schema version mismatch: CLI ${cliSchema.version}, bundled ${bundledSchema.version}`);
}

return { schema: cliSchema, warnings };
```

### Dynamic Tool Generation with Nested Subcommands

The server recursively generates tools from the schema, supporting multi-level nesting:

```javascript
function generateToolsRecursive(commands, parentPath = []) {
  const tools = [];
  
  for (const [cmdName, cmd] of Object.entries(commands)) {
    const currentPath = [...parentPath, cmdName];
    
    if (cmd.subcommands) {
      // Recurse into subcommands
      tools.push(...generateToolsRecursive(cmd.subcommands, currentPath));
    } else {
      // Leaf command - generate tool
      tools.push(generateToolFromCommand(currentPath, cmd));
    }
  }
  
  return tools;
}
```

This correctly handles commands like `doc.assets.list` → `jit_doc_assets_list`.

### Input Validation

Arguments are validated with Zod before CLI execution:

```javascript
// Validate arguments against tool schema
const validation = validateArguments(args, tool.inputSchema);

if (!validation.success) {
  return {
    content: [{
      type: "text",
      text: JSON.stringify({
        success: false,
        error: { code: "VALIDATION_ERROR", message: validation.error }
      })
    }],
    isError: true
  };
}
```

### CLI Execution with Timeouts

Each command execution includes timeout and concurrency controls:

```javascript
// Execute with concurrency limiting and timeout
const result = await concurrencyLimiter.run(async () => {
  return await executeCommand(cmdPath, args, cmdDef, 30000); // 30s timeout
});
```

### Structured Error Responses

All responses follow a consistent envelope format:

```javascript
// Success
{
  "success": true,
  "data": { /* command output */ }
}

// Error
{
  "success": false,
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Validation failed: id: Required"
  }
}
```

MCP responses mark errors with `isError: true` for proper client handling.

## Development

### Updating Tools

Tools are automatically synchronized with the CLI schema. The server prefers loading schema from `jit --schema` at runtime, but you can update the bundled fallback:

```bash
cd ..
./target/release/jit --schema > mcp-server/jit-schema.json
```

### Testing

Run the automated test suite:

```bash
# Ensure jit is in PATH first
cd ..
cargo build --release
export PATH="$(pwd)/target/release:$PATH"

# Run tests
cd mcp-server
npm test
```

The test suite verifies:
- MCP protocol initialization
- Tool listing (60+ tools)
- Nested subcommand tool generation and CLI mapping
- Input validation with Zod (required fields, type checking)
- Structured error responses with proper envelopes
- Schema correctness (new `backlog` and `gated` states)
- Tool execution and error handling
- Invalid tool/argument rejection

Test manually with JSON-RPC:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | node index.js
```

Test with isolated directory:

```bash
./test-with-env.sh
```

## Troubleshooting

### "jit: command not found"

The MCP server executes `jit` commands via shell. Ensure it's in your PATH:

```bash
# Check if jit is accessible
which jit

# If not, add to your shell config (~/.bashrc, ~/.zshrc, etc.)
export PATH="/path/to/just-in-time/target/release:$PATH"

# Then reload
source ~/.bashrc  # or ~/.zshrc
```

Alternatively, modify `index.js` to use absolute path:
```javascript
// Change in runJitCommand():
const cmd = `/absolute/path/to/jit ${args}${jsonFlag}`;
```

### Module not found

Install dependencies:
```bash
cd mcp-server
npm install
```

### Node version issues

Requires Node.js v16+:
```bash
node --version  # Should be v16 or later
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│ AI Agent (Claude Desktop, GitHub Copilot, etc.)       │
└────────────────────┬────────────────────────────────────┘
                     │ MCP Protocol (JSON-RPC over stdio)
                     │
┌────────────────────▼────────────────────────────────────┐
│ MCP Server (index.js)                                   │
│                                                          │
│  ┌─────────────────────────────────────────────┐        │
│  │ Schema Loader                               │        │
│  │ - Prefers jit --schema (runtime)            │        │
│  │ - Falls back to bundled jit-schema.json     │        │
│  │ - Warns on version mismatch                 │        │
│  └─────────────────────────────────────────────┘        │
│                     │                                    │
│  ┌─────────────────▼─────────────────────┐              │
│  │ Tool Generator                        │              │
│  │ - Recursive generation (nested cmds)  │              │
│  │ - Generates 60+ tools from schema     │              │
│  └─────────────────────────────────────┬─┘              │
│                                        │                 │
│  ┌─────────────────▼─────────────────┐ │                │
│  │ Validator (Zod)                   │ │                │
│  │ - Validates args before execution │ │                │
│  │ - Returns structured errors       │ │                │
│  └─────────────────────────────────┬─┘ │                │
│                                    │   │                 │
│  ┌─────────────────▼───────────────▼─┐                  │
│  │ CLI Executor                      │                  │
│  │ - Maps tool calls to CLI args     │                  │
│  │ - 30s timeout per command         │                  │
│  │ - Structured error responses      │                  │
│  └─────────────────┬─────────────────┘                  │
│                    │                                     │
│  ┌─────────────────▼─────────────────┐                  │
│  │ Concurrency Limiter               │                  │
│  │ - Max 10 concurrent commands      │                  │
│  └─────────────────┬─────────────────┘                  │
└────────────────────┼─────────────────────────────────────┘
                     │ execFile('jit', [...args])
                     │
┌────────────────────▼────────────────────────────────────┐
│ jit CLI (with --json flag)                              │
└─────────────────────────────────────────────────────────┘
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
