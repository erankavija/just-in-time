# Labels vs Dependencies: Understanding the Relationship

**Date**: 2025-12-15  
**Status**: Canonical reference  

---

## TL;DR

- **Labels**: Organizational membership ("belongs to")
- **Dependencies**: Work requirements ("is required by")
- **Direction**: Both flow the same way (task → epic → milestone)
- **Orthogonality**: Serve different purposes, can be used independently
- **Natural alignment**: When used together, they reinforce the same structure

---

## The Two Structures

### 1. Label Hierarchy (Membership/Grouping)

```
Task: Implement JWT
  └─ label "epic:auth" → belongs to Epic: Auth

Epic: Auth  
  └─ label "milestone:v1.0" → belongs to Milestone: v1.0

Flow: Task → Epic → Milestone (membership)
```

**Purpose**: Organization, filtering, reporting  
**Query**: `jit query label "epic:auth"` shows all members  
**Properties**:
- Hierarchical (smaller belongs to larger)
- Multiple issues can share same label
- No workflow impact

### 2. Dependency DAG (Work Order/Blocking)

```
Task: Implement JWT
  └─ is required by → Epic: Auth (jit dep add EPIC TASK)

Epic: Auth
  └─ is required by → Milestone: v1.0 (jit dep add MILESTONE EPIC)

Flow: Task → Epic → Milestone (work order)
```

**Purpose**: Workflow control, state transitions  
**Query**: `jit query ready` shows unblocked work  
**Properties**:
- DAG (can be any acyclic structure)
- Controls issue state (backlog → ready → done)
- Determines what can be worked on

---

## Same Direction, Different Meanings

Both structures flow in the **same direction**:

| From | To | Label Meaning | Dependency Meaning |
|------|-----|---------------|-------------------|
| Task | Epic | Task belongs to Epic | Epic requires Task |
| Epic | Milestone | Epic belongs to Milestone | Milestone requires Epic |

**This alignment is natural and intuitive when building hierarchies.**

---

## Orthogonality Examples

### Example 1: Labels Only (No Dependencies)

```bash
# Create epic and tasks with labels
jit issue create --title "Auth Epic" --label "type:epic" --label "epic:auth"
jit issue create --title "Login" --label "type:task" --label "epic:auth"
jit issue create --title "Logout" --label "type:task" --label "epic:auth"

# NO dependencies added
```

**Result**:
- ✅ All issues grouped by `epic:auth` label
- ✅ All issues immediately ready (no blockers)
- ✅ Tasks can be completed in any order
- ✅ Epic can be completed independently of tasks

**Use case**: Tracking related work without enforcing order

---

### Example 2: Dependencies Only (No Matching Labels)

```bash
# Backend and Frontend tasks (different components)
jit issue create --title "API" --label "type:task" --label "component:backend"
jit issue create --title "UI" --label "type:task" --label "component:frontend"

# Add dependency
jit dep add UI_ID API_ID  # UI depends on API
```

**Result**:
- ✅ No shared organizational labels
- ✅ UI is blocked until API is done
- ✅ Clear work order enforced
- ✅ Can query by component separately

**Use case**: Cross-cutting technical dependencies

---

### Example 3: Both Together (Recommended Pattern)

```bash
# Create with labels
EPIC=$(jit issue create --title "Auth" --label "type:epic" --label "epic:auth" | awk '{print $NF}')
TASK1=$(jit issue create --title "Login" --label "type:task" --label "epic:auth" | awk '{print $NF}')
TASK2=$(jit issue create --title "Logout" --label "type:task" --label "epic:auth" | awk '{print $NF}')

# Add dependencies
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2
```

**Result**:
- ✅ Labels: All grouped under `epic:auth`
- ✅ Dependencies: Epic blocked until tasks done
- ✅ `jit query label "epic:*"` shows all 3 issues
- ✅ `jit query ready` shows only tasks (epic blocked)

**Use case**: Most common workflow pattern

---

## Asymmetry: Dependencies Are More Flexible

### Membership (Labels): Hierarchical Only

