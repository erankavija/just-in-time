# Scoped batch-shape graph export: design notes

Implementation notes for issue 3e12ffbd. Companion to the use-cases document
(`3e12ffbd-batch-export-use-cases.md`).

## Surface

`jit graph export` gains two orthogonal flags:

- `--scope <container>` — restrict the export to a container's membership.
  Composes with every format.
- `--format batch` — emit the batch-create input schema (a bare JSON array of
  `BatchIssueDef`) instead of a graph rendering.

`--scope` alone filters the node set for `dot`/`mermaid`/`json`; `--format
batch` alone exports the whole graph in batch shape (REQ-08). Together they
export one container's subtree as a replayable seed.

## Scope = DAG-authoritative containment membership

The membership set is the container's **containment closure under the resolved
hierarchy** (`graph::hierarchy::resolve_hierarchy`), assembled by
`HierarchyResolution::membership_closure`: the container plus every node reached
by descending resolved `children` edges. Resolution stays repository-wide (a
scoped node's `parent`/`cluster` may point outside the subtree), exactly as
`resolve_hierarchy_tree` already scopes `graph tree`.

Containment membership — not the raw dependency closure
(`bracket_scope_ids`/`get_transitive_dependencies`) — is what makes REQ-06
meaningful. A dependency closure follows edges into *other* containers (a task
in epic E1 that depends on a task owned by epic E2 drags E2's node in), so it
has no boundary to cross. Containment membership stops at the container's own
subtree, so a cross-container dependency is a genuine boundary edge that
REQ-06 cuts and reports.

## Batch projection (`--format batch`)

Per in-scope node, a `BatchIssueDef`:

- `key` = short id (the symbolic key other entries reference).
- `title`, `description`, `priority` — copied.
- `type` — the `type:*` label value, lifted out of `labels` into the `type`
  field (batch-create re-applies it as a `type:` label; leaving it in `labels`
  too would trip namespace-uniqueness).
- `gates` — the node's `gates_required`.
- `labels` — surviving generic labels (see stripping below).
- `depends_on` — short ids of in-scope, non-bracket dependencies.

No lifecycle fields (state, assignee, timestamps) are emitted, and every
in-scope node exports regardless of state (REQ-03). `BatchIssueDef` derives
`Serialize` with `skip_serializing_if` on the defaultable fields, so the export
type *is* the import type — schema symmetry by construction (REQ-02/REQ-07).

### Identity-bound label stripping (REQ-04)

A label is dropped when its namespace is identity-bound, derived from config
(never hardcoded, `@/inv/domain-agnostic`):

- membership namespaces from `[type_hierarchy.label_associations]`
  (`HierarchyConfig::membership_namespaces`) — e.g. `epic:`, `milestone:`;
- the coverage rule's `satisfies-namespace` and `container-from-label` values,
  read from every `label-coverage` assertion in the effective ruleset — here
  `satisfies:` and `brackets:`.

`type:*` is always lifted to the `type` field. All other labels survive.

### Bracket-node exclusion (REQ-05)

Nodes whose type is a template planning- or breakdown-role node type
(`GraphTemplate::planning_type`/`breakdown_type` over every registered template)
are dropped from batch output together with every edge that touches them. This
keeps captured seeds template-compatible: the importing container scaffolds its
own bracket via `jit apply`.

### Boundary edges (REQ-06)

For a batch node's dependency:

- target in the batch node set → internal edge (`depends_on` entry);
- target is a bracket node → dropped as part of REQ-05 (silent, by policy);
- target is a real issue outside the membership scope → **boundary edge**:
  excluded and reported (count + `from -> to` short-id pairs) on stderr;
- target is a dangling id → skipped (the integrity check owns broken edges).

Boundary edges are returned in the `BatchExport` struct so callers/tests can
inspect them, and rendered to stderr so stdout stays a clean, pipeable batch
array.
