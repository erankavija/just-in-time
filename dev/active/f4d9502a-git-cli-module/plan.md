# Plan: Reach git through one storage-owned CLI module instead of libgit2 (f4d9502a)

> Planning node: 28ccf9a9. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

One file, `crates/jit/src/storage/git.rs`, starts every git process through one executable value, always with `-C` the selected worktree (`@/charter/D-21`). Each git fact a consumer needs gets one module operation, introduced with or just before its first production caller. Layout detection is one probe per invocation. Issue reads lose the `HEAD` tier, so they need no git at all. Once the last libgit2 reader moves, the binding goes. Core commands keep working without git, and claims fail with the typed error (`@/charter/D-4`). Each issue body is written against the code as it stands after the upstream tasks the entry follows, and cites current lines.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | Every libgit2 reader moves onto the module, then `drop-libgit2` removes both manifest lines and the lock entries with a dependency-policy test, and `binary-linkage-budget-check` rejects libgit2 and OpenSSL linkage in the build-policy script (DEC-15). | Investigation, libgit2 §1 C1, C3; §5d |
| REQ-02 | `git-module` is the one spawning file. Each spawn site moves in its own consumer entry, and `git-surface-trim` checks the end state. | CLI surface §3.1 |
| REQ-03 | The module's helper always passes `-C`, and the executable is one `Git` value (DEC-07). The process-cwd sites (drift, guard, gate evidence, hooks, server) move to the selected worktree. | CLI surface C5, C7, C9, REQ-03 row |
| REQ-04 | `layout-once`: `layout-probe-adoption` makes detection one probe, the server detects once from its data root, and the lazy fallbacks are deleted. Init and hooks read the layout. Validation's re-detection goes with upstream 98184c64, and `git-surface-trim` deletes `detect` and `detect_from`. | CLI surface C1–C5, §5.1 A |
| REQ-05 | `git-head` with one rendering of the branch label; claims, coordinator, worktree info, init identity, gate evidence and snapshot use it (DEC-10). | CLI surface §4.3 |
| REQ-06 | `head-tier-removal` (DEC-05) and the working-tree `doc show` (DEC-06); `git-process-budget-test` also bounds the linked checkout. | libgit2 §1 finding, §5a |
| REQ-07 | A PATH-prepended counting stand-in in one `cli_repo_workflow` module (DEC-08). | libgit2 §5b |
| REQ-08 | `git-worktree-listing`, then `lease-validation-lists-once` with a counting test. | CLI surface C8, §4.5 |
| REQ-09 | `gate-evidence-through-module`; in-memory runs record none (DEC-11). | CLI surface C7 |
| REQ-10 | `git-object-reads` (one reader per command), then path reads at a commit, document show, diff, history, asset checks, snapshot and archive planning each move in their own entry. The server image gains the git executable first. | libgit2 §3 replacement table |
| REQ-11 | Operations land only with production callers; `git-surface-trim` restricts test constructors to `test-support` and deletes test-only layout entry points. | CLI surface REQ-11 row, §5.4 |
| REQ-12 | A typed mapping in `claims-head-lookup`; `no-git-command-tests` covers the no-executable condition; the outside-repository tests already exist (DEC-17). | CLI surface REQ-12 row, §4.6 |
| REQ-13 | The `ureq` comment and Dockerfile builder step change with the removal; `libgit2-prose` corrects `dev/vision`. | libgit2 §3 REQ-13 sites |

## Shared architectural contracts

### `git-module` [implementation-produced] — Module foundation

`crates/jit/src/storage/git.rs` (`storage::git`, public) is the only production file that starts git or names its executable. `Git` names the executable. `Git::default()` is the bare name `git`, looked up on the invocation's `PATH` when a process starts; a constructor taking another program compiles only under `test-support`. One private helper runs `<program> -C <dir> <args…>`. A failure to start it is the module error for an unavailable executable, naming the program. `GitCheckout` pairs a `Git` with a worktree root and carries every operation. `WorktreePaths::git()` returns one built with the default executable for a git checkout and `None` otherwise. `JsonFileStorage` builds its checkouts from a `Git` it holds, so storage tests can inject one. `Git::probe(dir)` classifies a directory with one `rev-parse --git-common-dir --show-toplevel` process. The behaviour suite is `crates/jit/tests/fast_docs_templates/git_module_tests.rs`. Producer: `git-module-foundation`. 5f82e09a's dependency is re-homed onto this producer through `git-dirty-paths` (DEC-04).

### `layout-once` [plan-fixed] — Layout detected at the invocation boundary

