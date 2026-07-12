# Plan: Dependency-aware container artifact archival (7d3a3a47)

> Planning node: 1dbc72f6. Container criteria source: 7d3a3a47 `## Success Criteria`.

This plan builds a pure artifact-plan domain capability and a unified `jit archive`
command family that consumes it. It grounds every claim in
`dev/active/7d3a3a47-investigation.md` (line numbers re-verified against live code) and
carries the owner-approved decisions from `dev/active/7d3a3a47-planning-brief.md` (D-1
through D-11) as binding. The safety guarantee is **referential consistency, not
atomicity** across filesystem files, issue JSON, and the append-only event log.

## 1. Completeness vs criteria

Narrative of the approach per `[hard]` criterion. The criterion→item contract lives once,
in the §3 coverage map.

| Criterion | Approach (how it is met) | Notes / open gap |
|---|---|---|
| REQ-01: deterministic plan of every issue-linked artifact in a container subtree plus every supported embedded local dependency | Resolve the container root through normal storage id semantics, enumerate the root plus its DAG-authoritative transitive dependency closure (`crates/jit/src/commands/graph.rs:206`, `:227`), collect every distinct `DocumentReference.path` on those issues including opaque/binary roots, then recursively discover supported local dependencies (Markdown/HTML element URLs and CSS `url()`/`@import`) with cycle detection and deterministic ordering. Output is one stable, fully enumerated plan object with destinations computed by the D-12 mirror rule. | Opaque roots (CSV/PNG/SVG) are inventoried without an adapter; parsing support gates only embedded-edge discovery, not root eligibility. |
| REQ-02: classify each artifact move/copy/retain/blocked using active references, sharing, managed-path policy, destination conflicts | A pure classifier computes an action per artifact from all reference owners (repository-wide, not directory convention), current lifecycle state, component-aware managed/permanent policy, destination occupancy, and pinned-commit semantics. Ambiguous cases resolve to `block`. | Replaces the two inconsistent directory-heuristic notions of "shared" (`document.rs:1463-1495`, `assets.rs:185-233`) with one repository-wide reference-count classifier. |
| REQ-03: archive an eligible container without losing files, overwriting destinations, or leaving dangling issue references | Execution recomputes the plan under repository write coordination, stages the complete set, validates every supported local edge at its proposed destination, persists reference and event changes, then deletes only sources proven safe. Reuses the current copy-to-temp/finalize/relink/delete sequencing and rollback principles (`document.rs:1637-1787`, `:1391-1435`), generalized through a storage-owned mutation primitive holding the re-entrant write guard (`storage/mod.rs:126`). Finalization is atomic no-replace (D-14), closing the overwrite window against external writers. Guarantee: no failure loses an artifact, overwrites a destination, or leaves a reference relying exclusively on a missing path; unverifiable rollback retains resolvable duplicates and reports manual cleanup. | Only Done/Rejected containers execute (D-8); non-terminal containers preview only. |
| REQ-04: preserve functional relative links for supported bundles (HTML with sibling CSS, theme files, figures) | Recursive discovery follows HTML→CSS→nested figure/font edges and CSS `@import`/`url()`, so a sibling `base.css` and `themes/rust.css` are recognized as bundle dependencies rather than mislabelled shared. The D-12 mirror layout keeps every relative offset between bundle members invariant, and execution validates every moved edge resolves in the proposed layout before metadata commit, fixing the source/destination path-set mismatch in the current verifier (`document.rs:1832-1858`). | Detected JavaScript/runtime loading is reported unsupported, never guessed (D-5). |
| REQ-05: report container-oriented candidates using current terminal state, policy, ownership, and blockers without mutation | A read-only `jit archive candidates` lists terminal containers, each with documentation-policy status, repository-wide artifact ownership, artifact counts, blockers, and move/copy/retain summaries. It consumes the same plan model, always evaluates fully (archival takes no category input, D-21), and performs no filesystem, issue, or event mutation. | No time/retention semantics: eligibility is current terminal state and policy only (D-6); the live REQ-05 wording is already amended to remove `done_at`. |
| REQ-06: structured JSON previews verified against Markdown, HTML, CSS, CSV, PNG, and SVG fixtures | The plan model serializes to a stable JSON envelope; preview is the non-mutating default for both `jit archive document` and `jit archive container`. A representative fixture corpus (Markdown links; HTML→sibling CSS→nested theme image/font; CSS `@import`/`url()`; direct CSV/PNG/SVG roots; permanent shared figures; active outside consumers; identical filenames; missing edges; pinned commits; a dependency cycle) exercises discovery, classification, preview, and execution. | Preview is the default; mutation requires `--execute`, which recomputes and revalidates immediately before mutating (D-7). |

No criterion is silently narrowed or dropped. No missing criterion surfaced during
planning.

## 2. Technical soundness and architectural fit

