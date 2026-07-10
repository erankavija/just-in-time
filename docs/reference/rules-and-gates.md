# Rules and Gates Reference

> **Diátaxis Type:** Reference

`jit reference render` projects a project's two enforcement registries into a
markdown document, so the rules and gates a repository enforces reach prose from
their source of truth:

- **Rules**: validation rules from `.jit/rules.toml`, addressed as `@/rule/<name>`.
- **Gates**: quality gates from `.jit/gates.toml`, addressed as `@/gate/<key>`.

The target document and its mode are configured under `[rules_gates_projection]`
in `.jit/config.toml`. In region mode, rendering rewrites the delimited region
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
- **@/rule/coverage-preview** — On a breakdown node, assert every [hard] REQ-id in the bracketed container's Success Criteria is credited by some issue in its dependency closure via a satisfies:<id> label. A plan-time coverage preview; fires once breakdown is underway. (error, enforced)
- **@/rule/orphan-leaf** — Warn when a leaf-level-typed issue (a type at the deepest hierarchy level, e.g. task) carries no parent-membership label (e.g. `epic:*`), leaving it unattached to any strategic container. Advisory: never blocks a write. (warn, advisory)
- **@/rule/strategic-consistency** — Warn when a strategic-typed issue (a type with a membership namespace, e.g. epic/milestone) lacks its own identifying membership label, such as a `type:epic` issue that has no `epic:*` label. Advisory: never blocks a write. (warn, advisory)

## Gates

- **@/gate/breakdown-review** — AI Breakdown Review: AI-powered adversarial review of a breakdown against the design doc and content standards
- **@/gate/cargo-ci** — Cargo CI (fmt + clippy + tests): Full Rust CI pipeline: formatting check, zero-warning clippy, and the workspace test suite must all pass.
- **@/gate/cargo-ci-features** — Cargo CI (feature-gated parsers): Compiles and tests the optional html/xml content-parser features: feature-enabled clippy (zero warnings) and the cross-format parity test suite. Required on issues that touch feature-gated code so the default-only cargo-ci gate does not leave them unexercised.
- **@/gate/clippy** — Clippy Lints Pass: Zero clippy warnings allowed
- **@/gate/code-review** — AI Code Review: AI-powered code review against just-in-time project standards
- **@/gate/coverage-preview** — Coverage Preview: Run scoped validation for the container resolved from the breakdown node's brackets: label; blocks when a [hard] criterion is uncovered at plan time
- **@/gate/doc-review** — AI Documentation Review: AI-powered review of the shipped documentation surface against the current source tree. Apply to any work that adds or changes adopter-facing documentation (the docs/ tree and the product-describing READMEs): it verifies that statements about CLI behavior, storage layout, and repository structure match crates/jit/src and .jit/ config, that repo-local dogfood configuration is signalled as this repository's own, that prose states current behavior with no legacy narration or hand-maintained counts that rot, that links and cited paths resolve, and that content conforms to docs/reference/jit-content-standards.md including Mermaid for all diagrams.
- **@/gate/docs-mechanical** — Documentation Mechanical Checks: Deterministic mechanical checks over the adopter-facing documentation surface, each deriving both comparison sides live from the tree so it encodes no product facts: markdown link + heading-anchor resolution (M2), source-path + @/… citation existence (M3), and registry projection freshness (M5, re-runs jit invariant render / jit reference render and diffs the configured targets). Runs ./scripts/docs-mechanical.sh, which fans out to the three committed checkers under scripts/ (docs-check-links.sh, docs-check-citations.sh, docs-check-projections.sh). The footprint is caller-supplied: area audits and the container's full-surface run pass their own paths via positional arguments or the DOCS_FOOTPRINT environment variable; with none supplied the checker defaults to the configured permanent documentation roots (currently docs/).
- **@/gate/fmt** — Code Formatted: Code must be formatted with cargo fmt
- **@/gate/jit-validate** — JIT Validate: Per-issue validation must pass (jit validate <ISSUE_ID> exits 0; evaluates only the issue under review)
- **@/gate/mcp-ci** — MCP CI (mcp-server test suite): MCP server workspace checks: the mcp-server unit and integration suites both pass. Covers the workspace that npm-ci (web) and cargo-ci (Rust crates) leave unexercised.
- **@/gate/npm-ci** — NPM CI (test + lint + build): Web workspace checks: vitest suite, ESLint, and production build all clean. Web-side equivalent of cargo-ci.
- **@/gate/plan-review** — AI Plan Review: AI-powered plan/design review before fan-out, against the planning issue's success criteria and linked design document
- **@/gate/repo-validate** — Repo Validate: Whole-repository validation must pass (`jit validate` with NO issue id runs run_rules(None) plus the repo-integrity checks); blocks the bound container from reaching Done until the entire repository validates. Distinct from the per-issue jit-validate gate, which scopes to one issue via $JIT_ISSUE_ID.
- **@/gate/tdd-reminder** — TDD Reminder: Write tests first
- **@/gate/tests** — All Tests Pass: Full test suite must pass
<!-- jit:rules-and-gates:end -->
