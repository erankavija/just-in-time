# Rules and Gates Reference

> **Diátaxis Type:** Reference

This document is a generated projection of jit's two enforcement registries:

- **Rules** — validation rules from `.jit/rules.toml`, addressed as `@/rule/<name>`.
- **Gates** — quality gates from `.jit/gates.toml`, addressed as `@/gate/<key>`.

The region below is rewritten by `jit reference render` from the live registries;
everything outside the markers is hand-authored and byte-preserved. Do not edit
inside the markers by hand — edit the registries and re-render.

<!-- jit:rules-and-gates:begin -->
## Rules

- **@/rule/label-format** — severity: error, enforce: true
- **@/rule/namespace-registry** — severity: error, enforce: false
- **@/rule/type-hierarchy-known** — severity: error, enforce: false
- **@/rule/namespace-unique-resolution** — severity: error, enforce: true
- **@/rule/namespace-unique-team** — severity: error, enforce: true
- **@/rule/namespace-unique-type** — severity: error, enforce: true
- **@/rule/coverage-preview** — severity: error, enforce: true
- **@/rule/orphan-leaf** — severity: warn, enforce: false
- **@/rule/strategic-consistency** — severity: warn, enforce: false

## Gates

- **@/gate/breakdown-review** — AI Breakdown Review: AI-powered adversarial review of a breakdown against the design doc and content standards
- **@/gate/cargo-ci** — Cargo CI (fmt + clippy + tests): Full Rust CI pipeline: formatting check, zero-warning clippy, and the workspace test suite must all pass.
- **@/gate/cargo-ci-features** — Cargo CI (feature-gated parsers): Compiles and tests the optional html/xml content-parser features: feature-enabled clippy (zero warnings) and the cross-format parity test suite. Required on issues that touch feature-gated code so the default-only cargo-ci gate does not leave them unexercised.
- **@/gate/clippy** — Clippy Lints Pass: Zero clippy warnings allowed
- **@/gate/code-review** — AI Code Review: AI-powered code review against just-in-time project standards
- **@/gate/coverage-preview** — Coverage Preview: Run scoped validation for the container resolved from the breakdown node's brackets: label; blocks when a [hard] criterion is uncovered at plan time
- **@/gate/fmt** — Code Formatted: Code must be formatted with cargo fmt
- **@/gate/jit-validate** — JIT Validate: Per-issue validation must pass (jit validate <ISSUE_ID> exits 0; evaluates only the issue under review)
- **@/gate/npm-ci** — NPM CI (test + lint + build): Web workspace checks: vitest suite, ESLint, and production build all clean. Web-side equivalent of cargo-ci.
- **@/gate/plan-review** — AI Plan Review: AI-powered plan/design review before fan-out, against the planning issue's success criteria and linked design document
- **@/gate/repo-validate** — Repo Validate: Whole-repository validation must pass (`jit validate` with NO issue id runs run_rules(None) plus the repo-integrity checks); blocks the bound container from reaching Done until the entire repository validates. Distinct from the per-issue jit-validate gate, which scopes to one issue via $JIT_ISSUE_ID.
- **@/gate/tdd-reminder** — TDD Reminder: Write tests first
- **@/gate/tests** — All Tests Pass: Full test suite must pass
<!-- jit:rules-and-gates:end -->
