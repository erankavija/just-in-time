<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent lease coordination and recovery cleanup. This
reference is generated from the `crates/jit/src/runtime_defaults.rs`
module, which defines these values.

| Default | Value | Scope |
| --- | --- | --- |
| Lock acquisition timeout | 5 seconds | Default timeout for acquiring a file lock before failing. `JsonFileStorage` resolves one timeout from the `JIT_LOCK_TIMEOUT` environment variable when set, falling back to this default, and applies it to every lock it acquires — both its repository write lock and the shared `FileLocker` it uses for all other `.jit` file locks. The claim-coordination and worktree locks build their own `FileLocker` from this constant and ignore the environment variable. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Minimum age at which `cleanup_orphaned_temp_files` sweeps an orphaned `*.tmp` file. Passed by the `jit recover` command, the single recovery entry point. |
| Claim lease TTL | 600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
