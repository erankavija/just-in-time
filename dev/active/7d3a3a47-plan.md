# Plan: Dependency-aware container artifact archival (7d3a3a47)

> Planning node: 1dbc72f6. Container criteria source: 7d3a3a47 `## Success Criteria`.

This plan builds a pure artifact-plan domain capability and a unified `jit archive`
command family that consumes it. It grounds every claim in
`dev/active/7d3a3a47-investigation.md` (citations re-verified against live code) and
carries the owner-approved decisions from `dev/active/7d3a3a47-planning-brief.md` (D-1
through D-11, with the 2026-07-13 owner amendment scrapping document categorization) as
binding. The safety guarantee is **referential consistency, not atomicity** across
filesystem files, issue JSON, and the append-only event log.

## 1. Completeness vs criteria

Narrative of the approach per `[hard]` criterion. The criterion→item contract lives once,
in the §3 coverage map.

| Criterion | Approach (how it is met) | Notes / open gap |
|---|---|---|
| REQ-01: deterministic plan of every issue-linked artifact in a container subtree plus every supported embedded local dependency | Resolve the container root through normal storage id semantics, enumerate the root plus its **resolved-hierarchy descendants** (the DAG-authoritative membership relation, `graph/hierarchy.rs:458`, `:321` — not the raw dependency closure, which absorbs cross-container sequencing edges; D-25), collect every distinct `DocumentReference` on those issues as **(path, version)** artifacts (D-26) including opaque/binary roots, canonicalize every supplied pinned revision through git to its full immutable commit object ID, then recursively discover supported local dependencies (Markdown/HTML element URLs and CSS `url()`/`@import`) with cycle detection and deterministic ordering. Output is one stable, fully enumerated plan object with destinations computed by the D-12 mirror rule. | Opaque roots (CSV/PNG/SVG) are inventoried without an adapter; parsing support gates only embedded-edge discovery, not root eligibility. Pinned versions serialize, order, and hash only the canonical full OID; resolution failure blocks (D-26). |
| REQ-02: classify each artifact move/copy/retain/blocked using active references, sharing, managed-path policy, destination conflicts | Destination provenance is binding. A content-identical destination that predates the instance is `already_archived`/`preexisting-identical` and always supplies needs-source: no publication is needed, selected references may relink only after destination revalidation, and source copy/retain semantics forbid pending deletion. `ArchiveStarted.publication_mapping` is intent only. `published-by-instance` requires an exact `completed_publications` mapping in a durable, non-reconciliation `ArchiveExecuted` receipt written after storage reports successful no-replace publication. The existing archive-root, per-reference, managed/permanent, and edge calculus then applies. | Pre-existing identical, unproven publication intent, and receipted instance publication are never conflated. |
| REQ-03: archive an eligible container without losing files, overwriting destinations, or leaving dangling issue references | Source quarantine is forbidden until the non-reconciliation `ArchiveExecuted.completed_publications` receipt append succeeds. Immediately before quarantine, execution open-handle verifies that receipted destination against the initial content_identity and holds the handle through the decision. Mismatch means no removal: restore/retain source and safely abort/block. Recovery repeats verification without recapture and cannot synthesize publication provenance. Started intent without the receipt is downgraded to `preexisting-identical`/publication-unproven, retains source, and closes safely through `ArchiveAborted`. | Failure injection covers external identical creation after Started, crash after successful publication but before its receipt, and modified pre-existing/newly published destinations; source remains. The residual verified-handle/path-rebinding race is bounded by D-24, not denied. |
| REQ-04: preserve functional relative links for supported bundles (HTML with sibling CSS, theme files, figures) | Recursive discovery follows HTML→CSS→nested figure/font edges and CSS `@import`/`url()`. D-16 puts every relative-edge target at the current mirror: an archive-root dependency is copied there while retained at its archived source; root-relative and staying-parent edges keep using the source. Execution validates every supported edge before metadata commit, fixing the current verifier's source/destination mismatch (`document.rs:1832-1858`). | Detected JavaScript/runtime loading blocks per D-23. Archive-root sources are never moved/deleted. |
| REQ-05: report container-oriented candidates using current terminal state, policy, ownership, and blockers without mutation | A read-only `jit archive candidates` lists terminal containers (container-ness from the configured type hierarchy, D-20; membership per D-25), each with the three-state documentation-policy status (`configured`, `incomplete`, or `unconfigured`), repository-wide ownership evidence, artifact counts, target-specific blockers, and move/copy/retain summaries. Candidate details distinguish the `unmanaged-selected-root` blocker from `unmanaged-path` evidence on embedded dependencies, whose actions are only copy or retain. It consumes the same plan model, always evaluates fully where policy inputs are explicit, and performs no filesystem, issue, or event mutation. | Defaults may support display but never authorize mutation; incomplete/unconfigured policy remains ineligible. No time/retention semantics (D-6). |
| REQ-06: structured JSON previews verified against Markdown, HTML, CSS, CSV, PNG, and SVG fixtures | Fixtures additionally cover (a) an initially identical pre-existing destination modified after Started, (b) a receipted instance-published destination modified before deletion/retry, (c) an external identical create after Started but before JIT publish, and (d) crash after JIT publish succeeds but before the non-reconciliation `ArchiveExecuted` receipt; every unproven/mismatched case retains source and aborts/blocks safely. Existing format, quarantine-crash, nested-mount, recovery-target, ownership, and cycle cases remain. | Preview is informational; execute ignores it and recomputes/recoveries under the held guard (D-7). |

No criterion is silently narrowed or dropped. No missing criterion surfaced during
planning.

## 2. Technical soundness and architectural fit

- **Approach.** Introduce a pure artifact-plan model in the domain/document layers: inputs
  are the shared target-root inventory (resolved container subtree roots or an arbitrary
  normalized document root with repo-wide owners/version variants), documentation policy, and
  extracted dependency edges; output is one deterministic, serializable plan. Container
  traversal uses the resolved-hierarchy membership relation (D-25). The command layer loads
  inputs, calls the pure planner, renders preview, and executes an accepted plan; it does not
  duplicate decision logic between preview and execution. A storage-owned artifact mutation
  primitive centralizes staging, containment, no-replace finalization (D-14), quarantine
  deletion (D-17), and collision semantics so the command layer stops issuing raw `std::fs`
  calls (`document.rs:1637-1787`). Candidates, preview, and execution are all consumers of
  the one plan model (formal-planning obligation 5).

- **Container membership (D-25).** The archival subtree is the container plus its
  resolved-hierarchy descendants: the repository-wide hierarchy resolution
  (`graph/hierarchy.rs:458` `resolve_hierarchy`) assigns each node its parent and children
  (`:300`, `:321`), and membership is the transitive `children` closure of the root. The raw
  dependency closure (`DependencyGraph::get_transitive_dependencies`, used for tree *scoping*
  in `graph.rs:206-233`) is **not** membership: it absorbs sequencing edges onto issues that
  belong to other containers, and archiving those would relocate another container's
  artifacts (violating D-2).

- **Artifact identity (D-26).** An artifact is **(path, version)** with version
  `working-tree` or a canonical full immutable commit object ID. Every supplied pinned
  revision string (symbolic name, abbreviation, or full ID) first passes through a
  storage/git resolver with `rev-parse --verify <rev>^{commit}` semantics. The resolver
  returns the repository's hash-algorithm-agnostic full OID; only that value is serialized,
  ordered, fingerprinted, and used in identity comparisons—never the symbolic input or the
  current seven-character display value. Working-tree versions are the archival subjects. A
  pinned reference then reads through the storage layer at that canonical commit
  (`document.rs:349-357` — effective commit precedes working-tree; `storage/mod.rs:531`
  `read_path_text(path, canonical_oid)`), i.e. from git history: relocating or deleting the
  working-tree file does not break a git-resolvable pinned read, and per D-3 pinned
  references are never rewritten. Therefore a pinned reference imposes **no** working-tree
  retention constraint when revision canonicalization and every commit-specific read
  succeed. If revision canonicalization fails, git is unavailable, the commit is unreachable
  (`CommitNotFound`), or the referenced root or supported dependency is
  absent at a reachable commit (`NotFound`), planning records `pinned-read-failed` and makes
  the whole target ineligible for execution. There is no warning-plus-retention treatment and
  no working-tree fallback; D-3 remains intact because the pinned reference is untouched.
  Pinned versions appear in the plan as
  informational non-relocating entries with evidence `pinned-historical`, **including their
  commit-specific supported dependency closures**: the adapters are pure text extractors,
  so discovery runs them over commit-resolved content (`read_path_text(path, canonical_oid)`,
  `storage/mod.rs:531`) exactly as over working-tree content, producing (path, full OID)
  dependency entries that are likewise informational and non-relocating — REQ-01's
  "every supported embedded local dependency" is met for pinned roots deterministically,
  with nothing to move because history serves every pinned reader.

- **Durable content identity.** Every working-tree source that will be published or appears
  in planned deletions carries `content_identity = {sha256, byte_size}`, with both values
  derived from the same byte buffer/read so a concurrent edit cannot create a torn identity.
  SHA-256 follows the
  existing asset convention (`document/assets.rs:269-276`); byte size makes the comparison
  explicit. While holding `RepoWriteGuard`, the command reads these identities before
  `ArchiveStarted`, and Started persists each identity beside its exact source, destination,
  selected reference changes, and deletion intent. Initial staging must match the recorded
  digest+size before publication. Retry/reconciliation always uses the initial Started
  identity and never recaptures an expected identity from the current source. Every removal
  uses D-17 quarantine then open-handle verification against that recorded identity; mismatch
  is never deleted, is restored no-replace or remains quarantined/reported, and forces a safe
  abort or leaves recovery open with a blocker.

- **Destination layout (D-12).** The destination root is `<archive_root>/` plus, for
  container targets, `<container.id>/` (the full UUID — short-id prefixes can collide across
  containers, full UUIDs cannot, matching `.jit/issues/<uuid>.json` naming); every moved or
  copied artifact lands at that root plus its repository-relative source path. Archival
  takes no category input (D-21): the mirror rule alone determines every destination.
  Mirroring under one common prefix keeps every relative offset between bundle members
  invariant, keeps identical filenames from different directories distinct, and makes
  intra-plan destination collisions impossible. The mapping is invertible, which retry
  convergence exploits (D-18).

- **Destination-local staging (D-14).** Publication never assumes repository root and a
  nested destination share a device. Resolve the normalized intended destination through
  its nearest safe existing ancestor (never `canonicalize` the not-yet-created path), then
  create missing destination parents after Started. Before Started, derive/persist a unique
  hidden owner-private staging directory+entry adjacent to each destination from
  operation_instance_id plus SHA-256 of canonical destination path; both paths must be absent and
  component/symlink-safe (`0700` directory on POSIX, platform-equivalent owner-only access).
  After Started, create it, write the source bytes, verify Started
  identity, then no-replace publish on that destination filesystem. The command appends the
  successful publication receipt before staging cleanup or any source quarantine. Cleanup is best-effort
  and removes only the recorded verified entry and an empty recorded directory; it never
  erases a nonempty/unverified entry. Started also records destination-adjacent quarantine
  mappings for conditional rollback removals.

- **Destination provenance and deletion authority.** `preexisting-identical` means matching
  content occupied the destination before this operation instance. It is already_archived
  evidence and satisfies needs-destination without publication, but always adds needs-source:
  source is retained, never enters pending_deletions, and selected references relink only
  after revalidating destination against the planned identity. A Started
  `publication_mapping` records intent only and never proves who created the destination.
  `published-by-instance` exists only when storage returned successful no-replace publication
  and the command then durably appended that exact mapping in a **non-reconciliation**
  `ArchiveExecuted.completed_publications` receipt. Only that receipt-backed provenance can
  authorize source deletion. Immediately before
  quarantine, open the recorded destination, verify initial Started SHA-256+size, and hold
  the verified handle through the source-quarantine decision; recovery repeats this without
  recapture. Mismatch retains/restores source and safely aborts or blocks.

