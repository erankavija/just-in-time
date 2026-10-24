# Manifest-first Synthesizer Prompt

Write `[MANIFEST_PATH]` first, then `[PLAN_PATH]`; create no issues.

Inputs: container `[C_ID]` / planning node `[P_ID]`, confirmed criteria
`[CRITERIA]`, decisions `[DECISIONS]`, investigation `[INVESTIGATION_PATH]`,
research `[RESEARCH_PATHS]`, type hierarchy `[TYPES]`, gate tiers `[GATES]`, and
content standards `[CONTENT_STANDARDS]`.

## Manifest

Follow the canonical JSON schema and helper in this skill. Produce a bare array
accepted directly by `jit issue batch-create`. Each entry contains every native
field plus `planning`; its `description` is the complete final issue body. Use
semantic keys, concrete membership/coverage labels and gates, and real dependency
edges. A coverage label names the container it credits:
`satisfies:<C-short-id>/<id>` for `C`, `satisfies:{<key>.short_id}/<id>` for a
container this manifest creates. `planning.source_refs` provides total criterion/design/finding coverage;
`contract_refs` names contracts defined once in the plan. Mark each plan heading
`plan-fixed` or `implementation-produced`; the latter has exactly one manifest
producer named by `produces_contracts`, and every consumer depends transitively
on that producer.

Keep a complete known-source universe and distinguish mandatory coverage from
optional design/finding references. Every non-finest entry must aggregate a
strictly finer in-manifest descendant; never use a higher tier to hide leaf work.

Make each finest-tier entry the largest coherent change one worker can implement
and review in one focused cycle. Adding a form, moving its callers, and deleting
the old form within one module is one entry, with its tests and docs. Merge
entries that are only chain links or bridge producer/remover pairs. Split only
when one worker cannot deliver the entry in one cycle, or when it combines
implementation with release. Group a multi-area container's entries under the
configured intermediate tier. Stay within the validator's issue budget; when the
criteria cannot fit, list criteria to defer under Needs decision.
`landing_group` records shared integration only.

Every terminal declares a `footprint`: non-empty `creates` and/or `touches`, or a
concrete `uncertainty` reason. New files belong in `creates`; greenfield work has
no exemption. Footprints and exact overlaps remain reviewer-visible advisories.

## Plan

Use [plan-doc-template.md](plan-doc-template.md). Keep it concise: outcome and
criterion approach; named contracts; generated overview; material risks and owner
decisions; investigation links. Cite exhaustive inventories instead of copying
them. Do not copy issue bodies, task criteria, DAG prose, or review history.

Run manifest validation with the complete known-source universe, mandatory
criteria as required sources, and warnings denied. A necessary exception names
only the affected stable code in `terminal.warning_overrides`; its reason remains
reviewer-visible. Never use `worker_sized_reason` as a global waiver. Then render
write/check and run native batch-create dry-run. Fix failures before returning.

[RETURN CONTRACT — paste `.agents/skills/jit-manage/references/return-contract.md` verbatim]

Outcome deliverables: both paths and issue/edge counts. Unresolved owner decisions go
under Needs decision.
