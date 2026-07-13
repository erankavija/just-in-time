# Plan: Dependency-aware container artifact archival (7d3a3a47)

> Planning node: 1dbc72f6. Container criteria source: 7d3a3a47 `## Success Criteria`.

This plan builds a pure artifact-plan domain capability and a unified `jit archive`
command family that consumes it. It grounds every claim in
`dev/active/7d3a3a47-investigation.md` (citations re-verified against live code) and
carries the owner-approved decisions from `dev/active/7d3a3a47-planning-brief.md` (D-1
through D-11 plus both 2026-07-13 owner amendments: document categorization is scrapped,
and the concurrency/crash contract is scoped for proportionality) as binding.

**The safety contract (binding, from the epic and the proportionality amendment):**
referential consistency, not atomicity, across filesystem files, issue JSON, and the
append-only event log. Guarantees hold under the repository write guard against concurrent
JIT writers, and benign concurrent modification is detected before destructive steps
(no-replace publication, verify-recorded-hash-then-delete). Concurrent external mutation of
the working tree during an archival operation is out of contract; git history is the
recovery channel in versioned repositories. This deliberately excludes write-ahead intent
events, publication receipts, destination-provenance tracking, quarantine protocols, and
crash-recovery state machines — the operation is short, occasional, recomputable, and its
subjects are git-recoverable; rerun convergence by recomputation is the recovery story.

## 1. Completeness vs criteria

Narrative of the approach per `[hard]` criterion. The criterion→item contract lives once,
in the §3 coverage map.

| Criterion | Approach (how it is met) | Notes / open gap |
|---|---|---|
| REQ-01: deterministic plan of every issue-linked artifact in a container subtree plus every supported embedded local dependency of its working-tree artifacts | Resolve the container root through normal storage id semantics, enumerate the root plus its **resolved-hierarchy descendants** (the DAG-authoritative membership relation, `graph/hierarchy.rs:458`, `:321` — not the raw dependency closure, which absorbs cross-container sequencing edges; D-25), collect every distinct `DocumentReference` on those issues as **(path, version)** artifacts (D-26) including opaque/binary roots, canonicalize every supplied pinned revision to its full immutable commit object ID, then recursively discover supported local dependencies of **working-tree** artifacts (Markdown/HTML element URLs and CSS `url()`/`@import`) with cycle detection and deterministic ordering. Output is one stable, fully enumerated plan object with destinations computed by the D-12 mirror rule. | Opaque roots (CSV/PNG/SVG) are inventoried without an adapter; parsing support gates only embedded-edge discovery, not root eligibility. Pinned versions are enumerated as non-relocating historical entries; per the amended REQ-01 their commit-resolved closures are outside relocation scope and not enumerated (D-26). |
| REQ-02: classify each artifact move/copy/retain/blocked using active references, sharing, managed-path policy, destination conflicts | A pure classifier computes an action per working-tree artifact through the D-16 edge-aware calculus from all reference owners (repository-wide, direct and embedded; D-13), per-reference selection, current lifecycle state, component-aware managed/permanent policy with explicit three-state completeness, archive-root precedence, destination occupancy, and pinned semantics (D-26). Ownership and policy facts are evidence feeding copy/retain; blockers are structural (conflicts, unpreservable edges, symlinks, missing sources, failed pinned reads, ineligible targets). | Replaces the two inconsistent directory-heuristic notions of "shared" (`document.rs:1463-1495`, `assets.rs:185-233`) with one repository-wide reference analysis. |
| REQ-03: archive an eligible container without losing files, overwriting destinations, or leaving dangling issue references | Execution recomputes the plan under the held repository write guard, stages and verifies content, validates every supported local edge at its proposed destination, publishes no-replace (at the primitive layer a collision — even content-identical — is `AlreadyExists`, never a claimed creation; the command layer then converges by **adoption-by-content**: a mirror destination bearing exactly the planned bytes is adopted regardless of who created it, because content identity makes the relink semantics-preserving and later foreign removal is out-of-contract external mutation; D-14/D-18), applies exactly the planned reference changes, appends one archive event, then deletes each planned source under one unified precondition (D-17/D-18): **committed issue references are the deletion provenance** — a source is removed only when every selected reference for that artifact already points at its destination in durable issue state and the file re-verifies against its recorded content identity. Failed deletions are reported warnings; rerunning converges by recomputation (D-18): content-identical destinations classify already-archived (publication skipped), and a residual source rediscovered through the inverse mirror mapping satisfies exactly the same precondition before removal — no separate residue rule and no provenance tracking needed. Within the amended contract no step loses a file, overwrites a destination, or leaves a reference relying solely on a missing path. | Only Done/Rejected containers execute (D-8); document targets follow D-15; non-terminal targets preview only. |
| REQ-04: preserve functional relative links for supported bundles (HTML with sibling CSS, theme files, figures) | Recursive discovery follows HTML→CSS→nested figure/font edges and CSS `@import`/`url()`. The D-16 calculus puts every relative-edge target of a relocated parent at the mirror (an archive-root dependency is copied there while its archived source is retained); root-relative and staying-parent edges keep resolving at source. Execution validates every supported edge in the proposed layout before metadata commit, fixing the current verifier's source/destination path-set mismatch (`document.rs:1832-1858`). | Detected dynamic or module loading in a relocated member warns (`dynamic-loading-suspected`, D-23); an actually unpreservable supported edge still blocks (`unpreservable-layout`). |
| REQ-05: report container-oriented candidates using current terminal state, policy, ownership, and blockers without mutation | A read-only `jit archive candidates` lists terminal containers (container-ness from the configured type hierarchy, D-20; membership per D-25), each with three-state documentation-policy status (`configured`/`incomplete`/`unconfigured`), repository-wide ownership evidence, artifact counts, blockers, and move/copy/retain summaries. It consumes the same plan model, always evaluates fully (archival takes no category input, D-21), and performs no filesystem, issue, or event mutation. | Accessor defaults may support display but never authorize mutation; incomplete/unconfigured policy is ineligible. No time/retention semantics (D-6). |
| REQ-06: structured JSON previews verified against Markdown, HTML, CSS, CSV, PNG, and SVG fixtures | The plan model serializes to the binding §2 schema; preview is the non-mutating default for both `jit archive document` and `jit archive container`. A representative fixture corpus (Markdown links; HTML→sibling CSS→nested theme image/font; CSS `@import`/`url()`; direct CSV/PNG/SVG roots; permanent shared figures; active outside consumers; identical filenames; missing edges; pinned commits; a dependency cycle; occupied destinations; a partial-deletion rerun) exercises discovery, classification, preview, and execution. | Preview is informational; `--execute` recomputes and revalidates under the held guard rather than trusting preview output (D-7). |

No criterion is silently narrowed or dropped. The REQ-01 working-tree scoping and the
concurrency contract are owner amendments recorded in the epic Notes and the brief, not
plan-side narrowing.

## 2. Technical soundness and architectural fit

- **Approach.** Introduce a pure artifact-plan model in the domain/document layers: inputs
  are the shared target-root inventory (resolved container subtree roots or an arbitrary
  normalized document root with repo-wide owners and version variants), documentation
  policy, and extracted dependency edges; output is one deterministic, serializable plan.
  The command layer loads inputs, calls the pure planner, renders preview, and executes a
  recomputed plan under the write guard; it does not duplicate decision logic between
  preview and execution. A storage-owned artifact mutation primitive centralizes staging,
  containment, no-replace publication (D-14), and verified deletion (D-17) so the command
  layer stops issuing raw `std::fs` calls (`document.rs:1637-1787`). Candidates, preview,
  and execution are all consumers of the one plan model (formal-planning obligation 5).

