# JIT Agent Context - MCP Mode

**Version**: 2025-12-16  
**Prerequisites**: JIT MCP server is running and connected

You are an AI agent with access to the Just-In-Time (JIT) issue tracker via MCP tools. JIT is designed for AI agents to manage complex projects with dependency graphs and quality gates.

---

## ⚡ Quick Reference Card

### MCP Parameter Names (Use These!)
```javascript
jit_issue_create({
  title: "string",
  description: "string",        // ✅ Full word (consistent with CLI)
  label: ["type:task", ...],    // Array (singular form, standard practice)
  gate: ["review", ...],        // Array (singular form, standard practice)
  priority: "high"
})
```

### Efficiency Tips
- ✅ **Use MCP tools exclusively** - don't fall back to CLI/bash
- ✅ **Use Promise.all()** for parallel operations (creating multiple issues)
- ✅ **Chain MCP calls** - structured JSON responses are easy to parse
- ✅ **Check tool names**: Use `jit_issue_create` not `jit issue create`

---

## Core Concepts

**Issues**: Work items (milestones, epics, tasks, bugs, research)  
**Dependencies**: Directed acyclic graph (DAG) - issues block other issues  
**Gates**: Quality checkpoints (tests, reviews, scans) that must pass  
**Labels**: Hierarchical organization using `namespace:value` format  
**States**: `open` → `ready` → `in_progress` → `done` (or `cancelled`)

---

## Label Format (CRITICAL)

**ALL labels MUST use**: `namespace:value`

### Required Labels
```
type:*         (REQUIRED, unique - exactly ONE per issue)
├─ type:milestone    Release/time-bound goal
├─ type:epic         Large feature
├─ type:task         Concrete work item
├─ type:research     Time-boxed investigation
└─ type:bug          Defect to fix

milestone:*    Groups work under releases (e.g., milestone:v1.0)
epic:*         Groups tasks under features (e.g., epic:auth)
component:*    Technical area (e.g., component:backend)
```

### Label Examples
```
✅ CORRECT:
  type:task, epic:auth, milestone:v1.0, component:backend

❌ WRONG:
  auth (missing namespace)
  milestone-v1.0 (wrong separator)
  Type:task (uppercase namespace)
```

---

## MCP Tool Parameter Names (CRITICAL)

**MCP tool parameters match the CLI exactly:**

| CLI Flag | MCP Parameter | Type | Notes |
|----------|---------------|------|-------|
| `--title` | `title` | string | Same |
| `--description` | `description` | string | Full word (changed from `--desc`) |
| `--label` | `label` | array | Singular form (standard practice) |
| `--gate` | `gate` | array | Singular form (standard practice) |
| `--priority` | `priority` | string | Same |

**Key points:**
- Use `description` (full word, intuitive)
- Use `label` as array (singular is standard: Docker, K8s, GitHub CLI use this)
- Use `gate` as array (singular is standard)

---

## Creating Project Structure

### Pattern: Milestone → Epics → Tasks

```javascript
// 1. Create milestone
const milestone = await use_mcp_tool("jit_issue_create", {
  title: "Release v1.0",
  description: "First production-ready release with core features",
  label: ["type:milestone", "milestone:v1.0"],
  priority: "critical"
});

// 2. Create epic under milestone
const epic = await use_mcp_tool("jit_issue_create", {
  title: "User Authentication",
  description: "JWT-based authentication with login, logout, token refresh",
  label: ["type:epic", "epic:auth", "milestone:v1.0"],
  priority: "high",
  gate: ["integration-tests", "review"]
});

// 3. Create tasks under epic
const task1 = await use_mcp_tool("jit_issue_create", {
  title: "Implement JWT utilities",
  description: "Utility functions for creating, signing, and verifying JWT tokens",
  label: ["type:task", "epic:auth", "milestone:v1.0", "component:backend"],
  priority: "high",
  gate: ["unit-tests", "review"]
});

// 4. Set up dependencies
await use_mcp_tool("jit_dep_add", {
  from_id: milestone.id,  // milestone depends on epic
  to_id: epic.id
});

await use_mcp_tool("jit_dep_add", {
  from_id: epic.id,       // epic depends on task
  to_id: task1.id
});
```

**Pro tip**: Make parallel MCP calls for independent operations:
```javascript
// Create all epics simultaneously
const [authEpic, apiEpic, uiEpic] = await Promise.all([
  use_mcp_tool("jit_issue_create", {...}),
  use_mcp_tool("jit_issue_create", {...}),
  use_mcp_tool("jit_issue_create", {...})
]);
```

---

## Common Workflows

### Initialize New Project
```javascript
// 1. Check current state
await use_mcp_tool("jit_status");

// 2. Query what exists
await use_mcp_tool("jit_query_strategic");  // Shows milestones + epics

// 3. Create structure (see pattern above)

// 4. Verify
await use_mcp_tool("jit_validate");
```

