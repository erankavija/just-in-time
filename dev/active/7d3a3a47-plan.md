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
| REQ-01: deterministic plan of every issue-linked artifact in a container subtree plus every supported embedded local dependency | Resolve the container root through normal storage id semantics, enumerate the root plus its **resolved-hierarchy descendants** (the DAG-authoritative membership relation, `graph/hierarchy.rs:458`, `:321` — not the raw dependency closure, which absorbs cross-container sequencing edges; D-25), collect every distinct `DocumentReference` on those issues as **(path, version)** artifacts (D-26) including opaque/binary roots, then recursively discover supported local dependencies (Markdown/HTML element URLs and CSS `url()`/`@import`) with cycle detection and deterministic ordering. Output is one stable, fully enumerated plan object with destinations computed by the D-12 mirror rule. | Opaque roots (CSV/PNG/SVG) are inventoried without an adapter; parsing support gates only embedded-edge discovery, not root eligibility. Pinned versions are enumerated as non-relocating historical entries (D-26). |
| REQ-02: classify each artifact move/copy/retain/blocked using active references, sharing, managed-path policy, destination conflicts | A pure classifier computes an action per working-tree artifact through the D-16 edge-aware calculus from all reference owners (repository-wide, direct and embedded; D-13), current lifecycle state, component-aware managed/permanent policy, destination occupancy, and pinned semantics (D-26). Ambiguous or unpreservable cases resolve to `block`. | Replaces the two inconsistent directory-heuristic notions of "shared" (`document.rs:1463-1495`, `assets.rs:185-233`) with one repository-wide reference analysis. |
| REQ-03: archive an eligible container without losing files, overwriting destinations, or leaving dangling issue references | Execution recomputes the plan under repository write coordination, stages the complete set, validates every supported local edge at its proposed destination, persists reference and event changes, then removes sources through the **quarantine protocol** (D-17): a directory-handle-anchored rename/verify/unlink sequence in which nothing is unlinked before its bound inode is verified on an open handle. JIT never overwrites (D-14 no-replace finalization), and its removals add no destructive capability an interfering writer does not already possess (D-24 — the provable bound on a path-based filesystem). Reuses the proven sequencing and rollback principles (`document.rs:1637-1787`, `:1391-1435`) behind a storage-owned mutation primitive holding the re-entrant write guard (`storage/mod.rs:126`). | Only Done/Rejected containers execute (D-8); document targets follow D-15; non-terminal targets preview only. |
| REQ-04: preserve functional relative links for supported bundles (HTML with sibling CSS, theme files, figures) | Recursive discovery follows HTML→CSS→nested figure/font edges and CSS `@import`/`url()`, so a sibling `base.css` and `themes/rust.css` are recognized as bundle dependencies rather than mislabelled shared. The D-12 mirror layout keeps every relative offset between bundle members invariant, the D-16 calculus guarantees every relative edge from a relocated parent has its target at the mirror, and execution validates every supported edge in the proposed layout before metadata commit, fixing the source/destination path-set mismatch in the current verifier (`document.rs:1832-1858`). | Detected JavaScript/runtime loading blocks per the D-23 enumerated contract, never guessed (D-5). |
| REQ-05: report container-oriented candidates using current terminal state, policy, ownership, and blockers without mutation | A read-only `jit archive candidates` lists terminal containers (container-ness from the configured type hierarchy, D-20; membership per D-25), each with documentation-policy status, repository-wide artifact ownership, artifact counts, blockers, and move/copy/retain summaries. It consumes the same plan model, always evaluates fully (archival takes no category input, D-21), and performs no filesystem, issue, or event mutation. | No time/retention semantics: eligibility is current terminal state and policy only (D-6); the live REQ-05 wording is already amended to remove `done_at`. |
| REQ-06: structured JSON previews verified against Markdown, HTML, CSS, CSV, PNG, and SVG fixtures | The plan model serializes to the binding §2 JSON schema; preview is the non-mutating default for both `jit archive document` and `jit archive container`. A representative fixture corpus (Markdown links; HTML→sibling CSS→nested theme image/font; CSS `@import`/`url()`; direct CSV/PNG/SVG roots; permanent shared figures; active outside consumers; identical filenames; missing edges; pinned commits; a dependency cycle) exercises discovery, classification, preview, and execution. | Preview is the default; mutation requires `--execute`, which recomputes and revalidates immediately before mutating (D-7). |

No criterion is silently narrowed or dropped. No missing criterion surfaced during
planning.

## 2. Technical soundness and architectural fit

