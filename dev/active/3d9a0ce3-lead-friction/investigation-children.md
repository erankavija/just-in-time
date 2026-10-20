# 3d9a0ce3 investigation: gate, document and CLI-output children

Scope: epic criteria REQ-01, REQ-02, REQ-03 and REQ-05, plus children 72d89161, fd9bfab8, 8673dc5e, 66986f88, f37d228e, 3754095f and da12ad66. dcbfc47a (rejected) and the profile work (REQ-04, REQ-06..13, e643ef66) are out of scope here; `investigation-profile.md` covers them.

Baseline: HEAD `1ed0a049c` (2026-10-20). Paths are relative to the repository root. Line numbers are as of that HEAD.

Since the children were filed on 2026-10-15, `git log --since=2026-10-15` over every gate, document, output, CLI, server, web, MCP and `contrib/gates` file returns only one commit: `457c96ed2` (jit:6ff14f21). It only replaces test-mode env handling in `gate.rs`, `gate_cli_tests.rs`, `validate.rs` and `config.rs`. **No claim below has been fixed since filing.** `.jit/gates.toml`, `repository_inputs.rs` and `ai-review.sh` were last changed on or before 2026-09-21.

Classification:
- **valid** means the fact holds, or the criterion is open.
- **done** means the criterion is already satisfied today.
- **invalid** means invalid as stated.

## 1. Claim classification

### 72d89161: reuse gate verdicts only by declared-input digest

| Claim | Class | Evidence |
|---|---|---|
| A commit-based skip runs when the latest run passed at `HEAD` | valid | `commands/gate.rs:838-856` (`pass_gate`), helper `gate_passed_at_head` `gate.rs:985-1010`, which compares `run.commit` to `get_git_commit` (`gate.rs:1007`). The skip applies only per issue and only when `gates_status` is already Passed. `check_gate`/postchecks never skip (`gate_check.rs:841-845`, `1588+`). |
| Runs record `commit`, `branch`, `tree_dirty`; `tree_dirty` comes from `git status --porcelain` over the worktree | valid | `gate_execution.rs:101,164-166`; `get_git_tree_dirty` `gate_execution.rs:654-663` (`--porcelain --untracked-files=normal`, data root not excluded). The pairing machinery `probe_stable_git_pair` is at `gate_execution.rs:543-603`. |
| Tracker writes make the next run `tree_dirty: true`; committing them moves `HEAD` and re-runs the checker | valid | Each evaluation writes the issue, a gate-run result and an event (`gate_check.rs:988-999`). The skip keys on commit equality (`gate.rs:1007`). |
| Digest reuse works across issues over the same digest | done | `find_reusable_gate_run(gate_key, digest)` filters by gate and digest only: `storage/json.rs:1481-1489`, `storage/memory.rs:938-946`; `GateRunResult::is_reusable_for` `domain/types.rs:1022-1027`. Owning test: `gate_check.rs:2486` `test_check_gate_executes_once_across_a_wave_over_one_unchanged_tree`. |
| `.` is rejected as a root | valid | `DeclaredRoot::try_from` `domain/repository_inputs.rs:80-92` → `is_safe_relative_path` `:454-469` (`.` is `CurDir`; `has_ordinary_segments` `:473-477`). The predicate is shared with profile package paths (`:451-453`), so acceptance belongs in `DeclaredRoot`, not the predicate. `DeclaredRoot::covers` (`:59-63`) needs a `.` case; the walker already handles an empty prefix (`gate_execution.rs:483`). |
| A new top-level directory silently escapes the digest | valid, with a live instance | `.config/nextest.toml` is tracked (added in `82f5599de`) and read by `scripts/cargo-ci.sh:134-136`, but it is in none of the five 26-root lists (§2a). The tracked empty `.index.lock` is uncovered too. |
| A nested data root is digested whenever a root covers it | valid | The walker (`gate_execution.rs:387-497`) knows only `.git` (`is_git_plumbing` `:499-507`). `declared_inputs_digest` passes only the worktree root (`gate_check.rs:1022-1033`, `real_repo_root` `:697-704`). The layout knows the nesting (`RepositoryLayout.nested_data_relative`, `repository_state/path.rs:443`, computed at `:468-473`) but has no public accessor. |
| REQ-01: no skip, `already_passed` removed | valid | Field definitions: `gate.rs:66-95`, typed `GateEvaluateManyResult.already_passed` `output.rs:2636-2638` (published via `schema.rs:610-612`). Writers and renderers: `main.rs:5095-5127,5160,5183-5187,5219-5234,5268`. Tests: `tests/cli_gate/gate_evaluate_skip_at_head_test.rs` (393 lines; its manual-redefine test at `:224-318` owns a different property, so keep it) and `gate_evaluate_all_test.rs:160,261-296`. No consumers in `mcp-server/`, `web/` or `crates/server`, except through the generated schema. |
| REQ-02: no dirty-tree field | valid | Readers and writers: `domain/types.rs:956-964`; `gate_execution.rs` (above); `output.rs:2494-2500,2559` (`GateRunSummary`, exposed by `crates/server/src/routes.rs:892-916`); `main.rs:1662-1664` ("Tree: dirty/clean"); `storage/reference.rs:79-80,121,148,202-212,331,379,690`; `storage/gate_runs.rs:73,158-192,241,303`; `web/src/types/models.ts:37`; generated `docs/reference/storage-records.md:110,129`. There are 2135 existing `.jit/gate-runs/*/result.json` carrying the key. `GateRunResult` has no `deny_unknown_fields`, so those still parse. Unverified: whether any byte-level check re-serializes old records. |
| REQ-03: `.` accepted as a root | valid | Above. |
| REQ-04: the digest never includes a data root nested in the worktree | **invalid** (overbroad) | The data root holds both tracker-written records and the repository's own configuration (`.jit/config.toml`, `gates.toml`, `rules.toml`, `invariants.toml`, `templates.toml`, `schemas/`, `reference/`; `git ls-files .jit`). This repository's tree-wide gates include `.jit` on purpose and exclude only the records (`.jit/gates.toml:40-51` comment, excludes `:82-96`). Their checkers read that configuration: docs-mechanical resolves `@/` items through the `.jit` registries (`gates.toml:302-309`); cargo-ci compares projections. Excluding the whole data root would let a configuration edit reuse a stale verdict. `secret-detection` digests `.jit/issues` deliberately (`gates.toml:562-566`, excludes `:596-606`); as a result it records no `inputs_digest` reuse at all. The property the epic needs is narrower: the files jit writes during a mutation leave every digest unchanged. Those are the record, event, gate-run, index, lock, scratch and worktree-state paths, plus the worktree-root `.jit-bootstrap.lock` (`repository_state/path.rs:16`, `storage/json.rs:419-422`). |
| REQ-05: `--force` executes unconditionally; help describes only digest reuse | valid | `force` → `VerdictSource::Execute` at `gate.rs:863-867`. Help mentions HEAD at `cli.rs:1580-1586,1605-1608,1630-1632,1643-1646,1677-1679`. |
| REQ-06: one reuse mechanism in docs; net code removed | valid | Skip docs: `docs/reference/cli-commands.md:2118,2131,2137,2143-2156,2201,2220,2225-2227,2242,2248-2256,2276-2278,2292`. Digest docs: `cli-commands.md:2158-2170`, `docs/how-to/custom-gates.md:125-243`. Note that `custom-gates.md:224` already states the target behavior, which the code contradicts today. Net removal is likely: about 900 lines, most of them tests (§3). |
| Note: supersedes ea2bb1f6 | done | ea2bb1f6 is rejected with `resolution:superseded-by-digest-reuse`. |
| Note: a git-state checker is outside the digest | valid | The digest covers working-tree bytes plus `verdict_declaration` (`declarations/gates.rs:212-218`); there is no HEAD or index. |