### Work on Tasks
```javascript
// 1. Find ready work
const ready = await use_mcp_tool("jit_query_ready");

// 2. Claim a task
await use_mcp_tool("jit_issue_claim", {
  id: task_id,
  assignee: "agent:your-name"
});

// 3. Pass gates as you complete work
await use_mcp_tool("jit_gate_pass", {
  id: task_id,
  gate_key: "unit-tests"
});

// 4. Mark complete
await use_mcp_tool("jit_issue_update", {
  id: task_id,
  state: "done"
});
```

### Dynamic Issue Discovery
```javascript
// While working, discover new requirement
const newTask = await use_mcp_tool("jit_issue_create", {
  title: "Add rate limiting to login",
  description: "Prevent brute force attacks - 5 attempts per minute",
  label: ["type:task", "epic:auth", "milestone:v1.0"],  // 'label' is array!
  priority: "critical",
  gate: ["unit-tests", "security-scan"]  // 'gate' is array!
});

// Add to existing epic dependencies
await use_mcp_tool("jit_dep_add", {
  from_id: epic_id,
  to_id: newTask.id
});
```

### Monitor Progress
```javascript
// Overall status
await use_mcp_tool("jit_status");

// What's blocked and why
await use_mcp_tool("jit_query_blocked");

// Dependency graph
await use_mcp_tool("jit_graph_show", { id: milestone_id });

// Recent activity
await use_mcp_tool("jit_events_tail", { limit: 20 });
```

---

## Best Practices

### Creating Issues
1. **Always include `type:*` label** (required, unique)
2. **Add membership labels** (`epic:*`, `milestone:*`) unless standalone
3. **Use gates** for quality enforcement (`unit-tests`, `review`, etc.)
4. **Set priorities** realistically (`critical` > `high` > `normal` > `low`)
5. **Add descriptions** - help other agents understand context

### Dependencies
1. **Think DAG**: Dependencies are "A needs B complete" (A depends on B)
2. **Foundation first**: Infrastructure/setup tasks have no dependencies
3. **Cross-epic deps**: Database tasks often block auth/API tasks
4. **No cycles**: System will reject circular dependencies

### Gates
1. **Register first**: Use `jit_registry_add` to define gates if not exist
2. **Auto gates**: Mark `--auto true` for CI/automated checks
3. **Manual gates**: Use for human reviews, approvals
4. **Gate before done**: Can't transition to `done` with failing gates

### Labels
1. **Discover existing**: Use `jit_label_values` to see existing values
2. **Consistent naming**: `milestone:v1.0` not `milestone:1.0`
3. **Component optional**: Only add if organizing by tech area
4. **Strategic view**: Issues with `type:milestone` or `type:epic` appear in strategic view

---

## Validation Checklist

Before considering structure complete:

```javascript
// 1. Validate repository
const validation = await use_mcp_tool("jit_validate");
// Should show: ✅ All checks passed

// 2. Check strategic view
const strategic = await use_mcp_tool("jit_query_strategic");
// Should show clean hierarchy

// 3. Verify dependencies
const graph = await use_mcp_tool("jit_graph_show", { id: milestone_id });
// Should show logical tree

// 4. Check for ready work
const ready = await use_mcp_tool("jit_query_ready");
// Should have 1-2 foundation tasks ready to start
```

---

## Available MCP Tools (Key Subset)

### Issue Management
- `jit_issue_create` - Create new issue
- `jit_issue_update` - Update issue (state, priority, labels)
- `jit_issue_list` - List all issues
- `jit_issue_show` - Get issue details
- `jit_issue_claim` - Claim issue for work
- `jit_issue_search` - Text search across issues

### Dependencies
- `jit_dep_add` - Add dependency (from depends on to)
- `jit_dep_rm` - Remove dependency

### Gates
- `jit_registry_add` - Register gate definition
- `jit_gate_add` - Add gate to issue
- `jit_gate_pass` - Mark gate passed
- `jit_gate_fail` - Mark gate failed

### Queries
- `jit_query_ready` - Find unblocked, unassigned work
- `jit_query_blocked` - Show blocked issues with reasons
- `jit_query_strategic` - Show milestones + epics only
- `jit_query_state` - Filter by state
- `jit_query_assignee` - Filter by assignee

### Graph
- `jit_graph_show` - Visualize dependency tree
- `jit_graph_roots` - Show top-level issues (no dependencies)
- `jit_graph_downstream` - Show what depends on this issue

### System
- `jit_status` - Overall project status
- `jit_validate` - Check repository health
- `jit_events_tail` - Show recent activity

---

## Common Errors & Solutions