- **Approach.** Introduce a pure artifact-plan model in the domain/document layers: inputs
  are normalized issue/container records, documentation policy, a path inventory, and
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
  `working-tree` or a pinned commit. Working-tree versions are the archival subjects. A
  pinned reference resolves through the storage layer at its commit
  (`document.rs:349-357` — effective commit precedes working-tree; `storage/mod.rs:531`
  `read_path_text(path, commit)`), i.e. from git history: relocating or deleting the
  working-tree file does not break a git-resolvable pinned read, and per D-3 pinned
  references are never rewritten. Therefore a pinned reference imposes **no** working-tree
  retention constraint when its commit is reachable in git; where the repository lacks git
  or the commit is unreachable, the reference degrades to working-tree resolution and then
  and planning then imposes needs-source (and never-relink) on the working-tree file as the
  conservative constraint, with warning `pinned-unreachable`; **no read fallback is claimed
  or implemented** — an unreachable commit surfaces from storage as `CommitNotFound` today,
  and this plan does not change read semantics. Pinned versions appear in the plan as
  informational non-relocating entries with evidence `pinned-historical`, **including their
  commit-specific supported dependency closures**: the adapters are pure text extractors,
  so discovery runs them over commit-resolved content (`read_path_text(path, commit)`,
  `storage/mod.rs:531`) exactly as over working-tree content, producing (path, commit)
  dependency entries that are likewise informational and non-relocating — REQ-01's
  "every supported embedded local dependency" is met for pinned roots deterministically,
  with nothing to move because history serves every pinned reader.

- **Destination layout (D-12).** The destination root is `<archive_root>/` plus, for
  container targets, `<container.id>/` (the full UUID — short-id prefixes can collide across
  containers, full UUIDs cannot, matching `.jit/issues/<uuid>.json` naming); every moved or
  copied artifact lands at that root plus its repository-relative source path. Archival
  takes no category input (D-21): the mirror rule alone determines every destination.
  Mirroring under one common prefix keeps every relative offset between bundle members
  invariant, keeps identical filenames from different directories distinct, and makes
  intra-plan destination collisions impossible. The mapping is invertible, which retry
  convergence exploits (D-18).

- **Ownership universe (D-13).** Repository-wide ownership is computed over every
  `DocumentReference` across all issues plus the recursive supported embedded closure of
  each (Markdown/HTML/CSS edges); an artifact referenced directly or through an embedded
  edge from outside the selected subtree is outside-owned and never moved (D-2). Files
  unreachable from any issue reference are outside JIT's referential contract: reported as
  informational not-selected entries when they sit beside bundle members (D-10), never
  silently relocated, never counted as owners. The inventory task records direct owners; the
  classifier — ordered after discovery — completes the embedded half.

- **Edge-aware action calculus (D-16).** The action per working-tree artifact derives from
  two computed constraints. *Needs-destination* holds when the artifact is a selected
  explicit root, or a relocated (moved or copied) parent references it through a
  **relative** edge — the mirror layout preserves that edge only if the dependency exists at
  its mirrored path. *Needs-source* holds when the artifact has an owner outside the
  subtree, an active owner, a git-unresolvable pinned reference (D-26), lies on a permanent
  path, or any document that stays in place references it — including **root-relative**
  edges, which resolve from the repository root regardless of the referencing document's
  location (`assets.rs:151-162`). Then: move = needs-destination ∧ ¬needs-source; copy =
  needs-destination ∧ needs-source; retain = ¬needs-destination ∧ needs-source (and for
  unselected artifacts); block = destination conflict, detected dynamic loading in a
  relocated member (D-23), symlink involvement (D-22), repository escape, or any edge whose
  resolution the layout cannot preserve. Copy satisfies both constraint kinds, so the
  calculus is total and deterministic; execution's before/after validation then checks every
  supported edge in the final layout: relative edges at the mirror, root-relative edges at
  the repository root.