- **Archive-root evidence and precedence.** For fresh planning, before managed/permanent or
  selected-root classification, normalize every working-tree path and test component
  containment under configured `archive_root`. A match supplies `archived-source` and
  already-existing evidence plus needs-source, bypasses `unmanaged-selected-root`, and can
  never move or enter `pending_deletions`. A selected/direct root retains at its existing
  source with `already_archived: true`, `action: retain`, `destination: null`, and empty
  `reference_changes`. An embedded archive-root dependency instead remains edge-aware: a
  relative edge from a relocated parent supplies needs-destination, so it copies to the
  current target mirror while its original remains; root-relative or staying-parent edges
  retain. Retired `dev/archive/features/...` and another container segment follow the same
  rule. Recovery uses the Started event's recorded root, not current configuration.

- **Ownership universe (D-13).** Repository-wide ownership is computed over every
  `DocumentReference` across all issues plus the recursive supported embedded closure of
  each (Markdown/HTML/CSS edges); an artifact referenced directly or through an embedded
  edge from outside the selected subtree is outside-owned and never moved (D-2). Files
  unreachable from any issue reference are outside JIT's referential contract: reported as
  informational not-selected entries when they sit beside bundle members (D-10), never
  silently relocated, never counted as owners. The inventory task records direct owners; the
  classifier — ordered after discovery — completes the embedded half.

- **Per-reference selection (before D-16).** The planner classifies each direct
  `DocumentReference` independently before deriving its artifact action. Pinned references
  are historical and never selected for relink (D-3/D-26). For a container target, select
  exactly the unpinned references owned by terminal issues inside the selected resolved
  subtree; references outside the subtree or on non-terminal owners stay at source. For a
  document target, D-15 first requires all direct and embedded-closure owners to be terminal,
  then every unpinned direct owner is selected. Embedded edges are not issue metadata and
  never become `reference_changes`; copy/move topology preserves them. Every unpinned
  reference not selected for relink contributes needs-source. `reference_changes` is exactly
  the selected set (issue, old path, mirror path), and no other issue reference is rewritten.

- **Edge-aware action calculus (D-16).** The action per working-tree artifact derives from
  two computed constraints. *Needs-destination* holds when the artifact is a selected
  explicit root **unless it is an archived-source direct root**, or a relocated (moved or copied) parent references it through a
  **relative** edge — the mirror layout preserves that edge only if the dependency exists at
  its mirrored path. *Needs-source* holds when the artifact has an owner outside the
  subtree, an active owner, has any unpinned direct reference not selected for relink, lies
  on a permanent path, has a preexisting-identical destination, is an embedded dependency outside
  every explicitly configured managed path, or any document that stays in place references
  it — including **root-relative**
  edges, which resolve from the repository root regardless of the referencing document's
  location (`assets.rs:151-162`). Then: move = needs-destination ∧ ¬needs-source; copy =
  needs-destination ∧ needs-source; retain = ¬needs-destination ∧ needs-source (and for
  unselected artifacts); block = destination conflict, detected dynamic loading in a
  relocated member (D-23), symlink involvement (D-22), repository escape, or any edge whose
  resolution the layout cannot preserve. Move is therefore possible only when every
  source-dependent unpinned reference is selected and no other needs-source constraint
  holds. A selected root outside every managed path blocks
  the target as `unmanaged-selected-root` before this calculus. For an unmanaged embedded
  dependency, `unmanaged-path` is evidence that always supplies needs-source: it copies when
  needs-destination also holds and otherwise retains, never moves. Permanent-path,
  outside-owner, active-owner, and unmanaged-path are
  **evidence/constraint flags**, not blocker codes: for a container they normally produce
  copy or retain through needs-source. For a document target, any non-terminal direct or
  embedded-closure owner is instead a target-level `document-non-terminal-owner` blocker
  (D-15). `unpreservable-layout` is emitted only when the required source/destination
  combination cannot preserve a supported edge; it is not an alias for ownership evidence.
  `archived-source` likewise always supplies needs-source: direct roots retain, relative-edge
  dependencies copy when their relocated parent supplies needs-destination, and other edges
  retain. No archive-root source is ever moved or deleted.
  Copy satisfies both constraint kinds, so the
  calculus is total and deterministic; execution's before/after validation then checks every
  supported edge in the final layout: relative edges at the mirror, root-relative edges at
  the repository root.

