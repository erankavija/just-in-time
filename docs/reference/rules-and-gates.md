# Rules and Gates Reference

> **Diátaxis Type:** Reference

`jit project render` projects a project's two enforcement registries into a
markdown document, so the rules and gates a repository enforces reach prose from
their source of truth:

- **Rules**: validation rules from `.jit/rules.toml`, addressed as `@/rule/<name>`.
- **Gates**: quality gates from `.jit/gates.toml`, addressed as `@/gate/<key>`.

The target document and its mode are configured under a `[projection.<name>]`
table in `.jit/config.toml` (here, `[projection.rules-and-gates]` in `full`
style). In region mode, rendering rewrites the delimited region
and byte-preserves everything around it, leaving the surrounding prose yours to
author. Edit the registries and re-render to change what the region says.

The region below carries the rules and gates of the just-in-time repository
itself, which configures this document as its projection target. It stands here
as a live example: your own registries render your own region, with the rule
names, gate keys, and checkers your project declares.

<!-- jit:rules-and-gates:begin -->
## Rules

- **@/rule/label-format** — Every label must match the canonical `namespace:value` format (namespace lowercase-kebab, value non-empty). Blocks the write and fails validation. (error, enforced)
- **@/rule/namespace-registry** — Every label's namespace must be declared in the namespace registry. An unknown namespace fails validation but never blocks a write. (error, advisory)
- **@/rule/type-hierarchy-known** — Every `type:<value>` label must name a type declared in the configured type hierarchy. An unknown type fails validation but never blocks a write. (error, advisory)
- **@/rule/namespace-unique-resolution** — At most one `resolution:` label per issue: `resolution` is a unique namespace. Blocks the write and fails validation. (error, enforced)
- **@/rule/namespace-unique-team** — At most one `team:` label per issue: `team` is a unique namespace. Blocks the write and fails validation. (error, enforced)
- **@/rule/namespace-unique-type** — At most one `type:` label per issue: `type` is a unique namespace. Blocks the write and fails validation. (error, enforced)
- **@/rule/namespace-unique-brackets** — At most one `brackets:` label per issue: `brackets` is a unique namespace. Blocks the write and fails validation. (error, enforced)
- **@/rule/coverage-preview** — On a matching breakdown node, apply the bracketed container's plan-time coverage preview with this issue as the sole firing subject; every [hard] REQ-id must be credited by a satisfies:<container-short-id>/<id> descendant. (error, enforced)

## Gates

