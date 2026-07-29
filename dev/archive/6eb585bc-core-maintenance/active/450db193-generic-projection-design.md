# Generic config-declared projections — design note

Issue: 450db193 — Generic `[projection.<name>]` tables spanning registry- and
markdown-first item kinds.

## Problem

Two bespoke projection tables (`[invariant_projection]`, `[rules_gates_projection]`)
each had their own config struct and their own CLI verb (`jit invariant render`,
`jit reference render`). Both only covered registry-first kinds. A markdown-first
kind (the `charter` decision log) had no projection path into always-loaded agent
context. A new table + verb per projection does not scale.

## Shape

One generic config table per projection, keyed by name:

```toml
[projection.<name>]
kind   = "<kind>"          # or ["rule", "gate"]; string or array
mode   = "region"          # separate-file | region
target = "AGENTS.md"       # repo-relative
style  = "id-anchor"       # id-anchor | full
# region-begin / region-end optional; default to
#   <!-- jit:<name>:begin --> / <!-- jit:<name>:end -->
```

One command drives all: `jit project render [--name <name>]` renders every
declared projection, or the named one.

## Two styles, two renderers

- **`id-anchor` — generic, kind-agnostic (the new capability).**
  `render_id_anchor_rows(rows)` emits `- **{self-id}** — {display}` per addressable
  row, in source order. `display` is the row `text` with a leading `{self-id}` and
  its trailing separator stripped, so a charter line (`D-1: ...`, whose text repeats
  the self-id) renders `- **D-1** — ...` while an invariant statement (which never
  begins with its id) is unchanged. Rows come from the SAME resolution path as
  `jit item list`: markdown-first kinds are scanned from their `source` file,
  registry-first kinds are projected from their `.toml` via the descriptor mapping
  (REQ-02). A brand-new markdown-first kind needs ZERO Rust — only a
  `[projection.<name>]` table. This is what the charter dogfood exercises (REQ-06).

- **`full` — the two built-in rich renders.** `render_invariants_markdown` (invariant
  registry: `[kind]`, enforced-by) and `render_rules_and_gates_markdown` (`## Rules`
  / `## Gates` sections with severity/enforce metadata and gate titles) carry typed
  fields NOT present in a generic addressable row, so `full` dispatches to the typed
  renderer selected by the projection's declared kinds (`invariant`; or the
  `rule`+`gate` pair). These are jit's own built-in registry-first infrastructure
  kinds — the same ones already addressed by `@/rule/…`, `@/gate/…`, `@/invariant/…`
  literals elsewhere — so naming them here keeps the USER-domain surface agnostic
  (`@/inv/domain-agnostic`). An unrecognised kind set under `full` is a typed error.

## Region / write plumbing (unchanged mechanics)

`splice_region` (pure) and `write_projection` (the single I/O orchestrator) replace
only the bytes between `<!-- jit:<name>:begin -->` and `<!-- jit:<name>:end -->`,
byte-preserve everything outside, and write atomically through
`IssueStore::write_repo_file` (`@/inv/atomic-writes`). A missing target, missing
source, unknown kind, or absent marker is a typed `ProjectionError` before any
write — nothing is partially written (REQ-03, REQ-07).

## Migration (REQ-05, REQ-06)

- Removed: `[invariant_projection]`, `[rules_gates_projection]` config tables and
  structs; `jit invariant render`, `jit reference render` (and the whole `reference`
  verb). `jit invariant check` (enforcement drift, not a projection) stays.
- This repo's `.jit/config.toml` migrated to `[projection.invariants]` (id-anchor →
  AGENTS.md), `[projection.rules-and-gates]` (full → docs/reference/rules-and-gates.md),
  and new `[projection.charter]` (id-anchor → AGENTS.md `### Charter Decisions`).
- `scripts/docs-check-projections.sh` (docs-mechanical M5 freshness guard) re-runs
  `jit project render` and diffs the configured targets — it binds to the projection
  CODE, never a hand-written mirror (projection-guard rule).

## Parity guard (REQ-08)

The parity harness renders the two migrated projections through the generic command
and asserts the body equals the typed renderer output computed LIVE from the same
registries (`render_id_anchor_rows(invariant_rows)` ==
`render_invariants_markdown(reg, IdAnchor)`; generic full body ==
`render_rules_and_gates_markdown(rules, gates, Full)`). Two independent code paths,
no committed-doc mirror.