- **Archive-plan JSON schema and blocker taxonomy (binding for the plan-model task):**
  - **Envelope:** `schema_version` (starts at 1; codes are append-only and never change
    meaning), `target` (`{"kind": "container"|"document", "id"|"path": …}`),
    `destination_root`, `operation_id` (initial **definition key**, not the recovery lookup:
    hash of target,
    destination root, and the sorted canonical initial source-path set; an archive-root
    already-archived artifact contributes its existing path; D-19),
    `operation_instance_id` (UUID delimiting one logical operation by the D-19
    adopt-or-mint rule; null in previews/candidate reports), `execution_id` (fresh UUID for
    each execute attempt that emits Started or reconciliation completion, recorded in its
    events and `--execute` output; **null in previews and candidate reports**),
    `plan_fingerprint` (informational hash of the canonical plan serialization excluding
    volatile fields; expected to change as repository state changes, never an execution
    comparison input), `eligible` (bool), `policy_status`
    (`configured`|`incomplete`|`unconfigured`),
    `counts` (one integer per action plus `already_archived` and `pending_deletions`),
    `artifact_count` + `artifacts` (the repository list-envelope convention; entries
    deterministically ordered by normalized source path, then version), and plan-level
    `blockers`/`warnings` (each sorted by code, then path).
  - **Artifact entry:** `source`, `version` (`working-tree` or the canonical full immutable
    commit object ID returned by the D-26 resolver; never a symbolic/abbreviated revision),
    `content_identity` (`{"sha256": <64-lower-hex>, "byte_size": N}`; required for every
    working-tree publication source or planned deletion, null otherwise),
    `destination` (null when retained or historical), `action` (`move|copy|retain|block`),
    `destination_provenance` (`none|preexisting-identical|publication-unproven|published-by-instance`) and
    `publication_required` (bool),
    `publication_mapping` (null unless publishing; exact `destination`, hidden adjacent
    `staging_dir`, `staging_entry`, content_identity, and conditional rollback-deletion
    quarantine mapping for that destination; this is intent, never provenance proof),
    `publication_proof` (null unless `published-by-instance`; exact
    `{operation_instance_id, execution_id, reconciliation:false, completed_publication}` identifying a durable
    non-reconciliation `ArchiveExecuted.completed_publications` receipt whose mapping and
    initial content_identity equal Started),
    `already_archived` (bool true for pre-existing identical destination/source evidence;
    it never by itself grants deletion authority), `provenance` (a flag set — `["explicit"]`, `["embedded"]`,
    or both), `format`, `owners` (issue id, deterministic `document_index` within that
    issue's documents, state, `inside_subtree`, `pinned`, `selected_for_relink`), `edges`
    (`supported` / `unsupported` / `external`, each with resolution mode),
    `reference_changes` (exactly the selected unpinned direct references, each with issue,
    document_index, from-path, to-path), `pending_deletions` (exact `{source,
    quarantine_dir, quarantine_entry, content_identity}` for each path this
    plan will remove — allowed only when the exact non-reconciliation completion receipt
    above exists and destination provenance becomes published-by-instance; otherwise
    the source is excluded — or a residue left by an earlier
    partial execution — each removed only through the D-17 quarantine protocol; residue
    cleanup is an executable plan operation, not an out-of-band effect), and per-artifact
    `evidence`, `blockers`, `warnings`.
  - **Policy completeness (binding):** `configured` requires the `[documentation]` table and
    explicit values for all three post-category policy inputs: `managed_paths`,
    `permanent_paths`, and `archive_root`. A missing table is `unconfigured`; a present table
    missing any one or more fields is `incomplete`. `policy-unconfigured` and
    `policy-incomplete` make execution ineligible. Accessor defaults may be shown as preview
    context but **never** supply delegation or authorize mutation.
  - **Evidence/constraint flags (stable kebab-case, not blockers):** `permanent-path`,
    `outside-owner`, `active-owner`, `unmanaged-path`, `archived-source`,
    `pinned-historical`. The first five
    feed needs-source and normally yield `copy`/`retain` for container planning;
    explicit/embedded origin stays in the separate `provenance` field.
  - **Blocker codes (stable kebab-case):** `policy-unconfigured`, `policy-incomplete`,
    `unmanaged-selected-root`, `destination-conflict`, `document-non-terminal-owner`,
    `pinned-read-failed`, `unsupported-dynamic-edge`, `repository-escape`,
    `unresolvable-edge`, `unpreservable-layout`, `non-terminal-target`,
    `missing-source`, `symlink-artifact`, `archive-recovery-blocked`,
    `quarantine-unavailable`,
    `archive-recovery-ambiguous`.
    `document-non-terminal-owner` is target-specific
    to document execution; `unpreservable-layout` requires a concrete supported edge whose
    needs-source/needs-destination obligations cannot both be materialized.
  - **Blocker applicability (normative):** policy blockers are target-level and apply to
    document and container execution; `unmanaged-selected-root` applies when any selected
    working-tree root is outside every explicitly configured managed path. Every embedded
    working-tree dependency is also checked: unmanaged status contributes `unmanaged-path`
    evidence and needs-source under D-16, never a blocker by that name.
    `destination-conflict` applies to a differing-content occupied destination;
    `document-non-terminal-owner` applies only to
    document execution; `pinned-read-failed` is target-level after failed revision
    canonicalization or any failed commit-specific root/dependency read;
    `unsupported-dynamic-edge`, `repository-escape`,
    `unresolvable-edge`, and `unpreservable-layout` identify the concrete artifact/edge;
    `non-terminal-target` applies to container execution; `archive-recovery-ambiguous`
    means stable target identity matched multiple open intents across roots and forbids
    recovery/minting; `archive-recovery-blocked` means
    an adopted prior target intent cannot yet be completed or safely aborted and
    forbids a competing instance; `quarantine-unavailable` means the recorded adjacent
    same-filesystem quarantine cannot be prepared/used and leaves deletion pending without
    cross-device fallback; `missing-source` follows the working-tree rule below; and
    `symlink-artifact` follows D-22. No evidence flag makes `eligible=false` by itself.
  - **Warning codes:** `missing-edge-target`, `external-edge`, `no-owner`,
    `residue-source`, `deletion-failed`, `not-selected-sibling`,
    `quarantined-foreign-file`.
  - **Missing/artifact semantics (evaluation order is normative):** for fresh planning,
    component containment beneath configured `archive_root` runs before managed-path/root
    classification and supplies `archived-source`, already-existing, and needs-source while
    bypassing `unmanaged-selected-root`. A selected/direct root retains in place with null
    destination and no reference change. An embedded dependency copies to the current mirror
    only for a relative edge from a relocated parent; otherwise it retains. Neither action
    deletes the archived source. For pinned versions, revision canonicalization runs first; its failure, or any
    `CommitNotFound` or commit-path `NotFound` for the resulting canonical full OID, emits
    `pinned-read-failed`; the working-tree
    rules below cannot downgrade or replace that blocker. (1) **Retry mirror recognition:**
    a working-tree reference whose stated source path maps through D-12 to content-identical
    existing content is `already_archived`. Without a qualifying non-reconciliation
    `ArchiveExecuted.completed_publications` receipt it is `preexisting-identical` or
    `publication-unproven`: publication_required=false, needs-source=true, no pending
    deletion, even when Started records the exact publication_mapping. Recovery establishes
    `published-by-instance` only from that durable receipt plus current destination identity
    verification; it never infers publication from Started intent or content equality.
    (2) **`missing-source`:** only a
    working-tree root absent at both its stated location and its mirror image blocks — a
    genuinely dangling reference that must be resolved (or removed) before execution, never
    silently entrenched or skipped. A missing **embedded** target gets warning
    `missing-edge-target` on the referencing artifact: it contributes no needs-destination
    constraint, is excluded from before/after edge validation (the edge resolves nowhere
    before archival, so no regression is possible), and never blocks.
  - **Candidate collection envelope:** `jit archive candidates --json` emits
    `{"schema_version": 1, "candidate_count": N, "candidates": [...]}`; each candidate is
    the same target plan envelope above. Every check whose explicit inputs exist is
    evaluated. An incomplete/unconfigured policy reports its distinct blocker and does not
    substitute defaults to claim eligibility.
  - **Event contract (binding; four event kinds):** before computing or minting a fresh
    source-set operation—and before reading/applying current archive-root policy—scan **all**
    open `ArchiveStarted` events by stable target identity across recorded destination roots.
    Started's explicit `recovery_target` is
    `{"kind":"container","issue_id":<full-uuid>}` or
    `{"kind":"document","canonical_original":<target-root>,"destination_aliases":[...]}`
    with **only that root's own planned destination alias(es)**;
    embedded/shared dependency paths are excluded. A document invocation matches only those
    root aliases. One match recovers using that event's recorded destination root and contract,
    regardless current config. Multiple matches return `archive-recovery-ambiguous` and mint
    nothing. With no match (all prior target intents terminal), current policy may choose a
    fresh root and `ArchiveStarted` appends before the first
    mutation; under the held guard the command mints operation_instance_id, computes
    content_identity, and derives unique paths: hidden owner-private staging adjacent to each
    destination and hidden owner-private quarantine adjacent to each deletion source. Names
    derive from instance ID plus SHA-256 of canonical destination/source path. Every directory and
    entry must be absent and component/symlink-safe before append. Started then contains
    recovery_target, recorded destination root,
    new operation/instance/execution IDs, fingerprint, and sorted
    full publication/relink contract with exact staging mappings, and `deletion_intents`
    containing each exact `{source, quarantine_dir, quarantine_entry, content_identity}`, including conditional
    rollback deletion mappings for published destinations. Pre-existing identical
    destinations are recorded separately as such, with no publication_mapping or pending
    deletion; before applying their selected relinks, revalidate destination against Started
    identity or safely abort without relinking. Append failure means no mutation.
    Here “durable” is the repository's process-level append contract: `append_event` has
    returned success and the record is visible to a subsequent process; this plan does not
    invent stronger power-loss/fsync or exactly-once semantics beyond `@/inv/event-log`.
    Recovery adopts the open event's original operation/instance, recorded root, identities,
    staging/quarantine mappings, and initial
    artifact/source/destination contract. It re-evaluates current owners/references for
    safety but mutates only that contract; newly discovered unrelated artifacts wait for a
    later fresh operation. Before new recovery mutation, append a Started attempt with the
    adopted IDs, fresh execution ID, and remaining original-contract work. Normal execution
    calls storage's no-replace publish and requires its success return, then appends a
    non-reconciliation `ArchiveExecuted` with the exact successful mappings in
    `completed_publications`; source quarantine is forbidden until that append returns
    success. `ArchiveExecuted` also records relinks;
    `ArchiveSourcesRemoved` normally records removals successful in that attempt, while a
    reconciliation event records a deletion only after checking **both** recorded source and
    recorded quarantine entry. Reconciliation may report or close already-receipted work but
    can never add `completed_publications`, synthesize a publication receipt, or otherwise
    invent provenance. Only a destination backed by the exact durable non-reconciliation
    receipt for this instance can authorize deletion. Immediately before quarantine, open that
    destination, verify initial Started identity, and hold the handle through the quarantine
    decision; recovery does the same without recapture. Destination mismatch retains/restores
    source and forces safe abort/block. If Started has a matching destination but no qualifying
    receipt—including external identical creation before JIT's no-replace publish, or a crash
    after successful publish but before the receipt append—recovery downgrades it to
    publication-unproven/preexisting-identical, retains the source with no pending deletion,
    revalidates identity before any selected relinks, and closes through a safe
    `ArchiveAborted` reason `publication-unproven`; it never emits reconciliation
    `ArchiveExecuted` to bless the destination. After Started, create the
    recorded quarantine directory, atomically rename source to its recorded entry on the same
    parent filesystem, open relative to that directory handle, and verify Started digest+size.
    Match may unlink; mismatch never does and is no-replace restored or remains at the
    recorded quarantine path/reported, forcing safe abort/open recovery. Crash after rename
    is recoverable from the mapping. Source-present/entry-absent retries the rename;
    source-absent/entry-present resumes handle verification; both absent is completed; both
    present never overwrites either and requires verification/reporting before safe closure.
    Empty-dir cleanup is best-effort and never removes a
    nonempty/unverified entry. Inability to prepare/use same-filesystem quarantine returns
    `quarantine-unavailable`, leaves deletion pending, and never copy-deletes. Both carry
    `reconciliation: true|false`.
    `ArchiveAborted` is the fourth, terminal event. It may append only after recovery proves
    referential safety by restoring/retaining sources and references as current ownership
    requires, and carries the adopted operation/instance IDs, fresh execution ID, reason,
    and completed, reverted, and safe residual paths. Safe
    duplicate abort/completion events are permitted. If recovery can neither complete nor
    prove a safe abort, leave the intent open, return `archive-recovery-blocked`, and never
    mint a competing instance. An instance closes when cumulative completion events cover
    the initial Started contract or a safe ArchiveAborted terminates it; later Started
    attempts never redefine that contract. Only then may a fresh current-source-set operation
    mint. A brand-new already-converged invocation with no open Started appends no event.
  - **Symlink semantics (D-22):** an explicit root or embedded target that is a symbolic
    link, or whose repository-relative path traverses one, classifies as `block` with
    blocker `symlink-artifact`; the mutation primitive resolves paths physically, verifies
    containment on resolved targets, and never moves, copies, or deletes through a link.
    Richer symlink relocation is a follow-up.
  - **Dynamic-loading detection contract (D-23):** `<script src="…">` is a static supported
    edge (the script file is a bundle dependency like any other). The
    `unsupported-dynamic-edge` blocker fires when a relocated HTML or script bundle member
    textually contains a local-path-bearing loading construct from the binding initial
    set — runtime loaders `fetch(`, `import(`, `XMLHttpRequest`, `new Worker(`,
    `importScripts(`, a URL-bearing `data-*` attribute (e.g. reveal.js `data-markdown`,
    lazy-load `data-src`), or **static module syntax with a relative specifier**
    (`import … from './…'`, `export … from './…'`, `require('./…')`) — detected by pattern
    match over the content, never by execution or guessing (D-5). JIT has no JavaScript
    adapter, so a script whose content declares local dependencies is never relocated
    silently: it blocks until resolved by hand. Constructs outside the set are outside the
    contract; the set is append-only under `schema_version`.

- **Reuses / integrates with:**
  - Hierarchy membership: `crates/jit/src/graph/hierarchy.rs:458` (`resolve_hierarchy`),
    `:300` (`parent`), `:321` (`children`).
  - Storage read/write: `crates/jit/src/storage/mod.rs:531` (`read_path_text`, commit-aware
    for pinned resolution), `:498` (`read_path_bytes`, enables opaque binary roots without
    an adapter), `:126` (`acquire_repo_write_lock`, re-entrant `RepoWriteGuard` for the
    multi-write sequence).
  - Sequencing/rollback principles and occupied-destination no-op:
    `crates/jit/src/commands/document.rs:1637-1787`, `:1670-1683`, `:1391-1435`, with
    failure-injection coverage in `crates/jit/tests/archive_integrity_tests.rs:203-396`.
  - Domain records: `crates/jit/src/domain/types.rs:877` (`DocumentReference`; `path` :879,
    `commit` :881, `assets` :891), `:50` (`State::is_terminal` = Done|Rejected only),
    `:1466` (`Event::DocumentArchived`, superseded by the D-19 event family).
  - Adapters/scanner: `crates/jit/src/document/adapter.rs` (Markdown/HTML built-ins),
    `crates/jit/src/document/assets.rs:16` (`Asset`; `resolved_path` :20, root-relative
    resolution :151-162, supplied-map classifier limitation :189).

- **Grounding (from investigation, re-verified):**
  - Container membership primitive exists (hierarchy resolution) → **valid-and-open**:
    archive accepts only one path (`cli.rs:1950-1969`); no container consumer. The
    investigation's suggestion to enumerate via the transitive dependency closure is
    **corrected** here (D-25): the dependency closure absorbs cross-container sequencing
    edges; membership is the resolved `children` closure.
  - Referentially-consistent single-doc sequence → **already-done**: copy/relink/event/delete
    with occupied-destination no-op, failure-injection tested
    (`document.rs:1043-1082`, `archive_integrity_tests.rs:74-396`). Reuse principles; do not
    wrap `archive_document`.
  - Dry-run incompleteness → **valid-and-open**: returns before active-reference and
    destination-occupancy checks (`document.rs:1153-1176`, `:1670-1683`).
  - Directory-heuristic sharing → **valid-and-open**: `is_shared` ignored by archive;
    movability is folder placement (`document.rs:1463-1495`); the reference-count classifier
    has no archive consumer (`assets.rs:185-233`).
  - Shallow one-level discovery → **valid-and-open**: HTML sees only `src`/`href`, no
    recursion (`document/adapter.rs:101-146`).
  - Opaque roots blocked → **valid-and-open**: asset scan requires a text adapter; only
    Markdown/HTML registered (`document/assets.rs:92-111`, `adapter.rs:177-209`).
  - Pinned relink unsafe → **valid-and-open**: relink changes only `path`; reads prefer the
    pinned `commit` (`document.rs:1352-1375`, `:349-357`). Resolved by D-3 + D-26: pinned
    references are never rewritten; a storage/git resolver canonicalizes every supplied
    revision to its full commit OID, and every commit-specific read must succeed, while
    resolution failure, `CommitNotFound`, or commit-path `NotFound` blocks execution.
  - Post-archive verifier defect → **valid-and-open**: compares destination-resolved against
    source-resolved paths, so the check is normally skipped (`document.rs:1832-1858`).
  - Coordination too narrow → **valid-and-open**: no write guard held across
    plan/copy/saves/event (`document.rs`); guard available (`storage/mod.rs:126`).
  - String-prefix policy matching → **valid-and-open**: `starts_with` matches
    `dev/active-other` against `dev/active` (`document.rs:1243-1267`); use component-aware
    normalization.
  - Atomic batch execution → **invalid as stated**: true cross-surface atomicity is
    unavailable; the contract is referential consistency (`document.rs:1050-1074`).
    Preserve; do not strengthen.

Layer boundaries (per AGENTS.md, obligation 3): artifact-graph construction and action
classification are pure domain/document logic, free of I/O; storage exposes commit
canonicalization, atomic persistence, and narrowly scoped filesystem primitives for stage,
no-replace publish, quarantine/remove/restore, and physical containment. The command layer
acquires and retains `RepoWriteGuard` while it recomputes the plan and orchestrates storage
primitives, issue relinks/saves, event appends, and rollback; CLI/output stay
user-facing. Managed/permanent paths, archive root, and container-ness come from repository
configuration, never hardcoded (`@/inv/domain-agnostic`). The plan model is a versionable
list envelope with stable ordering. Existing replacing persistence continues to respect
`@/inv/atomic-writes`; D-14's atomic no-replace publication is a distinct new-file
operation, so its task must first amend the registry-first invariant wording (and render its
projection) to cover both temp-file + atomic-rename replacement and staged atomic no-replace
publication. `ArchiveStarted` makes the intended state change durable before mutation, and
every subsequent state change is represented by completion or safely-proven abort events;
an unclosable recovery remains durably open, respecting `@/inv/event-log` across retries.

## 3. Decomposition sketch (near-ready; jit-breakdown instantiates — no issues created here)

Three conceptual story-sized groups organize the work; the epic's children are the
task-tier items listed below, each independently landable and green at every boundary.
Ordering is expressed only through `depends-on`. Group A is the pure foundation; Groups B
and C consume it where needed (preview and candidates depend on the classifier); the
filesystem-only storage primitive is independent, and coordinated execution joins it with
the preview/planner surface. Groups B and C can
proceed independently after A. The plan schema and blocker taxonomy land first, before any
CLI fan-out (obligation 4). Coverage is enforced at the task tier: each
requirement-bearing task carries its `satisfies: REQ-*` label directly. The clean-cut legacy
migration is a support task, credits no container requirement, and is explicitly exempt;
group headers are organizational only, not issues.

### Group A: Artifact plan model and resolver — covers REQ-01, REQ-02, REQ-04

- **Artifact plan model and blocker taxonomy**  `type: task`  `satisfies: REQ-01`  `depends-on: —`
  Outcome: a serializable artifact-plan type implementing the §2 archive-plan JSON schema
  and blocker/warning taxonomy verbatim (envelope, artifact entry, identity fields, and
  code sets are binding), emitted as a deterministically ordered JSON envelope.
  Own criteria: `[hard] LOCAL-01: Serializes a plan with stable artifact and blocker
  ordering independent of input order.` `[hard] LOCAL-02: The same plan object is the input
  to both preview rendering and execution.` `[hard] LOCAL-03: Artifact identity is
  (path, version); successfully resolved pinned versions serialize, order, and fingerprint
  only by the canonical full immutable commit OID as non-relocating historical entries,
  while failed resolution or commit-specific reads serialize pinned-read-failed blockers.`
  `[hard] LOCAL-34: Policy status is configured only when managed_paths, permanent_paths,
  and archive_root are all explicit; partial and absent policy serialize distinctly and
  defaults never make either executable.` `[hard] LOCAL-45: Every working-tree publication
  or deletion serializes content_identity as lowercase SHA-256 plus byte_size, and Started
  binds that identity to the exact source/destination/reference/deletion entries.`
  Blast radius: self-contained new module; no existing consumer changes.

- **Target root inventory and ownership**  `type: task`  `satisfies: REQ-01`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: one versioned explicit-root inventory for both target kinds. A container yields
  its resolved-hierarchy subtree (D-25) and every distinct issue-linked root on those issues.
  A document target normalizes its arbitrary path as an explicit working-tree root even when
  opaque or zero-owner, looks up every direct owner repository-wide, and adds canonical
  pinned historical variants for matching references alongside the unpinned working-tree
  variant. Policy eligibility and zero-owner execution semantics are applied later.
  Own criteria: `[hard] LOCAL-04: Membership is the resolved-hierarchy children closure; a
  cross-container sequencing dependency contributes no member.` `[hard] LOCAL-05:
  Inventories every distinct DocumentReference in the closure, including CSV/PNG/SVG roots
  with no registered adapter and pinned versions as historical entries.` `[hard] LOCAL-06:
  Records every direct issue-reference owner of each artifact repository-wide, flagging
  owners outside the subtree.` `[hard] LOCAL-43: A zero-owner arbitrary CSV/PNG/SVG document
  path becomes a normalized explicit working-tree root with empty owners/no-owner evidence,
  without requiring a text adapter.` `[hard] LOCAL-44: A document path with mixed
  pinned/unpinned owners yields one working-tree variant plus canonical full-OID historical
  variants, with every direct owner associated to the correct version.` `[hard] LOCAL-38: Canonicalizes each supplied pinned revision
  through the storage/git resolver to the hash-algorithm-agnostic full commit OID (including
  symbolic and abbreviated inputs), and blocks a failed resolution.`
  Blast radius: adds the reusable storage/git canonical-revision resolver and otherwise
  reuses `hierarchy.rs:458` and `storage/mod.rs:498`.

- **Recursive supported-dependency discovery**  `type: task`  `satisfies: REQ-01, REQ-04`  `depends-on: Artifact plan model and blocker taxonomy, Target root inventory and ownership`
  Outcome: recursive discovery of supported local dependencies — Markdown/HTML element URLs
  and CSS `@import`/`url()` — with cycle detection, normalized component-aware path
  resolution, before/after edge reachability, repository-escape rejection, and the D-23
  detection contract for dynamic and module-loading constructs. Discovery is version-aware
  (D-26): pinned roots are scanned over commit-resolved content through the storage layer's
  commit-aware reads, yielding informational (path, canonical full OID) dependency entries.
  Own criteria: `[hard] LOCAL-07: Discovers an HTML→sibling-CSS→nested-figure chain and CSS
  @import/url() targets to full depth, treating script-element src URLs as static supported
  edges.` `[hard] LOCAL-08: Terminates on dependency cycles and reports local-path-bearing
  constructs from the binding D-23 set as unsupported edges, by textual pattern match only.`
  `[hard] LOCAL-33: Discovers a pinned root's supported closure from its commit-resolved
  content, emitting non-relocating (path, canonical full OID) entries; CommitNotFound and NotFound for
  either root or supported commit-path dependency emit pinned-read-failed and block the
  target, with no working-tree fallback.`
  Blast radius: extends the document adapter/scanner surface; existing Markdown/HTML
  callers (snapshot export) are unaffected because discovery is a new recursive path.

- **Move/copy/retain/block classification**  `type: task`  `satisfies: REQ-02, REQ-04`  `depends-on: Target root inventory and ownership, Recursive supported-dependency discovery`
  Outcome: a pure classifier assigning each working-tree artifact move, copy, retain, or
  block through the D-16 edge-aware calculus, completing the D-13 ownership universe by
  extending direct owners with the supported embedded closure over every issue-linked
  document repository-wide, proposing destinations by the D-12 mirror rule, and applying
  the D-26 pinned model, per-reference selection, archive-root precedence, explicit policy
  completeness, and the normative missing/already-archived evaluation order.
  Own criteria: `[hard] LOCAL-09: Ownership incorporates supported embedded references
  reachable from any issue-linked document repository-wide, flagging outside owners.`
  `[hard] LOCAL-10: Classifies deterministically per the calculus — a figure shared by a
  terminal and an active issue copies when a relocated parent references it relatively and
  retains otherwise, never force-relinking the active consumer; permanent-path,
  outside-owner, and active-owner are evidence rather than blockers; a successfully read
  pin imposes no working-tree constraint while a failed commit-specific read blocks the
  target.`
  `[hard] LOCAL-11: Every supported edge's resolution mode (relative vs root-relative)
  drives the decision, and the classifier never emits a layout in which a relocated
  parent's supported edge lacks its target; specifically, a relocated parent's relative
  edge into archive_root copies that dependency to the current mirror while retaining the
  archived source.` `[hard] LOCAL-12: Uses path-component
  containment to evaluate managed/permanent status for every working-tree artifact, so
  dev/active-other does not match dev/active.` `[hard] LOCAL-36: An unmanaged selected root
  emits the target blocker unmanaged-selected-root; an unmanaged embedded dependency emits
  unmanaged-path evidence, always needs-source, copies when its relocated parent needs a
  destination copy and otherwise retains, and never moves.` `[hard] LOCAL-13: Proposes destinations by mirroring repository-relative
  source paths beneath the destination root (full container UUID segment), keeping
  identical filenames distinct.` `[hard] LOCAL-14: Classifies a differing-content occupied
  destination as blocked; a content-identical pre-existing or unreceipted one as already_archived/
  preexisting-identical with publication_required=false and needs-source=true, excluding its
  source from pending_deletions; a Started publication_mapping is intent only, and only an
  exact durable non-reconciliation ArchiveExecuted.completed_publications receipt can yield
  published-by-instance deletion authority. Symlink paths block with the symlink code.`
  `[hard] LOCAL-39: Before managed/root classification, marks every path beneath archive_root
  (including dev/archive/features/... and another container segment) archived-source,
  already-existing, needs-source, and exempt from unmanaged-selected-root: selected/direct
  roots retain with null destination/no relink; relative-edge dependencies of relocated
  parents copy to the mirror; root-relative/staying edges retain; none move/delete.`
  `[hard] LOCAL-40: Computes
  reference_changes per reference: pinned never; container only unpinned terminal-inside;
  document all unpinned direct owners after D-15; every unselected unpinned reference forces
  needs-source, so mixed-owner artifacts copy/retain and move only when all source-dependent
  unpinned references are selected.`
  Blast radius: self-contained; supersedes the archive-time directory heuristic without
  touching the legacy command until Group B.

### Group B: Unified archive CLI and safe executor — covers REQ-03, REQ-04, REQ-06

- **Storage-owned artifact mutation primitive**  `type: task`  `satisfies: REQ-03`  `depends-on: —`
  Outcome: narrowly scoped filesystem primitives that stage bytes in recorded hidden
  owner-private locations adjacent to each destination,
  verify staged bytes against caller-supplied SHA-256+size before publishing new
  destinations atomically with no replacement and return an explicit success result only
  when this call created the destination (D-14), physically validate
  containment/no-symlink traversal, and quarantine/remove/restore sources through D-17.
  They neither acquire the operation-wide guard nor plan, relink/save issues, append events,
  or orchestrate rollback; those responsibilities belong to Coordinated plan execution.
  Testable through tempdir-backed storage (the in-memory backend performs no virtual file
  I/O, so filesystem assertions run against `JsonFileStorage` at a tempdir).
  Own criteria: `[hard] LOCAL-15: Rejects an occupied destination with the pre-existing
  file preserved byte-for-byte and, before introducing no-replace publication, amends the
  registry-first atomic-writes invariant plus rendered projection to explicitly cover
  staged atomic no-replace publication alongside temp-file + atomic-rename replacement.`
  `[hard] LOCAL-16: Exposes only filesystem stage, atomic no-replace publish,
  quarantine/remove/restore, and physical-containment operations using caller-supplied exact
  mappings; publish returns success only for a completed no-replace creation and a collision,
  including an identical external file, returns AlreadyExists rather than success; it performs no issue save,
  event append, planning, operation-wide locking, expected-identity recapture, or rollback
  orchestration.` `[hard] LOCAL-17: A destination created by an
  external writer between planning and finalization fails that artifact's finalization
  without overwriting the foreign file, verified by a race-focused test.` `[hard] LOCAL-18:
  Never unlinks a path directly: every removal runs the D-17 directory-handle-anchored
  sequence at the caller-supplied adjacent quarantine mapping using Started digest+size
  (same-filesystem rename in,
  openat-verify on the handle, unlinkat only on match; no-replace restore or
  quarantined-and-reported on mismatch), never recapturing expected identity, race-tested;
  platforms without the required directory-handle operations return quarantine-unavailable;
  recovery
  checks both recorded source and quarantine entry, including crash immediately after
  rename. The primitive itself never derives deletion authority from Started intent: command
  orchestration must supply a receipt-backed published-by-instance destination, which it
  opens/verifies against initial identity and holds through the quarantine decision;
  mismatch never removes source. Failure injection modifies a pre-existing identical
  destination and a receipted instance-published destination after Started; both retain source.`
  `[hard] LOCAL-46: Staging/quarantine dirs and entries are hidden, owner-private,
  adjacent to destination/source respectively, absent and symlink/component-safe before
  use; cleanup removes only a verified recorded entry and empty dir, and quarantine has no
  cross-device copy-delete fallback.`
  Blast radius: new storage API plus the authoritative `.jit/invariants.toml` entry and its
  rendered AGENTS.md projection; the legacy command keeps its inline `std::fs` path until it
  is removed.

- **Unified archive preview surface**  `type: task`  `satisfies: REQ-06`  `depends-on: Move/copy/retain/block classification`
  Outcome: `jit archive document <path>` and `jit archive container <id>` produce a
  complete non-mutating plan by default, running every precondition execution relies on
  (source availability, explicit complete policy, terminal eligibility, outside owners, pinned
  semantics, destination conflicts, before/after reachability), rendered as both the
  binding JSON envelope and human output, and verified against the
  Markdown/HTML/CSS/CSV/PNG/SVG fixture corpus.
  Own criteria: `[hard] LOCAL-19: Preview enumerates every artifact, action, evidence flag,
  and blocker and mutates nothing.` `[hard] LOCAL-20: Document and container targets
  produce the same plan schema from the shared planner.` `[hard] LOCAL-21: In a repository
  with absent or partial documentation policy, preview returns a non-mutating inventory
  distinguishing unconfigured from incomplete, explaining that archival is disabled, and
  execution is refused without substituting defaults.`
  Blast radius: adds a new `archive` command group alongside the existing `doc archive`;
  both coexist until the removal task. MCP tools regenerate from the CLI schema
  automatically.

- **Coordinated plan execution**  `type: task`  `satisfies: REQ-03, REQ-04`  `depends-on: Storage-owned artifact mutation primitive, Unified archive preview surface`
  Outcome: `--execute` acquires and holds `RepoWriteGuard`, ignores any prior preview,
  and before reading/applying current archive_root scans all open Started events across
  roots by recovery_target (full container UUID; document canonical original target root
  plus only that root's planned destination aliases, excluding dependencies). One match recovers with its recorded root and
  contract; multiple matches return archive-recovery-ambiguous; only no match after all
  prior target intents are terminal permits current policy to choose a fresh root/plan. It
  orchestrates the planner, storage filesystem
  primitives, issue relinks/saves, event appends, and rollback. Before any new mutation it
  either computes SHA-256+byte-size identities under the guard for a fresh operation or
  loads the original Started identities/mappings for recovery. Fresh execution mints the
  instance and derives absent, safe destination-adjacent staging and source-adjacent
  quarantine mappings; then it must append D-19 `ArchiveStarted` with exact mappings,
  identities, and applicable initial/remaining contract; an
  append failure aborts with no mutation. Initial staging must match Started identity before
  publish. Preexisting-identical destinations require revalidation before relinks and always
  retain source. After each successful no-replace publish, the command records its exact
  mapping in a durable non-reconciliation `ArchiveExecuted.completed_publications` receipt;
  append failure or crash leaves publication unproven, and source quarantine is forbidden.
  Only receipt-backed publications may authorize deletion; immediately before source
  quarantine, execution open-handle verifies that recorded destination against
  initial Started identity and holds the handle through the decision. Mismatch retains/
  restores source and safely aborts/blocks. It stages the complete set, validates every
  supported local edge at its proposed destination before metadata commit, persists
  exact computed reference_changes and completion events, refreshes or invalidates cached asset metadata
  on moved references, leaves pinned references untouched (D-3, D-26), refuses execution if
  canonical pinned resolution or any pinned root/dependency read fails, and removes sources
  only through recorded same-filesystem quarantine mappings and Started identity, checking
  both source and quarantine entry on recovery; no cross-device copy-delete. Only Done/Rejected
  containers execute; a document target executes only when every direct or embedded-closure
  owner of the document and of every bundle artifact is terminal, active owners block
  execution (preview still reports), and a zero-owner document in a managed path executes
  with an informational note (D-15). Recovery adopts the original operation/instance even
  when current state would produce a different source-set key, stays within the initial
  artifact/source/destination contract and Started content identities (never recapturing
  expected identity), re-evaluates current owners/references for safety,
  and defers new unrelated artifacts. It completes/reconciles when safe. If changed ownership
  invalidates initial deletion/relink intent, it restores/retains sources and references,
  proves referential safety, then appends terminal `ArchiveAborted` with reason and
  completed/reverted/residual paths. If automatic safe closure is impossible, it returns
  `archive-recovery-blocked`, leaves the intent open, and mints no competing instance.
  Event recording follows D-19: normal non-reconciliation `ArchiveExecuted`
  after successful finalization/relinks and before deletions (identity, plan fingerprint,
  exact `completed_publications`, completed mutations, pending deletions, reconciliation=false);
  `ArchiveSourcesRemoved` after removals, recording exactly the successfully removed paths
  and omitted when zero removals succeed (reconciliation checks both recorded source and
  quarantine entry before recording deletion). `ArchiveExecuted` remains recorded whenever
  finalization/relinks mutated even if every later deletion fails. A reconciliation event
  cannot add completed_publications or create publication provenance. Reconciliation is the
  explicit no-new-mutation exception; only a brand-new already-converged no-op with no open
  Started appends no events.
  Own criteria: `[hard] LOCAL-22: Validates each staged local edge in the proposed layout,
  not against source-resolved paths, before commit.` `[hard] LOCAL-23: Refuses
  document-target execution while any owner of the document or its bundle artifacts is
  non-terminal.` `[hard] LOCAL-41: Applies exactly reference_changes: pinned references are
  untouched; container execution relinks only unpinned terminal-inside references; document
  execution relinks every unpinned direct owner after D-15; mixed nonselected references
  remain source-resolvable.` `[hard] LOCAL-24: A partial-relink, event-append, deletion
  failure, crash after source→recorded-quarantine rename, or source modification after
  Started leaves no artifact lost: recovery checks source and recorded entry; mismatch is
  never deleted and is restored no-replace or left at that quarantine path/reported, with no destination
  overwritten and no reference relying solely on a missing path. If either an initially
  identical pre-existing destination or a receipted instance-published destination changes
  after Started, source is retained and the operation safely aborts/blocks. Failure injection
  also covers an external identical create after Started before JIT publish and a crash after
  JIT publish before its ArchiveExecuted receipt: neither has publication proof, neither has
  a pending deletion, selected relinks occur only after identity revalidation, and recovery
  closes safely with ArchiveAborted reason publication-unproven.` `[hard] LOCAL-25: Refuses execution on a non-terminal container; otherwise
  execute ignores preview state, discovers recovery under RepoWriteGuard, then recovers the
  recorded contract or computes exactly one fresh plan; no prior preview or expected
  fingerprint is an execution input.` `[hard] LOCAL-26: Before current archive_root is read/applied or a fresh
  key computed, recovery scans all opens by recovery_target: full container UUID, or document
  canonical original root/that root's own destination aliases only. Shared/embedded
  dependency paths never match. One match adopts its recorded root/contract; multiple
  matches return archive-recovery-ambiguous; no match permits fresh planning only after all
  prior target intents are terminal and uses the initial Started digest+size plus durable
  event history without recapture. Recovery grants published-by-instance only from an exact
  non-reconciliation completed_publications receipt and re-verifies that destination before
  deletion; Started mapping plus matching identity alone is publication-unproven and safely
  aborts with source retained. Tests cover source/destination edits, hierarchy/owner changes, archive_root change,
  document invocation by root destination alias, multiple matching opens, and two open
  documents sharing a dependency without false adoption/ambiguity.` `[hard] LOCAL-27:
  ArchiveStarted records explicit recovery_target, recorded destination root,
  operation_id, the minted/adopted operation_instance_id, fresh execution_id, fingerprint,
  exact destination intent, publication staging mappings, and `deletion_intents` containing
  `{source, quarantine_dir, quarantine_entry, content_identity}` mappings; initial staging
  matches identity and recovery reuses mappings/identity
  unchanged. It appends successfully before mutation but never proves publication. After
  storage returns no-replace success, normal ArchiveExecuted appends after relinks and before
  deletions with the identity triple, exact completed_publications, and pending deletions
  and remains present when all deletions fail; ArchiveSourcesRemoved
  appends after removals with exactly the successfully removed paths and is omitted when
  zero removals succeed; reconciliation cannot add completed_publications or invent provenance,
  and verifies a previously receipted destination before checking both source and quarantine
  entry. Tests modify pre-existing identical and receipted published destinations after
  Started, inject external identical creation before publish and crash after publish before
  receipt (source retained; publication-unproven abort), and crash immediately after
  quarantine rename and recover the entry.
  ArchiveAborted appends only after a referentially safe restored/retained
  state and records adopted operation/instance IDs, fresh execution ID, reason, and
  completed/reverted/residual paths; safe duplicates are
  allowed. An unclosable or ambiguous intent remains open and blocks. Only after completion/abort may a
  fresh operation mint; a brand-new already-converged no-op with no open intent appends no
  events. Event tests prove an archive_root change recovers with the Started recorded root,
  while multiple matching target roots emit no new event/mutation and shared dependency
  aliases do not match.`
  Blast radius: consumes the mutation primitive and preview planner; no legacy-command
  change.

- **Remove the legacy document-archive command**  `type: task`  `satisfies: —`  `depends-on: Coordinated plan execution`
  Outcome: `jit doc archive` is removed completely as a clean-cut migration with no alias
  or stub, and all consumers (CLI/main dispatch, `ArchiveResult`, event catalog/schema
  references, integration and unit tests, fixtures, docs, README, and the auto-generated
  MCP tool set) are updated in the same change so the tree builds and tests pass with only
  the unified surface. The legacy `[documentation.categories]` configuration table retires
  with it (D-21): its loader field, docs, and this repository's config entries go in the
  same change.
  Coverage: migration support only; it carries no container requirement and is exempt from
  `satisfies:` coverage rather than claiming credit already borne by the replacement tasks.
  Own criteria: `[hard] LOCAL-28: A tree-wide search for the legacy surface (the strings
  "doc archive", "archive_document", "ArchiveResult") across crates/, mcp-server/, web/,
  docs/, and dev/ returns no live production or test reference; only historical dev/
  records may mention it.`
  Blast radius: enumerated in the acceptance-check note below; `crates/server` and `web`
  have no live consumer (web matches are the `Archived` issue state, not the command).

### Group C: Container candidate reporting — covers REQ-05

- **Read-only container candidate report**  `type: task`  `satisfies: REQ-05`  `depends-on: Move/copy/retain/block classification`
  Outcome: `jit archive candidates` lists terminal containers using the shared planner —
  container-ness derives from the configured type hierarchy (any type at a non-leaf level
  of `[type_hierarchy]`, D-20), membership from the resolved hierarchy (D-25), never from
  hardcoded type names — each with documentation-policy status, managed/permanent path
  status, repository-wide artifact ownership, artifact counts,
  outside-subtree/active/permanent/unmanaged evidence plus unmanaged-selected-root,
  pinned-read/missing/unsupported/conflict blockers, and a
  move/copy/retain summary, in both human and JSON form, with no filesystem, issue, or
  event mutation and no age or retention filtering. Archive-root sources are reported as
  already-existing and never unmanaged: direct roots retain, relocated-parent relative
  dependencies copy with original retained, and other edges retain.
  Every candidate is evaluated fully:
  archival takes no category input (D-21), so the D-12 mirror rule determines each
  destination and no check is deferred or guessed.
  Own criteria: `[hard] LOCAL-29: Lists terminal containers with policy status, ownership,
  counts, and blockers, explaining exclusions rather than omitting them.` `[hard] LOCAL-30:
  Emits identical human and JSON results while mutating nothing, and applies no
  time/retention filter.` `[hard] LOCAL-31: Derives candidate container-ness from the
  configured type hierarchy's non-leaf levels and resolved-hierarchy membership, with no
  hardcoded type names.` `[hard] LOCAL-32: Evaluates every candidate fully — including
  destination conflicts under the mirror rule — with no category input, suggestion, or
  deferred check.` `[hard] LOCAL-35: JSON uses the candidate_count + candidates collection
  envelope and preserves configured/incomplete/unconfigured policy status per candidate.`
  `[hard] LOCAL-37: Candidate details distinguish unmanaged-selected-root blockers from
  unmanaged-path evidence on embedded dependencies and summarize the latter only as copy or
  retain, never move.` `[hard] LOCAL-42: Candidate details mark archive-root sources as
  already-existing: selected/direct roots retain at existing paths; relative-edge
  dependencies of relocated parents copy to the mirror with originals retained; other edges
  retain—including retired dev/archive/features/... layouts.`
  Blast radius: self-contained new read-only command consuming Group A.

**Coverage map** (single source for criterion→item; every `[hard]` criterion → ≥1 item):

| Criterion | Satisfied by (item) |
|---|---|
| REQ-01 | Artifact plan model and blocker taxonomy; Target root inventory and ownership; Recursive supported-dependency discovery |
| REQ-02 | Move/copy/retain/block classification |
| REQ-03 | Storage-owned artifact mutation primitive; Coordinated plan execution |
| REQ-04 | Recursive supported-dependency discovery; Move/copy/retain/block classification; Coordinated plan execution |
| REQ-05 | Read-only container candidate report |
| REQ-06 | Unified archive preview surface |

> **Removal acceptance check (D-9, clean cut, no alias).** The removal task "Remove the
> legacy document-archive command" is ordered last in Group B via `depends-on: Coordinated
> plan execution`, so both the old `jit doc archive` and the new `jit archive` family
> coexist through every intermediate wave and no wave breaks a consumer before its
> replacement exists. Acceptance is a tree-wide search across `crates/`, `mcp-server/`,
> `web/`, `docs/`, and `dev/` for `doc archive`, `archive_document`, and `ArchiveResult`,
> which must return no live production or test reference (historical `dev/` records
> excepted). The pre-plan sweep of that exact pattern reported **103 matches across 33
> files**; the live production Rust consumers are `crates/jit/src/{cli.rs, main.rs,
> commands/document.rs, commands/mod.rs, schema.rs, domain/event_catalog.rs}`; the test
> consumers are `crates/jit/tests/{doc_archive_tests.rs, archive_integrity_tests.rs,
> command_exit_code_projection_tests.rs}`; the rest are docs/ and dev/ prose. The MCP tool
> set is auto-generated from the CLI schema, so removing the subcommand drops it on
> regeneration. `crates/server` and `web` carry no live consumer.

## 4. Risks and actionability

| Risk / open question | Severity | Mitigation or decision |
|---|---|---|
| Generalized safe execution weakens the current referential-consistency guarantee | High | Reuse the proven sequencing/rollback (`document.rs:1391-1435`) and its failure-injection suite (`archive_integrity_tests.rs:203-396`); execution carries partial-failure criteria (LOCAL-24). Contract stays referential consistency, never atomicity. |
| Membership drawn from the wrong relation relocates another container's artifacts | High | D-25: membership is the resolved-hierarchy children closure (`hierarchy.rs:458`, `:321`), never the raw dependency closure; a sequencing edge contributes no member (LOCAL-04). |
| Symbolic or abbreviated pinned revisions make identity/fingerprints nondeterministic | High | D-26: the storage/git resolver canonicalizes every supplied revision to the hash-algorithm-agnostic full immutable commit OID; only that OID is serialized, ordered, and hashed. Resolution failure, `CommitNotFound`, and commit-path `NotFound` block with no fallback (LOCAL-03, LOCAL-33, LOCAL-38). |
| A partial documentation table silently authorizes default policy | High | D-1: only explicit managed_paths, permanent_paths, and archive_root produce `configured`; absent and incomplete policy remain distinct, previewable, and non-executable (LOCAL-34, LOCAL-21). |
| Archive-root dependency is moved/deleted or a relocated relative edge loses its target | High | `archived-source` always supplies needs-source and bypasses unmanaged blocking. Direct roots retain; a relative-edge dependency of a relocated parent copies to the current mirror; root-relative/staying edges retain. Original `dev/archive/features/...` or other archive-root paths never move/delete (LOCAL-11, LOCAL-39, LOCAL-42). |
| Recursive discovery moves an embedded dependency outside delegated managed paths | High | Managed status is computed for every working-tree artifact. An unmanaged selected root blocks as `unmanaged-selected-root`; an unmanaged embedded dependency carries `unmanaged-path`, always needs-source, and therefore copies when its relocated parent needs a destination or otherwise retains—never moves (D-1, D-2, D-16, LOCAL-36). |
| Mixed owners cause the wrong issue references to relink or lose their source | High | Per-reference selection precedes D-16: pinned never relink; container selects only unpinned terminal-inside refs; document selects all unpinned direct refs after D-15; every unselected unpinned ref forces needs-source. `reference_changes` is exactly that set (LOCAL-40, LOCAL-41). |
| A retained dependency leaves a relocated parent's relative link without its target | High | D-16's calculus is total: a relative edge from a relocated parent forces needs-destination; root-relative edges force source retention; the classifier never emits an unpreservable layout (LOCAL-11) and execution re-validates every edge (LOCAL-22). |
| Retry deletes a source edited after Started by recapturing its new identity | High | Under the guard, fresh execution records SHA-256+byte-size per publication/deletion in Started. Staging and every retry/quarantine compare against that durable identity, never recapture it; mismatch is restored/quarantined and forces safe abort/open blocker (LOCAL-18/24/26/27, D-17–D-19). |
| Pre-existing or receipted instance-published destination changes after Started, then source is deleted | High | Preexisting-identical always needs-source and never enters pending_deletions. Instance-published deletion requires an exact durable non-reconciliation `ArchiveExecuted.completed_publications` receipt plus open-handle destination verification against initial identity held through quarantine decision; mismatch retains source and aborts/blocks. Both races are injected (LOCAL-14/18/24/26/27). |
| Started publication intent is mistaken for proof after an external identical create or crash between publish and receipt | High | Storage success proves only the live call; the command durably records its exact mapping in non-reconciliation ArchiveExecuted before any deletion. Without that receipt, recovery classifies the matching destination as publication-unproven/preexisting-identical, retains source, revalidates before selected relinks, and safely aborts `publication-unproven`; reconciliation cannot manufacture the receipt. Both timing windows are injected (LOCAL-14/16/24/26/27, D-14/D-18/D-19). |
| Crash after source→quarantine rename strands content or falsely marks deletion complete | High | Started records unique adjacent same-filesystem quarantine mapping per deletion. Recovery checks both source and entry; match may unlink, mismatch restores/remains reported; empty-dir cleanup cannot erase nonempty/unverified entries. Crash-after-rename is injected (LOCAL-18/24/27). |
| Nested mounts make repo-local staging/quarantine cross-device | High | Staging is hidden/adjacent to each destination; quarantine hidden/adjacent to each source. Paths are absent/private/symlink-safe. Quarantine-unavailable leaves deletion pending; no copy-delete fallback (LOCAL-46, D-14/D-17). |
| Removing `jit doc archive` breaks docs/tests/MCP mid-flight | Medium | Removal ordered last (`depends-on: Coordinated plan execution`); old and new coexist until then; consumer migration in the same change; acceptance is the tree-wide search above (LOCAL-28). |
| Recursive CSS/HTML discovery mis-scopes a shared theme or figure | Medium | Repository-wide reference-count classification over the D-13 universe (LOCAL-09, LOCAL-10), not directory heuristics; shared-but-active artifacts copy/retain, never force-relink (D-2, D-10). |
| An artifact embedded by an outside document misclassified as unshared and moved | High | D-13's universe includes the supported embedded closure of every issue-linked document; the supplied-map limitation of the existing classifier (`assets.rs:189`) is superseded (LOCAL-09). |
| Post-archive verification defect masks broken links | Medium | Executor validates each edge in the proposed layout, not against source-resolved paths (LOCAL-22); replaces `document.rs:1832-1858`. |
| Storage primitives and command orchestration overlap or leave a coordination gap | High | Storage only stages/publishes/quarantines/restores and checks containment (LOCAL-16); command holds `RepoWriteGuard` across recovery discovery, recorded-contract recovery or one fresh plan, events, filesystem calls, relinks/saves, and rollback (LOCAL-25/27, `storage/mod.rs:126`). |
| archive_root/config changes strand intent, or shared dependency adopts wrong document operation | High | recovery_target is full container UUID or only document canonical target root + its own destination aliases; dependency paths are excluded. Cross-root scan adopts one, blocks multiple true target matches, and tests shared-dependency nonmatches (LOCAL-26/27, D-18/D-19). |
| Archive events misreport completion, abort, or residual state | Medium | D-19's four-event family keeps Started open until cumulative completion or a safe terminal ArchiveAborted. Abort records reason and completed/reverted/residual paths only after referential safety is proven; safe duplicates are permitted by append-only recovery truth (LOCAL-27, `@/inv/event-log`). |
| No-replace publication diverges from the current atomic-writes invariant wording | Medium | The storage task first amends the registry-first invariant and renders its projection to cover staged atomic no-replace publication while retaining temp-file + atomic-rename for replacement writes; implementation and tests then target the amended contract (LOCAL-15). |
| Document-target execution on a path with mixed-state owners dangles an active reference | Medium | D-15: all direct and embedded-closure owners must be terminal; active owners block execution while preview still reports; zero-owner managed-path documents execute with an informational note (LOCAL-23). |
| Arbitrary zero-owner or opaque document target has no shared-planner root | High | Target root inventory normalizes every document path as an explicit working-tree root, looks up all owners/versions, and needs no adapter for CSV/PNG/SVG; policy and zero-owner semantics remain later classification/execution concerns (LOCAL-43/44, `storage/mod.rs:498`). |

Every load-bearing question is resolved or owned. Each sketch item is executable from its
description plus the cited grounding without re-deriving the design.

## Decisions

Carried from the owner-approved brief (`dev/active/7d3a3a47-planning-brief.md`), binding,
including its 2026-07-13 owner amendment. Plan-level decisions D-12 onward resolve
specification gaps surfaced during planning and review; none is REOPEN.

- **D-1 — Archival is explicit opt-in:** chosen **`configured` requires an explicit
  `[documentation]` table with explicit `managed_paths`, `permanent_paths`, and
  `archive_root`; a missing table is `unconfigured`, a partial table is `incomplete`, and
  neither may execute**. Accessor defaults may aid preview display but never delegate
  mutation authority. Managed-path authority is checked per working-tree artifact: an
  archive-root-contained path first gains archived-source/needs-source and bypasses
  unmanaged blocking (direct roots retain; relocated-parent relative dependencies copy with
  original retained); otherwise an unmanaged
  selected root blocks, while an unmanaged embedded dependency may be copied to
  satisfy a relocated edge only if its original remains untouched (D-2, D-16). Rejected:
  moving any unmanaged artifact; treating table presence or defaulted accessors as
  configured (moves evidence never delegated to JIT).
- **D-2 — Do not relocate another container's artifacts:** chosen **never relocate an
  artifact referenced outside the subtree; for container archival relink only unpinned
  references on terminal issues inside the resolved subtree, leave all other references at
  source, and copy if the selected bundle also needs a destination**. Rejected:
  global relink; a general force option.
- **D-3 — Pinned references remain historical:** chosen **never rewrite a pinned reference;
  keep its historical source path resolvable, copy content into the bundle when needed,
  and block execution when any commit-specific root/dependency read fails**.
  Rejected: path-only relink; implicit conversion to working-tree provenance. Refined by
  D-26.
- **D-4 — One container-owned destination (amended by owner, 2026-07-13):** chosen
  **destination is a container-owned directory preserving internal topology; the brief's
  category-selection clause is superseded — archival takes no category input (D-21)**.
  Rejected: category-per-artifact placement (fragments bundles); caller-selected categories
  (legacy taxonomy scrapped by the owner in favor of the pure mirror rule).
- **D-5 — Initial recursive discovery is static:** chosen **follow Markdown/HTML/CSS local
  references incl CSS `url()`/`@import`; block on detected local JavaScript/runtime
  loading**. Rejected: guessing dynamic dependencies. Made precise by D-23.
- **D-6 — Time-based archival deferred:** chosen **no retention periods, age filters,
  scheduled sweeps, or lifecycle triggers; candidates use current terminal state and
  policy**. Rejected: first-done, updated, or reconstructed timestamps. (Live REQ-05
  already amended.)
- **D-7 — Preview defaults, execution explicit:** chosen **complete non-mutating plan by
  default; mutation requires `--execute`, which ignores preview output, acquires the write
  guard, performs stable-identity recovery discovery before current policy, then either
  recovers the recorded contract or recomputes exactly one fresh current plan**. The fingerprint remains
  informational; no expected-fingerprint input or comparison protocol exists. Rejected:
  applying preview data as an execution plan.
- **D-8 — Only terminal containers execute:** chosen **Done/Rejected may execute;
  non-terminal may preview only, no force override**. Rejected: force execution of active
  containers.
- **D-9 — One unified archive command family (amended by owner, 2026-07-13):** chosen
  **`jit archive document|container|candidates` on one planner/schema/policy/coordination/
  executor, with no category flag; remove `jit doc archive` completely as a clean-cut
  pre-v1.0 migration**. Rejected: retaining an alias or migration stub.
- **D-10 — Reachability defines a bundle:** chosen **explicit issue-linked artifacts plus
  recursively reachable supported static dependencies; unreferenced siblings reported as
  informational, not moved**. Rejected: sweeping an entire source directory implicitly.
- **D-11 — Candidates are container-oriented:** chosen **list terminal containers with
  policy status, artifact counts, blockers, and summaries; individual artifacts are details
  inside a candidate**. Rejected: individual artifacts as independent archival candidates.
- **D-12 — Destination layout mirrors repository-relative paths:** chosen **destination
  root `<archive_root>/` (+ `<container.id>/`, the full UUID, for container targets), each
  artifact at root + repository-relative source path, with no category segment (D-21)**.
  The full UUID is the container segment because short-id prefixes are not unique across
  containers, and destination determinism must not depend on the current id population.
  Preserves every relative offset under a common prefix, keeps identical filenames
  distinct, makes intra-plan collisions structurally impossible, and is invertible (used by
  D-18). Rejected: stripping the managed prefix per artifact (the current single-doc rule,
  `document.rs:1284-1319`) — artifacts from different managed roots could collide and
  cross-directory relative links would break; content-addressed layout — destroys
  human-navigable topology; short-id segments — prefix collisions.
  **Archive-root precedence is edge-aware:** containment supplies archived-source,
  already-existing, and needs-source while bypassing unmanaged-selected-root. A direct root
  retains at its exact path with no relink; an embedded target of a relocated relative edge
  copies to this target's mirror while the archived original remains; root-relative/staying
  edges retain. Archive-root originals are never moved or deleted.
- **D-13 — Ownership universe is the issue-linked closure:** chosen **all
  `DocumentReference`s across all issues plus each one's recursive supported embedded
  closure; outside owners (direct or embedded) forbid a move**. Files unreachable from any
  issue reference are outside JIT's referential contract: reported informationally near
  bundles (D-10), never counted as owners, never relocated. Rejected: scanning the entire
  working tree for arbitrary referencing files — unbounded, and JIT's guarantee is scoped
  to issue references.
- **D-14 — Finalization is atomic no-replace:** chosen **hard-link from same-filesystem
  destination-adjacent staging then unlink staging; the storage call returns success only
  when it created the destination, while `AlreadyExists` (including identical content)
  aborts that artifact's finalization and triggers source-retaining recovery. Before Started,
  derive/persist a unique hidden
  owner-private staging dir/entry adjacent to each destination from instance ID + SHA-256 of
  canonical destination path; require absent, component/symlink-safe paths. Create after Started,
  verify staged bytes against Started identity, publish no-replace on the same destination
  filesystem, return success, durably append the command-owned non-reconciliation receipt,
  then best-effort clean only the recorded verified entry and empty dir. This
  never assumes repository root and a nested destination share a device. Because
  `@/inv/atomic-writes` currently names only temp-file + atomic
  rename, the same storage task must first amend the registry-first invariant (and render
  its projection) to define staged atomic no-replace publication for new files while
  retaining temp-file + atomic rename for replacing writes; compliance is claimed only
  against that amended contract. A content-identical destination that predates Started is
  preexisting-identical, not publication: it needs no write but always retains source and
  cannot enter pending_deletions. Started publication_mapping is intent only. After storage
  returns successful no-replace publication, the command must durably append that exact
  mapping in non-reconciliation `ArchiveExecuted.completed_publications`; only this receipt
  can become published-by-instance, and cleanup/deletion cannot precede its successful
  append**. Rejected: silently treating hard-link publication as
  covered by the current literal wording; treating an identical pre-existing destination as
  this instance's publication (could delete source after destination mutation);
  repository-global staging (nested destination may
  be another device); check-then-`rename` — `rename` replaces a destination
  created after the check, and the repository write guard cannot exclude external
  filesystem writers (`storage/mod.rs:90`).
- **D-15 — Document-target execution requires all-terminal owners:** chosen **`jit archive
  document --execute` requires every direct or embedded-closure owner of the document and
  its bundle artifacts to be terminal; active owners block with
  `document-non-terminal-owner` (preview still reports); a
  zero-owner managed-path document executes with an informational note. After that check,
  every unpinned direct owner is selected for relink; pinned owners never are, and embedded
  edges have no issue metadata. Target inventory is path-first, so a zero-owner opaque
  document still exists as an explicit working-tree root before these later checks**. Rejected:
  selecting an owning container implicitly (ambiguous with several owners); a force
  override (mirrors the rejected D-8 force path).
- **D-16 — Edge-aware action calculus:** chosen **derive each action from needs-destination
  (selected root except archived-source direct roots, or relative edge from a relocated
  parent) × needs-source (outside owner,
  active owner, permanent path, preexisting-identical destination, unmanaged embedded
  dependency, archived-source, or inbound
  root-relative/staying-document edge, or any unpinned reference not selected for relink):
  move / copy / retain, with conflicts and
  unpreservable edges blocking**. Copy creates the mirrored destination and retains the
  source, satisfying both constraints, so every supported edge — relative at the mirror,
  root-relative at the repository root — resolves in the final layout. Rejected: a free
  copy-or-retain choice (leaves a relocated parent's relative edge without its target);
  treating root-relative edges like relative ones (their resolution ignores the referencing
  document's location, `assets.rs:151-162`). A selected root outside all managed paths is
  excluded before the calculus by `unmanaged-selected-root`; an unmanaged embedded
  dependency carries `unmanaged-path`, always needs-source, and therefore copies when
  needs-destination holds or retains otherwise, never moves. Permanent-path, outside-owner,
  active-owner, unmanaged-path, and archived-source are evidence flags, not blockers; for document execution
  D-15 separately maps any non-terminal owner to `document-non-terminal-owner`. A supported edge whose combined
  constraints cannot be materialized emits `unpreservable-layout` with that edge.
  Archive-root recognition precedes unmanaged/reference classification but remains in the
  edge calculus: direct roots retain; a relative-edge dependency of a relocated parent
  copies to its mirror; root-relative/staying edges retain; none move/delete. Otherwise
  per-reference selection proceeds: pinned never relink; container selects only
  unpinned terminal-inside references; document selects all unpinned direct owners after
  D-15. `reference_changes` is exactly this set, and move is allowed only when every
  source-dependent unpinned reference is selected. A preexisting-identical destination
  satisfies needs-destination but always adds needs-source and cannot schedule deletion;
  only published-by-instance backed by an exact durable non-reconciliation
  `ArchiveExecuted.completed_publications` receipt can.
- **D-17 — Every removal is a directory-handle-anchored quarantine sequence:** chosen
  **every publication/deletion source has a SHA-256+byte-size identity recorded in the
  initial Started event before mutation (reusing `document/assets.rs:269-276` SHA-256).
  Before Started, each deletion gets an exact `{source, quarantine_dir, quarantine_entry,
  content_identity}` mapping: hidden owner-private quarantine adjacent to the source, unique
  from operation_instance_id + SHA-256 of canonical source path; dir/entry must be absent and
  component/symlink-safe, with POSIX `0700`/platform-equivalent directory access. Deletion
  requires this instance's exact non-reconciliation `ArchiveExecuted.completed_publications`
  receipt; a Started mapping alone is insufficient. Immediately before quarantine,
  open/verify its recorded destination against initial Started identity and hold that handle
  through the quarantine decision. Mismatch restores/retains source and aborts/blocks. No path
  is unlinked directly. After Started, a removal (1) creates
  and opens the recorded quarantine directory, (2) atomically same-filesystem renames source
  to the recorded entry, (3) opens the entry **relative to
  that directory handle** via the safe `openat` wrapper in `nix` — already a direct
  dependency of `crates/jit` (`Cargo.toml:41`); this adds the `fs` feature, not a new
  crate, and keeps `#![deny(unsafe_code)]` intact because the unsafety lives inside the
  dependency — (4) verifies the open handle against the initial Started digest+size—never a
  recaptured current-source identity—and (5) unlinks the entry by name **relative to the same directory handle**
  (`unlinkat`) on match — or restores it no-replace on mismatch, leaving it quarantined
  with a `quarantined-foreign-file` warning if restore is impossible. A mismatch is never
  deleted and forces safe abort or leaves recovery open. Recovery checks both recorded
  source and entry before declaring deletion complete, so crash after rename is recoverable.
  Empty-dir cleanup is best-effort and never erases a nonempty/unverified entry.
  `quarantine-unavailable` leaves deletion pending; cross-device copy-delete is forbidden.** The claim is
  stated exactly: `unlinkat` acts on the *name*, whose binding to the verified inode can
  change between steps 4 and 5 only through a write inside the just-created, unpredictably
  named, private quarantine directory. That residual race is **bounded, not denied**: the
  capability it requires already suffices to delete any repository file directly, so JIT's
  removal adds no destructive power the interferer lacks (D-24). Rename preserves whatever
  file is present at capture time, so nothing is overwritten at any step. Applies to ordinary move-source deletions, residue cleanup, and rollback of
  finalized destinations alike; platforms without required directory-handle semantics return
  `quarantine-unavailable` and leave deletion pending. Rejected:
  recapturing expected identity from a retry's current source (could authorize deletion of a
  later edit);
  fixed `.jit/tmp` quarantine or cross-device copy-delete (not atomic/recoverable);
  verify-then-unlink at the original path (a substitution between verification and unlink
  destroys the substitute); unverified cleanup (deletes foreign files); global filesystem
  locking (unavailable against arbitrary external writers).
- **D-18 — Retry converges by recomputation:** chosen **a rerun after any partial failure
  first scans all open Started intents across roots by stable target identity, before current
  archive_root is read/applied. `recovery_target` is container full UUID or document
  canonical original target root + only that root's planned destination aliases; shared/
  embedded dependency paths are excluded. One match adopts its recorded
  root plus original operation/instance and initial
  artifact/source/destination contract, re-evaluates current owners/references, and defers
  new unrelated artifacts; multiple matches block as archive-recovery-ambiguous**. It may complete/reconcile the original contract, or restore/
  retain sources and references then safely abort it; if neither is automatically safe the
  intent remains open and blocks. Only after all prior target intents complete/abort may
  current config choose a fresh root and mint. Archive-root evidence and
  preexisting-identical destinations prevent nesting but always retain source. The initial
  Started contract recovers intent, original paths, adjacent staging/quarantine mappings,
  and expected digest+size, but never distinguishes actual publication. Only an exact durable
  non-reconciliation ArchiveExecuted completed_publications receipt does so. Without it, a
  matching destination is publication-unproven/preexisting-identical: retry retains source,
  removes any effective pending deletion, revalidates identity before selected relinks, and
  appends safe ArchiveAborted reason publication-unproven. It never synthesizes provenance
  through reconciliation. Retry verifies each receipted destination before any deletion,
  checks both source/quarantine entry, and never recaptures identity. Rejected: a separate
  mutable resumable journal (the append-only Started event is the durable intent); treating
  every occupied destination as a blocker (would make every partial failure permanent).
- **D-19 — Durable intent, three-level identity, and four event kinds:** chosen
  **`operation_id` = hash of
  (target, destination root, sorted canonical initial source-path set; archive-root
  already-archived artifacts contribute their existing paths): the definition key.
  `operation_instance_id` delimits one logical operation by durable adopt-or-mint. Before
  current archive-root policy or a fresh key is read/computed, history is searched across
  roots by recovery_target: full container UUID, or document canonical original root plus
  only its own planned destination aliases; dependency paths never match. One open match adopts its recorded root/IDs/contract;
  multiple matches return archive-recovery-ambiguous and mint nothing. Only with all prior
  target intents terminal may current config choose a fresh root. Before any mutation, a
  new operation computes SHA-256+byte-size under the command-held guard and mints the
  instance UUID. It derives absent, safe hidden adjacent staging/quarantine paths from that
  ID plus SHA-256 canonical-path hashes and must append `ArchiveStarted` with recovery_target,
  recorded root, operation/instance/execution IDs, fingerprint, exact destination intent and
  preexisting classification,
  staging mappings, and `deletion_intents` with exact
  `{source, quarantine_dir, quarantine_entry, content_identity}` mappings. Append
  failure means no mutation. Preexisting-identical records have no publication_mapping or
  pending source deletion and always needs-source. Started publication mappings are intent
  only. Initial destination-adjacent staging verifies bytes before publish; storage must
  report successful no-replace creation, after which the command durably appends a normal,
  non-reconciliation `ArchiveExecuted` with exact `completed_publications`. No source deletion
  begins before that append succeeds. A recovery
  attempt is constrained to that initial set, rechecks current ownership/reference safety,
  reuses initial Started mappings/identities without recapture. Only its exact durable
  non-reconciliation completed_publications receipt may justify deletion; immediately
  beforehand recovery open-handle verifies that
  destination against Started identity and holds the handle through the quarantine decision,
  then checks source and quarantine entry. It defers unrelated new artifacts and appends Started with adopted IDs plus remaining work
  before new mutation. Normal `ArchiveExecuted` appends after finalization/relinks and before
  deletions, recording exact completed_publications, completed mutations, and pending
  deletions; normal
  `ArchiveSourcesRemoved` records exactly removals successful in that attempt and is omitted
  when zero succeed, while reconciliation SourcesRemoved records deletion only after both
  recorded locations establish completion. Reconciliation ArchiveExecuted may close other
  already-proven converged work but cannot contain completed_publications or establish
  publication provenance. Completion events carry a reconciliation marker. The instance remains open
  until cumulative completion events cover the initial Started event's full
  publication/relink/deletion sets; later Started attempts record remaining work without
  redefining that contract, and failed deletions remain pending. `ArchiveAborted` is the
  fourth terminal event and appends only after sources/references are restored or retained
  into a proven referentially safe state; it records adopted operation/instance IDs, fresh
  execution ID, reason, completed, reverted, and safe residual paths. Safe duplicate terminal events are allowed. If safe automatic closure
  is impossible, return archive-recovery-blocked and leave the intent open—never mint a
  competing instance. If a matching destination lacks the qualifying receipt—whether an
  external writer created identical bytes after Started before JIT publish, or JIT published
  then crashed before receipt—recovery retains source, revalidates before selected relinks,
  and closes with ArchiveAborted reason publication-unproven; it never appends reconciliation
  ArchiveExecuted as a substitute receipt. If a crash leaves an open Started whose other,
  already-proven work is converged, retry may append missing reconciliation
  `ArchiveExecuted` and/or `ArchiveSourcesRemoved` without new mutation to close that work.
  A brand-new already-converged
  invocation with no open Started appends no events. Only a completed or aborted old intent
  permits a fresh current-set operation.** A crash can
  leave a duplicate completion event or an unrecorded removal of an already-safe path —
  both within the established referential-consistency contract (`document.rs:1050-1074`).
  Rejected: the plan fingerprint as retry identity (recomputation changes it); literal
  exactly-once completion semantics (unachievable across a crash between mutation and its
  append-only completion event); event-only post-mutation identity (loses a minted UUID when
  append fails); same-key-only lookup (strands intent when owners/hierarchy change);
  treating embedded/shared dependency paths as document recovery aliases (false adoption);
  per-artifact events without an operation id (cannot audit a
  multi-artifact action).
- **D-20 — Container-ness derives from the type hierarchy:** chosen **a candidate container
  is a terminal issue whose configured type sits at a non-leaf level of
  `[type_hierarchy]`**. Engine code names no type; the boundary comes from repository
  configuration (`@/inv/domain-agnostic`). Rejected: hardcoding strategic type names
  (epic/milestone); treating every issue with DAG children as a container (an incidental
  dependency fan-in is not a container).
- **D-21 — Document categorization is scrapped (owner, 2026-07-13):** chosen **archival
  takes no category input anywhere in the command family; the D-12 mirror rule alone
  determines destinations, candidates always evaluate fully, and the legacy
  `[documentation.categories]` table retires with the legacy command**. The category
  taxonomy was a legacy idea; the mirror rule already yields unambiguous, human-navigable,
  collision-free destinations without it. Rejected: caller-selected categories (an extra
  input that adds a failure mode and no information the path does not already carry);
  doc_type-derived category inference (no doc_type→category mapping exists in the
  configuration model — `doc_type` is a free-form optional string, `types.rs:885` — so any
  inference would be guessing).
- **D-22 — Symlinks block in v1:** chosen **a symlink root or embedded target, or a path
  traversing one, classifies `block` with `symlink-artifact`; the mutation primitive
  resolves paths physically, verifies containment on resolved targets, and never moves,
  copies, or deletes through a link**. Rejected: transparent symlink following (relocation
  through a link silently changes what other referents resolve to); rewriting links
  (out-of-scope link mutation). Richer symlink relocation is a follow-up.
- **D-23 — Dynamic-loading detection is an enumerated textual contract:** chosen
  **`<script src>` is a static supported edge; a relocated HTML or script member containing
  a local-path-bearing construct from the binding set — `fetch(`, `import(`,
  `XMLHttpRequest`, `new Worker(`, `importScripts(`, URL-bearing `data-*` attributes, or
  static module syntax with a relative specifier (`import`/`export … from './…'`,
  `require('./…')`) — blocks with `unsupported-dynamic-edge`; detection is pattern match
  only, and the set is append-only under `schema_version`**. JIT has no JavaScript adapter,
  so a script declaring local dependencies never relocates silently. Rejected: executing or
  parsing JavaScript to resolve its dependencies (guessing, D-5); ignoring script content
  (silently breaks relocated bundles).
- **D-24 — The safety bound is stated, provable, and maximal for the platform:** chosen
  **two claims, each verifiable. (1) JIT never overwrites: every finalization and restore
  is no-replace (D-14, D-17). (2) Source deletion requires an exact durable
  non-reconciliation `ArchiveExecuted.completed_publications` receipt for successful
  no-replace creation; Started intent or reconciliation never suffices. JIT open-handle
  verifies the receipted destination against initial identity and
  holds that handle through the quarantine decision, then unlinks a source name only after verifying the inode bound
  to it inside the directory-handle-anchored quarantine sequence (D-17); rebinding that
  name between verification and unlink requires write access to the quarantine directory
  JIT just created — a capability that already suffices to delete any repository file
  without JIT's involvement, so JIT's removals add no destructive power an interfering
  writer does not independently possess. A destination pathname can still be rebound after
  its handle is verified while the verified inode remains held; defeating path resolution
  requires the same external write capability. Both destination path-rebinding and
  quarantine name-rebinding residual races are bounded by this capability argument, not
  denied.** This is the
  strongest no-loss statement a path-based filesystem admits (POSIX has no unlink-by-handle),
  and it is how REQ-03 is satisfied: benign concurrent modification is detected and
  preserved (quarantine/restore/report), and adversarial loss is attributable only to the
  adversary's own pre-existing access, never to JIT's operations. Concurrent JIT writers
  are excluded outright by the repository write guard. Rejected: claiming unconditional
  no-loss under adversarial substitution (unprovable — and vacuous, since such an adversary
  can delete files directly); claiming adversarial liveness (an interferer can always force
  a failure or reported residue; only safety is guaranteed); global filesystem locking
  (unavailable against arbitrary external writers).
- **D-25 — Container membership is the resolved-hierarchy subtree:** chosen **the archival
  membership of a container is the root plus the transitive `children` closure of the
  repository-wide hierarchy resolution (`hierarchy.rs:458`, `:321`)** — the same
  DAG-authoritative relation the tree and divergence surfaces use. Rejected: the raw
  transitive dependency closure (absorbs cross-container sequencing edges and would
  relocate other containers' artifacts, violating D-2); label-based membership (labels are
  hints; the DAG is authoritative).
- **D-26 — Artifact identity is (path, version); pinned versions are historical:** chosen
  **artifacts are keyed by path plus version (`working-tree` or the canonical full immutable
  commit object ID). Every supplied symbolic, abbreviated, or full revision is first
  resolved by a storage/git primitive with `rev-parse --verify <rev>^{commit}` semantics to
  the repository's hash-algorithm-agnostic full OID. Only that canonical OID is serialized,
  ordered, fingerprinted, or compared; the supplied string and current seven-character
  display hash never participate in identity. Working-tree versions are the archival
  subjects; pinned versions resolve through the storage layer's
  commit-aware reads (`document.rs:349-357`, `storage/mod.rs:531`) from git history, are
  never relocated or rewritten, appear in plans as informational `pinned-historical`
  entries, and impose no working-tree retention constraint while their commit is
  reachable and canonicalization plus every commit-specific read succeeds. Failed revision
  resolution, unavailable git, or an unreachable commit surfaces `CommitNotFound`; where a root or supported dependency path is absent at
  a reachable commit, storage surfaces `NotFound`. Either produces `pinned-read-failed` and
  blocks execution of the target. No fallback, warning-plus-retention, or relink is allowed;
  D-3 remains unchanged. Pinned roots' **commit-specific
  supported dependency closures are discovered**: the adapters are pure text extractors
  run over commit-resolved content (`read_path_text(path, canonical_oid)`, `storage/mod.rs:531`),
  yielding informational, non-relocating (path, canonical full OID) entries (LOCAL-33), so REQ-01's
  enumeration is complete for pinned roots with nothing to move — history serves every
  pinned reader.** Rejected: path-only identity (cannot represent two versions of one
  path); symbolic input or abbreviated display hash as identity (mutable or
  collision-prone); treating every pinned reference as a working-tree retention constraint
  (needlessly blocks archival of files whose history serves all pinned readers); rewriting
  pinned references (D-3); inventing a read fallback for unreachable commits (storage has
  none; planning must not assume semantics the code does not implement).
- **Assumptions:** Coverage is enforced at the task tier: each requirement-bearing task is a
  direct child of the epic and carries its own `satisfies: REQ-*` label. The legacy-command
  migration support task is intentionally exempt and credits no requirement; the A/B/C
  group headers are conceptual only. This assumes a single breakdown pass produces
  executable leaves rather than an intermediate story tier; risk if wrong is a
  coverage-gate reshuffle, not a design change.
