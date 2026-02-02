# Troubleshooting Guide

> **Diátaxis Type:** How-To  
> **Last Updated:** 2026-02-02

Solutions for common issues with parallel work and claim coordination.

## Quick Diagnostics

```bash
# Check overall repository health
jit validate

# Run automatic recovery
jit recover

# Check active leases
jit claim list

# Check worktree status
jit worktree list
```

---

## Lease and Claim Issues

### Issue Already Claimed

**Symptom:**
```
Error: Issue abc123 already claimed by agent:worker-1 until 2026-02-02 17:30:00 UTC
```

**Cause:** Another agent holds an active lease on the issue.

**Solutions:**

1. **Wait for expiration** — Leases expire automatically after TTL
2. **Check who has it:**
   ```bash
   jit claim status --issue abc123
   ```
3. **Coordinate** — If you know the other agent, ask them to release
4. **Force evict (admin)** — For crashed agents:
   ```bash
   jit claim force-evict <lease-id> --reason "Agent crashed"
   ```

### Lease Expired During Work

**Symptom:** Pre-commit hook rejects your commit, or another agent claimed your issue.

**Cause:** Your lease expired before you committed.

**Prevention:**
- Use longer TTL: `jit claim acquire <issue> --ttl 3600`
- Renew before expiration: `jit claim renew <lease-id>`
- Use indefinite lease for manual work: `jit claim acquire <issue> --ttl 0 --reason "..."`

**Recovery:**
1. Check if issue is still available:
   ```bash
   jit claim status --issue <issue-id>
   ```
2. Re-acquire if available:
   ```bash
   jit claim acquire <issue-id>
   ```

### Stale Indefinite Lease

**Symptom:**
```
⚠️  STALE: Lease marked stale (no heartbeat for 75 minutes)
```

**Cause:** Indefinite lease (TTL=0) hasn't received heartbeat within threshold (default 1 hour).

**Solutions:**

1. **Send heartbeat** if still working:
   ```bash
   jit claim heartbeat <lease-id>
   ```
2. **Release** if done:
   ```bash
   jit claim release <lease-id>
   ```
3. **Force evict** if agent is gone:
   ```bash
   jit claim force-evict <lease-id> --reason "Agent abandoned"
   ```

### Exceeded Indefinite Lease Limit

**Symptom:**
```
Error: Exceeded per-agent limit for indefinite leases.
Agent copilot:worker-1 already has 2 indefinite lease(s) (max: 2).
```

**Cause:** Policy limits prevent too many indefinite leases to avoid deadlocks.

**Solutions:**

1. **Release an existing indefinite lease:**
   ```bash
   jit claim status  # Find your indefinite leases
   jit claim release <lease-id>
   ```
2. **Use finite TTL instead:**
   ```bash
   jit claim acquire <issue> --ttl 3600  # 1 hour
   ```
3. **Increase limit** (if appropriate) in `.jit/config.toml`:
   ```toml
   [coordination]
   max_indefinite_leases_per_agent = 5
   ```

---

## Lock and Recovery Issues

### Stale Lock File

**Symptom:**
```
Error: Failed to acquire lock - timed out after 5 seconds
```

**Cause:** A previous process died while holding a lock.

**Solution:** Run recovery:
```bash
jit recover
```

This cleans up stale locks by checking if owning PIDs are still running.

### Corrupted Claims Index

**Symptom:**
```
Error: Failed to parse claims index
```

Or inconsistent claim status.

**Solution:** Rebuild from audit log:
```bash
jit recover
```

The claims index can always be rebuilt from the append-only `claims.jsonl` log.

### Validation Errors

**Symptom:**
```bash
$ jit validate
✗ Validation failed with 2 issues:
  - Orphaned lock file: .git/jit/locks/claims.lock
  - Index sequence gap at position 42
```

**Solution:**
```bash
# Dry run to see what would be fixed
jit validate --fix --dry-run

# Apply fixes
jit validate --fix
```

---

## Git Hook Issues

### Pre-Commit Hook Rejection

**Symptom:**
```
❌ Pre-commit hook: Structural edit to issue abc123 without active lease
```

**Cause:** You're editing an issue file without holding a lease (in strict mode).

**Solutions:**

1. **Acquire a lease:**
   ```bash
   jit claim acquire abc123
   git commit  # Retry
   ```

2. **Set your agent identity:**
   ```bash
   export JIT_AGENT_ID='agent:your-name'
   ```
   Or configure in `~/.config/jit/agent.toml`:
   ```toml
   id = "agent:your-name"
   ```

