# Decision: remove the plan-doc location resolver rather than wire it to a caller

**Issue:** `985ab96d` — Unreferenced plan-doc location resolver
**Date:** 2026-07-29
**Scope:** `crates/jit/src/commands/plan_doc.rs` and the `PlanDocError` classification arm in `crates/jit/src/main.rs`

## Decision

Remove the surface. `crates/jit/src/commands/plan_doc.rs` is deleted, taking
`resolve_plan_doc_location`, `load_plan_content`, `project_plan_doc`, the
`PlanDocContainer` / `PlanDocLocation` types, the `PlanDocError` type, and the
`INLINE_LOCATION` sentinel with it. The one item in that module with a
production reader, the `PLAN_DOC_LABEL` document-reference label, moves to
`crates/jit/src/commands/validate.rs` beside its only reader,
`planning_node_plan_path`. The `PlanDocError` arm in `error_to_error_code` goes
with the type it classified.

## Why

Each of the three functions is a superseded representation of a step the
production plan-content path now performs a different way, so
`@/inv/canonical-cutover` ("superseded aliases, fields, and representations are
removed after cutover; compatibility code is allowed only in a named, versioned
migration boundary with a tracked removal condition") governs. No migration
boundary names this module and no removal condition tracks it, so the
compatibility exemption does not apply.

The three supersessions:

| Removed step | What performs it in production |
| --- | --- |
| `resolve_plan_doc_location` — derive the plan path by substituting `{container.*}` in the template's planning-node `doc` | `find_planning_node` + `planning_node_plan_path` read the path from the planning node's `plan`-labeled document reference (`crates/jit/src/commands/validate.rs`) |
| `load_plan_content` — read the plan file from the working tree | `plan_content_from_image` (`crates/jit/src/commands/validate.rs`) and `project_plan_content` (`crates/jit/src/validation/repository.rs`) read the plan bytes out of the captured repository image |
| `project_plan_doc` — project an issue with the loaded plan's sections | `evaluate_graph`'s injected `plan_content` map (`crates/jit/src/validation/graph.rs`): the boundary hands loaded strings to the pure engine, which parses sections itself |

The cutovers are two prior commits:

- `b75ef93e` (2026-06-26) made the planning node's `plan` document reference the
  validation-time source of truth for the plan location, so a plan that is
  moved, archived, and re-linked keeps validating. The graph template's
  `plan_doc_location` became the creation-time default only. That left
  `resolve_plan_doc_location` without a caller.
- `936a4c27` (2026-07-20) migrated validation onto captured-image reads, which
  is what dropped the last `load_plan_content` call. `project_plan_doc` never
  had a production caller: it calls the pure projection entry point rather than
  being called by it.

`git grep` at `33e86a2b^`, the base of the most recent rewrite of the module,
finds no production reference to any of the three, confirming the condition
predates that rewrite rather than being introduced by it.

```mermaid
flowchart LR
    T["templates.toml<br/>planning node doc"] --> A["apply engine<br/>template_expand"]
    A --> P["planning node<br/>plan doc reference"]
    P --> L["planning_node_plan_path"]
    L --> I["captured repository image<br/>plan_content_from_image"]
    I --> E["pure engine<br/>evaluate_graph(plan_content)"]
```

## Rejected alternative: wire the surface to a caller

The alternative was to give the three functions a production caller — have
`plan_content_from_image` and `project_plan_content` locate the plan through
`resolve_plan_doc_location` when a planning node records no `plan` reference,
and load it through `load_plan_content`. It was rejected on three counts.

**It reintroduces the derivation `b75ef93e` replaced.** A container whose plan
has been moved or archived and re-linked has a `plan` reference pointing at the
new location; a template-derived fallback would resolve the same container to
the old default path. Two derivations of one plan location is precisely the
staleness the doc-reference cutover removed.

**It reintroduces an ambient filesystem read.** `load_plan_content` calls
`std::fs::read_to_string` against a caller-supplied base directory. Validation
reads every other input through the captured repository image, which is what
makes a validation run reproducible against a fixed repository state and lets a
proposed-base capture validate content that is not in the working tree. Routing
plan content around that boundary would make plan-derived findings depend on
whatever the working tree happened to hold at read time.

**It duplicates substitution the apply engine already owns.** The apply engine
resolves `{container.id}` and `{container.dir}` through
`InterpolationContext` and `resolve_artifact_directory`
(`crates/jit/src/commands/template_expand.rs`), including rejecting an
issue-scoped area the registry does not declare. The deleted resolver's tests
asserted its answer equalled the apply engine's answer for the same declaration
against the shared `test_declarations` fixtures, so the behaviour is retained by
the surviving derivation and stays covered by `template_expand.rs` and
`template.rs` tests. No substitution behaviour is lost with the module.

If a production path that derives a plan location is ever needed again, the
apply engine is where that derivation already lives; the right move then is to
call it, not to restore a second copy.

## What this does not change

No CLI behaviour changes. `PlanDocError` was unconstructible outside the deleted
module, so its `error_to_error_code` arm was unreachable and no exit code moves.
A missing required plan document surfaces through the untyped
`required plan document '<path>' is missing` error that
`plan_content_from_image` and `project_plan_content` raise, which
`error_to_error_code` classifies as a generic failure. That is why this carries
no `CHANGELOG.md` entry: there is no adopter-observable change to record.
