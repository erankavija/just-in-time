# v1.0 production-readiness external research

**Container:** `8b05a612` · **Planning node:** `e131f1dc`  
**Scope basis:** `dev/active/8b05a612-investigation.md`, the production-readiness scope brief, and charter D-9..D-13.

## 1. Reproducible GitHub release workflow

**Question.** How can the tag workflow prove that tests and security passed, align every product version, and publish a useful but small Linux release?

- **[VERIFIED]** The current tag workflow builds artifacts but does not depend on normal CI or security. Product manifests currently say CLI `0.2.1`, server `0.1.0`, MCP `0.1.0`, and web `0.0.0`; dual licensing is declared but license-text files are absent.
- **[CITED]** GitHub job `needs` dependencies run only after prerequisites succeed, and a failed/skipped prerequisite prevents the dependent job unless its condition overrides that behavior: [workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idneeds). Reusable workflows avoid copying CI into the release file: [reusing workflows](https://docs.github.com/en/actions/reference/workflows-and-actions/reusing-workflow-configurations).
- **[CITED]** `actions/attest@v4` can attest file paths and bind an SPDX JSON SBOM to an artifact with `id-token: write` and `attestations: write`: [GitHub artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).
- **Recommendation [ASSUMED].** Make normal CI and blocking security callable workflows, then use one tag-triggered release job graph: `version-check -> {ci, security} -> build-and-smoke -> publish`. `version-check` strips the leading `v`, rejects non-full SemVer, reads Rust package versions from `cargo metadata` and Node versions from the manifests, and requires one value equal to the tag—no workflow version literal. Build web before `jit-server`, package `jit` and `jit-server` for Linux x86_64 musl, and attach that archive, `LICENSE-MIT`, `LICENSE-APACHE`, SHA-256 checksums, and one SPDX JSON SBOM. Generate SPDX JSON with a pinned Syft/Anchore SBOM action, then attest the archive for provenance and the SBOM association. Smoke the extracted binaries before publication.
- **Rejected.** Relying on branch-protection checks or a previous branch workflow run: it does not encode a job dependency in the tag workflow and can validate a different commit.

## 2. npm trusted publication and package smoke

**Question.** How should the exact MCP tarball be tested and published without a long-lived write token?

- **[VERIFIED]** Release automation currently uses Node 20 and stops at `npm pack`; `mcp-server/package.json` lacks the repository metadata required to bind provenance to this GitHub repository.
- **[CITED]** npm trusted publishing accepts GitHub-hosted Actions through OIDC, requires `id-token: write`, and requires an npm-side publisher binding to the exact repository/workflow; current npm also requires Node >=22.14 and npm >=11.5.1. Eligible public packages receive provenance automatically: [trusted publishers](https://docs.npmjs.com/trusted-publishers/) and [npm provenance](https://docs.npmjs.com/generating-provenance-statements/).
- **Recommendation [ASSUMED].** Configure `@erankavija/jit-mcp-server` with the exact repository URL and an npm trusted publisher for the release workflow/environment. On a GitHub-hosted runner, use a supported Node/npm pair, run tests, produce one tarball with `npm pack --json`, install that tarball into a clean temporary prefix, and run an MCP initialize/list-tools smoke against the installed `jit-mcp-server` executable. Only then publish the same tarball with `npm publish <tarball> --access public`; omit `NODE_AUTH_TOKEN` so OIDC is the only write credential. Record npm's one-time publisher configuration in the release checklist because it is external state.
- **Rejected.** An automation token plus publishing from the source directory: it retains a long-lived secret and may publish a repacked payload different from the tarball that was smoked.

## 3. GHCR tags and attestations

**Question.** How can consumers distinguish immutable releases from mutable development images?

- **[VERIFIED]** Docker automation currently emits branch, full/minor/major SemVer, SHA, and `latest` tags across four image topologies, with no attestations.
- **[CITED]** Docker's metadata action derives event/SemVer/SHA tags, while build-push supports `provenance: mode=max` and requires explicit `sbom: true`: [tags and labels](https://docs.docker.com/build/ci/github-actions/manage-tags-labels/) and [Docker attestations](https://docs.docker.com/build/ci/github-actions/attestations/).
- **Recommendation [ASSUMED].** Publish only the consolidated image. A release tag produces exactly `ghcr.io/<owner>/<repo>:1.0.0`; consumers can additionally pin its digest. A main push produces `:main` and `:sha-<commit>` only. Disable automatic `latest`, major, and minor SemVer aliases. Push release images with `provenance: mode=max` and `sbom: true`, retain OCI source/revision/version labels, and smoke the image on pull requests before any push. The naming makes the sole mutable tag visibly developmental.
- **Rejected.** `latest`, `1`, or `1.0` release aliases: each can move and therefore weakens the full-version/digest contract.

## 4. Rust stable and MSRV recency

**Question.** Is 1.97 current, and how can the tag prove it is no more than one stable release behind without a second literal?

- **[VERIFIED]** Workspace `rust-version` is declared once as `1.97`; both Rust packages inherit it, `cargo metadata --no-deps --format-version 1` reports `1.97`, and existing CI already installs/tests that derived value.
- **[CITED]** Rust 1.97.0 became stable on 2026-07-09, so the declaration is current on 2026-07-14: [official Rust release](https://blog.rust-lang.org/releases/latest/).
- **Recommendation [ASSUMED].** At tag time, derive the one unique `rust_version` from `cargo metadata`; install/query the `stable` toolchain and parse `rustc +stable --version`. Require major `1`, declared minor <= stable minor, and `stable_minor - declared_minor <= 1`; patch releases do not consume a stable-release interval. Keep the existing exact-declared-toolchain build as the compatibility proof. Document this one algorithm, not the current stable number.
- **Rejected.** Hard-coding `1.97` or an expected stable number in workflow YAML, which creates a second version source and goes stale at the next Rust release.

## 5. Production dependency audit semantics

**Question.** Which commands make all current production findings release-blocking?

- **[VERIFIED]** Security automation currently suppresses audit-tool installation failure, marks all audits `continue-on-error`, uses `npm audit --production`, and is absent from pull requests and release dependencies.
- **[CITED]** RustSec directs users to install with `cargo install cargo-audit --locked`, run `cargo audit` at the project top level, and prefer upgrading over ignores: [cargo-audit README](https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md). The current CLI can deny informational warnings such as unmaintained/unsound/yanked dependencies with `-D warnings`.
- **[CITED]** Modern npm accepts `--omit=dev`, omits that dependency class from the audit payload/report, and exits zero only when no vulnerability at the configured threshold is found: [npm audit v11](https://docs.npmjs.com/cli/v11/commands/npm-audit/).
- **Recommendation [ASSUMED].** Fail hard on installing a pinned/locked cargo-audit and run `cargo audit -D warnings` at the workspace root, with no ignore list. In both `web/` and `mcp-server/`, run `npm audit --omit=dev --audit-level=info` against committed lockfiles. Do not use `continue-on-error`, shell fallbacks, or blanket allowlists; registry/database failures are audit failures. Cargo has no equivalent production-only lockfile view, so its whole-lock audit is intentionally stricter.
- **Rejected.** Advisory-only/default-warning execution, including `continue-on-error`, `|| true`, or blanket ignores; those commands can report a gap while still authorizing release.
