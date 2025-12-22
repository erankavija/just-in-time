# JIT Labels Reference

**Canonical reference for JIT's label system**

Labels provide organizational membership and classification for issues. This guide covers label format, namespaces, validation rules, and usage patterns.

**Related:** For work ordering and blocking relationships, see [Dependencies](../concepts/dependencies.md)

---

## Overview

### What Are Labels?

Labels are **namespace:value** pairs that provide:
- **Organizational membership** - Group issues by epic, milestone, component
- **Classification** - Mark issue types, priorities, status
- **Filtering** - Query and report on related work
- **Strategic planning** - Identify high-level vs tactical work

**Example:**
```bash
jit issue create --title "Add login" \
  --label type:task \
  --label epic:auth \
  --label component:backend \
  --label milestone:v1.0
```

### Labels vs Dependencies

**Labels** (membership/grouping):
- Purpose: Organization, filtering, reporting
- Relationship: "belongs to" (many-to-many)
- Query: `jit query label "epic:auth"` shows all members
- No workflow impact

**Dependencies** (work order/blocking):
- Purpose: Work sequencing, blocking relationships
- Relationship: "is required by" (directed acyclic graph)
- Query: `jit query blocked` shows blocked issues
- Blocks workflow until complete

**Example:**
```
Task: "Implement JWT"
  ├─ label "epic:auth"      → belongs to Auth epic (membership)
  └─ dependency on "Setup DB" → cannot start until DB ready (blocking)
```

Both can flow the same direction (task → epic → milestone) but serve different purposes and can be used independently.

---

## Label Format Specification

### Enforced Format

```
<namespace>:<value>

Where:
- namespace: [a-z][a-z0-9-]* (lowercase, alphanumeric, hyphens)
- value: [a-zA-Z0-9][a-zA-Z0-9._-]* (alphanumeric, dots, hyphens, underscores)
- separator: exactly one colon ':'
```

**Examples of VALID labels:**
- ✅ `milestone:v1.0`
- ✅ `epic:user-auth`
- ✅ `component:backend`
- ✅ `type:feature`
- ✅ `priority:p0`
- ✅ `team:platform-eng`

**Examples of INVALID labels:**
- ❌ `auth` (no namespace)
- ❌ `milestone-v1.0` (wrong separator)
- ❌ `Milestone:v1.0` (uppercase namespace)
- ❌ `milestone:` (empty value)
- ❌ `milestone:v1.0:extra` (multiple colons)

### Validation

```rust
use regex::Regex;

pub fn validate_label(label: &str) -> Result<(), String> {
    let re = Regex::new(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$").unwrap();
    
    if !re.is_match(label) {
        return Err(format!(
            "Invalid label format: '{}'. Expected format: 'namespace:value' \
             where namespace is lowercase alphanumeric+hyphens, \
             value is alphanumeric with dots/hyphens/underscores",
            label
        ));
    }
    
    Ok(())
}
```

**CLI enforcement:**
```bash
jit issue update <id> --label "Auth"
# Error: Invalid label format: 'Auth'. Expected: 'namespace:value'
# Did you mean: 'epic:auth' or 'component:auth'?

jit issue update <id> --label "milestone-v1.0"
# Error: Invalid label format: 'milestone-v1.0'. 
# Did you mean: 'milestone:v1.0'?
```

---

## Standard Namespaces (Registry)

### Core Namespaces (Built-in)

Store in `.jit/label-namespaces.json`:

