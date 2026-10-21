<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent lease coordination and recovery cleanup. This
reference is generated from the `crates/jit/src/runtime_defaults.rs`
module, which defines these values.

| Default | Value | Scope |
| --- | --- | --- |
| Lock acquisition timeout | 60 seconds | Default wait for acquiring a file lock before failing. Every lock jit acquires, including the repository lock chain, the shared `.jit` file locks, and claim coordination, resolves its wait from the `JIT_LOCK_TIMEOUT` environment variable (whole seconds) when set, falling back to this default. A wait that expires exits with the dedicated lock-timeout exit code. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Minimum age at which `jit recover` removes an orphaned `*.tmp` file under `.jit/`. A pending redo record is not swept: the next mutation session applies it (see [Publication and Recovery](storage-format.md#publication-and-recovery)). |
| Claim lease TTL | 3600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
| Claim stale threshold | 3600 seconds | Time without a heartbeat after which an indefinite lease is stale. A stale lease no longer counts as active for lease enforcement and is not evicted automatically. |
| Indefinite leases per agent | 2 | Default maximum concurrent indefinite (`--ttl 0`) leases one agent holds; `jit claim acquire` refuses beyond it. Overridden by `[coordination] max_indefinite_leases_per_agent`. |
| Indefinite leases per repository | 10 | Default maximum concurrent indefinite leases across the repository. Overridden by `[coordination] max_indefinite_leases_per_repo`. |
| Lease enforcement mode | strict | Mode applied when the repository's `[worktree]` section omits `enforce_leases`. A repository with no `[worktree]` section enforces no leases. |
| Linked-checkout write policy | refuse | Stance on state-mutating commands run inside a linked non-primary checkout when the repository declares no `[worktree] write_policy`, whether the key or the whole section is absent. The `JIT_WORKTREE_WRITE_POLICY` environment variable outranks it for one invocation. |