### fd9bfab8: record a checker infrastructure fault as an error

| Claim | Class | Evidence |
|---|---|---|
| Run status already distinguishes `error` (timeout, missing command) from `failed` | done | `GateRunStatus` `domain/types.rs:1098-1112`; mapping `gate_execution.rs:148-155`. |
| "Every nonzero exit becomes `failed`" | **invalid** | 126 and 127 already map to `Error` (`gate_execution.rs:150-152`), and a signal or timeout (`None`) maps to `Error` (`:154`). |
| `ai-review.sh` exits 1 on no output or an unparsable verdict | valid | No output: `contrib/gates/ai-review.sh:142-147`. Unparsable: under `set -euo pipefail` (`:2`), the `grep` in `VERDICT=$(…)` at `:156` fails with no match, so the script exits 1 there; the `else` branch at `:167-172` is unreachable for that case. The reviewer invocation is wrapped in `|| true` (`:103`), so a missing reviewer tool surfaces only as "no output". Setup failures (`:40-60`) also exit 1. |
| Exit 75 is `EX_TEMPFAIL` and is used by downstream wrappers | cannot verify | External convention. Nothing in `crates/` or `contrib/` handles 75 today. |
| An evaluation terminated before recording leaves no run | **invalid** (ambiguous) | Killing the **jit process** records nothing: the run is published only in the final mutation (`gate_check.rs:944-1006`), and a spawn failure is an `Err` with no run (`gate_execution.rs:244`). Killing the **checker** child records an `Error` run (`gate_execution.rs:154`). The statement and its documentation must name which process is terminated. |
| REQ-01: exit 75 → `error`; runner-error exit code; JSON verdict `error` | valid | Add a 75 arm at `gate_execution.rs:148-155`. The runner error is `ErrorCode::IoError` → `ExitCode::ExternalError` = 10 (`main.rs:70-75`, `output.rs:576-577`), with JSON `"verdict": "error"` (`main.rs:694-697`). There is no gate-specific constant. |
| REQ-02: "every other nonzero exit is recorded as `failed`, unchanged" | **invalid** | Read literally, this turns 126, 127 and signal kills into `failed`. It should say: every other exit keeps its current status. |
| REQ-03: `ai-review.sh` exits 75 / 1 | valid | Needs an explicit tool check, removal of the `|| true` swallow or a classification of it, and the `pipefail` fix at `:156`. Header contract at `:9-11`. |
| REQ-04: docs state exit 75 and the no-run case | valid | `docs/how-to/custom-gates.md:318-337` says "Non-zero – Gate failed", which is already inaccurate for 126 and 127. Exit codes: `docs/reference/exit-codes.md:44-45`, `cli.rs:1568-1578`. |