```json
{
  "schema_version": 1,
  "namespaces": {
    "milestone": {
      "description": "Release or time-bounded goal",
      "examples": ["milestone:v1.0", "milestone:q1-2026"],
      "unique": false,
      "required_for_strategic_view": true
    },
    "epic": {
      "description": "Large feature or initiative",
      "examples": ["epic:auth", "epic:api"],
      "unique": false,
      "required_for_strategic_view": true
    },
    "component": {
      "description": "Technical area or subsystem",
      "examples": ["component:backend", "component:frontend", "component:infra"],
      "unique": false,
      "required_for_strategic_view": false
    },
    "type": {
      "description": "Work item type",
      "examples": ["type:idea", "type:research", "type:task", "type:epic", "type:milestone", "type:bug", "type:feature"],
      "unique": true,
      "required_for_strategic_view": false,
      "note": "Research tasks are time-boxed investigations (sometimes called 'spikes' in Agile contexts)"
    },
    "priority": {
      "description": "Priority level (supplements built-in priority field)",
      "examples": ["priority:p0", "priority:p1", "priority:p2"],
      "unique": true,
      "required_for_strategic_view": false
    },
    "status": {
      "description": "Additional status markers",
      "examples": ["status:needs-review", "status:blocked", "status:ready-to-merge"],
      "unique": true,
      "required_for_strategic_view": false
    },
    "team": {
      "description": "Owning team or group",
      "examples": ["team:backend", "team:frontend", "team:platform"],
      "unique": true,
      "required_for_strategic_view": false
    }
  }
}
```

### Namespace Properties

- **unique**: If true, issue can only have ONE label from this namespace
- **required_for_strategic_view**: If true, this namespace defines strategic issues

### Standard Work Item Types

**CRITICAL: Every issue MUST have exactly ONE `type:*` label.**

The `type:*` namespace defines the kind of work an issue represents:

| Type | Description | Typical Use | Time-boxed? |
|------|-------------|-------------|-------------|
| `type:idea` | Exploratory concept, not yet validated | Early-stage thoughts needing validation | No |
| `type:research` | Time-boxed investigation or feasibility study | Technical feasibility, user research, literature review | Yes (recommended) |
| `type:task` | Concrete implementation work | Coding, configuration, deployment | No |
| `type:epic` | Large, coherent body of work | Major features spanning multiple tasks | No |
| `type:milestone` | Time-bound release goal | Version releases, quarterly goals | Yes (by definition) |
| `type:bug` | Defect or error to fix | Production issues, broken functionality | No |
| `type:feature` | New functionality or enhancement | User-facing additions | No |

### Epic and Milestone Labels: Membership vs Type

**KEY DISTINCTION:**

- **`type:epic`** = "This issue IS an epic" (the work item type)
- **`epic:auth`** = "This issue belongs to the auth epic" (membership/grouping)

- **`type:milestone`** = "This issue IS a milestone" (the work item type)
- **`milestone:v1.0`** = "This issue belongs to the v1.0 milestone" (membership/grouping)

**Examples:**

```bash
# Epic issue itself
jit issue create \
  --title "User Authentication System" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0"
# type:epic = this IS an epic
# epic:auth = this epic is about auth (self-referential)
# milestone:v1.0 = this epic is part of v1.0 milestone

# Task under that epic
jit issue create \
  --title "Implement login endpoint" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend"
# type:task = this IS a task
# epic:auth = this task belongs to auth epic
# milestone:v1.0 = this task contributes to v1.0 milestone
```

**Strategic View Filtering:**
- Shows issues with `type:epic` OR `type:milestone` labels
- Alternative: Shows issues with `epic:*` OR `milestone:*` labels (catches all strategic work)
- Recommendation: Use the latter to include epics and their container milestones

**Note on "research" vs "spike":**
- `type:research` is the standard term for time-boxed investigations
- "Spike" is Agile jargon with the same meaning (used in Jira, Rally, etc.)
- Both terms acceptable in descriptions; use `type:research` for the label

**Example research task:**
```bash
jit issue create \
  --title "Research: Evaluate vector database options" \
  --description "Compare Qdrant, Milvus, pgvector. Time-box: 2 days" \
  --label "type:research" \
  --label "component:search"
```

**Validation rules:**
```rust
pub fn validate_issue_labels(issue: &Issue, registry: &NamespaceRegistry) -> Result<()> {
    let mut namespace_counts = HashMap::new();
    
    for label in &issue.labels {
        let (namespace, _value) = parse_label(label)?;
        *namespace_counts.entry(namespace).or_insert(0) += 1;
        
        // Check if namespace is registered
        if let Some(ns_def) = registry.get(&namespace) {
            // Check uniqueness constraint
            if ns_def.unique && namespace_counts[&namespace] > 1 {
                return Err(format!(
                    "Issue can only have one label from namespace '{}'. Found: {}",
                    namespace,
                    issue.labels.iter()
                        .filter(|l| l.starts_with(&format!("{}:", namespace)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }
    
    Ok(())
}
```

