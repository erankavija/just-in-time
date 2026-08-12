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
- **@/rule/coverage-preview** — On a matching breakdown node, apply the bracketed container's plan-time coverage preview with this issue as the sole firing subject; every [hard] REQ-id must be credited by a satisfies:<id> descendant. (error, enforced)

## Gates

- **@/gate/breakdown-review** — AI Breakdown Review: AI-powered adversarial review of a breakdown against the design doc and content standards
- **@/gate/cargo-ci** — Cargo CI (fmt + clippy + tests + build policy): Full Rust CI pipeline: formatting check, zero-warning clippy, the workspace test suite compiled in its own step and then measured by the named suite clock, the build-footprint budget checker (integration-target, active-executable, and measured suite-duration budgets, profile, and dependency-feature policy; scripts/rust-build-budget.sh), and the incremental-state check must all pass (@/invariant/bounded-rust-build-footprint).
- **@/gate/cargo-ci-features** — Cargo CI (feature-gated parsers): Compiles and tests the optional html/xml content-parser features: feature-enabled clippy (zero warnings) and the cross-format parity test suite. Required on issues that touch feature-gated code so the default-only cargo-ci gate does not leave them unexercised.
- **@/gate/clippy** — Clippy Lints Pass: Zero clippy warnings allowed
- **@/gate/code-review** — AI Code Review: Issue-scoped, read-only AI code review using `jit:<short-id>` commit attribution, latest gate evidence, blocking issue-impact versus advisory pre-existing findings, and mandatory truncation recovery
- **@/gate/coverage-preview** — Coverage Preview: Evaluate the configured coverage-preview rule with the gated breakdown as its sole firing issue; blocks when a [hard] criterion is uncovered at plan time
- **@/gate/dependency-audit** — Dependency vulnerabilities checked: cargo-audit checks Cargo.lock and treats vulnerabilities plus all audit warnings as blocking failures.
- **@/gate/doc-review** — AI Documentation Review: An issue-scoped documentation-impact review. It uses the repository-configured hierarchy and DAG-resolved descendants to distinguish leaves from containers. A leaf footprint contains only its own individually inspected `jit:<short-id>` commit patches; a container footprint also contains every delivered descendant's patches without absorbing unrelated sequencing dependencies. It derives the smallest impact cone, including required but untouched docs; rather than replaying leaf reviews, container reviews assess the combined documentation contract for workflow coverage, cross-child consistency, canonical placement, discoverability, and aggregate concision. Issue-impact defects fail the gate; unrelated pre-existing drift is advisory. Missing tags fall back to issue intent and linked documents without attributing uncommitted changes. All reviews enforce concise style.
- **@/gate/docs-mechanical** — Documentation Mechanical Checks: Deterministic mechanical checks over the adopter-facing documentation surface. Runs ./scripts/docs-mechanical.sh, which fans out to the committed checkers beside it under scripts/ and aggregates their exit statuses, an environment error dominating a finding. Each checker derives both sides of its comparison live from the tree, so the gate encodes no product fact of its own. A checker that takes a footprint receives one: area audits and the container's full-surface run pass their own paths via positional arguments, and this gate supplies the adopter documentation root through the DOCS_FOOTPRINT environment variable. A checker whose targets are fixed by configuration takes none.
- **@/gate/fmt** — Code Formatted: Code must be formatted with cargo fmt
- **@/gate/holistic-review** — Independent Holistic Review: Container-scoped independent holistic coherence review by a reviewer distinct from the building agent: verifies every hard criterion against repository artifacts, tests cross-child coherence, and rejects self-attested narrative as evidence
- **@/gate/jit-validate** — JIT Validate: Per-issue validation must pass (jit validate <ISSUE_ID> exits 0; evaluates only the issue under review)
- **@/gate/mcp-ci** — MCP CI (mcp-server test suite): MCP server workspace checks: the mcp-server unit and integration suites both pass. Covers the workspace that npm-ci (web) and cargo-ci (Rust crates) leave unexercised.
- **@/gate/npm-ci** — NPM CI (test + lint + build): Web workspace checks: vitest suite, ESLint, and production build all clean. Web-side equivalent of cargo-ci. Scoped to web/ — does not exercise mcp-server/; see mcp-ci for that workspace.
- **@/gate/plan-review** — AI Plan Review: AI-powered plan/design review before fan-out, against the planning issue's success criteria and linked design document
- **@/gate/repo-validate** — Repo Validate: Whole-repository validation must pass (`jit validate` with NO issue id runs run_rules(None) plus the repo-integrity checks); blocks the bound container from reaching Done until the entire repository validates. Distinct from the per-issue jit-validate gate, which scopes to one issue via $JIT_ISSUE_ID.
- **@/gate/secret-detection** — No secrets in code: gitleaks scans the working-tree version of every tracked file (scripts/secret-scan.sh) without Git history; generated and operational trees are excluded by construction, and a finding, failed copy, or unavailable scanner fails closed and blocks completion.
- **@/gate/security-review** — Security review completed: Manual threat-model review covers path containment, symlink and junction races, recovery journals, untrusted manifest content, dependency risk, and secrets.
- **@/gate/tdd-reminder** — TDD Reminder: Write tests first
- **@/gate/tests** — All Tests Pass: Full test suite must pass
<!-- jit:rules-and-gates:end -->