- **Archive-plan JSON schema and blocker taxonomy (binding for the plan-model task):**
  - **Envelope:** `schema_version` (starts at 1; codes are append-only and never change
    meaning), `target` (`{"kind": "container"|"document", "id"|"path": …}`),
    `destination_root`, `operation_id` (retry-invariant **definition key**: hash of target,
    destination root, and the sorted canonical source-path set, where an already-archived
    artifact contributes its inverse-mapped original source; D-19),
    `operation_instance_id` (UUID delimiting one logical operation by the D-19
    adopt-or-mint rule; null in non-mutating envelopes), `execution_id` (fresh UUID minted
    only when a mutating execution begins, recorded in its event and `--execute` output;
    **null in every non-mutating envelope** — previews and candidate reports),
    `plan_fingerprint` (hash of the canonical plan serialization excluding volatile
    fields — staleness detection for D-7 revalidation, expected to change as repository
    state changes), `eligible` (bool), `policy_status` (`configured`|`unconfigured`),
    `counts` (one integer per action plus `already_archived` and `pending_deletions`),
    `artifact_count` + `artifacts` (the repository list-envelope convention; entries
    deterministically ordered by normalized source path, then version), and plan-level
    `blockers`/`warnings` (each sorted by code, then path).
  - **Artifact entry:** `source`, `version` (`working-tree` or a commit id; D-26),
    `destination` (null when retained or historical), `action` (`move|copy|retain|block`),
    `already_archived` (bool), `provenance` (a flag set — `["explicit"]`, `["embedded"]`,
    or both), `format`, `owners` (issue id, state, `inside_subtree`, `pinned`), `edges`
    (`supported` / `unsupported` / `external`, each with resolution mode),
    `reference_changes` (issue, from-path, to-path), `pending_deletions` (source paths this
    plan will remove — the ordinary source of a `move`, or a residue left by an earlier
    partial execution — each removed only through the D-17 quarantine protocol; residue
    cleanup is an executable plan operation, not an out-of-band effect), and per-artifact
    `evidence`, `blockers`, `warnings`.
  - **Blocker codes (stable kebab-case):** `policy-unconfigured`, `unmanaged-path`,
    `permanent-path`, `destination-conflict`, `outside-owner-conflict`, `active-owner`,
    `unsupported-dynamic-edge`, `repository-escape`, `unresolvable-edge`,
    `non-terminal-target`, `stale-plan`, `missing-source`, `symlink-artifact`.
  - **Warning codes:** `missing-edge-target`, `external-edge`, `no-owner`,
    `residue-source`, `deletion-failed`, `not-selected-sibling`, `pinned-historical`,
    `pinned-unreachable`, `quarantined-foreign-file`.
  - **Missing-artifact semantics (evaluation order is normative):** each explicit root is
    evaluated in this order. (1) **Already-archived recognition:** a reference whose path
    resolves under the destination root, or whose stated source path maps through the D-12
    mirror to existing content, classifies `already_archived` — a root moved by a prior
    partial execution can never read as missing on retry. (2) **`missing-source`:** only a
    working-tree root absent at both its stated location and its mirror image blocks — a
    genuinely dangling reference that must be resolved (or removed) before execution, never
    silently entrenched or skipped. A missing **embedded** target gets warning
    `missing-edge-target` on the referencing artifact: it contributes no needs-destination
    constraint, is excluded from before/after edge validation (the edge resolves nowhere
    before archival, so no regression is possible), and never blocks.
  - **Candidate evaluation context:** `jit archive candidates` produces the same envelope
    per candidate with every check evaluated — archival takes no category input (D-21), so
    destinations are always computable and nothing is deferred or guessed.
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
    `:1466` (`Event::DocumentArchived`, superseded by the D-19 event pair).
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
    references are never rewritten, and git-resolvable pins impose no working-tree
    constraint.
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
classification are pure domain/document logic, free of I/O; storage owns all persistence,
the mutation primitive, and the quarantine protocol; commands orchestrate; CLI/output stay
user-facing. Categories, managed/permanent paths, archive root, and container-ness come
from repository configuration, never hardcoded (`@/inv/domain-agnostic`). The plan model is
a versionable list envelope with stable ordering; storage atomic writes and event append
respect `@/inv/atomic-writes` and `@/inv/event-log`.

## 3. Decomposition sketch (near-ready; jit-breakdown instantiates — no issues created here)

Three conceptual story-sized groups organize the work; the epic's children are the
task-tier items listed below, each independently landable and green at every boundary.
Ordering is expressed only through `depends-on`. Group A is the pure foundation; Groups B
and C consume it (each entry task `depends-on` the Group A item it actually needs: the
mutation primitive on the plan model, preview and candidates on the classifier) and can
proceed independently after A. The plan schema and blocker taxonomy land first, before any
CLI fan-out (obligation 4). Coverage is enforced at the task tier: each task carries its
`satisfies: REQ-*` label directly, and the group headers are organizational only, not
issues.

### Group A: Artifact plan model and resolver — covers REQ-01, REQ-02, REQ-04