The CLI bootstrap (`crates/jit/src/main.rs`, the `WorktreePaths::detect_for_data_root` call before storage construction) and server startup each detect `WorktreePaths` once from the selected data root. Storage receives the value through `configure_worktree_paths`, the executor through `with_worktree_paths`, and claim and worktree commands as a parameter. A handle given none is non-git and never probes. `WorktreePaths` keeps its fields, so no struct literal changes. Data-root discovery's `.git` existence check (`crates/jit/src/storage/discovery.rs`) is a filesystem test that must work without git; it is not layout detection and stays (investigation, CLI surface §3.5).

### `git-test-fixtures` [implementation-produced] — Shared git fixtures

`jit::test_harness` offers a git repository fixture (init through the CLI, repository-local identity, write, commit returning the id) and, on Unix, a counting stand-in: a `git` file in its own directory that records each invocation's arguments and runs the real git by an absolute path resolved at creation. A test passes the stand-in as an explicit program or prepends its directory to a child's `PATH`. Producer: `git-cli-test-fixture`.

### `git-head` [implementation-produced] — One head lookup

`GitCheckout::head()` runs `rev-parse HEAD --abbrev-ref HEAD` once. It returns `Ok(None)` on an unborn branch, or `Head { commit: CanonicalCommitOid, branch: Option<String> }`, with `branch` absent on a detached HEAD. `Head::branch_label()` is the branch name, or `HEAD` when detached. Consumers that record a branch record the label, so their values are unchanged; only init's identity changes (DEC-10). Producer: `git-head-lookup`.

### `git-worktree-listing` [implementation-produced] — One worktree listing

`GitCheckout::list_worktrees()` runs one `worktree list --porcelain` and returns each worktree's path and branch name, absent when detached or bare. Consumers apply their own filters. Producer: `git-worktree-listing`.

### `git-base-comparison` [implementation-produced] — One base comparison

`GitCheckout::compare_with_base(base)` returns the merge base of `HEAD` and `base` with `base`'s commit, or `None` when either does not resolve. The caller names the base (`origin/main` today); whether the drift check and the guard run, and against which base, is f368ce3d's decision. Producer: `git-base-comparison`.

### `git-pinned-reads` [implementation-produced] — Revision reads in the module

`GitCheckout::{resolve_commit, inspect_pinned_targets}` keep the semantics of `storage/git_revision.rs`, which is deleted, and the module error carries its variants. `JsonFileStorage` holds a `Git` in place of `git_program`. Producer: `pinned-reads-into-module`.

### `git-revision-resolver-bridge` [implementation-produced] — Interim archive resolver

`storage::GitRevisionResolver` keeps only `new(repo_root)` and `read_pinned_path`, delegating to a default-executable `GitCheckout`, so the archive planner compiles untouched while the resolver code moves. It is a named temporary bridge with one producer, `pinned-reads-into-module`, and one user, the archive planner. Its one remover is `archive-pinned-reads-batched`, which depends on the producer. Removal criterion: neither `GitRevisionResolver` nor `GitCheckout::read_pinned_path` exists (`@/inv/canonical-cutover`).

### `git-object-reads` [implementation-produced] — One object reader per command

`GitCheckout::object_reader()` holds one `cat-file --batch` process that answers reads one at a time until dropped. A read returns the object's id, type and bytes, or absent for a missing name. `GitCheckout::object_types(names)` answers presence in one `--batch-check` process. A name containing a newline is rejected before git sees it. Pinned inspection uses the same parser. Every command that reads git objects holds at most one reader. Producer: `git-object-reads`.

### `git-dirty-paths` [implementation-produced] — One dirty-path query

`GitCheckout::dirty_paths()` runs one `status --porcelain -z --untracked-files=all` and returns the repository-relative paths that are modified, staged, deleted, renamed (both names) or untracked, excluding ignored files. Producer: `git-dirty-paths`. Consumers: snapshot export here, and 5f82e09a's apply refusal (DEC-04, DEC-12).

### `git-file-history` [implementation-produced] — One path history query

`GitCheckout::file_history(path)` runs one `git log` over the path and returns commits newest first with full id, author, commit time and message, under git's default history simplification. Producer: `document-history-through-module`. 0f8efc37's document convergence consumes it (DEC-04).

### `issue-read-sources` [plan-fixed] — Two issue read sources

Issue and index reads consult the checkout's own data root and, in a linked checkout, the primary checkout's data root (`WorktreePaths::primary_data_root`). The first source naming an id wins, and a local tombstone hides a primary-checkout record. No issue read starts git or loads a git object.

### `document-read-versions` [plan-fixed] — Document version selection