### 8673dc5e: gate status shows a manual attestation as never run

| Claim | Class | Evidence |
|---|---|---|
| The latest-run view says "has not been run yet" after attestation | valid | Attestation publishes `CapturedIssueMutation::SetManualGateStatus` (`gate.rs:902-907`), which writes only `issue.gates_status` and a `gate_passed` event (`commands/mod.rs:1603-1651`). There is no `RecordGateRun`. The latest-run view reads `.jit/gate-runs` through `get_last_gate_run` (`gate_check.rs:1247-1257`). The message is printed at `main.rs:4883` and duplicated at `:4693,4804,4991`. |
| The readiness view counts it as passed | done (fact) | `status-all` reads `gates_status` via `get_required_gate_statuses_for_issue` (`gate_check.rs:1429-1459`); rendered at `main.rs:4907-5007`. |
| Reproduction output | valid | The verbatim string is at `main.rs:4883`. |
| REQ-01 / REQ-02 / REQ-03 | valid | The history view `list_gate_runs` (`gate.rs:1447-1461`) is also run-only (`main.rs:4755-4757`). The handlers span `main.rs:4611-5007` (about 400 lines). |

### 66986f88: derive an issue's documents from its artifact directory

| Claim | Class | Evidence |
|---|---|---|
| A durable artifact must be linked to be discoverable | valid | Server content, raw, history and diff routes require a link (`commands/document.rs:567-761`); server search is limited to linked paths (`routes.rs:285`, `document.rs:767-781`). Archive roots are the explicit links (`domain/artifact_inventory.rs:284-290`; `discover_embedded_owners` `domain/artifact_discovery.rs:501-520`). |
| …and to pass artifact-layout review | **invalid** for `jit doc conformance` | Conformance attributes a path component by its short-id prefix, with no link needed (`domain/artifact_conformance.rs:207-243`). Every canonical directory starts with the short id (`domain/artifact_directory.rs:62-108`). Links matter only for prefix-less names (`:220`, `:263-277`). If "review" means the AI review gates, `contrib/gates/plan-review-prompt.md:8` fails an "unlinked" plan or manifest; that is a prompt, not code. |
| Workers in linked worktrees cannot write tracker state | valid for the shipped default | The default is `refuse` (`config.rs:3338`). This repository sets `[worktree] write_policy = "allow"` (`.jit/config.toml:281-284`): a worker writes its own local `.jit/` store, which merges with the branch. |
| The layout already resolves the directory an issue owns | done, with two caveats | Pure, config-driven resolver (`domain/artifact_directory.rs:62-108`; `[documentation].issue_scoped_areas`, `config.rs:333-335,403-409`). (a) An issue owns one directory **per declared area**; this repository declares four (`.jit/config.toml:43`). (b) The slug comes from the membership label (`artifact_directory.rs` module doc, lines 11-21), so relabeling a container renames its derived directory. |
| REQ-01: listing includes files under the directory | valid | `doc list` returns `issue.documents` verbatim (`document.rs:112-124`). A git-free recursive walk exists: `inspect_artifact_evidence(…, RecursiveFiles)` (`storage/artifact_planning.rs:246-436,358-360`). |
| REQ-02: link check, validation and conformance treat them as tracked | valid; conformance mostly done | Link check: the tracked set is `all_document_paths`, built from in-scope issues' links (`document.rs:1070-1077,1096`); the untracked warning is at `document/link_validator.rs:153-160`. Validation has no "untracked file" concept. The document-dependent parts are reference resolution (`validation/repository.rs:451-466`) and the plan lookup by the hard-coded label `"plan"` (`commands/validate.rs:2906-2921`; callers `validate.rs:1295,1370`, `validation/repository.rs:752`). A Done planning node with a missing plan is a hard error (`validate.rs:1378`). |
| REQ-03: explicit links required only outside; inside links removed from records | valid | In this repository, 11 of 542 links (in 300 of the 1185 issues) point inside the owner's own canonical directory; 1 points inside another issue's (201634a8 → `dev/active/842b0ac6-greenfield-cutover/investigation.md`); 530 are outside, 460 of them in `dev/archive/`. Removing the 11 is a one-time repository data edit. |
| REQ-04: linked after merge with no tracker write | valid, feasible | After merge the files sit in main's working tree, so a walk lists them (`RepositoryLayout::worktree_root`, `path.rs:488`). Caveat: `doc show` reads git `HEAD` for unpinned documents when a repository exists (`document.rs:415-438`), so an uncommitted derived file fails `doc show` until it is committed. |
| Note: open design point on `doc_type` and `label` | valid | See §2c. |

### f37d228e: link check exits 0 on warnings

| Claim | Class | Evidence |
|---|---|---|
| Exit statuses are 0, 1 and 2 (2 = warnings only) | done (fact) | `document.rs:1290-1297`; the process exits at `main.rs:6306`. |
| Example warning: a link to an existing file that is not a tracked document | valid | `link_validator.rs:153-160`. The other warning kinds are `risky_asset_path`, `url_validation_failed`, `risky_link` and `scan_error` (`document.rs:1152,1208,1255,1269`). |
| REQ-01 / REQ-02 / REQ-03 | valid | JSON `LinkCheckResult` (`commands/mod.rs:1814-1832`) already carries `warnings[]` and `summary.warnings`. The docs are already partly wrong: `docs/reference/exit-codes.md:54` says code 2 means "only risky-link warnings"; see also `cli-commands.md:3035-3055`. |

