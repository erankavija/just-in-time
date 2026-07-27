# Batch-shape graph export: high-value use cases

Supporting analysis for issue 3e12ffbd (scoped batch-shape graph export as the
batch-create inverse). Each use case assumes the pinned semantics: pure
structural seed (no lifecycle fields), identity-bound labels stripped, bracket
nodes excluded, boundary edges reported.

## 1. Subtree templating — capture a proven shape and replay it

Recurring work has recurring shape: a release-hardening checklist, an
experiment pipeline (design → implement → measure → write up), an onboarding
sequence, a per-component audit fan-out. Today each recurrence is rebuilt by
hand or from a prose plan. With batch export, the first well-built instance
becomes the template: export the container's subtree once, keep the JSON in the
repo, re-import it under each new container. Titles are edited in the seed
file; structure, types, priorities, and gate sets replay exactly.

## 2. Interplay with graph templates — the capture dual of `jit apply`

The template registry (`.jit/templates.toml`) and batch seeds solve the same
problem from opposite ends:

| | `[[template]]` registry | batch seed file |
|---|---|---|
| Form | Abstract, parameterized (`{container.short_id}` interpolation, roles, anchors, transforms) | Concrete, literal titles and keys |
| Origin | Authored by hand | Captured from a real subgraph |
| Validation | At every config load | At import (batch pre-validation) |
| Instantiation | `jit apply <template> <container>` | batch creation from the file |
| Best at | Small invariant scaffolds (the plan bracket) | Larger one-off or evolving shapes |

Batch export completes a promotion pipeline: **capture** a real subtree as a
seed → **replay** it while it earns its keep → **promote** the stabilized shape
into a registered template by parameterizing what varies. The promotion step
(seed-to-template conversion, possibly `jit template promote`) is future work
the export deliberately leaves room for: excluding bracket nodes keeps captured
seeds template-compatible, since the target container scaffolds its own bracket
via `jit apply plan`.

## 3. Cross-repository transplant

A planned-but-unstarted epic moves to another repository or team: export the
subtree, import it there. Batch creation's whole-file pre-validation is the
safety net — unknown types or gates in the target config are reported before
anything is written, turning config incompatibility into a checklist (declare
these types, define these gates) instead of a half-created mess. This is the
work-graph counterpart to `jit snapshot export`, which archives issues and
documents but does not produce a re-creatable structural seed.

## 4. Planning sandbox — edit the structure as data

A captured seed is a small JSON file: bulk-retitle, retype, re-tier, or rewire
in an editor or with jq, then import into a scratch repository to inspect the
result (`jit graph tree`, scoped Mermaid render). Structural what-if
experiments — "what does this epic look like split into three stories?" —
become file edits plus a throwaway import, with the real repository untouched.

## 5. Partial replay and recovery

When a subtree is damaged — a botched bulk operation, an over-eager rejection
sweep — a seed captured from the last good state (the file lives in git, so any
prior commit has one) recreates the structure without replaying the entire
repository. Complements snapshot export: snapshots answer "what did the whole
tracker look like", seeds answer "give me this subtree back, fresh".

## 6. Agent-to-agent interchange

Lead agents already exchange wave plans and breakdown structures as ad-hoc
JSON. With export and import sharing one schema, that interchange becomes a
pipeline: a planning agent emits a seed, a reviewing human edits it, an
executing agent imports it — or an execution lead in one repository captures a
proven decomposition for a sibling project's lead. One schema in both
directions means agent tooling learns a single format, and the scoped export of
a milestone is simultaneously a machine-readable status artifact and a
replayable plan.

## 7. Reproducible showcases and evaluations

Demo and evaluation repositories (the planning-bracket showcase, skill eval
fixtures) need identical work graphs on every run. A committed seed plus batch
creation replaces bespoke setup scripts, and fixture drift becomes a reviewable
JSON diff.
