# Troubleshooting Guide

> **Diátaxis Type:** How-To  
> **Last Updated:** 2026-09-21

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

### Issue Blocked by Dependencies or Gates

**Symptom:**
```text
Error: Cannot transition to 'ready': issue blocked by 1 unmet dependencies

Blockers:
  - abc12345 Blocked prerequisite [ready]

To fix:
  - jit graph deps def67890
  - jit issue show abc12345
```

Or, when completing work with pending gates:

```text
Error: Gate validation failed: Cannot transition to 'done': 1 gate(s) not passed

Blockers:
  - code-review [pending]

To fix:
  - jit gate status-all def67890
  - jit gate evaluate def67890 code-review --by <attestor>

Issue transitioned to `gated`. Evaluate or attest the listed gates; passing the
last one moves it to `done`.
```

A manual gate's `evaluate` requires `--by <attestor>` (e.g. `--by
"human:reviewer"`) — the remediation above names it when the blocking gate is
manual. Automated gates run their checker without `--by`.

**Cause:** JIT enforces dependency and gate blockers before state transitions.
Dependencies must be effectively terminal (`done`, `rejected`, or `archived`
from one of those) before dependent work can
become ready or complete. Required gates must pass before an issue can become
`done`.

**Solutions:**

1. **Inspect dependency blockers:**
   ```bash
   jit graph deps <issue-id>
   jit issue show <blocking-issue-id>
   ```
2. **Inspect gate blockers:**
   ```bash
   jit gate status-all <issue-id>
   ```
3. **Resolve the blocker:** complete/reject dependency issues, or pass required
   gates:
   ```bash
   jit gate evaluate <issue-id> <gate-key>                    # auto gate: runs the checker
   jit gate evaluate <issue-id> <gate-key> --by <attestor>    # manual gate: records attestation
   jit gate status-all <issue-id>
   jit issue update <issue-id> --state done   # completes an in_progress issue; no change once done
   ```

For scripts and MCP clients, use `--json` on `issue update`, `issue claim`, or
`issue claim-next`. Blocked transitions include structured `details.blockers`
and `details.remediation` fields so agents do not need to parse prose.

### Issue Already Claimed

**Symptom:**
```
Error: Issue abc123 already claimed by agent:worker-1 until 2026-02-02 17:30:00 UTC
```

**Cause:** Another agent holds an active lease on the issue.

**Solutions:**

1. **Wait for expiration** — A finite lease expires at its TTL and is evicted on the next claim acquisition (an indefinite lease never expires; once its heartbeat is stale, `jit recover` reclaims it)
2. **Check who has it:**
   ```bash
   jit claim status --issue abc123
   ```
3. **Coordinate** — If you know the other agent, ask them to release
4. **Force evict (admin)** — For crashed agents:
   ```bash
   jit claim force-evict <issue-id> --reason "Agent crashed"
   ```

### Lease Expired During Work

**Symptom:** A `jit` write refuses because another agent holds a lease on your issue.

**Cause:** Your lease expired, and another agent claimed the issue.

**Prevention:**
- Use longer TTL: `jit claim acquire <issue> --ttl 3600`
- Renew before expiration: `jit claim renew <issue-id>`
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
  STALE: Lease marked stale (no heartbeat for 75 minutes)
```

**Cause:** Indefinite lease (TTL=0) hasn't received a heartbeat within the built-in claim stale threshold ([Runtime Defaults](../reference/runtime-defaults.md)).

**Solutions:**

1. **Send heartbeat** if still working:
   ```bash
   jit claim heartbeat <issue-id>
   ```
2. **Release** if done:
   ```bash
   jit claim release <issue-id>
   ```
3. **Reclaim** if the agent is gone:
   ```bash
   jit recover
   ```

### Exceeded Indefinite Lease Limit

**Symptom:**
```
Error: Exceeded per-agent limit for indefinite leases.
Agent copilot:worker-1 already has 3 indefinite lease(s) (max: 3).
```

**Cause:** Policy limits prevent too many indefinite leases to avoid deadlocks.

**Solutions:**