**CLI behavior:**
```bash
# Trying to add second unique namespace label
jit issue update <id> --label "type:feature"
jit issue update <id> --label "type:bug"
# Error: Issue already has label 'type:feature'. Namespace 'type' allows only one label.
# Use --remove-label first or --replace-label to change it.

# Correct way
jit issue update <id> --remove-label "type:feature" --label "type:bug"
# Or shortcut:
jit issue update <id> --replace-label "type:bug"
```

---

## Agent-Friendly CLI

### Autocomplete & Suggestions

```bash
# Show available namespaces
jit label namespaces
# Output:
# milestone     Release or time-bounded goal
# epic          Large feature or initiative  
# component     Technical area or subsystem
# type          Work item type
# team          Owning team or group

# Show existing values for a namespace
jit label values milestone
# Output:
# milestone:v1.0
# milestone:v2.0
# milestone:q1-2026

# Suggest labels based on issue content
jit label suggest <id>
# Analyzes title/description and suggests:
# Suggested labels:
# - component:backend (keyword: "API", "server")
# - type:feature (keyword: "implement", "add")
# - epic:auth (keyword: "authentication")
```

### Atomic Label Operations

```bash
# Add single label (idempotent)
jit issue update <id> --label "epic:auth"
# If already present, no error (idempotent)

# Add multiple labels atomically
jit issue update <id> --labels "epic:auth,component:backend,type:feature"
# All or nothing - if any invalid, none added

# Replace label in namespace
jit issue update <id> --replace-label "type:bug"
# Removes existing type:* label and adds type:bug

# Remove label
jit issue update <id> --remove-label "epic:auth"

# Remove all labels from namespace
jit issue update <id> --remove-namespace "epic"
# Removes all epic:* labels
```

---

## MCP Tool Schema

### Clear Tool Definitions for AI Agents

```typescript
// MCP tool: label_add
{
  name: "label_add",
  description: "Add labels to an issue. Labels MUST be in format 'namespace:value'",
  inputSchema: {
    type: "object",
    properties: {
      issue_id: { type: "string", description: "Issue ID" },
      labels: {
        type: "array",
        items: { type: "string" },
        description: "Array of labels in format 'namespace:value'. Examples: ['milestone:v1.0', 'epic:auth']"
      }
    },
    required: ["issue_id", "labels"]
  },
  examples: [
    {
      input: { issue_id: "01ABC", labels: ["milestone:v1.0", "component:backend"] },
      output: "Added 2 labels to issue 01ABC"
    }
  ]
}

// MCP tool: label_list_namespaces
{
  name: "label_list_namespaces",
  description: "List all available label namespaces with descriptions and examples",
  inputSchema: { type: "object", properties: {} }
}

// MCP tool: label_query
{
  name: "label_query",
  description: "Find issues by label. Supports wildcards: 'milestone:*' finds all milestones",
  inputSchema: {
    type: "object",
    properties: {
      pattern: {
        type: "string",
        description: "Label pattern. Use 'namespace:*' for all in namespace, or exact match 'namespace:value'"
      }
    },
    required: ["pattern"]
  }
}
```

### Agent Prompt Additions

In the MCP server description or system prompt:

```markdown
## Label Usage Rules

1. **Format**: Always use `namespace:value` format
   - ✅ Correct: "milestone:v1.0", "epic:auth"
   - ❌ Wrong: "auth", "milestone-v1.0"

2. **Standard Namespaces**:
   - `milestone:*` - Release goals (strategic)
   - `epic:*` - Large features (strategic)
   - `component:*` - Technical areas
   - `type:*` - Work type (unique, only one per issue)
   - `team:*` - Owning team

3. **Strategic Issues**: Issues with `milestone:*` or `epic:*` labels appear in strategic view

4. **Check Before Adding**: Use `label_list_namespaces` to see available namespaces

5. **Query Examples**:
   - All milestones: `label_query("milestone:*")`
   - Specific epic: `label_query("epic:auth")`
   - Backend work: `label_query("component:backend")`
```

