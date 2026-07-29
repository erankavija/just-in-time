# Investigation: dependency-aware container artifact archival

**Issue:** `7d3a3a47`
**Scope:** read-only audit of the existing document/archive implementation and the claims carried into the replacement epic
**Audit date:** 2026-07-11

## Executive finding

The replacement epic is justified, but it should be planned as a new **artifact-plan domain capability with a container-scoped command consumer**, not as a loop around `archive_document`.

The current implementation has a useful and recently hardened single-document execution primitive: it copies a Markdown or HTML document plus conventionally located assets, re-links every matching issue reference, appends one repository-scoped archive event, and deletes sources only after the metadata/event commit point. Its intended guarantee is referential consistency rather than cross-file atomicity, and focused failure-injection tests substantiate that guarantee for unpinned document references (`crates/jit/src/commands/document.rs:1043-1082`, `crates/jit/tests/archive_integrity_tests.rs:74-120`, `crates/jit/tests/archive_integrity_tests.rs:203-266`, `crates/jit/tests/archive_integrity_tests.rs:284-334`).

It is not yet a complete planning primitive:

- It accepts one path and one category, not a container or artifact set (`crates/jit/src/cli.rs:1950-1969`).
- Dry-run returns before active-reference and destination-occupancy checks, and exposes only an asset count rather than the proposed actions and blockers (`crates/jit/src/commands/document.rs:1153-1176`, `crates/jit/src/commands/document.rs:1670-1683`, `crates/jit/src/commands/mod.rs:176-185`).
- Archive-time “shared” classification is a directory-name heuristic, not repository-wide reference analysis; the real reference-count classifier exists but has no production archive consumer (`crates/jit/src/commands/document.rs:1463-1495`, `crates/jit/src/document/assets.rs:185-233`).
- Dependency discovery is one level deep. HTML scanning sees only `src`/`href`; it does not recursively scan CSS, nested Markdown slides, scripts, or assets discovered through another artifact (`crates/jit/src/document/adapter.rs:101-146`).
- Unknown/opaque formats such as CSV, PNG, and SVG cannot be passed through the current archive command because asset scanning requires a registered text adapter; only Markdown and HTML are built in (`crates/jit/src/document/assets.rs:92-111`, `crates/jit/src/document/adapter.rs:177-209`).
- Pinned references are not safely handled: relinking changes only `DocumentReference.path`, while reads prefer the reference's unchanged pinned commit. The destination normally does not exist at that older commit (`crates/jit/src/commands/document.rs:1352-1375`, `crates/jit/src/commands/document.rs:349-357`, `crates/jit/src/domain/types.rs:865-879`).
- Post-archive verification compares the scanner's destination-resolved paths with the original source asset paths. After a real relocation those sets differ, so the conditional existence check is normally skipped; the integration test proves final files exist, but does not expose this comparison defect (`crates/jit/src/commands/document.rs:1832-1858`, `crates/jit/tests/doc_archive_tests.rs:1287-1314`).

The epic's highest-value deliverables therefore remain open: deterministic container/subtree inventory, recursive bundle discovery, repository-wide sharing/active-reference analysis, explicit move/copy/retain/block classification, a truthful JSON preview, and safe execution of the resulting plan. General link rewriting and Git LFS policy correctly remain separate follow-ups (`.jit/issues/7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc.json:3-4`).

## 1. Claim audit

### 1.1 Epic requirements

