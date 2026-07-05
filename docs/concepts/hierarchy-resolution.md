# Hierarchy Resolution

> **Diátaxis Type:** Explanation

JIT expresses containment two ways, and they can disagree. This note states the
canonical model: **the dependency DAG is authoritative; membership labels are
advisory.** The resolution lives in the core library
(`crates/jit/src/graph/hierarchy.rs`) so the CLI, the web UI, and exports all
share one answer instead of each re-deriving it.

## Two ways to express containment

1. **The dependency DAG.** A container issue *depends on* the work it contains:
   an epic depends on its stories and tasks, a milestone on its epics. Following
   a container's outgoing dependency edges into more-tactical nodes yields its
   contents. Dependencies are otherwise unrestricted — any issue may depend on
   any other (see [Core Model](core-model.md)).
2. **Membership labels.** An advisory grouping tag (`epic:auth`,
   `milestone:v1.0`) an issue carries to say "I belong to this group."

When a label claims a membership the DAG does not back, the **DAG wins**. The
disagreement is *reported* (via `jit query divergence` and an advisory count in
`jit validate`), never silently reconciled: the label does not change the
resolved hierarchy.

## What counts as a container

Container-ness comes from the configured
[type hierarchy levels](../reference/configuration.md), never from label
namespaces. A type is a **container** iff its level is strictly less than the
deepest (leaf) configured level. With the default
`milestone=1, epic=2, story=3, task=4`, the leaf level is `4`, so `milestone`,
`epic`, and `story` are containers and `task` is a leaf. A node whose type is
unknown to the config, or that carries no `type:` label, has no level and is
treated as a leaf.

## The resolved facts

For every node the resolver produces four facts:

| Fact | Definition |
|------|------------|
| **parent** | The *nearest dominating container*: among all containers whose dependency closure includes the node, the one with the deepest level (the most-tactical container that still contains it). Ties break deterministically — fewer hops, then the lexicographically smallest container id. `null` for a root. |
| **children** | The inverse of `parent`: the nodes whose resolved parent is this node, sorted by id. `parent`/`children` therefore form a consistent forest. |
| **cluster** | The strategic root of the parent chain: follow `parent` pointers up to the topmost container. A leaf with no container ancestor has no cluster; a root container is its own cluster. |
| **rank** | The longest dependency-path length from the node to a sink (a node with no in-set dependencies). Sinks have rank `0`. A stable layout depth. |

### Defined edge cases

- **Root nodes** (no container ancestor): `parent = null`. A root *container* is
  its own `cluster`; a root *leaf* has `cluster = null` (an orphan).
- **Multi-parent / diamond containment** (two containers both contain a node):
  exactly one is chosen as the canonical `parent` by the deterministic tie break
  above; the node appears under only that container's `children`.
- **Rejected / archived containers**: resolution is purely structural and does
  not consult issue state. A rejected or archived container still contains its
  DAG descendants; a consumer that wants to hide such subtrees filters by state
  itself.

## Deliberate divergence from the web UI's historical clustering

The web UI's original clustering (for display) picked the epic level as its top
render cluster and treated milestones as visible nodes rather than containers.
The canonical core instead treats **every** non-leaf level as a container, so a
task under an epic under a milestone clusters to the *milestone* (the strategic
root). The DAG-authoritative rule wins; the UI's level choice is a presentation
concern layered on top. The web now shares the canonical resolver
(`web/src/utils/hierarchyResolution.ts`), pinned to the core by the shared
fixture `test-vectors/hierarchy_resolution.json` (asserted from both Rust and
vitest).

## Where it surfaces

- **`jit graph tree [<root-id>] --json`** — resolved parent/children/cluster/rank
  per node.
- **`jit graph export --format json --full`** — each node gains `resolved_parent`
  and `cluster` (additive; the default summary shape is unchanged).
- **`jit query divergence`** — membership labels not backed by the DAG.
- **`jit validate`** — an advisory `divergence_count` (never changes the exit
  status).

See [CLI reference § Graph Commands](../reference/cli-commands.md#graph-commands)
for exact JSON shapes.