---

## Namespace Management

### CLI Commands

```bash
# List all namespaces
jit label namespaces list
# Or: jit label ns list

# Show namespace details
jit label namespace show milestone
# Output:
# Namespace: milestone
# Description: Release or time-bounded goal
# Unique: false (issues can have multiple)
# Strategic: true (appears in strategic view)
# Examples: milestone:v1.0, milestone:q1-2026
# Current values in use:
# - milestone:v1.0 (12 issues)
# - milestone:v2.0 (3 issues)

# Add custom namespace
jit label namespace add platform \
  --description "Platform-specific labels" \
  --examples "platform:web,platform:mobile" \
  --unique

# Remove namespace (must have no issues using it)
jit label namespace remove old-namespace
```

### Migration Helper

```bash
# Find issues with malformed labels
jit label audit
# Output:
# Found 5 issues with malformed labels:
# Issue 01ABC: "auth" (missing namespace)
#   Suggestion: Remove and add "epic:auth" or "component:auth"?
# Issue 02DEF: "milestone-v1.0" (wrong format)
#   Suggestion: Remove and add "milestone:v1.0"?

# Auto-fix with prompts
jit label fix --interactive
```

---

## Disambiguation Rules

### 1. Namespace Conflicts

**Problem**: `epic:backend` vs `component:backend` - which is correct?

**Solution**: Namespace defines meaning
- `epic:backend` = Epic-level initiative to build backend
- `component:backend` = Task is in backend area

Both can coexist:
```bash
jit issue create \
  --title "Backend Infrastructure Epic" \
  --label "epic:backend" \
  --label "component:infra"
```

### 2. Value Conflicts

**Problem**: `milestone:v1.0` vs `milestone:1.0` - same milestone?

**Solution**: Exact string match
- These are DIFFERENT milestones
- Convention: Use consistent naming (recommend `v1.0` format)
- Tool can warn about similar values:

```bash
jit issue update <id> --label "milestone:1.0"
# Warning: Similar milestone exists: "milestone:v1.0"
# Did you mean that one? (y/n)
```

### 3. Case Sensitivity

**Solution**: Namespaces are lowercase-only (enforced)
Values are case-sensitive:

```bash
jit issue update <id> --label "Epic:auth"
# Error: Namespace must be lowercase. Did you mean "epic:auth"?

jit issue update <id> --label "epic:Auth"
# OK - value can be mixed case
```

### 4. Label Discovery

**Problem**: Agent doesn't know what labels exist

**Solution**: Provide discovery tools
```bash
# What milestones exist?
jit label values milestone
# Output: v1.0, v2.0, q1-2026

# What epics exist?
jit label values epic
# Output: auth, api, dashboard

# What labels does this issue have?
jit issue show <id> --json | jq '.labels'
```

---

## Agent Workflow Examples

### Example 1: Create Epic with Tasks

```bash
# 1. Agent checks available namespaces
jit label namespaces list

# 2. Creates epic issue
EPIC=$(jit issue create \
  --title "User Authentication System" \
  --priority high \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --json | jq -r '.id')

# 3. Creates tasks under epic (labels inherited if using breakdown)
jit issue breakdown $EPIC
# Editor opens, agent adds:
# - JWT token implementation
# - OAuth provider integration
# - Password reset flow

# All subtasks automatically get:
# - epic:auth (inherited)
# - milestone:v1.0 (inherited)

# 4. Agent can add component labels to tasks
TASK_IDS=$(jit issue list --json | jq -r '.[] | select(.dependencies[] == "'$EPIC'") | .id')
for task in $TASK_IDS; do
  jit issue update $task --label "component:backend"
done
```

### Example 2: Query Strategic View

