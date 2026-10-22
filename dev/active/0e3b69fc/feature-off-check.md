# Feature-off build evidence for 0e3b69fc

REQ-02 evidence: the reference-page renderers compile only with the `test-support` feature, and a warnings-denied build without that feature is clean.

Recorded by the execution lead on main at `dcf00b1d8` (after merge `5f3e8140d` of this issue), `CARGO_INCREMENTAL=0`, under the shared CI lock.

## `RUSTFLAGS=-Dwarnings cargo check --workspace` (feature off)

```text
    Checking axum v0.7.9
    Checking axum-server v0.8.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 19.29s
```

No warning or error was emitted. Resolver 2 does not unify dev-dependency features into a non-test `cargo check`, so `jit` builds without `test-support` here. The worker confirmed this with `cargo check -v`.

## `cargo check -p jit --features test-support`

```text
    Checking jsonschema v0.46.2
    Checking jit v1.0.0 (/home/vkaskivuo/Projects/just-in-time/crates/jit)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.90s
```