- **Artifact plan model and blocker taxonomy**  `type: task`  `satisfies: REQ-01`  `depends-on: —`
  Outcome: a serializable artifact-plan type implementing the §2 archive-plan JSON schema
  and blocker/warning taxonomy verbatim (envelope, artifact entry, identity fields, and
  code sets are binding), emitted as a deterministically ordered JSON envelope.
  Own criteria: `[hard] LOCAL-01: Serializes a plan with stable artifact and blocker
  ordering independent of input order.` `[hard] LOCAL-02: The same plan object is the input
  to both preview rendering and execution.` `[hard] LOCAL-03: Artifact identity is
  (path, version); pinned versions serialize as non-relocating historical entries.`
  Blast radius: self-contained new module; no existing consumer changes.

- **Container closure and explicit-root inventory**  `type: task`  `satisfies: REQ-01`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: resolving a container yields the root plus its resolved-hierarchy descendants
  (D-25) and every distinct issue-linked artifact on those issues, including opaque/binary
  roots, each annotated with explicit-reference provenance, version (D-26), and all direct
  issue-reference owners across the whole repository, flagging owners outside the subtree.
  Own criteria: `[hard] LOCAL-04: Membership is the resolved-hierarchy children closure; a
  cross-container sequencing dependency contributes no member.` `[hard] LOCAL-05:
  Inventories every distinct DocumentReference in the closure, including CSV/PNG/SVG roots
  with no registered adapter and pinned versions as historical entries.` `[hard] LOCAL-06:
  Records every direct issue-reference owner of each artifact repository-wide, flagging
  owners outside the subtree.`
  Blast radius: self-contained; reuses `hierarchy.rs:458` and `storage/mod.rs:498`.

- **Recursive supported-dependency discovery**  `type: task`  `satisfies: REQ-01, REQ-04`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: recursive discovery of supported local dependencies — Markdown/HTML element URLs
  and CSS `@import`/`url()` — with cycle detection, normalized component-aware path
  resolution, before/after edge reachability, repository-escape rejection, and the D-23
  detection contract for dynamic and module-loading constructs. Discovery is version-aware
  (D-26): pinned roots are scanned over commit-resolved content through the storage layer's
  commit-aware reads, yielding informational (path, commit) dependency entries.
  Own criteria: `[hard] LOCAL-07: Discovers an HTML→sibling-CSS→nested-figure chain and CSS
  @import/url() targets to full depth, treating script-element src URLs as static supported
  edges.` `[hard] LOCAL-08: Terminates on dependency cycles and reports local-path-bearing
  constructs from the binding D-23 set as unsupported edges, by textual pattern match only.`
  `[hard] LOCAL-33: Discovers a pinned root's supported closure from its commit-resolved
  content, emitting non-relocating (path, commit) entries, and emits pinned-unreachable
  when the commit cannot be read.`
  Blast radius: extends the document adapter/scanner surface; existing Markdown/HTML
  callers (snapshot export) are unaffected because discovery is a new recursive path.

- **Move/copy/retain/block classification**  `type: task`  `satisfies: REQ-02, REQ-04`  `depends-on: Container closure and explicit-root inventory, Recursive supported-dependency discovery`
  Outcome: a pure classifier assigning each working-tree artifact move, copy, retain, or
  block through the D-16 edge-aware calculus, completing the D-13 ownership universe by
  extending direct owners with the supported embedded closure over every issue-linked
  document repository-wide, proposing destinations by the D-12 mirror rule, and applying
  the D-26 pinned model and the normative missing/already-archived evaluation order.
  Own criteria: `[hard] LOCAL-09: Ownership incorporates supported embedded references
  reachable from any issue-linked document repository-wide, flagging outside owners.`
  `[hard] LOCAL-10: Classifies deterministically per the calculus — a figure shared by a
  terminal and an active issue copies when a relocated parent references it relatively and
  retains otherwise, never force-relinking the active consumer; a git-resolvable pinned
  reference imposes no working-tree constraint while an unresolvable one forces retention.`
  `[hard] LOCAL-11: Every supported edge's resolution mode (relative vs root-relative)
  drives the decision, and the classifier never emits a layout in which a relocated
  parent's supported edge lacks its target.` `[hard] LOCAL-12: Uses path-component
  containment for managed/permanent matching, so dev/active-other does not match
  dev/active.` `[hard] LOCAL-13: Proposes destinations by mirroring repository-relative
  source paths beneath the destination root (full container UUID segment), keeping
  identical filenames distinct.` `[hard] LOCAL-14: Classifies a differing-content occupied
  destination as blocked citing the conflicting path, a content-identical one as already
  archived, and a symlink root/target/traversal as blocked with the symlink code.`
  Blast radius: self-contained; supersedes the archive-time directory heuristic without
  touching the legacy command until Group B.

