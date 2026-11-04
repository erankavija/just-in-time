# Investigation: Reach git through one storage-owned CLI module instead of libgit2 (f4d9502a)

> Grounding for planning node 28ccf9a9. Two scoped investigations against `main` after the 2195c98f and f8a6b298 breakdowns were created. The shared integration harness now lives at `crates/jit/src/test_harness.rs` (`jit::test_harness`, moved in 6470711b4).

## Part: The git CLI surface and the one module

## f4d9502a investigation — git CLI surface, layout, branch lookups, no-git (REQ-02/03/04/05/08/09/11/12)

Code read at HEAD `4d7b416e5`. Line numbers in the epic background predate several commits; current lines are given below. Spawn counts were measured with a logging `git` wrapper on `PATH` against the installed dogfood binary (`~/.cargo/bin/jit`, built 2026-10-03). It is older than HEAD, but the code paths it exercises are unchanged at HEAD.

### 1. Claim classification

| # | Claim (epic background / REQ) | Verdict | Current evidence |
|---|---|---|---|
| C1 | Layout detection spawns three `git rev-parse` per invocation | valid-and-open | `storage/worktree_paths.rs:64` (`--is-inside-work-tree`), `:87` (`--git-common-dir`), `:112` (`--show-toplevel`). Measured: `query available`, `issue show` → exactly these 3 spawns. All three use literal `"git"`, not the configured program. |
| C2 | `validate` detects again (`validate.rs:3044`) | valid-and-open, moved | `commands/validate.rs:2726-2731` `validate_claims_index_at` → `WorktreePaths::detect_from`. Caller: `validation/repository.rs:248` → `:540-551` `validate_machine_local_claims`, which runs on every `validate_repository` call (`commands/config.rs:335`, `gate_check.rs:438,1065`, `init.rs:284`, `validate.rs`). Measured: `jit validate --json` spawns the 3-probe twice (6 rev-parse). f8a6b298 task 98184c64 deletes this path (§7). |
| C3 | `claim` detects again (`claim.rs:24`) | valid-and-open | `commands/claim.rs:24-32` `is_inside_git_work_tree` spawns `rev-parse --is-inside-work-tree`, and only on the branch-lookup failure path (`:53`). |
| C4 | `init` detects again (`init.rs:540`) | valid-and-open | `commands/init.rs:538-547` `worktree_is_git_work_tree`, called from `gitattributes_claim` `:524-535` ← `:175`. |
| C5 | Main-history guard detects again (`worktree.rs:456`) | valid-and-open | `commands/worktree.rs:448-478` spawns `rev-parse --git-dir` **in the process cwd**. Callers: `commands/gate.rs:1098,1147,1243,1258`. |
| C6 | Six branch-name lookups | valid-and-open, moved | `commands/claim.rs:45-62`, `commands/worktree.rs:46-62`, `storage/claim_coordinator.rs:979-997`, `commands/mod.rs:3307-3322`, `gate_execution.rs:567-581`, `commands/snapshot.rs:25-29` (libgit2). Their semantics differ (§4.3). |
| C7 | Exec checker spawns up to four times for commit, branch and **tree** evidence (`gate_execution.rs:529-664`) | invalid-as-stated | Two spawns per gate run now: `get_git_commit` `gate_execution.rs:551-565` and `get_git_branch` `:567-581`, called at `:102-103`. No tree spawn remains: the declared-input digest walks the filesystem (`:449-543`). Both spawns use `"git"` from `PATH` and run in the checker `working_dir`. With in-memory storage that directory is the process cwd (`commands/gate_check.rs:668-674`). |
| C8 | Lease validation spawns `git worktree list` once per lease (`validate.rs:2824`, `2995-3035`) | valid-and-open, moved | `commands/validate.rs:2482-2537` calls `check_worktree_exists` (`:2682-2723`) inside the per-lease loop at `:2510`. Measured: `jit validate --leases --json` in this repo → 9 `worktree list --porcelain` spawns. A second porcelain parser exists: `commands/worktree.rs:64-120` `parse_git_worktree_porcelain`, used at `:231`. |
| C9 | Branch-divergence checks run `merge-base` / `rev-parse` in the process cwd (`worktree.rs:389-470`, `validate.rs:2735-2770`) | valid-and-open, moved | `commands/worktree.rs:388-424` `check_branch_divergence` and `commands/validate.rs:2421-2471` `validate_branch_drift`: neither passes `-C` or `current_dir`. Both compute the same fact (D-20 duplicate). Measured: `validate --branch-drift` → `merge-base HEAD origin/main`, `rev-parse origin/main` with no `-C`. |
| C10 | "The configured `git_program`" exists | invalid-as-stated (partially) | There is **no production configuration surface**. `JsonFileStorage.git_program` defaults to `"git"` (`storage/json.rs:404,442`). Its only setter is `#[cfg(test)] with_git_program` (`json.rs:481-489`), used in `storage/repository_state_store_tests.rs:2615,2653,2702,2730`. `GitRevisionResolver::with_git_program` (`git_revision.rs:138`) is public, but production calls it only from `json.rs:503`. `archive.rs:184` uses `GitRevisionResolver::new` (literal `"git"`). No doc, env var or config key names a git executable. |
| REQ-02 | One storage-owned module spawns git | open | 20 production `Command::new("git")` sites in 9 files, plus 2 honouring sites in `git_revision.rs` (§3.1). |
| REQ-03 | One configured executable; selected worktree; never process cwd | open | Process-cwd sites: `worktree.rs:389,399,456`, `validate.rs:2426,2444`, server `crates/server/src/main.rs:67` (`WorktreePaths::detect()` = cwd), `gate_check.rs:670` (in-memory cwd fallback feeding gate evidence), and `hooks.rs:82-120` (`find_git_dir` from cwd; a filesystem walk, no spawn). |
| REQ-04 | Layout detected once, by ≤1 process, reaching every consumer | open | Main already detects once and threads the result (`main.rs:1987-1991`, `:2010` storage, `:2050-2052` executor). Re-detections: C2-C5 above; lazy fallbacks `storage/json.rs:557-575` and `commands/mod.rs:2200-2216`; `claim.rs:724` builds an executor without worktree paths; server `main.rs:67` plus its storage and executor fallbacks; `hooks.rs:82`. |
| REQ-05 | One branch lookup | open | Six lookups (C6). |
| REQ-08 | Lease validation lists worktrees at most once | open | C8. |
| REQ-09 | Gate commit and branch evidence go through the module | open | C7. |
| REQ-11 | Module exposes only production-called operations | partly done | 842b0ac6 `trim-git-revision-access` (**52732879, done**) removed `changed_paths_between`, `changed_worktree_paths`, `inspect_pinned_target`, `parse_changed_paths`, `parse_pinned_tree_entry` and `ChangedPathsFailed`, and made `GitRevisionError: Clone` (`git_revision.rs:24-32`, `Arc<io::Error>`). Remaining non-production surface: `WorktreePaths::detect()` (prod caller: server only), `WorktreePaths::is_main_worktree` (test-only; only call `commands/worktree.rs:1004`, inside `#[cfg(test)]` at `:479`), the public `git_repository` field duplicated by `is_git_repository()`, and `GitRevisionResolver::with_git_program` (production caller is crate-internal only). |
| REQ-12 | Core commands work outside git and without a git executable; claims and leases give the typed error | partially valid | Core commands: confirmed working with an empty `PATH` (`issue create`, `query available` succeed). **Claims without a git executable are wrong**: `claim list --json` with no git on `PATH` → `REPOSITORY_NOT_FOUND`, exit 3. The spawn error is `.context(...)` at `claim.rs:50` and is never mapped to the then claim-specific Git requirement error. Outside a repo with git present → the then claim-specific Git requirement code, exit 10 (correct). |