```
✅ Task belongs to Epic (makes sense)
✅ Epic belongs to Milestone (makes sense)
❌ Milestone belongs to Task (nonsensical)
```

### Work Order (Dependencies): Arbitrary DAG

```
✅ Task required by Epic (common)
✅ Epic required by Milestone (common)
✅ Future task required by Past milestone (valid!)
```

**Example**: Sequential releases
```bash
# v1.0 Release milestone
V1=$(jit issue create --title "v1.0 Release" --label "type:milestone" --label "milestone:v1.0")

# v2.0 Planning task (part of v2.0)
V2_PLAN=$(jit issue create --title "v2.0 Planning" --label "type:task" --label "milestone:v2.0")

# v2.0 planning depends on v1.0 completing
jit dep add $V2_PLAN $V1

# Valid: Future work (v2.0) waits for past release (v1.0)
# But: v1.0 cannot "belong to" v2.0 task (labels wouldn't make sense)
```

---

## Beyond Task/Epic/Milestone

The pattern applies to **any workflow**, not just software development.

### Research Workflow Example

```bash
# Research hypothesis
HYPO=$(jit issue create --title "Vector search improves quality" \
  --label "type:hypothesis" --label "project:semantic-search")

# Investigation task
RESEARCH=$(jit issue create --title "Evaluate vector DBs" \
  --label "type:research" --label "project:semantic-search")

# Hypothesis depends on research completing
jit dep add $HYPO $RESEARCH

# Labels: Both part of semantic-search project
# Dependencies: Hypothesis resolution waits for research
```

### Infrastructure Workflow Example

```bash
# Infrastructure epic
INFRA=$(jit issue create --title "Kubernetes Migration" \
  --label "type:epic" --label "epic:k8s-migration")

# Operations tasks
SETUP=$(jit issue create --title "Setup cluster" \
  --label "type:ops" --label "epic:k8s-migration")
MIGRATE=$(jit issue create --title "Migrate services" \
  --label "type:ops" --label "epic:k8s-migration")

# Dependencies: Migration waits for setup
jit dep add $MIGRATE $SETUP
jit dep add $INFRA $MIGRATE

# Labels: All part of k8s-migration epic
# Dependencies: Clear execution order
```

---

## Command Syntax Clarification

### `jit dep add FROM TO`

**Meaning**: "FROM depends on TO" or "FROM requires TO"

```bash
jit dep add EPIC TASK
# Epic depends on Task
# Epic is blocked until Task completes
# Task is upstream, Epic is downstream
# Work flows: Task → Epic
```

**Mental model**: "Add to FROM's dependency list: TO"

### `jit graph downstream ISSUE`

**Meaning**: "Show issues that depend on ISSUE" (dependents)

```bash
jit graph downstream TASK
# Shows: Epic, Milestone (they depend on TASK)
# Work flows FROM task INTO these issues
# TASK is upstream, results are downstream
```

**"Downstream" = direction work flows toward completion**

---

## Query Patterns

### Organizational View (Labels)

```bash
# Find all work in an epic
jit query label "epic:auth"

# Find all work in a milestone
jit query label "milestone:v1.0"

# Find strategic issues (epic/milestone labels)
jit query strategic

# Find by component
jit query label "component:backend"
```

### Workflow View (Dependencies)

```bash
# Find work that's ready to start
jit query ready

# Find blocked work
jit query blocked

# Find what depends on this issue
jit graph downstream ISSUE_ID

# Show full dependency tree
jit graph show
```

---

## Summary

**Key Takeaways**:

1. **Parallel structure**: Labels and dependencies flow the same direction
2. **Different purposes**: Grouping vs workflow control
3. **Orthogonal**: Can use independently or together
4. **Natural alignment**: When combined, they reinforce the same hierarchy
5. **Asymmetry**: Dependencies are more flexible than membership
6. **Not just task/epic**: Pattern works for any workflow type

**Best Practice**: Use both together for most workflows
- Labels for organization and reporting
- Dependencies for workflow control and state management