### Group B: Unified archive CLI and safe executor — covers REQ-03, REQ-04, REQ-06

- **Storage-owned artifact mutation primitive**  `type: task`  `satisfies: REQ-03`  `depends-on: Artifact plan model and blocker taxonomy`
  Outcome: a reusable storage primitive that stages a full artifact set to temp, finalizes
  by atomic no-replace hard-link-then-unlink from same-filesystem staging (D-14, `.jit/tmp`
  lives inside the repository), relinks references, and removes sources last through the
  D-17 quarantine protocol — atomic rename into a unique quarantine location, identity
  verification (hash and size captured at staging) on the opened handle, then unlink, or
  no-replace restore on mismatch — holding the re-entrant repository write guard across the
  sequence, resolving paths physically, and never operating through a symbolic link (D-22).
  Testable through tempdir-backed storage (the in-memory backend performs no virtual file
  I/O, so filesystem assertions run against `JsonFileStorage` at a tempdir).
  Own criteria: `[hard] LOCAL-15: Rejects an occupied destination with the pre-existing
  file preserved byte-for-byte.` `[hard] LOCAL-16: Holds one write guard across staging,
  reference saves, and event append.` `[hard] LOCAL-17: A destination created by an
  external writer between planning and finalization fails that artifact's finalization
  without overwriting the foreign file, verified by a race-focused test.` `[hard] LOCAL-18:
  Never unlinks a path directly: every removal runs the D-17 directory-handle-anchored
  quarantine sequence (rename in, openat-verify on the handle, unlinkat on match; no-replace
  restore or quarantined-and-reported on mismatch), race-tested, with the path-anchored
  fallback and its D-24 bound documented on platforms without openat semantics.`
  Blast radius: self-contained new storage API; the legacy command keeps its inline
  `std::fs` path until it is removed.

- **Unified archive preview surface**  `type: task`  `satisfies: REQ-06`  `depends-on: Move/copy/retain/block classification`
  Outcome: `jit archive document <path>` and `jit archive container <id>` produce a
  complete non-mutating plan by default, running every precondition execution relies on
  (source availability, normalized policy, terminal eligibility, outside owners, pinned
  semantics, destination conflicts, before/after reachability), rendered as both the
  binding JSON envelope and human output, and verified against the
  Markdown/HTML/CSS/CSV/PNG/SVG fixture corpus.
  Own criteria: `[hard] LOCAL-19: Preview enumerates every artifact, action, evidence flag,
  and blocker and mutates nothing.` `[hard] LOCAL-20: Document and container targets
  produce the same plan schema from the shared planner.` `[hard] LOCAL-21: In a repository
  without configured documentation policy, preview returns a non-mutating inventory
  explaining that archival is disabled, and execution is refused.`
  Blast radius: adds a new `archive` command group alongside the existing `doc archive`;
  both coexist until the removal task. MCP tools regenerate from the CLI schema
  automatically.

