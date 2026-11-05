# Plain Breakdown Analyst Prompt

Fill the bracketed inputs and dispatch a write-capable agent. It authors a
manifest; it does not create issues or edit `.jit/`.

## Inputs

- Parent: `[PARENT_ID]` — `[PARENT_TITLE]`
- Parent description: `[PARENT_DESCRIPTION]`
- Specification: `[SPEC_PATH]`
- Manifest output: `[MANIFEST_PATH]`
- Configured type hierarchy: `[TYPE_HIERARCHY]`
- Membership label: `[MEMBERSHIP_LABEL]`
- Gate tiers: `[GATE_TIERS]`
- Required criterion/source ids: `[REQUIRED_SOURCES]`

Read the specification, live repository, and content standards; all supplied
paths are resolved from the repository root. Use the canonical manifest schema
linked from [plan-schema.md](plan-schema.md). Write only a bare JSON array to
`[MANIFEST_PATH]`. Every entry uses native batch-create fields
`key`, `title`, `description`, `type`, `priority`, `labels`, `gates`, and
`depends_on`, plus `planning`.

Use semantic kebab-case keys, never `T1`/`C2`. Descriptions are final standalone
issue bodies with observable `## Success Criteria`; labels include membership
and exact `satisfies:<container-short-id>/<id>` credits
(`satisfies:{<key>.short_id}/<id>` for a container created in this manifest);
gates are concrete configured keys. Express
relationships only in `depends_on`.

`planning` supplies one concise `outcome`, semantic `contract_refs` (possibly
empty), optional `produces_contracts`, non-empty `source_refs`, optional
`landing_group`, and terminal metadata.
Every finest-tier issue has exactly one scalar `consumer_family`, one scalar
`test_boundary`, and a concrete `worker_sized_reason`.

Each terminal also discloses non-empty `footprint.creates`/`touches` or a concrete
`uncertainty` reason. Greenfield files use `creates`. Plan-fixed contracts have no
producer; each implementation-produced contract has exactly one producer and
every consumer depends transitively on it.

Enumerate every valid criterion, finding, and design-section source id before
validation. Non-finest entries aggregate strictly finer descendants. If a sizing
heuristic is genuinely indivisible, override only its stable code with a concrete
`warning_overrides` reason; the override remains visible to review.

Before returning, simulate assigning every finest-tier entry to one worker. Each
is the largest coherent change one worker can implement and review in one
focused cycle, with one consumer family and one observable test boundary.
Adding a form, moving its callers, and deleting the old form within one module
is one entry, with its tests and docs. Merge entries that are only chain links
or bridge producer/remover pairs. Split only when one worker cannot deliver the
entry in one cycle, or when it combines implementation with release. Stay within
the validator's issue budget; when the criteria cannot fit, list criteria to
defer. A shared `landing_group` never combines tasks.

Self-check unique/known keys, total source coverage, acyclicity, real parallelism,
and blank-workspace roots.

[RETURN CONTRACT — paste `.agents/skills/jit-manage/references/return-contract.md` verbatim]

Outcome deliverables: manifest path and issue/edge counts.