### 3754095f: emoji status markers to words

| Claim | Class | Evidence |
|---|---|---|
| Markers appear in validation, gate, document and initialization output | **invalid** in part | Validation and document output: yes. Gate output: only `ℹ` on `gate add` (`main.rs:5044`) and `⚠️` on `gate preset apply` (`:5540`); evaluate and status use words. Initialization: **none**. The `Commands::Init` arm `main.rs:2543-2621` prints through `print_success` (`output.rs:348-354`), and `commands/init.rs` has no marker. |
| Type icons are a separate repository-declared feature | valid | `type_icons.rs:25-33,63,93`; config `config.rs:214-223`; resolved at `config_manager.rs:217`. **No CLI output uses type icons.** The only consumer is the web UI, via `crates/server/src/routes.rs:64,952-986`. |
| REQ-01 | valid | Inventory in §2d. |
| REQ-02 | done today | No CLI path renders icons, so this criterion only constrains the sweep. |
| REQ-03 | valid | No test scans output or literals for markers. The existing policy-test home is `crates/jit/tests/scratch_build/` (see `process_state_policy_tests.rs` from `457c96ed2`). |
| REQ-04 | valid | Docs inventory in §2d. |

### da12ad66: one rejection command form

| Claim | Class | Evidence |
|---|---|---|
| A dedicated reject command records its reason as a `resolution:` label | valid | `cli.rs:1167-1178`; `main.rs:3943-3978`. It is two publications: `update_issue_state` at `:3951`, then `add_label` at `:3954-3958`. |
| A state update to `rejected` accepts no reason | valid | `IssueCommands::Update` has no `--reason` (`cli.rs:975-1072`), although `--label resolution:x` can ride in the same call. |
| Undocumented divergence | (new fact) | `reject` skips local-rule enforcement (`commands/issue.rs:634-664`) and appends the label unchecked (`commands/labels.rs:20-36`, `AppendUnchecked`). `update --state rejected` enforces local rules and label validation (`commands/mod.rs` `derive_field_update` → `derive_write_validation`). |
| REQ-01 / REQ-02 | valid | Other surfaces: MCP `mcp-server/curated-tools.json:18` says `issue update` is the single transition entry point, while `:28` curates `jit_issue_reject` in, which is inconsistent. Docs: `docs/reference/cli-commands.md:1651-1714`, `docs/concepts/core-model.md:669,816,836,853`, `docs/reference/glossary.md:32`. Skill bug: `.agents/skills/jit-execution-lead/SKILL.md:225` uses `update --state rejected --reason`, which does not exist. Tests: `tests/cli_repo_workflow/workflow_tests.rs:764-860` and about 15 `--state rejected` sites. The server and web UI have no reject path. |

## 2. Do the children deliver the epic criteria?

### (a) REQ-01: each checker executes once over unchanged inputs, whatever commits the tracker makes

72d89161 removes the commit-based path, which is necessary. Remaining gaps:

1. **What may enter the digest.** As written, 72d89161 REQ-04 excludes the whole data root. That breaks soundness for checkers that read `.jit` configuration (§1, REQ-04 row). The sound property is "paths jit writes during a mutation never enter a digest". That set is derivable from the storage layout constants (`repository_state/path.rs`, e.g. `RECORD_SCRATCH_DIR` `:12`, `BOOTSTRAP_LOCK_FILE` `:16`) rather than from per-repository exclusions. *Decision needed.*
2. **Records written during evaluation.** These are `issues/<id>.json`, `gate-runs/<run>/result.json` and `events.jsonl` (`gate_check.rs:988-999`); `index.json` is unverified. For the tree-wide gates they are excluded only by hand (`.jit/gates.toml:82-96` and three copies). No HEAD component exists in the digest (`gates.rs:212-218`), so commits alone never break reuse.
3. **`secret-detection` can never reuse.** Its digest covers the records on purpose (`gates.toml:562-566,596-606`). Once a mechanism-level exclusion of tracker-written paths lands, its records stop being in the digest. Either the gate accepts that (a secret in an issue record is then caught only on the next content change), or it declares no inputs. *Repository-local decision.*
4. **Gates without inputs** run every time by design (`custom-gates.md:224`). In this repository these are the five `ai-review.sh` gates, `coverage-preview` (`rule_validation`), `dependency-audit` (network), `jit-validate` and `repo-validate`. Their verdicts are functions of tracker records or issue context, so "unchanged checker inputs" never holds across tracker writes for them. REQ-01 is met for them vacuously.
5. **This repository's root lists.** Five gates hand-keep the same 26-root list:

   | Gate | Roots (lines) | Excludes (lines) |
   |---|---|---|
   | cargo-ci | `.jit/gates.toml:54-81` | `:82-96` |
   | cargo-ci-features | `:122-149` | `:150-164` |
   | docs-mechanical | `:311-338` | `:339-353` |
   | secret-detection | `:568-595` | `:596-606` |
   | tests | `:657-684` | `:685-699` |

   That is 130 root lines and about 60 exclude lines. With `.` (72d89161 REQ-03) and mechanism-level tracker-path exclusion (refined REQ-04), each list collapses to `roots = ["."]` plus generated-tree excludes. The repository then also closes the `.config/nextest.toml` gap.

   Two items stay explicit excludes under `.`: the worktree-root `target/` and the 9.9 GB `.agents/cache-pool` (ignored by git `.gitignore:2`, last written 2026-09-21). `.agents/cache-pool` is **not excluded today** under the `.agents` root, so every tree-wide evaluation walks it. Its runtime cost is unverified. `.claude` and `.codex` are also untracked top-level entries. This is a **repository-local follow-up task** (dogfood configuration, not product), and it depends on 72d89161.