Totals for this scope: already-done 0 (REQ-11 is partly done), valid-and-open 12 (C1-C6, C8, C9, REQ-02, -03, -04, -05, -08, -09, -12 partially), invalid-as-stated 2 (C7, C10).

### 2. Prior-art sweep

- **Charter D-21** (`dev/vision/9db27a3a-charter.md:398-410`): every git access goes through one storage-owned module that runs the CLI. Layout facts are "detected once per invocation and passed down". The rejected options are "libgit2 everywhere" and "a module hiding both backends". Reasoning: "One spawn per needed fact".
- **Charter D-4** (`:83-95`): only `jit claim acquire/release/renew/heartbeat/status/list` require git, failing with the then claim-specific Git requirement error, exit 10.
- `docs/concepts/guarantees.md:221,233-250` documents the NoCommits case ("`git init` alone is not enough") and lists `force-evict` as git-requiring. `docs/reference/claim.md:98-104` ("resolvable HEAD"). `docs/reference/error-codes.md:26` (the then claim-specific Git requirement code → 10). `docs/reference/worktree-validate.md:338` (`--branch-drift` "requires git"). `docs/how-to/troubleshooting.md:345,396-437` (`origin/main` message, `git rev-parse --git-common-dir`).
- No doc, config reference or `dev/` study describes a git-executable setting. `dev/architecture/*.md` say nothing about git access.
- Audit: `findings-tables.md` rows 9, 21 and 24 recommend "one `rev-parse` call returning three lines", "one `rev-parse`" for evidence, and "list worktrees once". `jit-audit-report.md:77` originally proposed a *git2-backed* module; D-21 superseded that.
- Related in-flight epics: f368ce3d (domain-agnostic engine) REQ-10/11/12 owns *whether* the main-history guard and `--branch-drift` run and *which* base branch they use (the f4d9502a notes defer this). 0f8efc37 REQ-01 has the server obtain layout from the core constructor. a807e699 (df129d9e) moves the CLI prelude, including `detect_for_data_root` and `configure_worktree_paths`, into a library function.

### 3. Consumer sweep

#### 3.1 Production git spawn sites (by module)

| Module | Site | Operation | Program | Directory |
|---|---|---|---|---|
| storage/worktree_paths.rs | `:64`, `:87`, `:112` | layout probe ×3 | `"git"` | `-C <probe>` |
| storage/git_revision.rs | `:443` (`run`), `:512` (`cat-file --batch`) | pinned reads | configured | `current_dir(repo_root)` |
| storage/json.rs | `:905` | `git show HEAD:<issue>` per missing issue (REQ-06, other part) | `"git"` | worktree root |
| storage/claim_coordinator.rs | `:982` | branch (`--abbrev-ref HEAD`) | `"git"` | `-C worktree_root` |
| commands/claim.rs | `:25` is-inside-work-tree; `:46` branch; `:304` `config user.name` | — | `"git"` | `-C worktree_root` |
| commands/worktree.rs | `:47` branch; `:231` worktree list; `:389` merge-base; `:399` rev-parse origin/main; `:456` rev-parse --git-dir | — | `"git"` | `:47,:231` worktree root; **`:389,:399,:456` process cwd** |
| commands/validate.rs | `:2426` merge-base; `:2444` rev-parse origin/main; `:2687` worktree list (per lease) | — | `"git"` | **`:2426,:2444` process cwd**; `:2687` `-C repo root` |
| commands/init.rs | `:540` is-inside-work-tree | — | `"git"` | `-C worktree` |
| commands/mod.rs | `:3308` `branch --show-current` | — | `"git"` | `-C worktree_root` |
| gate_execution.rs | `:552` `rev-parse HEAD`; `:568` `--abbrev-ref HEAD` | — | `"git"` | checker working dir |
| commands/archive.rs | `:184` `GitRevisionResolver::new` → spawns via git_revision | pinned reads | **`"git"` (ignores configured)** | worktree root |

Test-only spawns (not REQ-02 scope): `gate_execution.rs:736`, `commands/hooks.rs:237`, `commands/mod.rs:4396`, `commands/worktree.rs:671,692`, `commands/archive.rs:1687`, `profile/repository_package.rs:299,630` (whole file `#[cfg(test)] mod tests`), `storage/json.rs:2068-2793`, `storage/repository_state_store_tests.rs`, `crates/server/src/routes.rs:2207`.

libgit2 sites (REQ-01/06/10, other parts): `storage/json.rs:829-860,1525`, `commands/snapshot.rs:19-29,55,140,199,362`, `commands/document.rs:495-1883`, `commands/validate.rs:2153-2240` (deleted by 842b0ac6 **84745746**, dispatched).

#### 3.2 Layout carrier `WorktreePaths` consumers (mentions per file)

`commands/worktree.rs` 12, `commands/claim.rs` 11, `commands/mod.rs` 10, `storage/claim_coordinator.rs` 10, `storage/worktree_paths.rs` 7, `main.rs` 6, `storage/json.rs` 5, `test_utils.rs` 5, `storage/worktree_identity.rs` 4, `commands/validate.rs` 3, `tests/cli_repo_workflow/worktree_identity_tests.rs` 3, and 1 each in `commands/template.rs`, `storage/mod.rs`, `storage/claim_coordinator_proptests.rs`, `tests/cli_repo_workflow/init_tests.rs`, `tests/scratch_build/repository_state_cutover_tests.rs` and `crates/server/src/main.rs`.

Struct-literal constructions, which a field change would touch: `commands/worktree.rs` 3, `main.rs` 2, `storage/claim_coordinator.rs` 6, `claim_coordinator_proptests.rs` 1, `test_utils.rs` 2, `worktree_identity_tests.rs` 1.

Detection entry points: `detect()` (`server/main.rs:67`; test `worktree_paths.rs:272`); `detect_for_data_root` (`main.rs:1987`, `json.rs:568`, `mod.rs:2211`; tests `worktree_identity.rs:293,344`, `worktree.rs:1040`, `tests/.../worktree_identity_tests.rs:15`); `detect_from` (`validate.rs:2730`; tests `mod.rs:4288,4318,4403`).

#### 3.3 Git-program and resolver consumers

`with_git_program` / `GitRevisionResolver`: `storage/json.rs:482,503`, `storage/mod.rs:60` (re-export), `commands/archive.rs:31,92,99,184`, `storage/repository_state_store.rs:1135`, and `tests/fast_docs_templates/git_revision_tests.rs` (17 references).

#### 3.4 Gate evidence consumers

`GateRunResult.commit/branch` (`domain/types.rs:952,955`), `output.rs:2399,2401`, `snapshot.rs:42,44`. Tests: `gate_execution.rs:720` (`test_git_context_graceful_degradation`) and `:1112` (`test_execute_gate_checker_records_head_commit`).

#### 3.5 Other surfaces

- mcp-server, web and test-vectors: no git-executable or layout code. The MCP bridge is schema-generated.
- Shipped hooks: `scripts/hooks/pre-commit:37-45` compares against `origin/main` (f368ce3d scope). `commands/hooks.rs:25-120` `install_hooks`/`find_git_dir` re-derives the common dir from cwd by parsing a `.git` file. Called from `main.rs:6490`.
- `storage/discovery.rs:49,90` uses `.git` existence as the `.jit` discovery boundary. This is a filesystem check that runs before probing and must work without git (D-4). Recommend keeping it as data-root discovery rather than layout detection.