| Claim | Status | Audit result |
|---|---|---|
| REQ-01: deterministic plan for a container subtree and embedded dependencies | **Valid-open** | JIT has DAG-authoritative subtree primitives, but archive accepts only one path. Existing scanning is non-recursive and adapter-limited (`crates/jit/src/commands/graph.rs:166-176`, `crates/jit/src/commands/graph.rs:206-230`, `crates/jit/src/cli.rs:1950-1969`, `crates/jit/src/document/assets.rs:92-111`). |
| REQ-02: classify move/copy/retain/blocked using active refs, sharing, path policy, and conflicts | **Valid-open** | Individual checks exist, but no unified classification model or report. Archive-time per-doc classification is path-convention based; the reference-count classifier is separate (`crates/jit/src/commands/document.rs:1243-1281`, `crates/jit/src/commands/document.rs:1321-1340`, `crates/jit/src/commands/document.rs:1463-1495`, `crates/jit/src/document/assets.rs:185-233`). |
| REQ-03: no loss, overwrite, or dangling issue references | **Partly already done; valid-open at container scope** | The single-document copy/relink/event/delete sequence and occupied-destination no-op are implemented and tested. A container plan still needs cross-artifact conflict analysis, pinned-reference semantics, and coordination over the full execution (`crates/jit/src/commands/document.rs:1178-1231`, `crates/jit/src/commands/document.rs:1670-1749`, `crates/jit/tests/archive_integrity_tests.rs:156-201`). |
| REQ-04: preserve functional relative links for HTML/CSS/theme/figure bundles | **Valid-open** | Relative topology is preserved for assets under `assets/` or `<stem>_assets/`, but sibling `base.css`, `themes/`, and `figures/` are not generally classified as movable per-doc assets; recursive CSS dependency discovery is absent (`crates/jit/src/commands/document.rs:1463-1495`, `crates/jit/src/commands/document.rs:1660-1667`, `crates/jit/src/document/adapter.rs:101-146`). |
| REQ-05: non-mutating candidate report using terminal state, `done_at`, paths, category | **Valid-open** | `Issue.state`, `done_at`, documentation config, and hierarchy primitives exist. There is no candidate-report command. `done_at` is first-completion time and persists across reopening, so eligibility must require current terminal state as well as age (`crates/jit/src/domain/types.rs:480-544`, `crates/jit/src/domain/types.rs:696-713`, `crates/jit/src/config.rs:325-376`). |
| REQ-06: structured JSON preview and Markdown/HTML/CSS/CSV/PNG/SVG fixtures | **Partly already done; valid-open** | Per-doc dry-run has JSON output, and Markdown asset fixtures exist, but the schema contains only paths/category/count/updated ids/dry-run. It does not enumerate artifacts, actions, evidence, or blockers and cannot scan opaque CSV/image roots (`crates/jit/src/main.rs:4768-4789`, `crates/jit/src/commands/mod.rs:176-185`, `crates/jit/tests/doc_archive_tests.rs:923-1000`). |

### 1.2 Claims inherited from the rejected Phase 2 proposal

| Older claim | Classification | Evidence and disposition |
|---|---|---|
| “Limited format support (Markdown only)” | **Obsoleted as stated** | HTML is a built-in core adapter, and the multi-format web story is done (`crates/jit/src/document/adapter.rs:177-182`, `.jit/issues/abfd6016-a79d-4a5c-9d1d-d5393e7acb2d.json:2-5`). This does not imply arbitrary archival support for CSV/images. |
| HTML relative assets do not render | **Already done for serving; not for relocation** | The raw-assets bug is done and its design uses a wildcard raw route plus HTML base injection (`.jit/issues/5c060496-f33e-4a70-9414-ec9436b84b1e.json:2-5`, `dev/active/5c060496-raw-assets-design.md:14-30`). That browser-serving fix explicitly leaves author-side bundling out of scope (`dev/active/5c060496-raw-assets-design.md:134-138`). |
| Manual one-document archival | **Still valid** | The public command takes one document path; there is no container archive or sweep (`crates/jit/src/cli.rs:1950-1969`). |
| Shared relative links cannot be archived | **Still valid, though detection is incomplete** | Current behavior rejects some relative links to non-moved assets when source and destination component depth differs (`crates/jit/src/commands/document.rs:1537-1608`). |
| Atomic batch execution / rollback | **Invalid as stated** | The rejected design promised all-succeed/all-rollback (`dev/active/documentation-lifecycle-phase2-design.md:207-212`). The hardened implementation correctly documents that true atomicity across files, N issue records, and an append-only event log is unavailable; it promises resolvability and may leave a reported duplicate (`crates/jit/src/commands/document.rs:1050-1074`). Preserve that safety contract. |
| Link rewriting engine and `assets collect` | **Valid but separate** | The adapter hook is still an `unimplemented!` stub (`crates/jit/src/document/adapter.rs:44-50`). The new epic should first preserve bundle topology; targeted rewriting belongs in a follow-up only when move/copy/retain cannot preserve function. |
| AsciiDoc/reST/MDX adapters | **Invalid for this epic / speculative** | Only Markdown and HTML are registered (`crates/jit/src/document/adapter.rs:177-182`), but the replacement epic deliberately excludes speculative adapters (`.jit/issues/7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc.json:4`). Opaque artifact relocation should not require a parser. |
| Scheduled/lifecycle-triggered automation | **Invalid for this epic / intentionally dropped** | The old proposal included Done/Rejected/periodic triggers (`dev/active/documentation-lifecycle-phase2-design.md:390-405`); the replacement explicitly excludes scheduled automation (`.jit/issues/7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc.json:4`). |
| LFS policies | **Not implemented; separate correctness follow-up** | Snapshot manifests hard-code `allow-pointers` without pointer detection or policy enforcement (`crates/jit/src/commands/snapshot.rs:310-327`). The replacement epic explicitly leaves LFS policy separate (`.jit/issues/7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc.json:4`). |

