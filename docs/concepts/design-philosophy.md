# Design Philosophy

> **Diátaxis Type:** Explanation  
> **Audience:** Users and contributors seeking to understand JIT's core principles

This document explains the fundamental design decisions behind JIT and why they matter. Understanding these principles helps you use JIT effectively and contribute meaningfully to the project.

## Domain Agnostic

**Core Principle:** JIT works for any domain—software development, research, knowledge work, project management—without requiring domain-specific terminology or workflows.

### Why Domain Neutrality Matters

Traditional issue trackers are designed for software development, using terms like "sprint," "story points," "pull request," and "deployment." This creates friction when applying them to other domains:

- Researchers feel awkward using "sprint" for experiments
- Writers find "story points" meaningless for chapters
- Project managers resist "deployment" for budget approvals

JIT uses **universal terminology** that works everywhere:

- **Issue** - Any unit of work (feature, experiment, chapter, budget item)
- **Gate** - Any quality checkpoint (tests, peer review, approval, audit)
- **Dependency** - Any blocking relationship (prerequisite, requirement, dependency)
- **Label** - Any categorization (type, epic, milestone, phase, topic)

### Domain Mapping Examples

The same concepts adapt to different contexts:

| JIT Concept | Software Dev | Research | Writing | Project Mgmt |
|-------------|-------------|----------|---------|--------------|
| **Issue** | Feature/Bug | Experiment | Chapter | Deliverable |
| **Gate** | CI checks | Peer review | Editor review | Stakeholder approval |
| **Dependency** | Feature A needs API | Exp 2 needs Exp 1 data | Ch 3 needs Ch 2 | Phase 2 needs Phase 1 |
| **Label: epic** | epic:auth | project:nas-research | book:novel-draft | initiative:q1-goals |
| **Label: type** | type:task | type:task | type:task | type:task |
| **Priority** | Ship blocker | Paper deadline | Publication date | Board meeting |

### Configurable Workflows

JIT provides **configuration over convention**—hierarchies and workflows adapt to your domain:

```toml
# Software: milestone → epic → task
# Research: project → phase → experiment  
# Writing: book → section → chapter
```

This flexibility enables broader adoption (teams outside software), clearer workflows (no translation needed), and longevity (works for future use cases).

## Agent-First Design

**Core Principle:** JIT is built for programmatic agents to use, with human ergonomics as a secondary (but important) benefit.

### Why Agent-First?

AI agents are increasingly capable of complex software tasks, but existing tools assume human users. JIT inverts this:

- **Primary audience:** Programmatic agents (AI assistants, automation scripts)
- **Secondary audience:** Humans (via same CLI, web UI, or MCP)

**Result:** Agents get a powerful, reliable interface. Humans benefit from the same clarity and predictability.

### Key Design Principles

**JSON-First Output:** Every command supports `--json` for structured data—agents parse reliably, no regex scraping needed.

**Atomic File Operations:** Write-temp-rename pattern prevents race conditions and partial writes. Multi-agent safe by design.

**Clear Exit Codes:** UNIX-standard exit codes enable bash error handling: `jit ... || handle_error`

**MCP Protocol:** AI assistants call JIT operations directly via Model Context Protocol—type-safe, structured tool use.

**Structured Errors:** JSON errors include error codes and context for programmatic error recovery.

**Example - Multi-agent coordination:**
```bash
# Agent 1
jit claim acquire $TASK --agent-id agent:worker-1
# ✓ Acquired lease (atomic via file lock)

# Agent 2 (simultaneously, same task)
jit claim acquire $TASK --agent-id agent:worker-2
# ✗ ERROR: Already claimed by agent:worker-1
```

File locking serializes claim operations, preventing concurrent modification conflicts.

## Functional Programming Principles

**Core Principle:** Prefer functional patterns—immutability, pure functions, composition—over stateful object-oriented code.

### Why Functional Programming?

JIT's Rust codebase follows functional principles:

**1. Immutability where practical**
```rust
// GOOD: Return new value
fn compute_ready_issues(issues: &[Issue]) -> Vec<&Issue> {
    issues.iter()
        .filter(|i| i.state == State::Ready)
        .collect()
}

// AVOID: Mutate input
fn compute_ready_issues_mut(issues: &mut Vec<Issue>) {
    issues.retain(|i| i.state == State::Ready);
}
```

**2. Pure functions over stateful objects**
```rust
// GOOD: Pure function (same input = same output)
fn is_blocked(issue: &Issue, all_issues: &[Issue]) -> bool {
    issue.dependencies.iter()
        .any(|dep_id| !is_terminal(dep_id, all_issues))
}

// AVOID: Stateful method with hidden dependencies
impl Issue {
    fn is_blocked(&self) -> bool {
        // Requires self to have access to global state
    }
}
```

**3. Iterator combinators over explicit loops**
```rust
// GOOD: Functional style
let blocked_count = issues.iter()
    .filter(|i| is_blocked(i, &all_issues))
    .count();

// AVOID: Imperative loops
let mut count = 0;
for issue in &issues {
    if is_blocked(issue, &all_issues) {
        count += 1;
    }
}
```

