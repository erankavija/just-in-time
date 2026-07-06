# Dangling-Citation Check — Evidence for 76cb968b REQ-04

**Issue:** 76cb968b
**Date:** 2026-07-06
**Verified at:** commit e70268af
**Result: PASS — every concrete address on a citation surface resolves, across ALL item kinds.**

## Citation surfaces

Two surface classes carry citations of project knowledge:

1. **Authored markdown:** `.claude/skills/` (excluding `*/evals/` historical transcripts), `README.md`, `docs/`, `CLAUDE.md`.
2. **Rust doc comments:** every `///` and `//!` line under `crates/` (public API documentation, including doctest bodies).

Code that is not documentation — `#[cfg(test)]` modules and `crates/jit/tests/` fixture code — constructs local registries and negative-path assertions; its address strings are fixtures whose resolution semantics the test suite itself asserts, not citations. (Test-file `//!` module headers ARE doc comments and are included in surface 2.)

## Method

1. Extract every concrete kind-segmented token, with **no kind filter**:

   ```
   # authored markdown
   grep -rhoE "@/[a-z][a-z0-9-]*(/[A-Za-z0-9-]+)+" .claude/skills/ README.md docs/ CLAUDE.md | grep -v evals
   # Rust doc-comment lines
   grep -rh --include="*.rs" -E "^\s*(///|//!)" crates/ | grep -oE "@/[a-z][a-z0-9-]*(/[A-Za-z0-9-]+)+"
   ```

   Placeholder grammar forms (`@/invariant/<self-id>`, `@/issue/<short-id>/…`, `@/kind/self-id`) contain non-token characters and are not concrete addresses; the pattern excludes them by construction.

2. Canonicalize the config-declared alias (`@/inv/…` -> `@/invariant/…`).

3. Resolve each distinct token against the full item index (`jit item list --json`, 453 items across every configured kind: requirement, decision, risk, invariant, rule, gate, definition, and issue-scoped items), using the installed binary.

## Results

34 distinct concrete tokens on the citation surfaces. 31 resolve against the live index; 3 are self-contained-example addresses (below). Zero dangling citations.

### Resolved (31)

- **invariant (7)** — assignee-format, atomic-writes, dag-acyclic, domain-agnostic, event-log, gate-semantics, label-format (cited as `@/inv/…` in skills, CLAUDE.md guidance, and Rust comments; canonical `@/invariant/…` in README/docs and Rust doc examples).
- **rule (9)** — coverage-preview, label-format, namespace-registry, namespace-unique-resolution, namespace-unique-team, namespace-unique-type, orphan-leaf, strategic-consistency, type-hierarchy-known.
- **gate (13)** — breakdown-review, cargo-ci, cargo-ci-features, clippy, code-review, coverage-preview, fmt, jit-validate, npm-ci, plan-review, repo-validate, tdd-reminder, tests.
- **definition (1)** — `@/definition/State`.
- **issue-scoped (1)** — `@/issue/56ab0224/requirement/REQ-01` (doc examples anchor to a real issue and item; verified via `jit item show`).

### Self-contained example registries (3 tokens, 2 sites)

`@/policy/POL-01`, `@/policy/POL-02`, `@/policy/POL-NN` appear in exactly two places, each an example that **defines the registry it cites** and demonstrates resolution within it:

- `crates/jit/src/domain/item.rs` — a doctest that declares a `policy` kind with an inline TOML source (`id = "POL-01"`) and asserts the projected item; the passing doctest is the resolution proof.
- `crates/jit/tests/markdown_kind_source_path_tests.rs` — the `//!` module header describes the test's fixture repo (a `policies.md` with `POL-01`/`POL-02`); the test body asserts `jit item show @/policy/POL-01` resolves there.

These demonstrate the config-driven-kinds API (kinds are repository configuration, never hardcoded — `@/inv/domain-agnostic`); they cite their own example registries, not this repository's.

## Repository state

- `jit validate` — passed.
- `jit invariant check` — no enforcement drift.
- `jit invariant render` — idempotent (second render produces no diff).
- `cargo test` (doctests included), `cargo clippy --workspace --all-targets`, `cargo fmt --all -- --check` — clean.