- **Coordinated plan execution**  `type: task`  `satisfies: REQ-03, REQ-04`  `depends-on: Storage-owned artifact mutation primitive, Unified archive preview surface`
  Outcome: `--execute` recomputes the plan under repository coordination, stages the
  complete set, validates every supported local edge at its proposed destination before
  metadata commit, persists reference changes and the D-19 events, refreshes or invalidates
  cached asset metadata on moved references, leaves pinned references untouched (D-3,
  D-26), and removes sources only through the quarantine protocol. Only Done/Rejected
  containers execute; a document target executes only when every direct or embedded-closure
  owner of the document and of every bundle artifact is terminal, active owners block
  execution (preview still reports), and a zero-owner document in a managed path executes
  with an informational note (D-15). Retry converges by recomputation (D-18): a rerun
  treats content-identical occupied destinations as already archived, applies only the
  remaining mutations, rediscovers orphaned sources through the inverse D-12 mapping, and
  appends no event when nothing mutates. Event recording follows D-19: `ArchiveExecuted`
  after finalization/relinks and before deletions (definition key, adopt-or-mint instance
  id, execution id, plan fingerprint, completed mutations, pending deletions);
  `ArchiveSourcesRemoved` after removals complete, recording exactly the removed paths.
  Own criteria: `[hard] LOCAL-22: Validates each staged local edge in the proposed layout,
  not against source-resolved paths, before commit.` `[hard] LOCAL-23: Refuses
  document-target execution while any owner of the document or its bundle artifacts is
  non-terminal.` `[hard] LOCAL-24: A partial-relink, event-append, or deletion failure
  leaves no artifact lost, no destination overwritten, and no reference relying solely on a
  missing path.` `[hard] LOCAL-25: Refuses execution on a non-terminal container and on a
  stale recomputed plan.` `[hard] LOCAL-26: Rerunning execution after a partial staging,
  relink, event, or deletion failure converges — content-identical occupied destinations
  count as archived, no duplicate destination or reference is created, event emission
  follows D-19, orphaned sources are rediscovered through the inverse mapping and removed
  via quarantine — verified by failure-injection retry tests.` `[hard] LOCAL-27:
  ArchiveExecuted appends after relinks and before deletions with the D-19 identity triple
  and pending deletions; ArchiveSourcesRemoved appends after removals with the exact
  removed paths; a rerun that mutates nothing appends no event; retries share an instance
  id while a re-archival after convergence mints a new one.`
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
  outside-subtree/active/pinned/missing/unsupported/conflict blockers, and a
  move/copy/retain summary, in both human and JSON form, with no filesystem, issue, or
  event mutation and no age or retention filtering. Every candidate is evaluated fully:
  archival takes no category input (D-21), so the D-12 mirror rule determines each
  destination and no check is deferred or guessed.
  Own criteria: `[hard] LOCAL-29: Lists terminal containers with policy status, ownership,
  counts, and blockers, explaining exclusions rather than omitting them.` `[hard] LOCAL-30:
  Emits identical human and JSON results while mutating nothing, and applies no
  time/retention filter.` `[hard] LOCAL-31: Derives candidate container-ness from the
  configured type hierarchy's non-leaf levels and resolved-hierarchy membership, with no
  hardcoded type names.` `[hard] LOCAL-32: Evaluates every candidate fully — including
  destination conflicts under the mirror rule — with no category input, suggestion, or
  deferred check.`
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
| Pinned references silently corrupt provenance | High | D-3 + D-26: pinned references are never rewritten; git-resolvable pins read from history and impose no working-tree constraint; unresolvable pins force retention (LOCAL-10). |
| A retained dependency leaves a relocated parent's relative link without its target | High | D-16's calculus is total: a relative edge from a relocated parent forces needs-destination; root-relative edges force source retention; the classifier never emits an unpreservable layout (LOCAL-11) and execution re-validates every edge (LOCAL-22). |
| Deletion or rollback destroys a file an external writer replaced | High | D-17: removals are directory-handle-anchored quarantine sequences — rename in, verify on the open handle, unlinkat on match, no-replace restore on mismatch (LOCAL-18). Defeating the sequence requires write access that already suffices to delete files directly, so JIT adds no destructive capability (D-24, the provable platform bound). |
| Removing `jit doc archive` breaks docs/tests/MCP mid-flight | Medium | Removal ordered last (`depends-on: Coordinated plan execution`); old and new coexist until then; consumer migration in the same change; acceptance is the tree-wide search above (LOCAL-28). |
| Recursive CSS/HTML discovery mis-scopes a shared theme or figure | Medium | Repository-wide reference-count classification over the D-13 universe (LOCAL-09, LOCAL-10), not directory heuristics; shared-but-active artifacts copy/retain, never force-relink (D-2, D-10). |
| An artifact embedded by an outside document misclassified as unshared and moved | High | D-13's universe includes the supported embedded closure of every issue-linked document; the supplied-map limitation of the existing classifier (`assets.rs:189`) is superseded (LOCAL-09). |
| Post-archive verification defect masks broken links | Medium | Executor validates each edge in the proposed layout, not against source-resolved paths (LOCAL-22); replaces `document.rs:1832-1858`. |
| Coordination gap during multi-write execution | Medium | The mutation primitive holds the re-entrant write guard across staging, saves, and event append (LOCAL-16, `storage/mod.rs:126`); execution rejects stale recomputed plans (LOCAL-25); no-replace finalization closes the external-writer overwrite window (D-14, LOCAL-17). |
| A partial failure leaves an unrecoverable half-archived state on retry | Medium | D-18: retry recomputes and converges — content-identical destinations count as archived, orphaned sources are rediscovered through the inverse D-12 mapping and quarantine-removed (LOCAL-26); covered by failure-injection retry tests. |
| Archive events cannot be attributed or deduplicated across retries and crashes | Medium | D-19: three-level identity (definition key, adopt-or-mint instance id, execution id) and two event kinds (ArchiveExecuted before deletions, ArchiveSourcesRemoved after) keep the record truthful; nothing-mutated reruns append nothing; crash-window anomalies stay within the established contract (LOCAL-27). |
| Document-target execution on a path with mixed-state owners dangles an active reference | Medium | D-15: all direct and embedded-closure owners must be terminal; active owners block execution while preview still reports; zero-owner managed-path documents execute with an informational note (LOCAL-23). |
| Opaque roots need an adapter to be inventoried | Low | Inventory reads bytes via `storage/mod.rs:498`; parser support gates edge discovery only, not root eligibility (LOCAL-05). |

