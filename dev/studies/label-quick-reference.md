# Label Quick Reference

**Updated**: 2025-12-10

## The Golden Rules

### Rule 1: Every Issue MUST Have a Type
```bash
# ❌ WRONG - No type label
jit issue create --title "Login API" --label "epic:auth"

# ✅ CORRECT - Has type label
jit issue create --title "Login API" --label "type:task" --label "epic:auth"
```

### Rule 2: Type vs Membership Labels

| Label | Meaning | Answers |
|-------|---------|---------|
| `type:*` | **What it IS** | "What kind of work item?" |
| `epic:*` | **What it BELONGS TO** | "Which epic does this contribute to?" |
| `milestone:*` | **What it BELONGS TO** | "Which release is this part of?" |

## Common Patterns

### Creating an Epic

```bash
jit issue create \
  --title "User Authentication System" \
  --label "type:epic" \         # This IS an epic
  --label "epic:auth" \          # This epic is about auth
  --label "milestone:v1.0"       # This epic is part of v1.0
```

**Why both `type:epic` and `epic:auth`?**
- `type:epic` = Tells you what it **is**
- `epic:auth` = Creates a **group identifier** for child tasks
- Child tasks reference `epic:auth` to show membership

### Creating Tasks Under an Epic

```bash
jit issue create \
  --title "Implement JWT validation" \
  --label "type:task" \          # This IS a task
  --label "epic:auth" \           # Belongs to auth epic
  --label "milestone:v1.0" \      # Belongs to v1.0 milestone
  --label "component:backend"     # Additional metadata
```

### Creating a Milestone

```bash
jit issue create \
  --title "Release v1.0" \
  --label "type:milestone" \     # This IS a milestone
  --label "milestone:v1.0"       # Self-referential group ID
```

## Label Namespace Reference

### Required on Every Issue

| Namespace | Unique? | Examples | Purpose |
|-----------|---------|----------|---------|
| `type:*` | ✅ Yes | `type:task`, `type:epic`, `type:milestone` | Defines what the issue IS |

### Optional Strategic Labels

| Namespace | Unique? | Examples | Purpose |
|-----------|---------|----------|---------|
| `epic:*` | ❌ No | `epic:auth`, `epic:billing` | Groups work under an epic |
| `milestone:*` | ❌ No | `milestone:v1.0`, `milestone:q1-2026` | Groups work in a release |

### Optional Metadata Labels

| Namespace | Unique? | Examples | Purpose |
|-----------|---------|----------|---------|
| `component:*` | ❌ No | `component:backend`, `component:frontend` | Technical area |
| `team:*` | ✅ Yes | `team:platform`, `team:api` | Owning team |

## View Behavior

### Strategic View (🎯)
**Shows**: Issues with `epic:*` OR `milestone:*` labels  
**Hides**: Regular tasks without epic/milestone membership  
**Purpose**: High-level planning view

```bash
# Visible in strategic view:
- type:milestone + milestone:v1.0
- type:epic + epic:auth + milestone:v1.0
- type:epic + epic:billing + milestone:v1.0

# Hidden in strategic view:
- type:task + epic:auth + milestone:v1.0  (no epic:* label at top level)
```

### Tactical View (📋)
**Shows**: All issues  
**Purpose**: Detailed execution view

## Label Filtering Examples

### Filter by Type
```bash
# Show only epics
Filter: type:epic

# Show only tasks
Filter: type:task

# Show strategic items
Filter: type:epic OR type:milestone
```

### Filter by Membership
```bash
# Show auth epic work
Filter: epic:auth

# Show v1.0 milestone work
Filter: milestone:v1.0

# Show auth work in v1.0
Filter: epic:auth AND milestone:v1.0
```

### Filter by Component
```bash
# Show backend work
Filter: component:backend

# Show backend tasks
Filter: type:task AND component:backend
```

## Common Mistakes

### Mistake 1: Missing Type Label

```bash
# ❌ WRONG
jit issue create --title "API Gateway" --label "epic:api"
# Result: No type - what IS this issue?

# ✅ CORRECT
jit issue create --title "API Gateway" \
  --label "type:epic" \
  --label "epic:api"
```

### Mistake 2: Using Epic Label for Type

```bash
# ❌ WRONG - epic:auth doesn't tell us what it IS
jit issue create --title "User Auth" --label "epic:auth"

# ✅ CORRECT - Clearly states it's an epic
jit issue create --title "User Auth" \
  --label "type:epic" \
  --label "epic:auth"
```

### Mistake 3: Duplicate Type Labels

```bash
# ❌ WRONG - type namespace allows only ONE label
jit issue update <id> --label "type:task" --label "type:bug"
# Error: Namespace 'type' allows only one label

# ✅ CORRECT - Replace the type
jit issue update <id> --replace-label "type:bug"
```

### Mistake 4: Task Without Epic/Milestone