1. **Release an existing indefinite lease:**
   ```bash
   jit claim status  # Find your indefinite leases
   jit claim release <issue-id>
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

### Lock Timeout

**Symptom:**
```
Error: Lock timeout: could not acquire exclusive lock on <path> after <wait>
```

The command exits with the dedicated lock-timeout exit code (see
[Exit Codes](../reference/exit-codes.md)).

**Cause:** Another jit process held the lock for longer than this command was
willing to wait. Mutations serialize on the repository lock, so a burst of
concurrent mutations, or one slow mutation, can exhaust a short wait. The default
wait is the lock-acquisition timeout in
[Runtime Defaults](../reference/runtime-defaults.md).

**Solution:** Retry the command, or raise `JIT_LOCK_TIMEOUT` (whole seconds) for
workflows that run many mutations concurrently. If no other jit process is
running, run recovery:
```bash
jit recover
```

This cleans up stale locks by checking if owning PIDs are still running.

### Malformed Lease Record

**Symptom:**
```
Error: Failed to parse lease record at .git/jit/leases/<issue-id>.json
```

**Solution:** The record is that issue's lease alone. Removing it releases the
lease and leaves every other lease intact:
```bash
rm .git/jit/leases/<issue-id>.json
```

### Coordination-State Repair

`jit validate` reports repository validation findings. Use `jit recover` to
clear stale locks, evict expired leases, and reclaim stale indefinite leases.
It also removes orphaned `*.tmp` files, lease stage files included. It never removes `redo.record`
from the scratch directory `.jit/tmp/`: that record is a pending mutation's
commit point. [Stale Temporary Files](../concepts/guarantees.md#stale-temporary-files)
names what is safe to remove by hand.

```bash
# Repair only provably stale coordination data, then validate the repository.
jit recover
jit validate
```

### Interrupted Mutation

A mutation interrupted after its commit point leaves its redo record in
`.jit/tmp/`. The next mutating command applies the record when its session
opens, before its own work, so the interrupted mutation completes; `jit recover`
applies it without other work. Leave the record in place. The record and its
application are defined in
[Publication and Recovery](../reference/storage-format.md#publication-and-recovery).

### Leftover Record Fails to Apply

**Symptom:**
```
Error: applying the leftover redo record failed: <cause>
```

`jit recover` prints `Recovery failed:` in place of `Error:`, and under `--json`
reports `recovery_failed` where other mutating commands report `GENERIC_ERROR`.

**Cause:** Session open could not apply the record, for example because a path
it writes is not writable or a file occupies a directory it creates. The record
stays in place.

**Solution:** Fix the named cause and rerun the command. Application can be
repeated, so the part already applied does no harm.

### Unreadable Leftover Record

**Symptom:**
```
Error: leftover redo record tmp/redo.record is not readable: <reason>
```

Every mutating command fails this way until the record is dealt with;
`jit recover` reports it in the recovery-failure form described above. Read-only
commands still run and show the repository without the pending mutation. JIT refuses the record rather than skipping it, because a
record damaged after it was committed may belong to a partly applied mutation.

**Solution:** Copy the record (`redo.record` in `.jit/tmp/`) outside `.jit/`,
then act on the reason:

- `redo record carries no header line` or `redo record holds fewer content
  bytes than its header declares`: what an interrupted write of the record
  leaves. The record is written before any other file of its mutation, so an
  interrupted record write means the mutation changed nothing. Remove the record
  (`rm .jit/tmp/redo.record`) and rerun the interrupted command. A complete
  record cut short later by something other than JIT gives the same reasons,
  and JIT cannot tell the two apart: remove the record only when the last JIT
  mutation was interrupted (crash, kill, power loss) and nothing has touched the
  record since. Otherwise, treat it as the last case.
- `redo record format version <N> is not supported`: a JIT build with a
  different record format wrote it. Run a build that reads that format; it
  applies the record.
- Any other reason: an interrupted write cannot produce it, so the record was
  complete and was changed afterwards. Its mutation may be partly applied, and
  JIT cannot finish it. Restore the repository from its last commit or backup
  (`git status` shows what changed), which discards the mutation along with any
  other uncommitted work, then remove the record and run `jit validate`.

---

## Git Hook Issues

### Divergence Error

**Symptom:**
```
error: Pre-commit hook: Global .jit/ changes require common history with main
   Your branch has diverged from origin/main
```

**Cause:** You're modifying shared repository configuration (`.jit/`) while diverged from main.

**Solution:**
```bash
git fetch origin
git rebase origin/main
git commit  # Retry
```

### Hook Not Running

**Symptom:** Hooks don't enforce anything.

**Cause:** The hook isn't installed or enforcement is off. The hook checks only branch divergence on shared configuration; leases are enforced when a mutation is written, and a hand edit of an issue record is not caught at commit.

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

# This worktree command loads or creates its identity.
jit worktree info
```

Over a data root outside every checkout, the file is
`"$(git rev-parse --absolute-git-dir)/jit/worktree.json"` (see
[Worktree Identity](../reference/storage-format.md#worktree-identity)).

### Secondary Worktree Can't See Issues

**Symptom:** `jit query all` returns nothing in secondary worktree.

**Cause:** The aggregated read path could not find the issue in this checkout's
store or the primary checkout's store.

**Solutions:**

1. **Confirm this is a linked Git worktree:**
   ```bash
   git worktree list
   git rev-parse --git-common-dir
   ```

2. **Verify the issue record is in one of the two stores:**
   ```bash
   git worktree list
   ls .jit/issues/
   ls "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/.jit/issues/"
   ```

3. **If the issue exists only in the primary checkout,** keep that checkout
   available at the path Git reports. Linked-checkout reads fall back to it
   automatically when the standard Git layout can be resolved.

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

2. **Agent identity file** (`~/.config/jit/agent.toml`):
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

# 2. Apply any leftover redo record at session open, then run the repair routines
jit recover

# 3. Verify fixed
jit validate

# 4. Check claims are consistent
jit claim list
```

### Emergency: Clear All Leases

**Warning:** Only do this if coordination is completely broken.

```bash
# Backup first
cp -r .git/jit/leases .git/jit/leases.bak

# Clear every lease; claims.jsonl keeps the history
rm -r .git/jit/leases
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