## 2. Current mechanism, end to end

### 2.1 CLI and output contract

`jit doc archive PATH --type CATEGORY [--dry-run] [--force] [--json]` is defined directly as a document subcommand. `--force` means “allow archival of docs linked to active issues”; it is not a general conflict override (`crates/jit/src/cli.rs:1950-1969`). CLI dispatch calls `CommandExecutor::archive_document`, prints warnings separately, serializes `ArchiveResult` in the standard success envelope for JSON, and otherwise renders a document path, destination, category, and asset count (`crates/jit/src/main.rs:4750-4799`).

`ArchiveResult` contains `source_path`, `dest_path`, `category`, `assets_moved`, `updated_issues`, and `dry_run`. It has no per-artifact entries, decisions, reasons, reference owners, destination conflicts, or plan identifier (`crates/jit/src/commands/mod.rs:176-185`). Typed error-to-exit-code mapping exists for missing source (`NotFound`) and occupied destination (`AlreadyExists`) (`crates/jit/src/main.rs:213-218`).

**Dry-run audit.** Dry-run validates configuration/category, managed/permanent path, source existence, adapter resolution/asset scan, and the current relative-link heuristic; then it returns. It does not call `check_active_issue_links`, which occurs later, and does not execute `copy_to_archive`, where destination occupancy is checked (`crates/jit/src/commands/document.rs:1111-1164`, `crates/jit/src/commands/document.rs:1166-1182`, `crates/jit/src/commands/document.rs:1670-1683`). The integration test confirms non-mutation and absence of an archive event, but does not assert blocker completeness (`crates/jit/tests/doc_archive_tests.rs:936-1000`). Thus dry-run is safe but not execution-faithful.

### 2.2 Configuration and destination calculation

The command loads `[documentation]`, requires an explicit category key, and computes `archive_root/<category mapping>/<path relative to the first matching managed prefix>` (`crates/jit/src/commands/document.rs:1111-1139`, `crates/jit/src/commands/document.rs:1284-1319`). The repository config currently manages `dev/active`, `dev/studies`, and `dev/sessions`, archives under `dev/archive`, protects `docs/`, and maps five categories (`.jit/config.toml:9-22`). Defaults and schema live in `DocumentationConfig` (`crates/jit/src/config.rs:325-376`).

Path membership uses string `starts_with`, not path-component containment. For example, a configured `dev/active` also matches `dev/active-other`; planning should use normalized component-aware paths (`crates/jit/src/commands/document.rs:1243-1267`). Source existence is a typed, pre-mutation no-op check (`crates/jit/src/commands/document.rs:1269-1281`).

### 2.3 Discovery and classification

The command reads the working-tree document as UTF-8 text through `IssueStore::read_path_text`, resolves a built-in adapter, and scans the content (`crates/jit/src/commands/document.rs:1438-1461`). This is independent of `DocumentReference.assets`; persisted scan metadata is neither trusted nor refreshed by archive.

The adapter registry contains only Markdown and HTML (`crates/jit/src/document/adapter.rs:177-182`). Markdown uses `pulldown_cmark` and treats both links and images as “assets,” including local linked documents (`crates/jit/src/document/adapter.rs:258-297`). HTML uses one regex over `src` and `href`; it excludes anchors and mailto but does not parse `srcset`, CSS `url(...)`, inline style URLs, `data-markdown`, imports, or runtime fetches (`crates/jit/src/document/adapter.rs:101-146`). The scanner resolves relative/root-relative paths, marks local/external/missing, computes MIME/hash metadata, and rejects simple parent traversal beyond document depth (`crates/jit/src/document/assets.rs:114-183`). Discovery is not recursive.

There are two inconsistent meanings of “shared/per-doc”:

