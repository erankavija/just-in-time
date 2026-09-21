<!-- Generated from `crate::runtime_defaults` — do not edit by hand. -->

# Runtime Coordination Defaults

Built-in defaults for multi-agent lease coordination and recovery cleanup. This
reference is generated from the `crates/jit/src/runtime_defaults.rs`
module, which defines these values.

| Default | Value | Scope |
| --- | --- | --- |
| Lock acquisition timeout | 60 seconds | Default wait for acquiring a file lock before failing. Every lock jit acquires, including the repository lock chain, the shared `.jit` file locks, and claim coordination, resolves its wait from the `JIT_LOCK_TIMEOUT` environment variable (whole seconds) when set, falling back to this default. A wait that expires exits with the dedicated lock-timeout exit code. |
| Lock poll interval | 10 milliseconds | Wait between successive attempts while blocking on a contended file lock. |
| Temp-file cleanup threshold | 3600 seconds | Minimum age at which `jit recover` removes an orphaned `*.tmp` file under `.jit/`. A pending redo record is not swept: every session open applies a leftover record (see [Publication and Recovery](storage-format.md#publication-and-recovery)). |
| Claim lease TTL | 3600 seconds | Default time-to-live for a lease from `jit claim acquire`, and the default extension for `jit claim renew`. |
