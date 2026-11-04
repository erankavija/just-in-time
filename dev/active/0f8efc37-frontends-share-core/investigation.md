# Frontends share core: current-system investigation

> Reference; planning evidence for 0f8efc37. Investigated 2026-11-04/05 against the current shared working tree. This report does not implement or choose the plan. Original-claim classifications are preserved below; the final scope update immediately below governs current planning.


## Final scope update (dispatcher/steward, 2026-11-04)

The user chose complete MCP retirement by a separately owned task. REQ-13 and REQ-14 are retired without renumbering; the conditional original MCP findings below are historical evidence for that owner and are not server/web implementation scope. The live epic now has 13 active criteria. Its amended REQ-06 explicitly requires one shared issue capture; amended REQ-08 names the core issue matcher without assuming one already exists in domain; amended REQ-10 explicitly preserves `/issues` and `/health` and bounds the requested route removal inventory. Thus the three original invalid-as-stated criterion findings are corrected in the live text, rather than remaining blockers.

Original classifications: 15 criteria (0 done, 12 valid/open including 2 retired conditional, 3 invalid as originally written). Current amended criteria: 13 valid-and-open. The steward retained HTTP document diff because historical `dev/vision/document-viewer-implementation-plan.md:39` declares that supported route; current REQ-09/10 consolidate it on core methods. Missing in-tree callers do not justify deleting it. Remaining scope points: `hierarchy.ts` deletion and level-2 selection are excluded by current D-4; do not add them as work. D-17 attribution is corrected. Startup combined migration c5062e21 belongs to the Git owner and is being amended to adopt `open_repository`; this epic consumes it rather than producing parallel startup code.

The steward retained top-level `jit search` as a distinct raw-content capability after the precise contract in `docs/reference/cli-commands.md:3572` showed that explicit globs may search history. Its regex, case, context, limit and glob behavior remains unchanged. The REQ-08 restriction applies to the HTTP/web semantic issue and linked-document composition; `jit issue search` shares its semantic issue matcher. Do not turn raw CLI grep into semantic search or retire its metadata/history corpus. Earlier proposals below are original investigation alternatives, superseded by this decision.

REQ-02 is a coherent fallible-binding contract change: trait declaration `storage/mod.rs:183`, JSON implementation `json.rs:919`, executor builder `commands/mod.rs:2531` (returns Self and binds storage before config), plus all inventoried callers/implementations below. Mutation session binds after recovery (`repository_state_store.rs:621`) and refreshes same-root evidence (`:941`). Validate incompatibility before side effects; preserve same-root refresh and recovery ordering. Avoid a second setter alongside the infallible one or a separate task for each mechanical call-site edit.

## Original material corrections and decisions needed

1. Preserve the already-landed repository bootstrap and executor search producers. `commands/bootstrap.rs:71` is the core `open_repository` boundary; `dispatch/issue.rs:146` already calls `queries().search_issues_with_filters`. Adopt these, rather than creating another constructor or executor search API. Active peer work moves command result types and finalizes the error/schema boundary; consume its result.
2. REQ-06 needs a stronger shared-input statement if the headline promise is one repository issue read per version. Computing the list once and graph once independently still permits two reads. Specify one version-scoped issue input with both response projections, including concurrent misses, and define behavior when a tracker version changes during computation. Current `AppState` has no cache (`server/src/routes.rs:116`), and both routes list issues (`:232`, `:326`). A debounced watcher version is an invalidation hint, not a transaction revision (`watcher.rs:137`, `:142`). Do not claim it proves a coherent read.
3. REQ-08's existing “core domain search” does not exist: issue text matching is in `QueryOps::search_issues` (`commands/search.rs:53`), with storage I/O at `:54`. Keep the public executor entry point; a reusable matching computation belongs in a pure inward layer. Decide the canonical matching contract explicitly: today command search matches whole-query case-insensitive title/description substrings and ID prefixes (`:65`), web search tokenizes/scores (`web/src/utils/clientSearch.ts:44`), and `/search` exposes regex/case options for ripgrep (`routes.rs:453`). Do not silently promise these are identical existing contracts.
4. REQ-10 cannot infer absence of external API callers from an in-repository search. It also conflicts with REQ-05 plus REQ-06: `/issues` loses its only in-tree production caller when the web refresh/list is removed, while REQ-06 still explicitly retains a cached issue-list response. Recommend a deliberate supported-route contract and explicit deletion decisions. A client method used only by tests is provably removable within the repository; a public HTTP endpoint needs a scope decision. Preserve pending that decision. `/issues/:id/documents/:path/diff` is also tests-only in-tree, beyond the four unused routes named in the Background.
5. REQ-09 consumes the Git epic's working-tree content fix and storage Git operations. Its remaining work is one command method family and adapter adoption. Git plan `dev/active/f4d9502a-git-cli-module/plan.md:119`–`:123` already assigns pinned reads, show version selection, both diff variants, and both history variants. Do not create duplicate producers. Compare document content before HTML serving transformations: raw HTML responses intentionally inject a base element (`routes.rs:774`), so byte identity with those rendered raw responses is not an appropriate content contract.
6. REQ-01 overlaps Git epic selected-root migration, currently `server-layout-from-data-root` (same plan, decision section `:295`, `:331`). Reconcile it into adoption of the shared bootstrap before assigning changes to server startup. Parent reports c5062e21 owns that migration.
7. The Background's dead `web/src/types/hierarchy.ts` and hardcoded container level 2 (`subgraphClustering.ts:66`) are real but lack explicit success-criterion coverage. Include accepted scope with explicit criteria or remove these from the scope explanation. REQ-15 indexes do not themselves correct a domain-specific container-selection policy.
8. D-17 is about redo-record mutation cost independent of history (`dev/vision/9db27a3a-charter.md`, D-17), not refresh cost independent of browser count. D-18 and D-20 support the latter; correct the attribution.
9. MCP retirement is a product-surface decision. D-9 explicitly preserves MCP (`dev/vision/9db27a3a-charter.md:189`–`:195`); D-11 and D-16 also mention its audits/package. Hold REQ-13/14 and the schema's MCP-only metadata policy until the steward/user decides. No unique domain capability exists in the bridge; its unique capability is MCP transport for clients that cannot or do not invoke a shell (evidence below).

## Success-criterion classification

Each row classifies the current requested outcome, not the severity of its defect. “Open” below means **valid-and-open**. No complete criterion is already satisfied. Conditional MCP rows describe defects if that surface is retained, without deciding retention.

| Criterion | Classification | Current evidence and required correction |
|---|---|---|
| REQ-01 | valid-and-open | Server independently calls `WorktreePaths::detect` and `discover_repository_layout` (`crates/server/src/main.rs:67`, `:70`) and builds its own storage (`lib.rs:79`). CLI now uses the existing `open_repository` (`crates/jit/src/commands/bootstrap.rs:71`, `dispatch/bootstrap.rs`). Adopt it; reconcile Git owner first. |
| REQ-02 | valid-and-open | General `IssueStore::configure_repository_layout` still returns unit (`storage/mod.rs:183`); JSON mismatched data roots return silently (`json.rs:923`). However server startup already builds storage from normalized layout root (`server/src/lib.rs:79`) and opens a session, whose `session_layout` rejects mismatched roots (`storage/repository_state_store.rs:531`). Existing server mismatch/500 explanation is stale; preserve startup recovery. |
| REQ-03 | valid-and-open | Watcher already uses `VirtualPath` constants (`watcher.rs:80`–`:112`), but predicate ownership remains in server at `:69`; storage-owned predicate is still missing. Server also has the search glob `issues/*.json` (`routes.rs:468`). Avoid interpreting storage-literal prohibition as HTTP route or fixture-name prohibition. |
| REQ-04 | valid-and-open | CONFIG omitted from watcher list (`watcher.rs:112`), explicitly asserted irrelevant in existing test (`:337`). UI loads hierarchy once (`GraphView.tsx:560`–`:575`); graph gets hierarchy per call (`routes.rs:335`–`:340`). ConfigOps hierarchy/namespaces read configuration afresh (`commands/config.rs:242`, `:247`); do not replace these with executor lifetime OnceLocks. |
| REQ-05 | valid-and-open | App lists whole issues per SSE version (`App.tsx:89`–`:103`), passes list to IssueDetail (`:292`); detail uses list for dependency title (`IssueDetail.tsx:426`). Graph already has ID/title-as-label/labels/state (`routes.rs:345`–`:355`). Graph callback/shared state must expose unfiltered graph metadata to App/detail or filters will hide dependency titles/labels. |
| REQ-06 | invalid-as-stated | The separate at-most-once computations do not guarantee the stated one-read outcome. Both presently list (`routes.rs:232`, `:326`), no cache in AppState (`:116`). Clarify one shared issue input and version-crossing semantics. Caching itself is valid open work. |
| REQ-07 | valid-and-open | Routes directly invoke synchronous executor methods; no `spawn_blocking` or `block_in_place` in server source. Existing `reject` and exhaustive `ErrorCode` projection already centralize status classification (`routes.rs:30`, `:74`, `:79`). Reuse these; create only execution/offload helper, keep HTTP types in server. Multiple executor operations inside one request should run inside one offloaded closure. |
| REQ-08 | invalid-as-stated | Existing canonical issue matcher is commands code, not domain code (`commands/search.rs:53`); server greps JSON (`routes.rs:468`), web scores (`clientSearch.ts:44`, `useSearch.ts:39`). Correct layer/matching premise and retain executor API. Ripgrep linked-document behavior already guards empty paths (`routes.rs:449`). |
| REQ-09 | valid-and-open | CLI calls show/history/diff (`dispatch/doc.rs:75`, `:109`, `:124`); server calls read/get-history/get-diff (`routes.rs:740`, `:925`, `:972`). CLI unpinned chooses HEAD (`commands/document.rs:575`, `:590`), server selects None/working-tree (`:719`, `:727`). Git peer owns primitives and version-selection behavior. Converge adapters and remove superseded method family after it lands; retain byte API for binary raw documents. |
| REQ-10 | invalid-as-stated | The route set includes public endpoints without in-tree production callers (`routes.rs:125`–`:155`), which does not prove no external caller. Retaining list caching conflicts with removing every route after REQ-05. Server `StatusResponse` repeats status fields (`:379`; core `commands/mod.rs:2126`), but HTTP-specific graph/raw/namespace adapters are legitimate projections. Distinguish duplicate core contract from transport-specific shape. |
| REQ-11 | valid-and-open | Binary repeats modules (`main.rs:6`–`:10`) exported by lib (`lib.rs:6`–`:13`); therefore embedded/routes/sse/watcher and their unit tests compile in both targets. `git2`/`tower` occur only in manifest (dependencies `Cargo.toml:43`, `:62`), no source use; confirm dependency removal with normal workspace checks. |
| REQ-12 | valid-and-open | Raw body/header blocks repeated in `routes.rs:772`–`:795`, `:822` onward, `:875` onward. `infer_content_type` is handwritten (`:521`); embedded server already uses mime_guess. Custom encoder (`routes.rs:551`) and decoder (`embedded.rs:21`) are two halves of percent coding; replace both using one established implementation, preserving malformed UTF-8/path rejection and HTML CSP/base behavior. MIME dependency currently optional under embed-web, so make route availability feature-independent. |
| REQ-13 | valid-and-open, conditional | Schema type State already derives from `State::ALL` (`schema.rs:528`); bridge state prose is hardcoded (`index.js:41`), timeout commands hardcoded (`cli-executor.js:158`), metadata internals emitted (`tool-generator.js:92`, `:94`), and lookup linear (`index.js:241`). Schema lacks long-running metadata (`schema.rs:40`–`:65`). `hidden_commands` is another duplicated MCP policy (`schema.rs:288`; curated manifest). Hold retention-specific design; schema additions coordinate with peer owner. |
| REQ-14 | valid-and-open, conditional | `ConcurrencyLimiter.run` decrements before `setImmediate(resolve)` (`concurrency.js:38`–`:44`), admitting a fresh caller into the released slot before queued waiter increments at `:33`. Existing tests do not own adversarial handoff. Hold retention-specific implementation decision. |
| REQ-15 | valid-and-open | Hot loops search clusters (`GraphView.tsx:162`, `:165`), nodes (`:246`, `:833`), cluster membership (`:656`); `clusterAwareLayout.ts:90`, `:91`, `:103` use includes. Existing `nodeToCluster` map (`:66`) is partial prior art. Define index lifetime per layout/refresh pass and scope prohibition to repeated per-node/edge work, not isolated click handlers (`GraphView.tsx:958`). |

Criteria totals: 0 already-done; 12 valid-and-open (2 MCP-conditional); 3 invalid-as-stated.

## Background claim classification