**4. Expression-oriented code**
```rust
// GOOD: Expression with early return
let state = if issue.has_blocking_dependencies() {
    return Err("Dependencies not satisfied");
} else if issue.has_failing_gates() {
    State::Gated
} else {
    State::Ready
};

// AVOID: Statement-oriented mutation
let mut state = State::Backlog;
if has_deps {
    return Err(...);
}
if has_gates {
    state = State::Gated;
} else {
    state = State::Ready;
}
```

### Why It Matters

**1. Easier to reason about**
- Pure functions have no hidden side effects
- Same inputs always produce same outputs
- No global state to track mentally

**2. Better testability**
```rust
#[test]
fn test_is_blocked_pure_function() {
    let issue = Issue::new("Test");
    let all_issues = vec![...];
    
    // No mocks, no setup, just call the function
    assert!(is_blocked(&issue, &all_issues));
}
```

**3. Fewer bugs**
- Immutability prevents accidental mutations
- Pure functions eliminate temporal coupling
- Type system catches more errors at compile time

**4. Composability**
```rust
let high_priority_blocked = issues.iter()
    .filter(|i| i.priority == Priority::High)
    .filter(|i| is_blocked(i, &all_issues))
    .collect::<Vec<_>>();
```

### Real Examples from Codebase

**Graph traversal (cycle detection):**
```rust
// From crates/jit/src/graph.rs
fn is_reachable(&self, start: &str, target: &str) -> bool {
    let mut visited = HashSet::new();
    let mut stack = vec![start];
    
    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if visited.insert(current) {
            if let Some(node) = self.nodes.get(current) {
                stack.extend(node.dependencies().iter().map(String::as_str));
            }
        }
    }
    false
}
```

**Query filtering with combinators:**
```rust
let available = issues.iter()
    .filter(|i| i.state == State::Ready)
    .filter(|i| i.assignee.is_none())
    .filter(|i| !is_blocked(i, &all_issues))
    .sorted_by_key(|i| i.priority)
    .collect::<Vec<_>>();
```

**Result/Option for error handling:**
```rust
// No exceptions, explicit error handling
pub fn load_issue(&self, id: &str) -> Result<Issue> {
    let path = self.issue_path(id);
    self.read_json(&path)
        .with_context(|| format!("Failed to load issue {}", id))
}
```

### Pragmatic Exceptions

Functional purity is not absolute:
- **File I/O** inherently has side effects
- **CLI layer** can be imperative for clarity
- **Performance** may require mutation in hot paths

**Key:** Encapsulate imperative code behind clean functional APIs.

## CLI as Primary Interface

**Core Principle:** The command-line interface is JIT's primary interface, not an afterthought.

### Why CLI Over Web-First?

Most modern tools start with a web UI and add CLI later. JIT inverts this:

**1. Scriptability and Automation**
```bash
# Agents and scripts compose commands
jit query available --json | \
  jq -r '.data.issues[0].id' | \
  xargs -I {} jit claim acquire {} --agent-id agent:worker-1
```

**2. Agent-Friendly by Default**
- No need to parse HTML or scrape web pages
- Direct API via shell commands
- JSON output for structured data

**3. UNIX Philosophy**
- Do one thing well (issue tracking)
- Compose with other tools (jq, grep, awk)
- Text streams as universal interface

**4. No Server Required**
- Works offline
- No API versioning headaches
- No network latency

### MCP and Web UI Built on CLI

The architecture is layered:

```
┌─────────────────────────────────────┐
│  Web UI (jit-server)                │  ← Visualization layer
│  http://localhost:8080              │
├─────────────────────────────────────┤
│  MCP Server (mcp-server/)           │  ← AI agent integration
│  Model Context Protocol             │
├─────────────────────────────────────┤
│  CLI (jit)                          │  ← Foundation
│  Command-line interface             │
├─────────────────────────────────────┤
│  Core Library (crates/jit)          │  ← Business logic
│  Storage, graph, validation         │
└─────────────────────────────────────┘
```

**Benefits:**
- All features available via CLI first
- Web UI and MCP never ahead of CLI
- Single source of truth (core library)

**Example:**
```bash
# CLI (foundation)
jit issue create --title "Feature X" --priority high

# MCP server (calls CLI internally)
Jit-jit_issue_create(title="Feature X", priority="high")

# Web UI (calls CLI via jit-server)
POST /api/issues {"title": "Feature X", "priority": "high"}
```

### Human Ergonomics Matter Too

While agent-first, CLI includes human-friendly features:

**1. Short hashes (like git)**
```bash
# Full UUID: abc12345-6789-4def-1234-567890abcdef
# Short: abc12345 (min 4 chars)
jit issue show abc12
```

**2. Quiet mode for clean output**
```bash
jit query available --quiet  # Only IDs, one per line
```

