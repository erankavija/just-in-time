---
name: jit-breakdown
description: >
  Instantiate one container's authoritative JIT breakdown manifest, or produce
  that same manifest for plain work, then batch-create and verify the graph.
  Use jit-planning-lead when the reviewed plan and manifest do not exist yet.
---

# JIT Issue Breakdown

Create the complete child graph in one native batch. For bracketed containers,
the approved JSON manifest is authoritative; never reinterpret Markdown.

## 1. Discover the contract

1. Run `jit recover` and `jit validate`. Read the parent, linked documents,
   `.jit/config.toml`, `.jit/templates.toml`, gate registry/presets, rules, and
   `.jit/reference/content-standards.md`.
2. A parent is bracketed only when a live template applies to its configured
   type. Otherwise use the plain path. Derive node types, gate names, membership
   namespaces, and finest-tier types; hardcode none.
3. For bracketed work, locate scaffold nodes `P` and `B`, require P's plan gate
   passed, and resolve the artifact directory with `jit doc dir <C> dev/active`, then
   locate `breakdown.json` in it. If the approved work has only Markdown, stop with:
   “Missing authoritative breakdown manifest; rerun jit-planning-lead to author and
   review `breakdown.json`.” There is no compatibility fallback or automated backfill.
4. For plain work, dispatch the analyst with
   [analysis-prompt.md](references/analysis-prompt.md). It writes the same bare
   manifest format before anything is created.

The canonical schema and helper belong to `jit-planning-lead`; this skill only
routes to them through [plan-schema.md](references/plan-schema.md).

## 2. Validate before creation

Run all checks without mutation:

```bash
.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py validate \
  <manifest> --config .jit/config.toml --known-source <every-valid-source-id> ... \
  --required-source <mandatory-source-id> ... \
  --required-criterion <criterion-id> ... --deny-warnings
.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py render \
  <manifest> <plan> --check                 # bracketed work
jit issue batch-create --from-json <manifest> --dry-run --json
```

Reject malformed fields, semantic-key violations, unknown/cyclic edges,
incomplete criterion coverage, stale generated output, or unresolved sizing
warnings. Missing hierarchy config, invented sources, non-aggregating non-finest
entries, and malformed/duplicate contract headings also fail. Every issue
description is the final standalone body. Every finest-tier
entry must pass the one-worker assignment simulation: one outcome, bounded
consumer family, observable test boundary, focused implementation/review cycle,
no inner decomposition, and no mixed deliverable categories. `landing_group`
never changes that judgment. Per-code warning overrides and terminal footprints
stay visible for review. Contract modes and producer reachability must be exact.

## 3. Create once

Run:

```bash
jit issue batch-create --from-json <manifest> --json
```

Capture the complete semantic key→UUID map. Native fields are persisted;
`planning` is deliberately ignored. Intra-manifest edges are already wired—do
not replay them with `jit dep add`.

## 4. Wire only external geometry

For plain work, make the parent depend on manifest sinks unless the live ruleset
defines another containment shape.

For bracketed work, follow [bracket-spine.md](references/bracket-spine.md):

- each manifest source depends on pre-created `B`;
- `C` depends on each manifest sink via `jit dep add --reduce`, atomically
  dropping the scaffold's redundant `C → B` anchor;
- each external dependency recorded for re-home on a semantic key is attached to
  that mapped issue;
- no parent-centric containment edges are added.

Use the returned map for every operation.

## 5. Verify and review

Compare the manifest with stored issues and graph before advancing:

- one created issue per entry; no missing, extra, merged, or split work;
- exact title, description, type, priority, labels, gates, and dependency edges;
- no persisted/exported `planning` metadata;
- exact source/sink bracket edges and recorded external re-homes;
- content standards and terminal assignment simulation still pass.

Run `jit validate`, then the configured coverage and breakdown-review gates on
`B`. Review uses the manifest, not a reconstructed Markdown task list. On failure,
edit the manifest first, replace superseded issues/prose, regenerate/revalidate,
and reconcile the graph. Rerun plan-review if a shared architectural contract or
approved graph contract changed. Stop and escalate on the third recorded failure;
never run a fourth review.

## 6. Report

Report the manifest path, issue/edge counts, key→UUID map, external wiring,
validation result, assignment simulation, and gate outcomes. Commit JIT state in
the repository's required state-only batches.
