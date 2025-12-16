# Agent Project Initialization Guide

**Goal**: Have an AI agent create a complete, best-practice JIT project structure from scratch.

**Last Updated**: 2025-12-16

---

## Overview

This guide walks through having an AI agent (like GitHub Copilot, Claude, or a custom agent) create a full project structure in JIT following best practices including:
- Type hierarchy (milestones → epics → tasks)
- Proper label conventions
- Dependency graphs
- Quality gates
- Strategic/tactical organization

---

## Prerequisites

1. **JIT installed and in PATH**:
   ```bash
   cargo build --release --workspace
   export PATH="$PWD/target/release:$PATH"
   ```

2. **Test directory initialized**:
   ```bash
   mkdir ~/jit-agent-demo && cd ~/jit-agent-demo
   jit init
   ```

3. **Gate registry configured**:
   ```bash
   jit registry add unit-tests --title "Unit Tests" --auto
   jit registry add integration-tests --title "Integration Tests" --auto
   jit registry add review --title "Code Review"
   jit registry add security-scan --title "Security Scan" --auto
   jit registry add docs --title "Documentation"
   ```

---

## Agent Instructions Template

Copy this into your agent's context (e.g., as a prompt to Copilot/Claude):

```markdown
You are setting up a new software project using the JIT issue tracker.
Create a complete project structure following these rules:

### Label Format Rules (CRITICAL)
- ALL labels MUST use format: `namespace:value`
- Namespace: lowercase alphanumeric with hyphens
- Value: alphanumeric with dots, hyphens, underscores
- Examples: ✅ `milestone:v1.0`, `epic:auth`, `type:task`
- NEVER: ❌ `auth`, `milestone-v1.0`, `Epic:Auth`

### Required Label Namespaces
1. **type:*** (REQUIRED, unique - exactly one per issue):
   - `type:milestone` - Release goal
   - `type:epic` - Large feature
   - `type:task` - Concrete work
   - `type:research` - Time-boxed investigation
   - `type:bug` - Defect to fix

2. **milestone:*** (for membership):
   - Groups work under release goals
   - Example: `milestone:v1.0`

3. **epic:*** (for membership):
   - Groups tasks under features
   - Example: `epic:auth`, `epic:api`

4. **component:*** (optional):
   - Technical area
   - Example: `component:backend`, `component:frontend`

### Label Usage Pattern
```bash
# Milestone issue (the container)
jit issue create \
  --title "Release v1.0" \
  --label "type:milestone" \
  --label "milestone:v1.0" \
  --priority critical

# Epic issue (feature within milestone)
jit issue create \
  --title "User Authentication" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high

# Task (work item under epic)
jit issue create \
  --title "Implement login endpoint" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review
```

### Dependency Setup
```bash
# Milestone depends on all its epics
jit dep add <milestone-id> <epic-id>

# Epic depends on all its tasks
jit dep add <epic-id> <task-id>

# Tasks can depend on other tasks
jit dep add <task2-id> <task1-id>  # task2 needs task1 done first
```

### Project Structure to Create

For a typical web application, create:

**1 Milestone**: Release v1.0

**3-4 Epics under milestone**:
- User authentication
- Core API
- Frontend UI
- Database setup

**3-5 Tasks per epic**:
- Concrete, actionable work items
- With appropriate gates (unit-tests, review)
- With dependencies where needed

### Output Format
After creating the structure, provide:
1. Summary of created issues (count by type)
2. Dependency graph visualization (`jit graph show <milestone-id>`)
3. Strategic view (`jit query strategic`)
4. List of ready-to-work tasks (`jit query ready`)
```

---

## Step-by-Step Agent Workflow

### Phase 1: Discovery
```bash
# Agent should first check available namespaces
jit label namespaces

# Check gate registry
jit registry list

# Verify clean state
jit status
```

### Phase 2: Create Milestone
```bash
# Create the top-level goal
MILESTONE=$(jit issue create \
  --title "Release v1.0 - MVP Launch" \
  --desc "First production-ready release with core features" \
  --label "type:milestone" \
  --label "milestone:v1.0" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')

echo "Created milestone: $MILESTONE"
```

### Phase 3: Create Epics
```bash
# Epic 1: Authentication
AUTH_EPIC=$(jit issue create \
  --title "User Authentication System" \
  --desc "JWT-based authentication with login, logout, token refresh" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high \
  --gate integration-tests --gate review | grep -oP 'Created issue: \K\S+')

# Epic 2: Core API
API_EPIC=$(jit issue create \
  --title "RESTful API Endpoints" \
  --desc "CRUD operations for main entities" \
  --label "type:epic" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --priority high \
  --gate integration-tests --gate review | grep -oP 'Created issue: \K\S+')

# Epic 3: Frontend
UI_EPIC=$(jit issue create \
  --title "Web User Interface" \
  --desc "React-based responsive UI" \
  --label "type:epic" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --priority high \
  --gate review | grep -oP 'Created issue: \K\S+')

# Epic 4: Infrastructure
INFRA_EPIC=$(jit issue create \
  --title "Database & Infrastructure Setup" \
  --desc "PostgreSQL schema, migrations, deployment config" \
  --label "type:epic" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --priority critical \
  --gate review --gate security-scan | grep -oP 'Created issue: \K\S+')

# Link epics to milestone
jit dep add $MILESTONE $AUTH_EPIC
jit dep add $MILESTONE $API_EPIC
jit dep add $MILESTONE $UI_EPIC
jit dep add $MILESTONE $INFRA_EPIC
```