- **Container membership (D-25).** The archival subtree is the container plus its
  resolved-hierarchy descendants: the repository-wide hierarchy resolution
  (`graph/hierarchy.rs:458` `resolve_hierarchy`) assigns each node its parent and children
  (`:300`, `:321`), and membership is the transitive `children` closure of the root. The
  raw dependency closure (`DependencyGraph::get_transitive_dependencies`, used for tree
  *scoping* in `graph.rs:206-233`) is **not** membership: it absorbs sequencing edges onto
  issues that belong to other containers, and archiving those would relocate another
  container's artifacts (violating D-2).

- **Artifact identity (D-26).** An artifact is **(path, version)** with version
  `working-tree` or a canonical full immutable commit object ID: every supplied pinned
  revision (symbolic, abbreviated, or full) is canonicalized through a storage/git resolver
  with `rev-parse --verify <rev>^{commit}` semantics, and only the canonical OID is
  serialized, ordered, and compared. Working-tree versions are the archival subjects. A
  pinned reference reads through the storage layer at its commit (`document.rs:349-357`,
  `storage/mod.rs:531` `read_path_text(path, commit)`), i.e. from git history: relocating
  the working-tree file does not break a git-resolvable pinned read, and per D-3 pinned
  references are never rewritten. A reachable pin therefore imposes no working-tree
  constraint. Failed canonicalization, missing git, `CommitNotFound`, or a commit-path
  `NotFound` for the pinned root emits the target-level `pinned-read-failed` blocker; there
  is no read fallback and no warning-plus-retention half-state. Pinned versions appear as
  informational non-relocating `pinned-historical` entries; per the amended REQ-01 their
  commit-resolved dependency closures are outside relocation scope and are not enumerated —
  nothing about them moves, and history serves every pinned reader.

- **Content identity for deletions (D-17 input).** Every working-tree source that a plan
  will delete carries `content_identity = {sha256, byte_size}` captured from one read while
  the guard is held (SHA-256 follows the existing asset convention,
  `document/assets.rs:269-276`). Every removal — fresh run or rerun — requires the D-17
  unified precondition: all selected references for the artifact already point at its
  destination in durable issue state (committed relinks are the deletion provenance), and
  the file re-verifies against the recorded identity immediately before removal; a
  mismatch or an unrelinked reference skips the deletion with a `deletion-failed` warning
  and the file stays. Reruns re-derive the expected identity from the published
  destination (which the mirror mapping locates), never from the possibly-edited source.

- **Destination layout (D-12).** The destination root is `<archive_root>/` plus, for
  container targets, `<container.id>/` (the full UUID — short-id prefixes can collide
  across containers, full UUIDs cannot, matching `.jit/issues/<uuid>.json` naming); every
  moved or copied artifact lands at that root plus its repository-relative source path.
  Archival takes no category input (D-21): the mirror rule alone determines every
  destination. Mirroring under one common prefix keeps every relative offset between bundle
  members invariant, keeps identical filenames from different directories distinct, and
  makes intra-plan destination collisions impossible. The mapping is invertible, which
  rerun convergence exploits (D-18). An `archive_root` on a different filesystem from the
  repository fails with a typed error naming the paths; there is no cross-device staging
  design (owner proportionality amendment).

- **Archive-root precedence.** Before managed/permanent or selected-root classification,
  normalize every working-tree path and test component containment under the configured
  `archive_root`. A match supplies `archived-source` evidence plus needs-source and bypasses
  `unmanaged-selected-root`: a selected/direct root already under the archive root retains
  in place with `already_archived: true`, null destination, and no reference change; an
  embedded archive-root dependency stays edge-aware (a relative edge from a relocated
  parent copies it to the current mirror while the original remains; root-relative or
  staying-parent edges retain). No archive-root source is ever moved or deleted.

- **Ownership universe (D-13).** Repository-wide ownership is computed over every
  `DocumentReference` across all issues plus the recursive supported embedded closure of
  each working-tree document (Markdown/HTML/CSS edges); an artifact referenced directly or
  through an embedded edge from outside the selected subtree is outside-owned and never
  moved (D-2). Files unreachable from any issue reference are outside JIT's referential
  contract: reported as informational not-selected entries when they sit beside bundle
  members (D-10), never silently relocated, never counted as owners. The inventory task
  records direct owners; the classifier — ordered after discovery — completes the embedded
  half.

