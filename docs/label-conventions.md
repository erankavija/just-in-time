# Label Conventions & Agent Usage

**Date**: 2025-12-07  
**Status**: Design specification  
**Goal**: Unambiguous label system that AI agents can use reliably

---

## Problem Statement

Labels can be ambiguous if not well-designed:
- `"auth"` vs `"epic:auth"` - which is correct?
- `"milestone:v1.0"` vs `"milestone-v1.0"` - format confusion
- `"backend"` vs `"component:backend"` - category unclear
- Multiple agents creating inconsistent labels

**Solution**: Enforce structure and provide clear rules.

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
      "examples": ["type:feature", "type:bug", "type:tech-debt", "type:docs"],
      "unique": true,
      "required_for_strategic_view": false
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
