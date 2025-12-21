# Contributor Quick Start

**Goal:** Get productive in 5 minutes. Understand what to work on and how to proceed.

## 1. Get Context (2 minutes)

```bash
# See what we're building
cat README.md | head -30

# What happened recently?
git log --oneline -10

# What's the current work state?
jit status
jit query ready --json | jq -r '.issues[] | "\(.priority) | \(.id[0:8]) | \(.title)"' | head -10
```

## 2. Find Your Next Task (1 minute)

```bash
# See what's ready to work on (prioritized)
jit query ready | grep -E "critical|high"

# Claim a task
jit issue claim <short-hash> agent:your-name

# Read the design doc (if linked)
jit issue show <short-hash> --json | jq -r '.data.documents[0].path'
jit doc show <short-hash> <path>
```

## 3. Work on It (following TDD)

```bash
# 1. Write tests first
cargo test <feature_name> --test  # Should fail

# 2. Implement minimal code to pass
cargo test <feature_name>          # Should pass

# 3. Run full suite
cargo test --workspace --quiet

# 4. Check quality
cargo clippy --workspace --all-targets
cargo fmt --all

# 5. Pass gates
jit gate pass <short-hash> tests
jit gate pass <short-hash> clippy  
jit gate pass <short-hash> fmt
```

## 4. Complete and Move On

```bash
# Mark done (auto-transitions if all gates passed)
jit issue update <short-hash> --state done

# Find next task
jit query ready --json | head -5
```

## Key Files to Know

- **README.md** - Project overview, why JIT exists
- **ROADMAP.md** - Where we are, where we're going
- **TESTING.md** - TDD approach, test strategy
- **.copilot-instructions.md** - Coding standards, patterns to follow
- **docs/design.md** - Core architecture
- **EXAMPLE.md** - Usage examples

## Common Patterns

**Issue has design doc?** Read it first - contains acceptance criteria and implementation plan.

**Issue has no design doc?** Check description for requirements, or create a plan in session notes.

**Tests failing?** That's expected if you're doing TDD right. Implement to make them pass.

**Need to understand code?** Use ripgrep:
```bash
# Find where something is used
rg "function_name" crates/

# Find examples of a pattern
rg "resolve_issue_id" --type rust
```

## Pro Tips

- **Use short hashes**: `jit issue show 003f9f8` instead of full UUID
- **Check blocked reasons**: `jit query blocked` shows why issues can't start
- **Follow the gates**: They enforce quality (TDD, tests, clippy, fmt, code-review)
- **Read session notes**: Issues in progress often have `docs/session-*.md` attached
- **Commit often**: Small focused commits with clear messages
- **No hacks**: Code quality matters - if you're tempted to shortcut, add a TODO issue instead

## What JIT Is

A **CLI-first issue tracker** designed for **AI agents** to orchestrate their own work:
- **Dependency DAG** - "Task B needs Task A done first"
- **Quality Gates** - "Tests must pass before marking done"
- **Agent-friendly** - JSON output, atomic operations, clear errors
- **Dogfooding** - We use JIT to build JIT

Everything is in `.jit/` (like `.git/`). Plain JSON files. Version controlled.

## When Stuck

1. Read the linked design doc
2. Check recent commits for similar work
3. Look at test files for examples
4. Ask! (In issue comments, or add session notes)

**That's it!** You're ready to contribute. Pick a ready task and start coding.
