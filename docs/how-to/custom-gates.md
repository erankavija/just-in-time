# How-To: Custom Gates

> **Status:** Draft - Story 5326b331  
> **Diátaxis Type:** How-To Guide

## Create Your First Gate

<!-- jit gate define -->

## Manual vs Automated Gates

<!-- When to use each -->

## Write Gate Checker Scripts

<!-- Best practices, exit codes -->

## Prechecks vs Postchecks

<!-- Stage selection strategy -->

## Gate Presets and Templates

<!-- Reusable configurations -->

## Troubleshooting Gate Failures

### Common Issues and Solutions

#### "Repository not initialized"

```bash
Error: .jit directory not found
```

**Solution:** Run `jit init` in your project directory first.

#### "Cycle detected"

```bash
Error: Adding dependency would create a cycle
```

**Solution:** Check your dependency graph with `jit graph show` and remove circular references. Dependencies must form a directed acyclic graph (DAG).

#### "Invalid label format"

```bash
Error: Invalid label format: 'milestone-v1.0'
Expected format: 'namespace:value'
```

**Solution:** Use colon separator: `--label "milestone:v1.0"`

#### "Missing type label"

```bash
Warning: Issue created without type label
```

**Solution:** Add type label: `jit issue update $ISSUE --label "type:task"`

#### "Orphaned task"

```bash
Warning: Issue is an orphaned task (no epic:* or milestone:* label)
```

**Solution:** Add parent label: `jit issue update $ISSUE --label "epic:auth"` or use `--orphan` flag to explicitly allow orphaned issues.

### Validation and Recovery

```bash
# Check repository health
jit validate

# Automatically fix issues
jit validate --fix

# Preview fixes without applying
jit validate --fix --dry-run
```

### Getting Help

```bash
# Command-specific help
jit issue create --help
jit dep add --help
jit gate define --help

# List available commands
jit --help

# Check configured label namespaces
jit label namespaces

# View existing label values
jit label values milestone
jit label values epic
```
