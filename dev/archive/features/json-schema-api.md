# JSON Schema Export API

**Version:** 0.2.0  
**Status:** Complete  
**Feature:** Command schema introspection for AI agents

## Overview

The `jit --schema` command exports a complete JSON schema of all available commands, arguments, flags, types, and exit codes. This enables AI agents to programmatically discover and understand the CLI's capabilities.

## Usage

```bash
jit --schema > schema.json
```

The command outputs a structured JSON document to stdout, which can be piped to a file or consumed directly by AI agents.

## Schema Structure

### Top-Level Fields

```json
{
  "version": "0.2.0",
  "commands": { /* Command definitions */ },
  "types": { /* Type definitions */ },
  "exit_codes": [ /* Exit code documentation */ ]
}
```

### Command Definition

Each command includes:

```json
{
  "description": "Human-readable description",
  "subcommands": { /* Optional nested commands */ },
  "args": [
    {
      "name": "arg_name",
      "type": "string|number|boolean|array[type]",
      "required": true|false,
      "default": "optional default value",
      "description": "Argument description"
    }
  ],
  "flags": [
    {
      "name": "flag_name",
      "type": "boolean|string",
      "description": "Flag description"
    }
  ],
  "output": {
    "success": "TypeName",
    "error": "ErrorResponse"
  }
}
```

### Type Definitions

Common types are defined with JSON Schema-compatible syntax:

```json
{
  "State": {
    "type": "enum",
    "enum": ["open", "ready", "in_progress", "done", "archived"],
    "description": "Issue lifecycle state"
  },
  "Issue": {
    "type": "object",
    "properties": {
      "id": { "type": "string" },
      "title": { "type": "string" },
      "state": { "$ref": "#/types/State" },
      /* ... more properties */
    }
  }
}
```

### Exit Codes

All exit codes are documented:

```json
[
  {
    "code": 0,
    "description": "Command succeeded"
  },
  {
    "code": 3,
    "description": "Resource not found (issue, gate, etc.)"
  }
  /* ... more codes */
]
```

## Example Queries

### List All Commands

```bash
jit --schema | jq '.commands | keys'
```

Output:
```json
[
  "dep",
  "events",
  "gate",
  "graph",
  "init",
  "issue",
  "query",
  "registry",
  "status",
  "validate"
]
```

### Get Issue Subcommands

```bash
jit --schema | jq '.commands.issue.subcommands | keys'
```

Output:
```json
[
  "claim",
  "create",
  "delete",
  "list",
  "search",
  "show",
  "unclaim",
  "update"
]
```

### Inspect Create Command

```bash
jit --schema | jq '.commands.issue.subcommands.create'
```

Output:
```json
{
  "description": "Create a new issue",
  "args": [
    {
      "name": "title",
      "type": "string",
      "required": true,
      "description": "Issue title"
    },
    {
      "name": "priority",
      "type": "string",
      "required": false,
      "default": "normal",
      "description": "Priority: low, normal, high, critical"
    }
    /* ... more args */
  ],
  "flags": [
    {
      "name": "json",
      "type": "boolean",
      "description": "Output JSON format"
    }
  ],
  "output": {
    "success": "Issue",
    "error": "ErrorResponse"
  }
}
```

### Check Available States

```bash
jit --schema | jq '.types.State.enum'
```

Output:
```json
[
  "open",
  "ready",
  "in_progress",
  "done",
  "archived"
]
```

### Review Exit Codes

```bash
jit --schema | jq '.exit_codes'
```

## AI Agent Integration

### Using the Schema

AI agents can use the schema to:

1. **Discover commands**: Query `.commands` to see all available operations
2. **Validate arguments**: Check `.args` to ensure correct parameters
3. **Understand types**: Use `.types` to interpret output and construct valid input
4. **Handle errors**: Reference `.exit_codes` to interpret command failures

### Example: Claude with MCP

```typescript
// In an MCP server, cache the schema at startup
const schema = JSON.parse(execSync('jit --schema').toString());

// Dynamically generate MCP tools from schema
for (const [name, cmd] of Object.entries(schema.commands)) {
  if (cmd.subcommands) {
    for (const [subname, subcmd] of Object.entries(cmd.subcommands)) {
      registerTool({
        name: `jit_${name}_${subname}`,
        description: subcmd.description,
        inputSchema: generateInputSchema(subcmd.args),
      });
    }
  }
}
```

### Example: Python Script

```python
import json
import subprocess

# Load schema
schema = json.loads(subprocess.check_output(['jit', '--schema']))

# Find all commands that support --json flag
json_commands = []
for name, cmd in schema['commands'].items():
    if any(f['name'] == 'json' for f in cmd.get('flags', [])):
        json_commands.append(name)
    if 'subcommands' in cmd:
        for subname, subcmd in cmd['subcommands'].items():
            if any(f['name'] == 'json' for f in subcmd.get('flags', [])):
                json_commands.append(f"{name} {subname}")

print(f"Commands with --json: {json_commands}")
```

## Schema Versioning

The schema includes a `version` field that matches the JIT CLI version. Agents should:

1. Check `version` field on startup
2. Warn if schema version differs from expected version
3. Cache schema to avoid repeated calls
4. Re-fetch schema when CLI is updated

### Compatibility

- **Patch versions** (0.2.0 → 0.2.1): Backward compatible, no breaking changes
- **Minor versions** (0.2.0 → 0.3.0): New commands/args added, existing preserved
- **Major versions** (0.x.x → 1.0.0): Breaking changes possible

## Performance

- Schema generation is fast (~1ms)
- Output is ~50-100KB of JSON
- Recommended: Cache schema on agent startup
- Re-validate only when CLI version changes

## Implementation Notes

The schema is generated from:

1. **CLI definitions**: Command structure from `clap` parser
2. **Domain types**: Core types (Issue, State, Priority) from `domain.rs`
3. **Exit codes**: Standardized error codes from `output.rs`

The schema module (`schema.rs`) maintains this metadata separately from the CLI parser to ensure:

- Complete documentation
- Stable API surface
- Easy maintenance
- Version tracking

## Future Enhancements

Potential additions in future versions:

- [ ] OpenAPI-compatible schemas
- [ ] GraphQL schema export
- [ ] Command examples in schema
- [ ] Validation rules (regex patterns, min/max values)
- [ ] Deprecation warnings for old commands
- [ ] Command aliases and shortcuts

## See Also

- [CLI Consistency & MCP Strategy](cli-and-mcp-strategy.md)
- [Output Module](../crates/jit/src/output.rs) - JSON output format
- [Exit Codes](../crates/jit/src/output.rs) - Standardized exit codes
