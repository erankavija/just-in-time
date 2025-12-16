# JIT Agent Quickstart

**Goal**: Test JIT with an AI agent that creates a complete, best-practice project structure.

## 🚀 One-Command Setup

```bash
# 1. Build JIT
cargo build --release --workspace
export PATH="$PWD/target/release:$PATH"

# 2. Create test project
mkdir ~/jit-agent-test && cd ~/jit-agent-test
jit init

# 3. Run the agent initialization script
bash ~/Projects/just-in-time/scripts/agent-init-demo-project.sh
```

**That's it!** The script creates a complete project with:
- 1 Milestone (v1.0 release)
- 4 Epics (Auth, API, UI, Infrastructure)
- ~15 Tasks with proper dependencies
- Quality gates configured
- Full dependency graph

---

## 📋 What Gets Created

```
Release v1.0 (milestone)
├── User Authentication (epic)
│   ├── Create User model (task)
│   ├── Implement JWT utilities (task)
│   ├── Build login endpoint (task)
│   ├── Create auth middleware (task)
│   └── Write integration tests (task)
├── RESTful API (epic)
│   ├── Create router structure (task)
│   ├── Implement CRUD endpoints (task)
│   ├── Add error handling (task)
│   └── Write API tests (task)
├── Web UI (epic)
│   ├── Setup React project (task)
│   ├── Create auth components (task)
│   ├── Build dashboard (task)
│   └── Implement API client (task)
└── Infrastructure (epic)
    ├── Design database schema (task)
    ├── Setup migrations (task)
    └── Configure connection pooling (task)
```

---

## 🎯 Agent Testing Scenarios

### 1. View the Structure
```bash
# Strategic overview (milestones + epics)
jit query strategic

# Full dependency graph
jit graph show <milestone-id>

# Current status
jit status
```

### 2. Start Working
```bash
# Mark foundation task as ready
jit issue update <task-id> --state ready

# Agent claims work
jit issue claim <task-id> agent:copilot-test

# Complete the work
jit gate pass <task-id> unit-tests
jit issue update <task-id> --state done
```

### 3. Dynamic Issue Creation
```bash
# Agent discovers new work needed
NEW_TASK=$(jit issue create \
  --title "Add rate limiting" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority critical | grep -oP 'Created issue: \K\S+')

# Add as dependency
jit dep add <epic-id> $NEW_TASK
```

### 4. Monitor Progress
```bash
jit events tail        # Audit log
jit query blocked      # What's blocked and why
jit query ready        # What's ready to work on
```

---

## 🤖 Custom Agent Instructions

If you want your agent to create a DIFFERENT project structure, give it these rules:

### Label Format (CRITICAL)
```
ALL labels MUST use: namespace:value

✅ CORRECT:
  --label "type:task"
  --label "epic:auth"
  --label "milestone:v1.0"
  --label "component:backend"

❌ WRONG:
  --label "auth"           (missing namespace)
  --label "milestone-v1.0" (wrong separator)
  --label "Type:task"      (uppercase namespace)
```

### Required Labels
1. **type:*** (required, unique - exactly ONE per issue):
   - `type:milestone` - Release goal
   - `type:epic` - Large feature
   - `type:task` - Concrete work
   - `type:research` - Investigation
   - `type:bug` - Defect

2. **milestone:*** - Groups work under releases
3. **epic:*** - Groups tasks under features
4. **component:*** (optional) - Technical area

### Hierarchy Pattern
```bash
# Milestone (the release)
MILESTONE=$(jit issue create \
  --title "Release v2.0" \
  --label "type:milestone" \
  --label "milestone:v2.0" \
  --priority critical)

# Epic (feature in release)
EPIC=$(jit issue create \
  --title "Payment System" \
  --label "type:epic" \
  --label "epic:payments" \
  --label "milestone:v2.0" \
  --priority high)

# Task (work in epic)
TASK=$(jit issue create \
  --title "Integrate Stripe API" \
  --label "type:task" \
  --label "epic:payments" \
  --label "milestone:v2.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review)

# Dependencies
jit dep add $MILESTONE $EPIC  # Milestone depends on epic
jit dep add $EPIC $TASK       # Epic depends on task
```

### Agent Checklist
After creating structure, verify:
- [ ] All issues have exactly one `type:*` label
- [ ] All labels use `namespace:value` format
- [ ] Dependencies form a valid DAG (no cycles)
- [ ] Strategic view shows clean hierarchy
- [ ] `jit validate` passes with no errors

---

## 📚 Full Documentation

- **Complete guide**: `docs/agent-project-initialization-guide.md`
- **Label conventions**: `docs/label-conventions.md`
- **Examples**: `EXAMPLE.md`
- **Design docs**: `docs/design.md`

---

## ✅ Success Criteria

Your agent initialized the project correctly if:

1. **Strategic view is clean**:
   ```bash
   jit query strategic
   # Shows: 1 milestone, 3-5 epics
   ```

2. **No validation errors**:
   ```bash
   jit validate
   # Output: ✅ All checks passed
   ```

3. **Dependency graph is logical**:
   ```bash
   jit graph show <milestone-id>
   # Shows clear hierarchy: milestone → epics → tasks
   ```

4. **Foundation tasks are ready**:
   ```bash
   jit query ready
   # Shows 1-2 unblocked tasks you can start with
   ```

---

## 🎉 What's Next?

**Option A: Multi-Agent Orchestration**
1. Start coordinator: `jit coordinator start`
2. Let multiple agents claim and work on ready tasks
3. Monitor: `jit coordinator agents`

**Option B: Single Agent Testing**
1. Agent claims next task: `jit issue claim-next agent:test`
2. Complete work, pass gates
3. Watch dependencies unblock

**Option C: MCP Server Integration**
```bash
cd mcp-server
npm install && npm run build && npm start
# Connect to Claude/Copilot for full AI integration
```

---

**Ready to test with AI agents!** 🤖
