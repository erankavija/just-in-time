# Visual Guide: Labels and Dependencies

**Date**: 2025-12-15  
**Purpose**: Visual representations of how labels and dependencies work together

---

## Same Direction, Different Meanings

```
┌─────────────────────────────────────────────────────────────────┐
│                    Task: Implement Login                        │
│                                                                 │
│  Label: "epic:auth"                                            │
│  └─→ "This task belongs to auth epic" (membership)            │
│                                                                 │
│  Dependency of: Epic                                            │
│  └─→ "Epic requires this task to complete" (work order)       │
└─────────────────────────────────────────────────────────────────┘
                            ↓ flows into
┌─────────────────────────────────────────────────────────────────┐
│                    Epic: Auth System                            │
│                                                                 │
│  Label: "milestone:v1.0"                                       │
│  └─→ "This epic belongs to v1.0 milestone" (membership)       │
│                                                                 │
│  Dependency of: Milestone                                       │
│  └─→ "Milestone requires this epic to complete" (work order)  │
└─────────────────────────────────────────────────────────────────┘
                            ↓ flows into
┌─────────────────────────────────────────────────────────────────┐
│                    Milestone: v1.0 Release                      │
└─────────────────────────────────────────────────────────────────┘

Both flow: Task → Epic → Milestone (same direction)
But serve different purposes: grouping vs workflow
```

---

## Asymmetry: Dependencies Are More Flexible

```
Labels (Membership):        Dependencies (Work Order):
Hierarchical only           Arbitrary DAG

Task                        Task
  ↓ belongs to               ↓ required by
Epic                        Epic
  ↓ belongs to               ↓ required by  
Milestone                   Milestone
  ❌ cannot belong to         ✅ CAN be required by
Future Task                 Future Task (sequential releases!)
```

**Example: Sequential Releases**

```
┌─────────────────────────────────────────────────────────────────┐
│         v1.0 Release Milestone (completed)                      │
│         label: "milestone:v1.0"                                 │
└─────────────────────────────────────────────────────────────────┘
                            ↓ required by
┌─────────────────────────────────────────────────────────────────┐
│         v2.0 Planning Task                                      │
│         label: "milestone:v2.0"                                 │
│                                                                 │
│   Valid dependency: Future work waits for past release         │
│   Invalid label: v1.0 cannot "belong to" v2.0 task            │
└─────────────────────────────────────────────────────────────────┘
```

---

## Research Workflow Example

```
┌──────────────────────────────────────────────────────────────┐
│              Paper: Survey on Vector Databases               │
│                                                              │
│  Label: "paper:vector-survey"                               │
│  Type: type:paper (deliverable)                             │
│  Dependency of: (none - final deliverable)                   │
│                                                              │
│  ↑ depends on (blocked by)                                  │
└──────────────────────────────────────────────────────────────┘
                            │
            ┌───────────────┼───────────────┐
            │               │               │
┌───────────▼──────┐ ┌─────▼──────┐ ┌─────▼──────┐
│ Review: Qdrant   │ │ Review:    │ │ Review:    │
│                  │ │ Milvus     │ │ pgvector   │
│                  │ │            │ │            │
│ label:           │ │ label:     │ │ label:     │
│ paper:vector-    │ │ paper:     │ │ paper:     │
│ survey           │ │ vector-    │ │ vector-    │
│                  │ │ survey     │ │ survey     │
│ type:research    │ │ type:      │ │ type:      │
│                  │ │ research   │ │ research   │
└──────────────────┘ └────────────┘ └────────────┘

Labels: All share "paper:vector-survey" (grouped)
Dependencies: Clear execution order (reviews → paper)
Query by label: Shows all 4 issues
Query ready: Shows only reviews (paper blocked)
```

---

## Complete Software Development Example

```
Organizational Hierarchy (Labels):
═══════════════════════════════════

Milestone: v1.0
├─ Epic: Auth System
│  ├─ Task: Login endpoint
│  ├─ Task: Password hashing
│  └─ Task: Session management
│
└─ Epic: User Profile
   ├─ Task: Profile CRUD API
   └─ Task: Avatar upload

Query: jit query label "milestone:v1.0"  → All 7 issues
Query: jit query label "epic:auth"       → Epic + 3 tasks

Work Flow (Dependencies):
═════════════════════════

Milestone: v1.0                 (blocked by both epics)
       ↑                              ↑
       │                              │
       │                              │
Epic: Auth              Epic: User Profile
       ↑                              ↑
       │                              │
   ┌───┴───┬────────┐            ┌────┴────┐
   │       │        │            │         │
Login  Password  Session    Profile    Avatar
Task    Task     Task       Task       Task

Query: jit query ready    → 5 tasks (epics + milestone blocked)
Query: jit query blocked  → 2 epics + 1 milestone
```

---

## Cross-Cutting Dependencies (No Shared Labels)

```
Backend Task                  Frontend Task
label: component:backend      label: component:frontend
     │                              ↑
     │    jit dep add FRONTEND BACKEND
     └──────────────────────────────┘

Different organizational scopes (backend vs frontend)
But clear work order (frontend waits for backend API)

Query: jit query label "component:backend"   → Backend only
Query: jit query label "component:frontend"  → Frontend only
Query: jit query ready                       → Backend only (frontend blocked)
```

---

## Infrastructure Workflow Example

```
┌─────────────────────────────────────────────────────────────────┐
│            Epic: Kubernetes Migration                           │
│            label: "epic:k8s-migration"                          │
│                                                                 │
│  ↑ depends on                                                   │
└─────────────────────────────────────────────────────────────────┘
                            │
            ┌───────────────┴───────────────┐
            │                               │
┌───────────▼──────────┐     ┌──────────────▼──────────┐
│ Setup Cluster        │     │ Migrate Services        │
│ label: epic:k8s-     │     │ label: epic:k8s-        │
│        migration     │     │        migration        │
│ type: ops            │     │ type: ops               │
│                      │────→│                         │
│                      │     │ (depends on setup)      │
└──────────────────────┘     └─────────────────────────┘

Labels: All part of k8s-migration epic
Dependencies: Clear execution order (setup → migrate → epic)
Query: Shows all ops tasks in the epic
Query ready: Shows only setup (others blocked)
```

---

## Key Takeaways

### Visual Summary

```
Labels:           WHAT BELONGS WHERE    (grouping)
Dependencies:     WHAT BLOCKS WHAT      (workflow)

Both flow:        Task → Epic → Milestone
Same direction:   Natural alignment
Orthogonal:       Serve different purposes
```

### When to Use What

```
Use Labels When:                Use Dependencies When:
───────────────                 ──────────────────────
• Organizing related work       • Enforcing work order
• Filtering by scope            • Blocking until ready
• Reporting progress            • Determining what's available
• Grouping for queries          • Controlling state transitions
```

### Remember

1. **Parallel structure**: Both flow the same way (task → epic → milestone)
2. **Different purposes**: Organization vs execution
3. **Can be independent**: Labels without deps, deps without labels
4. **Usually together**: Most workflows use both for maximum clarity
5. **Asymmetry exists**: Dependencies more flexible than membership