### (b) REQ-02: every gate-run record distinguishes a fault from a verdict

fd9bfab8 covers exec checkers only. Built-in checkers are uncovered (kinds: `declarations/gates.rs:108-147`; dispatch: `gate_check.rs:1052-1242`). Their status is only ever Passed or Failed (`:1170-1174`, synthesized exit 0 or 4 at `:1217-1221`):

- `RepositoryValidation`: a structural or integrity fault is folded into a blocking "repository-integrity" finding and recorded as `Failed` (`gate_check.rs:646-672`). **Fault recorded as a verdict.**
- `LabelTargetValidation`: a misconfigured namespace or count becomes a blocking finding, recorded as `Failed` (`gate_check.rs:1108-1132`). Arguably a configuration fault.
- `RuleValidation`: a missing or misconfigured rule becomes an Error-severity finding, recorded as `Failed` (`commands/validate.rs:1413-1430`).
- `IssueValidation`, and config or declaration load failures in every kind: these propagate as `Err`, so **no run is recorded** (`gate_check.rs:468-469,476-477,535-536,558-559`; `validate.rs:1405-1411`). The CLI reports exit 6 (`ErrorCode::GateError` → `AlreadyExists`, `main.rs:752-753`, `output.rs:906`) with `--json`, or exit 1 without it. It never reports the runner-error code 10.
- `ReviewPlaceholder`: always `Passed`.

Two more points:
- Manual attestations create no run record (§1, 8673dc5e), so "every gate-run record" is satisfied for them only vacuously.
- `GateRunStatus::Pending` and `Skipped` are never produced. They appear only in the `--status` filter parser (`main.rs:1446-1447`), which is relevant to the semantic-types epic 2195c98f REQ-11.

**Gap:** a new task is needed for built-in checker fault classification, or fd9bfab8's scope must grow.

### (c) REQ-03: worktree artifacts linked after merge with no tracker write

66986f88 delivers listing and link-check acceptance. Uncovered:

- Archive roots are explicit links only (`artifact_inventory.rs:284-290`, `artifact_discovery.rs:314-330,501-520`). Archive rewrites `document.path` (`repository_state/archive.rs:175-190`), so derived files must also be archive roots, or archiving leaves them behind. Unverified: whether the classifier otherwise sweeps unlinked files under a short-id directory.
- Server routes and search require links (`document.rs:567-761,767-781`).
- `doc show` reads git `HEAD` (`document.rs:415-438`).
- The plan-document lookup depends on `label == "plan"` (`validate.rs:2906`), and so do the plan-review and breakdown-review prompts.

**Current explicit-link record:** `DocumentReference` (`domain/types.rs:829-844`) has `path`, `commit: Option`, `label: Option`, `doc_type: Option`, `format: Option` (serde default) and `assets: Vec` (skipped when empty). It is stored in `Issue.documents` (`types.rs:435`).

**Consumers of `doc_type`:**
- Rule selector `has_doc_type`: `declarations/rules.rs:237-238,289-292,326-333`.
- Rule assertion `require-doc-type`: `rules.rs:559-561,1109-1111,1371-1373`, desugared at `validation/desugar.rs:57,237-246` over projection `doc_types` (`domain/projection.rs:128-132,157`). Rule serialization: `schema.rs:209-210,244-246`. Neither rule kind is used in `.jit/*.toml` or `profiles/`.
- Gate env `JIT_ISSUE_DOCS`: `gate_execution.rs:43-75`, fed from `gate_check.rs:1349`.
- CLI display: `main.rs:2118,5861,5897`; `DocumentContentResult` `document.rs:441-446`.
- Web UI: `web/src/components/Issue/IssueDetail.tsx:650-722`.

**Consumers of `label`:**
- The `PLAN_DOC_LABEL` lookup (above).
- `JIT_ISSUE_DOCS`.
- CLI and web display.

Other surfaces:
- Archive uses neither field.
- Conformance uses paths only (`artifact_conformance.rs:263-277`).
- The server passes the record through.
- MCP tools are generated from the schema; `doc add/list/show/dir/conformance/remove` are curated in (`mcp-server/curated-tools.json:45-50`).

