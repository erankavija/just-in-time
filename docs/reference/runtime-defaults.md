<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent coordination and startup recovery. This
reference is generated from the `crates/jit/src/runtime_defaults.rs`
module, which defines these values.

| Default | Value | Scope |
| --- | --- | --- |
| Heartbeat interval | 30 seconds | Default cadence for the heartbeat updates that keep an indefinite (TTL=0) lease alive; the heartbeat helper marks a heartbeat stale after twice this interval. Finite claim leases expire on their own TTL instead (see Claim lease TTL). |
| Lock acquisition timeout | 5 seconds | Default timeout to acquire a file lock before failing. The repository storage write lock additionally honors the `JIT_LOCK_TIMEOUT` environment override; other file locks use this fixed default. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Age at which orphaned `*.tmp` files are swept during startup recovery. |
| Claim lease TTL | 600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
