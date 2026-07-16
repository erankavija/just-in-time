# Write default-rule membership through to rules.toml on config-driven refresh

**Issue:** d74a9ed1
**Type:** bug
**Priority:** normal
**Date:** 2026-07-16

## Problem Statement

`jit:af4c901a` made default-rule MEMBERSHIP (the `namespace-unique-*` family)
and assertions reconcile against the `config.toml` registry in memory at every
`effective_rules` load, via `validation::defaults::reconcile_default_rules_with_config`.
That closed the validation gap, but `.jit/rules.toml` on disk was left exactly
as scaffolded: a hand-declared unique namespace changes what validates, yet the
file never gains the corresponding `[[rules]]` row.

The `rule` item kind is registry-first (`@/rule/<name>` resolves straight from
`rules.toml`'s `[[rules]]` table, not the in-memory-reconciled ruleset —
`domain::item::load_toml_scope_items`). So a reconciled-but-unwritten default
rule is invisible to `jit item show`/`list`, and any citation of it (e.g. this
project's `rules-and-gates.md` projection, or an `enforces:@/rule/<name>`
label) dangles until someone adds the row by hand. Observed live in this repo:
`brackets` was declared unique in `config.toml`; reconciliation enforced
`namespace-unique-brackets` immediately, but `@/rule/namespace-unique-brackets`
resolved only after the row was added manually.

## Decision

Write the `namespace-unique-*` family's file MEMBERSHIP through to
`rules.toml`, on the SAME jit-driven-write triggers that already republish
`schemas/default-*.json` (`CommandExecutor::refresh_default_schema_projections`,
called from `jit init` re-run and `jit config set`):
`CommandExecutor::sync_default_rule_membership` computes
`validation::defaults::default_rule_membership_diff` (loaded rules.toml vs. the
current registry) and, when non-empty, calls
`storage::ruleset_store::sync_namespace_unique_rules` to apply it.

The write is a structural block-level splice, not a re-serialize: each
`[[rules]]` block's `name`/`origin` is parsed to identity only (never its
`assert` table), a block is dropped only when its identity matches a `to_drop`
name AND `origin = "default"`, and every surviving block's bytes — including
hand-edited policy fields on other default rules and any custom rule's
comments — are copied through unchanged. New rows are appended at the END of
the file (matching REQ-01's own wording and how the `brackets` row was added by
hand), rendered via `validation::serialize::render_rule_block` so an appended
row is byte-identical to what a fresh `jit init` would have scaffolded for that
namespace.

The diff mirrors `reconcile_default_rules_with_config`'s opt-out rule exactly:
a `rules.toml` carrying no `origin = "default"` row at all has deliberately
opted out of the defaults, so the diff (and thus the write) is empty
regardless of the registry. A custom rule that happens to share a
`namespace-unique-<ns>` NAME is never treated as satisfying that membership
(consistent with in-memory reconciliation), and is never a drop target.

Scope is deliberately narrow: only `namespace-unique-*` MEMBERSHIP is written
through. `label-format`, `namespace-registry`, and `type-hierarchy-known` vary
only in ASSERTION content (never existence, except `namespace-registry`'s
registry-non-empty gate, which this issue does not touch) and stay purely
in-memory-reconciled, exactly as `jit:af4c901a` decided. In-memory
reconciliation remains the validation authority; this write-through exists
solely so `rules.toml` cannot lag it for addressability.

## Consequences

- `rules.toml` is no longer scaffold-then-frozen for the `namespace-unique-*`
  family: it tracks `[namespaces]` the same way `schemas/default-*.json`
  already did, on the same triggers.
- `@/rule/<name>` addressing resolves the `namespace-unique-<ns>` rule for
  every namespace the registry currently declares unique after any
  jit-driven config write, so docs-mechanical citation checking and an
  `enforces:` label naming that rule cannot dangle on a
  reconciled-but-unwritten row.
- Hand edits to a surviving default rule's policy fields (severity, enforce,
  selector, description) and all custom rules survive every sync byte-exact —
  the splice never touches a block it does not add or drop.