```bash
# Agent wants to see high-level progress
jit query label "milestone:*" --or label "epic:*"
# Returns all strategic issues

# Check milestone progress
MILESTONE_ID=$(jit query label "milestone:v1.0" --json | jq -r '.[0].id')
jit graph downstream $MILESTONE_ID --json | \
  jq '[.[] | .state] | group_by(.) | map({state: .[0], count: length})'
# Returns: {"state":"done","count":12}, {"state":"in_progress","count":5}, ...
```

### Example 3: Error Handling

```bash
# Agent tries malformed label
jit issue update <id> --label "backend"
# Error with clear guidance:
# Invalid label format: 'backend'. Expected: 'namespace:value'
# Available namespaces: milestone, epic, component, type, team
# Did you mean: 'component:backend' or 'epic:backend'?

# Agent corrects
jit issue update <id> --label "component:backend"
# Success

# Agent tries duplicate unique label
jit issue update <id> --label "type:bug"
# Error: Issue already has label 'type:feature'. 
# Namespace 'type' allows only one label.
# To replace: jit issue update <id> --replace-label "type:bug"

# Agent corrects
jit issue update <id> --replace-label "type:bug"
# Success
```

---

## Validation Integration

### Pre-commit Validation

```bash
# In .git/hooks/pre-commit
jit label audit --check
# Exit code 0: all labels valid
# Exit code 1: malformed labels found
```

### CI Validation

```yaml
# .github/workflows/validate.yml
- name: Validate labels
  run: |
    jit label audit
    if [ $? -ne 0 ]; then
      echo "Found invalid labels. Run: jit label fix --interactive"
      exit 1
    fi
```

---

## JSON Schema for Agents

Provide machine-readable schema:

```bash
jit label schema --json
```

Output:
```json
{
  "version": 1,
  "format": {
    "pattern": "^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$",
    "examples": ["milestone:v1.0", "epic:auth", "component:backend"]
  },
  "namespaces": {
    "milestone": {
      "description": "Release or time-bounded goal",
      "unique": false,
      "strategic": true,
      "values_in_use": ["v1.0", "v2.0", "q1-2026"]
    },
    "epic": {
      "description": "Large feature or initiative",
      "unique": false,
      "strategic": true,
      "values_in_use": ["auth", "api", "dashboard"]
    }
  }
}
```

Agents can parse this to:
- Validate format before sending
- Discover available namespaces
- See existing values
- Understand strategic labels

---

## Configuration

Labels are configured in `.jit/config.toml` under the `[type_hierarchy]` section:

```toml
[type_hierarchy]
# Type name to hierarchy level mapping (lower numbers = more strategic)
types = { milestone = 1, epic = 2, story = 3, task = 4 }

# List of type names that are considered strategic (for query strategic)
strategic_types = ["milestone", "epic"]

# Type name to membership label namespace mapping
[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"
story = "story"
```

**Customization examples:**

### Minimal 2-Level Hierarchy
```toml
[type_hierarchy]
types = { epic = 1, task = 2 }

[type_hierarchy.label_associations]
epic = "epic"
```

### Extended 5-Level Hierarchy
```toml
[type_hierarchy]
types = { program = 1, milestone = 2, epic = 3, story = 4, task = 5 }
strategic_types = ["program", "milestone", "epic"]

[type_hierarchy.label_associations]
program = "program"
milestone = "milestone"
epic = "epic"
story = "story"
```

### Custom Naming (Theme Instead of Epic)
```toml
[type_hierarchy]
types = { release = 1, theme = 2, task = 3 }
strategic_types = ["release", "theme"]

[type_hierarchy.label_associations]
theme = "epic"        # Map theme type to epic namespace
release = "milestone" # Map release type to milestone namespace
```

**See also:** [docs/reference/example-config.toml](example-config.toml) for complete configuration examples.

---

## Quick Reference

### The Golden Rules

**Rule 1: Every Issue MUST Have a Type**
```bash
# ❌ WRONG - No type label
jit issue create --title "Login API" --label "epic:auth"

# ✅ CORRECT - Has type label
jit issue create --title "Login API" --label "type:task" --label "epic:auth"
```