1. `AssetScanner::classify_assets` counts references across a supplied map of documents and marks multi-referenced assets shared, but also marks every asset outside a `_assets/` folder shared (`crates/jit/src/document/assets.rs:185-233`). No production archive call invokes this method.
2. `archive_document` ignores `Asset.is_shared` and treats an asset as movable solely when its resolved path lies below the document directory in `assets/` or `<document-stem>_assets/` (`crates/jit/src/commands/document.rs:1463-1495`). A file in those folders moves even if another document references it; a sibling `base.css` or `figures/...` remains “shared” even if it belongs only to this bundle.

The relative-link guard rescans, considers every non-moved resolved asset “shared,” and rejects a relative link only when source and destination have different component counts (`crates/jit/src/commands/document.rs:1527-1573`). Equal depth is not sufficient proof that a relative target still resolves to the same file, and missing assets have a resolved path and can be treated as shared; root-relative and external references pass (`crates/jit/src/commands/document.rs:1537-1573`). This is a heuristic, not a before/after resolution comparison.

### 2.4 Active references and reference relinking

Without `--force`, the command lists every non-terminal issue that directly references the exact source path and rejects execution (`crates/jit/src/commands/document.rs:1166-1176`, `crates/jit/src/commands/document.rs:1321-1340`). `State::is_terminal` means only Done or Rejected; `Archived` is not terminal under current query semantics (`crates/jit/src/domain/types.rs:46-58`). With `--force`, active issues are not retained on the source: all exact path matches across every issue are re-linked to the archive destination (`crates/jit/src/commands/document.rs:1352-1375`).

Relinking preserves the rest of `DocumentReference` unchanged, including `commit`, `format`, and cached `assets` (`crates/jit/src/domain/types.rs:865-879`, `crates/jit/src/commands/document.rs:1355-1364`). Consequences:

- A pinned reference will subsequently try the new path at its old commit because reads prefer `doc.commit`; that path normally did not exist there (`crates/jit/src/commands/document.rs:349-357`). This violates the broad “every issue reference resolves” claim outside the unpinned test fixture.
- Cached asset `resolved_path` values can still describe the source layout after relocation (`crates/jit/src/domain/types.rs:877-879`).
- The archive event records only source, destination, category, and issue count, so it cannot reconstruct the exact set or prior reference metadata (`crates/jit/src/domain/types.rs:1425-1439`, `crates/jit/src/domain/types.rs:1740-1755`).

### 2.5 Filesystem and metadata sequencing

The implemented sequence is:

1. Stage document and selected assets under `.jit/tmp/archive-<uuid>` on the repository filesystem (`crates/jit/src/commands/document.rs:1637-1668`).
2. Reject if any final destination already exists, before creating final destinations (`crates/jit/src/commands/document.rs:1670-1683`).
3. Copy all sources to temp, then atomically rename each temp file to its final destination; on a mid-finalization error, best-effort delete destinations already created (`crates/jit/src/commands/document.rs:1685-1742`).
4. Verify the archived document, then re-link issue records one at a time and append one `DocumentArchived` event (`crates/jit/src/commands/document.rs:1186-1213`).
5. Delete source files only after metadata and event persistence; deletion failures are returned as warnings and leave harmless source duplicates (`crates/jit/src/commands/document.rs:1215-1231`, `crates/jit/src/commands/document.rs:1752-1775`).

Destination conflict behavior is sound for a single planned set: any document or asset collision yields typed `DestinationOccupied`, and a focused test proves the pre-existing file is byte-for-byte preserved (`crates/jit/src/commands/document.rs:1670-1683`, `crates/jit/tests/archive_integrity_tests.rs:156-201`). There is still a time-of-check/time-of-use window: the command does not acquire the repository write guard across planning, filesystem copying, all issue saves, and event append, although `IssueStore` exposes a re-entrant guard specifically for multi-write sequences (`crates/jit/src/storage/mod.rs:115-124`). Container execution should hold this coordination guard or establish an equivalent plan freshness/version check.

`save_issue` persists each issue using the storage layer's atomic temp-file-and-rename primitive, while event append is a separately locked append (`crates/jit/src/storage/json.rs:212-223`, `crates/jit/src/storage/json.rs:550-554`, `crates/jit/src/storage/json.rs:735-749`). This supports the stated conclusion that filesystem + N issue records + append-only event cannot be one literal transaction.