**Options for derived documents:**
1. A derived document has no type or label. An explicit link remains the way to attach metadata to any path, inside or outside the directory. REQ-03's "remove inside links" then applies only to links carrying no metadata. The plan lookup keeps working through a labeled link. Smallest change.
2. Derive the plan location from the template's `plan_doc_location` (`templates.rs`, cited at `validate.rs:2902-2905`) instead of a label, then drop `label` entirely. Removes the hard-coded `"plan"` literal, which is a domain-agnostic smell. Larger change.
3. Drop both `doc_type` and `label` from the record. This removes the `has_doc_type` / `require-doc-type` rule kinds (unused here, but shipped) and needs option 2 for plans. Largest deletion; consistent with D-18.

Recommendation: option 1 in 66986f88, and file option 2 or 3 as a separate decision.

**Link counts in this repository:** 542 explicit links across 300 issues. 11 point inside the owner's own canonical directory (10 bare `dev/active/<sid>/`, 1 `dev/active/9db27a3a-v1-0/…`). 1 points inside another issue's directory. 530 are outside: 460 in `dev/archive/`, 25 in `.agents/skills`, 30 in `docs/`, and 15 elsewhere. 4 links are pinned. `doc_type` values: design 189, notes 71, unset 69, research 31, documentation 31, reference 28.

### (d) REQ-05: words, not emoji

3754095f covers this, with these corrections.

**Inventory of status-marker string literals (non-test):**
- `crates/jit/src/main.rs`: 59 sites. By symbol: ✓ 21, ✗ 6, ❌ 11, ⚠ 8, ⚠️ 6, ✅ 1, ℹ 5, 🌐 1.
- `commands/validate.rs`: 5 (✓, at `:84,87,89,607,1544`; `validate --fix`).
- `crates/server/src`: 0.

**By command:**

| Command | `main.rs` lines |
|---|---|
| profile validate | 1225-1235 (deleted by D-8) |
| invariant check | 1911-1916 |
| issue create | 3038, 3080 |
| issue update (single and bulk) | 3659-3752 |
| apply | 4078 |
| dep add / rm | 4121-4194 |
| gate add | 5044 |
| gate preset apply | 5540 |
| doc assets | 6176-6218 |
| doc check-links | 6268-6295 |
| config validate | 7044-7047 |
| hooks install | 7104 |
| validate (four copies of the `❌`/`⚠` block at 7366, 7414, 7526, 7787) | 7361-7800 |
| claim | 8205-8535 |
| snapshot export | 8770 |

**Borderline glyphs:**
- `○`/`✓` from `MinimalIssue::state_symbol` (`domain/types.rs:744-751`, in `issue show` dependencies at `main.rs:2085-2091`). Redundant there: the state word follows on the same line.
- Its twin `DependencyTreeNode::state_symbol` (`output.rs:1605-1613`, used by `graph deps` at `main.rs:1690-1725`). There it is the **only** state information, so it needs a word.

Structural `├─ └─ │ • →` are not status markers.

**Tests that pin markers:**
- `tests/cli_repo_workflow/project_config_tests.rs:222` asserts `"✓ Configuration is valid\n"`.
- `label_hierarchy_e2e_test.rs:298` checks loosely.
- `output.rs:2924,2930`.

**Docs with output examples containing markers:**
- `docs/how-to/dependency-management.md` (`:134-141,216-220,420-426`)
- `docs/reference/worktree-validate.md:378-390`
- `docs/how-to/troubleshooting.md:139,313,344`
- `docs/reference/claim.md:363`
- `docs/tutorials/parallel-work-worktrees.md:127`
- `docs/concepts/core-model.md:638-645` (this one shows output the code never prints)
- `docs/concepts/design-philosophy.md:344-346` (aspirational)
- `docs/concepts/guarantees.md:541`
- `docs/how-to/custom-gates.md:53`
- `dev/authoring-conventions.md:323,342,364`

Prose do/don't lists (e.g. `docs/reference/labels.md`) are not output. The type-icon example `docs/reference/example-config.toml:60` stays. Skills contain no markers in SKILL.md or reference files.

## 3. Sizing against one worker, one focused cycle

