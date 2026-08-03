# Release Policy

> **Diátaxis Type:** Reference

The procedures a release is produced under: how the product version is
declared and verified, what a release publishes, and which Rust release the
workspace is built on. The version an adopter compares against, the
capabilities it aligns, and what an upgrade replaces are the
[compatibility record](compatibility.md).

## Product version

One product version spans the CLI, the server, the MCP package, the web
bundle, their lock metadata, the release tag, and the compatibility record.
`scripts/release-version-contract.py` takes the version the `jit` manifest
declares and requires every one of those declarations to agree with it,
together with the release's legal and narrative metadata: the license texts the
manifest expression names, the changelog entry for the declared version, the
compatibility-and-upgrade record, the committed release-note source the
publication workflow renders, and the compatibility range this repository's
workflow profile package declares, which has to admit the derived version.

```bash
python3 scripts/release-version-contract.py             # the verdict
python3 scripts/release-version-contract.py --declared  # the agreed version alone
python3 scripts/release_version_contract_test.py        # the command's own tests
```

CI runs the verdict form on every push and pull request
(`.github/workflows/ci.yml`). The publication workflow runs the `--declared`
form on the tag, adding `--tag` so the pushed tag has to name the version the
manifests already agree on; it then names the release note after the value it
printed, which is how the workflow carries no version literal of its own. A
disagreeing tree fails before that value is printed.

Raising the product version therefore means raising it in every manifest and
lock, adding the changelog entry, updating the compatibility record, and
committing the release note for the new version — the command names each
declaration that still disagrees.

## What a release publishes

A pushed `v*` tag is the only trigger, and one GitHub release for that tag is
the only output (`.github/workflows/release-publish.yml`). The workflow creates,
moves, and deletes no git ref: a maintainer owns the annotated tag, and
publication verifies that annotation before uploading anything.

The release carries these assets:

| Asset | Contents | Installed by |
| --- | --- | --- |
| native archive | the `jit` CLI, the `jit-server` binary, both license texts, the `jit-default` vocabulary package under `packages/jit-default/`, and the assembled `jit-dogfood` workflow package under `packages/jit-dogfood/`, built for Linux x86_64 against musl | [Installation Guide](../../INSTALL.md#pre-built-binaries) |
| MCP server tarball | the packaged MCP server | [MCP Integration](../how-to/mcp-integration.md) |
| checksum file | SHA-256 sums covering both archives, computed where the assets are assembled | [Installation Guide](../../INSTALL.md#download-and-verify) |
| license texts | the two texts the manifest expression names, published beside the archives that also carry them | downloaded from the release page |

The committed release note for the declared version is the release body, so
each version's published narrative is reviewed in the repository rather than
written at the tag.

That is the complete published output. `gh release create` in that workflow is
the only publishing step the repository allows — `.github/workflow-contract.yml`
names it and rejects a release or a registry push anywhere else — so there is no
package-registry version, no container-registry image, and no separate web
bundle. The web UI is compiled into `jit-server`, and a [container
deployment](../how-to/deployment.md#container-deployment) builds its image from
a checkout.

Every artifact is built and smoked downstream of the validation suites and the
security audits (`.github/workflows/release-artifacts.yml`), so nothing is
released that was not validated, audited, and started from the archive an
adopter downloads. That build also runs on every pull request, which keeps the
artifact-producing path exercised continuously rather than first on a tag.

## Compatibility and upgrades

[The compatibility record](compatibility.md) is the adopter-facing statement of
the product compatibility version, the capabilities that version aligns, and
what an upgrade replaces. It is also a checked input rather than prose beside
the policy: the version contract above reads that file, requires it to declare
exactly one product compatibility version matching the manifests, and requires
it to carry each supported capability and its upgrade expectations. Recording
those facts anywhere else would leave the checked copy behind.

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

CI's `msrv` job runs the command on every push and pull request. It takes the
declared version as the compiler it installs, checks the window, then builds and
tests the committed workspace on that compiler against the committed
`Cargo.lock`.

A release tag is created by a maintainer, who runs the same command at the
commit that tag will point at, immediately before creating it. A tag therefore
carries a declaration that was inside the window at creation, and a verdict
outside the window is answered by [refreshing the declaration](#refreshing-the-declaration)
before the tag exists.

The command judges a tree rather than an environment — `--root` names the tree,
the verdict goes to stdout — so both runs make the same comparison and each can
record the result it acted on.

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
