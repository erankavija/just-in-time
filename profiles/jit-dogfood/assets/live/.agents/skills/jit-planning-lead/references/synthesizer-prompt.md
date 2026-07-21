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
edges. `planning.source_refs` provides total criterion/design/finding coverage;
`contract_refs` names contracts defined once in the plan.

Refine every finest-tier entry horizontally until a one-worker simulation passes:
one outcome, bounded consumer family, observable test boundary, one focused
implementation/review cycle, no internal decomposition, and no mixed foundation,
migration, deletion, documentation, or release deliverables. Never use lack of a
finer type or one final landing as justification. `landing_group` records shared
integration only.

## Plan

Use [plan-doc-template.md](plan-doc-template.md). Keep it concise: outcome and
criterion approach; named contracts; generated overview; material risks and owner
decisions; investigation links. Cite exhaustive inventories instead of copying
them. Do not copy issue bodies, task criteria, DAG prose, or review history.

Run manifest validation with all criteria as required sources and warnings denied,
render write then check, and native batch-create dry-run. Fix failures before
returning. Return only both paths, counts, and unresolved owner decisions.