```bash
# ⚠️  ALLOWED but not recommended
jit issue create --title "Fix typo" --label "type:task"
# Result: Task exists but not grouped under any epic/milestone

# ✅ BETTER - Associate with strategic work
jit issue create --title "Fix auth typo" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0"
```

## Validation

### Check Your Labels

```bash
# Validate all issues have proper labels
jit validate

# Check specific issue
jit issue show <id> --json | jq '.labels'
```

### Expected Output

Every issue should have **at minimum**:
```json
{
  "labels": [
    "type:task"  // or type:epic, type:milestone, etc.
  ]
}
```

Ideally also include:
```json
{
  "labels": [
    "type:task",
    "epic:auth",
    "milestone:v1.0",
    "component:backend"
  ]
}
```

## Agent Checklist

When creating an issue, ask:

1. ✅ **What IS this issue?** → Add `type:*` label
2. ✅ **Which epic does it belong to?** → Add `epic:*` label (if applicable)
3. ✅ **Which milestone is it part of?** → Add `milestone:*` label (if applicable)
4. ✅ **What component/area?** → Add `component:*` label (if applicable)
5. ✅ **Which team owns it?** → Add `team:*` label (if applicable)

## TL;DR

- **`type:*`** = What it **IS** (required, unique)
- **`epic:*`** = What epic it **BELONGS TO** (optional, multiple allowed)
- **`milestone:*`** = What release it **BELONGS TO** (optional, multiple allowed)
- **Strategic view** = Filter by `epic:*` or `milestone:*` presence
- **Always add `type:*`** to every issue!

---

## ⚠️ Labels vs Dependencies: Parallel but Orthogonal

### Same Direction, Different Purposes

Labels (membership) and dependencies (work order) naturally align:

```
Task: Login
  ├─ label "epic:auth" → belongs to Epic: Auth
  └─ dependency of Epic → required by Epic: Auth
  
Epic: Auth
  ├─ label "milestone:v1.0" → belongs to Milestone: v1.0
  └─ dependency of Milestone → required by Milestone: v1.0

Both flow: Task → Epic → Milestone
```

### What They Mean

**Labels** = Organizational membership
- "This task is part of the auth epic"
- Purpose: Grouping, filtering, reporting
- Query: `jit query label "epic:auth"`

**Dependencies** = Work requirements  
- "The auth epic requires this task to complete"
- Purpose: Workflow control, blocking, state transitions
- Query: `jit query ready` (unblocked work)

### Key: They're Independent

✅ **Labels without dependencies** (just grouping)
```bash
jit issue create --title "Login" --label "type:task" --label "epic:auth"
jit issue create --title "Logout" --label "type:task" --label "epic:auth"
# No dependencies → Both tasks ready immediately
# Still grouped by epic:auth for queries
```

✅ **Dependencies without matching labels** (just ordering)
```bash
jit issue create --title "Deploy v1.0" --label "type:task" --label "milestone:v2.0"
jit issue create --title "v1.0 Release" --label "type:milestone" --label "milestone:v1.0"
jit dep add DEPLOY V1_RELEASE
# Deploy (v2.0 work) depends on v1.0 Release completing
# Different organizational scopes, but clear work order
```

✅ **Both together** (recommended for most workflows)
```bash
# Create epic and tasks with matching labels
EPIC=$(jit issue create --title "Auth Epic" --label "type:epic" --label "epic:auth" | awk '{print $NF}')
TASK=$(jit issue create --title "Login Task" --label "type:task" --label "epic:auth" | awk '{print $NF}')

# Add dependency for workflow control
jit dep add $EPIC $TASK
# Now: Labels group them + Dependencies control workflow
```

### Asymmetry: Dependencies are More Flexible

**Membership (labels)**: Hierarchical
- Task belongs to Epic ✅
- Epic belongs to Milestone ✅
- Milestone belongs to Task ❌ (doesn't make sense)

**Work order (dependencies)**: Arbitrary DAG
- Task is required by Epic ✅
- Epic is required by Milestone ✅
- Milestone is required by Future Task ✅ (sequential releases!)

**Example**:
```bash
# v2.0 planning task depends on v1.0 release milestone
jit dep add V2_PLANNING_TASK V1_RELEASE_MILESTONE
# Valid: Future work waits for past milestone
# But: V1 milestone cannot "belong to" v2.0 task (labels don't work this way)
```

### Research Workflow Example

```bash
# Research paper
PAPER=$(jit issue create --title "Survey Paper: Vector DBs" \
  --label "type:paper" --label "paper:vector-survey")

# Literature reviews
LIT1=$(jit issue create --title "Review: Qdrant" \
  --label "type:research" --label "paper:vector-survey")
LIT2=$(jit issue create --title "Review: Milvus" \
  --label "type:research" --label "paper:vector-survey")

# Paper depends on reviews
jit dep add $PAPER $LIT1
jit dep add $PAPER $LIT2

# Labels: All part of vector-survey paper (grouping)
# Dependencies: Paper writing waits for reviews (workflow)
```

**See also**: `docs/dependency-vs-labels-clarity.md` for complete explanation