#### 3.6 No-git tests (REQ-12)

- Outside a repo, git present: `tests/cli_repo_workflow/claim_integration_tests.rs:1189,1213,1229,1266` (exit 10) and `:1364,1392` (NoCommits hint); `tests/cli_repo_workflow/profile_no_git_lifecycle_tests.rs:46` (core lifecycle with no `.git`); `worktree_paths.rs` tests (`test_detect_in_non_git_directory`, `test_detect_for_data_root_outside_version_control_is_not_linked`); `output/failure.rs:683`.
- **No git executable**: there is no test for core commands or for claims. `tests/cli_issue/search_exit_status_tests.rs:41-52` runs `search` with an empty `PATH`, which incidentally proves startup survives without git. It is not an owning test.

### 4. Primitive verification

#### 4.1 One layout process

A single `git -C <dir> rev-parse --path-format=absolute --git-common-dir --show-toplevel` gives both facts in one process (verified, git 2.56):

- **primary**: `<root>/.git` and `<root>`;
- **linked**: the common `.git` and the linked root;
- **unborn**: succeeds;
- **outside a repo**: exit 128 with empty stdout;
- **inside `.git`**: exit 128.

Spawn failure (ENOENT) means git is unavailable and is classified as non-git for core commands. `--is-inside-work-tree` is implied by `--show-toplevel` succeeding. `--path-format=absolute` needs git ≥ 2.31. No minimum git version is documented. Without the flag, today's canonicalize-relative step (`worktree_paths.rs:104-109`) keeps working.

Including `HEAD`/`--abbrev-ref HEAD` in the same probe fails the whole probe (exit 128) on an unborn branch, so HEAD facts cannot be folded into the layout probe while keeping "≤1 process" in every state.

#### 4.2 Configured executable honoured

There is no mechanism today (C10). Delivering it means one `Git { program }` value, built once at bootstrap and carried by the layout handle, with every operation spawning `Command::new(&program)`. A CLI test can install a counting stand-in by prepending it to the child's `PATH` (pattern in `tests/cli_repo_workflow/profile_acceptance_tests.rs:31-75` and `search_exit_status_tests.rs`). This needs no new configuration surface.

#### 4.3 Branch semantics today and a canonical lookup

| Site | Command | Detached HEAD | Unborn branch | Failure |
|---|---|---|---|---|
| claim.rs:45 | `rev-parse --abbrev-ref HEAD` | `"HEAD"` | spawns again to classify → the then claim-specific Git requirement error (`NoCommits`) | spawn error → untyped (exit 3) |
| worktree.rs:46 | same, `current_dir` | `"HEAD"` | generic bail | generic |
| claim_coordinator.rs:979 | same | `"HEAD"` | `git_command_failed` → callers `.ok()` → `None` (`:483,616,778`) | — |
| mod.rs:3308 | `branch --show-current` | `""` | unborn name (e.g. `trunk`) | `"main"` fallback |
| gate_execution.rs:567 | `--abbrev-ref HEAD` | `"HEAD"` | `None` | `None` |
| snapshot.rs:25 | libgit2 `head().shorthand()` | `"HEAD"` | `None` | `None` |

Verified: `git rev-parse HEAD --abbrev-ref HEAD` prints `<oid>\n<branch>`. For detached HEAD it prints `<oid>\nHEAD`. On an unborn branch it exits 128.

A canonical `head() -> Result<Option<Head>, GitError>` with `Head { commit, branch: Option<String> }`, where `None` means unborn, needs one process. It preserves 5 of the 6 semantics exactly (claims NoCommits, worktree, coordinator, gate evidence, snapshot), and it removes the extra `is_inside_git_work_tree` spawn because the layout already says whether a repository exists. Only init identity (`mod.rs:3308`) changes, in edge states: detached becomes `"HEAD"` instead of `""`, and unborn takes a fallback instead of the unborn name.

`symbolic-ref` semantics (unborn → branch) would instead let claims succeed in a commit-less repo. That breaks `claim_integration_tests.rs:1364,1392` and `guarantees.md:221`, and gate evidence would need a second spawn for the commit.

#### 4.4 Worktree-scoped cwd

Every module operation runs `git -C <worktree_root>`. This also removes the process-cwd sites. For gate evidence, the checker working directory is a subdirectory of the worktree, so `-C worktree_root` is equivalent. With in-memory storage there is no worktree: evidence should be `None` rather than computed in cwd.

#### 4.5 One worktree listing per run

`git worktree list --porcelain` reports every worktree with `HEAD <oid>` (all zeros when unborn) and `branch refs/heads/x` or `detached`. Verified. One parser in the module serves `worktree list` (`worktree.rs:231`) and lease validation (`validate.rs:2687`). Lease validation builds the set of worktree ids once, before the loop.

#### 4.6 No-git operation

Startup already degrades: `worktree_paths.rs:64-70` turns a spawn error into "not a repo". Claims need the typed error on **both** "not a repo" and "git unavailable". Today the second gives exit 3 (§1, REQ-12). Mapping a missing layout handle to the then claim-specific Git requirement error (`NoRepository`) at one place fixes both.

#### 4.7 Cost

Measured per spawn in this repo (8282 tracked files, 13 worktrees): `rev-parse` (layout) ~1 ms, `symbolic-ref` ~1 ms, `worktree list --porcelain` ~2 ms, `status --porcelain=v2 --branch -uno` ~7 ms. `status` refreshes the index and can take `index.lock`, so it is unsuitable as a HEAD probe.

### 5. Design options

#### 5.1 Layout object and how consumers receive it (REQ-04)