On relink/event failure, rollback re-links any destination references to the still-existing source and removes destination copies only after confirming no issue points at the destination. If restoration or verification fails, both copies remain and an explicit manual-cleanup error is returned (`crates/jit/src/commands/document.rs:1391-1435`). Failure-injection tests cover relink persistence failure, event append failure, and multiple reference owners without dangling unpinned references (`crates/jit/tests/archive_integrity_tests.rs:203-266`, `crates/jit/tests/archive_integrity_tests.rs:284-334`, `crates/jit/tests/archive_integrity_tests.rs:336-396`).

### 2.6 Post-archive verification defect

`verify_post_archival_links` scans the document at `dest_doc`, then constructs `per_doc_set` directly from the original source-resolved paths and checks existence only if the destination-resolved asset path is contained in that old set (`crates/jit/src/commands/document.rs:1832-1858`). For a normal move:

```text
old expected: dev/active/assets/icon.svg
new scanned:  dev/archive/features/assets/icon.svg
```

The membership predicate is false, so no existence check runs. The integration test independently asserts that the moved assets exist and the link text is unchanged, which validates the happy-path topology but not the verification algorithm (`crates/jit/tests/doc_archive_tests.rs:1287-1314`). The planner/executor should validate every supported local edge by resolving it against both the proposed layout and staged destination before metadata commit.

### 2.7 Snapshot and LFS adjacent behavior

Snapshot export is a separate consumer of `AdapterRegistry` and `AssetScanner`. It scans each document one level deep, silently substitutes an empty asset list when scanning fails, includes only successfully readable local assets, and preserves repository paths (`crates/jit/src/commands/snapshot.rs:203-263`, `crates/jit/src/commands/snapshot.rs:437-464`). It warns and continues when a document snapshot fails (`crates/jit/src/commands/snapshot.rs:535-545`).

The manifest declares `external_assets_policy = "exclude"` and `lfs_policy = "allow-pointers"`, but there is no LFS-pointer parsing or policy branch in this path (`crates/jit/src/commands/snapshot.rs:310-327`). Consequently “allow-pointers” is descriptive hard-coded metadata, not verified behavior. Snapshot can reuse a future recursive artifact graph, but LFS correctness should remain a separate issue so it does not expand archival scope.

## 3. Architecture and consumers

### Existing layers

| Layer | Existing archive-related responsibility | Evidence |
|---|---|---|
| CLI | Defines one-path archive flags and snapshot flags | `crates/jit/src/cli.rs:1950-1969`, `crates/jit/src/cli.rs:2484-2548` |
| Main/output | Dispatches archive and renders human/JSON results | `crates/jit/src/main.rs:4750-4799`, `crates/jit/src/commands/mod.rs:176-185` |
| Command | Owns configuration, scanning, classification heuristic, filesystem mutation, relinking, event sequencing, rollback, and verification | `crates/jit/src/commands/document.rs:1100-1231`, `crates/jit/src/commands/document.rs:1438-1875` |
| Document subsystem | Adapter registry, Markdown/HTML extraction, path resolution, metadata/hash, optional multi-doc classification | `crates/jit/src/document/adapter.rs:24-50`, `crates/jit/src/document/adapter.rs:169-209`, `crates/jit/src/document/assets.rs:83-233` |
| Domain | `Issue`, `DocumentReference`, lifecycle timestamps, `DocumentArchived` event | `crates/jit/src/domain/types.rs:480-544`, `crates/jit/src/domain/types.rs:860-880`, `crates/jit/src/domain/types.rs:1425-1439` |
| Graph/hierarchy | DAG-authoritative resolved children and subtree/closure primitives | `crates/jit/src/graph/hierarchy.rs:260-326`, `crates/jit/src/commands/graph.rs:166-176` |
| Storage | Lists/saves issues, reads repo paths safely, appends events, exposes repo-wide write guard | `crates/jit/src/storage/mod.rs:115-135`, `crates/jit/src/storage/mod.rs:288-300`, `crates/jit/src/storage/mod.rs:500-535` |
| Snapshot consumer | Independently scans/copies documents and assets | `crates/jit/src/commands/snapshot.rs:203-263`, `crates/jit/src/commands/snapshot.rs:437-464` |
| Server/web consumer | Reads issue-linked bytes and separately serves raw repository assets; rendering is not relocation | `crates/jit/src/commands/document.rs:360-372`, `dev/active/5c060496-raw-assets-design.md:14-30` |

### Boundary assessment

