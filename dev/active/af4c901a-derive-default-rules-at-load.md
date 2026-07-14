# Derive default-origin rule assertions from config at load

**Issue:** af4c901a
**Type:** bug
**Priority:** high
**Date:** 2026-07-14

## Problem Statement

The fixed default rules (`label-format`, `namespace-registry`,
`type-hierarchy-known`, the `namespace-unique-*` family) are baked from the
`config.toml` registry into `.jit/schemas/default-*.json` at `jit init` scaffold
time. No production path regenerated those files afterward, yet
`effective_rules` loaded a present `rules.toml` and read each default-origin
json-schema rule's schema from its baked file as the validation authority.

A hand edit of the declared registry — e.g. adding `[namespaces.enforces]` —
therefore left the baked `default-namespace-registry.json` pattern behind. The
first `enforces:` label failed `namespace-registry` against the stale file, far
from the causal config edit; hand-repairing the JSON was the only workaround.
This is the machine-artifact analogue of the staleness defect named by
`@/inv/single-source-prose`: the registry in `config.toml` is the single source
of truth, and the baked schema is a copy treated as authority.

## Decision

Derive at load. When a `rules.toml` is present, `effective_rules` still reads it
for which rules exist and, for default-origin rules, their editable policy fields
(`severity`, `enforce`, `when`, `description`). But each default-origin rule's
**assertion** is rebuilt from the declared registry in memory, via the pure
`validation::defaults::with_default_assertions_from_config`, which substitutes
the assertion that `default_ruleset(registry)` generates for a rule of the same
name. Custom rules (any `origin` other than `default`) are used verbatim and keep
reading their own declared schema files.

Rejected alternatives (pinned at intake): regenerate-on-write only (a hand edit
stays stale until some jit write happens) and drift-check plus a `jit rules sync`
command (the failure still occurs and adds a manual step, when the registry alone
answers the question the schema was baked to ask).

## Consequences

- `schemas/default-*.json` remain as write-through projections for external
  consumers, refreshed from the registry by
  `CommandExecutor::refresh_default_schema_projections` whenever jit writes
  `config.toml` or `rules.toml` (init/re-init and `config set`). They no longer
  decide validation, so a projection can never desync it.
- This generalizes the earlier type-hierarchy-specific fix (`jit:c78168d8`): the
  `type-hierarchy-known` rule now derives its enum at load like the other
  default rules, rather than depending on a write-time regeneration helper.
- The `rules.toml` header states the new contract: default rules are editable for
  policy, but their assertion derives from the `[namespaces]` / `[type_hierarchy]`
  registry; edit that registry to change what a default rule checks.