Every load-bearing question is resolved or owned. Each sketch item is executable from its
description plus the cited grounding without re-deriving the design.

## Decisions

Carried from the owner-approved brief (`dev/active/7d3a3a47-planning-brief.md`), binding,
including its 2026-07-13 owner amendment. Plan-level decisions D-12 onward resolve
specification gaps surfaced during planning and review; none is REOPEN.

- **D-1 — Archival is explicit opt-in:** chosen **require configured documentation policy**.
  Rejected: silently use default paths (moves evidence never delegated to JIT).
- **D-2 — Do not relocate another container's artifacts:** chosen **never relocate an
  artifact referenced outside the subtree; copy if required, block if ambiguous**. Rejected:
  global relink; a general force option.
- **D-3 — Pinned references remain historical:** chosen **never rewrite a pinned reference;
  keep its historical source path resolvable, copy content into the bundle when needed**.
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
  default; mutation requires `--execute`, which recomputes and revalidates before
  mutating**. Rejected: applying stale preview data.
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
- **D-13 — Ownership universe is the issue-linked closure:** chosen **all
  `DocumentReference`s across all issues plus each one's recursive supported embedded
  closure; outside owners (direct or embedded) forbid a move**. Files unreachable from any
  issue reference are outside JIT's referential contract: reported informationally near
  bundles (D-10), never counted as owners, never relocated. Rejected: scanning the entire
  working tree for arbitrary referencing files — unbounded, and JIT's guarantee is scoped
  to issue references.
- **D-14 — Finalization is atomic no-replace:** chosen **hard-link from same-filesystem
  staging then unlink staging; `AlreadyExists` aborts that artifact's finalization and
  triggers rollback**. Rejected: check-then-`rename` — `rename` replaces a destination
  created after the check, and the repository write guard cannot exclude external
  filesystem writers (`storage/mod.rs:90`).
- **D-15 — Document-target execution requires all-terminal owners:** chosen **`jit archive
  document --execute` requires every direct or embedded-closure owner of the document and
  its bundle artifacts to be terminal; active owners block (preview still reports); a
  zero-owner managed-path document executes with an informational note**. Rejected:
  selecting an owning container implicitly (ambiguous with several owners); a force
  override (mirrors the rejected D-8 force path).
