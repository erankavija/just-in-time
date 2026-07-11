<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent coordination and startup recovery. Each
value is what jit uses when nothing overrides it; the source of truth is
the `crates/jit/src/runtime_defaults.rs` module, which every production
call site reads.

| Default | Value | Scope |
| --- | --- | --- |
| Heartbeat interval | 30 seconds | Cadence at which the optional auto-heartbeat daemon renews an indefinite (TTL=0) lease. |
| Lock acquisition timeout | 5 seconds | Maximum time a writer waits for a `.jit` file lock or the repository write lock before failing. Override with the `JIT_LOCK_TIMEOUT` environment variable. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Age at which orphaned `*.tmp` files are swept during startup recovery. |
| Claim lease TTL | 600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