Compound rows name each checked subclaim. “already-done” denotes a previously described defect that current code has removed.

| ID | Claim | Classification | Evidence |
|---|---|---|---|
| B01 | Server has its own discovery, unlike CLI data-root-aware bootstrap | valid-and-open | `server/src/main.rs:67`, `:70`; `jit/src/commands/bootstrap.rs:77`, `:92`. CLI source locations in epic predate dispatch extraction. |
| B02 | prepare_server_storage builds storage from raw data-dir | already-done | It builds from normalized `layout.data_root()` (`server/src/lib.rs:79`), covered by relative-path recovery test `:100`. |
| B03 | configure_repository_layout silently ignores a root mismatch | valid-and-open | `storage/json.rs:923`–`:925`; this is the public binding behavior, distinct from session opening. |
| B04 | This mismatched raw server construction causes a 500 on every request | invalid-as-stated | Server normalized construction (`lib.rs:79`) and rejecting session open (`repository_state_store.rs:531`) prevent the stated scenario at startup. |
| B05 | Watcher receives raw data-dir | already-done | Server passes normalized `data_root` (`main.rs:72`, `:89`). |
| B06 | Watcher repeats storage names as literals and filters external claims paths | already-done | Predicate now uses `VirtualPath` (`watcher.rs:78`–`:112`); no claims paths remain in it. |
| B07 | Watcher ignores config, affecting graph; UI hierarchy only mounts | valid-and-open | `watcher.rs:112`, `:337`; `routes.rs:338`; `GraphView.tsx:575`. |
| B08 | Each SSE version triggers whole list plus graph per tab | valid-and-open | `App.tsx:89`–`:92`; `GraphView.tsx:582`, `:886`; `routes.rs:232`, `:326`. Two nominal full issue listings per tab, with possible additional passes/derived-document work (`routes.rs:233`), not an exact universal I/O count. |
| B09 | No response cache by tracker version | valid-and-open | Entire AppState (`routes.rs:116`–`:123`), handlers `:226`, `:320`. |
| B10 | Audit byte counts and dated issue-file totals | invalid-as-stated (unverifiable historical measurements) | No cited audit artifact or reproducer in epic; repository-wide study/doc search finds no recorded 3,393,794/702,018/3,714,853 measurements. Current payload/file totals cannot verify 2026-10-20 facts. Remove fixed volatile counts from plan premises; keep measured current semantics. |
| B11 | List's production uses are search, labels, dependency titles; graph contains needed summary | valid-and-open | `App.tsx:64`, `:94`, `:292`; `IssueDetail.tsx:426`; `routes.rs:345`. Search's full descriptions must come from core search after migration, not graph. |
| B12 | Async handlers perform blocking filesystem/Git/ripgrep calls inline | valid-and-open | Direct sync calls in `routes.rs:232`, `:460`, `:740`, `:925`; no offload helper or spawn_blocking/block_in_place anywhere in server source. |
| B13 | Three issue search mechanisms exist; JSON text grep matches representation | valid-and-open | `commands/search.rs:65`; `routes.rs:468`; `clientSearch.ts:44`. JSON-key/escape matching follows mechanically from ripgrep over serialized files. |
| B14 | CLI issue search uses a domain search_issues function | invalid-as-stated | It calls QueryOps in command layer (`dispatch/issue.rs:146`; `commands/search.rs:53`); no corresponding function under domain. |
| B15 | CLI unpinned HEAD versus server working-tree yields differing edited content | valid-and-open | `commands/document.rs:590` versus `:719`–`:727`. Git epic already owns correction; this epic consumes it. |
| B16 | CLI/server history and diff copies exist | valid-and-open | `document.rs:613`, `:636`, `:777`, `:810`. Git epic producer does not itself eliminate both public families. |
| B17 | Public document dead_code allowances and explanation are stale | valid-and-open | `document.rs:701`–`:707`, `:743`, `:776`, `:809`; routes call these methods (`routes.rs:740`, `:769`, `:925`, `:972`). A public exported function's external crate caller is not invisible dead code needing suppression. Prior study repeats the old rationale. |
| B18 | Four routes and three client methods have no production consumer | invalid-as-stated | No in-tree production callers found for listed methods/routes; their test callers exist. External public HTTP consumers are unobservable from this repository. Also diff route lacks in-tree production caller. Scope correction in REQ-10. |
| B19 | StatusResponse drops gated/rejected | valid-and-open | `routes.rs:379` versus `commands/mod.rs:2126` (core has both at `:2130`, `:2132`). |
| B20 | Binary redeclares four library modules, repeating tests | valid-and-open | `main.rs:6`–`:10`; `lib.rs:6`–`:13`; Rust compiles each crate target's module tree independently. |
| B21 | Server git2 and tower dependencies unused | valid-and-open | `crates/server/Cargo.toml:43`, `:62`; whole crates/server search finds no source use of either. This is source evidence, not a dependency-auditor run. |
| B22 | Three repeated raw-response blocks | valid-and-open | `routes.rs:772`, `:822`, `:875`: MIME, optional HTML base injection, body, Content-Type, CSP. |
| B23 | Handwritten MIME table beside mime_guess; two percent codecs | valid-and-open | `routes.rs:521`, `:551`; `embedded.rs:21`; mime_guess is used by embedded and declared optional (`Cargo.toml:22`, `:64`). |
| B24 | MCP success/data unwrapping is stale for CLI success payloads | valid-and-open, conditional | `cli-executor.js:52`–`:55`; core dispatch emitter serializes registered response bodies. Distinguish CLI compatibility unwrapping from MCP bridge's own local result envelope (`cli-executor.js:178`), which index still consumes (`index.js:290`, `:303`). |
| B25 | MCP state instructions omit gated and archived | valid-and-open, conditional | `index.js:41`; schema already derives full State enum (`schema.rs:528`). |
| B26 | MCP timeout verbs are hand-maintained command facts | valid-and-open, conditional | `cli-executor.js:158`; schema Command currently has no timeout/execution field (`schema.rs:40`). |
| B27 | MCP internal fields leak in tool list; calls do linear lookup | valid-and-open, conditional | `tool-generator.js:92`, `:94`; `index.js:208`, `:241`, plus schema-tree lookup `:219`. |
| B28 | Limiter release admits fresh caller before queued waiter | valid-and-open, conditional | `concurrency.js:29`–`:44`. Node event-loop ordering permits this; deterministic test can arrange fresh admission before setImmediate waiter. |
| B29 | hierarchy.ts is unused | valid-and-open | File exists; whole web/src import sweep finds no import of `types/hierarchy`. `models.ts` is the actual shared type source used by `api/client.ts:2`. |
| B30 | Subgraph container selection hardcodes level 2 | valid-and-open | `subgraphClustering.ts:62`–`:66`; production `clusteredGraphLayout.ts:57` calls it. |
| B31 | Layout/refresh repeatedly search arrays | valid-and-open | `GraphView.tsx:162`, `:165`, `:246`, `:833`; `clusterAwareLayout.ts:90`–`:103`. |
| B32 | D-17 supports browser-count independent refresh | invalid-as-stated | Charter D-17 is synced redo-record mutation publication independent of project history; D-20 is the relevant one-computation principle. Resolved with `jit item show`. |
| B33 | hidden_commands duplicates MCP curation policy | valid-and-open, conditional | `schema.rs:288`; `mcp-server/curated-tools.json`; generator reads manifest (`tool-generator.js:14`). This is listing policy, not clap hidden-command extraction (`schema.rs:347`). |

Background totals: 3 already-done; 24 valid-and-open (6 MCP-conditional); 6 invalid-as-stated (B10 historical measurement unverifiable).

## Prior art and owner boundaries

- `dev/active/df129d9e-rust-code-quality/plan.md:498` expressly says that epic produces bootstrap and executor search while this epic adopts/replaces computation; `:497` chooses shared classification with server-owned HTTP projection; `:499` defers duplicated hidden_commands policy. Current code confirms bootstrap/search/classification are already landed. Its investigation `:129`–`:135` documents pre-fix status mappings; do not reimplement them from that stale inventory.
- `dev/active/f4d9502a-git-cli-module/investigation.md:458` already records document convergence overlap. Its plan `:119`–`:123` produces document versions/history/diff through storage Git. Parent identifies producer issue IDs a273667c, 1acfa592, 83c8632f, 933ff852, d48e28bf; do not repeat their Git implementation. Git selected-root migration c5062e21 requires reconciliation with bootstrap adoption.
- Peer f35c86de owns removing dispatch storage access; do not expand this epic into a general dispatch/storage cleanup. Active fb38bf1f moves result types output to commands; active 487f45a9 owns RequiresGitError/ErrorCode schema enumeration. Their authoritative output should be consumed by server response/error work.
- `dev/architecture/cli-and-mcp-strategy.md:22`–`:30` describes CLI-first plus thin MCP wrapper; `:43`–`:52` proposes the old success/data envelope, explaining stale bridge code. This 2025 planning document is prior art, not current CLI contract.
- `dev/studies/clippy-suppressions.md:54`–`:68` records the public-server-callers rationale for document dead_code suppression. Current cross-crate calls disprove the need; update or archive this current study if suppression scope is adopted.
- `dev/studies/multi-agent-parallelism-analysis.md:115`–`:123` records the previously missed MCP gate coverage and why MCP changes must use mcp-ci when retained. Do not mistake web npm-ci for bridge validation.
- `dev/archive/1cc809de-repository-state-quality/1cc809de-plan.md:222` contains the earlier MCP schema-warning cleanup; the archived July audit at `studies/cdc840ad-audit-2026-07-23.md:174` identifies corresponding advisory debt. Neither verifies October response-size claims.

## Primitive verification and architectural fit

| Property or reusable primitive | Confirmed / contradicted and evidence |
|---|---|
| Shared constructor validates before handler use | Confirmed existing boundary: `commands/bootstrap.rs:71` constructs once; worktree paths `:77`, layout `:79`, storage `:91`, executor `:121`, validation `:127`. Server must choose invocation facts explicitly, preserving startup recovery before validation (`server/src/lib.rs:80`–`:90`). Read-only bootstrap with opens_mutation_session=false does not recover (`bootstrap.rs:107`); blindly using that mode would lose current server startup recovery. |
| Binding rejects all incompatible layouts | Contradicted for configure_repository_layout (`json.rs:923`, `:938`); confirmed for mutation session's normalized root checks (`repository_state_store.rs:509`–`:536`). A Result-returning correction requires all implementors/callers, inventoried below; avoid alternate checked setter. |
| Tracker makes a capture coherent | Contradicted: atomic counter/broadcast (`watcher.rs:41`, `:52`) and delayed filesystem event debouncing (`:142`) are not a storage read transaction. Reuse storage's lock-free session contract for consistency if needed (`storage/repository_state_store.rs:307`, `:327`), respecting D-22; do not hold mutation/write locks across read refresh or async await. |
| At most once for concurrent clients | Not supplied by current code: no cache, no in-flight admission state (`routes.rs:116`). A plain check-then-compute cache permits concurrent duplicates. Cache admission must share a pending computation per generation; do not add per-client repositories/executors. |
| Shared HTTP error classification | Already confirmed `routes.rs:74` invokes `jit::output::classify`; `status_of` is exhaustive. Reuse, and allow peer result/error ownership changes to settle. Executor layers must not import StatusCode or spawn concerns. |
| Shared core issue search | Existing executor entry point confirmed; pure domain predicate absent. `search_content` also currently ripgreps storage root (`commands/search.rs:25`), so changing only `/search` leaves a second issue-text mechanism in the top-level CLI content-search path. Its documented/default scope needs explicit decision (inventory below). |
| Linked-document ripgrep only | Server obtains explicit/derived paths (`routes.rs:441`–`:445`) and guards empty list (`:449`); retain that guard because an empty glob list would search all files. Generic CLI content search is broader today. Paths/linked-document enumeration belongs to command/storage orchestration, not pure matcher. |
| Shared document bytes | Server `DocumentOps::read_document_bytes` (`document.rs:744`) calls storage (`:764`) preserving raw bytes; text method (`:708`) calls read_path_text. Respect linked-document validation and explicit/pinned/working-tree precedence (`:719`, `:755`); do not route binary assets through lossy String. |
| Domain-agnostic hierarchy | Core hierarchy resolves configured types (`routes.rs:338`–`:340`); web level 2 literal is a policy leak (`subgraphClustering.ts:66`). Indexing changes can reuse server-supplied parent/cluster relationships rather than invent hierarchy from labels. |
| MIME, codecs and raw response | Existing MIME/percent machinery should be replaced, not wrapped by parallel implementations. Percent decoding is part of embedded traversal/API-route rejection (`embedded.rs:48`–`:65`); preserving these errors matters. Raw HTML base/CSP behavior is a transport concern in server (`routes.rs:774`–`:794`). |
| Bounded build footprint | Importing library modules in main removes duplicate module/test compilation (`main.rs:6`, `lib.rs:6`). Extend existing server unit/integration suites and web tests; no new Cargo target is needed. |

