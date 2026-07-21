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
and exact `satisfies:<id>` credits; gates are concrete configured keys. Express
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

Before returning, simulate assigning every finest-tier entry to one worker. Split
horizontally until it has one outcome, one bounded consumer family, one observable
test boundary, fits one focused implementation/review cycle, needs no internal
decomposition, and does not mix independently testable foundation, migration,
deletion, documentation, or release work. Broad quantifiers, independent verbs,
several component families, three acceptance clusters, and implementation plus
release are presumed oversized. A shared `landing_group` never combines tasks.

Self-check unique/known keys, total source coverage, acyclicity, real parallelism,
and blank-workspace roots. Return the manifest path and a short count summary;
return no pasted manifest.
