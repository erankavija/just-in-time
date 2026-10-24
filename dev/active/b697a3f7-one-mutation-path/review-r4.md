# Review r4: b697a3f7 plan at 700a37294 (planning node 0dc73711)

Verdict: FAIL. 17 blocking findings; 18 of 45 entries fail.

Mechanical checks pass: the validator with `--deny-warnings` exits 0 (overrides and overlap advisories only), `render --check` exits 0, and the dry run creates 45 issues and 58 edges. The manifest is within DEC-26's 50-issue budget.

Review was done without sub-reviewers (the fan-out was declined); slice evidence and helper scripts are in `scratchpad/review/r4/`. Round 3's unconsolidated slice reports (`scratchpad/review/r3/sub-*.md`) were re-verified against this manifest; findings below marked "(r3)" were raised there and are still open.

## D-22 admission protocol: sound

Checked `optimistic-read-admission`, `read-session-driver` and `event-append-invariant` against both backends.

- JSON: `RedoRecord::commit` creates and syncs the record (redo_record.rs:208-216) before `apply`; `apply_operations` runs make-directory, put, delete, then the append last (redo_apply.rs:130-175, append :168-174); `remove_record` runs after (`:121-124`). The append is positioned at the captured end (`compose_event_action`, mutation.rs:1236-1240), so the log length grows on every appending commit; `set_len` (redo_apply.rs:319) only truncates a replay, which runs only while the record exists.
- Memory: `commit_record` installs the record in one state section (memory.rs:499); `MemoryRedoApplier::apply` applies, appends and clears `state.record` last under one guard (memory.rs:533-585). The memory mutation session holds the write-lock guard, not the state mutex, so readers can probe beside a held session (repository_state_store.rs:513-542).
- Open probe → L0 → capture → revalidate → probe → L1 refuses every writer with a live write inside the capture window, given `event-per-commit`. The absent-root exemption is sound (presence change → `Moved`; carried worktree record → `Busy`). No bypass writer of captured content exists (claims/leases, PID, identity and locks are uncaptured).

The protocol itself needs no change. Defects are in the entries that implement and consume it (findings 1, 2, 4, 6, 7).

## R2 closure

All substantive r2 findings stay closed: 1, 2, 4, 7, 11, 14, 15, 23 are ordered by edges; 3 and 20 are stated in `readiness-in-mutation-plans` and the Behaviour changes row; 5, 12, 13, 16, 21, 22 are addressed in the entry text; 6, 19 are dropped by DEC-26; 8 and 9 are closed (`executor-config-caches-removal` reaches 1f24e3e9 and the three other removers). Sizing findings 10, 17, 18 are superseded by DEC-26.

DEC-01/DEC-02 re-homing verified with `jit issue show`: every "covering"/"reaches" claim holds (6c0f8739→0bd2c797/417be079/e98ae5a4, 7244ecb2→2f563673/53d19851, f22147ec→2c8c13b2/208edf4c/03bf6983, 6ac41880→145b4d57, d0d73bef→5e1ba005, e1a6c8c2→945f65f3), all 18 "reached transitively" ids are reached, and every "done" id is done. One edge is missing (finding 2).

## Chain-link overrides: accepted

- `gate-evidence-capture`: REQ-12's evidence capture is its own change; it shares only files with `state-derivation-convergence`. Not a producer-to-remover link.
- `archive-reference-resolution-once`: a pure domain algorithm change (REQ-08). Not a link.
- `archive-candidates-one-session`: a distinct behaviour (one session for all candidates, DEC-12/DEC-22) with its own test. Not a link.

## Blocking findings

### 1. Backgrounds predate code that landed before the plan commit (r3)

a807e699 (done; 5d9e1def3, 893168b19, both before 700a37294) moved repository opening from main.rs into `crates/jit/src/commands/bootstrap.rs`, added 14 lines at repository_state_store.rs:87 and 3 lines early in commands/mod.rs. d4373d07 rewrote serve.rs. The entries still say "line citations name current code". Substantive consequences:

- `captured-declaration-reads` REQ-03 and Background name `linked_checkout_write_decision` in main.rs (`:1694-1730`, tests `:7627-7788`). It is `bootstrap.rs:218`, reading `ConfigManager::new(jit_dir).get_write_policy()` at `:227`; its tests are `bootstrap.rs:351-`. bootstrap.rs is in no footprint. plan.md "Pre-executor write-policy read" row cites the main.rs contract.
- `executor-config-caches-removal` deletes `ConfigManager`, but its "remaining users" list omits `bootstrap.rs:227`; the deletion breaks the build outside its footprint. Its `invocation_write_override` cites `main.rs:1791, 5920` are `main.rs:1625, 5618`.
- `read-session` REQ-05: `is_leftover_record_failure` is `storage/repository_state_store.rs:89` (`pub(crate)`), not main.rs:188-200; a typed `RecoveryFailedError` already exists (bootstrap.rs:45) and `exit_recovery_failed` is main.rs:173, reached only from bootstrap's recover path (main.rs:1651). `output/failure.rs::classify_code` maps nothing to `RecoveryFailed`, and that file is not in the footprint. A new "typed leftover-record error" beside `RecoveryFailedError` is a parallel variant (`@/inv/convention-convergence`).
- `lease-check-in-session` (startup session `main.rs:1876-1905`) and `snapshot-from-one-image` (retained session `main.rs:1893-1899`): the code is `bootstrap.rs:102-130`, consumed at main.rs:1626-1648.
- `process-liveness-primitive`: `is_process_alive` is serve.rs:154 (cited :118-138); callers are :571, :701, :751, :794 (cited :538/:661/:704); tests :906-920, :1494, :1527. main.rs has no liveness call, so the cited "foreground serve path main.rs:6867,6873" and main.rs in the footprint are phantom.
- `control-files-publication`: `write_pid_file` is serve.rs:121 (cited :85).
- Mechanical drift: every repository_state_store.rs cite past :84 is 14 low (`read-session`, `event-per-commit`, `image-identity-token`, `capability-fs-module`, `leaf-open-once`, `record-commit-through-capabilities`, `redo-apply-parents-once`); every commands/mod.rs cite is 3 low (e.g. `with_mutation_session` :455, `captured_precheck_plan` :659, `captured_lease_warnings` :778, `plan_repository_export` :2265, `transition_blockers` :3258, `require_active_lease` :3399); every main.rs cite in 11 entries is stale.

Correction: rebase every Background on 700a37294. Add `crates/jit/src/commands/bootstrap.rs` to `captured-declaration-reads` and `executor-config-caches-removal` and name `bootstrap.rs:227` in the latter's user list. In `read-session` REQ-05, reuse `RecoveryFailedError` (moved where storage and commands both reach it) and map it in `classify_code`; add `crates/jit/src/output/failure.rs`. Drop main.rs from `process-liveness-primitive`.

### 2. `lock-free-read-consumers` edits the `--fix --dry-run` path that open 1f24e3e9 rewrites, with no edge (r3)

- REQ-01/REQ-05 move `repair_derived_state(true)` onto `with_read_session`. Its only `dry_run = true` caller is `validate_with_fix` (validate.rs:72); `jit recover` calls it with `false` (claim.rs:725).
- 1f24e3e9 (backlog) REQ-01 makes `--fix` and `--fix --dry-run` derive every fix from one validation capture, which replaces that path. After it lands, `repair_derived_state(true)` has no caller, and DEC-24's `--fix --dry-run` read is moved by no task.
- No path joins them: `lock-free-read-consumers`' ancestors' externals (42bcaa6e, bc1955a8, e1a6c8c2, 1f6d3754, 182350ce, 4575a03d, 5c232305, 18dc6e3e, 855d027d, 0851fae4) do not reach 1f24e3e9.

Correction: add DEC-02 external dependency 1f24e3e9 → `lock-free-read-consumers`. Rewrite REQ-01/REQ-05 against 1f24e3e9's one-capture derivation: `--fix --dry-run` runs it through `with_read_session`, `--fix` through `with_mutation_session`; `repair_derived_state` stays for `jit recover`.