The new **artifact graph and decision rules should be pure domain/document logic**: inputs are normalized issue/container records, documentation policy, an inventory of paths, and extracted dependency edges; output is a deterministic plan. Container traversal should reuse DAG-authoritative hierarchy/closure behavior rather than label membership (`crates/jit/src/commands/graph.rs:169-176`).

The command layer should orchestrate loading issues/config/content, call the pure planner, render the plan, and execute an accepted plan. It should not duplicate decision logic between preview and execution. The existing archive command currently performs raw `std::fs` copies, renames, and deletes directly (`crates/jit/src/commands/document.rs:1637-1747`, `crates/jit/src/commands/document.rs:1759-1787`), despite the repository architecture assigning persistence to storage. A reusable artifact mutation abstraction in storage would make JSON-file and in-memory testing possible and centralize containment, symlink, atomic-write, and collision semantics.

The output layer needs a versionable list envelope with stable ordering and explicit evidence. At minimum each artifact entry should include source, destination (if any), action, kind/format, discovery provenance, owning documents/issues, active/terminal owners, embedded dependencies, blockers/warnings, and whether execution will change issue metadata. This is materially richer than `ArchiveResult` (`crates/jit/src/commands/mod.rs:176-185`).

Potential consumers of the same plan model are:

- container archive execution;
- read-only candidate reporting using current terminal state plus `done_at`;
- `doc check-links`/validation of proposed layouts;
- snapshot export's bundle inventory and deduplication;
- later targeted link rewriting or `assets collect`;
- human and JSON preview surfaces.

## 4. Planning constraints and recommended split

### Story A — pure container artifact-plan model and resolver

This is the foundation and should satisfy REQ-01 plus the analysis half of REQ-02/REQ-04.

Required decisions/acceptance details:

1. Resolve the root through normal short/full-id storage semantics, then enumerate the root plus DAG-authoritative transitive dependency closure (`crates/jit/src/commands/graph.rs:206-230`).
2. Include every distinct `DocumentReference.path` on those issues, including opaque/binary roots; parsing support controls embedded-edge discovery, not whether the artifact can be inventoried (`crates/jit/src/domain/types.rs:860-879`).
3. Model provenance for each artifact: explicit issue reference versus embedded edge, plus all owners inside and outside the selected subtree.
4. Recursively discover supported local dependencies with cycle detection and deterministic path ordering. Initial supported extractors should be explicit: Markdown and HTML element URLs; CSS `url()`/`@import` is needed to meet the stated HTML theme requirement. Dynamic JavaScript fetches should be reported unsupported, not guessed.
5. Resolve links by normalized path identity, compare actual before/after targets, reject repository escapes, and retain external URLs as non-moving informational edges.
6. Compute move/copy/retain/block from all reference owners, current lifecycle state, policy paths, sharing, destination occupancy, and pinned-commit semantics. Do not equate directory convention with sharing.
7. Emit a stable, fully enumerated JSON plan. The same plan object must drive execution.

### Story B — safe plan execution and container command

This should satisfy REQ-03 and the execution half of REQ-04/REQ-06.

Required decisions/acceptance details:

1. Add a container-scoped command without weakening the existing `jit doc archive` contract. Preview must run every non-mutating precondition that execution relies on, including active references and all destination collisions.
2. Preserve relative topology as the default. Copy shared/active artifacts or retain them only when every moved dependent still resolves; block ambiguity.
3. Hold repository coordination across freshness validation, staged filesystem work, reference writes, and event append, or reject stale plans by fingerprint. Reuse the re-entrant write guard (`crates/jit/src/storage/mod.rs:115-124`).
4. Generalize copy-to-temp/finalize/relink/delete while retaining the current guarantee: never delete a path while any recorded reference might rely exclusively on it; keep reported duplicates if rollback cannot be verified (`crates/jit/src/commands/document.rs:1391-1435`).
5. Define pinned references explicitly: likely leave historical pinned refs at their historical path and copy/retain the source, or rewrite only working-tree refs while preserving pinned history. Merely changing `path` is unsafe (`crates/jit/src/commands/document.rs:349-357`, `crates/jit/src/commands/document.rs:1355-1364`).
6. Refresh or invalidate persisted asset metadata whenever a reference moves (`crates/jit/src/domain/types.rs:874-879`).
7. Validate every staged local dependency edge at its proposed destination before metadata commit; fix the source/destination path-set mismatch in the existing verifier (`crates/jit/src/commands/document.rs:1832-1858`).
8. Define event granularity: one plan/container event with artifact/reference details, per-artifact events, or both. The current event cannot audit a multi-artifact action (`crates/jit/src/domain/types.rs:1425-1439`).