- **Approach.** Introduce a pure artifact-plan model in the domain/document layers: inputs
  are normalized issue/container records, documentation policy, a path inventory, and
  extracted dependency edges; output is one deterministic, serializable plan carrying, per
  artifact, its normalized source and proposed destination, action (move/copy/retain/block),
  explicit-reference vs embedded-dependency provenance, owners inside and outside the subtree,
  active/terminal/permanent/pinned/missing/conflict evidence, supported/unsupported edges,
  issue-reference changes, and stable machine-readable warnings/blockers. Container traversal
  reuses DAG-authoritative closure, not label membership.
  **Destination layout (D-12):** the destination root is `<archive_root>/` plus, for
  container targets, `<container.id>/` (the full UUID — short-id prefixes can collide
  across containers, full UUIDs cannot, matching `.jit/issues/<uuid>.json` naming); every
  moved or copied artifact lands at that root plus its repository-relative source path. Archival takes no category input (D-21):
  the mirror rule alone determines every destination.
  Mirroring repository-relative paths under one common prefix keeps every relative offset
  between bundle members invariant, keeps identical filenames from different directories
  distinct, and makes intra-plan destination collisions impossible (two artifacts collide only
  if they share a repository path, which cannot occur).
  **Ownership universe (D-13):** repository-wide ownership is computed over every
  `DocumentReference` across all issues plus the recursive supported embedded closure of each
  (Markdown/HTML/CSS edges); an artifact referenced directly or through an embedded edge from
  outside the selected subtree is outside-owned and never moved (D-2). Files unreachable from
  any issue reference are outside JIT's referential contract: they are reported as
  informational not-selected entries when they sit beside bundle members (D-10) but are never
  silently relocated and never counted as owners.
  **Edge-aware action calculus (D-16):** the action per artifact derives from two computed
  constraints. *Needs-destination* holds when the artifact is a selected explicit root, or a
  relocated (moved or copied) parent references it through a **relative** edge — the mirror
  layout preserves that edge only if the dependency exists at its mirrored path.
  *Needs-source* holds when the artifact has an owner outside the subtree, an active owner, a
  pinned reference, lies on a permanent path, or any document that stays in place (or resolves
  root-relatively from anywhere — root-relative edges resolve from the repository root
  regardless of the referencing document's location, `assets.rs:151-162`) references it.
  Then: move = needs-destination ∧ ¬needs-source; copy = needs-destination ∧ needs-source;
  retain = ¬needs-destination ∧ needs-source (and for unselected artifacts with no constraint);
  block = destination conflict, detected dynamic loading in a relocated parent, repository
  escape, or any edge whose resolution the layout cannot preserve. Copy satisfies both
  constraint kinds, so the calculus is total and deterministic; execution's before/after
  validation then checks every supported edge in the final layout: relative edges at the
  mirror, root-relative edges at the repository root.

- **Archive-plan JSON schema and blocker taxonomy (binding for the plan-model task):**
  - **Envelope:** `schema_version` (starts at 1; codes are append-only and never change
    meaning), `target` (`{"kind": "container"|"document", "id"|"path": …}`),
    `destination_root`, `operation_id` (hash of the stable operation identity — target,
    destination root, and the sorted canonical source-path set, where an
    already-archived artifact contributes its inverse-mapped original source — invariant
    across retries of one operation yet distinct for a later operation over a different
    artifact set; D-19), `operation_instance_id` (UUID delimiting one logical operation:
    a mutating execution **adopts** the instance id of the latest event bearing the same
    definition key when current state still shows that operation unconverged — outstanding
    relinks or pending deletions attributable to it — and **mints a fresh one** when the
    prior operation converged, so retries share an instance while a later independent
    re-archival of the identical set starts a new one; null in non-mutating envelopes;
    D-19), `execution_id` (fresh UUID minted only when a mutating execution
    begins and recorded in its event and `--execute` output; **null in every non-mutating
    envelope** — previews and candidate reports, which execute nothing; D-19),
    `plan_fingerprint`
    (hash of the canonical plan serialization excluding volatile fields — staleness detection
    for D-7 revalidation, expected to change as repository state changes), `eligible` (bool),
    `policy_status` (`configured`|`unconfigured`), `counts` (one integer per action plus
    `already_archived`), `artifact_count` + `artifacts` (the repository list-envelope
    convention; entries deterministically ordered by normalized source path), and plan-level
    `blockers`/`warnings` (each sorted by code, then path).
  - **Artifact entry:** `source`, `destination` (null when retained), `action`
    (`move|copy|retain|block`), `already_archived` (bool), `provenance`
    (a flag set — `["explicit"]`, `["embedded"]`, or both when an artifact is issue-linked
    and embedded), `format`, `owners` (issue id, state, `inside_subtree`, `pinned`),
    `edges` (`supported` / `unsupported` / `external`, each with resolution mode),
    `reference_changes` (issue, from-path, to-path), `pending_deletions` (source paths
    this plan will remove — the ordinary source of a `move`, or a residue left by an
    earlier partial execution, each removed only under D-17 identity verification; this
    makes residue cleanup an executable plan operation, not an out-of-band effect), and
    per-artifact `evidence`, `blockers`, `warnings`. The envelope `counts` include
    `pending_deletions`.
  - **Blocker codes (stable kebab-case):** `policy-unconfigured`, `unmanaged-path`,
    `permanent-path`, `destination-conflict`, `outside-owner-conflict`, `active-owner`,
    `unsupported-dynamic-edge`, `repository-escape`, `unresolvable-edge`,
    `non-terminal-target`, `stale-plan`, `missing-source`, `symlink-artifact`.
  - **Warning codes:** `missing-edge-target`, `external-edge`, `no-owner`,
    `residue-source`, `deletion-failed`, `not-selected-sibling`.
  - **Missing-artifact semantics (evaluation order is normative):** each explicit root is
    evaluated in this order. (1) **Already-archived recognition:** a reference whose path
    resolves under the destination root, or whose stated source path maps through the D-12
    mirror to existing content, classifies `already_archived` — so a root moved by a prior
    partial execution can never read as missing on retry, and cleanup of its residues
    proceeds. (2) **`missing-source`:** only a root absent at both its stated location and
    its mirror image blocks — a genuinely dangling issue reference that must be resolved (or
    removed) before the operation executes, never silently entrenched or skipped. A missing
    **embedded** target gets warning `missing-edge-target` on the referencing artifact: it
    contributes no needs-destination constraint, is excluded from before/after edge
    validation (the edge resolves nowhere before archival, so no regression is possible),
    and never blocks.
  - **Candidate evaluation context:** `jit archive candidates` produces the same envelope
    per candidate with every check evaluated — archival takes no category input (D-21), so
    destinations are always computable and nothing is deferred or guessed.
  - **Symlink semantics (D-22):** an explicit root or embedded target that is a symbolic
    link, or whose repository-relative path traverses one, classifies as `block` with
    blocker `symlink-artifact`; the mutation primitive resolves paths physically and
    verifies the resolved target stays inside the repository before staging, and never
    moves, copies, or deletes through a link. Richer symlink relocation is a follow-up.
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
    silently: it blocks until the dependency situation is resolved by hand. Constructs
    outside the set are outside the contract; the set is append-only under
    `schema_version`. The command layer loads inputs,
  calls the pure planner, renders preview, and executes an accepted plan; it does not
  duplicate decision logic between preview and execution. A storage-owned artifact mutation
  primitive centralizes staging, containment, no-replace finalization (D-14), and collision
  semantics so the
  command layer stops issuing raw `std::fs` calls (`document.rs:1637-1787`). Candidates,
  preview, and execution are all consumers of the one plan model (formal-planning obligation 5).

- **Reuses / integrates with:**
  - DAG closure: `crates/jit/src/commands/graph.rs:206` (`resolve_hierarchy_tree`), `:227`
    (`get_transitive_dependencies`).
  - Storage read/write: `crates/jit/src/storage/mod.rs:531` (`read_path_text`), `:498`
    (`read_path_bytes`, enables opaque binary roots without an adapter), `:126`
    (`acquire_repo_write_lock`, re-entrant `RepoWriteGuard` for the multi-write sequence).
  - Sequencing/rollback principles and occupied-destination no-op:
    `crates/jit/src/commands/document.rs:1637-1787`, `:1670-1683`, `:1391-1435`, with
    failure-injection coverage in `crates/jit/tests/archive_integrity_tests.rs:203-396`.
  - Domain records: `crates/jit/src/domain/types.rs:877` (`DocumentReference`; `path` :879,
    `commit` :881, `assets` :891), `:50` (`State::is_terminal` = Done|Rejected only), `:556`
    (`done_at`), `:1466` (`Event::DocumentArchived`).
  - Adapters/scanner: `crates/jit/src/document/adapter.rs` (Markdown/HTML built-ins),
    `crates/jit/src/document/assets.rs:16` (`Asset`, `resolved_path` :20, `is_shared` :28).

- **Grounding (from investigation, re-verified):**
  - Container/subtree closure primitive exists → **valid-and-open**: archive accepts only one
    path (`cli.rs:1950-1969`); no container consumer.
  - Referentially-consistent single-doc sequence → **already-done**: copy/relink/event/delete
    with occupied-destination no-op, failure-injection tested
    (`document.rs:1043-1082`, `archive_integrity_tests.rs:74-396`). Reuse principles; do not
    wrap `archive_document`.
  - Dry-run incompleteness → **valid-and-open**: returns before active-reference and
    destination-occupancy checks (`document.rs:1153-1176`, `:1670-1683`).
  - Directory-heuristic sharing → **valid-and-open**: `is_shared` ignored by archive; movability
    is folder placement (`document.rs:1463-1495`); the reference-count classifier has no archive
    consumer (`assets.rs:185-233`).
  - Shallow one-level discovery → **valid-and-open**: HTML sees only `src`/`href`, no recursion
    (`document/adapter.rs:101-146`).
  - Opaque roots blocked → **valid-and-open**: asset scan requires a text adapter; only
    Markdown/HTML registered (`document/assets.rs:92-111`, `adapter.rs:177-209`).
  - Pinned relink unsafe → **valid-and-open**: relink changes only `path`; reads prefer the
    pinned `commit`, where the new path normally does not exist (`document.rs:1352-1375`,
    `:349-357`, `types.rs:877-891`). Resolve per D-3: leave pinned refs historical, copy source.
  - Post-archive verifier defect → **valid-and-open**: compares destination-resolved against
    source-resolved paths, so the check is normally skipped (`document.rs:1832-1858`).
  - Coordination too narrow → **valid-and-open**: no write guard held across plan/copy/saves/event
    (`document.rs`); guard available (`storage/mod.rs:126`).
  - String-prefix policy matching → **valid-and-open**: `starts_with` matches `dev/active-other`
    against `dev/active` (`document.rs:1243-1267`); use component-aware normalization.
  - Atomic batch execution → **invalid as stated**: true cross-surface atomicity is unavailable;
    the contract is referential consistency (`document.rs:1050-1074`). Preserve; do not strengthen.

Layer boundaries (per AGENTS.md, obligation 3): artifact-graph construction and
action classification are pure domain/document logic, free of I/O; storage owns all
persistence and the mutation primitive; commands orchestrate; CLI/output stay user-facing.
The plan model is emitted as a versionable list envelope with stable ordering
(`@/inv/single-source-prose`, `@/inv/atomic-writes`, `@/inv/event-log` are respected by
reusing storage atomic writes and event append).

## 3. Decomposition sketch (near-ready; jit-breakdown instantiates — no issues created here)

Three conceptual story-sized groups organize the work; the epic's children are the
task-tier items listed below, each independently landable and green at every boundary.
Ordering is expressed only through `depends-on`. Group A is the pure foundation; Groups B
and C consume it (each entry task `depends-on` the Group A item it actually needs: the
mutation primitive on the plan model, preview and candidates on the classifier) and can
proceed independently after A. The plan schema and blocker taxonomy land first, before any CLI
fan-out (obligation 4). Coverage is enforced at the task tier: each task carries its
`satisfies: REQ-*` label directly, and the group headers are organizational only, not issues.

### Group A: Artifact plan model and resolver — covers REQ-01, REQ-02, REQ-04

- **Artifact plan model and blocker taxonomy**  `type: task`  `satisfies: REQ-01`  `depends-on: —`
  Outcome: a serializable artifact-plan type implementing the §2 archive-plan JSON schema and
  blocker/warning taxonomy verbatim (envelope, artifact entry, and code sets are binding),
  emitted as a deterministically ordered JSON envelope carrying the operation id and plan
  fingerprint.
  Own criteria: `[hard] LOCAL-01: Serializes a plan with stable artifact and blocker ordering
  independent of input order.` `[hard] LOCAL-02: The same plan object is the input to both
  preview rendering and execution.`
  Blast radius: self-contained new module; no existing consumer changes.

- **Container closure and explicit-root inventory**  `type: task`  `satisfies: REQ-01`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: resolving a container yields the root plus its DAG transitive dependency closure and
  every distinct issue-linked artifact path on those issues, including opaque/binary roots, each
  annotated with explicit-reference provenance and all owning issues inside and outside the
  selected subtree.
  Own criteria: `[hard] LOCAL-03: Inventories every distinct DocumentReference path in the
  closure, including CSV/PNG/SVG roots with no registered adapter.` `[hard] LOCAL-04: Records
  every direct issue-reference owner of each artifact across the whole repository, flagging
  owners outside the subtree.` (The embedded half of the D-13 ownership universe lands in the
  classifier, which is ordered after discovery.)
  Blast radius: self-contained; reuses `graph.rs:206`/`:227` and `storage/mod.rs:498`.

- **Recursive supported-dependency discovery**  `type: task`  `satisfies: REQ-01, REQ-04`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: recursive discovery of supported local dependencies — Markdown/HTML element URLs and
  CSS `@import`/`url()` — with cycle detection, normalized component-aware path resolution,
  before/after edge reachability, repository-escape rejection, and detected JavaScript/runtime
  loading reported as an unsupported edge rather than guessed.
  Own criteria: `[hard] LOCAL-05: Discovers an HTML→sibling-CSS→nested-figure chain and CSS
  @import/url() targets to full depth, treating script-element src URLs as static supported
  edges.` `[hard] LOCAL-06: Terminates on dependency cycles and reports runtime-loading
  constructs from the binding detection set (§2 dynamic-loading contract) as unsupported
  edges, by textual pattern match only.`
  Blast radius: extends the document adapter/scanner surface; existing Markdown/HTML callers
  (snapshot export) are unaffected because discovery is a new recursive path.

- **Move/copy/retain/block classification**  `type: task`  `satisfies: REQ-02, REQ-04`  `depends-on: Container closure and explicit-root inventory, Recursive supported-dependency discovery`
  Outcome: a pure classifier assigning each artifact move, copy, retain, or block through the
  D-16 edge-aware calculus: needs-destination (selected root, or relative edge from a relocated
  parent) crossed with needs-source (outside owner, active owner, pinned reference, permanent
  path, or inbound root-relative/staying-document edge), with conflicts and unpreservable edges
  blocking. The classifier completes the D-13 ownership universe: it extends the inventory's
  direct owners with the supported embedded closure computed by running discovery over every
  issue-linked document repository-wide — it depends on both the inventory and discovery tasks,
  so both inputs exist when it runs.
  Own criteria: `[hard] LOCAL-27: Ownership incorporates supported embedded references
  reachable from any issue-linked document repository-wide, flagging outside owners (D-13).`
  `[hard] LOCAL-07: Classifies deterministically per the edge-aware calculus — a
  figure shared by a terminal and an active issue copies when a relocated parent references it
  relatively and retains otherwise, never force-relinking the active consumer; a pinned or
  root-relative-referenced artifact always keeps its source.` `[hard] LOCAL-24: Every supported
  edge's resolution mode (relative vs root-relative) drives the decision, and the classifier
  never emits a layout in which a relocated parent's supported edge lacks its target.`
  `[hard] LOCAL-08: Uses path-component containment for managed/permanent matching, so
  dev/active-other does not match dev/active.`
  `[hard] LOCAL-19: Classifies an artifact whose proposed destination is occupied by differing
  content as blocked, citing the conflicting path in the blocker evidence; a content-identical
  occupied destination classifies as already archived.` `[hard] LOCAL-32: Classifies a symlink
  root or embedded target, or a path traversing one, as blocked with the symlink-artifact
  code (D-22).`
  `[hard] LOCAL-21: Proposes destinations by mirroring each artifact's repository-relative
  source path beneath the target's destination root, so identical filenames from different
  directories stay distinct and every relative offset between bundle members is preserved.`
  Blast radius: self-contained; supersedes the archive-time directory heuristic without touching
  the legacy command until Group B.

### Group B: Unified archive CLI and safe executor — covers REQ-03, REQ-04, REQ-06

- **Storage-owned artifact mutation primitive**  `type: task`  `satisfies: REQ-03`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: a reusable storage primitive that stages a full artifact set to temp, finalizes by
  atomic no-replace finalization (hard-link-then-unlink, D-14), relinks references, and deletes
  sources last, holding the re-entrant repository
  write guard across the sequence and centralizing containment, atomic-write, and collision
  semantics, testable through tempdir-backed storage (the in-memory backend performs no virtual
  file I/O, so filesystem assertions run against `JsonFileStorage` at a tempdir).
  Finalization uses an atomic no-replace primitive — hard-link from the same-filesystem staging
  directory (`.jit/tmp` lives inside the repository) then unlink the staged copy, which fails
  with `AlreadyExists` instead of clobbering a destination created after the plan's occupancy
  check — never check-then-`rename`, whose overwrite window the repository write guard cannot
  close against external filesystem writers (D-14). Containment checks resolve paths
  physically before staging, the primitive never moves, copies, or deletes through a
  symbolic link (D-22), and identity verification runs on the opened handle with
  handle-anchored directory traversal used wherever the platform provides it. The binding
  concurrency contract is explicit (D-24): REQ-03's guarantee holds under concurrent JIT
  writers (the guard) and non-adversarial external modification (detected by no-replace
  finalization and identity-verified deletion); adversarial substitution of path components
  between check and operation is a documented non-goal, not a silently assumed impossibility.
  Own criteria: `[hard] LOCAL-09: Rejects an occupied destination with the pre-existing file
  preserved byte-for-byte.` `[hard] LOCAL-10: Holds one write guard across staging, reference
  saves, and event append.` `[hard] LOCAL-22: A destination created by an external writer
  between planning and finalization fails that artifact's finalization without overwriting the
  foreign file, verified by a race-focused test.` `[hard] LOCAL-25: Deletes a source or rolls
  back a finalized destination only after re-verifying the file's recorded content identity
  (hash and size captured at staging) on the opened handle; on mismatch the file is left in
  place and reported for manual cleanup.` `[hard] LOCAL-33: Documents and tests the D-24
  concurrency contract boundary — the non-adversarial detection cases are test-covered, and
  no code comment, doc, or output claims protection against adversarial path substitution.`
  Blast radius: self-contained new storage API; the legacy command keeps its inline `std::fs`
  path until it is removed.

- **Unified archive preview surface**  `type: task`  `satisfies: REQ-06`  `depends-on: Move/copy/retain/block classification`
  Outcome: `jit archive document <path>` and `jit archive container <id>` produce a complete
  non-mutating plan by default, running every precondition execution relies on (source
  availability, normalized policy, terminal eligibility, outside owners, pinned references,
  destination conflicts, before/after reachability), rendered as both a stable JSON envelope and
  human output, and verified against the Markdown/HTML/CSS/CSV/PNG/SVG fixture corpus.
  Own criteria: `[hard] LOCAL-11: Preview enumerates every artifact, action, evidence flag, and
  blocker and mutates nothing.` `[hard] LOCAL-12: Document and container targets produce the same
  plan schema from the shared planner.` `[hard] LOCAL-20: In a repository without configured
  documentation policy, preview returns a non-mutating inventory explaining that archival is
  disabled, and execution is refused.`
  Blast radius: adds a new `archive` command group alongside the existing `doc archive`; both
  coexist until the removal task. MCP tools regenerate from the CLI schema automatically.

- **Coordinated plan execution**  `type: task`  `satisfies: REQ-03, REQ-04`  `depends-on: Storage-owned artifact mutation primitive, Unified archive preview surface`
  Outcome: `--execute` recomputes the plan under repository coordination, stages the complete
  set, validates every supported local edge at its proposed destination before metadata commit,
  persists reference changes and the archive event(s), refreshes or invalidates cached asset
  metadata on moved references, leaves pinned references at their historical path (copying the
  source when needed), and removes only sources proven safe, retaining resolvable duplicates with
  a manual-cleanup report when rollback cannot be verified. Only Done/Rejected containers execute;
  a document target executes only when every direct or embedded-closure owner of the document and
  of every bundle artifact is terminal, active owners block execution (preview still reports), and
  a zero-owner document in a managed path executes with an informational no-owner note (D-15).
  Retry converges by recomputation (D-18): rerunning `--execute` after any partial failure
  recomputes the plan against current state; an occupied destination whose content identity
  equals the planned artifact counts as already archived (not a blocker), references already
  pointing at their destinations are satisfied, only the remaining mutations are applied, and
  an execution that mutates nothing appends no event, so no duplicate destinations, references,
  or events arise. Destinations derive from repository-relative source paths (D-12), so a retry
  can never nest an archived bundle under itself. The D-12 mapping is invertible, which closes
  the post-commit deletion-failure case: for every already-archived artifact, recomputation
  derives the original source path from the destination, and a still-existing, content-identical
  source (`residue-source` warning) is completed as an identity-verified deletion (D-17) even
  though no reference points at it anymore. Event recording uses two kinds so the record
  stays truthful under the deletion-last principle (D-19). An execution that finalizes or
  relinks appends **`ArchiveExecuted`** after those mutations persist and before any source
  deletion, recording the operation-definition key, operation instance id, its fresh
  `execution_id`, the executed plan's fingerprint, the completed finalization/relink
  mutations, and the plan's `pending_deletions`. Deletions then run; a run whose deletions
  (ordinary or residue) actually remove sources appends **`ArchiveSourcesRemoved`** after
  they complete, recording exactly the paths removed — so completed deletions are recorded
  as completed, and a run whose deletions all fail appends nothing and mutates nothing,
  leaving `residue-source` warnings. A crash between deletion and the removal event leaves
  an unrecorded removal of an already-safe-to-delete path; like the crash-window duplicate,
  this is within the referential-consistency contract (`document.rs:1050-1074`) — the
  authoritative record is `ArchiveExecuted` plus observable state. Identity is three-level
  (D-19): the **definition key** (hash of target, destination root, sorted canonical source
  set) is retry-invariant; the **instance id** delimits one logical operation via the
  adopt-or-mint rule (adopt while the prior same-key operation is unconverged, mint fresh
  after convergence), so retries group and independent re-archivals separate; the
  **execution id** is unique per mutating run.
  Own criteria: `[hard] LOCAL-13: Validates each staged local edge in the proposed layout, not
  against source-resolved paths, before commit.` `[hard] LOCAL-23: Refuses document-target
  execution while any owner of the document or its bundle artifacts is non-terminal.`
  `[hard] LOCAL-26: Rerunning execution after a partial staging, relink, event, or deletion
  failure converges — content-identical occupied destinations count as archived, no duplicate
  destination or reference is created, event emission follows the D-19 contract, and the
  remaining mutations complete — verified by failure-injection retry tests.` `[hard] LOCAL-28: After a post-commit deletion
  failure, a rerun rediscovers the orphaned source through the inverse destination mapping and
  completes its identity-verified removal.` `[hard] LOCAL-29: A finalizing or relinking execution appends
  ArchiveExecuted — after relinks, before deletions — carrying the definition key, the
  adopt-or-mint instance id, a unique execution id, the plan fingerprint, completed
  mutations, and pending deletions; completed source removals append ArchiveSourcesRemoved
  after they happen; a rerun that mutates nothing appends no event; retries share an
  instance id while a re-archival after convergence mints a new one.` `[hard] LOCAL-14: A partial-relink, event-append,
  or deletion failure leaves no artifact lost, no destination overwritten, and no reference
  relying solely on a missing path.` `[hard] LOCAL-15: Refuses execution on a non-terminal
  container and on a stale recomputed plan.`
  Blast radius: consumes the mutation primitive and preview planner; no legacy-command change.

- **Remove the legacy document-archive command**  `type: task`  `satisfies: —`  `depends-on: Coordinated plan execution`
  Outcome: `jit doc archive` is removed completely as a clean-cut migration with no alias or
  stub, and all consumers (CLI/main dispatch, `ArchiveResult`, event catalog/schema references,
  integration and unit tests, fixtures, docs, README, and the auto-generated MCP tool set) are
  updated in the same change so the tree builds and tests pass with only the unified surface.
  The legacy `[documentation.categories]` configuration table retires with it (D-21): its
  loader field, docs, and this repository's config entries go in the same change.
  Own criteria: `[hard] LOCAL-16: A tree-wide search for the legacy surface (the strings
  "doc archive", "archive_document", "ArchiveResult") across crates/, mcp-server/, web/,
  docs/, and dev/ returns no live production or test reference; only historical dev/ records
  may mention it.`
  Blast radius: enumerated below in the acceptance-check note; the `crates/server` and `web`
  trees have no live consumer (web matches are the `Archived` issue state, not the command).

### Group C: Container candidate reporting — covers REQ-05

- **Read-only container candidate report**  `type: task`  `satisfies: REQ-05`  `depends-on: Move/copy/retain/block classification`
  Outcome: `jit archive candidates` lists terminal containers using the shared planner —
  container-ness derives from the configured type hierarchy (any type at a non-leaf level of
  `[type_hierarchy]`, D-20), never from hardcoded type names, with membership resolved through
  the DAG — each with
  documentation-policy status, managed/permanent path status, repository-wide artifact ownership,
  artifact counts, outside-subtree/active/pinned/missing/unsupported/conflict blockers, a
  move/copy/retain summary, in both human and JSON form, with no filesystem, issue, or event
  mutation and no age or retention filtering. Every candidate is evaluated fully: archival
  takes no category input (D-21), so the D-12 mirror rule determines each destination and no
  check is deferred or guessed.
  Own criteria: `[hard] LOCAL-17: Lists terminal containers with policy status, ownership, counts,
  and blockers, explaining exclusions rather than omitting them.` `[hard] LOCAL-18: Emits identical
  human and JSON results while mutating nothing, and applies no time/retention filter.`
  `[hard] LOCAL-30: Derives candidate container-ness from the configured type hierarchy's
  non-leaf levels and DAG-authoritative membership, with no hardcoded type names.`
  `[hard] LOCAL-31: Evaluates every candidate fully — including destination conflicts under
  the mirror rule — with no category input, suggestion, or deferred check.`
  Blast radius: self-contained new read-only command consuming Group A.

**Coverage map** (single source for criterion→item; every `[hard]` criterion → ≥1 item):

| Criterion | Satisfied by (item) |
|---|---|
| REQ-01 | Artifact plan model and blocker taxonomy; Container closure and explicit-root inventory; Recursive supported-dependency discovery |
| REQ-02 | Move/copy/retain/block classification |
| REQ-03 | Storage-owned artifact mutation primitive; Coordinated plan execution |
| REQ-04 | Recursive supported-dependency discovery; Move/copy/retain/block classification; Coordinated plan execution |
| REQ-05 | Read-only container candidate report |
| REQ-06 | Unified archive preview surface |

> **Removal acceptance check (D-9, clean cut, no alias).** The removal task "Remove the
> legacy document-archive command" is ordered last in Group B via `depends-on: Coordinated
> plan execution`, so both the old `jit doc archive` and the new `jit archive` family coexist
> through every intermediate wave and no wave breaks a consumer before its replacement exists.
> Acceptance is a tree-wide search across `crates/`, `mcp-server/`, `web/`, `docs/`, and
> `dev/` for `doc archive`, `archive_document`, and `ArchiveResult`, which must return no live
> production or test reference (historical `dev/` records excepted). The current sweep of that
> exact pattern reports **103 matches across 33 files**; the live production Rust consumers are
> `crates/jit/src/{cli.rs, main.rs, commands/document.rs, commands/mod.rs, schema.rs,
> domain/event_catalog.rs}`; the test consumers are
> `crates/jit/tests/{doc_archive_tests.rs, archive_integrity_tests.rs,
> command_exit_code_projection_tests.rs}`; the rest are docs/ and dev/ prose. The MCP tool set
> is auto-generated from the CLI schema, so removing the subcommand drops it on regeneration.
> `crates/server` and `web` carry no live consumer.

## 4. Risks and actionability

| Risk / open question | Severity | Mitigation or decision |
|---|---|---|
| Generalized safe execution weakens the current referential-consistency guarantee | High | Reuse the proven sequencing/rollback (`document.rs:1391-1435`) and its failure-injection suite (`archive_integrity_tests.rs:203-396`); execution task carries partial-failure criteria (LOCAL-14). Contract stays referential consistency, never atomicity. |
| Pinned references silently corrupt provenance if relinked | High | D-3 fixes this: pinned refs stay at their historical path; source is copied when needed. Classifier (LOCAL-07) and executor (D-3) enforce; never rewrite a pinned `commit`. |
| Removing `jit doc archive` breaks docs/tests/MCP mid-flight | Medium | Removal ordered last (`depends-on: Coordinated plan execution`); old and new coexist until then; consumer migration is in the same change; acceptance is the tree-wide search above. |
| Recursive CSS/HTML discovery mis-scopes a shared theme or figure | Medium | Repository-wide reference-count classification (LOCAL-07), not directory heuristic; shared-but-active artifacts copy/retain, never force-relink the active consumer (D-2, D-10). |
| Post-archive verification defect masks broken links | Medium | Executor validates each edge in the proposed layout, not against source-resolved paths (LOCAL-13); replaces `document.rs:1832-1858`. |
| Coordination gap (time-of-check/time-of-use) during multi-write execution | Medium | Storage mutation primitive holds the re-entrant write guard across staging, saves, and event append (LOCAL-10, `storage/mod.rs:126`); execution rejects stale recomputed plans (LOCAL-15). The guard covers only JIT writers, so finalization additionally uses the atomic no-replace primitive (D-14, LOCAL-22) against external filesystem writers. |
| An artifact embedded by an outside document (not directly issue-linked) misclassified as unshared and moved | High | Ownership universe is defined repository-wide over all issue references plus their supported embedded closure (D-13, LOCAL-04); the existing classifier's supplied-map limitation (`assets.rs:189`) is superseded. |
| Document-target execution on a path with mixed-state owners dangles an active reference | Medium | D-15 fixes eligibility: all direct and embedded-closure owners must be terminal; active owners block execution while preview still reports; zero-owner managed-path documents execute with an informational note (LOCAL-23). |
| A retained dependency leaves a relocated parent's relative link without its target | High | D-16's calculus is total: a relative edge from a relocated parent forces needs-destination, so the dependency moves or copies; root-relative edges instead force source retention; the classifier never emits an unpreservable layout (LOCAL-24) and execution re-validates every edge (LOCAL-13). |
| Rollback or source deletion removes a file an external writer replaced | High | D-17: deletions are identity-verified on the opened handle against the hash/size captured at staging; a mismatch leaves the file and reports manual cleanup (LOCAL-25). Adversarial substitution beyond detection is an explicit D-24 non-goal, stated rather than silently assumed away (LOCAL-33). |
| A partial failure leaves an unrecoverable half-archived state on retry | Medium | D-18: retry recomputes and converges — content-identical destinations count as archived, only remaining mutations apply, no duplicate events; orphaned sources are rediscovered through the inverse D-12 mapping (LOCAL-28); covered by failure-injection retry tests (LOCAL-26). |
| Archive events cannot be attributed or deduplicated across retries and crashes | Medium | D-19: three-level identity (definition key, adopt-or-mint instance id, execution id) and two event kinds (ArchiveExecuted before deletions, ArchiveSourcesRemoved after) keep the record truthful; nothing-mutated reruns append nothing; crash-window anomalies stay within the established contract (LOCAL-29). |
| Opaque roots need an adapter to be inventoried | Low | Inventory reads bytes via `storage/mod.rs:498` and treats parser support as edge-discovery-only, not root eligibility (LOCAL-03). |
| Event granularity for a multi-artifact action undefined | Low | Executor emits a container/plan-scoped archive event carrying artifact and reference detail; current single event cannot audit a multi-artifact action (`types.rs:1466`). Decided in the execution task. |

Every load-bearing question is resolved or owned. Each sketch item is executable from its
description plus the cited grounding without re-deriving the design.

## Decisions

Carried from the owner-approved brief (`dev/active/7d3a3a47-planning-brief.md`), binding.
Provisional; none is REOPEN — the investigation supports each.

- **D-1 — Archival is explicit opt-in:** chosen **require configured documentation policy**.
  Rejected: silently use default paths (moves evidence never delegated to JIT).
- **D-2 — Do not relocate another container's artifacts:** chosen **never relocate an
  artifact referenced outside the subtree; copy if required, block if ambiguous**. Rejected:
  global relink; a general force option.
- **D-3 — Pinned references remain historical:** chosen **never rewrite a pinned reference;
  keep its historical source path, copy content into the bundle when needed**. Rejected:
  path-only relink; implicit conversion to working-tree provenance.
- **D-4 — One container-owned destination (amended by owner, 2026-07-13):** chosen
  **destination is a container-owned directory preserving internal topology; the brief's
  category-selection clause is superseded — archival takes no category input (D-21)**.
  Rejected: category-per-artifact placement (fragments bundles); caller-selected categories
  (legacy taxonomy scrapped by the owner in favor of the pure mirror rule).
- **D-5 — Initial recursive discovery is static:** chosen **follow Markdown/HTML/CSS local
  references incl CSS `url()`/`@import`; block on detected local JavaScript/runtime loading**.
  Rejected: guessing dynamic dependencies.
- **D-6 — Time-based archival deferred:** chosen **no retention periods, age filters,
  scheduled sweeps, or lifecycle triggers; candidates use current terminal state and policy**.
  Rejected: first-done, updated, or reconstructed timestamps. (Live REQ-05 already amended.)
- **D-7 — Preview defaults, execution explicit:** chosen **complete non-mutating plan by
  default; mutation requires `--execute`, which recomputes and revalidates before mutating**.
  Rejected: applying stale preview data.
- **D-8 — Only terminal containers execute:** chosen **Done/Rejected may execute; non-terminal
  may preview only, no force override**. Rejected: force execution of active containers.
- **D-9 — One unified archive command family:** chosen **`jit archive document|container|
  candidates` on one planner/schema/policy/coordination/executor; remove `jit doc archive`
  completely as a clean-cut pre-v1.0 migration**. Rejected: retaining an alias or migration stub.
- **D-10 — Reachability defines a bundle:** chosen **explicit issue-linked artifacts plus
  recursively reachable supported static dependencies; unreferenced siblings reported as
  informational, not moved**. Rejected: sweeping an entire source directory implicitly.
- **D-11 — Candidates are container-oriented:** chosen **list terminal containers with policy
  status, artifact counts, blockers, and summaries; individual artifacts are details inside a
  candidate**. Rejected: individual artifacts as independent archival candidates.

Plan-level decisions resolving specification gaps surfaced in review (extend, and do not
override, the brief's D-1..D-11):

- **D-12 — Destination layout mirrors repository-relative paths:** chosen **destination root
  `<archive_root>/` (+ `<container.id>/`, the full UUID, for container targets), each
  artifact at root + repository-relative source path, with no category segment (D-21)**.
  The full UUID is the container segment because short-id prefixes are not unique across
  containers, and destination determinism must not depend on the current id population. Preserves every relative offset under a
  common prefix, keeps identical filenames distinct, and makes intra-plan collisions
  structurally impossible. Rejected: stripping the managed prefix per artifact (the current
  single-doc rule, `document.rs:1284-1319`) — artifacts from different managed roots could
  collide and cross-directory relative links would break; content-addressed layout — destroys
  human-navigable topology.
- **D-13 — Ownership universe is the issue-linked closure:** chosen **all `DocumentReference`s
  across all issues plus each one's recursive supported embedded closure; outside owners
  (direct or embedded) forbid a move**. Files unreachable from any issue reference are outside
  JIT's referential contract: reported informationally near bundles (D-10), never counted as
  owners, never relocated. Rejected: scanning the entire working tree for arbitrary referencing
  files — unbounded, and JIT's guarantee is scoped to issue references.
- **D-14 — Finalization is atomic no-replace:** chosen **hard-link from same-filesystem staging
  then unlink staging; `AlreadyExists` aborts that artifact's finalization and triggers
  rollback**. Rejected: check-then-`rename` — `rename` replaces a destination created after the
  check, and the repository write guard cannot exclude external filesystem writers
  (`storage/mod.rs:90`).
- **D-15 — Document-target execution requires all-terminal owners:** chosen **`jit archive
  document --execute` requires every direct or embedded-closure owner of the document and its
  bundle artifacts to be terminal; active owners block (preview still reports); a zero-owner
  managed-path document executes with an informational note**. Rejected: selecting an owning
  container implicitly (ambiguous with several owners); a force override (mirrors the rejected
  D-8 force path).
- **D-16 — Edge-aware action calculus:** chosen **derive each action from needs-destination
  (selected root, or relative edge from a relocated parent) × needs-source (outside owner,
  active owner, pinned reference, permanent path, or inbound root-relative/staying-document
  edge): move / copy / retain, with conflicts and unpreservable edges blocking**. Copy creates
  the mirrored destination and retains the source, satisfying both constraints, so every
  supported edge — relative at the mirror, root-relative at the repository root — resolves in
  the final layout. Rejected: a free copy-or-retain choice (leaves a relocated parent's
  relative edge without its target); treating root-relative edges like relative ones (their
  resolution ignores the referencing document's location, `assets.rs:151-162`).
- **D-17 — Deletions are identity-verified:** chosen **capture each artifact's content hash
  and size at staging; delete a source or roll back a finalized destination only after the
  file on disk matches; mismatch leaves the file and reports manual cleanup**. Closes the
  cleanup half of the external-writer race that D-14 closes for finalization (the current
  rollback deletes destination paths unverified, `document.rs:1735-1739`, `:1777-1787`).
  Rejected: unverified cleanup (can delete a foreign file); global filesystem locking
  (unavailable against arbitrary external writers).
- **D-18 — Retry converges by recomputation:** chosen **a rerun after any partial failure
  recomputes the plan against current state, treats content-identical occupied destinations as
  already archived, applies only the remaining mutations, and appends no event when nothing
  mutates**. Repository-relative destination derivation (D-12) makes nesting impossible on
  retry. Rejected: persisted resumable journals (adds state that can itself go stale);
  treating every occupied destination as a blocker (would make every partial failure
  permanent). The invertible D-12 mapping additionally lets a rerun derive the original
  source path of every already-archived artifact, so an orphaned source left by a
  post-commit deletion failure is rediscovered and removed under D-17 identity verification
  even though no reference points at it any longer.
- **D-19 — Two identities: retry-stable operation id, state-dependent plan fingerprint:**
  chosen **three-level identity plus two event kinds**. `operation_id` = hash of (target,
  destination root, sorted canonical source-path set — already-archived artifacts contribute
  their inverse-mapped original sources): the retry-invariant **definition key**.
  `operation_instance_id` delimits one logical operation by **adopt-or-mint**: a mutating
  execution adopts the instance id of the latest same-key event while current state shows
  that operation unconverged (outstanding relinks or pending deletions), and mints a fresh
  UUID once it converged — retries group, independent re-archivals of an identical set
  separate, and no persistent state beyond the existing event log is needed because
  convergence is recomputable (D-18). `execution_id` is unique per mutating run; both
  UUIDs are null in non-mutating envelopes. **`ArchiveExecuted`** appends after
  finalization/relinks and before deletions, recording completed mutations and pending
  deletions; **`ArchiveSourcesRemoved`** appends after source removals complete, recording
  exactly the removed paths; a run whose deletions all fail appends nothing. A crash can
  leave a duplicate `ArchiveExecuted` or an unrecorded removal of an already-safe path —
  both within the established referential-consistency contract (`document.rs:1050-1074`);
  `plan_fingerprint` = hash of the canonical plan serialization excluding volatile fields,
  used only for D-7 staleness detection and expected to change whenever repository state
  changes**. Every mutating execution appends one archive event recording both plus the
  mutations actually performed; a rerun that mutates nothing appends nothing; a crash-window
  duplicate for one operation id remains possible and is exactly the reported-duplicate case
  the established referential-consistency contract already tolerates
  (`document.rs:1050-1074`). Rejected: the plan fingerprint as retry identity (recomputation
  changes it, so retries would not correlate); literal exactly-once event semantics
  (unachievable across a crash between mutation and the append-only event write);
  per-artifact events without an operation id (cannot audit a multi-artifact action).
- **D-20 — Container-ness derives from the type hierarchy:** chosen **a candidate container
  is a terminal issue whose configured type sits at a non-leaf level of `[type_hierarchy]`,
  with membership resolved DAG-authoritatively**. Engine code names no type; the boundary
  comes from repository configuration (`@/inv/domain-agnostic`). Rejected: hardcoding
  strategic type names (epic/milestone); treating every issue with DAG children as a
  container (an incidental dependency fan-in is not a container).
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
- **D-24 — The concurrency contract is explicit and non-adversarial:** chosen **REQ-03's
  guarantee binds under concurrent JIT writers (repository write guard) and non-adversarial
  external modification, which the design detects rather than prevents: no-replace
  finalization (D-14), identity verification on opened handles before any deletion (D-17),
  and handle-anchored directory traversal wherever the platform provides it. Adversarial
  races — substituting a parent path component with a symlink, or replacing a file with
  same-content different identity between check and operation — are a documented non-goal.**
  Rejected: claiming immunity to adversarial filesystem racing (unattainable with a
  path-based repository on a shared filesystem and would overstate REQ-03); global
  filesystem locking (unavailable against arbitrary external writers). The safety posture
  matches the epic's contract: referential consistency against accidents, not a security
  boundary against adversaries.
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
- **Assumptions:** Coverage is enforced at the task tier: each of the nine task-tier
  items is a direct child of the epic and carries its own `satisfies: REQ-*` label; the
  A/B/C group headers are conceptual only. This assumes a single breakdown pass produces
  executable leaves rather than an intermediate story tier; risk if wrong is a coverage-gate
  reshuffle, not a design change.
