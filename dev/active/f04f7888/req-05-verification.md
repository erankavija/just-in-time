# REQ-05 verification record — f04f7888

**Recorded:** 2026-07-29 UTC
**Scope:** `Implement graceful jit-server process shutdown`, REQ-05 only.

This is point-in-time evidence for the integrated shutdown dependency graph. It
does not replace the audit and declared-MSRV checks required when a release
candidate or tag is built; those checks must be rerun against that candidate's
committed lockfile.

## Committed inputs inspected

| Input | Observation |
| --- | --- |
| Workspace MSRV | `Cargo.toml:7` declares `rust-version = "1.97"`. |
| Server MSRV | `crates/server/Cargo.toml:5` inherits that declaration with `rust-version.workspace = true`. |
| Declared shutdown dependencies | `crates/server/Cargo.toml:29` declares `axum-server = "0.8"`; line 35 declares `tokio-util = "0.7"`. |
| Locked versions | `Cargo.lock:250-253` pins `axum-server` to `0.8.0` (checksum `b1df331683d982a0b9492b38127151e6453639cd34926eb9c07d4cd8c6d22bfc`); `Cargo.lock:2770-2773` pins `tokio-util` to `0.7.18` (checksum `9ae9cec805b01e8fc3fd2fe289f89149a9b66dd16786abd8b19cfa7b48cb0098`). |

`cargo +1.97.0 metadata --no-deps --format-version 1` also reports
`rust_version: "1.97"` for both workspace packages and lists the server's
`^0.8` `axum-server` and `^0.7` `tokio-util` requirements. This is a manifest
consistency check; the lockfile entries above are the exact resolved versions.

## Recorded execution results

The execution lead ran the following commands against the current integrated
code on 2026-07-29 UTC. Each exited 0.

| Check | Command and recorded result |
| --- | --- |
| Toolchain identity | Exact installed toolchain: `1.97.0-x86_64-unknown-linux-gnu`; `rustc 1.97.0 (2d8144b78 2026-07-07)`. |
| Security audit | `cargo audit -D warnings` — loaded 1,173 RustSec security advisories; scanned `Cargo.lock` with 386 crate dependencies; no warnings or findings. |
| Declared-MSRV compile | `env CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/f04f7888-msrv-target cargo +1.97.0 build --workspace` — isolated target directory and incremental compilation disabled. It compiled `tokio-util v0.7.18`, `axum-server v0.8.0`, `jit v0.2.1`, and `jit-server v0.1.0`, then completed the dev build successfully. |

## Advisory exception and deferred-work sweep

No Cargo-audit configuration, `deny.toml`, advisory-ignore file, or
advisory-allowlist file is tracked. A tracked-file-name sweep and a repository
content sweep for RustSec/cargo-audit advisory `ignore` and `allowlist` forms
found no such exception. The only active matching prose is the release scope
brief's requirement that no blanket advisory allowlist be introduced; it is
not configuration and grants no exception.

`jit doc list f04f7888` reports no linked documents. Before this record,
`dev/active/f04f7888/` did not exist. The upstream active epic handoff has one
`## Open questions needing invoker input` marker; it says none are blocking
and contains no shutdown, dependency, audit, or MSRV deferral. No deferred
marker changes REQ-05's checks.
