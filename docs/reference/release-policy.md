# Release Policy

> **Diátaxis Type:** Reference

The policies a release is produced under. The product version, the capabilities
it aligns, and what an upgrade replaces are the
[compatibility record](compatibility.md).

## Supported Rust version

The workspace declares one `rust-version` under `[workspace.package]` in
`Cargo.toml`, and every crate inherits it with `rust-version.workspace = true`,
so a single minimum supported Rust version (MSRV) covers the whole workspace.

`scripts/rust-version-policy.py` derives both sides of the policy each time it
runs:

| Value | Source |
| --- | --- |
| declared MSRV | `cargo metadata --no-deps`, which reports the `rust_version` of every workspace package |
| current stable | the `rust` package in the Rust release channel manifest at `https://static.rust-lang.org/dist/channel-rust-stable.toml`, the manifest rustup resolves the stable channel through |

Each number therefore has one home: the workspace manifest and the release
channel.

### The recency window

Current stable may be at most one minor release ahead of the declaration, and
the declaration names a release the stable channel has reached. A patch release
is release-train maintenance rather than a stable-release interval, so the
window is measured in minor releases alone.

The command writes its verdict to stdout as one JSON object — the declared
version, the derived stable version, the distance between them, the window that
distance was judged against, and the resulting status — so a caller can record
the exact comparison it acted on. Findings go to stderr.

| Exit code | Meaning |
| --- | --- |
| 0 | the declaration is inside the window |
| 1 | the declaration is outside the window |
| 2 | a version could not be derived, so no comparison was made |

Exit code 2 covers an unreachable release channel and a workspace whose
packages declare nothing or disagree. Each says the declaration was not judged,
which is separate from judging it and finding it stale.

### Running it

```bash
python3 scripts/rust-version-policy.py             # the verdict
python3 scripts/rust-version-policy.py --declared  # the declared version alone
python3 scripts/rust_version_policy_test.py        # the command's own tests
```

Two callers run it. CI's `msrv` job takes the declared version as the compiler
it installs, checks the window, and then builds and tests the committed
workspace on exactly that compiler against the committed `Cargo.lock`. The
pre-tag check runs the command against the commit a release tag will point at,
so a tag carries a declaration that was inside the window when it was created.

### Refreshing the declaration

Exit code 1 is answered in the workspace manifest:

1. Raise `rust-version` under `[workspace.package]` in `Cargo.toml` to a release
   inside the window — ordinarily the stable version the command reported.
2. Install that toolchain and run the workspace on it:

   ```bash
   rustup toolchain install <version>
   cargo +<version> build --workspace --all-targets --all-features --locked
   cargo +<version> test --workspace --all-features --locked
   ```

3. Record the raised MSRV in `CHANGELOG.md`.
4. Rerun `python3 scripts/rust-version-policy.py`. It now reports
   `within-window`, and CI installs the raised compiler on the next run.