- **A (recommended)**: `storage::git` owns `Git { program }`, `Git::probe(dir) -> Option<GitCheckout>` (one process) and `GitCheckout { program, worktree_root, common_dir }`, which holds every operation (`-C worktree_root`).
  - `WorktreePaths` replaces `git_repository: bool` with `git: Option<GitCheckout>`. `common_dir`, `worktree_root`, `local_jit` and `shared_jit` stay. The non-git fallback keeps `worktree_root/.git` for `shared_jit`.
  - Consumers already receive `WorktreePaths`: claims (`main.rs:7235-7916`), worktree commands (`:7953-8057`), executor (`mod.rs:2194`), storage (`json.rs:545`). They call `paths.git()`.
  - Remaining consumers get it threaded:
    - init: `gitattributes_claim` takes `paths.git().is_some()`.
    - validation: the `validate_machine_local_claims` path is deleted by f8a6b298 98184c64; if f4 lands first, pass the executor's paths.
    - gate execution: `gate_check.rs:1323-1342` passes `self.require_worktree_paths()?.git().map(GitCheckout::head)` evidence into `execute_gate_checker_with_context`.
    - main-history guard and branch drift: become executor methods using `paths.git()`.
    - hooks install: `install_hooks(paths.common_dir)`.
    - `execute_recover`: `claim.rs:724` adds `.with_worktree_paths(paths.clone())`.
  - Delete the lazy fallbacks (`json.rs:557-575`, `mod.rs:2200-2216`), plus `detect()` and `detect_from`. One bootstrap probe remains (`main.rs:1987`, later in a807e699's library function).
  - Layer fit: domain untouched; storage owns the spawn and `Head`; commands orchestrate; `gate_execution.rs` receives values, not git.
- **B**: Put git facts in `RepositoryLayout` (`repository_state/path.rs:472`). Every consumer already has a layout. Rejected: `repository_state` is the pure capability/path layer, and git process handles there invert `@/inv/architecture-dependency-direction`.
- **C**: Keep a lazy probe, memoized in one shared `Arc<OnceLock>` held by both storage and executor. This keeps "once" without changing constructors. It still hides a detection inside storage and leaves the server fallback path. Simpler churn, but weaker against REQ-04's "no consumer detects them again".

#### 5.2 Configured executable (REQ-03)

- **A (recommended)**: `PATH`'s `git`, resolved once into `Git { program }` at bootstrap. In-process tests use `Git::with_program`; CLI tests prepend a stand-in to the child `PATH`. No new surface (D-18).
- **B**: `JIT_GIT` env var. It adds a documented surface and a process-global read.
- **C**: A config.toml key. Rejected: the executable is machine-local, not repository data.

#### 5.3 Branch lookup (REQ-05)

- **A (recommended)**: `GitCheckout::head()` via `rev-parse HEAD --abbrev-ref HEAD` (§4.3). It is the only branch lookup; gate evidence, claims, coordinator, worktree info, init identity and snapshot all call it.
- **B**: `symbolic-ref` semantics. It changes claims on unborn repos and needs a second spawn for commit evidence.
- **C**: Derive from `worktree list --porcelain` matched on `worktree_root`. One parser in total, but it is O(worktrees) and path-matching is fragile.

Coordinator: drop `ClaimCoordinator::get_current_branch` and pass the branch the command already resolved (`claim.rs:99,235,...`) into the acquire path, or read it from `paths.git()`.

#### 5.4 Module API (REQ-11), production-called only

`Git::probe`; `GitCheckout::{head, list_worktrees, user_name, resolve_commit, read_pinned_path, inspect_pinned_targets}`; and a base-comparison op for the guard and drift (`merge-base HEAD <base>` + `rev-parse <base>`, or `merge-base --is-ancestor`). The exact shape follows f368ce3d's base-branch decision.

REQ-06 (HEAD-tree reads) and REQ-10 (document, snapshot, asset) add their own operations.

Delete or make private: `WorktreePaths::detect`, `detect_from`, `is_main_worktree` (test-only), the public `git_repository` field, `GitRevisionResolver::new`/`with_git_program` (folded into `GitCheckout`), and `json.rs` `git_program` with its `#[cfg(test)]` setter (replaced by the checkout handle). `resolve_commit` keeps a production caller (`read_pinned_path`; snapshot `--at` after REQ-10).

### 6. Architecture fit / invariant check

- `@/inv/architecture-dependency-direction`: holds with option 5.1 A. `validation/repository.rs:542` calls up into `commands::validate_claims_index_at` today; f8a6b298 98184c64 deletes that call.
- `@/inv/convention-convergence`: two porcelain parsers (`worktree.rs:64`, `validate.rs:2697-2703`), two divergence computations (`worktree.rs:388`, `validate.rs:2421`), six branch lookups and two layout detectors (`worktree_paths.rs`, `hooks.rs:82`) converge onto module operations.
- `@/charter/D-4`: claims without a git executable currently break the typed contract (§1, REQ-12). Fix it at the one `paths.git()` → the then claim-specific Git requirement error mapping.
- `@/inv/semantic-types`: branch names stay `String` in `Lease.branch`, `WorktreeIdentity.branch` and `GateRunResult.branch`. 2195c98f adds no `BranchName` or commit-OID type. `ArtifactVersion` (`domain/artifact_plan.rs:103`) is the existing canonical commit type.
- `@/inv/orthogonal-tests`: the module behaviour matrix (outside repo, git missing, unborn, detached, linked) belongs in one module suite, mostly migrated from `worktree_paths.rs:250-336`, `gate_execution.rs:720` and `git_revision_tests.rs`. Consumers prove only that they route through the module, e.g. a counting stand-in shows `validate --leases` lists once, and a cwd-elsewhere plus `JIT_DATA_DIR` run proves the selected worktree is used.
- `@/inv/pid-safety` and `atomic-writes`: unaffected.

### 7. Sequencing (scope of this part)

Each step leaves the workspace building, with clippy clean and tests green.

1. **git-module-probe**: create `storage/git.rs` with `Git`, `GitCheckout`, `probe` (one process) and `head`. `WorktreePaths` carries `Option<GitCheckout>`. Rewire `main.rs:1987` and server `main.rs:67` (probe the data dir; this deletes `detect()`). Delete the lazy fallbacks (`json.rs:557-575`, `mod.rs:2200-2216`) and fix `claim.rs:724`. Move the module behaviour suite here.
   - Files: `storage/{git.rs,worktree_paths.rs,json.rs,mod.rs}`, `main.rs`, `commands/mod.rs`, `commands/claim.rs`, `crates/server/src/main.rs`, test_utils and struct-literal sites (§3.2).
   - Needs 2195c98f done (epic edge). No specific key.
2. **one-branch-lookup**: route `claim.rs:45`, `worktree.rs:46`, `mod.rs:3308`, `gate_execution.rs:551-581` and the coordinator through `head()`. Delete `is_inside_git_work_tree`. Map a missing checkout or git to the then claim-specific Git requirement error (REQ-12 fix plus the no-git-executable CLI test).
   - Coordinator part: touches the code rewritten by f8a6b298 **dd53527f** (lease-ops-through-lock) and **2f563673** (lease-per-issue-records). Recommended edge: this step → after 2f563673 (ours waits). Alternatively split the coordinator edit into its own late step.
   - Snapshot `snapshot.rs:25` waits for the REQ-10 snapshot step (other part).
   - Same-file 2195c98f keys that land first: **a50e9be9** agent-identity-assignee, **6e8b9f31** system-actor-constants, **8cfbf19d** lease-worktree-ids.
3. **gate-evidence-through-module**: `gate_check.rs:1323-1342` passes `head()` evidence; `gate_execution.rs` loses its spawns (REQ-09). Independent of other epics.
4. **worktree-listing-once**: one `list_worktrees` parser; `worktree list` and `validate_leases` hoisted (REQ-08).
   - Touches `validate.rs:2482-2537`, which f8a6b298 **c3e3e2ff** (lease-reader-migration) and **915be53e** (lease-read-api) rewrite, and `worktree.rs:252-290`, which 2195c98f **8cfbf19d** retypes.
   - Recommended edge: this step → after c3e3e2ff.
   - f8a6b298 **614a6f97** also edits the `validate.rs:2515` remedy text.
5. **process-cwd-sites**: `check_branch_divergence` and `validate_branch_drift` become one module op run against `paths.git()`. `enforce_main_only_operations` becomes executor-scoped. `init.rs:540`, `hooks.rs:82` and `claim.rs:304` (user.name) go through the module.
   - f368ce3d (unplanned) later decides base branch and existence; no edge now. Flag for the steward.
6. **validation re-detection**: if f8a6b298 **98184c64** has landed, nothing remains. Otherwise thread paths into `validation/repository.rs:248`. Recommended edge: our step → after 98184c64.
7. **module-surface-trim (REQ-11)**: delete `is_main_worktree`, `detect_from`, the public `git_repository` field and `GitRevisionResolver::new`/`with_git_program`; fold `archive.rs:184` onto `paths.git()`. Last, after the REQ-06 and REQ-10 parts add their operations.

Cross-epic seams to flag (none proposed as default edges onto other epics' tasks):

- a807e699 (CLI bootstrap library function) moves `main.rs:1940-2052`. An edge from it onto step 1 would delay a807e699; prefer no edge, letting whichever lands second rebase.
- 0f8efc37 REQ-01 replaces the server's own detection. Step 1's server edit is one line and compatible.
- 083c9a91, 66790c84, cd8e04cc and 06ee9b67 (df129d9e facades and handlers) move claim, worktree and validate entry points. Expect textual overlap; no edge proposed.
- 34361448 (842b0ac6 unify-lease-limit-resolution, pending) edits `claim.rs:107-111`; minor overlap.
- 84745746 (842b0ac6, dispatched) removes `validate.rs:2152-2240` git2. Lands before f4 in practice.
- 5f82e09a (lead-friction) depends on f4d9502a and will add a working-tree status operation to the module.

## Part: libgit2 removal and HEAD-tree reads

## f4d9502a investigation: libgit2 removal and HEAD-tree reads (REQ-01, REQ-06, REQ-07, REQ-10, REQ-13)

Code read at HEAD `4d7b416e5` (2026-10-21). Paths are repository-relative.

### 1. Claim classification

| # | Claim | Verdict | Current evidence |
|---|---|---|---|
| C1 | `git2 = "0.21"` in `crates/jit/Cargo.toml:31`; unused in `crates/server/Cargo.toml:57` | valid-and-open (lines drifted) | `crates/jit/Cargo.toml:34`, `crates/server/Cargo.toml:61`. `crates/server/src` and `crates/server/tests` contain no `git2` use. |
| C2 | `load_index_from_git` (`json.rs:825-860`) opens libgit2 on every read, including the primary checkout | valid-and-open | `storage/json.rs:823-863`. Called from `load_aggregated_index` (`json.rs:791`). That function backs `list_issues` (`:1302-1307` through `read_indexed_issues` `:811`) and prefix resolution (`:1276`). No linked-checkout guard exists. |
| C3 | The installed binary links system `libgit2.so`, which links `libssl`/`libcrypto` | valid-and-open, with a nuance | `ldd ~/.cargo/bin/jit` and `ldd target/debug/jit` both list `libgit2.so.1.9`, `libssl.so.3`, `libcrypto.so.3`, `libssh2.so.1`, `libllhttp`, `libpcre2-8`, `libz`. Nuance: git2 0.21 has `default = []` (`~/.cargo/registry/.../git2-0.21.0/Cargo.toml:44-47`), so `Cargo.lock` holds no `openssl-sys` (`libgit2-sys` deps are `cc, libc, libz-sys, pkg-config`, `Cargo.lock:1447-1456`). OpenSSL arrives only through the system libgit2 that `libgit2-sys`'s build script finds with pkg-config. The musl release build (`.github/workflows/release-artifacts.yml:98-135`) vendors libgit2 without https, so that artifact links libgit2 statically and has no OpenSSL. |
| C4 | perf: 28% `git_libgit2_init`, 21% OpenSSL CA loading, about 30% of CPU | unverifiable | Not re-measured. The audit (`findings-tables.md:110`) rates it "medium; from a perf call graph only". |
| C5 | The HEAD issue fallback spawns `git show` once per missing issue and ignores `git_program` (`json.rs:894-917`) | valid-and-open | `json.rs:892-920`, with the literal `Command::new("git")` at `:905`. Observed: `jit query available` in linked checkout `.agents/worktrees/agent-84745746` spawned 2 `git show HEAD:.jit/issues/<id>.json`, both misses, plus 3 `rev-parse`. `git_program` (`json.rs:404`) is a test-only seam: it is set only by `#[cfg(test)] with_git_program` (`:481-489`), defaults to `"git"` (`:442`), and is read only for pinned reads (`:503`). No user configuration names a git executable. |
| C6 | document show/history/diff (`document.rs:383-815`), asset check (`:1099`, `:1854`) and snapshot export (`snapshot.rs:19-60`, `199`, `362`) call libgit2 from commands | valid-and-open (lines drifted) | document.rs: `show_document_content` `:489-544` (git2 `:495,513,519`); `document_history` `:546-567`; `document_diff` `:569-619`; server variants `get_document_history` `:706-729` and `get_document_diff` `:739-790`; `read_file_from_git` `:823-837`; `get_file_history` `:839-889`; `check_document_links` opens git2 at `:1128`; `check_asset_in_git` `:1883-1899`, called per asset at `:1164`. snapshot.rs: `origin_remote_url` `:19`; `current_branch_name` `:25`; `determine_snapshot_source_mode` `:55`; `read_from_git` `:138-170`; `read_content` `:199` (opens the repository once per file); `get_repo_info` `:362` (remote, branch, `statuses(None)`). |
| C7 | Not in the background: storage `read_path_bytes` at a commit uses libgit2 | valid-and-open, add to scope | `json.rs:1506-1551` (`use git2::Repository` at `:1525`). The server's raw-document endpoints consume it through `CommandExecutor::read_path_bytes` (`document.rs:815`) and `crates/server/src/routes.rs`. This is the "pinned artifact read" surface REQ-10 names. |
| C8 | Not in the background: `validate_document_references` / `check_file_exists_in_git` use libgit2 | in flight elsewhere | `commands/validate.rs:2152-2247`, reachable only from the uncalled `validate_integrity_silent` (`:1521`). 842b0ac6 key `remove-silent-integrity-validator`, issue **84745746** (ready, dispatched), deletes them. |
| C9 | Not in the background: test fixtures use git2 | valid-and-open | `commands/snapshot.rs:1075-1100` (unit test `test_git_metadata_extracts_remote_and_branch_with_fallible_text_accessors`). `tests/fast_docs_templates/document_event_log_tests.rs:259-283` (`test_add_pinned_document_uses_pinned_document_and_asset_bytes`) builds its commit with git2 through the normal dependency; there is no git2 dev-dependency. |
| REQ-01 | No git2 crate or lock entry; binary links neither libgit2 nor OpenSSL | valid-and-open | See C1 and C3. Removal also drops `libgit2-sys`, `libz-sys` and `pkg-config` from the lock. `cargo tree -i libz-sys` and `cargo tree -i pkg-config` show git2 as their only path; `cc` stays for `ring`. After removal the only `-sys` crates are platform crates (`linux-raw-sys`, `windows-sys`, `dirs-sys`, `inotify-sys`, ...). |
| REQ-06 | Primary checkout reads no git objects; linked checkout uses at most one git process for HEAD reads | valid-and-open | Primary: libgit2 reads `HEAD:index.json` on every aggregated read (C2). Linked: N `git show` processes (C5). |
| REQ-07 | A counting stand-in git executable shows at most one git process for `query available` and `issue show` | valid-and-open | No CLI-level process-count test exists. In-process counting wrappers exist twice (§2). Today's count in this primary checkout is 3 (`rev-parse` ×3, `worktree_paths.rs:64,87,112`); libgit2 does its HEAD read in-process and is invisible to a stub. |
| REQ-10 | Document, asset, snapshot and pinned reads go through the module; commands hold no git code | valid-and-open | C6 and C7, plus `commands/archive.rs:184`, which builds a `GitRevisionResolver` in the commands layer and calls `read_pinned_path` once per request (`:90-105`). |
| REQ-13 | Prose about libgit2 and its OpenSSL linkage removed or corrected | valid-and-open | §3 lists the sites. |

Counts: valid-and-open 12 (C1, C2, C3, C5, C6, C7, C9, REQ-01, REQ-06, REQ-07, REQ-10, REQ-13); in flight elsewhere 1 (C8); unverifiable 1 (C4); invalid-as-stated 0.

#### Finding: the HEAD tier is redundant and causes a visible inconsistency

- The shipped data-root `.gitignore` ignores only `/tmp/` (`.jit/.gitignore`), so `index.json` and `issues/*.json` are tracked. Both a primary and a linked checkout therefore have their HEAD tree's `.jit` on disk.
- The HEAD tier adds a record only when it is committed at HEAD and absent from the checkout's files, which means an uncommitted deletion. For an id missing locally in a linked checkout, the lookup misses: the 2 observed `git show` calls both failed, and the record came from the primary-checkout tier.
- Reproduced in a primary checkout (scratch repo under `.../scratchpad/f4/headprobe`): `jit init`, create an issue, commit, then `JIT_ALLOW_DELETION=1 jit issue delete <id>`. After that:
  - `jit issue show <full-uuid>` still returns the issue, read from HEAD by `load_issue` (`json.rs:1196-1203`).
  - `jit issue show <prefix>` returns ISSUE_NOT_FOUND.
  - `jit query all` returns 0 issues.
  - Cause: the local tombstone wins in the aggregated index (`:759-797`), but the full-id fast path in `resolve_issue_id` (`:1258-1265`) goes straight to `load_issue`.
- The only test that needs a HEAD-only record removes the local file by hand: `json.rs:2369`, plus the HEAD-index tests at `:2457`, `:2499`, `:2775` and `:3152`.
- The CLI test `cross_worktree_integration_tests.rs:86` (`test_issue_show_reads_from_git_in_secondary_worktree`) passes without the HEAD tier, because the linked checkout has the file on disk and the primary checkout has it as well.

### 2. Prior-art sweep

- **Charter D-21** (`dev/vision/9db27a3a-charter.md:397-410`): one storage-owned module, git CLI only, "one spawn per needed fact". It rejects a module that hides both backends. This text is correct and stays.
- **Existing CLI git module seed:** `storage/git_revision.rs`.
  - `GitRevisionResolver` holds `git_program` (`:123-146`). Its `run` (`:439-451`) sets `current_dir(repo_root)`.
  - `read_blob_batch` (`:508-607`) is a one-shot `git cat-file --batch`: it writes every oid, closes stdin, then reads. `parse_cat_file_batch` (`:640-690`) parses the responses.
  - `inspect_pinned_targets` combines `ls-tree` with that batch.
  - 842b0ac6 `trim-git-revision-access` (52732879, done) already trimmed it to production queries and made `GitRevisionError: Clone`. 842b0ac6 `remove-linked-worktree-evidence` (f11514dd) is also done.
- **Counting stand-ins already exist twice:** `tests/fast_docs_templates/git_revision_tests.rs:166-181` (`counted_git`: a `#!/bin/sh` wrapper that logs `$*` and runs `exec git "$@"`), and `storage/repository_state_store_tests.rs:2543-2620` (`capture_git_fixture`). Both inject through `with_git_program`.
- **No shared git test helper.** There are local copies:
  - `git()`/`run_git()` in `git_revision_tests.rs:36`, `validate_document_tests.rs:29-66`, `document_history_tests.rs:36-45`, `divergence_recovery_journey_tests.rs:35`, `failure_probe_fixture.rs:592`, `gate_evaluate_all_test.rs:17`, `gate_evaluate_rerun_test.rs:13`, `check_links_tests.rs:74-96`, `commands/hooks.rs:236`, `commands/worktree.rs:670`, `commands/archive.rs:1686`.
  - In total, 23 test files spawn `git` directly.
  - `test_utils.rs:906-1000` fakes a `.git` directory without git.
  - dev/TESTING.md:211-213 puts shared helpers in the library's test-support surface (`test_harness.rs` is `cfg(any(test, feature="test-support"))`, `lib.rs:35-36`).
- **Dependency policy tests:** `tests/scratch_build/dependency_feature_policy_tests.rs` already checks the manifest and runs `cargo tree` to keep TLS and remote-resolver crates out. `scripts/rust-build-budget.sh:226-249` mirrors those checks ("Mirrors ... dependency_feature_policy_tests.rs").
- **Audit:** `findings-tables.md:15` (#11) proposed "HEAD fallback only in linked checkouts" and dropping git2's https/ssh features. `:25` (#21) proposed reading blobs from an open git2 tree. Both are superseded by D-21.
- **User docs on the 3-tier read:**
  - `docs/tutorials/parallel-work-worktrees.md:107-110`
  - `docs/how-to/multi-agent-coordination.md:200-204` (mermaid)
  - `docs/how-to/troubleshooting.md:436` (`git show HEAD:.jit/index.json`)
  - `docs/reference/worktree-validate.md:171-173`
  - `docs/concepts/guarantees.md:232-252` lists the git-requiring document and snapshot operations, which remain true.
- **No user configuration for the git executable:** nothing in `docs/reference/configuration.md` or `runtime_defaults`. The only "configured git" is the test seam in C5.

### 3. Consumer sweep

**`git2::` uses (production):**

| File | Sites |
|---|---|
| `storage/json.rs` | 6 lines in 2 functions (`:829-851`, `:1525`) |
| `commands/document.rs` | 10 lines (`:495, 551, 576, 712, 747, 825, 839, 1128, 1883`) |
| `commands/snapshot.rs` | 6 lines (`:19, 25, 55, 140, 199, 362`) |
| `commands/validate.rs` | 2 lines (`:2153, 2235`), deleted by 84745746 |

**`git2::` uses (tests):** `commands/snapshot.rs:1077,1084`; `tests/fast_docs_templates/document_event_log_tests.rs:259,272`.

**Manifests:** `crates/jit/Cargo.toml:34`, `crates/server/Cargo.toml:61`; `Cargo.lock:929-937` (git2), `:1447-1456` (libgit2-sys), libz-sys `:1470`.

**CLI replacement per use:**

| Use | Replacement |
|---|---|
| HEAD index and issue reads (`json.rs:823-920`) | Delete (Option D, §5), or a lazily started `git cat-file --batch` session (Option S) |
| `read_path_bytes` at a commit (`json.rs:1524-1551`) | Module `read_blob_at(rev, path)`: one `cat-file --batch` request `<rev>:<path>`. For the short hash, use `rev-parse --verify <rev>^{commit}` (exists as `resolve_commit`), or request `<rev>^{commit}` in the same batch session and read the oid from the header. Map `missing` to `PathReadError::NotFound` and an unresolvable rev to `CommitNotFound`. |
| `show_document_content` / `document_diff` / `get_document_diff` / `read_file_from_git` | Same blob read: one batch process carrying both `<from>:<path>` and `<to>:<path>`. Keep the in-Rust `similar` diff so output is byte-identical; `git diff` output would differ. |
| `get_file_history` (revwalk of the whole history, O(history)) | `git log -z --format=%H%x00%an%x00%at%x00%B -- <path>`, one process. Its semantics differ: git's history simplification treats a merge as unchanged when it matches any parent, while the libgit2 code compares the first parent only. Commits are newest first, as today. Keep the 7-character `sha` (`%H` truncated). |
| `check_asset_in_git` (opens HEAD per asset) | One `git cat-file --batch-check` session with `HEAD:<path>` lines, or `ls-tree -z --name-only HEAD -- <paths…>`. One process per `check_document_links` run. |
| snapshot `resolve` (`:58-66`) | `GitRevisionResolver::resolve_commit` (existing rev-parse) |
| snapshot `read_from_git` (per file) | Batch blob reads `<commit>:<path>`; the header gives `blob_sha`. One process per export, versus the current repository open per file at `:199`. Choose this over `git archive` (no per-file oid, reads the whole tree) and over `ls-tree` (gives oids, not bytes). |
| snapshot `origin_remote_url` | `git remote get-url origin` |
| snapshot `current_branch_name` | REQ-05's single branch lookup (other investigator's scope). On a detached HEAD, libgit2's `shorthand()` yields `"HEAD"`; `symbolic-ref --short -q HEAD` yields nothing. Decide which. |
| snapshot `statuses(None)` (dirty) | `git status --porcelain` (includes untracked, like libgit2's default options). This is the dirty-path query that 5f82e09a requires as a module operation (§7). |
| Test fixtures | Use the git CLI through a shared test-support fixture (§5c). |

**HEAD-tier consumers (`json.rs`):**
- `load_aggregated_index` `:753`
- `read_indexed_issues` `:811`
- `load_issue` `:1196-1203`
- `load_issue_or_not_found` `:1229-1240`
- `resolve_issue_id` `:1276`
- Unit tests `:2369`, `:2457`, `:2499`, `:2775`, `:3152`
- CLI tests `cli_repo_workflow/cross_worktree_integration_tests.rs:86`, `:184`
- Rustdoc `json.rs:744-751`, `:866-891`, `:1196`, `:1229-1236`

**Document and snapshot consumers:**
- `main.rs` (6 call sites); `crates/server/src/routes.rs` (15 references to `get_document_history`, `get_document_diff`, `read_path_bytes` and `read_document_content`)
- `storage/mod.rs:332,352` (`read_path_bytes` and `read_path_text` trait methods), `storage/memory.rs:981`
- Tests: `server_integration/document_api_tests.rs` (2), `cli_repo_workflow/document_history_tests.rs`, `cli_query_graph/check_links_tests.rs` (6), `cli_issue/snapshot_export_tests.rs`

None in mcp-server, web or test-vectors reference git2. The web client's `getDocumentDiff` has no production consumer (0f8efc37).

**REQ-13 prose sites:**
- `crates/jit/Cargo.toml:59-70`: the ureq comment says Rustls is "independent of the system OpenSSL that git2 links for its own purposes". Reword it to drop git2.
- `Dockerfile:13-15`: `apt-get install libssl-dev pkg-config`. `libssl-dev` is already dead (no `openssl-sys` in the lock), and `pkg-config` is used only by the `libgit2-sys`/`libz-sys` build scripts, so remove the whole step.
- `dev/vision/document-viewer-implementation-plan.md:597` ("`git2` crate - Already integrated") and `dev/vision/knowledge-management-vision.md:123,130,352,539` describe git2 as the integration. Correct them or mark them historical.
- `dev/active/842b0ac6-*` mentions are plan records; leave them.
- Clean: docs/, INSTALL.md, README.md, CHANGELOG.md, `.github/workflows/*`, and Rust rustdoc. No `deny.toml` exists, and the security audit is `cargo audit -D warnings` (`security-audit.yml:57`).
- Do not touch `dev/archive/**` or `dev/benchmarks/**` (history and raw logs).

### 4. Primitive verification

| Property | Status |
|---|---|
| Spawn by bare name so PATH can inject a stand-in | Holds today for `worktree_paths.rs` and `json.rs:905` (literal `"git"`). Measured: a PATH stub logged all 3 `rev-parse` calls for `query available`, `issue show` and `issue list`. Note that zsh does not word-split, so the probe used `${=c}`. |
| One configured executable | Not yet. `json.rs:905` and `worktree_paths.rs:64,87,112` ignore `git_program`; `GitRevisionResolver::new` (`archive.rs:184`) hard-codes `"git"`. The module must own one program value. |
| Worktree-scoped cwd | `GitRevisionResolver::run` uses `current_dir(repo_root)`; `json.rs:905` uses `layout.worktree_root()`; `worktree_paths.rs` uses `-C <data root probe>`. All HEAD and document sites in scope use the selected layout. |
| Primary checkout reads no git objects | Not yet (C2). Achieved by Option D, or by gating on `WorktreePaths::is_worktree()` (`worktree_paths.rs:161`) from the cached `configured_worktree_paths()`. |
| At most one process for HEAD reads in a linked checkout | Not yet (C5). A persistent `cat-file --batch` (without `--buffer`, git flushes per object) supports request/response interleaving. Requesting `<tree-oid>:<path>` after resolving `HEAD^{tree}` once keeps one consistent tree. |
| Missing executable with no git | Option D removes the read-path dependency entirely. Under Option S, linked checkouts imply git, so a primary or no-git checkout never starts the session. `load_issue` already swallows HEAD errors (`json.rs:1202`), but `load_aggregated_index` propagates them (`:791`). |
| Binary links no libgit2/OpenSSL | No current check. The lock-level property is mechanically checkable (§5d). |

### 5. Design options

#### a. HEAD tier (REQ-06)

- **Option D (recommended): delete the HEAD tier.**
  - Remove `load_index_from_git`, `load_issue_from_git` and their fallback arms.
  - Aggregation becomes local plus the primary checkout.
  - REQ-06 holds with zero git processes in both checkout kinds. The full-id resurrection bug (§1) goes away. One mechanism disappears (D-18), and no session lifetime problem arises in the long-lived server.
  - Cost: the four docs in §2 and the json.rs rustdoc change from 3-tier to 2-tier wording, and the HEAD-only unit tests are deleted.
- **Option S: linked-only lazy batch session.**
  - Storage holds a `HeadTreeReader`, started on the first HEAD read and only when `is_worktree()`.
  - It runs one `git cat-file --batch`: request `HEAD^{tree}` once, then `<tree>:<data-rel>/index.json` and `<tree>:<data-rel>/issues/<id>.json`, and stop the child on `Drop`.
  - CLI: one process per invocation.
  - Server: `JsonFileStorage` lives for the whole process (`crates/server/src/lib.rs:65-68`, `main.rs:76`), so a pinned tree goes stale. Re-resolving `HEAD^{tree}` per top-level read through the same process depends on git re-reading refs in a long-lived `cat-file`. That is unverified; re-reading loose refs is the expected behaviour, but it needs a test.
- **Option G: gate the current mechanism on `is_worktree()` only.** Still N processes in a linked checkout, so it fails REQ-06.

#### b. Counting stand-in (REQ-07)

- **PATH injection (recommended):** the test writes a `git` shell stub into a temp directory. The stub appends `$*` to a log and runs `exec <real git>`, resolved before PATH is overridden; the `which` crate is already a normal dependency (`Cargo.toml:35`). The test spawns `CARGO_BIN_EXE_jit` with `.env("PATH", stub_dir:$PATH)`. This adds no product surface, and the child's environment is not process-global, which satisfies `@/inv/orthogonal-tests`. It requires that the module spawns the bare program name by default.
- **Environment variable `JIT_GIT`:** a new runtime-default surface with its docs and reference-generation burden (`scripts/generate-runtime-defaults-reference.sh`).
- **`git_program` in `config.toml`:** wrong home, because the file is committed repository policy naming a machine-local executable.
- **Placement:** a `cli_repo_workflow` module, for example `git_process_budget_tests.rs` with a `mod` line in `main.rs`. This adds no new target and stays within `MAX_INTEGRATION_TARGETS=12` (`rust-build-budget.sh:42`). Gate it with `#[cfg(unix)]`, as the existing wrappers are.
- **Assertions:** at most one line for `query available`, `issue show <id>` and `issue list` in a primary checkout. Today's binary produces 3 (all layout `rev-parse`), so REQ-07 cannot pass before REQ-04 collapses layout detection to one process.

#### c. Shared git fixtures

Add a test-support `GitRepoFixture` (init, user config, `write`, `commit_all -> oid`) and a `CountingGit` stand-in in the library's test-support surface (`test_utils.rs` or `test_harness.rs`). Replace both git2 fixtures and the two `counted_git` copies with them. Converging the other 21 local `git()` helpers is optional follow-up; leave them out of this epic unless the steward wants it.

#### d. Linkage proof (REQ-01)

- **Recommended:** one owning test in `tests/scratch_build/dependency_feature_policy_tests.rs`. It parses `Cargo.lock` and asserts none of `git2`, `libgit2-sys`, `openssl-sys`, `openssl`, `native-tls`, `libssh2-sys` is present, and it asserts `cargo tree --workspace -e normal,build,dev -i git2` finds nothing.
- Lock absence implies link absence: no `-sys` crate remains that could link those libraries. The check is portable and also covers the musl release and `jit-server`.
- **Optional (Linux only):** the same test reads the dynamic dependencies of `env!("CARGO_BIN_EXE_jit")` (`ldd` or `readelf -d`) and asserts no `libgit2`, `libssl` or `libcrypto`.
- The budget script mirrors the dependency policy today; adding a mirror there is a second owner of the property. Recommend the test only, unless the steward keeps the mirror convention.

### 6. Architecture fit / invariant check

- All replacements live in the storage git module. The commands layer keeps the `similar` diff and output assembly only, which satisfies `@/inv/architecture-dependency-direction` and REQ-10.
- `archive.rs:184` must obtain pinned reads from storage instead of constructing `GitRevisionResolver`.
- `@/inv/convention-convergence`: one blob-read primitive (the batch session) should serve:
  - HEAD reads (if kept)
  - `read_path_bytes`
  - document show and diff
  - snapshot
  - asset checks (batch-check)
  - the existing `read_blob_batch`

  Two batch implementations would be a parallel variant. Today's `read_blob_batch` is write-all-then-read; a single interactive session subsumes it (D-20).
- `@/inv/orthogonal-tests`:
  - The module's behaviour matrix (missing path, non-blob, unresolvable rev, git missing, bytes with NUL and newline) is proven once against the module, extending `git_revision_tests.rs`.
  - Consumers prove only routing.
  - REQ-07 is the CLI wiring proof.
- `@/inv/canonical-cutover`: no compatibility path for the HEAD tier or for git2.
- `@/charter/D-4`: none of these paths runs without git, except `doc show` without `--at`. It currently prefers HEAD when git exists (`document.rs:518-534`), and that is a git-object read by a read command in a primary checkout (§Needs decision).
- `@/charter/D-19`: `get_file_history` walks all of history today. `git log -- <path>` delegates the walk to git, which can use commit-graph Bloom filters.

### 7. Sequencing

The whole 2195c98f epic precedes f4d9502a: the dependency `f4d9502a → 2195c98f` exists. The relevant keys:
- **89e7a273** `stage-storage-issue-ids`: `load_issue(&IssueId)`.
- **0102769f** `issue-record-path-storage`: rewrites `json.rs:730-731,904,940` to use `VirtualPath::issue_record`.
- **e1a6c8c2** `well-known-paths-storage`: `INDEX_FILE` literals in `json.rs`.
- **d50d6976** `stage-document-command-issue-ids`: `document.rs`.
- **855d027d** `issue-record-path-commands`: `document.rs`, `snapshot.rs`.
- **c1c86312** `scoped-issue-id-arguments`: `SnapshotScope::Issue(IssueId)`.
- **945f65f3** `well-known-paths-commands`: `snapshot.rs`.

Steps (each compiles, clippy-clean, tests green):

1. **`remove-head-tier`** (Option D)
   - Files: `storage/json.rs` (functions, fallbacks, 5 unit tests, rustdoc); `cli_repo_workflow/cross_worktree_integration_tests.rs` (rename or retarget `:86`, update the `:184` assertion messages); docs `parallel-work-worktrees.md`, `multi-agent-coordination.md`, `troubleshooting.md`, `worktree-validate.md`.
   - Removes git2 from `json.rs:829-851`.
   - Add a regression test: a locally deleted, committed issue is not found by full id.
   - Needs 89e7a273, 0102769f, e1a6c8c2 landed (code-region conflicts).
   - Independent of the module. Can run first.
   - Under Option S instead, it follows the module-foundation step (other scope) and is larger.
2. **`git-blob-reads`**
   - Module gains the interactive batch-read operation, subsuming `read_blob_batch`, plus a batch-check presence query.
   - Rewires `json.rs:1506-1551` (`read_path_bytes`), document show and diff (both variants), `check_asset_in_git`, and `archive.rs:184`.
   - Files: storage git module, `storage/json.rs`, `storage/mod.rs`, `storage/memory.rs` (if the trait grows), `commands/document.rs`, `commands/archive.rs`, `git_revision_tests.rs`.
   - Needs: the module-foundation step (REQ-02, other investigator), d50d6976, 855d027d.
3. **`git-document-history`**
   - Module `file_history(path)` via `git log`; rewires `document_history` and `get_document_history`; deletes `get_file_history`.
   - Files: the module, `commands/document.rs`, `cli_repo_workflow/document_history_tests.rs`, `server_integration/document_api_tests.rs` (assertions only if semantics change).
   - Needs step 2 (shared module file).
4. **`git-snapshot-export`**
   - snapshot.rs through the module: `resolve_commit`, batch reads with `blob_sha`, `remote get-url`, the REQ-05 branch lookup, and a `status --porcelain` dirty query. Replace the git2 unit test (`snapshot.rs:1075-1100`) with the CLI fixture or delete it (the module owns the property).
   - Files: `commands/snapshot.rs`, the module, `cli_issue/snapshot_export_tests.rs`.
   - Needs step 2, the branch-lookup step (REQ-05, other scope), c1c86312, 855d027d, 945f65f3.
5. **`drop-libgit2`**
   - Remove git2 from both manifests and the lock. Convert `document_event_log_tests.rs:259-283` to the shared CLI fixture.
   - Add the §5d lock/linkage test.
   - REQ-13 prose: `crates/jit/Cargo.toml:59-70`, `Dockerfile:13-15`, dev/vision docs.
   - Needs steps 1–4 and 842b0ac6 **84745746** (`remove-silent-integrity-validator`).
6. **`git-process-budget-test`** (REQ-07)
   - A shared `CountingGit` test-support fixture (replacing `git_revision_tests.rs:166` and `repository_state_store_tests.rs:2543` copies) plus the `cli_repo_workflow` test.
   - Needs step 1 and the layout-once step (REQ-04, other scope).

#### Cross-epic seams (all flagged for the steward)

- **0f8efc37** (epic frontends-share-core, unplanned): its criterion "Document history and diff reach git through the storage-owned git module" and its CLI/server document-read convergence overlap steps 2–3.
  - Recommended: steps 2–3 route both existing variants through the module without merging them; 0f8efc37's document-convergence task follows step 3.
  - This edge delays 0f8efc37.
- **5f82e09a** (lead-friction, "Refuse a profile apply over uncommitted target changes"): REQ-06 requires the dirty-path query to be a module operation, the same query step 4 needs.
  - Either 5f82e09a follows step 4 or the module foundation (delays 5f82e09a), or step 4 follows 5f82e09a and reuses its operation (delays f4d9502a).
  - Recommend whichever lands first owns the operation, with no hard edge.
- **182f4c03** (snapshot and migrate to library handlers) and **a015dcbb** (archive and snapshot facade), epic rust-code-quality: textual overlap in `snapshot.rs` and `archive.rs`. No semantic edge; serialise only if dispatched concurrently.
- **22189592** (classify document path-read failures): touches `PathReadError` mapping in `read_path_bytes`. Step 2 must keep the `CommitNotFound`/`NotFound` variants. No edge needed if step 2 preserves the variants.