Resolved project context: @/charter/D-18, D-20, D-17, D-21 and @/inv/convention-convergence, architecture-dependency-direction, single-source-prose, canonical-cutover, domain-agnostic, bounded-rust-build-footprint. The mandatory boundaries are also in `AGENTS.md` under Core Architecture and Project Invariants. Keep pure matching/index derivation free of filesystem/HTTP/process I/O; storage owns paths/Git; command services orchestrate; server owns async execution and HTTP shaping; web owns presentation and interactions. No epic/story/type names may become engine defaults. Derive state/schema facts from canonical semantic types, not copied lists.

## MCP scope evidence and options (held)

`mcp-server/index.js:160` declares only tools capability, no resources/prompts or independent domain service. Every accepted call resolves CLI schema (`:219`), validates tool input (`:262`) and runs `executeCommand` (`:286`), which uses child_process.execFile on jit (`cli-executor.js:33`). No independent issue operation exists. Unique production interface is stdio MCP discovery/transport (`index.js:10`–`:20`, server connection at end of file), curated default listing (`:193`), schema-driven input validation and MCP result shaping (`:139`). `docs/how-to/mcp-integration.md` documents clients using that transport. Its compaction can truncate values (`index.js:93`), and timeout/concurrency policies are wrapper behaviors, not capabilities unavailable from CLI.

Options for steward/user: retain a deliberately minimal transport adapter with REQ-13/14; retire MCP and its package/audits/CI/docs/schema-only policy through a superseding charter decision; or defer its delivery while completing server/web. Evidence supports removal of duplicate domain machinery regardless. This report does not recommend retention against the user's scope question. Retirement preserves CLI domain capabilities but removes MCP-only client access; D-9's explicit surface promise must be superseded. Remove MCP-only hidden metadata rather than adding schema fields if retirement wins; do not remove schema itself, which serves CLI introspection and other consumers.

Retirement footprint includes the entire mcp-server tree (including tests, fixture/schema cache, package-lock), external references inventoried below, repository-local mcp-ci gate/config and associated active work, release packaging/publish inputs, security audit job, workflow-contract declarations/fixtures, release version checks, canonical-home registry, installation/quickstart/overview docs, README/INSTALL/CHANGELOG and contributor instructions. Historical archives are evidence and normally remain historical; live authoritative projections are changed at their source. The consumer inventory includes archives to make that distinction visible, not to instruct rewriting history.

## Consumer sweeps

The following inventories are generated from whole-tree searches including hidden repository configuration, examples, fixtures, tests, and docs. Each entry records every matching file and its first matching line; within-file additional uses are owned by that file's migration. Exclusions are generated dependency/build outputs (`target`, `node_modules`, dist), git object storage, and this investigation to avoid self-reference. A filename in an inventory is an inspect/migrate candidate, not proof of a production call. The route/client inventory below separately identifies real production consumers.

### Production route/client map