### Phase 4: Create Tasks (Example for Auth Epic)
```bash
# Task 1: Database models
AUTH_T1=$(jit issue create \
  --title "Create User model with authentication fields" \
  --desc "SQLAlchemy model: id, email, password_hash, created_at" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

# Task 2: JWT utilities
AUTH_T2=$(jit issue create \
  --title "Implement JWT token generation and validation" \
  --desc "Utility functions for creating and verifying JWTs" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

# Task 3: Login endpoint (depends on T1 and T2)
AUTH_T3=$(jit issue create \
  --title "Build POST /api/auth/login endpoint" \
  --desc "Accept credentials, validate, return JWT token" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

# Task 4: Middleware
AUTH_T4=$(jit issue create \
  --title "Create authentication middleware" \
  --desc "Verify JWT on protected routes, add user to context" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

# Task 5: Integration tests (depends on all above)
AUTH_T5=$(jit issue create \
  --title "Write authentication integration tests" \
  --desc "End-to-end tests: login flow, token validation, protected routes" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority normal \
  --gate review | grep -oP 'Created issue: \K\S+')

# Setup dependencies
jit dep add $AUTH_EPIC $AUTH_T1
jit dep add $AUTH_EPIC $AUTH_T2
jit dep add $AUTH_EPIC $AUTH_T3
jit dep add $AUTH_EPIC $AUTH_T4
jit dep add $AUTH_EPIC $AUTH_T5

# Task dependencies (order matters)
jit dep add $AUTH_T3 $AUTH_T1  # Login needs User model
jit dep add $AUTH_T3 $AUTH_T2  # Login needs JWT utils
jit dep add $AUTH_T4 $AUTH_T2  # Middleware needs JWT utils
jit dep add $AUTH_T5 $AUTH_T1  # Tests need User model
jit dep add $AUTH_T5 $AUTH_T2  # Tests need JWT utils
jit dep add $AUTH_T5 $AUTH_T3  # Tests need login endpoint
jit dep add $AUTH_T5 $AUTH_T4  # Tests need middleware
```

### Phase 5: Repeat for Other Epics
```bash
# Repeat Phase 4 pattern for:
# - API_EPIC: Create 4-5 tasks for CRUD operations
# - UI_EPIC: Create 4-5 tasks for React components
# - INFRA_EPIC: Create 3-4 tasks for database setup

# Example for INFRA (with cross-epic dependency):
INFRA_T1=$(jit issue create \
  --title "Design PostgreSQL schema" \
  --label "type:task" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')

# Auth tasks depend on database being ready
jit dep add $AUTH_T1 $INFRA_T1
```

### Phase 6: Verification
```bash
# View the full dependency graph
jit graph show $MILESTONE

# Check strategic view (should show milestone + 4 epics)
jit query strategic

# Count issues by type
jit issue list --json | jq 'group_by(.labels[] | select(startswith("type:"))) | map({type: .[0].labels[] | select(startswith("type:")), count: length})'

# Verify no cycles
jit validate

# Check initial state (all should be 'open', none 'ready' yet)
jit status
```

---

## Example Complete Script

Here's a complete bash script an agent could execute:

