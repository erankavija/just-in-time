# Dangling-Citation Check — Evidence for 76cb968b REQ-04

**Issue:** 76cb968b
**Date:** 2026-07-06
**Verified at:** commit e70268af
**Result: PASS — every concrete address on a citation surface resolves, across ALL item kinds and ALL comment forms.**

## Citation surfaces

1. **Authored markdown:** `.claude/skills/` (excluding `*/evals/` historical transcripts), `README.md`, `docs/`, `CLAUDE.md`.
2. **Rust comments, all forms:** doc comments (`///`, `//!`, doctest bodies) and ordinary comments (`//`, including trailing comments after code) across every `.rs` file under `crates/` — test files included.

## Method

1. Extract every concrete kind-segmented token, with **no kind filter and no comment-form filter**:

   ```
   # authored markdown
   grep -rhoE "@/[a-z][a-z0-9-]*(/[A-Za-z0-9-]+)+" .claude/skills/ README.md docs/ CLAUDE.md | grep -v evals
   # the comment portion of every Rust line containing //
   grep -rh --include="*.rs" "//" crates/ | sed 's|^[^/]*//|//|' \
     | grep -oE "@/[a-z][a-z0-9-]*(/[A-Za-z0-9-]+)+"
   ```

   Placeholder grammar forms (`@/invariant/<self-id>`, `@/issue/<short-id>/…`) contain non-token characters and are excluded by construction.

2. Canonicalize the config-declared alias (`@/inv/…` -> `@/invariant/…`).

3. Resolve each distinct token against the full item index (`jit item list --json`, 453 items across every configured kind: requirement, decision, risk, invariant, rule, gate, definition, and issue-scoped items), using the installed binary.

4. For any token absent from the index, adjudicate **every occurrence** by location and demonstrate it is an example/fixture construct, not a citation of this repository's knowledge.

## Results

37 distinct concrete tokens across both surface classes. 31 resolve against the live index. 6 do not; each is adjudicated below with all its occurrence sites. Zero dangling citations.

### Resolved (31)

- **invariant (7)** — assignee-format, atomic-writes, dag-acyclic, domain-agnostic, event-log, gate-semantics, label-format (cited as `@/inv/…` in skills, CLAUDE.md guidance, and Rust comments; canonical `@/invariant/…` in README/docs and Rust doc examples).
- **rule (9)** — coverage-preview, label-format, namespace-registry, namespace-unique-resolution, namespace-unique-team, namespace-unique-type, orphan-leaf, strategic-consistency, type-hierarchy-known.
- **gate (13)** — breakdown-review, cargo-ci, cargo-ci-features, clippy, code-review, coverage-preview, fmt, jit-validate, npm-ci, plan-review, repo-validate, tdd-reminder, tests.
- **definition (1)** — `@/definition/State`.
- **issue-scoped (1)** — `@/issue/56ab0224/requirement/REQ-01` (doc examples anchor to a real issue and item; verified via `jit item show`).

### Adjudicated non-index tokens (6)

**Self-contained example registries — `@/policy/POL-01`, `@/policy/POL-02`, `@/policy/POL-NN` (2 sites).** Each site defines the registry it cites and demonstrates resolution within it:

- `crates/jit/src/domain/item.rs` (doctest) — declares a `policy` kind with an inline TOML source (`id = "POL-01"`) and asserts the projected item; the passing doctest is the resolution proof.
- `crates/jit/tests/markdown_kind_source_path_tests.rs` (`//!` header) — describes the test's fixture repo (a `policies.md` with `POL-01`/`POL-02`); the test body asserts `jit item show @/policy/POL-01` resolves there.

These demonstrate the config-driven-kinds API (kinds are repository configuration, never hardcoded — `@/inv/domain-agnostic`); they cite their own example registries, not this repository's.

**Test-fixture registry ids — `@/invariant/sample-invariant`, `@/invariant/second-invariant`, `@/invariant/missing-invariant`.** Every occurrence (comments and code alike) lies inside test code: past the `#[cfg(test)]` boundary in `crates/jit/src/domain/item.rs` (boundary line 1874, first occurrence 1975), `crates/jit/src/commands/item.rs` (601 / 1295), `crates/jit/src/commands/validate.rs` (2010 / 2078), and in `crates/jit/tests/{invariant_registry_story_tests,item_cli_tests,cross_substrate_generality_tests}.rs`. The first two are ids in fixture registries the tests construct and then resolve; `missing-invariant` is the negative-path fixture for the `enforces:`-label validation — its purpose is to BE dangling so the test can assert the validator flags it.

## Repository state

- `jit validate` — passed.
- `jit invariant check` — no enforcement drift.
- `jit invariant render` — idempotent (second render produces no diff).
- `cargo test` (doctests included), `cargo clippy --workspace --all-targets`, `cargo fmt --all -- --check` — clean.