### "Invalid parameter name"
```
Error: Unknown parameter 'labels' or 'gates'
Solution: Use correct MCP parameter names:
  - 'description' (full word, intuitive)
  - 'label' (singular, array) - standard practice
  - 'gate' (singular, array) - standard practice
```

### "Invalid label format"
```
Error: Invalid label format: 'auth'
Solution: Use namespace:value format → "epic:auth"
```

### "Cycle detected"
```
Error: Adding dependency would create cycle
Solution: Check dependency direction with jit_graph_show
```

### "Issue blocked by dependencies"
```
Error: Cannot transition to done - has blocking dependencies
Solution: Complete or remove dependencies first
```

### "Gate blocking state transition"
```
Error: Cannot mark done - gate 'unit-tests' not passed
Solution: Pass gate with jit_gate_pass or remove gate
```

### "Multiple type labels"
```
Error: Issue can only have one type:* label
Solution: Each issue needs exactly ONE type label
```

---

## Success Indicators

You've created a good structure when:

✅ `jit_validate` returns no errors  
✅ `jit_query_strategic` shows clean milestone/epic hierarchy  
✅ `jit_graph_show` displays logical dependency tree  
✅ All issues have exactly one `type:*` label  
✅ All labels use `namespace:value` format  
✅ Foundation tasks are `ready` (unblocked)  
✅ No circular dependencies exist  

---

## Example: Complete Project Initialization

```javascript
// 1. Create milestone
const m = await use_mcp_tool("jit_issue_create", {
  title: "Release v1.0",
  description: "First production-ready release with core features",
  label: ["type:milestone", "milestone:v1.0"],
  priority: "critical"
});

// 2. Create epics in parallel (efficient!)
const [authEpic, infraEpic] = await Promise.all([
  use_mcp_tool("jit_issue_create", {
    title: "User Authentication",
    description: "JWT-based auth with login, logout, token refresh",
    label: ["type:epic", "epic:auth", "milestone:v1.0"],
    priority: "high",
    gate: ["integration-tests", "review"]
  }),
  use_mcp_tool("jit_issue_create", {
    title: "Infrastructure Setup",
    description: "PostgreSQL schema, migrations, deployment config",
    label: ["type:epic", "epic:infra", "milestone:v1.0"],
    priority: "critical",
    gate: ["review", "security-scan"]
  })
]);

// 3. Create tasks in parallel (efficient!)
const [dbTask, userModelTask, loginTask] = await Promise.all([
  use_mcp_tool("jit_issue_create", {
    title: "Design PostgreSQL schema",
    description: "Define tables, relationships, indexes for all entities",
    label: ["type:task", "epic:infra", "milestone:v1.0", "component:backend"],
    priority: "critical",
    gate: ["review"]
  }),
  use_mcp_tool("jit_issue_create", {
    title: "Create User model",
    description: "SQLAlchemy model: id, email, password_hash, created_at, updated_at",
    label: ["type:task", "epic:auth", "milestone:v1.0", "component:backend"],
    priority: "high",
    gate: ["unit-tests", "review"]
  }),
  use_mcp_tool("jit_issue_create", {
    title: "Implement login endpoint",
    description: "POST /api/auth/login - accept email/password, return JWT",
    label: ["type:task", "epic:auth", "milestone:v1.0", "component:backend"],
    priority: "high",
    gate: ["unit-tests", "review"]
  })
]);

// 4. Wire dependencies in parallel (efficient!)
await Promise.all([
  use_mcp_tool("jit_dep_add", {from_id: m.id, to_id: authEpic.id}),
  use_mcp_tool("jit_dep_add", {from_id: m.id, to_id: infraEpic.id}),
  use_mcp_tool("jit_dep_add", {from_id: infraEpic.id, to_id: dbTask.id}),
  use_mcp_tool("jit_dep_add", {from_id: authEpic.id, to_id: userModelTask.id}),
  use_mcp_tool("jit_dep_add", {from_id: authEpic.id, to_id: loginTask.id}),
  use_mcp_tool("jit_dep_add", {from_id: userModelTask.id, to_id: dbTask.id}),
  use_mcp_tool("jit_dep_add", {from_id: loginTask.id, to_id: userModelTask.id})
]);

// 5. Verify
const validation = await use_mcp_tool("jit_validate");
const strategic = await use_mcp_tool("jit_query_strategic");
const ready = await use_mcp_tool("jit_query_ready");  // dbTask should be ready
```

**Key Efficiency Tips:**
- ✅ Use `Promise.all()` for independent operations (creating multiple issues)
- ✅ Use MCP tools exclusively (don't mix with CLI/bash)
- ✅ Remember parameter names: `desc`, `label`, `gate` (not `description`, `labels`, `gates`)
- ✅ Check MCP tool responses for structured data (easier to chain)

---

**Ready to manage complex projects with AI agents!** 🤖