A document read uses the explicit `--at` revision, else the reference's pinned commit, else the working tree, in the CLI and the server alike. Reads at a commit go through the module's object reader, one per command. History and diff read git objects by definition.

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| git-cli-test-fixture | Shared git repository and counting stand-in test fixtures | task | Tests build git repositories and count git processes through one shared fixture pair. | — | REQ-01, REQ-07, DEC-08, DEC-16 | touches 2 | — | — |
| git-module-foundation | Storage-owned git module with a one-process probe | task | One public storage module owns the git executable and a one-process directory probe. | git-test-fixtures | REQ-02, REQ-03, REQ-12, DEC-07, DEC-09, DEC-17 | creates 2, touches 3 | — | git-cli-test-fixture |
| layout-probe-adoption | Layout detection uses the module's one-process probe | task | WorktreePaths detection runs one probe process instead of three. | git-module, layout-once | REQ-02, REQ-04, DEC-09 | touches 1 | — | git-module-foundation |
| server-layout-from-data-root | Server detects its layout once from the selected data root | task | jit-server detects its layout from --data-dir once and configures its handles with it. | layout-once, git-module | REQ-03, REQ-04, DEC-09 | touches 2 | — | layout-probe-adoption |
| layout-fallback-removal | Storage and executor stop re-detecting the layout | task | Storage and the executor use supplied layouts only and never detect lazily. | layout-once | REQ-04, DEC-09 | touches 3, uncertain | — | server-layout-from-data-root, head-tier-removal |
| init-git-classification-from-layout | Init reads git classification from the invocation's layout | task | jit init classifies git from the invocation's layout and starts no git process. | layout-once | REQ-02, REQ-04, DEC-09 | touches 1 | — | layout-fallback-removal |
| hooks-install-from-layout | Hook installation targets the selected repository's common directory | task | jit hooks install writes into the selected repository's common git directory. | layout-once | REQ-03, REQ-04, DEC-09 | touches 3 | — | — |
| git-process-budget-test | CLI test bounds the git processes of issue reads | task | A CLI test shows issue reads start at most one git process in either checkout kind. | git-test-fixtures, git-module, issue-read-sources | REQ-06, REQ-07, DEC-05, DEC-08 | creates 1, touches 1 | — | layout-probe-adoption, head-tier-removal |
| git-head-lookup | One head lookup in the git module | task | The git module answers commit and branch with one head lookup process. | git-module | REQ-05, DEC-10 | touches 2 | — | git-module-foundation |
| gate-evidence-through-module | Gate runs take commit and branch evidence from the module | task | Exec gate runs record the selected worktree's head passed in from the git module. | git-head, layout-once | REQ-02, REQ-03, REQ-05, REQ-09, DEC-10, DEC-11 | touches 2 | — | git-head-lookup, layout-fallback-removal |
| worktree-info-head-lookup | Worktree info reads its branch through the module | task | jit worktree info reads its branch from the module head lookup. | git-head | REQ-02, REQ-05, DEC-10 | touches 1 | — | git-head-lookup |
| init-identity-head-lookup | Init records the worktree branch through the module | task | jit init records the worktree branch from the module head lookup. | git-head | REQ-02, REQ-05, DEC-10 | touches 1 | — | git-head-lookup |
| claims-head-lookup | Claims take their branch and git requirement from the module | task | Claims read the branch through the module and report a missing git as the typed error. | git-head | REQ-02, REQ-05, REQ-12, DEC-03, DEC-10, DEC-17 | touches 2 | — | git-head-lookup |
| release-actor-through-module | Release actor reads git user.name through the module | task | The claim-release actor fallback reads user.name through the module. | git-module | REQ-02, REQ-03, DEC-07 | touches 3 | — | claims-head-lookup |
| git-worktree-listing | One worktree listing in the git module | task | The git module lists worktrees with one process and one parser. | git-module | REQ-08, DEC-09 | touches 2 | — | git-module-foundation |
| worktree-list-through-module | Worktree list reads worktrees through the module | task | jit worktree list reads its worktrees from the module listing. | git-worktree-listing | REQ-02, REQ-08, DEC-09 | touches 1 | — | git-worktree-listing |
| lease-validation-lists-once | Lease validation lists worktrees once per run | task | jit validate --leases lists worktrees once per run, whatever the lease count. | git-worktree-listing, git-test-fixtures | REQ-02, REQ-08, DEC-03, DEC-08 | touches 2 | — | git-worktree-listing |
| git-base-comparison | Base comparison operation in the git module | task | The git module compares HEAD with a named base in the selected worktree. | git-module | REQ-03, DEC-04, DEC-09 | touches 2 | — | git-module-foundation |
| branch-drift-through-module | Branch drift validation compares in the selected worktree | task | jit validate --branch-drift compares the selected worktree through the module. | git-base-comparison | REQ-02, REQ-03, DEC-02, DEC-04 | touches 2 | — | git-base-comparison |
| main-history-guard-through-module | Main-history guard runs against the selected worktree | task | The gate-registry guard compares the selected worktree through the module. | git-base-comparison | REQ-02, REQ-03, REQ-04, DEC-04 | touches 3 | — | git-base-comparison |
| no-git-command-tests | CLI tests for commands with no git executable | task | CLI tests show core commands work and claims fail typed with no git executable. | layout-once | REQ-12, DEC-17 | creates 1, touches 1 | — | claims-head-lookup |
| git-surface-trim | Trim the layout and git module surface to production callers | task | Test-only detection entry points are gone and the module exposes only production-called items. | git-module | REQ-02, REQ-04, REQ-11, DEC-07, DEC-09 | touches 4, uncertain | — | init-git-classification-from-layout, hooks-install-from-layout, gate-evidence-through-module, worktree-info-head-lookup, init-identity-head-lookup, release-actor-through-module, worktree-list-through-module, lease-validation-lists-once, branch-drift-through-module, main-history-guard-through-module, archive-pinned-reads-batched, drop-libgit2 |
| head-tier-removal | Issue reads stop consulting git HEAD | task | Issue reads use local and primary-checkout data roots only, with no git process. | issue-read-sources | REQ-02, REQ-06, DEC-05 | touches 3 | — | — |
| two-tier-read-docs | Issue visibility pages describe two read sources | task | Four user pages describe issue reads as checkout plus primary checkout. | issue-read-sources | REQ-06, DEC-05 | touches 4 | — | head-tier-removal |
| pinned-reads-into-module | Pinned revision reads move into the git module | task | Pinned revision reads live in the git module and git_revision.rs is gone. | git-module, git-test-fixtures | REQ-02, REQ-03, REQ-11, DEC-07, DEC-13 | touches 8 | — | git-module-foundation |
| git-object-reads | Object reader and presence check in the git module | task | The git module reads many objects through one cat-file process per reader. | git-pinned-reads | REQ-10, DEC-13, DEC-14 | touches 2 | — | pinned-reads-into-module |
| archive-pinned-reads-batched | Archive planning reads pinned evidence through one git process | task | Archive planning resolves pinned documents through one object reader process. | git-object-reads, git-revision-resolver-bridge, git-test-fixtures | REQ-03, REQ-10, REQ-11, DEC-13 | touches 4 | — | git-object-reads |
| pinned-path-reads-through-module | Path reads at a commit go through the git module | task | Storage reads a path at a commit through the module object reader. | git-object-reads, document-read-versions | REQ-01, REQ-10, DEC-13 | touches 1 | — | git-object-reads, server-image-git-cli |
| document-show-reads-storage | Document show reads through storage at the version it names | task | jit doc show reads unpinned documents from the working tree through storage. | document-read-versions | REQ-06, REQ-10, DEC-06 | touches 3 | — | pinned-path-reads-through-module |
| document-diff-through-module | Document diff reads both versions through one git process | task | Both document diff variants read their two versions through one reader process. | git-object-reads, document-read-versions, git-test-fixtures | REQ-01, REQ-10, DEC-13 | touches 2 | — | document-show-reads-storage |
| document-history-through-module | Document history comes from one git log process | task | Both document history variants read one git log process through the module. | git-module | REQ-01, REQ-10, DEC-13 | touches 4 | — | git-module-foundation, server-image-git-cli |
| asset-check-through-module | Link checks test asset presence through one git process | task | The link check tests unpinned asset presence through one presence-check process. | git-object-reads, git-test-fixtures | REQ-01, REQ-10, DEC-13 | touches 2 | — | git-object-reads |
| git-dirty-paths | Dirty-path query in the git module | task | The git module reports uncommitted paths with one status process. | git-module | REQ-10, DEC-12 | touches 2 | — | git-module-foundation |
| snapshot-metadata-through-module | Snapshot source and repository facts come from the module | task | Snapshot export takes commit, origin, branch and dirtiness from the git module. | git-dirty-paths, git-head, git-pinned-reads, git-test-fixtures | REQ-05, REQ-10, DEC-10, DEC-12, DEC-14 | touches 3 | — | git-dirty-paths, git-head-lookup, pinned-reads-into-module |
| snapshot-blobs-through-module | Snapshot export reads committed files through one git process | task | A git-mode snapshot export reads its files through one reader process. | git-object-reads, git-test-fixtures | REQ-01, REQ-10, DEC-14 | touches 2 | — | snapshot-metadata-through-module, git-object-reads |
| server-image-git-cli | Server image ships the git executable | task | The server image carries the git executable its document reads need. | — | REQ-10, DEC-13 | touches 2 | — | — |
| drop-libgit2 | Remove the libgit2 binding from the workspace | task | The workspace has no git2 dependency and a policy test keeps it out. | — | REQ-01, REQ-13, DEC-02, DEC-15, DEC-16 | touches 5 | — | head-tier-removal, document-diff-through-module, document-history-through-module, asset-check-through-module, snapshot-blobs-through-module |
| binary-linkage-budget-check | Build-policy check rejects native git and OpenSSL linkage | task | The build-policy script fails when a built executable links libgit2 or OpenSSL. | — | REQ-01, DEC-15 | touches 2 | — | drop-libgit2 |
| document-read-docs | Document command reference states the show and history semantics | task | The command reference states working-tree show and git-log history semantics. | document-read-versions, git-file-history | REQ-10, DEC-06, DEC-13 | touches 1 | — | document-show-reads-storage, document-history-through-module |
| git-executable-docs | Git requirement pages name the git executable | task | The git-requirement pages say git-backed operations need the git executable. | git-module | REQ-12, DEC-07, DEC-17 | touches 3 | — | drop-libgit2 |
| libgit2-prose | Contributor documents stop presenting libgit2 as the git integration | task | Contributor documents describe the git CLI module as the git integration. | — | REQ-13, DEC-15 | touches 2 | — | drop-libgit2 |
| git-layout-once | Layout facts are detected once and reach every consumer | story | Layout detection is one probe per invocation and every consumer receives it. | — | REQ-04, REQ-07, DEC-09 | — | — | init-git-classification-from-layout, hooks-install-from-layout, git-process-budget-test |
| git-operations | Branch, worktree and base facts come from one module | story | Every git fact consumers need has one module operation run in the selected worktree. | — | REQ-05, REQ-08, REQ-09, REQ-11, DEC-10 | — | — | git-surface-trim, no-git-command-tests |
| libgit2-removal | The workspace reaches git without libgit2 | story | No libgit2 remains and every git read goes through the module. | — | REQ-01, REQ-06, REQ-10, REQ-13, DEC-05 | — | — | two-tier-read-docs, binary-linkage-budget-check, document-read-docs, git-executable-docs, libgit2-prose, archive-pinned-reads-batched |

