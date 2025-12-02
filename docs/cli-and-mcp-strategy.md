# CLI Consistency & MCP Server Strategy

**Date**: 2025-12-02  
**Status**: Planning  
**Goal**: Make JIT optimally usable by both humans and AI agents with maximum consistency

## Current State Analysis

### Existing CLI Strengths ✅
- **Consistent structure**: All commands follow `jit <resource> <action>` pattern
- **JSON output**: `--json` flag on mutation commands (create, update, claim, etc.)
- **Predictable**: ID-first argument order for mutations
- **Machine-readable**: JSON output includes all relevant data

### Gaps to Address 🔍
- **Inconsistent JSON support**: Not all commands have `--json` (e.g., `list`, `show`, `validate`)
- **No structured errors**: Errors go to stderr as text, not JSON
- **Exit codes**: Not fully standardized
- **Query output**: Some commands output plain text when JSON would be better
- **Discoverability**: No `--help-json` for agents to introspect available commands

## Recommendation: Dual Strategy

### **Strategy 1: Enhanced CLI (Priority 1)**
**Why**: Direct, simple, works with any tool/agent, no additional server needed

### **Strategy 2: MCP Server (Priority 2)**  
**Why**: Better for AI agents that support MCP (Claude, future agents), provides type-safe interface

**Approach**: Do **both**, with CLI as foundation and MCP as a thin wrapper.

---

## Priority 1: Enhanced CLI for Agent Consistency

### 1.1 Universal JSON Output

**Goal**: Every command supports `--json` flag for machine-readable output

#### Actions:
- [ ] Add `--json` to all read commands (`list`, `show`, `search`, `validate`)
- [ ] Add `--json` to all query commands (`query ready`, `status`, `graph show`)
- [ ] Standardize JSON structure:
  ```json
  {
    "success": true,
    "data": { ... },
    "metadata": {
      "timestamp": "2025-12-02T20:56:35Z",
      "version": "0.2.0"
    }
  }
  ```

#### Example Transformations:

**Before** (human-readable):
```bash
$ jit issue list
Open: 5  Ready: 3  In Progress: 2  Done: 10

abc123 | Feature: Add authentication | high | open
def456 | Bug: Fix login redirect     | critical | ready
```

**After** (with --json):
```bash
$ jit issue list --json
{
  "success": true,
  "data": {
    "issues": [
      {
        "id": "abc123",
        "title": "Feature: Add authentication",
        "priority": "high",
        "state": "open",
        "assignee": null,
        "dependencies": [],
        "gates_required": ["unit-tests"],
        "gates_status": {},
        "description": "...",
        "context": {}
      },
      {
        "id": "def456",
        "title": "Bug: Fix login redirect",
        "priority": "critical",
        "state": "ready",
        ...
      }
    ],
    "summary": {
      "open": 5,
      "ready": 3,
      "in_progress": 2,
      "done": 10
    }
  },
  "metadata": {
    "timestamp": "2025-12-02T20:56:35Z",
    "version": "0.2.0",
    "command": "issue list"
  }
}
```

**Estimated**: 8-10 hours

---

### 1.2 Structured Error Output

**Goal**: Errors are JSON-formatted when `--json` is used

#### Actions:
- [ ] Create `ErrorResponse` type
  ```rust
  #[derive(Serialize)]
  struct ErrorResponse {
      success: bool,
      error: ErrorDetail,
      metadata: Metadata,
  }
  
  #[derive(Serialize)]
  struct ErrorDetail {
      code: String,           // "ISSUE_NOT_FOUND"
      message: String,        // Human-readable
      details: Option<Value>, // Additional context
      suggestions: Vec<String>,
  }
  ```

- [ ] Wrap all command handlers to catch errors
- [ ] Output JSON errors to stdout (not stderr) when `--json` flag present
- [ ] Keep text errors on stderr for human use

#### Example:

**Before**:
```bash
$ jit issue show nonexistent
Error: Issue not found: nonexistent
```

**After** (with --json):
```bash
$ jit issue show nonexistent --json
{
  "success": false,
  "error": {
    "code": "ISSUE_NOT_FOUND",
    "message": "Issue not found: nonexistent",
    "details": {
      "requested_id": "nonexistent"
    },
    "suggestions": [
      "Run 'jit issue list' to see available issues",
      "Check if the issue ID is correct"
    ]
  },
  "metadata": {
    "timestamp": "2025-12-02T20:56:35Z",
    "version": "0.2.0"
  }
}
$ echo $?
1
```

**Estimated**: 6-8 hours

---

### 1.3 Standardized Exit Codes