| API surface | In-tree production consumer |
|---|---|
| /health | `web/src/App.tsx:74`, server launch health probes in `crates/jit/src/commands/serve.rs`, container/systemd guidance in `docs/how-to/deployment.md:247`; health must remain. |
| /issues | `web/src/App.tsx:91`; only refresh/list consumer, removed by REQ-05/08. Decide supported API scope before deleting. |
| /issues/:id | `web/src/components/Issue/IssueDetail.tsx:198` via getIssue. |
| /graph | `web/src/components/Graph/GraphView.tsx:582` via getGraph. |
| /status | No in-tree production call; getStatus declaration only (`api/client.ts:36`) plus route tests. External use unknown. |
| /search | `web/src/components/Search/useSearch.ts:55`, via apiClient.search. |
| /documents and issue document content | `web/src/components/Document/DocumentViewer.tsx:98`, `:100`. |
| /documents/raw and issue document raw | getRawDocumentUrl (`web/src/api/client.ts:157`); consumed by DocumentViewer/renderers, including downloadable/binary content. |
| /raw/*path | Injected HTML base URLs from `routes.rs:595`, reached by browser sibling asset loads; generated production caller, not a literal apiClient method. |
| issue document history | `DocumentViewer.tsx:310`. |
| issue document diff | No in-tree production call; client method at `api/client.ts:74`, tests at `api/client.test.ts:127`, Rust integration `document_api_tests.rs:112`. External use unknown. |
| /gates | `IssueDetail.tsx:122` via listGates. |
| /gates/:key | No in-tree production call; client method declaration at `api/client.ts:114`. External use unknown. |
| issue gate runs/detail | `IssueDetail.tsx:150`, `:168`, `:185`. |
| /config/hierarchy | `GraphView.tsx:564`. |
| /config/namespaces | `App.tsx:84`. |
| /config/strategic-types | No in-tree production call or apiClient method. External use unknown. |
| /changes | No in-tree production caller; tests at `routes.rs:2603`, `:2633`. SSE is the active path. |
| /events/stream | `web/src/hooks/useEventStream.ts` (direct EventSource, independent of apiClient). |

All apiClient methods are covered by this table, including tests-only methods and generated URL helpers. Graph metadata consumers need a full-graph source before filtering: App label set and IssueDetail dependency names currently rely on allIssues, whereas GraphView applies label/visibility filters during load. Search result click behavior must retain issue navigation and linked-document opening (`App.tsx:134` onward; `useSearch.ts:98` onward). Search completion must guard obsolete requests when query/version changes; current debouncing cancels only timers (`useSearch.ts:75`–`:80`), not already in-flight responses.

## Validation limits

Read-only source/config/doc inspection and address resolution only; no product implementation, executable tests, build, gate evaluation, or JIT mutation was performed. File/line citations reflect this shared working tree; peer work may move them. `rg` probes initially referenced nonexistent guessed paths (`web/src/components/GraphView.tsx`, `web/src/hooks/useSearch.ts`, `crates/jit/src/dispatch/document.rs`, `crates/jit/src/repository_state/virtual_path.rs`) and a zsh-unmatched schema glob; all were corrected by `rg --files`/actual paths and no finding depends on a failed probe. Historical response-byte measurements could not be independently verified, as B10 states. Concurrency defect is established from scheduling semantics, not an executed reproducer. Source-only unused-dependency findings require normal build/lint/test checks when implementing.

## Complete matching-file inventories

Gate-run output, event-log history and derived index are additionally excluded from these source/consumer inventories; their matching text is execution evidence, not a code/document consumer. Individual issue records are included because they can carry active ownership constraints.

### Repository construction, layout binding and watcher consumers

Matched files: 97.

- `.jit/issues/030df4b1-708f-4139-ba3c-3890fd6c27ba.json:10`
- `.jit/issues/0fa1e09e-8014-413e-ad89-44ce5d535806.json:7`
- `.jit/issues/12c0f3fb-fde6-4504-aac0-773b435f0a83.json:9`
- `.jit/issues/30334066-4bae-49fb-b10a-2282c07a7344.json:9`
- `.jit/issues/943abd0b-8279-457a-a792-74c013edd0a4.json:8`
- `.jit/issues/a7314585-a227-4a97-9127-d3228736aa9e.json:10`
- `.jit/issues/a807e699-a7ed-474e-b6af-2ed4086a9dad.json:9`
- `.jit/issues/c2b3cefa-d690-4c6c-94dd-feaa3156acea.json:10`
- `.jit/issues/c5062e21-331d-4ab8-8468-3535d16d634f.json:8`
- `.jit/issues/eaca3f52-ed73-4127-8a5f-829aa5d2ca72.json:9`
- `.jit/issues/eb5b6ffe-f47c-4405-97c9-59b1b3b05bf2.json:7`
- `crates/jit/src/commands/bootstrap.rs:19`
- `crates/jit/src/commands/mod.rs:79`
- `crates/jit/src/config_manager.rs:402`
- `crates/jit/src/dispatch/init.rs:6`
- `crates/jit/src/dispatch/mod.rs:19`
- `crates/jit/src/storage/artifact_planning.rs:482`
- `crates/jit/src/storage/issue_store_conformance.rs:74`
- `crates/jit/src/storage/json.rs:304`
- `crates/jit/src/storage/mod.rs:183`
- `crates/jit/src/storage/repository_state_store.rs:621`
- `crates/jit/src/storage/repository_state_store_tests.rs:229`
- `crates/jit/tests/fast_docs_templates/template_apply_atomicity_tests.rs:132`
- `crates/jit/tests/fast_rules/local_rule_enforcement_tests.rs:1035`
- `crates/server/src/lib.rs:76`
- `crates/server/src/main.rs:25`
- `crates/server/src/routes.rs:27`
- `crates/server/src/sse.rs:12`
- `crates/server/src/watcher.rs:25`
- `crates/server/tests/server_integration/document_api_tests.rs:9`
- `dev/TESTING.md:539`
- `dev/active/b697a3f7-one-mutation-path/breakdown.json:258`
- `dev/active/b697a3f7-one-mutation-path/plan.md:330`
- `dev/active/df129d9e-rust-code-quality/breakdown.json:3596`
- `dev/active/df129d9e-rust-code-quality/handoff-10.md:25`
- `dev/active/df129d9e-rust-code-quality/investigation.md:706`
- `dev/active/f4d9502a-git-cli-module/breakdown.json:197`
- `dev/active/f4d9502a-git-cli-module/investigation.md:43`
- `dev/active/f4d9502a-git-cli-module/plan.md:34`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/post-change-test-inventory.json:12982`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:13083`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/post-change-test-inventory.json:12982`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:13083`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-17.md:39`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-9.md:27`
- `dev/archive/f1dc843a-redo-mutations/active/investigation.md:433`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/post-change-test-inventory.json:12982`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:13083`
- `dev/archive/f8a6b298-history-independent-reads/active/f8a6b298-history-independent-reads/breakdown.json:697`
- `dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/candidate-build/pre-change-test-inventory.json:17810`
- `dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/opt0-build/pre-change-test-inventory.json:17810`
- `dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/candidate-build/pre-change-test-inventory.json:17871`
- `dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/opt0-build/pre-change-test-inventory.json:17858`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/inventory/stdout.log:1`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stderr.log:4416`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stdout.log:8820`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stderr.log:4402`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stdout.log:8806`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stderr.log:4423`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stdout.log:8832`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-1.xml:162`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-2.xml:154`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-3.xml:156`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-1.out:86`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-2.out:82`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-3.out:86`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-1.xml:224`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-2.xml:224`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-3.xml:224`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-1.xml:226`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-2.xml:224`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-3.xml:226`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-1.out:118`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-2.out:117`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-3.out:118`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-1.out:118`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-2.out:117`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-3.out:117`
- `dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json:12982`
- `dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:13083`
- `dev/benchmarks/suite-profile.json:929`
- `dev/benchmarks/test-profile-123efe74/opt0-build/pre-change-test-inventory.json:17843`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.jsonl:8831`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.stderr:4422`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.jsonl:8831`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.stderr:4422`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.jsonl:8832`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.stderr:4422`
- `dev/benchmarks/test-profile-123efe74/opt1-build/pre-change-test-inventory.json:17830`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.jsonl:8832`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.stderr:4511`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.jsonl:8832`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.stderr:4511`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.jsonl:8832`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.stderr:4512`
- `dev/benchmarks/test-profile-efaec52f/opt0-build-screen/pre-change-test-inventory.json:17843`
- `dev/benchmarks/test-profile-efaec52f/scoped-build-screen/pre-change-test-inventory.json:17794`

### Issue/content search contracts and consumers

Matched files: 120.

- `.jit/issues/10751360-4148-410a-bcae-dfe9c1ee1282.json:12`
- `.jit/issues/11682766-316e-4fd7-9bbf-eaddf7c01b04.json:7`
- `.jit/issues/180e7f33-2d61-4e21-8548-fc533e06e400.json:7`
- `.jit/issues/2c018a6e-9ece-43fe-971b-4d080ea74ab3.json:9`
- `.jit/issues/34999aad-b3c6-4664-b80d-bf5a548f9200.json:7`
- `.jit/issues/3b033738-a6ef-40ab-8fc1-d296f3fbb863.json:9`
- `.jit/issues/4597743e-5685-46b5-a1f1-ff8e8bfa3e0a.json:9`
- `.jit/issues/53e3fa36-c0cb-4206-8e8e-0a21aafb213e.json:12`
- `.jit/issues/578ed10d-29f9-4b25-87c0-b3c50a1e9d6e.json:11`
- `.jit/issues/650e6a56-e3fe-4169-8c2b-58229c9aaeca.json:9`
- `.jit/issues/68c1f2f3-6620-4770-b6f3-395cc38219b7.json:9`
- `.jit/issues/7a7d0bfe-8f5d-4f04-a9a9-0470d7957b27.json:9`
- `.jit/issues/7a8773ca-c9a5-4edd-8e2e-b7dfcfc63ab8.json:10`
- `.jit/issues/98cc2097-026d-4744-aeaa-82532910328e.json:10`
- `.jit/issues/a3a99cf8-5c62-4c91-a890-7404d9cfe9c9.json:9`
- `.jit/issues/b58ea1e2-c53b-4743-a548-2fe0e06b2965.json:10`
- `.jit/issues/b5faef84-fd5b-477d-be87-2cf020e93fa4.json:7`
- `.jit/issues/ba3e3884-ca9c-4a51-8ced-2fb833050193.json:7`
- `.jit/issues/ba4c6054-4117-4ab8-8810-d5b74dea0a35.json:10`
- `.jit/issues/c0d0c8bf-679f-463b-87ca-25cae8945646.json:7`
- `.jit/issues/d05b5639-925e-414d-8abb-a639331d76f1.json:9`
- `.jit/issues/df129d9e-c795-4d92-9ee5-ffd776339252.json:13`
- `.jit/issues/e5d618ee-a23a-4857-baa9-e0d5188b8614.json:9`
- `.jit/issues/ed5241de-d5b7-47ea-b156-58d978b42eeb.json:4`
- `.jit/issues/f8a6b298-e6b5-4d62-b73c-3f7b217c1a01.json:13`
- `AGENTS.md:74`
- `CHANGELOG.md:1009`
- `crates/jit/src/cli.rs:795`
- `crates/jit/src/commands/search.rs:4`
- `crates/jit/src/dispatch/issue.rs:17`
- `crates/jit/src/dispatch/search.rs:1`
- `crates/jit/src/output.rs:53`
- `crates/jit/src/output/failure.rs:363`
- `crates/jit/src/output/issue.rs:7`
- `crates/jit/src/output/search.rs:1`
- `crates/jit/src/schema.rs:1739`
- `crates/jit/src/search.rs:48`
- `crates/jit/tests/cli_issue/issue_search_json_tests.rs:1`
- `crates/jit/tests/cli_issue/issue_search_label_filter_tests.rs:1`
- `crates/jit/tests/fast_docs_templates/schema_tests.rs:142`
- `crates/jit/tests/fast_rules/label_query_tests.rs:47`
- `crates/server/src/routes.rs:23`
- `dev/active/9db27a3a-v1-0/9db27a3a-progress.json:430`
- `dev/active/b697a3f7-one-mutation-path/breakdown.json:586`
- `dev/active/df129d9e-rust-code-quality/breakdown.json:661`
- `dev/active/df129d9e-rust-code-quality/handoff-11.md:71`
- `dev/active/df129d9e-rust-code-quality/investigation.md:21`
- `dev/active/df129d9e-rust-code-quality/plan.md:151`
- `dev/architecture/cli-and-mcp-strategy.md:493`
- `dev/architecture/web-ui-architecture.md:89`
- `dev/archive/14303b30-phase5-2/active/json-output-standardization-plan.md:193`
- `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/plugin/highlight/highlight.js:1`
- `dev/archive/2195c98f-semantic-types/active/completion-report.md:26`
- `dev/archive/25064508-structured-knowledge/features/25064508/plan.md:51`
- `dev/archive/53e3fa36-agent-ergonomics/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md:24`
- `dev/archive/53e3fa36-agent-ergonomics/studies/session-mining-jit-improvements.md:77`
- `dev/archive/5fe00921-production-stability/plans/benchmarks-8d80b5dd.md:96`
- `dev/archive/71373e37-docs-lifecycle/studies/documentation-lifecycle-strategy.md:212`
- `dev/archive/8376299c-audit-correctness/active/repro-verification.md:62`
- `dev/archive/8e071e18-dev-artifact-layout/presentations/vendor/reveal.js/plugin/highlight/highlight.js:1`
- `dev/archive/9876a23a-one-validation-report/active/handoff.md:72`
- `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md:16`
- `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-plan.md:25`
- `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-research.md:48`
- `dev/archive/9d427a6b-production-polish/active/json-output-standardization-plan.md:193`
- `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md:94`
- `dev/archive/9d427a6b-production-polish/design/search-focus-navigation.md:15`
- `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md:397`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/breakdown.json:3087`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/investigation.md:282`
- `dev/archive/f3f7de97-guards-and-ergonomics/active/completion-report.md:29`
- `dev/archive/f8a6b298-history-independent-reads/active/f8a6b298-history-independent-reads/breakdown.json:1543`
- `dev/archive/f8a6b298-history-independent-reads/active/f8a6b298-history-independent-reads/investigation.md:452`
- `dev/archive/features/search-implementation.md:83`
- `dev/archive/studies/document-search-performance.md:85`
- `dev/archive/studies/getting-started-complete.md:404`
- `dev/benchmarks/nextest-concurrency-934985ca/diagnostics/warmup-interrupted-client-timeout/stderr.log:1968`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w24-1/stderr.log:1962`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stderr.log:1947`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stderr.log:1937`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stderr.log:1958`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-1.xml:1064`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-2.xml:1066`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-3.xml:1062`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-1.out:537`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-2.out:538`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-3.out:539`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-1.xml:1012`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-2.xml:1012`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-3.xml:1014`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-1.xml:1014`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-2.xml:1010`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-3.xml:1010`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-1.out:512`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-2.out:510`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-3.out:510`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-1.out:512`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-2.out:511`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-3.out:512`
- `dev/benchmarks/suite-profile.json:15266`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.stderr:1958`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.stderr:1956`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.stderr:1958`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.stderr:1952`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.stderr:1952`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.stderr:1954`
- `dev/studies/jit-vs-agentic-trends-2026.md:46`
- `dev/studies/research-workflow-examples.md:542`
- `dev/vision/document-graph-implementation-plan.md:782`
- `dev/vision/knowledge-management-vision.md:149`
- `docs/reference/cli-commands.md:1167`
- `docs/reference/storage-format.md:215`
- `test-vectors/container-image/test_image_contract.py:384`
- `web/src/App.tsx:10`
- `web/src/api/client.ts:41`
- `web/src/components/Search/useSearch.test.ts:2`
- `web/src/components/Search/useSearch.ts:2`
- `web/src/types/models.ts:139`
- `web/src/utils/clientSearch.test.ts:2`
- `web/src/utils/clientSearch.ts:3`

### Document method families, path/raw APIs and MIME/codec consumers

Matched files: 133.

- `.jit/issues/030df4b1-708f-4139-ba3c-3890fd6c27ba.json:10`
- `.jit/issues/1acfa592-ce5a-4b73-ad86-a82881f1208f.json:9`
- `.jit/issues/26cbc32e-975d-472c-8d5c-27689577b253.json:9`
- `.jit/issues/408ed86a-12a5-45ec-8623-5739e14b1cb7.json:4`
- `.jit/issues/75b1ccb8-6796-41da-9a96-fa95ed96cc94.json:7`
- `.jit/issues/83c8632f-5495-4fef-a774-730ebeae95be.json:11`
- `.jit/issues/933ff852-7cc9-4902-a424-626b882df531.json:9`
- `.jit/issues/984cb1fa-85af-474a-b64c-d21e3ec6cd0c.json:9`
- `.jit/issues/985c2b15-b57f-47d5-b21e-6ca6b979667b.json:9`
- `.jit/issues/a273667c-6902-46f2-9580-b74e1e8abac3.json:10`
- `.jit/issues/a8f2f04b-f5e4-4cf7-9af4-7d714a2f7139.json:9`
- `.jit/issues/eeafaa78-290d-4390-9097-4d29ff48ef82.json:10`
- `.jit/issues/f766b092-5eec-4537-b391-ae7017965835.json:10`
- `crates/jit/src/commands/document.rs:557`
- `crates/jit/src/commands/mod.rs:2157`
- `crates/jit/src/dispatch/doc.rs:6`
- `crates/jit/src/output/doc.rs:5`
- `crates/jit/tests/cli_repo_workflow/mod.rs:13`
- `crates/server/src/embedded.rs:21`
- `crates/server/src/routes.rs:144`
- `crates/server/tests/server_integration/document_api_tests.rs:97`
- `dev/active/b697a3f7-one-mutation-path/breakdown.json:892`
- `dev/active/df129d9e-rust-code-quality/breakdown.json:1783`
- `dev/active/df129d9e-rust-code-quality/investigation.md:131`
- `dev/active/f4d9502a-git-cli-module/breakdown.json:1654`
- `dev/active/f4d9502a-git-cli-module/investigation.md:242`
- `dev/archive/14303b30-phase5-2/sessions/session-2025-12-21-short-hash-progress.md:37`
- `dev/archive/14303b30-phase5-2/studies/short-hash-implementation-plan.md:131`
- `dev/archive/3d9a0ce3-lead-friction/active/investigation-children.md:156`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/post-change-test-inventory.json:7255`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:7507`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json:321`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json:184`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/post-change-test-inventory.json:7255`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:7507`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json:321`
- `dev/archive/71373e37-docs-lifecycle/active/documentation-lifecycle-design.md:44`
- `dev/archive/71373e37-docs-lifecycle/active/snapshot-export-implementation-plan.md:857`
- `dev/archive/94f873c8-docs-lifecycle-p2/active/5c060496-raw-assets-design.md:32`
- `dev/archive/94f873c8-docs-lifecycle-p2/active/abfd6016-multi-format-doc-rendering-design.md:19`
- `dev/archive/9d427a6b-production-polish/features/d0b85bff-archive-json-plan.md:54`
- `dev/archive/9d427a6b-production-polish/refactorings/f766b092-remove-printing-plan.md:15`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/breakdown.json:5443`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/post-change-test-inventory.json:7255`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:7507`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json:321`
- `dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/candidate-build/pre-change-test-inventory.json:3831`
- `dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/opt0-build/pre-change-test-inventory.json:430`
- `dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/candidate-build/pre-change-test-inventory.json:903`
- `dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/opt0-build/pre-change-test-inventory.json:1003`
- `dev/benchmarks/nextest-concurrency-934985ca/diagnostics/warmup-interrupted-client-timeout/stderr.log:3437`
- `dev/benchmarks/nextest-concurrency-934985ca/diagnostics/warmup-interrupted-client-timeout/stdout.log:6841`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/inventory/stdout.log:1`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w24-1/stderr.log:3432`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w24-1/stdout.log:6841`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stderr.log:3449`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stdout.log:6828`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stderr.log:3445`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stdout.log:6815`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stderr.log:3435`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stdout.log:6841`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-1.xml:7526`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-2.xml:7524`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-3.xml:7522`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-1.out:3763`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-2.out:3762`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-3.out:3764`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-1.xml:7811`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-2.xml:7807`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-3.xml:7807`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-1.xml:7807`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-2.xml:7807`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-3.xml:7807`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-1.out:3901`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-2.out:3901`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-3.out:3901`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-1.out:3904`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-2.out:3901`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-3.out:3901`
- `dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json:7255`
- `dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:7507`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-clean-1/cargo-test-list.json:534`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-clean-1/test-no-run.log:316`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-clean-2/cargo-test-list.json:408`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-clean-2/test-no-run.log:316`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-clean-3/cargo-test-list.json:416`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-clean-3/test-no-run.log:316`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-rebuild-1/rebuild-test-no-run.log:41`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-rebuild-1/setup-test-no-run.log:316`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-rebuild-2/rebuild-test-no-run.log:41`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-rebuild-2/setup-test-no-run.log:316`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-rebuild-3/rebuild-test-no-run.log:41`
- `dev/benchmarks/rust-build-efficiency/raw/baseline-v2-rebuild-3/setup-test-no-run.log:316`
- `dev/benchmarks/rust-build-efficiency/raw/clean-1/cargo-test-list.json:420`
- `dev/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json:321`
- `dev/benchmarks/rust-build-efficiency/raw/clean-1/test-no-run.log:317`
- `dev/benchmarks/rust-build-efficiency/raw/clean-2/test-no-run.log:317`
- `dev/benchmarks/rust-build-efficiency/raw/clean-3/test-no-run.log:317`
- `dev/benchmarks/rust-build-efficiency/raw/rebuild-1/rebuild-test-no-run.log:42`
- `dev/benchmarks/rust-build-efficiency/raw/rebuild-1/setup-test-no-run.log:317`
- `dev/benchmarks/rust-build-efficiency/raw/rebuild-2/rebuild-test-no-run.log:42`
- `dev/benchmarks/rust-build-efficiency/raw/rebuild-2/setup-test-no-run.log:317`
- `dev/benchmarks/rust-build-efficiency/raw/rebuild-3/rebuild-test-no-run.log:42`
- `dev/benchmarks/rust-build-efficiency/raw/rebuild-3/setup-test-no-run.log:317`
- `dev/benchmarks/suite-profile.json:74`
- `dev/benchmarks/test-profile-123efe74/opt0-build/pre-change-test-inventory.json:16381`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.jsonl:6839`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.stderr:3425`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.jsonl:6839`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.stderr:3423`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.jsonl:6839`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.stderr:3423`
- `dev/benchmarks/test-profile-123efe74/opt1-build/pre-change-test-inventory.json:1075`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.jsonl:6839`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.stderr:3426`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.jsonl:6840`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.stderr:3423`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.jsonl:6840`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.stderr:3424`
- `dev/benchmarks/test-profile-efaec52f/opt0-build-screen/pre-change-test-inventory.json:2363`
- `dev/benchmarks/test-profile-efaec52f/scoped-build-screen/pre-change-test-inventory.json:14875`
- `dev/studies/clippy-suppressions.md:57`
- `dev/vision/document-viewer-implementation-plan.md:62`
- `dev/vision/knowledge-management-vision.md:270`
- `web/src/api/client.test.ts:78`
- `web/src/api/client.ts:2`
- `web/src/components/Document/DocumentViewer.test.tsx:23`
- `web/src/components/Document/DocumentViewer.tsx:3`
- `web/src/components/Document/renderers/HtmlRenderer.test.tsx:7`
- `web/src/components/Document/renderers/HtmlRenderer.tsx:2`
- `web/src/components/Document/renderers/ImageRenderer.test.tsx:7`
- `web/src/components/Document/renderers/ImageRenderer.tsx:2`
- `web/src/types/models.ts:168`

### Web list/graph/layout/type consumers, including tests

Matched files: 74.

- `.jit/issues/0ea4f9a8-c1ee-424d-b4a8-12197b2b8cc4.json:9`
- `.jit/issues/0f8efc37-8054-420d-9409-dde888ea0040.json:9`
- `.jit/issues/16c53cc2-f324-4e1e-8881-039bce61d96d.json:10`
- `.jit/issues/1a3db2c3-0faa-42c9-9498-d7bf7c0efaee.json:7`
- `.jit/issues/1aed41f5-bd08-4d01-81cc-032810ce6bd2.json:7`
- `.jit/issues/44d6f247-8ae1-48f2-9ebd-8f5af86fb4ce.json:10`
- `.jit/issues/63c11d08-054e-471a-9b0c-491c30b8d8a1.json:9`
- `.jit/issues/783a8086-2933-4b6a-9d4a-b5c5f1d3ddc0.json:6`
- `.jit/issues/842b0ac6-8a8b-4c20-b8dc-f39d0d9b083b.json:10`
- `.jit/issues/90575086-fcfe-4b0d-82b5-fc00c4daace7.json:7`
- `.jit/issues/acb38457-0d4a-47e6-b8ea-cab3a92b7676.json:10`
- `.jit/issues/be18da9e-bfe2-4813-a69e-72ee1904c344.json:9`
- `.jit/issues/ce2bdbb0-ef3b-4074-a0ba-71070cc555f3.json:9`
- `.jit/issues/d91ea408-3ed9-4021-9f9b-5ee736474c5f.json:9`
- `.jit/issues/dc6fac0c-f449-47fc-b2f3-29f756fad4b5.json:9`
- `.jit/issues/f368ce3d-2049-4171-b0ec-927bbfaf5321.json:9`
- `crates/jit/src/commands/template_expand.rs:421`
- `crates/jit/src/domain/types.rs:770`
- `crates/jit/src/graph/hierarchy.rs:145`
- `crates/jit/src/graph/mod.rs:6`
- `crates/jit/tests/fast_docs_templates/template_apply_tests.rs:1850`
- `crates/server/src/routes.rs:270`
- `dev/architecture/graph-filtering-architecture.md:26`
- `dev/architecture/web-ui-architecture.md:168`
- `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/plugin/highlight/highlight.js:1`
- `dev/archive/2195c98f-semantic-types/active/breakdown.json:56`
- `dev/archive/2195c98f-semantic-types/active/investigation.md:56`
- `dev/archive/2195c98f-semantic-types/active/plan.md:94`
- `dev/archive/2195c98f-semantic-types/active/raw-identity-sites.md:27`
- `dev/archive/5fe00921-production-stability/active/transitive-reduction-validation-plan.md:168`
- `dev/archive/6eb585bc-core-maintenance/6eb585bc-handoff.md:43`
- `dev/archive/842b0ac6-greenfield-cutover/active/breakdown.json:1144`
- `dev/archive/842b0ac6-greenfield-cutover/active/investigation.md:902`
- `dev/archive/842b0ac6-greenfield-cutover/active/sweep-66bd4b5c.md:183`
- `dev/archive/8e071e18-dev-artifact-layout/presentations/vendor/reveal.js/plugin/highlight/highlight.js:1`
- `dev/archive/9d427a6b-production-polish/design/hierarchy-icons-config.md:231`
- `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md:16`
- `dev/archive/9d427a6b-production-polish/design/phase3-advanced-features.md:269`
- `dev/archive/9d427a6b-production-polish/design/search-focus-navigation.md:21`
- `dev/archive/9d427a6b-production-polish/design/subgraph-clustering-layout.md:92`
- `dev/archive/bug-fixes/test-warnings-resolution.md:5`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/investigation.md:308`
- `dev/archive/dbe1e821-work-graph-integrity/features/dbe1e821/dbe1e821-progress.json:176`
- `dev/archive/e204e63d-derived-package-sources/active/7cbefe7c/findings.md:147`
- `dev/archive/e204e63d-derived-package-sources/active/e204e63d-breakdown.json:2986`
- `dev/archive/refactorings/generic-dag-refactoring.md:11`
- `dev/archive/refactorings/state-model-refactoring.md:219`
- `dev/archive/sessions/session-2025-12-03-frontend.md:20`
- `dev/archive/studies/label-hierarchy-implementation-plan.md:283`
- `dev/vision/document-graph-implementation-plan.md:536`
- `dev/vision/knowledge-management-vision.md:177`
- `web/TESTING.md:82`
- `web/src/App.tsx:3`
- `web/src/api/client.ts:2`
- `web/src/components/Graph/GraphView.collapse.test.tsx:10`
- `web/src/components/Graph/GraphView.tsx:17`
- `web/src/components/Graph/__tests__/GraphView.layoutCache.test.tsx:4`
- `web/src/components/Graph/__tests__/GraphView.test.tsx:3`
- `web/src/components/Graph/__tests__/stateColors.test.ts:4`
- `web/src/components/Issue/IssueDetail.tsx:83`
- `web/src/components/Search/useSearch.ts:31`
- `web/src/test/graphNode.ts:1`
- `web/src/types/models.ts:91`
- `web/src/types/subgraphCluster.ts:1`
- `web/src/utils/clientSearch.ts:35`
- `web/src/utils/clusterAwareLayout.test.ts:6`
- `web/src/utils/clusterAwareLayout.ts:3`
- `web/src/utils/clusteredGraphLayout.integration.test.ts:13`
- `web/src/utils/clusteredGraphLayout.test.ts:4`
- `web/src/utils/clusteredGraphLayout.ts:1`
- `web/src/utils/graphFilter.test.ts:12`
- `web/src/utils/graphFilter.ts:1`
- `web/src/utils/subgraphClustering.test.ts:4`
- `web/src/utils/subgraphClustering.ts:1`

### Schema/MCP retention or retirement footprint (separate owner)

Matched files: 518.

- `.agents/skills/jit-execution-lead/evals/evals.json:4`
- `.agents/skills/jit-execution-lead/evals/results.md:11`
- `.agents/skills/jit-manage/SKILL.md:8`
- `.agents/skills/jit-migrate/SKILL.md:6`
- `.agents/skills/jit-parallel/SKILL.md:7`
- `.agents/skills/jit-project-lead/evals/evals.json:4`
- `.agents/skills/jit-project-lead/evals/results.md:29`
- `.dockerignore:50`
- `.github/copilot-instructions.md:117`
- `.github/workflow-contract.yml:69`
- `.github/workflows/ci.yml:369`
- `.github/workflows/release-artifacts.yml:170`
- `.github/workflows/release-publish.yml:51`
- `.github/workflows/security-audit.yml:59`
- `.gitignore:15`
- `.jit/gates.toml:55`
- `.jit/issues/024b98b1-8afe-47f0-b7e3-6cbf904367db.json:9`
- `.jit/issues/03678c53-371e-410c-801f-19b502025a7e.json:16`
- `.jit/issues/03b89b77-fdb9-443c-a491-441ae9447286.json:13`
- `.jit/issues/03c59566-093f-4a7c-a3a6-65098a4b28de.json:16`
- `.jit/issues/06177584-1c3e-4344-8116-b60d44c1e3a4.json:6`
- `.jit/issues/06ee9b67-07c3-48ad-a938-179a83d12564.json:17`
- `.jit/issues/070ca05f-f7a9-4922-b796-3376a0b813b2.json:9`
- `.jit/issues/07657508-b020-4aeb-9f3a-1a836314b229.json:9`
- `.jit/issues/080e54c6-8b70-40c5-9bbe-6a9f720a09ce.json:9`
- `.jit/issues/0b3840f1-fa19-4b9c-8591-3d8c287f9f8f.json:19`
- `.jit/issues/0c3fcf3a-7d9d-4c3e-a40b-4db8d42b6bbe.json:7`
- `.jit/issues/0db719b1-0e95-44b2-a377-0acbcf9f8bde.json:7`
- `.jit/issues/0f2dbe50-7d6b-4fa2-948c-a8c121e2bb5a.json:11`
- `.jit/issues/0f8efc37-8054-420d-9409-dde888ea0040.json:9`
- `.jit/issues/0fe46027-8a33-4dce-bc6f-76162d067fee.json:17`
- `.jit/issues/10751360-4148-410a-bcae-dfe9c1ee1282.json:19`
- `.jit/issues/11682766-316e-4fd7-9bbf-eaddf7c01b04.json:7`
- `.jit/issues/153c9851-dca1-40f3-838b-ee2790fb9318.json:18`
- `.jit/issues/15cc28c5-1066-4db4-b5a6-4796b6144c08.json:23`
- `.jit/issues/165cf162-1cb1-491d-8c92-b2fb571e7f4c.json:7`
- `.jit/issues/182f4c03-2c8b-431c-9c1e-a208c234a525.json:18`
- `.jit/issues/187d7dde-c2de-488a-981b-f85865e4869f.json:10`
- `.jit/issues/1a4f2ebd-6041-4965-b15d-2411a6bdd9ec.json:16`
- `.jit/issues/1a989754-d48c-409a-a796-dee1671f800a.json:9`
- `.jit/issues/1ac651c7-665b-40a5-b214-9abbc14e0534.json:17`
- `.jit/issues/1c0eb82e-b42b-4cfe-9351-78880b415f79.json:19`
- `.jit/issues/20fc3b92-6ce6-48e1-9d51-9bd77956236e.json:18`
- `.jit/issues/233f4a09-7764-470b-867f-c816b4b9589d.json:9`
- `.jit/issues/24268b05-2ab4-4864-87fe-e8ff35945191.json:10`
- `.jit/issues/2492ab29-2047-4bae-9cdc-d17749f1e515.json:9`
- `.jit/issues/24a982b7-d2e0-4bd2-bdd8-2cd4dbeacfd0.json:17`
- `.jit/issues/26307dfd-9eac-4f43-90dd-183ad38d1814.json:7`
- `.jit/issues/26e88813-41f3-4228-8832-14b09bc31931.json:13`
- `.jit/issues/271dc224-a0cf-4f49-bfdb-a8be33f9e7d5.json:10`
- `.jit/issues/27c8256a-be51-4561-a9b8-3fc7648bf801.json:9`
- `.jit/issues/287f7bc9-c185-49a7-bc68-0102aaada58d.json:7`
- `.jit/issues/2c018a6e-9ece-43fe-971b-4d080ea74ab3.json:15`
- `.jit/issues/2d109173-91b2-49b1-9ef6-7d5a0f5a4b22.json:16`
- `.jit/issues/2d225728-bae1-45bc-b7f3-2d81c789c422.json:9`
- `.jit/issues/2dd6b833-8f5d-484a-b412-158b74e3d7f5.json:10`
- `.jit/issues/2e926e39-906e-4952-8ae0-38215a7e5aac.json:18`
- `.jit/issues/304573c5-e65e-47e8-923a-f446da8ff6fb.json:4`
- `.jit/issues/32779829-319f-40b0-bd41-fe213f4885fc.json:14`
- `.jit/issues/340a846d-19c1-4afb-a5bd-7be6d1334e8a.json:9`
- `.jit/issues/348b34f5-a103-4bbf-a0be-3df954f7b971.json:15`
- `.jit/issues/34f45da2-2dbc-4ba4-b9a8-40fd79526931.json:7`
- `.jit/issues/3559ca25-4d31-4e57-9eb1-0853ccd60996.json:16`
- `.jit/issues/36d5451e-05d1-41d7-baf5-208eba97c4a2.json:9`
- `.jit/issues/37ec9723-b94e-432c-a945-22af0216928d.json:9`
- `.jit/issues/3977f5da-0025-4785-b1e5-453a61ea1fcd.json:4`
- `.jit/issues/39c34568-98ee-47d6-ba5f-875b06c59285.json:17`
- `.jit/issues/3a3b5704-b385-4358-98e3-23e095b2ee97.json:16`
- `.jit/issues/3a7caca1-8126-4b02-b94e-435d5b679fbe.json:7`
- `.jit/issues/3b033738-a6ef-40ab-8fc1-d296f3fbb863.json:9`
- `.jit/issues/3c4d6fe8-fafa-461c-ba1b-03d9db29a17e.json:7`
- `.jit/issues/3cf69ecf-5ec1-4cb1-855f-100aa76f7520.json:18`
- `.jit/issues/3d4342f9-e24e-40f2-b83d-52e4de487344.json:9`
- `.jit/issues/3f816619-6163-46d4-8549-f988a3de65db.json:19`
- `.jit/issues/42478e5d-90aa-472e-bca2-f628a1d883c2.json:58`
- `.jit/issues/46fe7593-a860-4767-8a6e-fc631ca1fd81.json:17`
- `.jit/issues/4873b32f-8d16-4120-9fb3-987b818361a2.json:15`
- `.jit/issues/487f45a9-bd42-47f8-8dd1-6487ad287793.json:9`
- `.jit/issues/49adf23b-4fe9-48dd-b9c4-89dfb839c7e9.json:9`
- `.jit/issues/4a00b2b0-a527-4f04-9475-62f1d838f903.json:11`
- `.jit/issues/4cb2682e-79c6-4e9e-9721-8047db261af6.json:11`
- `.jit/issues/4d844e61-f310-4395-b938-81247a964c81.json:2`
- `.jit/issues/5046408a-9706-4ca0-9302-54d20e2088aa.json:9`
- `.jit/issues/51eb1aa6-186d-44c0-a60f-a1e3b0393110.json:24`
- `.jit/issues/52110fb4-8fe5-4090-9f7e-5da7081953ac.json:17`
- `.jit/issues/53f78490-ca3e-4bb4-aa18-503d2e04df97.json:9`
- `.jit/issues/5c581575-bef8-4ee6-be83-7598fd22b557.json:7`
- `.jit/issues/5d0ab945-3c8e-472c-9a54-92f2270c4d12.json:12`
- `.jit/issues/5dd0df5b-bd5d-44d5-9aa8-b94228c7fe55.json:16`
- `.jit/issues/5e538f48-3118-45b8-a7e1-bc6bb6e41dc2.json:9`
- `.jit/issues/6116d362-ccaa-45e1-aed6-40e7130efbbe.json:11`
- `.jit/issues/614a6f97-e1fc-4e03-bd44-05949e33078c.json:10`
- `.jit/issues/623f3163-776e-4e1c-8d98-a8c8f13d2566.json:16`
- `.jit/issues/650e6a56-e3fe-4169-8c2b-58229c9aaeca.json:9`
- `.jit/issues/661d6be2-e28b-463d-be93-58ee3e20e663.json:9`
- `.jit/issues/66790c84-f39f-4004-ba9d-4ceb83a1e8d6.json:17`
- `.jit/issues/66bd4b5c-bfbc-4ec9-aca4-049525a11700.json:24`
- `.jit/issues/68c1f2f3-6620-4770-b6f3-395cc38219b7.json:15`
- `.jit/issues/693a2fdd-18cc-477d-97a6-e4a8f52fcd3f.json:8`
- `.jit/issues/69ffdcf8-eb81-47be-bd94-35ced4963408.json:14`
- `.jit/issues/6a479c58-77ed-4592-8140-9afde0118664.json:15`
- `.jit/issues/6acd7a05-b199-43f8-b619-554b536dc420.json:11`
- `.jit/issues/6c47b99a-e0bc-4705-82b6-5404d285fa58.json:16`
- `.jit/issues/6d496b8c-587f-4246-a7e7-4c7e1d6792cb.json:15`
- `.jit/issues/6df7e456-d845-4a0e-b1f4-38a727285bba.json:17`
- `.jit/issues/6f6b842a-67cd-4032-91ba-253dab6f9754.json:7`
- `.jit/issues/7156f64b-8480-40d1-8af9-c0e48db2ba59.json:15`
- `.jit/issues/719a5501-d611-4a81-8728-1634a8c35b7c.json:9`
- `.jit/issues/71f5296c-7de3-498b-b088-75a2f052014c.json:16`
- `.jit/issues/755cf453-7b7f-47f1-aba6-f9094a26b8bc.json:16`
- `.jit/issues/79252d23-23ab-4c04-9327-8d2986f361d8.json:107`
- `.jit/issues/79dc28d7-b552-4227-bb4c-24894863874c.json:9`
- `.jit/issues/7af6eb3d-3f1d-434b-9db7-d66c706a4b1c.json:23`
- `.jit/issues/7af944ca-eac9-4ba8-883b-3bda654f7a03.json:7`
- `.jit/issues/7ee2efb5-ad81-4cef-8e36-bf2cc9d87e7e.json:10`
- `.jit/issues/7f0ac1b0-b976-44ca-a9e1-9cad27878140.json:9`
- `.jit/issues/81cd978e-f089-445f-a059-ca88e3e1a2de.json:9`
- `.jit/issues/83c61de0-9820-4f48-80f9-6b17ded0c467.json:11`
- `.jit/issues/84c4e956-5ccd-468a-8db8-ecc6958af538.json:9`
- `.jit/issues/8507f8e7-300a-45b6-95a4-44b66c7d9663.json:4`
- `.jit/issues/8847805b-9320-4f1f-b9cf-cb7fe9944f7a.json:18`
- `.jit/issues/896b5b5d-f11f-48de-88b6-88cc1ba47588.json:15`
- `.jit/issues/8b05a612-91e6-43c8-b36e-557e2417c88e.json:15`
- `.jit/issues/8b3cce62-28e5-4871-9303-3d114191af16.json:9`
- `.jit/issues/8bbe6557-2b47-4de5-9800-4061752a8767.json:7`
- `.jit/issues/8c46fc85-284a-4491-aa5c-57a5b22f96db.json:17`
- `.jit/issues/8d7fc762-0a6d-4ef0-b1bd-6b5eb6a77d06.json:7`
- `.jit/issues/8d97547d-f88e-4066-9dde-0afa08b260ab.json:7`
- `.jit/issues/8e91126b-c17b-4274-bccb-13385e40c318.json:16`
- `.jit/issues/8ea5b13d-676b-4727-acaf-f73e2e75a370.json:12`
- `.jit/issues/8f297e60-a74e-4efa-bdcc-54659f9959a4.json:17`
- `.jit/issues/8fa7261b-1be4-48ae-9968-ac0c7448fbc9.json:7`
- `.jit/issues/93d1caf7-1f6e-4d6c-a64d-42546401ebe4.json:7`
- `.jit/issues/950256ae-069c-4537-9379-06b457604fbc.json:7`
- `.jit/issues/9623fe6a-d9f9-4a28-80e2-cc490553b7f9.json:13`
- `.jit/issues/98cc2097-026d-4744-aeaa-82532910328e.json:17`
- `.jit/issues/998eedbc-50e0-4202-ae51-a836b7474f88.json:7`
- `.jit/issues/a122b9b3-0f2a-49bd-bbcf-7dce34703dea.json:7`
- `.jit/issues/a3a788f3-39c2-4218-adaa-1cc5112b30f4.json:9`
- `.jit/issues/a4e8d5ac-c3f7-40bb-b4ea-108283bcf611.json:9`
- `.jit/issues/a5d01ff8-6455-4f85-894b-e28eb4f21fe0.json:9`
- `.jit/issues/a5fbec12-fa6a-4684-994b-5fd0652471e3.json:9`
- `.jit/issues/aeafc378-3073-4b32-be1b-53a05b656168.json:7`
- `.jit/issues/b1586c0d-6f8e-451b-a46b-dedc2f16bb0f.json:33`
- `.jit/issues/b23b1bf3-e875-44aa-b879-4dc96c0b7caf.json:16`
- `.jit/issues/b23faa59-8451-4fc4-97b8-a139d1b210e6.json:16`
- `.jit/issues/b25c2e54-2614-4d87-b967-0f2cb25c37f9.json:17`
- `.jit/issues/b47c8918-8807-4eee-8474-39dab29fa4c2.json:9`
- `.jit/issues/b6aeb05b-dc02-435f-9ae1-ba0a1c6c63f4.json:4`
- `.jit/issues/b7439e3b-8f07-45dc-8558-fda070437ac7.json:9`
- `.jit/issues/b80e3c70-cf7d-4f72-b53d-6e556363be58.json:23`
- `.jit/issues/b8ba1b10-09f3-4807-90cb-95b91258347d.json:10`
- `.jit/issues/b91a1972-5027-41b0-a669-87db9f0f3eac.json:9`
- `.jit/issues/bb03df0a-65c7-496a-bb09-2620eb253fd5.json:31`
- `.jit/issues/bb302dbd-bd03-42ed-ae71-b0f035453f56.json:7`
- `.jit/issues/bc7eb715-9c7f-47ff-bbd6-5d63fee3f123.json:9`
- `.jit/issues/bcff56be-2e8f-44ae-9b3b-5739be414630.json:9`
- `.jit/issues/c06daf50-c151-402a-851a-fa71a9e6d764.json:8`
- `.jit/issues/c291e95c-82f0-437f-8b58-264ac73e27b2.json:7`
- `.jit/issues/c2f41057-2c48-4af5-9729-711d51374da3.json:15`
- `.jit/issues/c505031a-9ba4-4547-aeb7-a83aa8e8ae3f.json:9`
- `.jit/issues/c5c324df-662e-45cb-978e-b90d74280c62.json:7`
- `.jit/issues/c73c7618-0b79-422b-9227-bfcc99e6f345.json:10`
- `.jit/issues/c8355d70-4c21-4c96-b8d9-1f8036910368.json:7`
- `.jit/issues/c9103fcf-aa0c-4642-8531-5360f5fdd58d.json:17`
- `.jit/issues/c998dfd3-07a4-4f7c-ab5c-86c84867fc00.json:9`
- `.jit/issues/cb26de35-de6b-4297-983b-00aa8ef4c64f.json:15`
- `.jit/issues/cc4ea8a9-6f24-4dc2-ad6a-fccbe7d79be4.json:16`
- `.jit/issues/cc75b4e6-dca6-4b12-be45-ee25ca6560bd.json:19`
- `.jit/issues/cd8e04cc-1f52-40a5-935a-b2b77a67240e.json:16`
- `.jit/issues/d0f88ee2-7018-4779-8ccc-969b42eaabb7.json:16`
- `.jit/issues/d198030e-e637-444e-b444-e4e82a3d1ccf.json:15`
- `.jit/issues/d1ae9782-3bc9-42b6-bb1c-315cc41dd1e8.json:11`
- `.jit/issues/d24008f0-ecec-4f08-900b-9d686e9a5051.json:9`
- `.jit/issues/d7930fa7-36c1-4268-8ccc-3fa7e5999ddf.json:12`
- `.jit/issues/d8176fd3-5ed6-4e3f-8fac-6a7fd1438c9e.json:13`
- `.jit/issues/da12ad66-7451-4700-9731-74a36c895bb9.json:11`
- `.jit/issues/da30b92a-f0b8-41f3-bbc5-605ed0f739fb.json:10`
- `.jit/issues/dbe1e821-f4cb-4ef9-9b61-73c9495ee039.json:17`
- `.jit/issues/deb50576-7598-4b01-8cad-338824f22a92.json:11`
- `.jit/issues/df129d9e-c795-4d92-9ee5-ffd776339252.json:13`
- `.jit/issues/df505379-385f-4fe7-b7bd-4af3bb3b88aa.json:9`
- `.jit/issues/e0fed1bb-dd07-49d1-8321-79a2d93805dc.json:9`
- `.jit/issues/e2902949-e14d-4307-b2c3-3fc3c5ad07a5.json:16`
- `.jit/issues/e3259063-2ac4-40a6-86a7-40dbc9716b2f.json:12`
- `.jit/issues/e522a8e1-ac25-49a7-8c16-2c5fe6caaead.json:7`
- `.jit/issues/e6e25f9a-62ea-4686-b44b-21ad51b8afe5.json:26`
- `.jit/issues/e748afbb-63bf-41f3-8823-e0e643bff374.json:3`
- `.jit/issues/e90b9a3b-1a5c-444a-9a2a-2f99345d88d2.json:9`
- `.jit/issues/e93af54b-af11-4512-ba34-8122251de039.json:7`
- `.jit/issues/ea2bb1f6-8c39-402d-89d2-e40b658df16d.json:7`
- `.jit/issues/ea5f4536-6dc7-4c22-8de7-b68b6c30252d.json:9`
- `.jit/issues/eac6ec13-2c8b-4249-9ed4-372416bca255.json:15`
- `.jit/issues/ed5241de-d5b7-47ea-b156-58d978b42eeb.json:4`
- `.jit/issues/f2389b18-5ce3-4e55-ab22-c7807c2ab62c.json:15`
- `.jit/issues/f320923c-702d-4910-b046-0817375de510.json:11`
- `.jit/issues/f368ce3d-2049-4171-b0ec-927bbfaf5321.json:9`
- `.jit/issues/f7f80d53-508e-4b03-b045-5209061c8d0a.json:19`
- `.jit/issues/f87f9b00-c255-45b3-b66c-655b394c13bc.json:19`
- `.jit/issues/f89ea266-bf12-4fe6-a685-55cfffa73165.json:16`
- `.jit/issues/f930acf2-0f03-4ca0-8ad8-97ddcc85f368.json:16`
- `.jit/issues/f980db67-ed74-4e89-89ef-fb72261a0a86.json:19`
- `.jit/issues/f9af9788-9d54-4200-8da4-98818cc67fa9.json:13`
- `.jit/issues/fb38bf1f-a4b0-4acf-9b5b-bda244d0ee09.json:24`
- `.jit/issues/fccedb73-b4b4-4990-a26d-b99a2cda7739.json:11`
- `.jit/issues/fdae1023-6a10-45da-a56e-54916409a331.json:14`
- `.jit/issues/ff1bbada-b22a-432a-a75a-8506743e90f9.json:19`
- `AGENTS.md:16`
- `CHANGELOG.md:460`
- `INSTALL.md:6`
- `README.md:48`
- `crates/jit/examples/package-drift.rs:94`
- `crates/jit/src/schema.rs:43`
- `crates/jit/tests/cli_item_validate/validate_cli_rule_tests.rs:3`
- `crates/jit/tests/cli_repo_workflow/integration_schema.rs:249`
- `crates/jit/tests/cli_repo_workflow/serve_cli_tests.rs:4`
- `crates/jit/tests/scratch_build/repository_state_cutover_tests.rs:1111`
- `dev/active/9db27a3a-v1-0/9db27a3a-progress.json:269`
- `dev/active/b697a3f7-one-mutation-path/breakdown.json:315`
- `dev/active/b697a3f7-one-mutation-path/investigation.md:26`
- `dev/active/b80e3c70-v1-release/handoff-4.md:24`
- `dev/active/b80e3c70-v1-release/handoff-5.md:36`
- `dev/active/b80e3c70-v1-release/handoff-6.md:24`
- `dev/active/b80e3c70-v1-release/handoff.md:18`
- `dev/active/b80e3c70-v1-release/progress.json:11`
- `dev/active/bb302dbd/production-audit-evidence.md:1`
- `dev/active/df129d9e-rust-code-quality/breakdown.json:189`
- `dev/active/df129d9e-rust-code-quality/handoff-11.md:14`
- `dev/active/df129d9e-rust-code-quality/handoff-12.md:134`
- `dev/active/df129d9e-rust-code-quality/handoff-2.md:10`
- `dev/active/df129d9e-rust-code-quality/handoff-3.md:31`
- `dev/active/df129d9e-rust-code-quality/handoff-4.md:17`
- `dev/active/df129d9e-rust-code-quality/handoff-5.md:10`
- `dev/active/df129d9e-rust-code-quality/handoff-6.md:9`
- `dev/active/df129d9e-rust-code-quality/handoff.md:17`
- `dev/active/df129d9e-rust-code-quality/investigation.md:323`
- `dev/active/df129d9e-rust-code-quality/plan.md:16`
- `dev/active/df129d9e-rust-code-quality/progress.json:42`
- `dev/active/f4d9502a-git-cli-module/investigation.md:85`
- `dev/architecture/cli-and-mcp-strategy.md:1`
- `dev/archive/004d10b7-documentation-contract-followups/sessions/004d10b7-completion-report.md:11`
- `dev/archive/004d10b7-documentation-contract-followups/sessions/004d10b7-progress.json:15`
- `dev/archive/14303b30-phase5-2/active/gate-modification-flags-plan.md:275`
- `dev/archive/14303b30-phase5-2/active/quiet-mode-plan.md:404`
- `dev/archive/1cc809de-repository-state-quality/1cc809de-breakdown-keymap.json:15`
- `dev/archive/1cc809de-repository-state-quality/1cc809de-breakdown.json:103`
- `dev/archive/1cc809de-repository-state-quality/1cc809de-completion-report.md:44`
- `dev/archive/1cc809de-repository-state-quality/1cc809de-handoff-4.md:21`
- `dev/archive/1cc809de-repository-state-quality/1cc809de-plan.md:183`
- `dev/archive/1cc809de-repository-state-quality/1cc809de-progress.json:236`
- `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/plugin/highlight/highlight.js:1`
- `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md:125`
- `dev/archive/2195c98f-semantic-types/active/investigation.md:124`
- `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:94`
- `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md:139`
- `dev/archive/287c4051-docs-accuracy/287c4051-completion-report.md:25`
- `dev/archive/287c4051-docs-accuracy/287c4051-progress.json:8`
- `dev/archive/287c4051-docs-accuracy/bug-fixes/287c4051-docs-audit-findings.md:79`
- `dev/archive/2d109173-docs-exhaustive-audit/2d109173-handoff-2.md:22`
- `dev/archive/2d109173-docs-exhaustive-audit/2d109173-handoff-3.md:19`
- `dev/archive/2d109173-docs-exhaustive-audit/2d109173-investigation.md:17`
- `dev/archive/2d109173-docs-exhaustive-audit/2d109173-plan.md:26`
- `dev/archive/2d109173-docs-exhaustive-audit/2d109173-progress.json:40`
- `dev/archive/2d109173-docs-exhaustive-audit/active/2b9a80fb-audit-notes.md:51`
- `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md:3`
- `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-4.md:16`
- `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-5.md:13`
- `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-6.md:15`
- `dev/archive/2d109173-docs-exhaustive-audit/active/736a069e-audit-notes.md:88`
- `dev/archive/2d109173-docs-exhaustive-audit/active/99f4a2b4-req02-evidence.md:74`
- `dev/archive/2d109173-docs-exhaustive-audit/active/adc4c6ef-relocation-notes.md:97`
- `dev/archive/2d109173-docs-exhaustive-audit/bug-fixes/2d109173-planning-brief.md:21`
- `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md:16`
- `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-completion-report.md:63`
- `dev/archive/2e926e39-agent-seamlessness/presentations/2e926e39/talk.html:166`
- `dev/archive/3d9a0ce3-lead-friction/active/breakdown.json:248`
- `dev/archive/3d9a0ce3-lead-friction/active/completion-report.md:31`
- `dev/archive/3d9a0ce3-lead-friction/active/handoff-2.md:28`
- `dev/archive/3d9a0ce3-lead-friction/active/investigation-children.md:7`
- `dev/archive/3d9a0ce3-lead-friction/active/investigation-profile.md:241`
- `dev/archive/3d9a0ce3-lead-friction/active/plan.md:135`
- `dev/archive/3d9a0ce3-lead-friction/active/progress.json:521`
- `dev/archive/4a00b2b0-agent-validation/active/agent-validation-design.md:12`
- `dev/archive/4a559332-gf2-adopter-reliability/breakdown.json:335`
- `dev/archive/4a559332-gf2-adopter-reliability/completion-report.md:55`
- `dev/archive/4a559332-gf2-adopter-reliability/investigation.md:396`
- `dev/archive/4b7c06d0-test-suite-performance/active/investigation.md:46`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/post-change-test-inventory.json:4202`
- `dev/archive/4b7c06d0-test-suite-performance/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:4174`
- `dev/archive/5fe00921-production-stability/active/production-stability-design.md:112`
- `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-cli-integration.md:289`
- `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-operations-progress.md:238`
- `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-phase5-docs.md:1`
- `dev/archive/6eb585bc-core-maintenance/6eb585bc-completion-report.md:40`
- `dev/archive/6eb585bc-core-maintenance/6eb585bc-handoff-3.md:20`
- `dev/archive/6eb585bc-core-maintenance/6eb585bc-handoff-4.md:37`
- `dev/archive/6eb585bc-core-maintenance/6eb585bc-handoff.md:13`
- `dev/archive/6eb585bc-core-maintenance/6eb585bc-progress.json:7`
- `dev/archive/6eb585bc-core-maintenance/active/949cd9d0-gate-verb-semantics.md:146`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/post-change-test-inventory.json:4202`
- `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:4174`
- `dev/archive/71373e37-docs-lifecycle/sessions/session-2025-12-22-doc-consolidation.md:40`
- `dev/archive/71373e37-docs-lifecycle/studies/documentation-lifecycle-strategy.md:111`
- `dev/archive/7d3a3a47/7d3a3a47-completion-report.md:31`
- `dev/archive/7d3a3a47/active/7d3a3a47-plan.md:477`
- `dev/archive/842b0ac6-greenfield-cutover/active/breakdown.json:582`
- `dev/archive/842b0ac6-greenfield-cutover/active/investigation.md:31`
- `dev/archive/842b0ac6-greenfield-cutover/active/progress.json:441`
- `dev/archive/842b0ac6-greenfield-cutover/active/sweep-66bd4b5c.md:3`
- `dev/archive/8b05a612-production-readiness/active/v1-production-readiness-scope-brief.md:7`
- `dev/archive/8e071e18-dev-artifact-layout/active/8e071e18-breakdown.json:376`
- `dev/archive/8e071e18-dev-artifact-layout/active/8e071e18-handoff-2.md:34`
- `dev/archive/8e071e18-dev-artifact-layout/active/8e071e18-investigation.md:820`
- `dev/archive/8e071e18-dev-artifact-layout/active/8e071e18-progress.json:194`
- `dev/archive/8e071e18-dev-artifact-layout/active/archive-run-evidence.md:501`
- `dev/archive/8e071e18-dev-artifact-layout/active/handoff-3.md:73`
- `dev/archive/8e071e18-dev-artifact-layout/presentations/vendor/reveal.js/plugin/highlight/highlight.js:1`
- `dev/archive/9876a23a-one-validation-report/active/breakdown.json:1435`
- `dev/archive/9876a23a-one-validation-report/active/handoff.md:12`
- `dev/archive/9876a23a-one-validation-report/active/investigation.md:250`
- `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md:24`
- `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-completion-report.md:47`
- `dev/archive/9b7b5f9c-jit-profiles/9b7b5f9c-completion-report.md:13`
- `dev/archive/9b7b5f9c-jit-profiles/9b7b5f9c-progress.json:105`
- `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md:11`
- `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-plan.md:9`
- `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-research.md:10`
- `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-investigation.md:391`
- `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md:15`
- `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/talk.html:235`
- `dev/archive/9d427a6b-production-polish/active/dependency-display-improvements-plan.md:177`
- `dev/archive/9d427a6b-production-polish/sessions/session-20260109-query-consolidation.md:162`
- `dev/archive/9d427a6b-production-polish/studies/agent-ux-observations.md:4`
- `dev/archive/a2546471-json-error-contract/active/breakdown.json:365`
- `dev/archive/a2546471-json-error-contract/active/investigation-guard-docs.md:194`
- `dev/archive/a2546471-json-error-contract/active/investigation.md:19`
- `dev/archive/a2546471-json-error-contract/active/plan.md:158`
- `dev/archive/c639cfb5-jit-profiles-complete/active/breakdown.json:199`
- `dev/archive/c639cfb5-jit-profiles-complete/active/c639cfb5-research.md:444`
- `dev/archive/c639cfb5-jit-profiles-complete/active/completion-report.md:9`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-11.md:37`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-14.md:19`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-2.md:24`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-3.md:22`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-4.md:18`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-5.md:26`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-6.md:27`
- `dev/archive/c639cfb5-jit-profiles-complete/active/handoff-8.md:33`
- `dev/archive/c639cfb5-jit-profiles-complete/active/investigation.md:211`
- `dev/archive/c639cfb5-jit-profiles-complete/active/progress.json:220`
- `dev/archive/c639cfb5-jit-profiles-complete/presentations/c639cfb5/talk.html:464`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/breakdown.json:1125`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/handoff-2.md:52`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/handoff.md:37`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/investigation.md:224`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/plan.md:71`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/progress.json:1011`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-completion-report.md:178`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-10.md:47`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-11.md:119`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-14.md:24`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-15.md:33`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-16.md:24`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-17.md:45`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-8.md:32`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-9.md:36`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-investigation.md:120`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md:63`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-progress.json:69`
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-research.md:619`
- `dev/archive/cdc840ad-repository-materialization/features/cdc840ad/showcase/talk.html:353`
- `dev/archive/dbe1e821-work-graph-integrity/features/dbe1e821/dbe1e821-completion-report.md:31`
- `dev/archive/dbe1e821-work-graph-integrity/features/dbe1e821/dbe1e821-progress.json:8`
- `dev/archive/e204e63d-derived-package-sources/active/7cbefe7c/findings.md:57`
- `dev/archive/e204e63d-derived-package-sources/active/a62d444d/findings.md:253`
- `dev/archive/e204e63d-derived-package-sources/active/e204e63d-breakdown.json:975`
- `dev/archive/e204e63d-derived-package-sources/active/e204e63d-completion-report.md:134`
- `dev/archive/e204e63d-derived-package-sources/active/e204e63d-investigation.md:394`
- `dev/archive/e204e63d-derived-package-sources/active/handoff-3.md:45`
- `dev/archive/e204e63d-derived-package-sources/active/handoff-4.md:114`
- `dev/archive/e204e63d-derived-package-sources/active/handoff-8.md:81`
- `dev/archive/e204e63d-derived-package-sources/active/handoff-9.md:35`
- `dev/archive/e204e63d-derived-package-sources/active/progress.json:43`
- `dev/archive/f1dc843a-redo-mutations/active/breakdown.json:986`
- `dev/archive/f1dc843a-redo-mutations/active/e346d387/sweep-evidence.md:4`
- `dev/archive/f1dc843a-redo-mutations/active/e590fa91/checkpoint-evidence.md:17`
- `dev/archive/f1dc843a-redo-mutations/active/investigation.md:399`
- `dev/archive/f1dc843a-redo-mutations/active/progress.json:592`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/post-change-test-inventory.json:4202`
- `dev/archive/f1dc843a-redo-mutations/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:4174`
- `dev/archive/f3f7de97-guards-and-ergonomics/active/4c700c80/4c700c80-host-load-survey.md:158`
- `dev/archive/f3f7de97-guards-and-ergonomics/active/hosted-ci-evidence.md:54`
- `dev/archive/f6a704d0-validation-engine/features/f6a704d0-0abaddc0-fullmigration-plan.md:78`
- `dev/archive/f6a704d0-validation-engine/features/f6a704d0-expansion2-plan.md:53`
- `dev/archive/f6a704d0-validation-engine/features/f6a704d0-handoff-2.md:91`
- `dev/archive/f6a704d0-validation-engine/features/f6a704d0-validation-engine-plan.md:33`
- `dev/archive/f6a704d0-validation-engine/features/f6a704d0-validation-engine.md:22`
- `dev/archive/f8a6b298-history-independent-reads/active/f8a6b298-history-independent-reads/breakdown.json:856`
- `dev/archive/f8a6b298-history-independent-reads/active/f8a6b298-history-independent-reads/drift-audit.md:83`
- `dev/archive/f8a6b298-history-independent-reads/active/f8a6b298-history-independent-reads/investigation.md:49`
- `dev/archive/features/cdc840ad/showcase/talk.html:353`
- `dev/archive/features/file-locking-design.md:10`
- `dev/archive/features/file-locking-usage.md:13`
- `dev/archive/features/json-schema-api.md:222`
- `dev/archive/features/search-implementation.md:176`
- `dev/archive/refactorings/refactoring-plan-binary-to-library.md:315`
- `dev/archive/refactorings/state-model-refactoring.md:214`
- `dev/archive/sessions/session-2025-12-02-complete.md:288`
- `dev/archive/sessions/session-2025-12-02-mcp-server.md:1`
- `dev/archive/studies/ROADMAP.md:40`
- `dev/archive/studies/agent-context-mcp.md:1`
- `dev/archive/studies/file-classification-analysis.md:54`
- `dev/archive/studies/getting-started-complete.md:17`
- `dev/archive/studies/label-enforcement-proposal.md:314`
- `dev/archive/studies/label-hierarchy-audit-plan.md:15`
- `dev/archive/studies/label-hierarchy-implementation-plan.md:238`
- `dev/archive/studies/type-hierarchy-enforcement-proposal.md:1631`
- `dev/archive/studies/week1-completion-report.md:50`
- `dev/benchmarks/applied-profile-baselines-ae17798d/raw/inventory.stdout:1`
- `dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/candidate-build/pre-change-test-inventory.json:12664`
- `dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/opt0-build/pre-change-test-inventory.json:9432`
- `dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/candidate-build/pre-change-test-inventory.json:9736`
- `dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/opt0-build/pre-change-test-inventory.json:13033`
- `dev/benchmarks/nextest-concurrency-934985ca/diagnostics/warmup-interrupted-client-timeout/stderr.log:1954`
- `dev/benchmarks/nextest-concurrency-934985ca/diagnostics/warmup-interrupted-client-timeout/stdout.log:3871`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/inventory/stdout.log:1`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w24-1/stderr.log:1949`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w24-1/stdout.log:3871`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stderr.log:1945`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w36-1/stdout.log:3856`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stderr.log:1934`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/w48-1/stdout.log:3847`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stderr.log:1945`
- `dev/benchmarks/nextest-concurrency-934985ca/raw/warmup-24/stdout.log:3871`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-1.xml:4226`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-2.xml:4228`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/junit-3.xml:4228`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-1.out:2118`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-2.out:2119`
- `dev/benchmarks/per-test-durations-1eb949a5/raw/run-3.out:2122`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-1.xml:4422`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-2.xml:4422`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/junit-3.xml:4422`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-1.xml:4422`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-2.xml:4422`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/junit-3.xml:4422`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-1.out:2216`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-2.out:2216`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/quiet/run-3.out:2216`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-1.out:2217`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-2.out:2216`
- `dev/benchmarks/per-test-durations-1eb949a5/remeasure-4a01ccba/run-3.out:2216`
- `dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json:4202`
- `dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json:4174`
- `dev/benchmarks/suite-profile.json:34373`
- `dev/benchmarks/test-profile-123efe74/opt0-build/pre-change-test-inventory.json:9258`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.jsonl:3871`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-1.stderr:1943`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.jsonl:3871`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-2.stderr:1943`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.jsonl:3871`
- `dev/benchmarks/test-profile-123efe74/opt0-runtime/raw/nextest-3.stderr:1942`
- `dev/benchmarks/test-profile-123efe74/opt1-build/pre-change-test-inventory.json:12107`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.jsonl:3871`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-1.stderr:1949`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.jsonl:3871`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-2.stderr:1950`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.jsonl:3871`
- `dev/benchmarks/test-profile-123efe74/opt1-runtime/raw/nextest-3.stderr:1950`
- `dev/benchmarks/test-profile-efaec52f/opt0-build-screen/pre-change-test-inventory.json:13663`
- `dev/benchmarks/test-profile-efaec52f/scoped-build-screen/pre-change-test-inventory.json:11181`
- `dev/eval/skill-eval-adjudication.md:71`
- `dev/index.md:146`
- `dev/studies/jit-vs-agentic-trends-2026.md:17`
- `dev/studies/multi-agent-parallelism-analysis.md:58`
- `dev/vision/9db27a3a-charter.md:45`
- `dev/workflow-contract.md:75`
- `docs/concepts/design-philosophy.md:61`
- `docs/concepts/overview.md:23`
- `docs/how-to/deployment.md:266`
- `docs/how-to/mcp-integration.md:1`
- `docs/how-to/multi-agent-coordination.md:504`
- `docs/how-to/troubleshooting.md:88`
- `docs/index.md:41`
- `docs/reference/cli-commands.md:431`
- `docs/reference/compatibility.md:20`
- `docs/reference/labels.md:390`
- `docs/reference/profiles.md:362`
- `docs/reference/release-policy.md:13`
- `docs/reference/rules-and-gates.md:54`
- `docs/release-notes/v1.0.0.md:19`
- `docs/tutorials/quickstart.md:39`
- `mcp-server/README.md:1`
- `mcp-server/curated-tools.json:4`
- `mcp-server/index.js:4`
- `mcp-server/lib/cli-executor.js:156`
- `mcp-server/lib/concurrency.js:11`
- `mcp-server/lib/tool-generator.js:4`
- `mcp-server/package-lock.json:2`
- `mcp-server/package.json:2`
- `mcp-server/test-integration.js:3`
- `mcp-server/test-unit.js:3`
- `mcp-server/test-with-env.sh:2`
- `scripts/doc-review-prompt.md:47`
- `scripts/docs-canonical-homes.toml:29`
- `scripts/release-version-contract.py:4`
- `scripts/release_version_contract_test.py:20`
- `scripts/test-ci-local.sh:44`
- `scripts/test-ci-manual.sh:40`
- `scripts/validate-setup.sh:44`
- `scripts/workflow-contract.py:429`
- `test-vectors/container-image/test_image_contract.py:111`
- `test-vectors/workflow-contract/caller-suite-audit-boundaries/.github/workflow-contract.yml:28`
- `test-vectors/workflow-contract/caller-suite-audit-boundaries/.github/workflows/release.yml:15`
- `test-vectors/workflow-contract/caller-suite-audit-boundaries/.github/workflows/security-audit.yml:12`
- `test-vectors/workflow-contract/caller-suite-audit-boundary-bypassed/.github/workflow-contract.yml:27`
- `test-vectors/workflow-contract/caller-suite-audit-boundary-bypassed/.github/workflows/release.yml:14`
- `test-vectors/workflow-contract/caller-suite-audit-boundary-bypassed/.github/workflows/security-audit.yml:12`

### Executor with_layout builder callers (fallible-binding cutover)

Matched files: 31. Every caller must propagate or explicitly assert the construction result when this builder becomes fallible.

- `.jit/issues/a807e699-a7ed-474e-b6af-2ed4086a9dad.json:9`
- `crates/jit/src/commands/bootstrap.rs:122`
- `crates/jit/src/commands/claim.rs:730`
- `crates/jit/src/commands/config.rs:530`
- `crates/jit/src/commands/dependency.rs:756`
- `crates/jit/src/commands/document.rs:2039`
- `crates/jit/src/commands/gate.rs:1223`
- `crates/jit/src/commands/gate_check.rs:2613`
- `crates/jit/src/commands/graph.rs:489`
- `crates/jit/src/commands/init.rs:627`
- `crates/jit/src/commands/issue.rs:355`
- `crates/jit/src/commands/mod.rs:2531`
- `crates/jit/src/commands/profile.rs:2080`
- `crates/jit/src/commands/query.rs:153`
- `crates/jit/src/commands/test_helpers.rs:406`
- `crates/jit/src/commands/validate.rs:1535`
- `crates/jit/src/profile/repository_package.rs:843`
- `crates/jit/src/test_utils.rs:393`
- `crates/jit/tests/cli_gate/gate_evaluation_durability_test.rs:34`
- `crates/jit/tests/cli_repo_workflow/profile_publication_tests.rs:105`
- `crates/jit/tests/fast_docs_templates/document_event_log_tests.rs:300`
- `crates/jit/tests/fast_docs_templates/template_apply_atomicity_tests.rs:556`
- `crates/jit/tests/fast_docs_templates/template_apply_tests.rs:1009`
- `crates/jit/tests/fast_rules/config_loading_tests.rs:99`
- `crates/server/src/main.rs:76`
- `dev/active/df129d9e-rust-code-quality/breakdown.json:3596`
- `dev/active/df129d9e-rust-code-quality/investigation.md:675`
- `dev/archive/ad601a15-parallel-work/design/cli-quality-improvements.md:291`
- `dev/archive/ad601a15-parallel-work/sessions/session-20260201-refactor-items-5-7.md:20`
- `dev/archive/c780514e-test-topology/active/c780514e-test-topology/investigation.md:346`
- `dev/studies/architecture-pitfalls.md:182`