| Child | Footprint | Test boundary | Verdict |
|---|---|---|---|
| 72d89161 | Deletion: `gate.rs` ~70, `gate_execution.rs` ~100 production + ~178 tests, `main.rs` ~45, `output.rs` ~13, `types.rs` 9, `storage/reference.rs` ~17, `storage/gate_runs.rs` ~38, `cli.rs` ~10, tests ~430, `cli-commands.md` ~30, `models.ts` 1. Addition: `.` root (`repository_inputs.rs`) and tracker-path exclusion (`gate_execution.rs` walker + layout accessor). | Unit (walker, `DeclaredRoot`), harness (`gate_check.rs` reuse fixture `:2178-2300`), CLI (evaluate output) | **Split in two.** **72d89161-A** "Remove the commit-based skip and the dirty-tree field": REQ-01, 02, 05, 06; pure deletion, about 900 lines net removed. **72d89161-B** "Accept `.` as an input root; tracker-written paths never enter a digest": REQ-03 plus a reworded REQ-04, with its own docs line in `custom-gates.md:187-191`. They share only `gate_execution.rs`, in disjoint functions, so they can run in parallel or in sequence. |
| fd9bfab8 | `gate_execution.rs:148-155` (one arm), `ai-review.sh` (~20 lines), `custom-gates.md`, `exit-codes.md` | Unit (status mapping), script test (unverified whether `ai-review.sh` has one) | One cycle, after rewording REQ-02 and the background. |
| (new) built-in checker faults | `gate_check.rs:646-672,1052-1242`, `validate.rs:1405-1430`, error mapping in `main.rs:752-753` | Harness | New task for epic REQ-02 (§2b). |
| 8673dc5e | Either record attestations as gate-run records (`commands/mod.rs:1603-1651` adds `RecordGateRun`; the record has no command or exit code) or project `gates_status` into the views (`main.rs:4611-5007`). | Harness and CLI | One cycle. Recommend recording attestation as a run: one source for all three views, and it makes "every gate-run record" cover manual verdicts. *Decision.* |
| 66986f88 | Listing (`document.rs:112-124` + walker), link-check tracked set (`document.rs:1070-1096`), server routes (`document.rs:567-781`), archive roots (`artifact_inventory.rs:284-290`, `artifact_discovery.rs`), plan lookup (`validate.rs:2906`), record cleanup (11 links), docs | Unit, harness, CLI, server | **Oversized; split in three.** **66986f88-A** "Derive documents from artifact directories": one domain function returning an issue's derived paths across all declared areas; `doc list`, link check and server reads consume it (REQ-01, REQ-02, REQ-04). **66986f88-B** "Archive derived documents with their issue": a new criterion; depends on A. **66986f88-C** "Drop redundant explicit links inside artifact directories": REQ-03 as a repository-local data edit of the 11 links plus a docs statement; depends on A and the type/label decision. The `doc_type`/`label`/plan-label decision (§2c) is resolved in planning; options 2 or 3 become their own task if chosen. |
| f37d228e | `document.rs:1290-1297` (3 lines), `exit-codes.md:24,53-54`, `cli-commands.md:3035-3055`, check-links tests | CLI | Small; one cycle. |
| 3754095f | About 64 literal sites in two source files, 1-2 state-glyph functions, 3 tests, about 10 docs files, a new literal-scan test | Unit (renderer), policy scan test | One cycle if the profile-validate sites (`main.rs:1183-1243`) are left to D-8. Optional split: code plus test, then docs. Not required. |
| da12ad66 | `cli.rs`, `main.rs:3497-3978`, `commands/issue.rs:634-664`, `labels.rs`, `curated-tools.json`, docs, skill, about 15 tests | CLI | One cycle, but blocked by a decision and by collisions (§4). |

## 4. Coupling and ordering

**Shared files inside the epic:**
- **Gate-run record schema** (`domain/types.rs:936+`, `storage/reference.rs`, `output.rs` `GateRunSummary`, generated `storage-records.md`): touched by 72d89161-A (removes `tree_dirty`), 8673dc5e if attestations become runs, and the new built-in-fault task (status only).
- **Gate handlers in `main.rs`:** status `4611-5007` (8673dc5e); evaluate, evaluate-all and evaluate-many `5078-5300` (72d89161-A). These are disjoint arms.
- **`gate_execution.rs`:** 72d89161-A (git context), 72d89161-B (walker), fd9bfab8 (status mapping). Disjoint functions.
- **`main.rs` doc arms `6240-6307`:** f37d228e (exit), 3754095f (markers at `6268-6295`), 66986f88-A (list output `5875-5912`).
- **`main.rs` validate arms `7361-7800`:** 3754095f only.

**Recommended order:**
1. In parallel: 72d89161-A, 72d89161-B, fd9bfab8, f37d228e.
2. 8673dc5e after 72d89161-A, because both touch the record schema and the reference projection.
3. The built-in-fault task after fd9bfab8, which defines the fault vocabulary in docs.
4. 3754095f after f37d228e, and after the D-8 deletion of `profile validate` (`main.rs:1183-1243`), or explicitly excluding those lines.
5. 66986f88-A, then -B, then -C.
6. da12ad66 last.

**Profile work (D-8):**
- It deletes `render_profile_agreement` (`main.rs:1183-1243`, three markers) and the profile checks in `jit validate` (`commands/validate.rs:549,591-775`; `validation/repository.rs:668`). Both overlap 3754095f's validate sweep.
- Init output carries no markers.
- Coordinate `main.rs` edits with the profile children.

**Other epics:**
- **df129d9e** (no implementation children yet; planning 0ba00a6f is ready):
  - REQ-21: one renderer each for the gate-run status, validation report, issue list and archive report. This collides with 8673dc5e (gate status renderer) and 3754095f (the four copied validation blocks).
  - REQ-12: a single `process::exit` site. This collides with f37d228e (`main.rs:6306`).
  - Background L17 cites `main.rs:4707,4726,4818,4897`, the gate-status error paths.
  - Recommendation: land these 3d9a0ce3 children first, since they are small and words-only, and tell df129d9e planning that the validation-report and gate-status renderers will have changed text. Alternatively, fold 3754095f's validate block into REQ-21's single renderer. Then it would wait on df129d9e, which has no children.