### 3. `capture_archive_plan` is edited by unordered tasks

`archive-candidates-one-session` REQ-01 splits `capture_archive_plan` into base and per-target halves. `container-destination-once` REQ-03 replaces its inline destination block (archive.rs:540-651), and `archive-destination-no-legacy-fallback` edits its `legacy` computation (:540). `reach.py` reports NO PATH for candidates against both.

Correction: `container-destination-once` depends on `archive-candidates-one-session` (its Background then describes the per-target half). Add the order to the "Shared-file contention" row.

### 4. The storage-format read-discipline section has three unordered writers and an unowned part

- `lock-free-read-consumers` REQ-06, `captured-declaration-reads` REQ-06 and `gate-preset-catalog` REQ-06 each "state the read admission rule once" in `docs/reference/storage-format.md`; no path joins any two.
- `lock-free-read-consumers` Notes defer archive preview and candidates, declaration and gate-preset reads "once those land", but it precedes none of them. No archive entry touches storage-format.md, so the archive reads' documentation has no owner.

Correction: `lock-free-read-consumers` owns the rule, the fallback and its error, and its command list. `gate-preset-catalog`, `captured-declaration-reads` and `archive-candidates-one-session` depend on `lock-free-read-consumers` (no cycle: it reaches none of them) and each adds only its own command line; `archive-candidates-one-session` gains storage-format.md and covers preview and candidates. Delete the "once those land" note.

### 5. `read-session` breaks integration-test implementors outside its footprint (r3)

`template_apply_atomicity_tests.rs:216` and `local_rule_enforcement_tests.rs:797` implement `RepositoryMutationSession`; `:301` and `:833` implement `RepositoryStateStore`. Moving `layout`/`capture`/`extend` to the supertrait (REQ-01) and adding a required `open_read_session` (REQ-02) break both test targets.

Correction: add both files to the footprint; REQ-01 says every implementor, the two probe sessions included, implements both traits; REQ-02 says the probe stores delegate `open_read_session`.

### 6. `read-session` REQ-03 asks for a record that does not exist (r3)

At `RedoPoint::AfterApply` the record is already removed (redo_apply.rs:122-124; memory.rs `state.record = None` before the check). Correction: "a record left at each `RedoPoint` before `AfterApply` gives `Busy`; an interruption at `AfterApply` gives `Moved`".

### 7. `event-per-commit` would stamp published events with the preview context (r3)

REQ-01: "previews and validation pass `MutationContext::preview()`". `capture_repair_plan` (validate.rs:603, derive at :634) sits in validation but is published by `jit validate --fix` and `jit recover`; project render (project.rs:167) holds no context today. `compose_event_action` takes ids and timestamp from the context (mutation.rs:1208-1213), and `preview()` is deterministic with a zero seed and the epoch (mutation.rs:276-281), so every published repair would append an epoch-stamped `files_published` with a repeated id. Also "Four builders already take a context" lists three.

Correction: every caller whose plan is published (repair, project render, config set, export, package tree, init, profile apply) passes `production_mutation_context()`; only non-publishing derivations pass `preview()`. REQ-05 adds: two successive event-less writes append events with distinct ids and non-epoch timestamps. "Four" → "Three".

### 8. `read-concurrency` story claims reads it does not aggregate (r3)

Its REQ-21 includes declaration reads, but the story does not reach `captured-declaration-reads` (NO PATH). Correction: add `captured-declaration-reads` to the story's `depends_on`, or restate its criteria over its own subtree.

### 9. `one-form-convergence` story cites REQ-01 with no child carrying it

Its `source_refs` include REQ-01 (epic readiness); no child cites it, and its local "REQ-01" is the domain-purity criterion. Correction: remove REQ-01 from `source_refs` and re-render.

### 10. `cost-evidence` names deleted keys and a false environment fact (r3)

Background names `redo-parent-handles-once` and `redo-sync-coalescing`, merged into `redo-apply-parents-once` in 700a37294, and says "`strace` is not installed on the reference host"; it is (`/usr/bin/strace`). Correction: name `redo-apply-parents-once`; delete the strace sentence.