**Goal**: Predictable exit codes for automation

#### Exit Code Table:
```
0   - Success
1   - Generic error
2   - Invalid arguments / usage error
3   - Resource not found (issue, gate, etc.)
4   - Validation failed (cycle detected, broken refs, etc.)
5   - Permission denied
6   - Already exists (duplicate)
10  - External dependency failed (git, file system)
```

#### Actions:
- [ ] Define `ExitCode` enum in CLI module
- [ ] Map all error types to exit codes
- [ ] Document exit codes in `--help`
- [ ] Add tests for each exit code scenario

**Estimated**: 4-6 hours

---

### 1.4 Command Schema Export

**Goal**: Agents can discover available commands programmatically

#### New Command:
```bash
$ jit --schema json
{
  "version": "0.2.0",
  "commands": {
    "issue": {
      "subcommands": {
        "create": {
          "description": "Create a new issue",
          "args": [
            {"name": "title", "type": "string", "required": true},
            {"name": "desc", "type": "string", "required": false, "default": ""},
            {"name": "priority", "type": "enum", "values": ["low", "normal", "high", "critical"], "default": "normal"},
            {"name": "gate", "type": "array[string]", "required": false}
          ],
          "flags": [
            {"name": "json", "type": "boolean", "description": "Output JSON"}
          ],
          "output": {
            "success": {"type": "object", "schema": "Issue"},
            "error": {"type": "object", "schema": "ErrorResponse"}
          }
        },
        "list": { ... },
        "show": { ... }
      }
    },
    "dep": { ... },
    "gate": { ... }
  },
  "types": {
    "Issue": { ... },
    "ErrorResponse": { ... }
  }
}
```

#### Actions:
- [ ] Generate schema from clap definitions
- [ ] Include JSON schema for all output types
- [ ] Version schema to track API changes

**Estimated**: 6-8 hours

---

### 1.5 Batch Operations

**Goal**: Agents can perform multiple operations efficiently

#### New Flag:
```bash
$ jit --batch < operations.json
```

**Input format** (`operations.json`):
```json
{
  "operations": [
    {
      "command": "issue create",
      "args": {
        "title": "Task 1",
        "priority": "high"
      }
    },
    {
      "command": "issue create",
      "args": {
        "title": "Task 2",
        "priority": "normal"
      }
    },
    {
      "command": "dep add",
      "args": {
        "from": "$ops[1].data.id",  // Reference previous result
        "to": "$ops[0].data.id"
      }
    }
  ]
}
```

**Output**:
```json
{
  "success": true,
  "results": [
    { "success": true, "data": { "id": "abc123", ... } },
    { "success": true, "data": { "id": "def456", ... } },
    { "success": true, "data": { "from": "def456", "to": "abc123" } }
  ],
  "metadata": { ... }
}
```

#### Actions:
- [ ] Parse batch JSON input
- [ ] Execute operations sequentially
- [ ] Support result references (`$ops[N].data.field`)
- [ ] Rollback on error (optional `--atomic` flag)
- [ ] Return aggregated results

**Estimated**: 10-12 hours

---

## Priority 2: MCP Server Integration

**What is MCP?** Model Context Protocol - Standard for AI assistants to interact with external tools

### Why MCP for JIT?

✅ **Type-safe**: Strongly typed tool definitions  
✅ **Discoverable**: Agents automatically understand capabilities  
✅ **Composable**: Works with any MCP-compatible AI (Claude, future agents)  
✅ **Structured**: Native JSON request/response  
✅ **Real-time**: Can maintain state across operations  

### 2.1 MCP Server Architecture

```
┌─────────────┐
│ AI Agent    │
│ (Claude,etc)│
└──────┬──────┘
       │ MCP Protocol (JSON-RPC)
       │
┌──────▼──────┐
│ jit-mcp     │  ← New: Thin MCP server wrapper
│ (Rust/Node) │
└──────┬──────┘
       │ Uses CommandExecutor
       │
┌──────▼──────┐
│ jit library │  ← Existing core
│ (Rust)      │
└─────────────┘
```

### 2.2 MCP Server Implementation Options

#### Option A: Rust MCP Server (Recommended)
**Pros**: 
- Same language as core
- Direct use of CommandExecutor
- Type safety
- Performance

**Cons**:
- Need to implement MCP protocol in Rust (no official library yet)
- More initial work