- **Per-reference selection (before D-16).** The planner classifies each direct
  `DocumentReference` independently (with a deterministic `document_index` within its
  issue's documents) before deriving its artifact's action. Pinned references are
  historical and never selected for relink (D-3/D-26). For a container target, select
  exactly the unpinned references owned by terminal issues inside the selected resolved
  subtree; references outside the subtree or on non-terminal owners stay at source. For a
  document target, D-15 first requires all direct and embedded-closure owners to be
  terminal, then every unpinned direct owner is selected. Embedded edges are not issue
  metadata and never become `reference_changes`; copy/move topology preserves them. Every
  unpinned reference not selected for relink contributes needs-source. `reference_changes`
  is exactly the selected set (issue, document_index, old path, mirror path); no other
  issue reference is rewritten.

- **Edge-aware action calculus (D-16).** The action per working-tree artifact derives from
  two computed constraints. *Needs-destination* holds when the artifact is a selected
  explicit root (unless it is an archived-source direct root), or a relocated (moved or
  copied) parent references it through a **relative** edge — the mirror layout preserves
  that edge only if the dependency exists at its mirrored path. *Needs-source* holds when
  the artifact has an owner outside the subtree, an active owner, any unpinned direct
  reference not selected for relink, lies on a permanent path, sits under the archive root,
  is an embedded dependency outside every explicitly configured managed path, or any
  document that stays in place references it — including **root-relative** edges, which
  resolve from the repository root regardless of the referencing document's location
  (`assets.rs:151-162`). Then: move = needs-destination ∧ ¬needs-source; copy =
  needs-destination ∧ needs-source; retain = ¬needs-destination ∧ needs-source (and for
  unselected artifacts); block = differing-content destination conflict, repository escape,
  symlink involvement (D-22), or any supported edge whose resolution the layout cannot
  preserve (`unpreservable-layout`). Move is possible only when every source-dependent
  unpinned reference is selected and no other needs-source constraint holds. A selected
  root outside every managed path blocks the target as `unmanaged-selected-root` before the
  calculus; an unmanaged embedded dependency carries `unmanaged-path` evidence, always
  needs-source, and copies or retains, never moves. `permanent-path`, `outside-owner`,
  `active-owner`, `unmanaged-path`, and `archived-source` are **evidence/constraint
  flags**, not blockers: they normally produce copy or retain. For a document target, a
  non-terminal direct or embedded-closure owner is the target-level
  `document-non-terminal-owner` blocker (D-15). Copy satisfies both constraint kinds, so
  the calculus is total and deterministic; execution's before/after validation then checks
  every supported edge in the final layout: relative edges at the mirror, root-relative
  edges at the repository root.

- **Archive-plan JSON schema and blocker taxonomy (binding for the plan-model task):**
  - **Envelope:** `schema_version` (starts at 1; fields and codes are append-only and never
    change meaning), `target` (`{"kind": "container"|"document", "id"|"path": …}`),
    `destination_root`, `eligible` (bool), `policy_status`
    (`configured`|`incomplete`|`unconfigured`), `action_counts` (one integer per action
    plus `already_archived` and `pending_deletions`), `count` + `artifacts` (the
    repository list-envelope convention `{"count": N, "<collection>": [...]}`; entries
    deterministically ordered by normalized source path, then version), and plan-level
    `blockers`/`warnings` (each sorted by code, then path).
  - **Artifact entry:** `source`, `version` (`working-tree` or the canonical full commit
    OID; never a symbolic/abbreviated revision), `content_identity`
    (`{"sha256": <64-lower-hex>, "byte_size": N}`; required for every planned deletion and
    published source, null otherwise), `destination` (null when retained or historical),
    `action` (`move|copy|retain|block`), `already_archived` (bool), `provenance` (a flag
    set — `["explicit"]`, `["embedded"]`, or both), `format`, `owners` (issue id,
    deterministic `document_index`, state, `inside_subtree`, `pinned`,
    `selected_for_relink`), `edges` (`supported`/`unsupported`/`external`, each with
    resolution mode), `reference_changes` (exactly the selected unpinned direct references:
    issue, document_index, from-path, to-path), `pending_deletions` (the source paths this
    plan will remove, each carrying its recorded `content_identity`; a residual source
    rediscovered by a rerun through the inverse mirror mapping appears here too, so residue
    cleanup is an executable plan operation), and per-artifact `evidence`, `blockers`,
    `warnings`.
  - **Policy completeness (binding):** `configured` requires the `[documentation]` table
    with explicit `managed_paths`, `permanent_paths`, and `archive_root`. A missing table
    is `unconfigured`; a present table missing any field is `incomplete`. Both are
    target-level blockers (`policy-unconfigured`, `policy-incomplete`) that make execution
    ineligible; accessor defaults may support preview display but never authorize mutation
    (D-1).
  - **Evidence/constraint flags (stable kebab-case, not blockers):** `permanent-path`,
    `outside-owner`, `active-owner`, `unmanaged-path`, `archived-source`,
    `pinned-historical`.
  - **Blocker codes (stable kebab-case):** `policy-unconfigured`, `policy-incomplete`,
    `unmanaged-selected-root`, `destination-conflict`, `document-non-terminal-owner`,
    `pinned-read-failed`, `repository-escape`, `unresolvable-edge`, `unpreservable-layout`,
    `non-terminal-target`, `missing-source`, `symlink-artifact`.
  - **Warning codes:** `missing-edge-target`, `external-edge`, `no-owner`,
    `deletion-failed`, `not-selected-sibling`, `residue-source`,
    `dynamic-loading-suspected`.
  - **Missing/already-archived semantics (evaluation order is normative):** archive-root
    precedence runs first (above). For pinned versions, revision canonicalization runs
    next; its failure or a failed commit read emits `pinned-read-failed`, which the
    working-tree rules below cannot downgrade. Then, per working-tree root: (1)
    **mirror recognition** — a root whose stated source path maps through D-12 to
    content-identical existing content classifies `already_archived`, which means exactly
    that publication is unnecessary (a root moved by a prior partial execution can never
    read as missing on rerun). This is **adoption-by-content, deliberately without
    provenance** (D-18): whether the identical bytes were placed by an interrupted JIT
    execution or by someone else is immaterial — relinking to identical content is
    semantics-preserving either way, the publish primitive never overwrites anyone, and a
    foreign creator later removing the adopted file is out-of-contract external mutation.
    The artifact's action and any source deletion still follow the ordinary calculus and
    the unified deletion precondition (D-17/D-18), so already-archived recognition neither
    forbids nor authorizes removal by itself; only a **differing-content** occupied
    destination is a conflict (`destination-conflict`); (2)
    **`missing-source`** —
    only a root absent at both its stated location and its mirror image blocks, a genuinely
    dangling reference to resolve or remove, never silently entrenched or skipped. A
    missing **embedded** target gets warning `missing-edge-target` on the referencing
    artifact: no needs-destination constraint, excluded from before/after validation (the
    edge resolves nowhere before archival, so no regression is possible), never blocking.
  - **Candidate collection envelope:** `jit archive candidates --json` emits
    `{"schema_version": 1, "count": N, "candidates": [...]}` per the repository
    list-envelope convention; each candidate is the same target plan envelope. Every check
    with explicit inputs is evaluated; an incomplete/unconfigured policy reports its
    distinct blocker and never substitutes defaults to claim eligibility.
  - **Event contract (binding; one event kind).** Every mutating execution — filesystem or
    issue mutation alike, so a publication-only copy operation included — appends one
    archive event after publications and reference relinks persist and before source
    deletions, recording the target, destination root, published artifacts, applied
    reference changes,
    and the planned deletions with their recorded identities. **An event-append failure
    reverts the just-applied reference changes** (sources still exist because deletions
    have not run — the same rollback the current single-document command performs,
    `document.rs:1391-1435`), so no reference mutation ever persists unrecorded; the
    published destination files remain as inert content that the next mutating run adopts
    by content and records in its own event. Should both the append and the compensating
    revert fail, the record is still never lost permanently, because the event is
    **reconstructible from observed state**: a rerun that finds adopted state uncovered by
    any recorded archive event — references resolving under the destination root, or
    `already_archived` mirror content — appends one reconciling archive event describing
    that observed state; this is the sole exception to the no-op-rerun-appends-nothing
    rule and covers publication-only executions whose append failed. Torn-tail handling
    has two halves: **append performs tail repair** — when the log does not end in a
    newline, the appender writes one before its record, so a torn trailing record left by
    a failed append is isolated as exactly one line and a retry or reconciling append can
    never fuse into it — and **archive-path event reading detects and skips such an
    isolated malformed line**, so a failed append never leaves the log unreadable or
    blocks reconciliation. Deletions then run under the D-17 reference-proven
    precondition and identity verification; failures surface as `deletion-failed`
    warnings in the command result. A rerun that performs mutations (including residue deletions) appends its own
    event; a rerun that mutates nothing appends nothing. "Durable" means the repository's
    process-level append contract (`append_event` returned success); this plan does not
    add power-loss or exactly-once semantics beyond `@/inv/event-log`. A crash between
    mutation and append leaves state that the next rerun recomputes and reports; that
    window is within the amended contract.
  - **Symlink semantics (D-22):** an explicit root or embedded target that is a symbolic
    link, or whose repository-relative path traverses one, classifies as `block` with
    blocker `symlink-artifact`; the mutation primitive resolves paths physically, verifies
    containment on resolved targets, and never moves, copies, or deletes through a link.
    Richer symlink relocation is a follow-up.
  - **Dynamic-loading detection contract (D-23):** `<script src="…">` is a static supported
    edge (the script file is a bundle dependency like any other). The
    `dynamic-loading-suspected` **warning** fires when a relocated HTML or script bundle
    member textually contains a local-path-bearing loading construct from the initial set —
    runtime loaders `fetch(`, `import(`, `XMLHttpRequest`, `new Worker(`, `importScripts(`,
    a URL-bearing `data-*` attribute, or static module syntax with a relative specifier
    (`import … from './…'`, `export … from './…'`, `require('./…')`) — detected by pattern
    match only, never by execution (D-5). It warns rather than blocks (owner
    proportionality amendment): the pattern set cannot distinguish live loaders from code
    examples in presentation content, so it informs the preview reader instead of gating
    them. A supported edge the layout genuinely cannot preserve still blocks via
    `unpreservable-layout`. The set is append-only under `schema_version`.

- **Reuses / integrates with:**
  - Hierarchy membership: `crates/jit/src/graph/hierarchy.rs:458` (`resolve_hierarchy`),
    `:300` (`parent`), `:321` (`children`).
  - Storage read/write: `crates/jit/src/storage/mod.rs:531` (`read_path_text`, commit-aware
    for pinned resolution), `:498` (`read_path_bytes`, enables opaque binary roots without
    an adapter), `:126` (`acquire_repo_write_lock`, re-entrant `RepoWriteGuard` for the
    multi-write sequence).
  - Sequencing principles and occupied-destination no-op:
    `crates/jit/src/commands/document.rs:1637-1787`, `:1670-1683`, `:1391-1435`, with
    failure-injection coverage in `crates/jit/tests/archive_integrity_tests.rs:203-396`.
  - Domain records: `crates/jit/src/domain/types.rs:877` (`DocumentReference`; `path`
    :879, `commit` :881, `assets` :891), `:50` (`State::is_terminal` = Done|Rejected only),
    `:1466` (`Event::DocumentArchived`, whose family the container event extends).
  - Adapters/scanner: `crates/jit/src/document/adapter.rs` (Markdown/HTML built-ins),
    `crates/jit/src/document/assets.rs:16` (`Asset`; root-relative resolution :151-162,
    supplied-map classifier limitation :189, SHA-256 convention :269-276).

- **Grounding (from investigation, re-verified):** unchanged from the reviewed baseline —
  container membership primitive exists but has no archive consumer (`cli.rs:1950-1969`);
  the referentially-consistent single-doc sequence is done and its principles are reused,
  not wrapped (`document.rs:1043-1082`, `archive_integrity_tests.rs:74-396`); dry-run
  incompleteness (`document.rs:1153-1176`, `:1670-1683`), directory-heuristic sharing
  (`document.rs:1463-1495`, `assets.rs:185-233`), shallow discovery
  (`adapter.rs:101-146`), opaque-root exclusion (`assets.rs:92-111`), pinned-relink
  unsafety (`document.rs:1352-1375`, `:349-357`), verifier defect (`document.rs:1832-1858`),
  missing coordination (`storage/mod.rs:126`), and string-prefix policy matching
  (`document.rs:1243-1267`) are all valid-and-open and addressed above; literal atomic
  batch execution remains invalid as stated (`document.rs:1050-1074`).

Layer boundaries (per AGENTS.md, obligation 3): artifact-graph construction and action
classification are pure domain/document logic, free of I/O; storage owns persistence, the
mutation primitive, and verified deletion; commands orchestrate under the guard; CLI/output
stay user-facing. Categories, managed/permanent paths, archive root, and container-ness
come from repository configuration, never hardcoded (`@/inv/domain-agnostic`). No-replace
publication requires amending the registry-first `@/inv/atomic-writes` invariant and its
rendered projection before implementation diverges from its current wording.

## 3. Decomposition sketch (near-ready; jit-breakdown instantiates — no issues created here)

Three conceptual groups organize nine top-level task leaves under the epic; group headers
are organizational only, not issues (keeping every leaf directly under the epic also keeps
the dependency-derived hierarchy from absorbing the storage primitive or preview surface
into an intermediate container). Every task is independently landable and green at its
boundary. Ordering is expressed only through `depends-on`. Group A is the pure foundation;
the filesystem-only storage primitive is independent; coordinated execution joins it with
the preview/planner surface; the candidate report consumes the classifier plus the
`jit archive` command surface the preview task establishes; and the clean-cut legacy
removal lands only after the complete replacement family — execution and candidates —
exists. The plan schema and blocker taxonomy land first, before any CLI fan-out
(obligation 4). Coverage is
enforced at the task tier: each requirement-bearing task carries its `satisfies: REQ-*`
label directly.

### Group A: Artifact plan model and resolver — covers REQ-01, REQ-02, REQ-04

- **Artifact plan model and blocker taxonomy**  `type: task`  `satisfies: REQ-01`  `depends-on: —`
  Outcome: a serializable artifact-plan type implementing schema version 1 as a
  deterministically ordered JSON envelope, with exactly the §2 envelope fields, artifact
  fields, evidence codes, blocker codes, and warning codes; the sets are binding and
  append-only under schema versioning.
  Own criteria: `[hard] LOCAL-01: Serializes a plan with stable artifact and blocker
  ordering independent of input order.` `[hard] LOCAL-02: The same plan object is the
  input to both preview rendering and execution.` `[hard] LOCAL-03: Artifact identity is
  (path, version); resolved pinned versions serialize and order only by canonical full
  commit OID as non-relocating historical entries.` `[hard] LOCAL-04: Policy status is
  configured only when managed_paths, permanent_paths, and archive_root are all explicit;
  partial and absent policy serialize distinctly and neither is executable.`
  `[hard] LOCAL-05: A golden schema-version-1 conformance test asserts the complete field
  and code lists; missing, renamed, or reclassified entries fail, and additions require an
  explicit schema-versioning decision.`
  Blast radius: self-contained new module; no existing consumer changes.

- **Target root inventory and ownership**  `type: task`  `satisfies: REQ-01`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: one versioned explicit-root inventory for both target kinds. A container yields
  its resolved-hierarchy subtree (D-25) and every distinct issue-linked root on those
  issues; a document target normalizes its arbitrary path as an explicit working-tree root
  even when opaque or zero-owner, looks up every direct owner repository-wide, and adds
  canonical pinned historical variants beside the working-tree variant.
  Own criteria: `[hard] LOCAL-06: Membership is the resolved-hierarchy children closure; a
  cross-container sequencing dependency contributes no member.` `[hard] LOCAL-07:
  Inventories every distinct DocumentReference in the closure, including CSV/PNG/SVG roots
  with no registered adapter and pinned versions as historical entries.` `[hard] LOCAL-08:
  Records every direct issue-reference owner of each artifact repository-wide, flagging
  owners outside the subtree.` `[hard] LOCAL-09: Canonicalizes each supplied pinned
  revision (symbolic, abbreviated, or full) to the full commit OID and emits
  pinned-read-failed on failed resolution.` `[hard] LOCAL-10: A zero-owner arbitrary
  CSV/PNG/SVG document path becomes a normalized explicit working-tree root with no-owner
  evidence, without requiring a text adapter.`
  Blast radius: adds the reusable storage/git canonical-revision resolver; otherwise
  reuses `hierarchy.rs:458` and `storage/mod.rs:498`.

- **Recursive supported-dependency discovery**  `type: task`  `satisfies: REQ-01, REQ-04`  `depends-on: Artifact plan model and blocker taxonomy, Target root inventory and ownership`
  Outcome: recursive discovery of supported local dependencies of working-tree artifacts —
  Markdown/HTML element URLs and CSS `@import`/`url()` — with cycle detection, normalized
  component-aware path resolution, before/after edge reachability, repository-escape
  rejection, and the D-23 textual detection set emitted as warnings.
  Own criteria: `[hard] LOCAL-11: Discovers an HTML→sibling-CSS→nested-figure chain and
  CSS @import/url() targets to full depth, treating script-element src URLs as static
  supported edges.` `[hard] LOCAL-12: Terminates on dependency cycles and emits
  dynamic-loading-suspected warnings for local-path-bearing constructs from the binding
  D-23 set, by textual pattern match only, without blocking.` `[hard] LOCAL-13: A pinned
  root whose commit read fails emits pinned-read-failed; reachable pinned roots contribute
  no working-tree constraints and no embedded-closure scan.`
  Blast radius: extends the document adapter/scanner surface; existing Markdown/HTML
  callers (snapshot export) are unaffected because discovery is a new recursive path.

- **Move/copy/retain/block classification**  `type: task`  `satisfies: REQ-02, REQ-04`  `depends-on: Target root inventory and ownership, Recursive supported-dependency discovery`
  Outcome: a pure classifier assigning each working-tree artifact move, copy, retain, or
  block through the D-16 calculus, completing the D-13 ownership universe with the
  supported embedded closure over every issue-linked working-tree document, applying
  per-reference selection, archive-root precedence, explicit policy completeness, the
  D-26 pinned model, D-12 mirror destinations, and the normative
  missing/already-archived evaluation order.
  Own criteria: `[hard] LOCAL-14: Ownership incorporates supported embedded references
  reachable from any issue-linked working-tree document repository-wide, flagging outside
  owners.` `[hard] LOCAL-15: Classifies deterministically per the calculus — a figure
  shared by a terminal and an active issue copies when a relocated parent references it
  relatively and retains otherwise, never force-relinking the active consumer;
  permanent-path, outside-owner, active-owner, unmanaged-path, and archived-source are
  evidence, not blockers.` `[hard] LOCAL-16: Every supported edge's resolution mode
  (relative vs root-relative) drives the decision; the classifier never emits a layout in
  which a relocated parent's supported edge lacks its target, and a relative edge into
  archive_root copies that dependency to the mirror while the archived source is
  retained.` `[hard] LOCAL-17: Uses path-component containment for managed/permanent
  status, so dev/active-other does not match dev/active; an unmanaged selected root blocks
  as unmanaged-selected-root while an unmanaged embedded dependency copies or retains,
  never moves.` `[hard] LOCAL-18: Proposes destinations by mirroring repository-relative
  source paths beneath the destination root (full container UUID segment); a
  differing-content occupied destination blocks as destination-conflict and a
  content-identical one classifies already_archived with publication skipped, its action
  and deletion still following the calculus and the unified D-17/D-18 precondition;
  symlink paths block.` `[hard] LOCAL-19: Computes reference_changes
  per reference: pinned never; container targets select only unpinned terminal-inside
  references; document targets select all unpinned direct owners after D-15; every
  unselected unpinned reference forces needs-source, so mixed-owner artifacts copy or
  retain and move only when every source-dependent unpinned reference is selected.`
  Blast radius: self-contained; supersedes the archive-time directory heuristic without
  touching the legacy command until Group B.

### Group B: Unified archive CLI and safe executor — covers REQ-03, REQ-04, REQ-06

- **Storage-owned artifact mutation primitive**  `type: task`  `satisfies: REQ-03`  `depends-on: —`
  Outcome: narrowly scoped filesystem primitives that stage bytes, verify staged bytes
  against caller-supplied SHA-256+size, publish new destinations atomically with no
  replacement — returning success only when this call created the destination, and
  `AlreadyExists` for any collision including a content-identical one (D-14) — validate
  physical containment with no symlink traversal, and delete a caller-named source only
  after re-verifying its caller-supplied recorded identity (D-17). They neither acquire
  the operation-wide guard nor plan, relink issues, append events, or orchestrate
  execution. A cross-filesystem destination fails with a typed error. Testable through
  tempdir-backed storage (the in-memory backend performs no virtual file I/O).
  Own criteria: `[hard] LOCAL-20: Rejects an occupied destination — including a
  content-identical external creation between check and publish — with the pre-existing
  file preserved byte-for-byte, verified by a race-focused test.` `[hard] LOCAL-21:
  Before introducing no-replace publication, amends the registry-first atomic-writes
  invariant and its rendered projection to cover staged no-replace publication alongside
  temp-file + atomic-rename replacement.` `[hard] LOCAL-22: Deletes a source only after
  re-verifying the caller-supplied recorded content identity; mismatch performs no
  removal and reports it.` `[hard] LOCAL-23: Exposes only stage, no-replace publish,
  verified delete, and physical-containment operations; it performs no issue save, event
  append, planning, or operation-wide locking, and a cross-filesystem archive root is a
  typed error, not a fallback.`
  Blast radius: new storage API plus the authoritative `.jit/invariants.toml` entry and
  its rendered AGENTS.md projection; the legacy command keeps its inline `std::fs` path
  until removed.

- **Unified archive preview surface**  `type: task`  `satisfies: REQ-06`  `depends-on: Move/copy/retain/block classification`
  Outcome: `jit archive document <path>` and `jit archive container <id>` produce a
  complete non-mutating plan by default, running every precondition execution relies on
  (source availability, explicit complete policy, terminal eligibility, outside owners,
  pinned semantics, destination conflicts, before/after reachability), rendered as both
  the binding JSON envelope and human output, and verified against the
  Markdown/HTML/CSS/CSV/PNG/SVG fixture corpus.
  Own criteria: `[hard] LOCAL-24: Preview enumerates every artifact, action, evidence
  flag, and blocker and mutates nothing.` `[hard] LOCAL-25: Document and container targets
  produce the same plan schema from the shared planner.` `[hard] LOCAL-26: With absent or
  partial documentation policy, preview returns a non-mutating inventory distinguishing
  unconfigured from incomplete and explaining that archival is disabled, and execution is
  refused without substituting defaults.` `[hard] LOCAL-27: Fixture tests for Markdown,
  HTML, CSS, CSV, PNG, and SVG roots and bundles produce schema-version-1 previews with
  deterministic artifact/action/blocker results, and each human rendering represents the
  same plan as its JSON rendering.`
  Blast radius: adds a new `archive` command group; MCP tools regenerate from the CLI
  schema automatically.

- **Coordinated plan execution**  `type: task`  `satisfies: REQ-03, REQ-04`  `depends-on: Storage-owned artifact mutation primitive, Unified archive preview surface`
  Outcome: `--execute` acquires and holds `RepoWriteGuard` across the whole operation,
  recomputes the plan (preview output is never an execution input, D-7), refuses
  non-terminal containers and D-15-ineligible document targets, stages and verifies
  content identities captured under the guard, validates every supported local edge in
  the proposed layout, publishes no-replace, applies exactly the planned
  `reference_changes` (pinned references untouched), refreshes or invalidates cached
  asset metadata on moved references, appends one archive event recording publications,
  relinks, and planned deletions, then deletes each planned source under the D-17 unified
  precondition — every selected reference committed to its destination, plus
  recorded-identity verification — reporting failures as `deletion-failed` warnings.
  Rerunning after any partial failure converges by recomputation: content-identical
  destinations classify already-archived, residual sources are rediscovered through the
  inverse mirror mapping and deleted under the same precondition, and a run that mutates
  nothing appends no event.
  Own criteria: `[hard] LOCAL-28: Validates every staged supported local edge against the
  proposed destination layout, not source-resolved paths, before metadata commit.`
  `[hard] LOCAL-29: Refuses execution on a non-terminal container; refuses document-target
  execution while any direct or embedded-closure owner is non-terminal, while a managed
  zero-owner document executes with informational evidence.` `[hard] LOCAL-30: Applies
  exactly reference_changes — pinned references untouched, container targets relink only
  unpinned terminal-inside references, document targets relink every unpinned direct
  owner, unselected references remain source-resolvable — and holds one write guard from
  recomputation through the final deletion attempt.` `[hard] LOCAL-31: Appends one archive
  event after publications and relinks and before deletions, recording publications,
  reference changes, and planned deletions, publication-only executions included; an
  event-append failure reverts the just-applied reference changes; a rerun that mutates
  nothing appends no event, except that adopted state uncovered by any recorded archive
  event (destination-rooted references or already-archived mirror content) triggers one
  reconciling event; append performs tail repair so a torn record stays one isolated
  line, and event reading skips it.` `[hard] LOCAL-32: Failure injection for partial staging (including staged-temp cleanup
  verification), partial relink, event-append failure with and without a successful
  compensating revert, deletion failure, and a source edited after planning proves no
  artifact is lost, no
  destination overwritten, and no reference relying solely on a missing path; a rerun
  after each injected failure converges — already-archived recognition, inverse-mapping
  residue rediscovery, deletion only under the unified precondition (every selected
  reference committed to its destination plus identity verification, so a source with any
  reference still pointing at it is always retained), and no duplicate destination,
  reference, or event.`
  Blast radius: command-layer execution over the storage primitive and shared planner; no
  legacy-command change.

- **Remove the legacy document-archive command**  `type: task`  `satisfies: —`  `depends-on: Coordinated plan execution, Read-only container candidate report`
  Outcome: `jit doc archive` is removed completely as a clean-cut migration with no alias
  or stub, and all consumers (CLI/main dispatch, `ArchiveResult`, event catalog/schema
  references, integration and unit tests, fixtures, docs, README, and the auto-generated
  MCP tool set) are updated in the same change so the tree builds and tests pass with only
  the unified surface. The legacy `[documentation.categories]` configuration table retires
  with it (D-21): its loader field, docs, and this repository's config entries go in the
  same change.
  Coverage: migration support only; it carries no container requirement and is exempt from
  `satisfies:` coverage rather than claiming credit borne by the replacement tasks.
  Own criteria: `[hard] LOCAL-33: A tree-wide search for the legacy surface (the strings
  "doc archive", "archive_document", "ArchiveResult") across crates/, mcp-server/, web/,
  docs/, and dev/ returns no live production or test reference; only historical dev/
  records may mention it.`
  Blast radius: enumerated in the acceptance-check note below; `crates/server` and `web`
  have no live consumer (web matches are the `Archived` issue state, not the command).

### Group C: Container candidate reporting — covers REQ-05

- **Read-only container candidate report**  `type: task`  `satisfies: REQ-05`  `depends-on: Move/copy/retain/block classification, Unified archive preview surface`
  Outcome: `jit archive candidates` lists terminal containers using the shared planner —
  container-ness derives from the configured type hierarchy (any type at a non-leaf level
  of `[type_hierarchy]`, D-20), membership from the resolved hierarchy (D-25), never from
  hardcoded type names — each with three-state documentation-policy status,
  managed/permanent path status, repository-wide artifact ownership, artifact counts,
  blockers, and a move/copy/retain summary, in both human and JSON form, with no
  filesystem, issue, or event mutation and no age or retention filtering. Every candidate
  is evaluated fully: archival takes no category input (D-21), so the D-12 mirror rule
  determines each destination and no check is deferred or guessed.
  Own criteria: `[hard] LOCAL-34: Lists terminal containers with policy status, ownership,
  counts, and blockers, explaining exclusions rather than omitting them.`
  `[hard] LOCAL-35: Emits identical human and JSON results while mutating nothing, and
  applies no time/retention filter.` `[hard] LOCAL-36: Derives candidate container-ness
  from the configured type hierarchy's non-leaf levels and resolved-hierarchy membership,
  with no hardcoded type names, and evaluates every candidate fully with no category
  input.`
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

> **Removal acceptance check (D-9, clean cut, no alias).** The removal task depends on
> Coordinated plan execution and the candidate report, so the old `jit doc archive` and
> the complete new `jit archive` family coexist through every intermediate wave and no
> wave breaks a consumer before its replacement exists. Acceptance is a tree-wide search across `crates/`, `mcp-server/`,
> `web/`, `docs/`, and `dev/` for `doc archive`, `archive_document`, and `ArchiveResult`,
> which must return no live production or test reference (historical `dev/` records
> excepted). The pre-plan sweep of that exact pattern reported **103 matches across 33
> files**; the live production Rust consumers are `crates/jit/src/{cli.rs, main.rs,
> commands/document.rs, commands/mod.rs, schema.rs, domain/event_catalog.rs}`; the test
> consumers are `crates/jit/tests/{doc_archive_tests.rs, archive_integrity_tests.rs,
> command_exit_code_projection_tests.rs}`; the rest are docs/ and dev/ prose. The MCP
> tool set is auto-generated from the CLI schema, so removing the subcommand drops it on
> regeneration. `crates/server` and `web` carry no live consumer.

## 4. Risks and actionability

| Risk / open question | Severity | Mitigation or decision |
|---|---|---|
| Membership drawn from the wrong relation relocates another container's artifacts | High | D-25: membership is the resolved-hierarchy children closure (`hierarchy.rs:458`, `:321`), never the raw dependency closure; a sequencing edge contributes no member (LOCAL-06). |
| An intermediate container absorbs storage/preview prerequisites through the dependency-derived hierarchy | High | All nine leaves resolve directly under the epic; group headers are prose only, so no dependency-derived container captures the primitive or preview. |
| A retained dependency leaves a relocated parent's relative link without its target | High | D-16's calculus is total: a relative edge from a relocated parent forces needs-destination; root-relative edges force source retention; the classifier never emits an unpreservable layout (LOCAL-16) and execution re-validates every edge (LOCAL-28). |
| An artifact embedded by an outside document misclassified as unshared and moved | High | D-13's universe includes the supported embedded closure of every issue-linked working-tree document; the supplied-map limitation of the existing classifier (`assets.rs:189`) is superseded (LOCAL-14). |
| Mixed owners cause the wrong issue references to relink or lose their source | High | Per-reference selection precedes D-16: pinned never relink; container targets select only unpinned terminal-inside refs; document targets select all unpinned direct refs after D-15; every unselected unpinned ref forces needs-source (LOCAL-19, LOCAL-30). |
| Symbolic or abbreviated pinned revisions make identity nondeterministic | Medium | D-26: canonicalize every revision to the full commit OID; failed resolution or reads block as pinned-read-failed with no fallback (LOCAL-09, LOCAL-13). |
| A partial documentation table silently authorizes default policy | High | D-1: only explicit managed_paths, permanent_paths, and archive_root produce `configured`; absent and incomplete remain distinct, previewable, and non-executable (LOCAL-04, LOCAL-26). |
| Archive-root sources move or delete when a new container references them | Medium | Archive-root precedence: `archived-source` always supplies needs-source; direct roots retain, relative-edge dependencies copy to the current mirror, nothing under archive_root moves or deletes (LOCAL-16, LOCAL-18). |
| A source edited between planning and deletion is destroyed | Medium | Content identities are captured under the held guard and re-verified immediately before each deletion; mismatch skips with deletion-failed and the file stays (D-17, LOCAL-22, LOCAL-32). Broader external interference is out of contract per the owner amendment; git history recovers versioned repositories. |
| Post-archive verification defect masks broken links | Medium | Executor validates each edge in the proposed layout, not against source-resolved paths (LOCAL-28); replaces `document.rs:1832-1858`. |
| A crash or failure mid-execution leaves a half-archived container | Medium | D-18: rerun recomputes and converges — already-archived recognition, inverse-mapping residue rediscovery, verified deletion, no duplicate destinations/references/events (LOCAL-32). The event appends before deletions, so the audit trail exists for every published state. |
| No-replace publication diverges from the atomic-writes invariant wording | Medium | The storage task first amends the registry-first invariant and its rendered projection (LOCAL-21); implementation targets the amended contract. |
| The D-23 pattern set misfires on code examples in presentations | Low | Downgraded to the dynamic-loading-suspected warning (owner amendment): preview surfaces it, nothing blocks on content mentions; genuinely unpreservable supported edges still block (LOCAL-12). |
| Removing `jit doc archive` breaks docs/tests/MCP mid-flight | Medium | Removal ordered last; consumer migration in one clean-cut change; acceptance by the tree-wide search (LOCAL-33). |
| Storage primitives and command orchestration overlap or leave a coordination gap | Medium | Storage only stages/publishes/verifies/deletes and checks containment (LOCAL-23); the command holds `RepoWriteGuard` from recomputation through the final deletion attempt (LOCAL-30, `storage/mod.rs:126`). |

Every load-bearing question is resolved or owned. Each sketch item is executable from its
description plus the cited grounding without re-deriving the design.

## Decisions

Carried from the owner-approved brief (`dev/active/7d3a3a47-planning-brief.md`), binding,
including both 2026-07-13 owner amendments (categories scrapped; proportionality scope).
Plan-level decisions D-12 onward resolve specification gaps surfaced during planning and
review; none is REOPEN.

- **D-1 — Archival is explicit opt-in:** chosen **require configured documentation policy,
  with explicit three-state completeness (configured/incomplete/unconfigured); defaults
  never authorize mutation**. Rejected: silently use default paths (moves evidence never
  delegated to JIT).
- **D-2 — Do not relocate another container's artifacts:** chosen **never relocate an
  artifact referenced outside the subtree; copy if required, block if ambiguous**.
  Rejected: global relink; a general force option.
- **D-3 — Pinned references remain historical:** chosen **never rewrite a pinned
  reference; keep its historical source path resolvable, copy content into the bundle when
  needed**. Rejected: path-only relink; implicit conversion to working-tree provenance.
  Refined by D-26.
- **D-4 — One container-owned destination (amended by owner, 2026-07-13):** chosen
  **destination is a container-owned directory preserving internal topology; the brief's
  category-selection clause is superseded — archival takes no category input (D-21)**.
  Rejected: category-per-artifact placement (fragments bundles); caller-selected
  categories (legacy taxonomy scrapped in favor of the pure mirror rule).
- **D-5 — Initial recursive discovery is static:** chosen **follow Markdown/HTML/CSS local
  references incl CSS `url()`/`@import`; never execute or guess dynamic dependencies**.
  Made precise by D-23 (suspected dynamic loading warns; unpreservable supported edges
  block). Rejected: guessing dynamic dependencies.
- **D-6 — Time-based archival deferred:** chosen **no retention periods, age filters,
  scheduled sweeps, or lifecycle triggers; candidates use current terminal state and
  policy**. Rejected: first-done, updated, or reconstructed timestamps.
- **D-7 — Preview defaults, execution explicit:** chosen **complete non-mutating plan by
  default; mutation requires `--execute`, which recomputes and revalidates under the held
  guard; preview output is never an execution input**. Rejected: applying stale preview
  data.
- **D-8 — Only terminal containers execute:** chosen **Done/Rejected may execute;
  non-terminal may preview only, no force override**. Rejected: force execution of active
  containers.
- **D-9 — One unified archive command family (amended by owner, 2026-07-13):** chosen
  **`jit archive document|container|candidates` on one planner/schema/policy/executor,
  with no category flag; remove `jit doc archive` completely as a clean-cut pre-v1.0
  migration**. Rejected: retaining an alias or migration stub.
- **D-10 — Reachability defines a bundle:** chosen **explicit issue-linked artifacts plus
  recursively reachable supported static dependencies; unreferenced siblings reported as
  informational, not moved**. Rejected: sweeping an entire source directory implicitly.
- **D-11 — Candidates are container-oriented:** chosen **list terminal containers with
  policy status, artifact counts, blockers, and summaries; individual artifacts are
  details inside a candidate**. Rejected: individual artifacts as independent archival
  candidates.
- **D-12 — Destination layout mirrors repository-relative paths:** chosen **destination
  root `<archive_root>/` (+ `<container.id>/`, the full UUID, for container targets), each
  artifact at root + repository-relative source path, no category segment (D-21)**. The
  full UUID is the container segment because short-id prefixes are not unique and
  destination determinism must not depend on the current id population. Preserves every
  relative offset under a common prefix, keeps identical filenames distinct, makes
  intra-plan collisions structurally impossible, and is invertible (used by D-18). A
  cross-filesystem archive root is a typed error (owner proportionality amendment).
  Rejected: stripping the managed prefix per artifact (the current single-doc rule,
  `document.rs:1284-1319` — cross-root collisions, broken cross-directory links);
  content-addressed layout (destroys navigable topology); short-id segments (prefix
  collisions); destination-adjacent staging designs for nested mounts (complexity without
  a demonstrated case).
- **D-13 — Ownership universe is the issue-linked closure:** chosen **all
  `DocumentReference`s across all issues plus each working-tree one's recursive supported
  embedded closure; outside owners (direct or embedded) forbid a move**. Files unreachable
  from any issue reference are outside JIT's referential contract: reported
  informationally near bundles (D-10), never counted as owners, never relocated. Rejected:
  scanning the entire working tree for arbitrary referencing files (unbounded; JIT's
  guarantee is scoped to issue references).
- **D-14 — Publication is atomic no-replace:** chosen **stage, verify staged bytes against
  the captured identity, then publish so that success is returned only when this call
  created the destination; any collision — including a content-identical external
  creation — is `AlreadyExists`, never silent success**. Requires amending the
  registry-first `@/inv/atomic-writes` invariant and its projection before implementation
  (LOCAL-21). Rejected: check-then-`rename` (replaces a destination created after the
  check); the *primitive* claiming success on identical content (a primitive must report
  exactly what it did — it created nothing; the *command* layer separately adopts
  content-identical destinations on recompute via D-18 adoption-by-content, and the two
  layers are deliberately distinct).
- **D-15 — Document-target execution requires all-terminal owners:** chosen **`jit archive
  document --execute` requires every direct or embedded-closure owner of the document and
  its bundle artifacts to be terminal; active owners block (preview still reports); a
  zero-owner managed-path document executes with an informational note**. Rejected:
  selecting an owning container implicitly (ambiguous with several owners); a force
  override (mirrors the rejected D-8 force path).
- **D-16 — Edge-aware action calculus:** chosen **derive each action from
  needs-destination (selected root, or relative edge from a relocated parent) ×
  needs-source (outside owner, active owner, unselected unpinned reference, permanent
  path, archive-root containment, unmanaged embedded dependency, or inbound
  root-relative/staying-document edge): move / copy / retain, with structural conflicts
  blocking**. Copy creates the mirrored destination and retains the source, satisfying
  both constraints, so every supported edge resolves in the final layout. Rejected: a free
  copy-or-retain choice (leaves a relocated parent's relative edge without its target);
  treating root-relative edges like relative ones (their resolution ignores the
  referencing document's location, `assets.rs:151-162`); ownership facts as blockers
  (the calculus resolves them to copy/retain).
- **D-17 — Deletions are reference-proven and verify-then-delete:** chosen **one unified
  precondition for every source removal, fresh run or rerun: (a) every selected reference
  for the artifact already points at its destination in durable issue state — committed
  relinks are the deletion provenance, so no receipt or provenance tracking is needed —
  and (b) the file re-verifies against its recorded `{sha256, byte_size}` identity,
  captured under the held guard (reruns re-derive the expected identity from the published
  destination, never from the possibly-edited source); mismatch or an unrelinked
  reference skips the removal with `deletion-failed` and the file stays**. Rejected (owner
  proportionality amendment): quarantine protocols, directory-handle-anchored unlink
  sequences, and adversarial-substitution defenses — the interference they defend against
  is out of contract, and git history recovers versioned repositories; unverified deletion
  (destroys benign concurrent edits detectably avoidable at one hash's cost).
- **D-18 — Rerun converges by recomputation and adoption-by-content:** chosen **a rerun
  after any partial failure recomputes the plan against current state, treats
  content-identical occupied destinations as already archived (publication skipped,
  adoption-by-content: identical bytes are adopted without provenance because relinking
  to them is semantics-preserving whoever wrote them, and their later foreign removal is
  out-of-contract external mutation), rediscovers residual sources
  through the inverse D-12 mapping, and applies only the remaining mutations — every
  removal under the same D-17 reference-proven precondition, which is what distinguishes
  a deletable residue (all selected references committed to the destination) from a
  live or foreign source (any reference still at source ⇒ retained) without provenance
  records — and appends no event when nothing mutates**. Repository-relative destination derivation makes nesting
  impossible on rerun. Rejected: persisted write-ahead intent journals and receipts (owner
  proportionality amendment — recovery state machines for a short, recomputable,
  git-recoverable operation); treating every occupied destination as a blocker (makes
  every partial failure permanent).
- **D-19 — One archive event per mutating execution:** chosen **every execution that
  mutates anything — filesystem publications or issue state — appends one event after
  publications and relinks persist and before deletions, recording target, destination
  root, publications, applied reference changes, and planned deletions;
  deletion failures are command-result warnings; a mutating rerun appends its own event
  and a no-op rerun appends none; an event-append failure reverts the just-applied
  reference changes (deletions have not run, so sources exist for the revert — the
  rollback the single-document command already performs, `document.rs:1391-1435`); and
  when both the append and the revert fail, a rerun detecting adopted state uncovered by
  any recorded archive event — destination-rooted references or already-archived mirror
  content, covering publication-only executions — appends one reconciling event
  describing the observed state, so nothing persists permanently unrecorded; append
  performs tail repair (a newline precedes the record when the log lacks a trailing one),
  isolating a torn record as one line no retry can fuse into, and archive-path event
  reading detects and skips that isolated malformed line, so a failed append never
  leaves the log unreadable**. Durability is the existing process-level
  `append_event` contract (`@/inv/event-log`); a crash between mutation and append is
  within the amended contract and the next rerun recomputes and reports. Rejected (owner
  proportionality amendment): the four-event Started/Executed/SourcesRemoved/Aborted
  family, operation/instance/execution identity triples, and publication receipts —
  audit-grade provenance for a threat model the epic no longer carries; exactly-once
  event semantics (unachievable across a crash with an append-only log).
- **D-20 — Container-ness derives from the type hierarchy:** chosen **a candidate
  container is a terminal issue whose configured type sits at a non-leaf level of
  `[type_hierarchy]`**. Engine code names no type; the boundary comes from repository
  configuration (`@/inv/domain-agnostic`). Rejected: hardcoding strategic type names;
  treating every issue with DAG children as a container.
- **D-21 — Document categorization is scrapped (owner, 2026-07-13):** chosen **archival
  takes no category input anywhere in the command family; the D-12 mirror rule alone
  determines destinations, candidates always evaluate fully, and the legacy
  `[documentation.categories]` table retires with the legacy command**. Rejected:
  caller-selected categories (an extra input adding a failure mode and no information the
  path does not already carry); doc_type-derived inference (no doc_type→category mapping
  exists in the configuration model — `doc_type` is a free-form optional string,
  `types.rs:885`).
- **D-22 — Symlinks block in v1:** chosen **a symlink root or embedded target, or a path
  traversing one, classifies `block` with `symlink-artifact`; the mutation primitive
  resolves paths physically, verifies containment on resolved targets, and never moves,
  copies, or deletes through a link**. Rejected: transparent symlink following (relocation
  through a link silently changes what other referents resolve to); rewriting links.
- **D-23 — Dynamic-loading detection is an enumerated textual warning (amended by owner,
  2026-07-13):** chosen **`<script src>` is a static supported edge; a relocated HTML or
  script member containing a local-path-bearing construct from the binding set (runtime
  loaders, URL-bearing `data-*` attributes, static module syntax with relative
  specifiers) gets the `dynamic-loading-suspected` warning — pattern match only, never
  execution, never a block**. The pattern set cannot distinguish live loaders from code
  examples in presentation content, so blocking on it would fire falsely on exactly the
  target corpus; genuinely unpreservable supported edges still block via
  `unpreservable-layout`. Rejected: blocking on content mentions (false-positive
  hand-resolution burden defeats the easy-convention goal); executing or parsing
  JavaScript (guessing, D-5); ignoring script content entirely (no signal at all).
- **D-24 — The concurrency contract is scoped, not maximal (owner, 2026-07-13):** chosen
  **guarantees hold under the repository write guard against concurrent JIT writers;
  benign concurrent modification is detected before destructive steps (no-replace
  publication, verify-recorded-hash-then-delete); concurrent external mutation during an
  archival operation is out of contract, with git history as the recovery channel in
  versioned repositories**. Rejected: adversarial-filesystem defenses (quarantine,
  handle-anchored removal, receipts) — an adversary with repository write access can
  delete files directly, so those defenses purchase no real capability bound at permanent
  complexity cost; claiming unconditional no-loss (unprovable on a path-based
  filesystem).
- **D-25 — Container membership is the resolved-hierarchy subtree:** chosen **the archival
  membership of a container is the root plus the transitive `children` closure of the
  repository-wide hierarchy resolution (`hierarchy.rs:458`, `:321`)** — the same
  DAG-authoritative relation the tree and divergence surfaces use. Rejected: the raw
  transitive dependency closure (absorbs cross-container sequencing edges, violating D-2);
  label-based membership (labels are hints; the DAG is authoritative).
- **D-26 — Artifact identity is (path, version); pinned versions are historical and
  unenumerated (amended by owner, 2026-07-13):** chosen **artifacts are keyed by path plus
  version (`working-tree` or a canonical full commit OID via the rev-parse resolver);
  working-tree versions are the archival subjects; pinned versions are informational
  `pinned-historical` entries that are never relocated or rewritten and impose no
  working-tree constraint while readable; failed canonicalization or commit reads block
  the target as `pinned-read-failed` with no fallback; commit-resolved dependency closures
  are outside relocation scope and not enumerated (REQ-01 is scoped to working-tree
  artifacts accordingly)**. Rejected: path-only identity (cannot represent two versions of
  one path); warning-plus-retention for unreachable pins (retention cannot make a broken
  pinned read resolvable; ineligibility is honest); commit-closure enumeration (git-read
  plumbing and fixtures for entries that never move — report padding at maintenance
  cost).
- **Assumptions:** Coverage is enforced at the task tier: each of the nine task-tier items
  is a direct child of the epic and carries its own `satisfies: REQ-*` label (the legacy
  removal task is deliberate migration support without one); the A/B/C group headers are
  conceptual only. This assumes a single breakdown pass produces executable leaves rather
  than an intermediate story tier; risk if wrong is a coverage-gate reshuffle, not a
  design change.
