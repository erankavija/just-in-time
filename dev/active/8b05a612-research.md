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
- **[CITED]** npm trusted publishing accepts GitHub-hosted Actions through OIDC, requires `id-token: write`, and requires an npm-side publisher binding to the exact repository/workflow/environment. The binding can be restricted to `npm stage publish`, current trusted publishing requires Node >=22.14 and npm >=11.5.1, and eligible public packages receive provenance automatically: [trusted publishers](https://docs.npmjs.com/trusted-publishers/) and [npm provenance](https://docs.npmjs.com/generating-provenance-statements/).
- **[CITED]** Staged publishing is the supported non-public OIDC verification primitive. It requires an already-existing package, Node >=22.14, and npm >=11.15; `npm stage publish <package-spec>` reserves the version without making it public, while `npm stage approve <stage-id>` requires an interactive maintainer and 2FA. OIDC applies to publish/stage-publish only, so `npm whoami`, `npm stage view`, and a dry run cannot prove the trust binding: [staged publishing](https://docs.npmjs.com/staged-publishing/), [`npm stage` CLI](https://docs.npmjs.com/cli/v11/commands/npm-stage/), and [trusted-publisher limitations](https://docs.npmjs.com/trusted-publishers/#limitations-and-future-improvements).
- **Recommendation [ASSUMED].** First bootstrap the absent `@erankavija/jit-mcp-server` package at its existing pre-v1 version with an owner-authorized temporary credential. Then configure the exact release workflow/environment as stage-only, require 2FA/disallow tokens, and revoke the credential. For v1, produce and smoke one tarball with `npm pack --json`, assemble all candidates, and run `npm stage publish <tarball>` on a GitHub-hosted runner with no `NODE_AUTH_TOKEN`. Record the stage ID; an owner uses `npm stage view`/`download` to compare the pending bytes, the clean-room rehearsal consumes those same bytes, and only after it passes does the owner run `npm stage approve <stage-id>` with 2FA. This proves OIDC without exposing the version publicly before acceptance.
- **Rejected.** A dry-run or `npm whoami` authentication claim (OIDC is not exchanged by those commands); an automation token after bootstrap (long-lived secret); direct `npm publish` before rehearsal (irreversible public mutation); or publishing from the source directory (may repack bytes different from the smoked tarball).

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