**Structure**:
```rust
// crates/jit-mcp/src/main.rs
use jit::{CommandExecutor, JsonFileStorage};

struct JitMcpServer {
    executor: CommandExecutor<JsonFileStorage>,
}

impl McpServer for JitMcpServer {
    fn list_tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: "jit_create_issue",
                description: "Create a new issue",
                input_schema: json_schema::IssueCreateInput::schema(),
            },
            Tool {
                name: "jit_list_issues",
                description: "List issues with optional filters",
                input_schema: json_schema::IssueListInput::schema(),
            },
            // ... all commands as tools
        ]
    }
    
    fn call_tool(&mut self, name: &str, args: Value) -> Result<Value> {
        match name {
            "jit_create_issue" => {
                let input: IssueCreateInput = serde_json::from_value(args)?;
                let issue = self.executor.create_issue(
                    input.title,
                    input.description,
                    input.priority,
                )?;
                Ok(serde_json::to_value(issue)?)
            }
            "jit_list_issues" => { ... }
            _ => Err(anyhow!("Unknown tool: {}", name))
        }
    }
}
```

**Estimated**: 15-20 hours

#### Option B: TypeScript MCP Server
**Pros**:
- Official MCP SDK available (`@modelcontextprotocol/sdk`)
- Faster to implement
- Examples available

**Cons**:
- Need to shell out to `jit` CLI
- Parsing CLI output
- Less type safety

**Structure**:
```typescript
// mcp-server/src/index.ts
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { exec } from "child_process";
import { promisify } from "util";

const execAsync = promisify(exec);

const server = new Server({
  name: "jit-mcp-server",
  version: "0.1.0",
});

server.setRequestHandler("tools/list", async () => ({
  tools: [
    {
      name: "jit_create_issue",
      description: "Create a new issue in the tracker",
      inputSchema: {
        type: "object",
        properties: {
          title: { type: "string" },
          description: { type: "string" },
          priority: { type: "string", enum: ["low", "normal", "high", "critical"] },
        },
        required: ["title"],
      },
    },
    // ... more tools
  ],
}));

server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;
  
  switch (name) {
    case "jit_create_issue": {
      const cmd = `jit issue create --title "${args.title}" --json`;
      const { stdout } = await execAsync(cmd);
      return JSON.parse(stdout);
    }
    // ... more handlers
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
```

**Estimated**: 8-10 hours

---

### 2.3 MCP Tools Mapping

**Goal**: Map each CLI command to an MCP tool

| MCP Tool | CLI Command | Description |
|----------|-------------|-------------|
| `jit_create_issue` | `jit issue create` | Create new issue |
| `jit_list_issues` | `jit issue list` | List issues with filters |
| `jit_show_issue` | `jit issue show` | Get issue details |
| `jit_update_issue` | `jit issue update` | Update issue fields |
| `jit_claim_issue` | `jit issue claim` | Claim issue for agent |
| `jit_add_dependency` | `jit dep add` | Add dependency between issues |
| `jit_remove_dependency` | `jit dep rm` | Remove dependency |
| `jit_add_gate` | `jit gate add` | Add gate to issue |
| `jit_pass_gate` | `jit gate pass` | Mark gate as passed |
| `jit_fail_gate` | `jit gate fail` | Mark gate as failed |
| `jit_query_ready` | `jit query ready` | Get ready issues |
| `jit_query_blocked` | `jit query blocked` | Get blocked issues |
| `jit_show_graph` | `jit graph show` | Get dependency graph |
| `jit_validate` | `jit validate` | Validate repository |
| `jit_search_issues` | `jit issue search` | Search issues by text |

**Total tools**: ~15-20 tools covering all operations

---

### 2.4 Usage Comparison

#### Direct CLI Usage (Current):
```bash
# AI agent generates shell commands
$ jit issue create --title "Fix bug" --priority high --json
{"success": true, "data": {"id": "abc123", ...}}

$ jit dep add abc123 def456 --json
{"success": true, ...}
```

#### MCP Usage (Future):
```
AI Agent thinks: "I need to create an issue and add a dependency"

→ Calls tool: jit_create_issue(title="Fix bug", priority="high")
← Returns: {"id": "abc123", ...}

→ Calls tool: jit_add_dependency(from="abc123", to="def456")
← Returns: {"success": true}
```

**Key difference**: MCP provides **structured tool definitions** that AI understands natively, while CLI requires the AI to know command syntax.

---

## Recommended Implementation Order

### Phase 1: CLI Consistency (Weeks 1-2)
**Priority: CRITICAL** - Benefits all users immediately

1. **Week 1**: Universal JSON output + Structured errors (14-18 hrs)
   - ✅ Immediate value for agents
   - ✅ Foundation for MCP server
   
