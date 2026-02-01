# Git Hooks for JIT Enforcement

This directory contains git hook templates for enforcing lease requirements and branch divergence rules.

## Quick Install

```bash
jit hooks install
```

This command automatically:
- Copies hooks to `.git/hooks/`
- Makes them executable
- Skips hooks that already exist (won't overwrite)

## Available Hooks

### pre-commit

**Purpose:** Validates before commit:
1. Global operations (.jit/config, gates/registry) require common history with main
2. Structural issue edits require active, non-stale lease (in strict mode)

**Behavior by enforcement mode:**
- `off`: Hook exits immediately (no checks)
- `warn`: Not applicable (hook would just log warnings)
- `strict`: Full validation, blocks commit if checks fail

**Installation:**
```bash
# Recommended: Use jit command
jit hooks install

# Or manual copy:
cp scripts/hooks/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

**Emergency override:**
```bash
git commit --no-verify
```

### pre-push

**Purpose:** Validates before push that leases for modified issues are still active.

Prevents pushing work where leases have expired during development.

**Behavior by enforcement mode:**
- `off`: Hook exits immediately
- `warn`: Hook exits immediately
- `strict`: Full validation, blocks push if leases expired

**Installation:**
```bash
# Recommended: Use jit command
jit hooks install

# Or manual copy:
cp scripts/hooks/pre-push .git/hooks/pre-push
chmod +x .git/hooks/pre-push
```

**Emergency override:**
```bash
git push --no-verify
```

## Requirements

Both hooks require:
- `jq` command-line JSON processor
- `.jit/config.toml` with `[worktree]` section (optional)
- `.git/jit/claims.index.json` (for strict mode lease checks)

If `jq` is not installed, hooks will warn and skip lease validation.

## Configuration

Enforcement mode is configured in `.jit/config.toml`:

```toml
[worktree]
enforce_leases = "strict"  # or "warn" or "off"
```

**Default (no config):** `off` - hooks do nothing

## Design Principles

1. **Defense in depth:** Hooks are the second layer after CLI enforcement
2. **Fail-safe:** Missing dependencies (jq, config) cause warnings, not errors
3. **Override available:** `--no-verify` flag for emergencies
4. **Mode-aware:** Respect enforcement configuration

## Worktree Setup

To share hooks across all worktrees:

```bash
# Configure git to use shared hooks directory
git config core.hooksPath .git/hooks

# This makes hooks work in all worktrees created from this repository
```

## Testing

Test the hooks before relying on them:

```bash
# Test pre-commit (without actually committing)
.git/hooks/pre-commit

# Test pre-push (dry run)
git push --dry-run origin feature-branch
```

## Troubleshooting

**Hook doesn't run:**
- Check if file is executable: `ls -la .git/hooks/pre-commit`
- Make executable: `chmod +x .git/hooks/pre-commit`

**Hook blocks valid commit:**
- Check enforcement mode: `grep enforce_leases .jit/config.toml`
- Check lease status: `jit claim status`
- Override if necessary: `git commit --no-verify`

**Hook allows invalid commit:**
- Verify hook is installed: `cat .git/hooks/pre-commit`
- Check if jq is installed: `which jq`
- Verify enforcement mode is set: `cat .jit/config.toml`