### 11. `executor-config-caches-removal` migrates a method no production code calls (r3)

Its Background calls `CommandExecutor::container_anchor` (template.rs:96-101) "`jit apply`'s anchor binding" and the last production reader. It has no production caller; `jit apply` binds from the captured config in `captured_anchor_bindings` (template.rs:259). Correction (D-18): REQ-01 deletes `CommandExecutor::container_anchor`; its tests read the anchor through the captured-declaration helper; fix the Backgrounds of this entry and `captured-declaration-reads`.

### 12. `captured-declaration-reads` states a behaviour change that contradicts DEC-09 (r3)

Notes: "command and server reads serialize behind writers instead of reading disk directly". DEC-09 and the same Note's first clause say they take no lock while uncontended. Correction: delete the serialization clause.

### 13. `archive-candidates-one-session` REQ-03 cannot observe a read-session open (r3)

`SessionOpenCounter` counts only `MutationPoint::SessionOpen` (test_helpers.rs:63-69), which only `open_mutation_session` crosses; `read-session` REQ-02 defines a read open that crosses no point. An uncontended listing counts 0, so "exactly one session open, read or mutation" fails. Correction: `read-session` REQ-02 makes a read open cross a new `MutationPoint::ReadSessionOpen` on both backends (add `storage/mutation_interruption.rs`); `SessionOpenCounter` counts both (add `commands/test_helpers.rs` to the candidates footprint); REQ-03 asserts one read-session open and no mutation-session open.

### 14. `archive-destination-no-legacy-fallback` breaks the relocated citation-test helper (r3)

REQ-02 makes the destination root a required argument of `classify_artifacts`. `plan_with_scan` (artifact_planning.rs:501-532, call :530) calls it and `archive-preview-through-execute-planner` relocates it into `storage/artifact_inspection.rs`, which is not in this footprint. Correction: add the file and name the helper as a caller.

### 15. `record-commit-through-capabilities` breaks two test targets outside its footprint (r3)

- `recover_command_tests.rs:149-161` calls `RedoRecord::new(..).commit(..)`, which REQ-01 removes.
- `REPOSITORY_STATE_PUBLISHERS` admits `"pub struct RedoRecord {"` as a writer (repository_state_cutover_tests.rs:582-586); once redo_record.rs performs no I/O, the scan's "publisher writes nothing" assertion fails.

Correction: add both files; REQ-01 moves the publisher entry to the commit's new module; REQ-04 covers the recover test.

### 16. `control-files-publication` rests on a false location claim and an unholdable criterion (r3)

`pid_file_path(data_dir)` is `data_dir.join("server.pid.json")` (serve.rs:105) with `data_dir` the `.jit` data root (main.rs:6576); `worktree.json` is also in the data root. The cutover test says so (repository_state_cutover_tests.rs:660-662). The Background's "control files outside every repository root ... such as the server PID file" premise for a symlink-following path entry is false, and REQ-06's "cutover scan passes with its control-file classes unchanged" cannot hold for a new crate-visible storage writer. Correction: publish the in-root control files through a no-follow data-root parent (or state why following is acceptable), restrict the path entry to `.git/jit` files, and let REQ-06 add one named class for the entry.

### 17. Plan prose left over from merges and removed features

- DEC-03: "`executor-config-caches-removal` and `executor-config-caches-removal` delete those exposures. If they land first…" — duplicated key. Name the one task.
- DEC-13: "behind a re-export bridge with one remover" contradicts `root-relative-path-in-domain` ("with no re-export"). State the one-task move.
- DEC-15: "`configuration-store` and its four sibling leaves" — no siblings remain.
- Shared-file contention: "initial creation readiness precedes the base declarations (`finalize`)" — `declarations-on-mutation-base` does not edit `finalize`; the creation passes are in `finalize_delta` (mutation.rs:725), which `readiness-in-mutation-plans` cites as `finalize`. Rename to `finalize_delta` in the entry and drop or correct the pair.
- `configuration-store` contract: "(the user directory `config validate` checks)" — `config validate` checks only the repository config and the invocation override (main.rs:5611-5618).
- `gate-preset-catalog` contract: "over built-in presets" — none exist (gate_presets.rs:8: "Every preset is a project's own"); the entry builds from the image only.

