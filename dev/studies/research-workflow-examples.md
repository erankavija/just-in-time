# Research Workflow Examples

**Date**: 2025-12-08  
**Status**: Design specification  
**Goal**: Demonstrate how ideas evolve into structured work in research projects

---

## Table of Contents

1. [Core Concepts](#core-concepts)
2. [Workflow Patterns](#workflow-patterns)
3. [Complete Example: Semantic Search Feature](#complete-example-semantic-search-feature)
4. [Edge Direction Convention](#edge-direction-convention)
5. [Optional vs Required Membership](#optional-vs-required-membership)
6. [Query Patterns](#query-patterns)
7. [Common Pitfalls](#common-pitfalls)

---

## Core Concepts

### Issue Types in Research Projects

| Type | Purpose | Typical Labels | Dependencies |
|------|---------|----------------|--------------|
| **Idea** | Exploratory concept, not yet validated | `type:idea`, `component:*` | May depend on research tasks |
| **Research Task** | Time-boxed investigation or feasibility study | `type:research`, `epic:*` | None (leaf node) |
| **Research** | Investigation work, literature review | `type:research`, `component:*` | May depend on ideas |
| **Epic** | Large, coherent body of work | `type:epic`, `epic:*`, `milestone:*` | Depends on tasks/research tasks |
| **Task** | Concrete implementation work | `type:task`, `epic:*` | May have subtasks |
| **Milestone** | Time-bound release goal | `type:milestone`, `milestone:*` | Depends on epics/tasks |

### Label Semantics

**Membership labels** (conceptual grouping):
- `epic:<name>` - This work belongs to an epic
- `milestone:<version>` - This work belongs to a milestone
- `component:<area>` - This work touches a technical area
- `team:<name>` - This work is owned by a team

**Classification labels** (unique per issue):
- `type:*` - What kind of work this is
- `priority:*` - How urgent this is

**Strategic labels** (for high-level views):
- Issues with `epic:*` or `milestone:*` labels

---

## Labels and Dependencies in Research

Research workflows use the same parallel structure as development workflows.

### Labels vs Dependencies

**Labels (membership)** = Organizational grouping
- "This literature review belongs to the vector-survey paper"
- Purpose: Track related work, filter by project
- Query: `jit query all --label "paper:vector-survey"`

**Dependencies (work order)** = Blocking relationships
- "The paper writing requires literature reviews to complete"
- Purpose: Control workflow, determine what's ready
- Query: `jit query available` (unblocked work)

**Both flow the same direction**: Review → Paper → Publication

### Example: Literature Review → Paper

```bash
# Paper (milestone-like deliverable)
PAPER=$(jit issue create --title "Survey Paper: Vector Databases" \
  --label "type:paper" --label "paper:vector-db-survey")

# Literature review tasks
LIT1=$(jit issue create --title "Review: Qdrant architecture" \
  --label "type:research" --label "paper:vector-db-survey")
LIT2=$(jit issue create --title "Review: Milvus performance" \
  --label "type:research" --label "paper:vector-db-survey")
LIT3=$(jit issue create --title "Review: pgvector integration" \
  --label "type:research" --label "paper:vector-db-survey")

# Paper depends on literature reviews
jit dep add $PAPER $LIT1
jit dep add $PAPER $LIT2
jit dep add $PAPER $LIT3

# Result:
# Labels: All part of vector-db-survey paper (grouped for queries)
# Dependencies: Paper writing waits for reviews (workflow control)
# Query: jit query all --label "paper:*" shows all paper-related work
# Query: jit query available shows reviews (paper is blocked)
```

### Asymmetry: Follow-up Work Can Depend on Published Results

**Future research can depend on past publications:**

```bash
# Published paper (completed work)
PAPER_V1=$(jit issue create --title "Paper: Initial Survey" \
  --label "type:paper" --label "paper:vector-db-v1")
jit issue update $PAPER_V1 --state done

# Future research depends on published work
FOLLOWUP=$(jit issue create --title "Experiment: Benchmark new DB" \
  --label "type:experiment" --label "paper:vector-db-v2")

# Follow-up depends on published paper
jit dep add $FOLLOWUP $PAPER_V1

# Valid: Future experiment waits for past publication
# But: Published paper doesn't "belong to" future experiment (labels separate)
```

### Non-Hierarchical Research Dependencies

**Experiments without shared organizational labels:**

```bash
# Three parallel experiments (different areas)
EXP1=$(jit issue create --title "Exp: Latency test" \
  --label "type:experiment" --label "component:performance")
EXP2=$(jit issue create --title "Exp: Throughput test" \
  --label "type:experiment" --label "component:scalability")
EXP3=$(jit issue create --title "Exp: Accuracy test" \
  --label "type:experiment" --label "component:quality")

# Analysis depends on all experiments
ANALYSIS=$(jit issue create --title "Statistical analysis" \
  --label "type:analysis" --label "project:db-comparison")
jit dep add $ANALYSIS $EXP1
jit dep add $ANALYSIS $EXP2
jit dep add $ANALYSIS $EXP3

# No shared membership labels (experiments are independent areas)
# But clear dependency structure (analysis waits for all experiments)
# Query by component: Shows experiments in different areas
# Query ready: Shows only experiments (analysis blocked)
```

---

## Workflow Patterns

### Pattern 1: Idea → Research Task → Decision

**Scenario**: Explore a vague concept to decide if it's worth pursuing.

```bash
# 1. Capture the idea
IDEA=$(jit issue create \
  --title "Idea: Add semantic search to document corpus" \
  --description "Could vector embeddings improve search quality?" \
  --label "type:idea" \
  --label "component:search" \
  --json | jq -r '.id')

# 2. Create research task to validate feasibility
RESEARCH=$(jit issue create \
  --title "Research: Evaluate vector DB options for semantic search" \
  --description "Research Qdrant, Milvus, pgvector. Time-box: 2 days" \
  --label "type:research" \
  --label "component:search" \
  --json | jq -r '.id')

# 3. Idea depends on research results (research task gates the idea's resolution)
jit dep add $IDEA $RESEARCH

# 4. Assign and work on research task
jit issue claim $RESEARCH agent:researcher-1
jit issue update $RESEARCH --state in-progress

# 5a. If research task proves feasible → promote to epic (see Pattern 2)
# 5b. If research task proves infeasible → close idea with findings
jit issue update $RESEARCH --state done
jit issue update $IDEA --state archived \
  --context "research_result:infeasible;reason:performance concerns"
```

**Visualization:**
```
[Idea: Semantic Search] (backlog, type:idea)
  └─ depends on → [Research: Vector DB eval] (done, type:research)
```

---

### Pattern 2: Idea → Epic → Tasks

**Scenario**: An idea proves valuable and becomes a major feature.

```bash
# 1. Promote idea to epic (or create new epic issue)
EPIC=$(jit issue create \
  --title "Epic: Semantic Search System" \
  --description "Implement vector-based search for all documents" \
  --label "type:epic" \
  --label "epic:semantic-search" \
  --label "component:search" \
  --json | jq -r '.id')

# 2. Link original idea to epic (preserve narrative)
jit issue update $IDEA --label "epic:semantic-search"
jit dep add $EPIC $IDEA  # Epic depends on original idea (context)

# 3. Link research results to epic
jit dep add $EPIC $RESEARCH  # Epic depends on research findings

# 4. Break down into concrete tasks
TASK1=$(jit issue create \
  --title "Implement embedding pipeline" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:search" \
  --json | jq -r '.id')

TASK2=$(jit issue create \
  --title "Integrate Qdrant vector store" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:search" \
  --json | jq -r '.id')

TASK3=$(jit issue create \
  --title "Build search API endpoint" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:backend" \
  --json | jq -r '.id')

# 5. Epic depends on tasks (tasks gate epic completion)
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2
jit dep add $EPIC $TASK3

# 6. Tasks may have inter-dependencies
jit dep add $TASK3 $TASK1  # API needs embedding pipeline
jit dep add $TASK3 $TASK2  # API needs vector store
```

**Visualization:**
```
[Epic: Semantic Search] (backlog, type:epic, epic:semantic-search)
  ├─ depends on → [Idea: Semantic Search] (archived, type:idea)
  ├─ depends on → [Research: Vector DB] (done, type:research)
  ├─ depends on → [Task: Embedding pipeline] (ready, type:task)
  ├─ depends on → [Task: Qdrant integration] (ready, type:task)
  └─ depends on → [Task: Search API] (blocked, type:task)
                    ├─ depends on → [Task: Embedding pipeline]
                    └─ depends on → [Task: Qdrant integration]
```

**Progress tracking:**
```bash
# Show all work in this epic (membership via label)
jit query all --label "epic:semantic-search"

# Show blocking dependencies for epic (what gates completion)
jit graph show $EPIC

# Show downstream impact (what depends on this epic)
jit graph downstream $EPIC
```

---

### Pattern 3: Epic → Milestone

**Scenario**: Tie epics to time-bound release goals.

```bash
# 1. Create milestone for release
MILESTONE=$(jit issue create \
  --title "Milestone: v1.1 - Enhanced Search" \
  --description "Improve search capabilities with semantic features" \
  --label "type:milestone" \
  --label "milestone:v1.1" \
  --context "target_date:2026-03-01" \
  --json | jq -r '.id')

# 2. Milestone depends on epic (epic gates milestone)
jit dep add $MILESTONE $EPIC

# 3. Add other epics/tasks to milestone
OTHER_EPIC=$(jit issue create \
  --title "Epic: Search UI redesign" \
  --label "type:epic" \
  --label "epic:search-ui" \
  --label "milestone:v1.1" \
  --json | jq -r '.id')

jit dep add $MILESTONE $OTHER_EPIC

# 4. Strategic view: what's in this milestone?
jit query all --label "milestone:v1.1"

# 5. Gated view: what must complete for milestone?
jit graph show $MILESTONE
```

**Visualization:**
```
[Milestone: v1.1] (backlog, type:milestone, milestone:v1.1)
  ├─ depends on → [Epic: Semantic Search] (in-progress, epic:semantic-search)
  │                 ├─ depends on → [Task: Embedding] (done)
  │                 ├─ depends on → [Task: Qdrant] (in-progress)
  │                 └─ depends on → [Task: API] (blocked)
  └─ depends on → [Epic: Search UI] (backlog, epic:search-ui)
                    └─ depends on → [Task: Redesign mockups] (ready)
```

---

### Pattern 4: Parallel Exploration

**Scenario**: Multiple competing ideas, pick winner after research tasks.

```bash
# 1. Create parent idea
PARENT=$(jit issue create \
  --title "Idea: Improve search performance" \
  --label "type:idea" \
  --label "component:search" \
  --json | jq -r '.id')

# 2. Create competing research task approaches
RESEARCH_A=$(jit issue create \
  --title "Research: PostgreSQL full-text search" \
  --label "type:research" \
  --label "component:search" \
  --json | jq -r '.id')

RESEARCH_B=$(jit issue create \
  --title "Research: Elasticsearch integration" \
  --label "type:research" \
  --label "component:search" \
  --json | jq -r '.id')

RESEARCH_C=$(jit issue create \
  --title "Research: Vector embeddings (Qdrant)" \
  --label "type:research" \
  --label "component:search" \
  --json | jq -r '.id')

# 3. Parent idea depends on at least one research task succeeding
# (No hard dependency - any research task can validate the idea)
jit issue update $PARENT \
  --context "exploration_research_tasks:$RESEARCH_A,$RESEARCH_B,$RESEARCH_C"

# 4. Work research tasks in parallel
jit issue claim $RESEARCH_A agent:researcher-1
jit issue claim $RESEARCH_B agent:researcher-2
jit issue claim $RESEARCH_C agent:researcher-3

# 5. After research tasks complete, pick winner
jit issue update $RESEARCH_C --state done \
  --context "result:success;performance:excellent;complexity:moderate"
jit issue update $RESEARCH_A --state done \
  --context "result:success;performance:good;complexity:low"
jit issue update $RESEARCH_B --state done \
  --context "result:success;performance:excellent;complexity:high"

# 6. Decision: go with Research task C (best balance)
jit issue update $PARENT --state archived \
  --context "decision:proceed_with_qdrant;chosen_research:$RESEARCH_C"

# 7. Promote to epic (Pattern 2)
```

---

### Pattern 5: Incremental Breakdown with Discovery

**Scenario**: Start with high-level epic, discover subtasks as you go.

```bash
# 1. Create epic with known tasks
EPIC=$(jit issue create \
  --title "Epic: Multi-language support" \
  --label "type:epic" \
  --label "epic:i18n" \
  --json | jq -r '.id')

TASK1=$(jit issue create \
  --title "Task: Add i18n library" \
  --label "type:task" \
  --label "epic:i18n" \
  --json | jq -r '.id')

jit dep add $EPIC $TASK1

# 2. While working, discover new required tasks
jit issue claim $TASK1 agent:dev-1
jit issue update $TASK1 --state in-progress

# 3. Agent discovers need for additional work
TASK2=$(jit issue create \
  --title "Task: Extract hardcoded strings" \
  --label "type:task" \
  --label "epic:i18n" \
  --context "discovered_during:$TASK1" \
  --json | jq -r '.id')

TASK3=$(jit issue create \
  --title "Task: Build translation pipeline" \
  --label "type:task" \
  --label "epic:i18n" \
  --json | jq -r '.id')

# 4. Add dependencies retroactively
jit dep add $EPIC $TASK2
jit dep add $EPIC $TASK3
jit dep add $TASK1 $TASK2  # TASK1 needs strings extracted first

# 5. TASK1 becomes blocked until TASK2 completes
# Agent sees TASK1 is blocked, switches to TASK2
jit issue update $TASK1 --state ready  # Unassign and wait
jit issue claim $TASK2 agent:dev-1
```

**Key insight**: Discovery is natural in research. Dependencies can be added dynamically as understanding deepens.

---

## Complete Example: Semantic Search Feature

### Initial State: Idea Capture

```bash
# Week 1: Research team has an idea
IDEAS_MEETING=$(jit issue create \
  --title "Research Meeting: Q1 2026 Priorities" \
  --label "type:research" \
  --label "team:research" \
  --json | jq -r '.id')

IDEA=$(jit issue create \
  --title "Idea: Add semantic search to document corpus" \
  --description "Users report poor search results. Could vector embeddings help?" \
  --label "type:idea" \
  --label "component:search" \
  --label "team:research" \
  --context "originator:meeting:$IDEAS_MEETING" \
  --json | jq -r '.id')
```

**Graph at this point:**
```
[Idea: Semantic Search] (backlog, type:idea)
```

---

### Exploration Phase: Feasibility Research

```bash
# Week 2: Create research task to evaluate approaches
RESEARCH=$(jit issue create \
  --title "Research: Evaluate vector search solutions" \
  --description "Compare: pgvector, Qdrant, Milvus, Pinecone. Time-box: 3 days" \
  --label "type:research" \
  --label "component:search" \
  --label "team:research" \
  --json | jq -r '.id')

jit dep add $IDEA $RESEARCH  # Idea depends on research results

# Assign to researcher
jit issue claim $RESEARCH agent:researcher-alice
jit issue update $RESEARCH --state in-progress

# Week 2 (Day 3): Research task completes with findings
jit issue update $RESEARCH --state done \
  --context "recommendation:qdrant;reason:best_performance_cost_ratio;poc:successful"
```

**Graph at this point:**
```
[Idea: Semantic Search] (backlog, type:idea)
  └─ depends on → [Research: Vector Search] (done, type:research)
```

---

### Decision Phase: Promote to Epic

```bash
# Week 3: Tech lead reviews research task, approves as epic
EPIC=$(jit issue create \
  --title "Epic: Semantic Search System" \
  --description "Implement Qdrant-based vector search for documents" \
  --label "type:epic" \
  --label "epic:semantic-search" \
  --label "component:search" \
  --label "team:platform" \
  --json | jq -r '.id')

# Link history for traceability
jit dep add $EPIC $IDEA
jit dep add $EPIC $RESEARCH

# Update original idea
jit issue update $IDEA --label "epic:semantic-search" --state archived \
  --context "outcome:approved_as_epic:$EPIC"
```

**Graph at this point:**
```
[Epic: Semantic Search] (backlog, type:epic)
  ├─ depends on → [Idea: Semantic Search] (archived, type:idea)
  └─ depends on → [Research: Vector Search] (done, type:research)
```

---

### Planning Phase: Break Down Work

```bash
# Week 3: Use jit breakdown command (future feature)
# For now, create tasks manually

TASK_EMBED=$(jit issue create \
  --title "Implement document embedding pipeline" \
  --description "OpenAI API or local model? Batch processing." \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:backend" \
  --json | jq -r '.id')

TASK_DB=$(jit issue create \
  --title "Set up Qdrant vector database" \
  --description "Docker deployment, schema design, index config" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:infrastructure" \
  --json | jq -r '.id')

TASK_API=$(jit issue create \
  --title "Build semantic search API endpoint" \
  --description "/api/search?q=...&semantic=true" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:backend" \
  --json | jq -r '.id')

TASK_UI=$(jit issue create \
  --title "Update search UI with semantic toggle" \
  --description "Add checkbox for semantic vs keyword search" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "component:frontend" \
  --json | jq -r '.id')

# Dependencies: logical ordering
jit dep add $EPIC $TASK_EMBED
jit dep add $EPIC $TASK_DB
jit dep add $EPIC $TASK_API
jit dep add $EPIC $TASK_UI

jit dep add $TASK_API $TASK_EMBED  # API needs embeddings
jit dep add $TASK_API $TASK_DB     # API needs database
jit dep add $TASK_UI $TASK_API     # UI needs API
```

**Graph at this point:**
```
[Epic: Semantic Search] (backlog, type:epic)
  ├─ depends on → [Idea] (archived)
  ├─ depends on → [Research] (done)
  ├─ depends on → [Task: Embeddings] (ready, type:task)
  ├─ depends on → [Task: Qdrant setup] (ready, type:task)
  ├─ depends on → [Task: Search API] (blocked, type:task)
  │                 ├─ depends on → [Task: Embeddings]
  │                 └─ depends on → [Task: Qdrant setup]
  └─ depends on → [Task: Search UI] (blocked, type:task)
                    └─ depends on → [Task: Search API]
```

---

### Execution Phase: Parallel Work

```bash
# Week 4-5: Agents claim ready tasks
jit issue claim $TASK_EMBED agent:dev-bob
jit issue claim $TASK_DB agent:dev-carol

jit issue update $TASK_EMBED --state in-progress
jit issue update $TASK_DB --state in-progress

# Week 5: Bob finishes embeddings
jit issue update $TASK_EMBED --state done

# TASK_API becomes partially unblocked (still needs DB)
# Current state: TASK_API blocked by TASK_DB only

# Week 6: Carol finishes Qdrant setup
jit issue update $TASK_DB --state done

# TASK_API now fully unblocked → state:ready
jit query available
# Output: TASK_API

jit issue claim $TASK_API agent:dev-bob
jit issue update $TASK_API --state in-progress

# Week 7: Bob finishes API
jit issue update $TASK_API --state done

# TASK_UI now unblocked
jit issue claim $TASK_UI agent:dev-diana
jit issue update $TASK_UI --state in-progress

# Week 8: Diana finishes UI
jit issue update $TASK_UI --state done

# Epic automatically transitions to ready (all deps done)
```

**Final graph:**
```
[Epic: Semantic Search] (ready, type:epic)
  ├─ depends on → [Idea] (archived)
  ├─ depends on → [Research] (done)
  ├─ depends on → [Task: Embeddings] (done)
  ├─ depends on → [Task: Qdrant setup] (done)
  ├─ depends on → [Task: Search API] (done)
  └─ depends on → [Task: Search UI] (done)
```

---

### Integration Phase: Quality Gates

```bash
# Week 8: Add quality gates before marking epic done
jit gate add $EPIC "performance-test"
jit gate add $EPIC "security-review"
jit gate add $EPIC "documentation"

# Epic transitions to state:gated (deps done, gates pending)

# Week 9: Run tests
./scripts/perf-test-search.sh
jit gate pass $EPIC "performance-test"

# Week 9: Security review
jit issue create \
  --title "Security review: semantic search" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --json | jq -r '.id'
# After review completes:
jit gate pass $EPIC "security-review"

# Week 9: Write docs
jit doc add $EPIC "docs/semantic-search-guide.md"
jit gate pass $EPIC "documentation"

# All gates passed → epic transitions to done
```

---

### Milestone Rollup

```bash
# Week 10: Associate with milestone
MILESTONE=$(jit issue create \
  --title "Milestone: v1.1 - Enhanced Search" \
  --label "type:milestone" \
  --label "milestone:v1.1" \
  --context "target_date:2026-03-01" \
  --json | jq -r '.id')

jit dep add $MILESTONE $EPIC
jit issue update $EPIC --label "milestone:v1.1"

# Query all work in this milestone
jit query all --label "milestone:v1.1"

# Strategic view: show milestone progress
jit graph show $MILESTONE --format mermaid
```

---

## Edge Direction Convention

**Critical principle**: Higher-level items depend on their children.

### Why Parent → Child Direction?

1. **Intuitive blocking semantics**: Parent is blocked until children complete
2. **Natural rollup**: Query "what blocks X" shows child tasks
3. **Consistent progress calculation**: `jit graph show $EPIC` shows required work

### Convention Table

| Relationship | Direction | Meaning |
|--------------|-----------|---------|
| Milestone → Epic | `dep add $MILESTONE $EPIC` | Milestone blocked until epic done |
| Epic → Task | `dep add $EPIC $TASK` | Epic blocked until task done |
| Task → Subtask | `dep add $TASK $SUBTASK` | Task blocked until subtask done |
| Idea → Research Task | `dep add $IDEA $RESEARCH` | Idea validated after research task |
| Task A → Task B | `dep add $TASK_A $TASK_B` | Task A blocked until Task B done |

### Queries Enabled by This Convention

```bash
# What work is required to complete this epic?
jit graph show $EPIC

# What depends on this task (downstream impact)?
jit graph downstream $TASK

# What's blocking this milestone?
jit query blocked --issue $MILESTONE
```

---

## Optional vs Required Membership

**Key distinction**: Labels define membership; dependencies define gating.

### Optional Membership (Label Only)

```bash
# Task belongs to epic conceptually, but not required for epic completion
jit issue create \
  --title "Nice-to-have: Add search analytics" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "priority:low"
# No dependency edge added to epic
```

**Use case**: Related work that enhances the epic but isn't essential for MVP.

### Required Membership (Label + Dependency)

```bash
# Task is essential for epic completion
jit issue create \
  --title "Critical: Fix search crash on empty query" \
  --label "type:task" \
  --label "epic:semantic-search" \
  --label "priority:critical" \
  --json | jq -r '.id'

jit dep add $EPIC $CRITICAL_TASK  # Epic blocked until this completes
```

**Use case**: Core functionality that gates the epic.

### Visual Distinction in Web UI (Future)

```
Epic: Semantic Search
├─ [🔒] Task: Embeddings (required, has edge)
├─ [🔒] Task: API (required, has edge)
├─ [🏷️] Task: Analytics (optional, label only)
└─ [🏷️] Task: Advanced filters (optional, label only)
```

### Query Patterns

```bash
# All work in epic (membership)
jit query all --label "epic:semantic-search"

# Required work for epic (gating)
jit graph show $EPIC

# Optional work (label but no edge)
# Future: jit query all --label "epic:semantic-search" --exclude-dependencies $EPIC
```

---

## Query Patterns

### Strategic Queries

```bash
# All epics
jit query all --label "type:epic"

# All milestones
jit query all --label "type:milestone"

# Strategic view (epics + milestones)
jit query all --label "epic:*" --or label "milestone:*"

# Specific milestone's work
jit query all --label "milestone:v1.1"

# Specific epic's work
jit query all --label "epic:semantic-search"
```

### Operational Queries

```bash
# Ready tasks (unblocked, unassigned)
jit query available

# Ready tasks in specific epic
jit query available | jq '.[] | select(.labels | contains(["epic:semantic-search"]))'

# Blocked issues with reasons
jit query blocked

# My assigned work
jit query all --assignee "agent:dev-bob"

# High-priority work
jit query all --priority "high"
```

### Progress Tracking

```bash
# Epic progress (what's blocking completion?)
jit graph show $EPIC

# Milestone progress
jit graph show $MILESTONE

# Dependency tree from roots
jit graph roots
jit graph show  # Full graph

# Downstream impact analysis
jit graph downstream $TASK
```

### Audit and Hygiene

```bash
# Find ideas without research tasks
jit query all --label "type:idea" | jq '.[] | select(.dependencies | length == 0)'

# Find epics without tasks
jit query all --label "type:epic" | jq '.[] | select(.dependencies | length == 0)'

# Find orphaned research tasks (no idea/epic depends on them)
# Future: jit query orphaned --type research

# Find issues with malformed labels
jit validate
```

---

## Common Pitfalls

### ❌ Pitfall 1: Wrong Edge Direction

```bash
# WRONG: Child depends on parent
jit dep add $TASK $EPIC  # Task depends on epic ❌

# This means: "Task is blocked until entire epic completes"
# Nonsensical: tasks *comprise* the epic
```

**Fix**: Reverse the direction
```bash
# CORRECT: Parent depends on child
jit dep add $EPIC $TASK  # Epic depends on task ✅
```

---

### ❌ Pitfall 2: Over-Dependency

```bash
# WRONG: Every task in epic depends on every other task
jit dep add $TASK1 $TASK2
jit dep add $TASK1 $TASK3
jit dep add $TASK2 $TASK3
# Nothing can be worked in parallel ❌
```

**Fix**: Only add true blocking relationships
```bash
# CORRECT: Only necessary dependencies
jit dep add $TASK_API $TASK_EMBED  # API needs embeddings
# TASK_DB can be worked in parallel
```

---

### ❌ Pitfall 3: Mixing Membership and Gating

```bash
# AMBIGUOUS: Is this task required or optional?
jit issue create --title "Task: Add feature X" --label "epic:foo"
# Missing: Is there a dep edge to epic?
```

**Fix**: Be explicit about required vs optional
```bash
# CLEAR: Required work
jit dep add $EPIC $TASK  # Edge = required

# CLEAR: Optional related work
# No edge = nice-to-have
```

---

### ❌ Pitfall 4: Forgetting to Update State

```bash
# WRONG: Task completes but agent forgets to update
jit issue update $TASK --context "done_at:2026-01-15"
# Issue still shows state:in-progress ❌
# Epic remains blocked
```

**Fix**: Always transition state
```bash
# CORRECT:
jit issue update $TASK --state done
# Epic auto-transitions to ready when all deps done
```

---

### ❌ Pitfall 5: Creating Cycles

```bash
# WRONG: Circular dependency
jit dep add $EPIC $TASK1
jit dep add $TASK1 $TASK2
jit dep add $TASK2 $EPIC  # Creates cycle ❌
```

**Fix**: DAG validation will reject this
```
Error: Dependency would create cycle: EPIC → TASK1 → TASK2 → EPIC
```

---

### ❌ Pitfall 6: Orphaned Ideas

```bash
# WRONG: Idea created but never explored
jit issue create --title "Idea: Cool feature" --label "type:idea"
# Weeks pass, no research task, no decision ❌
```

**Fix**: Time-box idea review cycles
```bash
# BETTER: Add follow-up review task
REVIEW=$(jit issue create \
  --title "Review pending ideas" \
  --label "type:task" \
  --label "team:product" \
  --context "due_date:2026-02-01")

# Or: Create research task immediately
RESEARCH=$(jit issue create --title "Research: Validate idea" --label "type:research")
jit dep add $IDEA $RESEARCH
```

---

### ❌ Pitfall 7: Monolithic Epics

```bash
# WRONG: Epic with 50 tasks, no breakdown
EPIC=$(jit issue create --title "Epic: Rewrite entire system" --label "type:epic")
# Unmanageable ❌
```

**Fix**: Break into sub-epics or themes
```bash
# BETTER: Hierarchical epics
EPIC_PARENT=$(jit issue create --title "Epic: System modernization" --label "type:epic")
EPIC_AUTH=$(jit issue create --title "Epic: Rewrite auth" --label "type:epic" --label "epic:modernization")
EPIC_DB=$(jit issue create --title "Epic: Migrate database" --label "type:epic" --label "epic:modernization")

jit dep add $EPIC_PARENT $EPIC_AUTH
jit dep add $EPIC_PARENT $EPIC_DB

# Then break each sub-epic into tasks
```

---

## Summary

### Decision Tree: What to Create?

```
Is it a vague concept?
├─ YES → type:idea
│   └─ Needs validation? → Create type:research, add dependency
└─ NO → Is it a large body of work?
    ├─ YES → type:epic
    │   └─ Break into type:task issues
    └─ NO → Is it concrete?
        ├─ YES → type:task
        └─ NO → Is it time-boxed research?
            └─ YES → type:research
```

### Edge Direction Rule

**Always**: Parent depends on child
- ✅ Milestone → Epic
- ✅ Epic → Task
- ✅ Task → Subtask
- ✅ Idea → Research Task

### Membership vs Gating

| Scenario | Label | Edge | Meaning |
|----------|-------|------|---------|
| Core work | ✅ | ✅ | Required for completion |
| Nice-to-have | ✅ | ❌ | Related but optional |
| Blocking prerequisite | ❌ | ✅ | Gates progress but not member |

---

## Next Steps

1. **Review these patterns** - Do they match your research workflow?
2. **Identify gaps** - Are there workflows not covered?
3. **Refine conventions** - Adjust edge direction or labeling if needed
4. **Implement Phase 1** - Add core label support to `jit`
5. **Write agent prompts** - Teach AI agents these patterns

---

**See also:**
- `label-hierarchy-implementation-plan.md` - Technical implementation details
- `label-conventions.md` - Label format specification and registry
- `design.md` - Core system architecture