**Rule 2: Type vs Membership Labels**

| Label | Meaning | Answers |
|-------|---------|---------|
| `type:*` | **What it IS** | "What kind of work item?" |
| `epic:*` | **What it BELONGS TO** | "Which epic does this contribute to?" |
| `milestone:*` | **What it BELONGS TO** | "Which release is this part of?" |

### Common Patterns

**Creating an Epic:**
```bash
jit issue create \
  --title "User Authentication System" \
  --label "type:epic" \         # This IS an epic
  --label "epic:auth" \          # This epic is about auth (group ID)
  --label "milestone:v1.0"       # This epic is part of v1.0
```

Why both `type:epic` and `epic:auth`?
- `type:epic` = Tells you what it **is** (the type)
- `epic:auth` = Creates a **group identifier** for child tasks to reference
- Child tasks use `epic:auth` to show membership

**Creating Tasks Under an Epic:**
```bash
jit issue create \
  --title "Implement JWT validation" \
  --label "type:task" \          # This IS a task
  --label "epic:auth" \           # Belongs to auth epic
  --label "milestone:v1.0" \      # Belongs to v1.0 milestone
  --label "component:backend"     # Additional metadata
```

**Creating a Milestone:**
```bash
jit issue create \
  --title "Release v1.0" \
  --label "type:milestone" \     # This IS a milestone
  --label "milestone:v1.0"       # Self-referential group ID
```

### Namespace Reference Table

**Required on Every Issue:**

| Namespace | Unique? | Examples | Purpose |
|-----------|---------|----------|---------|
| `type:*` | ✅ Yes | `type:task`, `type:epic`, `type:milestone` | Defines what the issue IS |

**Optional Strategic Labels:**

| Namespace | Unique? | Examples | Purpose |
|-----------|---------|----------|---------|
| `epic:*` | ❌ No | `epic:auth`, `epic:billing` | Groups work under an epic |
| `milestone:*` | ❌ No | `milestone:v1.0`, `milestone:q1-2026` | Groups work in a release |

**Optional Metadata Labels:**

| Namespace | Unique? | Examples | Purpose |
|-----------|---------|----------|---------|
| `component:*` | ❌ No | `component:backend`, `component:frontend` | Technical area |
| `team:*` | ✅ Yes | `team:platform`, `team:api` | Owning team |
| `priority:*` | ✅ Yes | `priority:p0`, `priority:p1` | Priority level |
| `status:*` | ✅ Yes | `status:needs-review`, `status:blocked` | Additional status markers |

### DO's and DON'Ts

**DO:**
- ✅ Use `type:task` + `epic:auth` for tasks
- ✅ Use `type:epic` + `epic:auth` + `milestone:v1.0` for epics
- ✅ Query by membership: `jit query label "epic:auth"`
- ✅ Use lowercase for namespaces

**DON'T:**
- ❌ Skip the `type:` label
- ❌ Use uppercase in namespaces (`Type:task`)
- ❌ Use hyphens instead of colons (`epic-auth`)
- ❌ Create freeform labels without namespaces

---

## Summary: Making Labels Unambiguous

### 1. Enforce Format
- Regex validation: `namespace:value`
- Clear error messages with suggestions
- No freeform labels accepted

### 2. Namespace Registry
- Predefined standard namespaces
- Properties: description, uniqueness, strategic flag
- Extensible for custom namespaces

### 3. Agent-Friendly Tools
- Discovery: `label namespaces`, `label values`
- Validation: Clear errors before write
- Suggestions: `label suggest`
- JSON schema: Machine-readable rules

### 4. Atomic Operations
- Idempotent add
- Replace for unique namespaces
- Batch operations (all-or-nothing)

### 5. MCP Integration
- Explicit tool schemas
- Examples in tool definitions
- Prompt guidance on usage
- Error feedback loop

**Result**: Agents can reliably use labels without human intervention, with clear feedback when mistakes happen.