```mermaid
flowchart LR
    N0["git-cli-test-fixture: Shared git repository and counting stand-in test fixtures"]
    N1["git-module-foundation: Storage-owned git module with a one-process probe"]
    N2["layout-probe-adoption: Layout detection uses the module's one-process probe"]
    N3["server-layout-from-data-root: Server detects its layout once from the selected data root"]
    N4["layout-fallback-removal: Storage and executor stop re-detecting the layout"]
    N5["init-git-classification-from-layout: Init reads git classification from the invocation's layout"]
    N6["hooks-install-from-layout: Hook installation targets the selected repository's common directory"]
    N7["git-process-budget-test: CLI test bounds the git processes of issue reads"]
    N8["git-head-lookup: One head lookup in the git module"]
    N9["gate-evidence-through-module: Gate runs take commit and branch evidence from the module"]
    N10["worktree-info-head-lookup: Worktree info reads its branch through the module"]
    N11["init-identity-head-lookup: Init records the worktree branch through the module"]
    N12["claims-head-lookup: Claims take their branch and git requirement from the module"]
    N13["release-actor-through-module: Release actor reads git user.name through the module"]
    N14["git-worktree-listing: One worktree listing in the git module"]
    N15["worktree-list-through-module: Worktree list reads worktrees through the module"]
    N16["lease-validation-lists-once: Lease validation lists worktrees once per run"]
    N17["git-base-comparison: Base comparison operation in the git module"]
    N18["branch-drift-through-module: Branch drift validation compares in the selected worktree"]
    N19["main-history-guard-through-module: Main-history guard runs against the selected worktree"]
    N20["no-git-command-tests: CLI tests for commands with no git executable"]
    N21["git-surface-trim: Trim the layout and git module surface to production callers"]
    N22["head-tier-removal: Issue reads stop consulting git HEAD"]
    N23["two-tier-read-docs: Issue visibility pages describe two read sources"]
    N24["pinned-reads-into-module: Pinned revision reads move into the git module"]
    N25["git-object-reads: Object reader and presence check in the git module"]
    N26["archive-pinned-reads-batched: Archive planning reads pinned evidence through one git process"]
    N27["pinned-path-reads-through-module: Path reads at a commit go through the git module"]
    N28["document-show-reads-storage: Document show reads through storage at the version it names"]
    N29["document-diff-through-module: Document diff reads both versions through one git process"]
    N30["document-history-through-module: Document history comes from one git log process"]
    N31["asset-check-through-module: Link checks test asset presence through one git process"]
    N32["git-dirty-paths: Dirty-path query in the git module"]
    N33["snapshot-metadata-through-module: Snapshot source and repository facts come from the module"]
    N34["snapshot-blobs-through-module: Snapshot export reads committed files through one git process"]
    N35["server-image-git-cli: Server image ships the git executable"]
    N36["drop-libgit2: Remove the libgit2 binding from the workspace"]
    N37["binary-linkage-budget-check: Build-policy check rejects native git and OpenSSL linkage"]
    N38["document-read-docs: Document command reference states the show and history semantics"]
    N39["git-executable-docs: Git requirement pages name the git executable"]
    N40["libgit2-prose: Contributor documents stop presenting libgit2 as the git integration"]
    N41["git-layout-once: Layout facts are detected once and reach every consumer"]
    N42["git-operations: Branch, worktree and base facts come from one module"]
    N43["libgit2-removal: The workspace reaches git without libgit2"]
    N0 --> N1
    N1 --> N2
    N2 --> N3
    N3 --> N4
    N22 --> N4
    N4 --> N5
    N2 --> N7
    N22 --> N7
    N1 --> N8
    N8 --> N9
    N4 --> N9
    N8 --> N10
    N8 --> N11
    N8 --> N12
    N12 --> N13
    N1 --> N14
    N14 --> N15
    N14 --> N16
    N1 --> N17
    N17 --> N18
    N17 --> N19
    N12 --> N20
    N5 --> N21
    N6 --> N21
    N9 --> N21
    N10 --> N21
    N11 --> N21
    N13 --> N21
    N15 --> N21
    N16 --> N21
    N18 --> N21
    N19 --> N21
    N26 --> N21
    N36 --> N21
    N22 --> N23
    N1 --> N24
    N24 --> N25
    N25 --> N26
    N25 --> N27
    N35 --> N27
    N27 --> N28
    N28 --> N29
    N1 --> N30
    N35 --> N30
    N25 --> N31
    N1 --> N32
    N32 --> N33
    N8 --> N33
    N24 --> N33
    N33 --> N34
    N25 --> N34
    N22 --> N36
    N29 --> N36
    N30 --> N36
    N31 --> N36
    N34 --> N36
    N36 --> N37
    N28 --> N38
    N30 --> N38
    N36 --> N39
    N36 --> N40
    N5 --> N41
    N6 --> N41
    N7 --> N41
    N21 --> N42
    N20 --> N42
    N23 --> N43
    N37 --> N43
    N38 --> N43
    N39 --> N43
    N40 --> N43
    N26 --> N43
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| DEC-01 Upstream 2195c98f | Chosen: each root whose code a 2195c98f task changes depends on those specific tasks, picked by function overlap and reduced so no edge is implied by another. The epic-level f4d9502a → 2195c98f edge stays. Rejected: one edge per root to the whole epic, which would hold every entry until 2195c98f closes. External dependency 0102769f → `head-tier-removal`. External dependency e1a6c8c2 → `head-tier-removal`. External dependency 945f65f3 → `snapshot-metadata-through-module`. External dependency 79aa3aa7 → `gate-evidence-through-module`. External dependency 417be079 → `gate-evidence-through-module`, `main-history-guard-through-module`. External dependency 855d027d → `document-show-reads-storage`, `document-history-through-module`, `asset-check-through-module`, `snapshot-metadata-through-module`. External dependency f8f80eea → `archive-pinned-reads-batched`. Reached transitively, so a direct edge would be rejected as redundant: 0102769f, e1a6c8c2 and 945f65f3 for `layout-fallback-removal` and `init-git-classification-from-layout` (through `head-tier-removal`), 89e7a273 and c400730c (through 0102769f), d50d6976 and c1c86312 (through 855d027d and 5e1ba005), 488eb3c1 (through 417be079), 8e82fa49, faa19def and 842b0ac6's 86d3625d (through 79aa3aa7), and 8cfbf19d, a50e9be9 and 6e8b9f31 (through 614a6f97 and c3e3e2ff). Entries that change no code an upstream task changes carry no external edge: `git-cli-test-fixture`, `git-module-foundation`, `layout-probe-adoption`, `server-layout-from-data-root`, `hooks-install-from-layout`, `server-image-git-cli`, `branch-drift-through-module`, the head-lookup consumers `worktree-info-head-lookup` and `init-identity-head-lookup`, and the module-operation entries chained on the foundation. |
| DEC-02 Upstream 842b0ac6 | Chosen: no edge onto 84745746, 52732879 or f11514dd; all three are done, and 84745746 already removed `validate.rs`'s git2 use. External dependency ec190bd5 → `pinned-reads-into-module`, which edits the `JsonFileStorage` constructors that ec190bd5's cfg-twin removal rewrites. Every entry with a 2195c98f edge follows all of 842b0ac6, since each 2195c98f root depends on it. Rejected: an edge onto 34361448, whose `claim.rs` limit resolution touches no function this plan edits. |
| DEC-03 Upstream f8a6b298 | Chosen: external dependency 614a6f97 → `claims-head-lookup`, `lease-validation-lists-once`. 614a6f97 subsumes 2f563673, dd53527f, c3e3e2ff and 98184c64, and it rewrites the renew, heartbeat and force-evict functions and the orphaned-lease remedy that these two entries also edit. External dependency c3e3e2ff → `worktree-list-through-module`. External dependency 5e1ba005 → `archive-pinned-reads-batched`, which edits `plan_archive_target` after f8 moves preview onto archive evidence. `git-surface-trim` follows 98184c64 through `claims-head-lookup`. No f8a6b298 task depends on an entry here. Rejected: replacing `WorktreePaths.git_repository` with a checkout field, which would edit `WorktreePaths` struct literals inside the f8 lease tests of `storage/claim_coordinator.rs`; `WorktreePaths::git()` derives the checkout instead. |
| DEC-04 Cross-epic edges for the steward | Not wired here, because each would delay another epic. 0f8efc37's document-convergence task depends on `document-history-through-module` (contract `git-file-history`). 5f82e09a is re-homed from the f4d9502a epic onto `git-dirty-paths`, which implies `git-module-foundation`, the module-foundation producer (`git-module`); a direct edge onto the foundation as well would be redundant. Overlaps with no edge: a807e699 moves the CLI bootstrap; 0f8efc37 REQ-01 replaces the server's layout derivation; f368ce3d REQ-10/11/12 decides whether the main-history guard and `--branch-drift` run and against which base, and this plan only routes their git calls; 22189592 keeps `PathReadError` variants that `pinned-path-reads-through-module` preserves; 182f4c03 and a015dcbb touch `snapshot.rs` and `archive.rs`; 083c9a91, 66790c84, cd8e04cc and 06ee9b67 move the claim, worktree and validate entry points. |
| DEC-05 `HEAD` tier | Chosen: delete it (`head-tier-removal`, `issue-read-sources`), which also fixes the full-id resurrection of a locally deleted issue. Rejected: a linked-only lazy `cat-file --batch` session, which goes stale in the long-lived server and adds mechanism (`@/charter/D-18`); gating today's per-issue spawns, which fails REQ-06. |
| DEC-06 Unpinned `doc show` | Chosen: working tree, as the server already reads (`document-show-reads-storage`, `document-read-versions`). Rejected: keeping the `HEAD` preference, a git-object read by a read command in a primary checkout. |
| DEC-07 Git executable | Chosen: one `Git` value whose default is the bare `git` on `PATH`, named only in the module, with a `test-support` constructor; storage's test hook takes a `Git`. No configuration surface. Rejected: a `JIT_GIT` environment variable (a new documented surface and a process-global read); a `config.toml` key (repository data naming a machine-local program). |
| DEC-08 Counting stand-in | Chosen: the shared stand-in prepended to the spawned `jit`'s `PATH`, in one `cli_repo_workflow` module (`git-process-budget-test`) after the probe and the `HEAD`-tier removal. Rejected: in-process injection alone, which does not prove the CLI wiring. |
| DEC-09 Layout facts | Chosen: `layout-once` with `WorktreePaths` as the unchanged carrier plus `git()`. `layout-probe-adoption` replaces three processes with one; the lazy fallbacks go in `layout-fallback-removal`, and `detect()` and `detect_from` in `git-surface-trim`; init and hooks read the layout. Rejected: git facts in `RepositoryLayout`, which would put process handles in the pure path layer (`@/inv/architecture-dependency-direction`); a shared lazy `OnceLock`, which keeps a hidden detection. |
| DEC-10 Branch lookup | Chosen: `git-head`. An unborn branch reports no head, which keeps claims' "make a first commit" error. A detached HEAD reports no branch name, and `Head::branch_label()` renders it as `HEAD` for every consumer that records a branch, so only init's recorded branch changes: `HEAD` when detached (was empty) and `main` without commits (was the unborn branch's name), stated in `init-identity-head-lookup`. Rejected: `symbolic-ref` semantics, which let claims succeed without commits and need a second process for the commit. |
| DEC-11 Gate evidence | Chosen: the executor passes the selected worktree's head into the checker entry point (`gate-evidence-through-module`), and in-memory runs record none. Rejected: evidence from the checker's working directory, which can be another repository or the process directory. |
| DEC-12 Dirty paths | Chosen: `git-dirty-paths` lands right after the foundation, with `--untracked-files=all` so a per-path refusal sees every untracked file. Snapshot export uses it, and 5f82e09a is re-homed onto it. Rejected: a snapshot-local status call, which would leave two implementations. |
| DEC-13 Object and history reads | Chosen: one `cat-file --batch` reader per command, answering one request at a time because snapshot export discovers assets after reading documents; presence through one `--batch-check`. Diffs keep the in-Rust `similar` rendering for byte-identical output. History is one `git log`, and its merge and deletion semantics change, as stated in `document-history-through-module`. The server image gains git first (`server-image-git-cli`). Rejected: a write-all-then-read batch only, which cannot serve snapshot's second round in one process; `git diff` output, which is not byte-identical. |
| DEC-14 Snapshot export | Chosen: commits through `resolve_commit`, files through one reader with the blob id as `blob_sha`, origin through `git remote get-url origin`, branch through `Head::branch_label()`, and dirtiness through `git-dirty-paths`, which no longer counts ignored files. Rejected: `git archive` (no per-file object id, reads the whole tree); `ls-tree` (ids without bytes). |
| DEC-15 Linkage proof | Chosen: `drop-libgit2` asserts the manifest and lock facts in `dependency_feature_policy_tests.rs`, and `binary-linkage-budget-check` makes `scripts/rust-build-budget.sh` fail on a dynamic dependency on libgit2, libssl or libcrypto, through an injectable listing with fixtures. Rejected: an `ldd` assertion inside a Rust test, since the build-policy script is where linkage policy runs. |
| DEC-16 Git test fixtures | Chosen: `git-test-fixtures` in `jit::test_harness`. The libgit2-built document fixture moves in `git-cli-test-fixture`, the snapshot unit test in `snapshot-metadata-through-module`, and both local counting wrappers in `pinned-reads-into-module`. Rejected: converging the other local `git()` helpers here; that is unrelated test churn. |
| DEC-17 No-git behaviour | Chosen: the typed mapping in `claims-head-lookup`, and `no-git-command-tests` for the no-executable condition. The outside-repository condition keeps its existing owners (`profile_no_git_lifecycle_tests.rs`, `claim_integration_tests.rs`, `@/inv/orthogonal-tests`). |
| Investigation corrections | C7: gate execution starts two processes, both removed by `gate-evidence-through-module`. C10: no production `git_program` setting exists; DEC-07 covers it. Missed sites: `read_path_bytes` → `pinned-path-reads-through-module`; the `snapshot.rs` git2 unit test → `snapshot-metadata-through-module`; `document_event_log_tests.rs` → `git-cli-test-fixture`; `archive.rs`'s literal-`git` resolver → bridge, then `archive-pinned-reads-batched`. |
| Behaviour changes | Each change is stated with its asserting test in the owning entry: a `.git` directory classified as outside version control; server layout from `--data-dir`; hooks follow the selected data root; init identity branch in detached and commit-less repositories; gate evidence from the selected worktree, none in memory; a locally deleted issue stays deleted by full id; unpinned `doc show` reads the working tree; history lists deletions and drops merges identical to a parent; snapshot dirtiness ignores ignored files. |
| Container identity | Git refuses a repository its process user does not own, so a container that runs as the default `10001` identity over a host-owned mount probes as non-git and serves without history. The documented deployment maps the owner onto the container identity, which passes git's check. Chosen: no `safe.directory` override in the image. |
| Interim server window | `pinned-path-reads-through-module` and `document-history-through-module` follow `server-image-git-cli`, so the image carries git before any server path depends on it. |
| Lazy-detection test sweep | `layout-fallback-removal` records a footprint uncertainty: in-process tests that relied on lazy git detection are found by running the suite. Its criteria require those tests to supply their paths explicitly. |
| Shared-file contention | `storage/git.rs` and `git_module_tests.rs` receive one operation per mechanism entry, in distinct functions; `commands/document.rs`, `commands/worktree.rs`, `commands/claim.rs` and `storage/json.rs` are edited by several entries in distinct functions. Same-function edits are ordered by edges: show before diff, claims before the release actor, and the module-operation entries before their consumers. The remaining footprint advisories are distinct functions, and workers rebase. |

## Investigation sources

- [Investigation](investigation.md): the consumer sweeps, spawn measurements, primitive verification and option analyses for the CLI surface, layout, branch lookups and no-git behaviour, and for libgit2 removal and `HEAD`-tree reads.