## Failing simulation rows

| Key | Coherent change | Observable test boundary | Footprint credible | One focused cycle | Not a chain link | Result |
|---|---|---|---|---|---|---|
| read-session | Y | N | N | Y | Y | FAIL (1, 5, 6, 13) |
| lock-free-read-consumers | Y | Y | N | Y | Y | FAIL (2, 4) |
| event-per-commit | Y | Y | Y | Y | Y | FAIL (7) |
| cost-evidence | Y | Y | Y | Y | Y | FAIL (10) |
| read-concurrency (story) | Y | Y | — | Y | Y | FAIL (8) |
| captured-declaration-reads | Y | Y | N | Y | Y | FAIL (1, 4, 11, 12) |
| executor-config-caches-removal | Y | Y | N | Y | Y | FAIL (1, 11) |
| gate-preset-catalog | Y | Y | Y | Y | Y | FAIL (4, 17) |
| lease-check-in-session | Y | Y | Y | Y | Y | FAIL (1) |
| archive-candidates-one-session | Y | N | N | Y | Y | FAIL (3, 4, 13) |
| container-destination-once | Y | Y | Y | Y | Y | FAIL (3) |
| archive-destination-no-legacy-fallback | Y | Y | N | Y | Y | FAIL (14) |
| snapshot-from-one-image | Y | Y | Y | Y | Y | FAIL (1) |
| process-liveness-primitive | Y | Y | N | Y | Y | FAIL (1) |
| control-files-publication | Y | N | Y | Y | Y | FAIL (1, 16) |
| record-commit-through-capabilities | Y | Y | N | Y | Y | FAIL (15) |
| readiness-in-mutation-plans | Y | Y | Y | Y | Y | FAIL (17) |
| one-form-convergence (story) | Y | Y | — | Y | Y | FAIL (9) |

The other 27 entries pass.

## Non-blocking

- Sizing (heuristic only, DEC-26 standard met): `root-relative-path-in-domain` (23 files, mechanical repoint), `archive-preview-through-execute-planner` (16 files, six criteria), `captured-declaration-reads` and `executor-config-caches-removal` (15 files each) are large but each is one coherent change one worker can deliver and review.
- `admit` should map the `RetryableConflict` that `revalidate_layout_capabilities` returns for a replaced root of unchanged presence (repository_state_store.rs:1913-1943) to `Moved`; REQ-02 names only presence changes, so such an error would fail the read instead of retrying.
- `atomic-publication-core` does not say where `write_staged_file` (redo_apply.rs:256-270) goes once the redo put uses the core; its other caller is `init_publish.rs:13,63`, outside the footprint. State that the core's stage writer replaces it for both callers, or a parallel stage writer results (r3).
- `bulk-apply-one-session` adds a precheck preflight through the shared state derivation while `gate-evidence-capture` replaces the precheck evidence type, with no path between them. They edit different functions; whichever lands second adapts. Consider `gate-evidence-capture` → after `bulk-apply-one-session`.
- `event-per-commit` uncertainty is settled by the code: neither `RepositoryExportIntent::capture_spec` nor `PackageTreeCapture::capture_spec` captures `EVENTS`; name both and drop the uncertainty. Its REQ-05 CLI test file is not in the footprint.
- Entries carrying REQ-21 as a source (`captured-declaration-reads`, `gate-preset-catalog`, archive preview/candidates) lack `satisfies:b697a3f7/REQ-21`.
- `dev/benchmarks/read-concurrency/` and `dev/benchmarks/cost-evidence/` omit the `-<short-id>` suffix other issue-bound benchmark directories carry.

After the corrections, regenerate the plan and rerun the validator, `render --check` and the dry run.

VERDICT: FAIL