```bash
#!/bin/bash
# agent-init-project.sh - Initialize JIT project structure

set -e  # Exit on error

echo "🚀 Initializing JIT project structure..."

# Phase 1: Create Milestone
echo "📦 Creating milestone..."
MILESTONE=$(jit issue create \
  --title "Release v1.0 - MVP Launch" \
  --label "type:milestone" \
  --label "milestone:v1.0" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')

# Phase 2: Create Epics
echo "🎯 Creating epics..."
AUTH_EPIC=$(jit issue create \
  --title "User Authentication System" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high \
  --gate integration-tests --gate review | grep -oP 'Created issue: \K\S+')

API_EPIC=$(jit issue create \
  --title "RESTful API Endpoints" \
  --label "type:epic" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --priority high \
  --gate integration-tests --gate review | grep -oP 'Created issue: \K\S+')

INFRA_EPIC=$(jit issue create \
  --title "Database & Infrastructure" \
  --label "type:epic" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')

# Link to milestone
jit dep add $MILESTONE $AUTH_EPIC
jit dep add $MILESTONE $API_EPIC
jit dep add $MILESTONE $INFRA_EPIC

# Phase 3: Create Tasks
echo "✅ Creating tasks..."

# Infrastructure tasks (foundation)
INFRA_T1=$(jit issue create \
  --title "Design PostgreSQL schema" \
  --label "type:task" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')

INFRA_T2=$(jit issue create \
  --title "Setup database migrations" \
  --label "type:task" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

jit dep add $INFRA_EPIC $INFRA_T1
jit dep add $INFRA_EPIC $INFRA_T2
jit dep add $INFRA_T2 $INFRA_T1

# Auth tasks
AUTH_T1=$(jit issue create \
  --title "Create User model" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

AUTH_T2=$(jit issue create \
  --title "Implement JWT utilities" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

AUTH_T3=$(jit issue create \
  --title "Build login endpoint" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

jit dep add $AUTH_EPIC $AUTH_T1
jit dep add $AUTH_EPIC $AUTH_T2
jit dep add $AUTH_EPIC $AUTH_T3
jit dep add $AUTH_T1 $INFRA_T2  # User model needs migrations
jit dep add $AUTH_T3 $AUTH_T1   # Login needs User model
jit dep add $AUTH_T3 $AUTH_T2   # Login needs JWT utils

# API tasks
API_T1=$(jit issue create \
  --title "Create API router structure" \
  --label "type:task" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

API_T2=$(jit issue create \
  --title "Implement CRUD endpoints" \
  --label "type:task" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')

jit dep add $API_EPIC $API_T1
jit dep add $API_EPIC $API_T2
jit dep add $API_T1 $AUTH_T3  # API router needs auth
jit dep add $API_T2 $API_T1   # CRUD needs router

# Phase 4: Verification
echo ""
echo "📊 Project Structure Created!"
echo "=============================="
echo ""
echo "Strategic View:"
jit query strategic --json | jq -r '.[] | "[\(.labels[] | select(startswith("type:")))] \(.title)"'
echo ""
echo "Issue Count by Type:"
jit issue list --json | jq -r '[.[] | .labels[] | select(startswith("type:"))] | group_by(.) | map({type: .[0], count: length}) | .[]' | jq -r '"\(.type): \(.count)"'
echo ""
echo "Dependency Graph (milestone):"
jit graph show $MILESTONE
echo ""
echo "Ready Issues (should be none initially):"
jit query ready
echo ""
echo "✅ Project initialized successfully!"
echo "Milestone ID: $MILESTONE"
```

---

## Agent Testing Checklist

After the agent creates the structure, verify:

- [ ] All issues have exactly one `type:*` label
- [ ] Milestones have `type:milestone` + `milestone:*` labels
- [ ] Epics have `type:epic` + `epic:*` + `milestone:*` labels
- [ ] Tasks have `type:task` + `epic:*` + `milestone:*` labels
- [ ] No label format errors (all use `namespace:value`)
- [ ] Dependencies form a valid DAG (no cycles)
- [ ] Strategic view shows only milestones and epics
- [ ] Gates are assigned appropriately
- [ ] Cross-epic dependencies exist where needed (e.g., infra → auth)

**Validation command**:
```bash
jit validate
```

---

## Common Agent Mistakes to Watch For

### 1. Label Format Errors
```bash
# ❌ WRONG
--label "auth"
--label "milestone-v1.0"
--label "Type:task"

# ✅ CORRECT
--label "epic:auth"
--label "milestone:v1.0"
--label "type:task"
```

### 2. Missing Type Labels
```bash
# ❌ WRONG (no type label)
jit issue create --title "Fix bug" --label "epic:auth"

# ✅ CORRECT
jit issue create --title "Fix bug" --label "type:bug" --label "epic:auth"
```

### 3. Duplicate Type Labels
```bash
# ❌ WRONG (type is unique namespace)
--label "type:task" --label "type:bug"

# ✅ CORRECT (pick one)
--label "type:task"
```

### 4. Missing Membership Labels
```bash
# ❌ WRONG (task without parent)
jit issue create --title "Do work" --label "type:task"
# This creates an "orphaned task" warning

# ✅ CORRECT
jit issue create --title "Do work" --label "type:task" --label "epic:auth" --label "milestone:v1.0"
```

### 5. Circular Dependencies
```bash
# ❌ WRONG
jit dep add $TASK1 $TASK2
jit dep add $TASK2 $TASK1  # Error: creates cycle

# ✅ CORRECT
jit dep add $TASK2 $TASK1  # task2 depends on task1
```

---

## Next Steps After Initialization

1. **Mark foundation tasks as ready**:
   ```bash
   jit issue update $INFRA_T1 --state ready
   ```

2. **Start coordinator** (if using multi-agent):
   ```bash
   jit coordinator start
   ```

3. **Agents claim and work on tasks**:
   ```bash
   jit issue claim-next copilot:worker-1
   ```

4. **Monitor progress**:
   ```bash
   jit status
   jit coordinator agents
   jit events tail
   ```

---

## Resources

- Label conventions: `docs/label-conventions.md`
- Complete examples: `EXAMPLE.md`
- Design documentation: `docs/design.md`
- Type hierarchy: `docs/type-hierarchy-enforcement-proposal.md`

---

## Success Criteria

Your AI agent has successfully initialized the project when:

✅ Strategic view shows clean hierarchy (milestone → epics)  
✅ All labels follow `namespace:value` format  
✅ Every issue has exactly one `type:*` label  
✅ Dependency graph is acyclic and logical  
✅ Foundation tasks are unblocked and ready to start  
✅ `jit validate` passes with no errors  

**Now you're ready to start coordinated multi-agent development!** 🎉
