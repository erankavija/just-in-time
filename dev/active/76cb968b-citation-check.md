# Dangling-Citation Check — Evidence for 76cb968b REQ-04

**Issue:** 76cb968b
**Date:** 2026-07-06
**Verified at:** commit c4365d64
**Result: PASS — 0 dangling citations.**

## Method

1. Extract every `@/(inv|invariant|rule|gate)/<self-id>` token from the swept surfaces: `.claude/skills/`, `README.md`, `docs/`, `CLAUDE.md`, and `crates/` (Rust comments and sources).

   ```
   grep -rhoE "@/(inv|invariant|rule|gate)/[A-Za-z0-9._-]+" \
     .claude/skills/ README.md docs/ CLAUDE.md crates/ | sed 's/[.,;:]$//' | sort -u
   ```

2. Canonicalize the `@/inv/` alias to `@/invariant/` and compare against the full item index (`jit item list --json`, 453 items), using the installed binary carrying the alias resolver.

3. For every token absent from the index, verify by fixed-string grep that it appears in **no** authored surface (skills, README, docs, CLAUDE.md) and only inside Rust test/doctest fixture code.

## Results

58 distinct tokens extracted. 29 resolve; 29 do not.

### Resolved citations (29) — every citation on an authored surface or Rust comment

| Kind | Addresses |
|---|---|
| invariant (7; cited as `@/inv/…` in skills, CLAUDE.md guidance, and Rust comments; canonical `@/invariant/…` in README/docs) | assignee-format, atomic-writes, dag-acyclic, domain-agnostic, event-log, gate-semantics, label-format |
| rule (9) | coverage-preview, label-format, namespace-registry, namespace-unique-resolution, namespace-unique-team, namespace-unique-type, orphan-leaf, strategic-consistency, type-hierarchy-known |
| gate (13) | breakdown-review, cargo-ci, cargo-ci-features, clippy, code-review, coverage-preview, fmt, jit-validate, npm-ci, plan-review, repo-validate, tdd-reminder, tests |

### Non-resolving tokens (29) — all test fixtures, none on a citation surface

Tokens such as `@/invariant/INV-01`, `@/rule/ghost-rule`, `@/gate/no-such-gate`, `@/rule/..` occur exclusively inside `#[cfg(test)]` modules and doctest fixtures in `crates/jit/src/{commands,domain,validation}/…`, where they construct local registries or assert error paths (e.g. `show_item("@/gate/only-rule")` asserting a kind-mismatch error). Fixed-string grep over `.claude/skills/`, `README.md`, `docs/`, and `CLAUDE.md` finds zero occurrences of any of them (`evals/` transcripts excluded as historical records per the issue).

## Repository state

- `jit validate` — passed.
- `jit invariant check` — no enforcement drift.
- `jit invariant render` — idempotent (second render produces no diff).
