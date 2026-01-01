# Core Model

> **Status:** Draft - Story c8254dbf  
> **Diátaxis Type:** Explanation

## Issues

<!-- What are issues? Properties, lifecycle -->

## Dependencies vs Labels: Understanding the Difference

Dependencies and labels both organize work, but serve fundamentally different purposes. They often flow in the same direction (task → epic → milestone) but have distinct semantics.

### Labels: What Belongs Where (Grouping)

Labels provide **organizational membership** using `namespace:value` format:

```
Task: Implement Login
  label: "epic:auth"     → This task belongs to auth epic
  label: "type:task"     → This is a task-level work item
  label: "component:api" → This affects the API component
```

**Key properties:**
- Hierarchical grouping (milestone > epic > task)
- Multiple labels allowed (one per namespace if unique)
- Used for filtering and reporting
- Static relationship (doesn't change based on state)

### Dependencies: What Blocks What (Workflow)

Dependencies create **execution order** in a directed acyclic graph (DAG):

```
Epic: Auth System
  depends on: [Login Task, Password Task, Session Task]
  → Epic cannot complete until all tasks are done
```

**Key properties:**
- Arbitrary DAG structure (not limited to hierarchy)
- Determines what work is available (ready vs blocked)
- Dynamic relationship (affects state transitions)
- Transitive reduction (minimal edges preferred)

### Same Direction, Different Meanings

```
┌─────────────────────────────────────────────────────────┐
│                Task: Implement Login                    │
│                                                         │
│  Label: "epic:auth"                                    │
│  └─→ "This task belongs to auth epic" (membership)    │
│                                                         │
│  Dependency of: Epic                                    │
│  └─→ "Epic requires this task to complete" (order)    │
└─────────────────────────────────────────────────────────┘
                         ↓ flows into
┌─────────────────────────────────────────────────────────┐
│                Epic: Auth System                        │
│                                                         │
│  Label: "milestone:v1.0"                               │
│  └─→ "This epic belongs to v1.0" (membership)         │
│                                                         │
│  Dependency of: Milestone                               │
│  └─→ "Milestone requires this epic" (order)           │
└─────────────────────────────────────────────────────────┘
```

Both flow the same way, but labels organize while dependencies control workflow.

### Asymmetry: Dependencies Are More Flexible

Labels follow strict hierarchy (task → epic → milestone), but dependencies allow arbitrary DAG relationships:

```
Sequential releases (dependencies work, labels don't):

v1.0 Release (completed) ──→ blocks ──→ v2.0 Planning Task

Valid dependency: Future work waits for past release
Invalid label: v1.0 cannot "belong to" v2.0 task
```

### When to Use What

**Use Labels:**
- Organizing related work into groups
- Filtering by scope or domain
- Reporting progress by epic/milestone
- Querying specific subsets

**Use Dependencies:**
- Enforcing work order (A must complete before B)
- Blocking work until prerequisites ready
- Determining what's available to work on
- Controlling state transitions

**Use Both (Common Pattern):**
Most workflows use both for maximum clarity:

```
Task: Login Endpoint
  labels: ["epic:auth", "type:task", "component:backend"]
  dependencies: []  # No blockers, can start immediately

Epic: Auth System
  labels: ["milestone:v1.0", "type:epic"]
  dependencies: [Login Task, Password Task, Session Task]

Query by label: "epic:auth" → Shows all auth work
Query ready: → Shows Login Task (epic blocked by dependency)
```

## Dependencies

<!-- DAG model, blocking, transitive reduction -->

## Gates

<!-- Quality checkpoints, prechecks/postchecks, automated vs manual -->

## States

<!-- State machine, transitions, terminal states -->

## Labels

<!-- Namespace:value format, hierarchy, strategic vs tactical -->

## Assignees

<!-- Format, types, claim/release semantics -->