2. **Week 2**: Exit codes + Schema export (10-14 hrs)
   - ✅ Better automation
   - ✅ Discoverability

**Total**: 24-32 hours

### Phase 2: MCP Server (Weeks 3-4)
**Priority: HIGH** - Enhanced agent experience

3. **Week 3**: Choose implementation (TypeScript for speed)
   - Implement MCP server wrapping CLI (8-10 hrs)
   - Map 15-20 tools (6-8 hrs)
   
4. **Week 4**: Testing + Documentation
   - Integration tests (6-8 hrs)
   - Usage docs + examples (4-6 hrs)

**Total**: 24-32 hours

### Phase 3: Advanced Features (Weeks 5-6)
**Priority: MEDIUM** - Optimization

5. **Batch operations** (10-12 hrs)
6. **Streaming results** for long operations (8-10 hrs)
7. **Webhook notifications** (6-8 hrs)

**Total**: 24-30 hours

---

## Decision Matrix: CLI vs MCP

| Aspect | Enhanced CLI | MCP Server |
|--------|--------------|------------|
| **Implementation** | 24-32 hrs | 24-32 hrs |
| **Agent Support** | Any agent with shell access | MCP-compatible agents only |
| **Type Safety** | Manual (via schema export) | Built-in |
| **Discoverability** | `--schema` flag | Native MCP protocol |
| **Complexity** | Low | Medium |
| **Maintenance** | Easy | Medium |
| **Performance** | Direct execution | Shell exec overhead (TS) |
| **Error Handling** | JSON + exit codes | Structured exceptions |
| **State** | Stateless | Can maintain state |
| **Adoption** | Immediate | Requires MCP-compatible client |

---

## Recommendation: Both, Sequentially

### ✅ Start with Enhanced CLI (Phase 1)
**Why**:
1. Benefits **all** users (humans + all agents)
2. Required foundation for MCP server anyway
3. Lower risk, faster ROI
4. Works with **any** AI agent (Claude, GPT, local models)

### ✅ Add MCP Server (Phase 2)
**Why**:
1. Better experience for MCP-compatible agents
2. Reduced token usage (structured vs text commands)
3. Better error handling for agents
4. Future-proofing as MCP adoption grows

### 🎯 Combined Approach Benefits:
- **Humans**: Enhanced `--json` output for scripting
- **Simple agents**: Shell out to CLI with predictable JSON
- **Advanced agents**: Use MCP for type-safe, efficient access
- **Everyone**: Same underlying logic, consistent behavior

---

## Next Steps

### Immediate (Week 1):
1. **Create feature branch**: `git checkout -b feature/cli-consistency`
2. **Add universal `--json` support**:
   - Start with `list`, `show`, `status`
   - Write tests for JSON output format
   - Implement `JsonOutput` wrapper struct
3. **Standardize success responses**
4. **Add structured error handling**

### Week 2:
5. **Standardize exit codes**
6. **Implement `--schema` command**
7. **Document JSON API** in `docs/json-api.md`

### Week 3-4:
8. **Prototype MCP server** (TypeScript for speed)
9. **Test with Claude** (has MCP support)
10. **Document MCP usage**

---

## Questions to Decide

1. **MCP Implementation**: Rust (type-safe, more work) or TypeScript (faster, shell overhead)?
   - **Recommendation**: TypeScript initially, migrate to Rust if needed

2. **Schema versioning**: Semantic versioning for JSON API?
   - **Recommendation**: Yes, include `api_version` in responses

3. **Batch operations**: Transactional (rollback on error) or best-effort?
   - **Recommendation**: Best-effort default, `--atomic` flag for transactions

4. **Real-time updates**: WebSocket support for long-running operations?
   - **Recommendation**: Phase 4, not critical initially

5. **Backward compatibility**: Support old JSON format during transition?
   - **Recommendation**: Yes, deprecation warnings for 1-2 releases

---

## Success Criteria

### Phase 1 (CLI Consistency)
- [ ] All commands support `--json` flag
- [ ] Structured error responses with suggestions
- [ ] Standardized exit codes (documented)
- [ ] `--schema` exports complete API definition
- [ ] 40+ tests for JSON output correctness
- [ ] Documentation: `docs/json-api.md`

### Phase 2 (MCP Server)
- [ ] MCP server running on stdio transport
- [ ] 15-20 tools implemented
- [ ] Works with Claude Desktop
- [ ] Integration tests with MCP protocol
- [ ] Documentation: `docs/mcp-server.md`
- [ ] Example agent workflows

---

**Ready to start? Begin with Phase 1.1 - Universal JSON Output!**