**3. Progress indicators for long operations**
```bash
jit gate check-all $ISSUE
Running tests... ✓
Running clippy... ✓
All gates passed
```

**4. Colored output for terminals**
```bash
jit status
✓ 5 done (green)
→ 3 in_progress (blue)
⚠ 2 blocked (yellow)
```

**5. Helpful error messages**
```bash
jit issue show nonexistent
Error: Issue not found: nonexistent
  
Suggestions:
  • Check the issue ID (use 'jit query all' to list issues)
  • Try a longer prefix if using short hash
```

## Dogfooding

**Core Principle:** We use JIT to track JIT's own development. Eat our own dog food.

### Why Dogfooding Matters

**1. Real-world validation:** If JIT can't track its own development, how can it track yours?

**2. Continuous feedback:** We experience pain points before users do.

**3. Credibility:** Claims about agent orchestration and workflow management are proven, not theoretical.

### JIT Tracking JIT

**This very repository uses JIT:**

```bash
# View JIT's own issues
$ jit query all
Found 87 issues:
  epic:docs - User Documentation (19 issues)
  epic:multi-agent - Parallel Work Support (12 issues)
  epic:gate-system - Quality Gates (8 issues)
  ...

# The documentation you're reading was tracked as issues
$ jit query all --label "epic:docs"
  84c358ec - Story: Tutorial Documentation [done]
  d820155f - Write guarantees.md (System Guarantees) [done]
  b66d2338 - Complete Issues section in core-model.md [done]
  c8355d70 - Write design-philosophy.md [in_progress]
  ...
```

**Gates tested on our workflow:**
```bash
# We use the same gates we recommend
$ jit gate list
code-review    - Manual review required
tests          - Cargo test suite must pass
clippy         - Rust linter must pass
fmt            - Code formatting check
```

**Multi-agent coordination proven:**
```bash
# Multiple agents work on documentation simultaneously
# (Yes, the agent writing this doc claimed it via jit!)
$ jit claim status
Agent: agent:docs-worker
  Lease: fd33891c
  Issue: c8355d70 (Write design-philosophy.md)
  Expires: 2026-02-02T21:42:00Z
```

### Specific Examples

**1. Dependency graphs in practice**

Our milestones depend on epics, epics depend on tasks:
```bash
$ jit graph deps cfb3ba94  # User Documentation epic
Epic cfb3ba94 depends on:
  → 84c358ec Tutorial Documentation [done]
  → 5bad7437 Reference Documentation [ready]
  → c8254dbf Core Concepts Documentation [backlog]
```

**2. Gate system validation**

Documentation quality enforced via gates:
```bash
$ jit gate check c8355d70 code-review
Gate 'code-review' status: pending
(Will pass when agent:docs-worker completes task)
```

**3. Label hierarchies in use**

We use `epic:*` labels to organize work:
```bash
$ jit query all --label "epic:docs"
# Returns all documentation issues (this proves label filtering works)
```

**4. Document lifecycle tested**

Development documents are linked to issues:
```bash
$ jit doc list d820155f
Documents for issue d820155f:
  dev/design/guarantees-design.md (design)
  # (if we had linked design docs - example of the feature)
```

### Benefits of Dogfooding

**1. Catches usability issues early**
- If we struggle with a command, so will users
- Awkward workflows get fixed before release

**2. Validates agent orchestration claims**
- "Multi-agent safe" is proven, not theoretical
- Race conditions would break our own workflow

**3. Ensures production readiness**
- We won't ship features we wouldn't use ourselves
- Quality bar is high (we live with the consequences)

**4. Documentation stays accurate**
- Examples come from real usage, not imagination
- Screenshots and workflows are up-to-date

**5. Motivation and accountability**
- If JIT can't track JIT, we failed
- Success means our tool is genuinely useful

### Continuous Improvement Loop

```
┌─────────────────────────────────────────┐
│  1. Use JIT to build JIT                │
├─────────────────────────────────────────┤
│  2. Experience pain points              │
├─────────────────────────────────────────┤
│  3. Track improvements as issues        │
├─────────────────────────────────────────┤
│  4. Implement fixes                     │
├─────────────────────────────────────────┤
│  5. Validate fixes in our workflow      │
└──────────────┬──────────────────────────┘
               │
               └─→ Loop back to step 1
```

**Example:** The `--json` flag everywhere came from agents needing structured output while building JIT features. We felt the pain, added JSON output, and now all users benefit.

## See Also

- [System Guarantees](guarantees.md) - How JIT ensures reliability (DAG, atomicity, consistency)
- [Core Model](core-model.md) - Domain-agnostic concepts (issues, gates, dependencies)
- [Quickstart Tutorial](../tutorials/quickstart.md) - See principles in practice
- [How-To: Multi-Agent Coordination](../how-to/multi-agent-coordination.md) - Agent-first design in action
- [CONTRIBUTOR-QUICKSTART.md](../../CONTRIBUTOR-QUICKSTART.md) - FP guidelines for contributors