- **D-16 — Edge-aware action calculus:** chosen **derive each action from needs-destination
  (selected root, or relative edge from a relocated parent) × needs-source (outside owner,
  active owner, git-unresolvable pinned reference, permanent path, or inbound
  root-relative/staying-document edge): move / copy / retain, with conflicts and
  unpreservable edges blocking**. Copy creates the mirrored destination and retains the
  source, satisfying both constraints, so every supported edge — relative at the mirror,
  root-relative at the repository root — resolves in the final layout. Rejected: a free
  copy-or-retain choice (leaves a relocated parent's relative edge without its target);
  treating root-relative edges like relative ones (their resolution ignores the referencing
  document's location, `assets.rs:151-162`).
- **D-17 — Every removal is a directory-handle-anchored quarantine sequence:** chosen
  **no path is ever unlinked directly. A removal (1) opens the quarantine directory —
  created fresh under `.jit/tmp` with an unpredictable name — as a directory handle,
  (2) atomically renames the target into it, (3) opens the quarantined entry **relative to
  that directory handle** via the safe `openat` wrapper in `nix` — already a direct
  dependency of `crates/jit` (`Cargo.toml:41`); this adds the `fs` feature, not a new
  crate, and keeps `#![deny(unsafe_code)]` intact because the unsafety lives inside the
  dependency — (4) verifies content identity (hash and size captured at staging) on that
  open handle, and (5) unlinks the entry by name **relative to the same directory handle**
  (`unlinkat`) on match — or restores it no-replace on mismatch, leaving it quarantined
  with a `quarantined-foreign-file` warning if restore is impossible.** The claim is
  stated exactly: `unlinkat` acts on the *name*, whose binding to the verified inode can
  change between steps 4 and 5 only through a write inside the just-created, unpredictably
  named, private quarantine directory. That residual race is **bounded, not denied**: the
  capability it requires already suffices to delete any repository file directly, so JIT's
  removal adds no destructive power the interferer lacks (D-24). Rename preserves whatever
  file is present at capture time, so nothing is overwritten at any step. Applies to ordinary move-source deletions, residue cleanup, and rollback of
  finalized destinations alike; platforms without `openat` semantics fall back to the same
  sequence path-anchored, with the D-24 bound stated for them explicitly. Rejected:
  verify-then-unlink at the original path (a substitution between verification and unlink
  destroys the substitute); unverified cleanup (deletes foreign files); global filesystem
  locking (unavailable against arbitrary external writers).
- **D-18 — Retry converges by recomputation:** chosen **a rerun after any partial failure
  recomputes the plan against current state, treats content-identical occupied destinations
  as already archived, applies only the remaining mutations, and appends no event when
  nothing mutates**. Repository-relative destination derivation (D-12) makes nesting
  impossible on retry, and its invertibility lets a rerun derive the original source path
  of every already-archived artifact, so an orphaned source left by a deletion failure is
  rediscovered and quarantine-removed even though no reference points at it any longer.
  Rejected: persisted resumable journals (adds state that can itself go stale); treating
  every occupied destination as a blocker (would make every partial failure permanent).
- **D-19 — Three-level identity plus two event kinds:** chosen **`operation_id` = hash of
  (target, destination root, sorted canonical source-path set — already-archived artifacts
  contribute their inverse-mapped original sources): the retry-invariant definition key.
  `operation_instance_id` delimits one logical operation by adopt-or-mint: a mutating
  execution adopts the instance id of the latest same-key event while current state shows
  that operation unconverged (outstanding relinks or pending deletions), and mints a fresh
  UUID once it converged — retries group, independent re-archivals of an identical set
  separate, and no persistent state beyond the existing event log is needed because
  convergence is recomputable (D-18). `execution_id` is unique per mutating run; all
  identity UUIDs are null in non-mutating envelopes. `ArchiveExecuted` appends after
  finalization/relinks and before deletions, recording completed mutations and pending
  deletions; `ArchiveSourcesRemoved` appends after source removals complete, recording
  exactly the removed paths; a run whose removals all fail appends nothing.** A crash can
  leave a duplicate `ArchiveExecuted` or an unrecorded removal of an already-safe path —
  both within the established referential-consistency contract (`document.rs:1050-1074`).
  Rejected: the plan fingerprint as retry identity (recomputation changes it); literal
  exactly-once event semantics (unachievable across a crash between mutation and the
  append-only event write); per-artifact events without an operation id (cannot audit a
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
  is no-replace (D-14, D-17). (2) JIT unlinks a name only after verifying the inode bound
  to it inside the directory-handle-anchored quarantine sequence (D-17); rebinding that
  name between verification and unlink requires write access to the quarantine directory
  JIT just created — a capability that already suffices to delete any repository file
  without JIT's involvement, so JIT's removals add no destructive power an interfering
  writer does not independently possess. The residual name-rebinding race is bounded by
  this capability argument, not denied.** This is the
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
  **artifacts are keyed by path plus version (`working-tree` or a commit); working-tree
  versions are the archival subjects; pinned versions resolve through the storage layer's
  commit-aware reads (`document.rs:349-357`, `storage/mod.rs:531`) from git history, are
  never relocated or rewritten, appear in plans as informational `pinned-historical`
  entries, and impose no working-tree retention constraint while their commit is
  reachable. Where git or the commit is unavailable, **read behavior is unchanged** —
  storage surfaces `CommitNotFound` exactly as today, no fallback exists or is added —
  and planning responds conservatively: the working-tree file gets needs-source and
  never-relink, with warning `pinned-unreachable`. Pinned roots' **commit-specific
  supported dependency closures are discovered**: the adapters are pure text extractors
  run over commit-resolved content (`read_path_text(path, commit)`, `storage/mod.rs:531`),
  yielding informational, non-relocating (path, commit) entries (LOCAL-33), so REQ-01's
  enumeration is complete for pinned roots with nothing to move — history serves every
  pinned reader.** Rejected: path-only identity (cannot represent two versions of one
  path); treating every pinned reference as a working-tree retention constraint
  (needlessly blocks archival of files whose history serves all pinned readers); rewriting
  pinned references (D-3); inventing a read fallback for unreachable commits (storage has
  none; planning must not assume semantics the code does not implement).
- **Assumptions:** Coverage is enforced at the task tier: each of the nine task-tier items
  is a direct child of the epic and carries its own `satisfies: REQ-*` label; the A/B/C
  group headers are conceptual only. This assumes a single breakdown pass produces
  executable leaves rather than an intermediate story tier; risk if wrong is a
  coverage-gate reshuffle, not a design change.
