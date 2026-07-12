<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent coordination and startup recovery. This
reference is generated from the `crates/jit/src/runtime_defaults.rs`
module, which defines these values.

| Default | Value | Scope |
| --- | --- | --- |
| Heartbeat interval | 30 seconds | Default value of the agent `heartbeat_interval` setting — the recommended cadence at which an agent sends `jit claim heartbeat` to keep an indefinite (TTL=0) lease alive. jit does not run a heartbeat loop itself; the command records a single beat on demand. Lease staleness is governed separately: an indefinite lease is marked stale only after the claim staleness threshold (1 hour) without a beat, not after this interval. Finite leases expire on their own TTL instead (see Claim lease TTL). |
| Lock acquisition timeout | 5 seconds | Default timeout to acquire a file lock before failing. The `.jit` repository storage locks (held by `JsonFileStorage` for all `.jit` data writes) resolve their timeout from the `JIT_LOCK_TIMEOUT` environment variable when set, falling back to this default; the claim-coordination file locks use this default only and ignore the environment variable. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Age at which orphaned `*.tmp` files are swept during startup recovery. |
| Claim lease TTL | 600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