- **842b0ac6:** L40 names the `state_symbol` twin (`domain/types.rs:744`, `output.rs:1607`), which 3754095f touches. L39 covers dead `output.rs` items. Its investigation (`dev/active/842b0ac6-greenfield-cutover/investigation.md:93,149,162`) already references 66986f88 and plans to delete `read_issues`, the input to `doc conformance`. Coordinate.
- **8376299c:**
  - **2d10f1dd** (in progress) REQ-03 makes `issue reject --reason` one mutation at `main.rs:3951-3958`. It directly collides with da12ad66.
  - **05d1b34e** (ready) concerns bulk-update to Rejected.
- **b80e3c70** (not in the brief; found): **5f71a48d** (ready) REQ-01 has a reject `--reason` refused or "recorded somewhere that carries prose". That decides how da12ad66's "reason recorded the same way" is stored.
- **2195c98f:** REQ-11 wants one enum per closed vocabulary, including gate status. `GateRunStatus` (5 variants, 2 never produced) versus `GateStatus` (Error → Failed mapping at `gate_check.rs:229-250`). fd9bfab8 adds no variant. The built-in-fault task should reuse `Error`.

## 5. Primitive verification

| Property the plan may assert | Status | Evidence |
|---|---|---|
| The digest excludes `.git` | holds | `is_git_plumbing` (`gate_execution.rs:499-507`) matches any `.git` segment. It is checked at the root (`:443`) and per entry (`:480`), so it covers the linked-worktree file form too. |
| The digest is independent of commits | holds | `verdict_declaration` = `{stage, checker, inputs}` (`declarations/gates.rs:212-218`); content is walked bytes only (`repository_inputs.rs:411-446`). |
| The nested data root is detectable | holds, privately | `RepositoryLayout.nested_data_relative` (`repository_state/path.rs:443,468-473`) is private. A `pub(crate)` accessor is needed; `require_layout()` is at `commands/mod.rs:2140`. `JIT_DATA_DIR` can relocate the data root (`path.rs:555-557`), so hand-written `.jit/...` excludes are configuration-specific. |
| Runner-error exit code | holds | `ExitCode::ExternalError` = 10 via `ErrorCode::IoError` (`main.rs:70-75`, `output.rs:576-577,893-909`); JSON verdict at `main.rs:694-697`. |
| Errored runs are never reused | holds | `is_reusable_for` requires Passed or Failed (`types.rs:1022-1027`); test `gate_check.rs:2509`. An exit-75 `Error` run is therefore re-executed next time, which is the desired retry semantics. |
| Document listing can be derived git-free in a linked worktree | holds | Pure resolver plus no-follow filesystem walk (`artifact_directory.rs:62-108`, `storage/artifact_planning.rs:246-436`). `doc show` is git-backed (`document.rs:415-438`). |
| Attestation and run views share a source | does not hold | They read `gates_status` and `.jit/gate-runs` respectively (§1). |

## 6. Invariants

- **domain-agnostic.**
  - The artifact directory comes from `[documentation].issue_scoped_areas` and `label_associations` (`config.rs:333-335`; `artifact_directory.rs:117-126` tests use unrelated vocabulary). Holds.
  - The bare `jit init` declares no areas (`config.rs:307-315`); the `jit-default` profile contributes them (`profiles/jit-default/manifest.toml:408-429`). So derived documents produce nothing in a repository that declares nothing, which is mechanism.
  - Violations found nearby: `PLAN_DOC_LABEL = "plan"` (`validate.rs:2906`) and the link checker's hard-coded permanent paths `docs/`, `README.md`, `CONTRIBUTING.md`, `LICENSE`, `dev/architecture/` (`document/link_validator.rs:202-211`), which ignore `permanent_paths`. Both are candidates for follow-up; neither is required by the children.
  - Tracker-path exclusion for digests must come from storage layout constants, not from `.jit` spellings.
- **architecture-dependency-direction.**
  - Derived-document resolution belongs in `domain/` as a pure function over issue, config and a path set.
  - The filesystem walk stays in `storage/artifact_planning.rs`.
  - `doc list`, the link check and the server consume it through commands.
  - The digest walker in `gate_execution.rs` (I/O) receives the excluded prefixes as data.
- **semantic-types.**
  - Exit 75 maps to the existing `GateRunStatus::Error`; no new variant is needed.
  - `GateRunStatus::Pending` and `Skipped` are never produced (only parsed at `main.rs:1446-1447`), which is 2195c98f's concern.
  - The `--status` filter parses raw strings by hand at `main.rs:1446-1447` rather than through serde or clap `ValueEnum`.
  - A derived document's type, if kept, needs a closed vocabulary; today `doc_type` is a free `Option<String>`.

## Claim counts

Counted over the 50 rows in §1:
- valid: 36
- done: 7
- invalid: 6
- unverifiable: 1

Invalid as stated:
1. 72d89161 REQ-04 (excluding the whole data root is too broad).
2. fd9bfab8 background "every nonzero exit becomes failed".
3. fd9bfab8 background "terminated evaluation records nothing" (it does not say which process is terminated).
4. fd9bfab8 REQ-02 (would reclassify 126, 127 and signal exits).
5. 66986f88 background "a link is needed to pass artifact-layout review".
6. 3754095f background "emoji in initialization (and gate) output".

Unverifiable: fd9bfab8 "exit 75 is the convention downstream wrappers use".

Done but worth noting: 3754095f REQ-02 already holds, since no CLI output uses type icons.