3. **Bypass (emergency only):**
   ```bash
   git commit --no-verify
   ```

### Divergence Error

**Symptom:**
```
❌ Pre-commit hook: Global .jit/ changes require common history with main
   Your branch has diverged from origin/main
```

**Cause:** You're modifying global config while diverged from main.

**Solution:**
```bash
git fetch origin
git rebase origin/main
git commit  # Retry
```

### Hook Not Running

**Symptom:** Hooks don't enforce anything.

**Cause:** Hooks not installed or enforcement is off.

**Solution:**
1. Install hooks:
   ```bash
   jit hooks install
   ```
2. Enable enforcement in `.jit/config.toml`:
   ```toml
   [worktree]
   enforce_leases = "strict"  # or "warn"
   ```

---

## Worktree Issues

### Worktree Not Detected

**Symptom:**
```
Error: Failed to detect worktree paths
```

**Cause:** Not in a git repository, or git worktree not properly set up.

**Solutions:**

1. **Verify you're in a git repo:**
   ```bash
   git status
   ```

2. **Check worktree setup:**
   ```bash
   git worktree list
   ```

3. **Recreate worktree if corrupted:**
   ```bash
   git worktree remove <path>
   git worktree add <path> <branch>
   ```

### Missing Worktree Identity

**Symptom:** Claims work but worktree ID is empty or wrong.

**Solution:** Regenerate identity:
```bash
# Remove old identity
rm .jit/worktree.json

# Any jit command will regenerate it
jit status
```

### Secondary Worktree Can't See Issues

**Symptom:** `jit query all` returns nothing in secondary worktree.

**Cause:** Worktree mode disabled or path detection failed.

**Solutions:**

1. **Check worktree mode:**
   ```bash
   jit config show worktree.mode
   ```
   Should be `"auto"` or `"on"`.

2. **Verify shared jit directory:**
   ```bash
   ls -la .git  # Should show "gitdir: /path/to/main/.git/worktrees/..."
   ls -la $(cat .git)/jit  # Should exist
   ```

3. **Enable worktree mode:**
   ```bash
   # In main worktree:
   jit config set worktree.mode on
   ```

---

## Agent Identity Issues

### No Agent Identity

**Symptom:**
```
Error: No agent identity configured
```

**Cause:** Agent ID not set via any method.

**Solutions (in priority order):**

1. **Environment variable:**
   ```bash
   export JIT_AGENT_ID='agent:your-name'
   ```

2. **User config** (`~/.config/jit/agent.toml`):
   ```toml
   id = "agent:your-name"
   ```

3. **CLI flag:**
   ```bash
   jit claim acquire <issue> --agent-id "agent:your-name"
   ```

### Wrong Agent Showing

**Symptom:** Leases show unexpected agent ID.

**Cause:** Environment or config override.

**Debug:**
```bash
# Check what agent ID is being used
echo $JIT_AGENT_ID
cat ~/.config/jit/agent.toml
```

---

## Recovery Procedures

### Full Recovery

When things are badly broken:

```bash
# 1. Check what's wrong
jit validate

# 2. Run automatic recovery
jit recover

# 3. Verify fixed
jit validate

# 4. Check claims are consistent
jit claim list
```

### Manual Index Rebuild

If automatic recovery fails:

```bash
# The claims index is derived from claims.jsonl
# You can manually rebuild by:
rm .git/jit/claims.index.json
jit claim list  # Triggers rebuild
```

### Emergency: Clear All Leases

**Warning:** Only do this if coordination is completely broken.

```bash
# Backup first
cp .git/jit/claims.jsonl .git/jit/claims.jsonl.bak
cp .git/jit/claims.index.json .git/jit/claims.index.json.bak

# Clear (loses all lease history)
rm .git/jit/claims.jsonl
rm .git/jit/claims.index.json

# Reinitialize
jit claim list  # Creates empty index
```

---

## Getting Help

If issues persist:

1. **Check logs:**
   ```bash
   jit events tail -n 50
   ```

2. **Run with debug output:**
   ```bash
   RUST_LOG=debug jit <command>
   ```

3. **File an issue** with:
   - Output of `jit validate`
   - Output of `jit recover`
   - Relevant log entries

## See Also

- [Multi-Agent Coordination](multi-agent-coordination.md) - Usage patterns
- [Configuration Reference](../reference/configuration.md) - All settings
- [jit claim Reference](../reference/claim.md) - Command details
