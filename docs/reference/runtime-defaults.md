<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent coordination and startup recovery. Each
value is defined once in the `crates/jit/src/runtime_defaults.rs` module,
and this reference is generated from it.

| Default | Value | Scope |
| --- | --- | --- |
| Heartbeat interval | 30 seconds | Default interval between lease heartbeat updates; a lease heartbeat is treated as stale after twice this interval. |
| Lock acquisition timeout | 5 seconds | Maximum time a writer waits for a `.jit` file lock or the repository write lock before failing. Override with the `JIT_LOCK_TIMEOUT` environment variable. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Age at which orphaned `*.tmp` files are swept during startup recovery. |
| Claim lease TTL | 600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