### Story C — archival candidate report

This independently satisfies REQ-05 and the read-only half of REQ-06 after Story A.

Required decisions/acceptance details:

1. Candidate eligibility requires current terminal state and an age basis. `done_at` is first-Done only and remains set if reopened, while Rejected issues have no `done_at` by definition (`crates/jit/src/domain/types.rs:537-544`, `crates/jit/src/domain/types.rs:696-713`). Define whether rejected candidates use `updated_at`, event-derived rejection time, or no age filter.
2. Report containers and/or individual artifacts only after repository-wide owner analysis; a terminal issue's document can still be active elsewhere.
3. Respect managed/permanent paths and category availability, but report why an item is ineligible rather than silently omitting it (`crates/jit/src/config.rs:325-376`).
4. Guarantee no filesystem, issue, or event mutation; cover this explicitly for human and JSON modes.

### Separate follow-ups

- **Targeted link rewriting / asset collection:** adapter `rewrite_links` is unimplemented and should remain outside the critical path until topology-preserving plans encounter a demonstrated blocker (`crates/jit/src/document/adapter.rs:44-50`).
- **Snapshot/LFS accuracy:** detect pointer files and make manifest policy truthful; current hard-coded metadata is not enforcement (`crates/jit/src/commands/snapshot.rs:310-327`).
- **Additional text adapters:** demand-driven only; opaque artifacts must already be movable without them.

## 5. Questions for the planning interview

These choices materially affect the breakdown and should be resolved before implementation stories are finalized:

1. **Command and selection boundary:** Should the primary operation accept only a completed container (`jit archive <container>`), or should it also accept non-terminal containers when every selected action is explicitly reviewed? The epic says “completed container,” while REQ-02 already models active references (`.jit/issues/7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc.json:4`).
2. **Shared artifact policy:** When an artifact is used both inside and outside the selected subtree, is the default to copy it into the archive bundle, retain it in place and preserve links, or block for explicit choice? Copy maximizes bundle self-containment; retain minimizes duplication.
3. **Pinned historical references:** Should archive leave pinned references and their source path untouched (copying rather than moving as needed), or is it acceptable to convert a pinned reference into a working-tree/archive reference? The latter changes provenance and should not be implicit (`crates/jit/src/commands/document.rs:349-357`).
4. **Category mapping:** Is one category supplied for the entire container archive, inferred per document from `doc_type`, or represented in the plan per artifact? Current execution requires one caller-supplied category (`crates/jit/src/cli.rs:1955-1957`).
5. **Initial recursive formats:** Is CSS dependency parsing required in the first executable increment (necessary for gf2-style themes), and should JavaScript-discovered/runtime dependencies be blocked, warned, or out of scope?
6. **Candidate time semantics:** For Done issues, should retention age use first `done_at` even after a reopen/recomplete, or the most recent terminal transition from events? What timestamp should Rejected use?
7. **Execution UX:** Should preview be an implicit default that requires a second `--execute` invocation, or should `--dry-run` remain optional? The safety requirement only mandates structured previews, not whether preview is mandatory (`.jit/issues/7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc.json:4`).

## 6. Recommended quality gates

The breakdown should require tests at three layers:

- Pure unit/property tests for artifact graph traversal, normalized path resolution, cycles, stable ordering, action classification, and before/after edge reachability.
- In-process command/harness tests for container resolution, cross-subtree owners, lifecycle timestamps, category/policy handling, pinned references, and JSON schema.
- CLI/filesystem failure-injection tests for occupied destinations, stale plans/concurrent writers, partial staging, partial relink, event failure, source deletion failure, retry/idempotency, and exact non-mutation of previews.

Representative fixtures must include Markdown → image/document links; HTML → sibling CSS → nested theme image/font; direct CSV/PNG/SVG references; missing and external edges; a shared figure with terminal and active owners; identical filenames in separate directories; a pinned commit reference; and a dependency cycle in the artifact graph. Existing archive tests are a strong base for referential-consistency failure cases (`crates/jit/tests/archive_integrity_tests.rs:203-396`), while the current nested-asset fixture covers topology preservation only under the `assets/` convention (`crates/jit/tests/doc_archive_tests.rs:1238-1314`).