- **@/gate/breakdown-review** — AI Breakdown Review: AI-powered adversarial review of a breakdown against the design doc and content standards
- **@/gate/build-budget-selftest** — Build-budget checker self-test: Runs scripts/rust-build-budget.sh --self-test over synthetic policy, Cargo-output and sparse-executable fixtures. Any non-zero exit fails the gate, including missing-tool errors.
- **@/gate/cargo-ci** — Cargo CI (fmt + clippy + tests + build policy): Full Rust CI pipeline: formatting check, zero-warning clippy, the workspace test suite under the per-test limits of .config/nextest.toml, which terminate and name a test still running at its layer's limit (unit: lib and bin targets; integration: every crates/*/tests/* target), the excluded-tests check that fails the run when the suite reports a skipped test or the doctest step reports an ignored one (@/invariant/no-ignored-tests), and the build-footprint budget checker (integration-target, active-executable and main.rs production-line budgets, profile, and dependency-feature policy; scripts/rust-build-budget.sh) must all pass (@/invariant/bounded-rust-build-footprint). The run owns and removes its temporary directory: every step runs with TMPDIR set to a directory made inside the caller's temporary directory, a compiler-cache server keeps the caller's TMPDIR, and the directory is removed when the run exits, whether it passes, fails or is interrupted by a signal the script traps. The checker exits 1 for a verdict on repository content and 75 (recorded as an `error` run, never reused through digest reuse) when every failing step is environment-sensitive: a step whose output reports ENOSPC (`No space left on device` or os error 28) with no other failure, whatever the load, or test TIMEOUT results with no FAIL, with the host load average at or above the threshold printed in the run output; the header of scripts/cargo-ci.sh defines the convention. A timeout at low load exits 1 and is a real regression to investigate, not to retry.
- **@/gate/cargo-ci-features** — Cargo CI (feature-gated parsers): Compiles and tests the optional html/xml content-parser features: feature-enabled clippy (zero warnings) and the cross-format parity test suite. Required on issues that touch feature-gated code so the default-only cargo-ci gate does not leave them unexercised.
- **@/gate/cargo-ci-selftest** — cargo-ci self-test: Runs scripts/cargo-ci-selftest.sh against the shipped CI pipeline. Any non-zero exit fails the gate, including missing-tool errors.
- **@/gate/code-review** — AI Code Review: Issue-scoped, read-only AI code review using `jit:<short-id>` commit attribution, latest gate evidence, blocking issue-impact versus advisory pre-existing findings, and mandatory truncation recovery
- **@/gate/coverage-preview** — Coverage Preview: Evaluate the configured coverage-preview rule with the gated breakdown as its sole firing issue; blocks when a [hard] criterion is uncovered at plan time
- **@/gate/dependency-audit** — Dependency vulnerabilities checked: cargo-audit checks Cargo.lock and treats vulnerabilities plus all audit warnings as blocking failures.
- **@/gate/doc-review** — AI Documentation Review: An issue-scoped documentation-impact review. It uses the repository-configured hierarchy and DAG-resolved descendants to distinguish leaves from containers. A leaf footprint contains only its own individually inspected `jit:<short-id>` commit patches; a container footprint also contains every delivered descendant's patches without absorbing unrelated sequencing dependencies. It derives the smallest impact cone, including required but untouched docs; rather than replaying leaf reviews, container reviews assess the combined documentation contract for workflow coverage, cross-child consistency, canonical placement, discoverability, and aggregate concision. Issue-impact defects fail the gate; unrelated pre-existing drift is advisory. Missing tags fall back to issue intent and linked documents without attributing uncommitted changes. All reviews enforce concise style.
- **@/gate/docs-check-selftest** — Documentation checker self-test: Runs scripts/docs-check-selftest.sh against documentation checkers and their repository fixtures. Any non-zero exit or SKIP notice fails; permission preconditions must be enforced.
- **@/gate/docs-mechanical** — Documentation Mechanical Checks: Deterministic mechanical checks over the adopter-facing documentation surface. Runs ./scripts/docs-mechanical.sh, which fans out to the committed checkers beside it under scripts/ and aggregates their exit statuses, an environment error dominating a finding. Each checker derives both sides of its comparison live from the tree, so the gate encodes no product fact of its own. A checker that takes a footprint receives one: area audits and the container's full-surface run pass their own paths via positional arguments, and this gate supplies the adopter documentation root through the DOCS_FOOTPRINT environment variable. A checker whose targets are fixed by configuration takes none.
- **@/gate/holistic-review** — Independent Holistic Review: Container-scoped independent holistic coherence review by a reviewer distinct from the building agent: verifies every hard criterion against repository artifacts, tests cross-child coherence, and rejects self-attested narrative as evidence
- **@/gate/jit-validate** — JIT Validate: Per-issue validation must pass (`jit validate <ISSUE_ID>` exits 0): the issue's selection of the whole-repository report, so findings concerning other issues do not fail it
- **@/gate/land-script-test** — Landing script test suite: Runs the jit-execution-lead skill's test-land.sh, which drives land.sh through `jit worktree exec` over fixture repositories it owns: sync commit, merge, conflict abort, refusals and the real pre-commit hook. Any non-zero exit fails the gate.
- **@/gate/manifest-tool-test** — Breakdown-manifest tool test suite: Runs the jit-planning-lead skill's evals/test-manifest-tool.sh, which exercises scripts/breakdown_manifest.py over its fixture manifests and the shared batch structural-conformance fixture. Any non-zero exit fails the gate, including a missing python3 under set -e.
- **@/gate/npm-ci** — NPM CI (test + lint + build): Web workspace checks: vitest suite, ESLint, and production build all clean. Web-side equivalent of cargo-ci. Scoped to web/.
- **@/gate/package-drift** — Package Drift: The generated registry regions hold what the jit-dogfood package declares, and gate inputs hold their footprints. Runs the dev-only package-drift example.
- **@/gate/plan-review** — AI Plan Review: AI-powered plan/design review before fan-out, against the planning issue's success criteria and linked design document
- **@/gate/profile-suite-selftest** — Suite profiler self-test: Runs scripts/profile-test-suite.sh --self-test over the suite profiler. Any non-zero exit fails the gate, including missing-tool errors.
- **@/gate/repo-validate** — Repo Validate: Whole-repository validation must pass (`jit validate` with no issue id: the whole report, including the repository-integrity and history checks); blocks the bound container from reaching Done until the entire repository validates. Distinct from the per-issue jit-validate gate, which selects one issue via $JIT_ISSUE_ID.
- **@/gate/secret-detection** — No secrets in code: gitleaks scans the working-tree version of every tracked file (scripts/secret-scan.sh) without Git history; generated and operational trees are excluded by construction, and a finding, failed copy, or unavailable scanner fails closed and blocks completion.
- **@/gate/security-review** — Security review completed: Manual threat-model review covers path containment, symlink and junction races, redo record publication and replay, untrusted manifest content, dependency risk, and secrets.
- **@/gate/session-cost-benchmark-selftest** — Session-cost benchmark self-test: Runs scripts/benchmark-session-cost-selftest.sh, which drives scripts/benchmark-session-cost.sh against a fake jit and checks its output contract. Any non-zero exit fails the gate, including exit 2 for a missing cmp, find, git, grep, jq or sha256sum.
- **@/gate/shipped-policy-regions-selftest** — Shipped-policy region generator self-test: Runs scripts/generate-shipped-policy-regions-selftest.sh: the generator changes nothing on a current tree, restores a corrupted region while keeping every byte outside its markers, and fails on malformed markers. Any non-zero exit fails the gate, including exit 2 for a missing git or jit.
- **@/gate/standards-fix-test** — Standards-fix test suite: Runs the jit-project-lead skill's test-standards-fix.sh, which scans a seeded fixture, applies standards-fix.sh, and re-scans to check every mechanical correction and every reported skip record. Any non-zero exit fails the gate, including exit 2 for a missing jit, jq or gawk.
- **@/gate/standards-scan-test** — Standards-scan test suite: Runs the jit-project-lead skill's test-standards-scan.sh, which seeds a jit init fixture with issues and documents for each asserted finding and checks the findings standards-scan.sh emits. Any non-zero exit fails the gate, including exit 2 for a missing jit, jq or gawk.
- **@/gate/workflow-contract-selftest** — Workflow-contract harness self-test: Runs scripts/workflow-contract-selftest.sh, which shows the workflow-contract harness rejects each defect in its case vectors and that the actionlint wiring rejects an actionlint-only defect. Any non-zero exit fails the gate, including exit 2 for a missing python3 and the failure reported when actionlint is unavailable.
- **@/gate/worktree-cache-test** — Worktree cache seed and harvest test suite: Runs the jit-execution-lead skill's test-worktree-cache.sh, which drives dispatch-worker-worktree.sh and reclaim-worker-worktree.sh over a fixture repository and pool it owns. Any non-zero exit fails the gate, including exit 2 for a missing git, rsync, stat or diff.
<!-- jit:rules-and-gates:end -->
