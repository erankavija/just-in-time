# Execution handoff: 0f8efc37

Status: released. Planning `110a4a2f` and breakdown `32607213` are Done.
Coverage-preview passed in run `e442f916-31c6-4103-a4fa-0042de96c349`;
breakdown-review passed in run `cbd82ec4-d697-462c-9054-78a7d54363f6` with
all 13 terminal assignment simulations passing. Repair `db094cac` is Done,
merged in `876f41a81`, and installed; the real breakdown activation and completion
succeeded without force. A fresh execution lead may dispatch the ready frontier.

## Authoritative inputs

- `dev/active/0f8efc37-frontends-share-core/plan.md`: approved contracts and scope.
- `breakdown.json` in that directory: authoritative worker assignments.
- `instantiation.json`: existing semantic-key-to-UUID map and all external edges.
- `investigation.md`: evidence and caller inventories; refresh line locations and
  moved callers against the landing base before dispatch.

The graph has already been created and verified. Do not batch-create it again.
P `110a4a2f` is Done; its configured review run
`7c888b70-8279-4385-88f3-55a1938d13e5` passed after one rejected premise
about native external-edge publication was corrected. B's earlier review was
interrupted after a new infrastructure prerequisite changed its evidence; that
runner error (`b442f1f7-20fd-4fcc-b640-b081fec8d79f`) is not a completed
review verdict. The earlier P failure is preserved as
`d8f0b4fd-84e3-41a3-9b76-7fb8f2fccdd7`. The successful final B results above supersede that interrupted run.

## First implementation wave

At bracket completion, these four leaves are dependency-ready. Re-query live
states at dispatch; these are a frontier snapshot, not new edges.

| Issue / key | Suggested worker | Scope | Dispatch constraint |
|---|---|---|---|
| 9c3eb2bf / layout-binding | Terra, high | Storage layout binding, executor/bootstrap, all inventoried callers and conformance tests | Fallible API cutover includes caller migration. Coordinate server startup edits with Git owner c5062e21. |
| 1d2235b1 / watch-paths | Sol, high | `crates/server/src/watcher.rs`, `crates/jit/src/repository_state/path.rs` | Independent path predicate and consumer cutover. |
| 1cdf5965 / layout-indexes | Terra, high | `GraphView.tsx`, `clusterAwareLayout.ts` and owning tests | Index existing bulk passes; preserve container-selection policy. |
| abf68973 / server-modules | Sol, high | `crates/server/src/main.rs`, `crates/server/Cargo.toml` | Available fourth assignment; coordinate main.rs with layout-binding's caller migration and c5062e21 startup work. |

Fill three implementation slots with binding, watcher, and layout indexes in
isolated worktrees and private build targets. The fresh lead may substitute the
module task if a concrete same-hunk startup conflict makes binding temporarily
unavailable. Shared filenames require inspection and coordinated landing, not
invented DAG dependencies. AI review gate subprocesses can run concurrently and
do not consume the implementation-worker allocation or a global review lock.
Respect current full-build/resource coordination when running expensive checks.

## Subsequent frontier and external owners

`executor-offload` (0f302914) additionally waits on df129d9e owner `487f45a9`.
Once complete, it releases shared-capture, semantic-search, raw-responses, and
api-pruning (whose external fb38bf1f prerequisite is already Done at preparation).
These tasks share routes.rs; dispatch/rebase around concrete function footprints.
Do not invent independent executor helpers to avoid integration.

Documents (d4172016) also waits on Git document diff `933ff852`; external result
and dispatch owners `fb38bf1f` and `f35c86de` are Done at preparation. The Git-owned
startup task `c5062e21` satisfies REQ-01 through core-story; do not recreate it.
Its expanded selected-root/shared-bootstrap contract is in the live issue and
Git owner's plan, with that owner's approval refresh responsibility retained.

Watcher completion releases config-refresh, which shares GraphView.tsx with
layout-indexes. Semantic-search releases browser-search, then graph-summary;
configuration refresh and graph-summary also share GraphView.tsx. Inspect the
actual hunks and sequence or reconcile their landings while filling other slots.

## Contracts that must survive implementation

- Preserve `/issues`, `/health`, and HTTP document diff. Remove only the routes
  specified by api-pruning. MCP retirement is separately completed work.
- Keep top-level `jit search` as raw-file/history search. Semantic issue and
  linked-document search converges separately; an empty linked-path set must
  never widen its corpus.
- Capture issues once per invalidation generation for graph/base projections.
  Compose live reported-document discovery over captured records, so artifact
  add/delete/rename remains visible without another issue listing.
- Share in-flight capture, retry failures, and prevent stale completion from
  replacing a newer generation. Never hold the writer lock across async work.
- Configuration changes reload fresh hierarchy declarations; full-graph browser
  metadata is computed before view filters. Ignore stale search responses.
- Use existing core/storage boundaries and external Git primitives. The manifest
  includes one approved executor-offload chain-link override because its complete
  route migration is the runtime isolation outcome.

## Validation and provenance

Run configured issue gates and workspace checks for each change, then container
quality gates and holistic review under jit-execution-lead. Preserve failed and
interrupted gate evidence; no placeholder review is completion evidence.

Exact manifest fidelity passes after the independent membership cleanup. The
historical 80-warning comparison establishes provenance only; standalone
`3f844900` subsequently resolved those labels in `6020e79b4` and `7365be674`, and
the steward verified zero remaining warnings/divergences. The investigation
appendix and `warning-provenance.json` retain the historical comparison evidence.

## Concurrent standalone dependency-tree repair

`06177584` owns bounded recursive dependency-tree output in core graph commands
and output. At handoff preparation, searches found no `DependencyTreeNode` or
`build_dependency_tree` consumers in `crates/server` or `web/src`; `/graph` and
this epic's browser work consume flat graph nodes and edges. No specific planned
hunk conflict or prerequisite was identified. Preserve that surface distinction
and recheck actual consumers if the standalone implementation changes scope.
MCP is retired; its mention in the older standalone issue is not a live consumer.

## Binding review advisory: dispatch scope

Both configured reviews passed with one low advisory: the manifest's five-file
footprint uses its uncertainty clause to include the complete caller inventory.
The binding worker owns the fallible API plus the mechanical caller migration
as one cutover. Beyond storage and executor/bootstrap, that inventory covers
command-family tests and shared fixtures (`commands/test_helpers.rs`,
`test_utils.rs`), profile application, CLI gate/repository workflow tests, document
and template harness suites, configuration tests, and the server's startup
constructor. Resolve every actual `configure_repository_layout` and `with_layout`
consumer from fresh searches; historical issue/planning prose in the inventory
is evidence, not a code consumer to edit. This clarifies dispatch scope without
changing the approved manifest or creating a new assignment.
