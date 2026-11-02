# Drift audit of the f8a6b298 issues at 804acc4d6

Static audit taken before amending the plan on 2026-11-03. Line citations name that commit.

## Drift audit: story `per-key-gate-runs` (f8a6b298) at 804acc4d6

Method: every issue body read; claims checked against source and against `.jit/` data with a read-only script. No build or test was run, so behaviours are stated from reading the code. Paths are relative to `crates/jit/src/` unless they start with another root.

### Verdict

Needs issue-body amendments plus one owner decision. Replanning is not needed: the layout, verdict record and provenance designs still fit.

Drift is concentrated in two landed changes:

- 8673dc5e (d9f1d644d, e2f2693d8) already put a run pointer on the gate state and routed two latest-run readers through it, under the name `run_id`.
- 952ed1e7 (c36c6a393, 2026-11-01) added a whole-history gate-run reader to `jit validate` that no issue knows about. It blocks the cutover as written.

Class counts (17 issues): SOUND 7, PARTLY-DONE 5, CONTRADICTED 3, GAP 2, DONE-ALREADY 0.

### Issues

| Id | Title | Class | Evidence |
|---|---|---|---|
| 77f3db10 | Gate state names the run | CONTRADICTED | REQ-01 asks for `GateState.last_run`; `pub run_id: Option<GateRunId>` already exists, optional and skipped when absent (`domain/types.rs:331-335`, d9f1d644d). REQ-02 is met by `gate_state_from_run` (`commands/gate_check.rs:211-222`), callers `:974`, `:1468`, `commands/mod.rs:3025`. New-gate half of REQ-03 is met (`commands/mod.rs:1744-1749`). **Conflict:** REQ-04 "a manual status write leaves `last_run` unchanged" and REQ-03 "a status write that records no run keeps the prior" contradict the 8673dc5e design: a manual write mints a run and names it (`commands/mod.rs:1828-1835,1845-1852`). No status write without a run exists, so the keep-prior rule has no subject. Background "manual gate write replaces the state wholesale" is true but now carries a run. Issue is in state `ready`, so a worker could add a duplicate field. |
| 803b107c | Runs load by issue and run id | SOUND | `load_gate_run_result(run_id)` at `storage/mod.rs:302`, `json.rs:1214`, `memory.rs:863`; route discards `:id` at `crates/server/src/routes.rs:1002-1014`. Background is incomplete: `projected_gate_run` (`gate_check.rs:1206-1217`) is a second production caller, with the issue at hand. REQ-03's suite is `storage/issue_store_conformance.rs:558`. |
| bf8df628 | Executor accessor | PARTLY-DONE | `projected_gate_run` already loads the named run with no listing, and `get_last_gate_runs_for_issue` (`gate_check.rs:1331-1358`) applies it per required gate. Missing: a form over every gate state, the `(issue, run)` load, and REQ-02's tests. Background "three readers ... break ties differently" is false: two read the named run, and `GateView::build` uses `GateState::projected_run` (`output.rs:1752`). Check first whether a new accessor is warranted or `get_last_gate_runs_for_issue` serves (D-18). |
| 2117739f | Latest-run readers use accessor | PARTLY-DONE | Met: `get_last_gate_run` (`gate_check.rs:1195`) and `get_last_gate_runs_for_issue` list nothing. The `commands/gate.rs:1005` caller is gone (72d89161). Open: `from_issue` callers still list every run: `commands/issue.rs:757-764`, `:782-783`, `main.rs:2074`. REQ-03 and REQ-04 are open and executable; the test exists at `tests/cli_gate/gate_cli_integration_test.rs:65`. **Risk not in the body:** redo puts follow action order (`storage/repository_state_store.rs:1014-1028`), issue records before runs (finalizer "Pass 3", `repository_state/mutation.rs:1018`), and `load_issue` reads with no redo check (`storage/json.rs:1081-1098`). After this change, `issue show` hard-fails with not-found in that apply window or after a crash until a session completes the record; today `projected_run` yields `None`. `gate status` already has this window. |
| 0bd2c797 | Captures hold only latest runs | CONTRADICTED | **Premise false:** "these three captures are the only callers of `discover_gate_run_results`". A fourth is `capture_record_fidelity_validation` (`commands/validate.rs:948-957`), which needs every run. REQ-02 cannot hold without deciding that caller. `captured_gate_run_result_paths` no longer exists; its successor `captured_gate_run_ids` (`repository_state/mutation.rs:520`) has a further caller at `validation/record_fidelity.rs:38`. **Behaviour conflict:** REQ-01 captures runs only "for each history-consuming gate", but the checker's issue view builds every other gate's `last_run_at`/`exit_code` from those runs (`gate_check.rs:1263`, `output.rs:1868`; `test_projected_precheck_run_matches_next_checker_issue_view`, `:1810`). Non-consuming gates would silently go null. REQ-04 "without selecting among runs" fails when several gates' runs are present, and in the precheck loop that appends fresh results (`gate_check.rs:1432,1469`). `test_lifecycle_precheck_captures_large_history_and_ignores_root_clutter` (`gate_check.rs:4142-4184`) must be retired here and is unnamed. 512 MB budget holds (`commands/mod.rs:260`). |
| 93b7f1dd | Reused verdict names issue | SOUND | `Reused(GateRunId)` adjacently tagged (`domain/types.rs:1020-1036`), producer `gate_check.rs:67`, render `main.rs:935`, reference `storage/reference.rs:262,379`. Added since plan: an `Attested` unit variant, compatible with internal tagging. Unnamed stale text to sweep: `output.rs:2483`. 295 stored reused records (body says 123), 265 of them cross-issue, all sources resolve, none reused-from-reused. |
| 964db1a8 | Web provenance type | SOUND | `web/src/types/models.ts:48-51`, now with `attested`. No web code reads `source_run` beyond the type. |
| b33ed7eb | Verdict record writes | SOUND | Gate-run pass at `repository_state/mutation.rs:1018-1052`; reuse predicate `GateRunResult::is_reusable_for` (`domain/types.rs:945`). Only `check_gate_with` sets a digest (`gate_check.rs:937`), so the attestation capture budget (depth limit) is not reached. Depth 4 fits `MUTATION_CAPTURE_BUDGET` (32). Implementer must also capture `gate-runs/verdicts` and `<gate-key>/` for the create-directory decision, as the root does at `:1019-1030`. |
| 6c0f8739 | Reuse reads one verdict record | SOUND | `find_reusable_gate_run` at `storage/mod.rs:332`, `json.rs:1305`, `memory.rs:894`, both test wrappers present; `reusable_verdict` already gone. Executable note: digest is computed after the inputs session (`gate_check.rs:918-921`), so REQ-01's "from the mutation image" needs a capture after the digest, or the digest moved into the attempt. Conformance test `issue_store_conformance.rs:661,1230` goes with REQ-02. |
| be45195f | Scanning selection gone | GAP | `latest_gate_run` (`domain/types.rs:1004`) has only the two named callers. **Gap:** REQ-01 "no selection of a latest run over a list of runs remains in `domain/types.rs`" also catches `GateState::projected_run` (`:340`), added by d9f1d644d. Its callers go with 2117739f and 0bd2c797, it is `pub` so no dead-code warning fires, and no issue deletes it. Needs naming, plus an edge to 2117739f (this issue reaches only 0bd2c797 today). |
| 0851fae4 | One shared reader | PARTLY-DONE | Met or moot: REQ-02 is largely done by 2c9badcd (7f0473a36), `test_gate_run_readers_*` at `storage/issue_store_conformance.rs:637-840` across both backends. Background "conformance suite is `repository_state_store_tests.rs`" is stale. `gate_run_vpath` and `gate_run_directory_vpath` are gone. Open: `read_gate_runs` and load in both backends (`json.rs:858,1214`, `memory.rs:755,863`); paired tests `json.rs:1690,1709,1738`, `memory.rs:1011,1035`, helper `gate_run_parent_entry` (`:986`). **REQ-01 flaw:** "built on `read_repository_file_nofollow`" is a filesystem function (`storage/repository_state_store.rs:1710`, takes `root: &Path`), so the memory backend cannot sit on it, and no directory-listing primitive exists across backends. The issue must name the seam. `test_held_gate_run_directory_capability_cannot_escape_...` (`json.rs:1765`) is file-only and should not count as paired. |
| 60dc12bb | Constructor takes issue | CONTRADICTED | **Premise false:** "Every caller has the run's issue at hand." `record_drift` (`validation/record_fidelity.rs:38-42`) enumerates ids and has no issue before parsing; `discover_gate_run_results` (`commands/mod.rs:2201-2204`) likewise. `gate_run_record_paths(context, count)` (`commands/mod.rs:174`) takes no issue, though its callers have it. Caller list is stale: it omits `record_fidelity.rs:42`, `validation/repository.rs:1068,1096`, `crates/server/src/watcher.rs:324`, `issue_store_conformance.rs:254,740,818`. `VirtualPath::gate_run_directory(&GateRunId)` (`repository_state/path.rs:244`, 39 uses with the record-file constant) has no home in this issue. |
| a2a1b45b | Fixtures through constructor | PARTLY-DONE | Listed spellings already gone: `gate_check.rs:2688,2706-2722`, `commands/mod.rs:4401`, `storage/gate_runs.rs:28-30`, `repository_state_store_tests.rs:1440-1441`. Remaining in listed files: `gate_check.rs:4169,4184`, `repository_state_store_tests.rs:1472,1482,1496`. Not listed and needing it: `issue_store_conformance.rs:648,765-775`; `validation/repository.rs:1036` reads the first entry of the root as a run id, which breaks under the layout. Real remaining seam is test use of `gate_run_directory` and `GATE_RUN_RECORD_FILE`. |
| 1630015d | Per-issue layout flip | GAP | Requirements are executable, but the contract's "changes only the constructor body and the shared listing" no longer holds. Unlisted: `repository_state/mutation.rs:1039-1045` creates the run directory with `ExpectedPreimage::Absent`, which fails on an issue's second run, so it needs a captured entry. `gate_run_directory` and `GATE_RUN_RECORD_FILE` (`path.rs:15,244`) must change or go. `captured_gate_run_ids` filters on run directories. **Hard block:** `capture_record_fidelity_validation` uses `MUTATION_CAPTURE_BUDGET.max_listings = 256` (`validate.rs:879,952`, enforced at `repository_state/image.rs:733`); per-issue enumeration needs one listing per issue (1,415 today), so `jit validate` and the `repo-validate` gate fail. It must also skip `verdicts/`. Own test is at `path.rs:906`, not `mutation.rs`. Script `scripts/generate-storage-records-reference.sh` exists. Edge to 8673dc5e is satisfied. |
| e3b1908d | Layout prose | SOUND | `storage/reference.rs:184-186,197-199`; `domain/types.rs:857,861`; `docs/reference/storage-records.md:60,122,125`. |
| 2a23bb58 | Repository rewrite | PARTLY-DONE | **Moot:** the `last_run` step. e2f2693d8 converted the data: 3,367 of 3,367 stored `run_id`s equal the greatest `(started_at, run_id)` run, 0 dangling, 0 wrong owner. Only 2 states with runs lack one (issue 8673dc5e: `cargo-ci`, `code-review`). Counts are stale: 6,306 runs (body ~4,700), 1,415 owners (~980), 1,359 with digest (584) over 931 keys (395), of which 928 keys have a qualifying run; 295 reused (123); 150 run ids cited in `dev/`, `docs/` (92). **Unnamed requirements:** 3,560 records have no `origin` and must count as executed for verdict selection. "Run bytes otherwise unchanged" must still satisfy `gate_run_drift` (`validation/record_fidelity.rs:81-105`): origin must round-trip and current-keyset records must stay canonical. REQ-03 "validation passes" is blocked by the budget above. 2,289 records keep legacy `tree_dirty`, which fidelity tolerates. |
| 735949ef | Story | SOUND | Criteria abstract; edges correct. Note the epic-level conflict below. |

### Contract findings

| Contract | Status | Finding |
|---|---|---|
| `gate-run-record-layout` | Fits, with stale detail | Constructor takes `&GateRunId` (`path.rs:252`). "Everything under `gate-runs/` is tracker-written" holds via `VirtualPath::RECORDS` and `tracker_written_paths` (`path.rs:397,628-651`). Omits `gate_run_directory`. |
| `gate-state-last-run` | Superseded | The pointer exists as `run_id`, is documented (`docs/reference/storage-format.md:129,161`) and typed in the web client (`web/src/types/models.ts:15`). Recommendation: keep `run_id` and rewrite issues and contract to it; a rename adds a data rewrite of 3,367 states for no behaviour (D-18). "Keeps the prior value" has no subject. Attestations set it directly, not through `gate_state_from_run` (`commands/mod.rs:1829`). |
| `latest-run-accessor` | Partly landed | See bf8df628. |
| `gate-run-flat-path-bridge` | Broken as stated | An `(issue, run)` constructor cannot serve enumerators that learn the issue only by parsing. Listing by a requested issue still works; `record_drift` does not. "Layout flip changes only the body and listing" is false (1630015d). |
| `gate-verdict-record` | Fits | Finalizer pass, preimages, depth and `is_reusable_for` all support it. Redo folding (`made_directories`) handles nested creates. |
| `reused-verdict-provenance` | Fits | Must keep `Executed` and `Attested` as tag-only forms; a record with no `origin` defaults to executed. |
| Shared reader | Does not exist | Two readers; no cross-backend listing primitive. `read_path_bytes` is not a candidate: it resolves against the worktree root and uses git2 (`storage/json.rs:1358-1376`). |
| `CaptureSpec` | Fits, budget does not | `discover_paths`, `discover_listing`, `capture_discovering`, `grow_discovering` support addressed capture. 256-listing budget is the one conflict. |
| Redo publishing (D-17 amendment) | Fits | Gate evaluation and attestations append an event, so "every record appends" holds. Ordering risk under 2117739f. |
| D-22 | Not landed | `with_read_session` has no match in `crates/`; it arrives with b697a3f7 (578ed10d, 0672b087). Story code reads direct until then. |
| Plan risk table | Stale | "Edge: 8673dc5e depends on `gate-run-per-issue-layout`" is reversed in the graph (1630015d depends on 8673dc5e, now done). |

### Owner decision required

`jit validate` reads every gate-run record (and the whole event log, via `event_log_drift`) for record fidelity (`commands/validate.rs:948-957`, `validation/record_fidelity.rs:51-53`). That conflicts with D-19 ("gate-run history ... read only by commands that report history"), epic REQ-13, and DEC-19's reader list, which omits validate. Options: (a) name fidelity a history reader, raise its listing budget and add `validate` to the reference, or (b) move fidelity to a dedicated history command. Recommendation: (a), the smaller change (D-18), since its comment already scopes the cost to whole-repository validation, while checking that REQ-14's list stays unaffected. The event-log half belongs to the history-boundary audit (5b0e3d63).

### Uncovered consumers of the flat layout

| Site | Landed by | Coverage |
|---|---|---|
| `validation/record_fidelity.rs:38-42`, `commands/validate.rs:948-957` | 952ed1e7 (c36c6a393) | **None.** Blocks cutover. Also needs an issue-matches-directory check that today's run-id check has no analogue for. |
| `repository_state/mutation.rs:1039-1045`, `:520-535` | pre-plan | None by name. |
| `commands/mod.rs:174-189` `gate_run_record_paths` | post-plan | None by name; all four callers change. |
| `validation/repository.rs:1036` test helper | post-plan | None. |
| `commands/gate_check.rs:4142-4184` | post-plan shape | None. |
| `storage/issue_store_conformance.rs:648,765-775` | 2c9badcd | None. |
| `domain/types.rs:340` `projected_run` | d9f1d644d | None. |
| `output.rs:2483` rustdoc of provenance shape | post-plan | None. |

Checked and covered or unaffected:

- `.agents/skills/jit-execution-lead/references/lead-review-protocol.md:30`, `rework-prompt-template.md:24`: ba4c6054 covers these (they would match nothing, silently). Its `cli-commands.md:2171` citation moved to `:1975`.
- `docs/reference/storage-format.md:29-31,278`, `docs/concepts/overview.md:99`: ac8f07c2.
- `crates/server/src/watcher.rs:92-99`: matches any path component, unaffected apart from its test's call at `:324`.
- `web/src/api/client.ts:123,128`: route shape unchanged.
- `tests/cli_gate/bracket_coverage_gate_run_test.rs:153`, `gate_cli_integration_test.rs:397`: existence checks only, unaffected.
- `contrib/gates/ai-review.sh:114`, `scripts/cargo-ci.sh:18`, `.agents/skills/jit-parallel/references/worktree-mode.md:92`, `commands/gate.rs:25`: name the directory only.
- `mcp-server/`, `profiles`: no consumer. No script under `scripts/` reads records.

### Overlap with open epics

- **b697a3f7: ordered correctly, no duplicated work.** 700a96dd, c2b3cefa and 578ed10d reach 0bd2c797 and 6c0f8739, and 0672b087 reaches 1630015d. d58f80c9 depends on 0851fae4, and 4334df62 REQ-04 leaves the gate-run readers to it. Nothing in this story depends on b697a3f7. 578ed10d moves validation captures to a read session, so it touches `capture_record_fidelity_validation`; it follows 0bd2c797 but not 1630015d directly, though 0672b087 does. Stale citation on that side: 5d09e74b names `gate_run_result_relative_path`, which no longer exists.
- **f4d9502a: no functional overlap.** 3f8da8c4 and 5046408a touch commit and branch evidence on runs, not layout. Removing git2 touches `read_path_bytes`, one more reason not to build the reader on it. No edges exist and none is needed beyond shared-file ordering.

### Required amendments

1. 77f3db10: rename to `run_id` in the issue and every sibling. Keep only the missing tests (evaluating twice names the second run; a manual write names its attestation run). Delete the keep-prior criterion. It can then close quickly.
2. New issue or 0bd2c797/1630015d criteria: record-fidelity enumeration, budget and `verdicts/` skip, after the owner decision. Rewrite 0bd2c797's "only callers" and decide `discover_gate_run_results`' survival.
3. 0bd2c797 REQ-01: capture the named run of every required gate whenever a checker consumes history. Fix REQ-04 to select by gate.
4. 60dc12bb and 1630015d: state the treatment of `gate_run_directory`, `gate_run_record_paths`, the finalizer's absent expectation and the no-issue enumerators. Refresh caller lists.
5. 0851fae4: name the cross-backend seam; narrow REQ-02/REQ-03 to what remains.
6. bf8df628 and 2117739f: rewrite Background to current readers. Decide the redo ordering (run before issue) or accept the window until D-22 reads land.
7. be45195f: name `projected_run`; add the edge to 2117739f.
8. a2a1b45b: replace the list with the sites above.
9. 2a23bb58: refresh counts as computed at run time, drop the `last_run` step to the two missing states, state missing-`origin` and fidelity rules.
10. plan.md: amend `gate-state-last-run`, `gate-run-flat-path-bridge`, the 8673dc5e risk row and DEC-19.

## Drift audit: story per-key-leases (epic f8a6b298), main 804acc4d6

Read-only audit. Lines cite the tree at 804acc4d6. One side effect: `jit claim list --json` ran once, which the brief permits and which rewrites `.git/jit/claims.index.json` (no lease change; the index holds none).

Plan commit: f07c24822, closed by 094813585 (2026-10-21).

**Verdict: amend issue bodies, then execute. No replanning.** The storage design (per-issue files, `claims.seq`, held lock, lock order) fits the code. One behavioural blocker (recover runs inside a retained session), one owner ruling (D-22 versus lease listing), one false test-layout contract, and one set of rejected-627fdcc4 residue need fixing first.

Class counts: SOUND 9, CONTRADICTED 9 (1 high, 4 medium, 4 low), PARTLY-DONE 1, GAP 1, DONE-ALREADY 0.

### Issues

Moved lines with the symbol still present are not listed. Severity is added to CONTRADICTED because the class spans a false background sentence and a requirement that cannot run.

| Issue | Class | Evidence |
|---|---|---|
| 1a2e905c status stale at read | SOUND | `Lease.stale` is stored (`claim_coordinator.rs:55-57`); only the rebuild computes it (`:1266-1269`); `get_active_leases` returns the stored value (`:1129-1150`); human render reads it (`main.rs:7142`). Its hook motivation is gone but its own criteria stand, and a085ca6a depends on it. |
| 5ee85aed story | SOUND | Criteria restate children. |
| 63de0883 claim acquire event | SOUND | Capture requests the whole log (`commands/claim.rs:186`); `captured_events` used only at `mutation.rs:846`; `event_tail_reflects_claim` at `:673`; `discover_tail` at `image.rs:603`, and a tail-only claim spec already exercised at `mutation.rs:1757`. Doc comments `claim.rs:125-131,156-164` go stale and need the sweep. REQ-08 analysis below. |
| 6b1ed243 issue claim event | SOUND | Unconditional push at `commands/mod.rs:3196-3201`; `issue assign` conditional at `:1700-1703`. |
| 6b8bab91 checkout rewrite | SOUND | `.git/jit/heartbeat/` exists and is empty. "about 1,800 history lines" is now 3,695, and the index holds 0 leases (log replay also 0). Seeding `claims.seq` from the log as written is required: index `last_seq` 3692, log max 3695. |
| 7244ecb2 interruption matrix | SOUND | `MutationInterruptionInjector` (`storage/mutation_interruption.rs:28`), `RedoInterruptionInjector` (`storage/redo_apply.rs:51`). No lease seam exists. |
| 903cdeab tests through production | SOUND | All callers of `acquire_claim`, `acquire_claim_with_reason`, `release_lease` are in test code: `commands/claim.rs:799,819`, `commands/mod.rs:4449`, `commands/worktree.rs:731,734`, coordinator tests, proptests, `acquire_claim_when_reached` (`:1364`, cfg(test)). |
| a085ca6a lease lock handle | SOUND | Hand-rolled operations and `CoordinationOrderGuard::enter` (`guard_order.rs:59-64`) as stated; `load`/`list` over replay match `:649` and `:384`. |
| becea20a test-only API removal | SOUND | `:409,486,1014`; non-persisted eviction `:418-433`. |
| a7314585 recovery through lock | **CONTRADICTED, high** | REQ-01 "evicts expired leases and reclaims stale indefinite leases under one `LeaseLock`" cannot run: `jit recover` dispatches inside a retained session, and `lock()` enters `CoordinationOrderGuard`, which fails while that session holds `RepositoryOrderGuard`. Chain: `Recover` is not claims-first (`cli.rs:2796`), requires recovery dispatch (`cli.rs:2729`), so `bootstrap.rs:111-123` retains (test `bootstrap.rs:295`); the session holds the order guard (`repository_state_store.rs:457`, `json.rs:642-648`, `:185`); `main.rs:6611-6616` calls `execute_recover` with it held (only `serve` releases, `main.rs:6767`); refusal at `guard_order.rs:60-62`. Today's recover avoids this because its lease pass takes neither guard nor lock (`commands/claim.rs:664-678`). Predates the plan (b89f414f4, 1a6617868); neither plan nor investigation mentions it. Fix: classify `recover` claims-first in the same change, preserve the `is_recover` failure mapping, which exists only in the retaining arm (`bootstrap.rs:113-119`, test at `:311`), and add `recover` to `CLAIM_FIRST_LEAF_COMMANDS`. Also: 5b37a806 is still open (ready, `agent:claude`); its REQ-04 "appends an event" is ambiguous between logs, and an event-log event means a repository session inside the lease lock. |
| dd53527f ops through lock | **CONTRADICTED, medium, needs ruling** | REQ-01 "Acquire, renew, heartbeat, force-evict and lease listing each take one `LeaseLock`" against @/charter/D-22 "Reads never wait on writers", which post-dates the plan. `claim status`/`claim list` are classified read-only (`cli.rs:2930,3146`; test `cli.rs:3581-3582` "is read-only"), yet take the exclusive claims lock (`:1121-1125`) that claims-first mutations hold across their repository session (`commands/mod.rs:2731-2736`). b697a3f7 does not cover them. Option: with one atomically replaced file per lease, listing can use `read_active_leases` and leave eviction to writers and recover. That retires d7d0d0db's persist-on-read, which the epic note keeps, so it needs an owner decision. D-22's reasoning addresses repository captures, so keeping the lock is also defensible if stated. Rest of the body verified: renew's dropped eviction (`:951-961`), acquire persists eviction (`:650-651`), `evict_expired_at` log-first (`:838-849`), limits count every lease (`:680-706`). |
| cc6ef886 test helper | **CONTRADICTED, medium** | REQ-01 "One helper module under `crates/jit/tests/common/`" and the background's "`tests/common/` ... through `#[path]` (`tests/fast_issue/main.rs:6`)": the directory is gone since fed2a4f2c (ef9897ab, 2026-10-21). Harness now lives in the `test-support` surface: `src/test_cli.rs`, `src/test_utils.rs`, `src/test_harness.rs` (`lib.rs:38,43`, `dev/TESTING.md:88,203,207`). A new `tests/common` breaks @/inv/convention-convergence. Does not rely on 627fdcc4's work: the body never mentions hooks, the edge is the only link and rejection already unblocked it. Unlisted sites that REQ-01 still catches: `claim_integration_tests.rs:677` (removed later by 53d19851), plus `failure_lever_registry.toml:49-85` setup notes. |
| f0a5b90a docs | **CONTRADICTED, medium** | "the hooks ask `jit claim status`" and REQ-02 "the hooks README names `jit claim status` as the lease source" rely on rejected 627fdcc4. `scripts/hooks/README.md:17`: "No hook checks leases." Only `pre-commit` remains and it reads no lease. Delete the clause. Missing from the page list: `docs/how-to/dependency-management.md:237` ("rebuildable index"), `docs/concepts/guarantees.md:555,562` (lease-id commands; ac8f07c2 covers only that page's layout and diagram), and `dev/TESTING.md:585` once test mode goes. |
| 614a6f97 issue addressing | **CONTRADICTED, medium** | "Two remedies print the lease-id form" is false; a third: `main.rs:7148` ("Use 'jit claim heartbeat {lease}'"), also `docs/reference/claim.md:367`. Tests not listed: `lease_enforcement_tests.rs:85-91` (added by 17fbcf25), `cli.rs:3200-3230` (`test_claim_lease_arguments_parse_to_lease_ids_before_dispatch`), `cli.rs:3477-3499`. Behaviour: "resolved at the CLI boundary" removes the only working evict for a lease on a deleted issue. `issue delete` by the holder leaves the lease; `claim release` already fails on it (`main.rs` release arm through `resolve_issue_arg`, `commands/claim.rs:331`) although `validate.rs` prescribes it. State how a lease on a deleted issue is evicted. |
| 53d19851 index machinery | **CONTRADICTED, low** | REQ-02 "`jit recover`'s help names exactly its passes: stale-lock cleanup, expired-lease eviction and stale indefinite-lease reclaim" omits passes that exist: redo completion, derived-state repair, temp cleanup (`cli.rs:396-402`, `commands/claim.rs:710-718,734`). Also unlisted, forced by deleting `SequenceGap`: `ClaimsIndex::warnings` (`:220`), `ClaimsMutationGuard::warnings` (`:309`), its caller `commands/mod.rs:3018`, and `test_recover_empty_claim_log_does_not_warn_about_missing_index` (`claim_integration_tests.rs:663-668`) on `index_rebuilt`. All listed symbols exist. |
| 15009590 mutation guard | **CONTRADICTED, low** | `captured_lease_warnings` is gone, and "`ensure_claim_available`" now sits beside `refuse_foreign_leases_held` (`commands/mod.rs:815,853`, 2b21efee4, d643aaa42, 13e25ab41). The guard has seven call sites (`dependency.rs:260,348`, `gate.rs:1292`, `issue.rs:658`, `mod.rs:2732,2933,2969`), all before the session, so the design holds. REQ-01 must carry rules it omits: a stale lease is inactive (`claim_coordinator.rs:238`) and an anonymous writer conflicts with every active lease (`:232`). "takes no second claims lock" collides literally with the lifecycle's preliminary guard (`mod.rs:2933`, dropped before the held one, kept by b697a3f7 DEC-08): word it as nested. REQ-02's holder/other half duplicates `lease_enforcement_tests.rs` (@/inv/orthogonal-tests); only "claims log unchanged" is new. |
| 2f563673 store flip | **CONTRADICTED, low** | Only "`tests/common/lease_store.rs`" (see cc6ef886). `get_next_seq` (`:886`), `write_index_atomic` (`:804`), primitives (`atomic_write.rs:30,48,70`, `pub(in crate::storage)`), failure levers (`failure_lever_registry.toml:64-80`) all as stated. Stage-file gap in contracts below. |
| 915be53e read API | **CONTRADICTED, low** | Background "four places with four rules" cites `mod.rs:3355-3361` and `template.rs:303`, both gone (17fbcf25). Rules now: `ClaimsIndex::conflicting_lease` (`:235-240`), `worktree.rs:185`, `validate.rs:2268`. No `is_active`, `read_leases` or `read_active_lease` exists, so both requirements stand. |
| c3e3e2ff reader migration | **PARTLY-DONE** | REQ-02 largely met: template warnings go through `refuse_foreign_leases_read` (`template.rs:189`) over the shared `refuse_foreign_leases` (`mod.rs:784`), 2b21efee4. First bullet is obsolete: `check_active_lease`/`require_active_lease` are gone, replaced by `refuse_foreign_leases_read` (`mod.rs:831-849`), used by bulk, gate-definition and assign paths. Open: `worktree.rs:287`, `validate.rs:2255`, `mod.rs:841`. REQ-01 "No command calls `ClaimsIndex::load`" is unsatisfiable as written: `validate_claims_index_with_paths` (`validate.rs:2504`) survives until 98184c64, which follows this issue; plan-time defect (`f07c24822:validate.rs:2742`). Scope REQ-01 to lease readers. |
| 98184c64 validation removal | **GAP** | Criteria valid, symbols present (`validation/repository.rs:284,550`, `validate.rs:2488,2496`, `mod.rs:97`, `validation_lease_tests.rs:68`). Missing: JIT_TEST_MODE exists only for this check (`test_mode.rs:1`, sole reader `repository.rs:551`). Removal leaves `test_mode.rs`, `test_cli.rs:186,398`, `cli_process_fixture_tests.rs:51`, `process_state_policy_tests.rs:48`, and `dev/TESTING.md:585` as dead mechanism (@/inv/canonical-cutover). No issue removes them. 9876a23a's bridge `claims-index-input` has not landed; its REQ-03 is currently vacuous. |

### REQ-08: resolved

The epic's note is resolved by investigation §5.3 and DEC-04/DEC-12. I re-derived the assignment writers independently: `commands/mod.rs:1394,1396` (field update), `:1701,1714` (assign, unassign), `:3197,3211` (lifecycle), `mutation.rs:855` (`ClaimIssue`); `bulk_update.rs:578` is preview. `Issue::new` and batch or template creation set none. No writer landed after the plan; `issue claim --assign-only` still exists (`main.rs:2845`).

| Reachable state | Claim today | After 63de0883 |
|---|---|---|
| S-a: acquire A, `issue unassign`, acquire A (unassign writes `issue_updated`, `mod.rs:1714-1720`, so the tail still shows A's claim) | Writes the issue, appends **no event**: a record with no event, against @/inv/event-log and the amended D-17 | Issue write plus one `IssueClaimed` |
| S-b: out-of-band assignment (hand edit, merge) with no claim event | No issue write, repair `IssueClaimed` (19e7298a REQ-02) | Nothing; lease only (DEC-04 reversal) |
| Assigned to A through `issue assign`, `update --assignee`, `issue claim` with event | No-op | No-op |
| Assigned to B, or unassigned after release | Issue write plus event | Same |

`issue claim` on a same-assignee Ready issue appends a redundant `IssueClaimed` today; 6b1ed243 fixes it. Unchanged and out of scope: `claim acquire` overwrites another assignee with no refusal or release event (`mutation.rs:853-859`).

S-a interacts with b697a3f7 `event-per-commit`: if it lands first, S-a publishes a generic `files_published` instead of failing, so no edge is needed. After 63de0883 the case disappears. The DEC-04 event rule also satisfies D-22's commit counter: write and event now always travel together.

### Contract findings

| Contract | Finding |
|---|---|
| `lease-record-layout` | **Fits.** D-17 places claims outside the lock and the record (`charter:307-308`). `shared_jit = <common_dir>/jit` in all layouts (`worktree_paths.rs:101,118`): primary, linked (common dir), and a data root outside every checkout (the invoking checkout's common dir, `:94-97`, test `external_data_root_tests.rs:95`); `for_worktree_root` keeps it (`:190`). Literal `.git/jit` is a name for `shared_jit`; the constructor must build on that field. 7dd59ebf adds `worktree.json` and `worktree.lock` under `<git dir>/jit` for external roots; no collision with `leases/` or `claims.seq`. No-git: `has_claims_coordination` false, reads see no directory, D-4 holds. 5046408a keeps leases in the common dir (f4d9502a DEC-23). |
| `lease-read-api` | **Fits; unimplemented.** Signatures work. "One activeness rule" now has an extra prior holder, `conflicting_lease`, which must route through `is_active`. Note `worktree list` counting moves from "not expired" to "not expired or stale", a visible change. |
| `lease-sequence` | **Fits, with two defects.** Lock order unchanged (`guard_order.rs:3-4`, 7 guard sites before sessions; identity lock taken before coordination at `commands/claim.rs:103`, no inversion). Defect 1: recover (a7314585). Defect 2: `execute_claim_release_by_issue` takes the lock twice and lists all leases (`commands/claim.rs:362-374`). No issue moves it, though epic REQ-05 and REQ-06 name release; "Each operation takes the handle once" fails. Add to dd53527f or 614a6f97. `claim status --issue <prefix>` also lists the directory to match a prefix (`:1138-1141`), a literal REQ-05 tension of low cost. |
| `lease-id-scan-bridge` | **Fits.** Producer dd53527f, remover 614a6f97, edge chain intact. |
| `lease-store-test-helper` | **False.** `tests/common/` no longer exists. Re-home to the `test-support` surface, for example `src/test_cli.rs` or a module beside it. Linked and external fixtures need `shared_jit` as input, not `root/.git/jit`. |

Other design points:

- **Stage-file residue, gap.** `apply` publishes through a temp file in `.git/jit/leases/`. An interrupted operation leaves it, and `cleanup_orphaned_temp_files` scans only the data root (`commands/claim.rs:733-737`). b697a3f7's uuid stage names make this accumulate. `list()` must skip stage files, and something must remove them. No issue says so; 7244ecb2 and 53d19851 REQ-02 are the natural homes.
- **Own stale lease.** Acquire returns the caller's own stale lease without renewing it (`:656-663`), noted in `dev/active/9db27a3a-v1-0/9db27a3a-progress.json:562`. The sequence keeps this. It is untracked here.
- **Epic plan row "Hook lease checks"** and DEC-09 hook mentions are superseded by 17fbcf25. Epic REQ-12 needs nothing from hooks now.

Cross-epic edges: 5b37a806 → a7314585 present, 5b37a806 still open. 083c9a91 → 614a6f97 present. 083c9a91's note "claim coordination before the operation session" is false for recover today and becomes true once a7314585 reclassifies it.

### Uncovered consumers

Sweep of `claims.index`, `claims.jsonl`, `ClaimsIndex`, `index_rebuilt`, claims-index wording and lease-id command forms across crates, scripts, `.agents/`, web, server, mcp-server, profiles and tests. **Little drifted here.** web, `crates/server`, mcp-server and profiles hold no reference to the store; `mcp-server/curated-tools.json` names tools only.

| Consumer | Status |
|---|---|
| `tests/cli_repo_workflow/external_data_root_tests.rs:95` (new, c4e07f6d, e3bb042ae) | Asserts `jit/claims.jsonl` exists. Survives the flip. Should go through the helper by the contract's wording; no issue lists it. |
| `tests/cli_repo_workflow/lease_enforcement_tests.rs:85-91`, `:536` (new, 17fbcf25) | `{lease}`, `{w-lease}` argv. Not in 614a6f97. |
| `refuse_foreign_leases_read` (`commands/mod.rs:831`, new, 17fbcf25) and its callers `bulk_update.rs:247`, `gate.rs:1196,1231`, `template.rs:189` | Caught only by c3e3e2ff's general REQ-01. Name it. |
| `ClaimsIndex::conflicting_lease`, `ClaimsMutationGuard::conflicting_lease` (new, 17fbcf25) | Not named by 15009590 or 98184c64. Tests `:2622,2773,2799` move with them. |
| `main.rs:7148`, `docs/reference/claim.md:367` | Lease-id remedy; not in 614a6f97. |
| `docs/how-to/dependency-management.md:237`, `docs/concepts/guarantees.md:555,562`, `dev/TESTING.md:585` | Not in f0a5b90a. |
| `test_mode.rs`, `test_cli.rs:186,398` | See 98184c64. |
| `.agents/skills/jit-manage/SKILL.md:173` | Covered by f0a5b90a. |
| `.agents/skills/jit-project-lead/evals/transcripts/*` | Recorded transcripts naming `.git/jit/locks/`; lock path unchanged, no action. |
| `dev/active/df129d9e-rust-code-quality/handoff-3.md:41`, `dev/active/9876a23a-one-validation-report/*` | Planning material; the bridge has a named remover. No action. |

### Overlap with open epics

**f4d9502a.** Ordered correctly: 614a6f97 → f6eaba42 (`claims-head-lookup`), f649c144 (`lease-validation-lists-once`); c3e3e2ff → e3c38fea. `get_current_branch` in the coordinator (`:906`), `claim.rs:47`, and `git config user.name` (`claim.rs:295`) are its targets; lease issues leave them alone. No conflict. f649c144's "once per lease" premise may already be stale (`validate.rs:2258-2262` reads `recorded_worktree_ids` once); not verified, belongs to that epic.

**b697a3f7.** 7244ecb2 → 1b7f58ae present (lease writes through `publish_file`); the DEC-21 ordering fits. c2b3cefa (`lease-check-in-session`) reaches c3e3e2ff transitively, but its body and d3c94dc5's scope predate 17fbcf25: it cites `captured_lease_warnings`, `require_active_lease` and a "missing-lease" refusal, while the shipped rule refuses only another agent's lease. That epic needs its own amendment, and c3e3e2ff should land first as ordered. It does not cover `claim list`/`status` under D-22, and neither epic owns that decision.

### Required amendments, in execution order

1. **Owner ruling:** `claim status`/`list` under the lease lock versus D-22 (dd53527f, epic note on d7d0d0db).
2. a7314585: add recover's reclassification to claims-first with the failure mapping and classifier test; pin down 5b37a806 REQ-04's log.
3. cc6ef886, 2f563673, plan `lease-store-test-helper`: re-home the helper to the test-support surface; drop the 627fdcc4 edge note.
4. f0a5b90a: remove the hook clause and REQ-02's hook half; add the missing pages.
5. dd53527f or 614a6f97: move `execute_claim_release_by_issue` to one `LeaseLock` with direct load.
6. 614a6f97: add `main.rs:7148`, the two test sites, and the deleted-issue eviction route.
7. c3e3e2ff: drop met or obsolete text; scope REQ-01 to exclude `validate_claims_index_with_paths`.
8. 15009590, 915be53e: refresh backgrounds and carry the stale and anonymous rules.
9. 53d19851: correct REQ-02's pass list; list the `warnings` removals.
10. 98184c64: add JIT_TEST_MODE removal.
11. 7244ecb2 or 53d19851: stage-file handling for `list()` and recover.

## Drift audit: story `history-boundary` (f8a6b298) at 804acc4d6

Method: static reading of code, issues, plan and authority, plus one ripgrep experiment on a throwaway corpus. No build or test was run. Line numbers below are current. Moved citations whose symbol survives are not listed.

Also read, grep only: `.jit/events.jsonl` in `~/Projects/gf2`, `mealie-fi`, `inflected-messages`, because 2a504b17 makes them part of the format cutover.

### Verdict

Needs issue-body amendments, three new or rewired edges, and one owner decision. It does not need replanning: the graph shape, the archive-evidence chain, the committed-length chain and the redo-record chain all still fit the code.

Class counts (24 issues): SOUND 10, CONTRADICTED 7, GAP 6, PARTLY-DONE 1, DONE-ALREADY 0.

Two landed changes cause most of the drift:

1. **c47d78c8 (451929125, 2026-11-01)** made an unknown event tag an error and dropped retired-tag lines from the logs. That is the option DEC-15 rejected.
2. **0a6fd04e (fa75003ac) with 952ed1e7 (c36c6a393)** made whole-repository `jit validate` and the built-in `repository_validation` checker parse the whole event log and read every gate-run record, as a hard requirement. `record_fidelity.rs` did not exist when the plan was written.

### Owner decision required

Is whole-repository validation a history-reporting command?

- Conflict: epic REQ-13, @/charter/D-19 ("read only by commands that report history") and DEC-19's list, against 0a6fd04e REQ-02 and REQ-04 ("Whole-repository validation reports an event id that appears on more than one line of the event log"; `merge=union` creates such lines silently) and 952ed1e7 REQ-03.
- Code: `validation/record_fidelity.rs:50-52,107-137`; `commands/validate.rs:946-959` ("Whole-repository validation pays this read"); `commands/gate_check.rs:1074-1075`; documented at `docs/reference/worktree-validate.md:341-344`.
- Recommendation: name whole-repository `jit validate` and the `repository_validation` checker as history readers in DEC-19 and REQ-13. Duplicate-id detection has no other home and `merge=union` keeps producing the risk. Keep every other validation capture on the probe.
- Consequence to design: `capture_record_fidelity_validation` is already a separate capture. Moving the log read there lets the base capture go to tail, but 725af076 then refuses that whole read through `CaptureSpec`. It needs a named admitted read path, such as raw bytes through storage, or an explicit carve-out in 725af076 REQ-01.
- Alternative: move record fidelity of the log to `jit events`. That requires amending 0a6fd04e's landed criteria.

### Per-issue classification

| Id | Key | Class | Evidence |
|---|---|---|---|
| f9bf70a5 | archive-execution-record-type | SOUND | Variant at `domain/types.rs:1304`; folds at `domain/artifact_discovery.rs:184`, `repository_state/archive.rs:554`, `commands/archive.rs:109` all still take events. `EventBody` is internally tagged at `types.rs:1222`. Catalog has 6 sites, not 7. |
| db9bcf12 | archive-evidence-writes | SOUND | `finalize_archive_execution` `repository_state/archive.rs:21`; `capture_archive_base` `commands/validate.rs:296`; `PlanTarget::{Container,Document}` `domain/artifact_plan.rs:78`. `tracker_written_paths` is now `path.rs:628`, built from `RECORDS` plus a chain, with tests at `path.rs:975,998` and helper `gate_execution.rs:1022`. The named test citation `gate_execution.rs:1122-1135` is gone; the obligation stands. |
| 7fc08dff | archive-execute-reads-evidence | SOUND | Only callers of `captured_archive_events` are `repository_state/archive.rs:31` and `commands/archive.rs:776`; error variant `archive.rs:544`. Archive purpose captures whole because `mutation` is false at `validate.rs:335,367-370`. |
| 5e1ba005 | archive-preview-reads-evidence | SOUND | Reader call `commands/archive.rs:282`. The new `IssueStore` method is deleted later by 65aafaba REQ-06, which already depends on d0d73bef. This is planned churn, not conflict. |
| c2930451 | committed-length-append | SOUND | `compose_events` `mutation.rs:~597`, `captured_log_end` `:611-626`, refusal `:1151-1158`; both backends truncate (`redo_apply.rs:308-322`, `memory.rs:692-712`); overlay `overlay.rs:145-150`. The test it inverts, `commands/profile.rs:3683` (`..does_not_mark_valid_unterminated_event_as_torn`), shows a complete valid unterminated record is preserved today and will be overwritten. DEC-21 chose this; a worker should not be surprised by it. |
| b32a720e | redo-record-refuses-log-rewrite | SOUND | `RedoRecord::new` checks only overlap (`redo_record.rs:115-125`); delta rule at `image.rs:1441`, test `image.rs:2171`. |
| 9f610536 | log-rewrite-publish-branch-removal | SOUND | Branch at `repository_state_store.rs:946-948`. |
| 4466395d | history-isolation-test | SOUND | Target `tests/cli_repo_workflow/` exists. `.git/jit/claims.jsonl` is still the log (`claim_coordinator.rs:856`). Exec-checker evaluation does not reach record fidelity. Edge to 764e6068 must move with that issue's rejection. |
| ba4c6054 | gate-and-search-command-docs | SOUND | Skill globs still at both cited lines. `source_run` now at `custom-gates.md:217`, `cli-commands.md:1975`. `gate status <id> <gate> --all` is valid per `--help`. Note: `cli-commands.md:3533` positively says search covers "the event log"; REQ-03's edit must remove that sentence as well as add the exclusion. |
| 09ca911e | storage-records-per-key-reference | SOUND | Generator `scripts/generate-storage-records-reference.sh`, source `storage/reference.rs:471`; page has the three sections named. |
| 5b0e3d63 | validation-skips-event-log | CONTRADICTED | Quote: "`jit validate` and broad built-in prechecks never read the whole log" and "Repository validation no longer parses the event log." `record_drift` calls `image.file_bytes(&VirtualPath::EVENTS)?` (`record_fidelity.rs:50-52`), which returns `TailOnlyCapture` on a tail (`image.rs:924`) and propagates through `validate.rs:959`. As written, every whole-repository `jit validate` errors. Also flip `validate.rs:3408`. |
| cac57c93 | validation-event-count-removal | CONTRADICTED | `event_count` removal is valid (`validation/repository.rs:59,418-429`; no reader outside the module). Quote: "the event-log parse in repository validation no longer exist[s]". A second parse landed at `record_fidelity.rs:110,127`, inside the record-fidelity validation pass. Its comment (`:15-16`) assigns unparseable input to the records pass, so removing that parse leaves an unparseable log reported by nothing, since `event_log_drift` returns empty at `:110-113`. |
| 84e1e27a | event-log-read-rule | CONTRADICTED | Quote: REQ-01 "skips blank lines and records with a retired tag". Code: `event_log.rs:75-80` returns `UnknownTag`; conformance `test_read_events_rejects_unknown_tags_and_malformed_records` (`issue_store_conformance.rs:500`); c47d78c8 REQ-08 and @/inv/canonical-cutover. Second defect: `event_log_drift` feeds each newline-stripped line to the parser (`record_fidelity.rs:127`). Under the new rule every such line is an ignored unterminated final line, which fails `test_event_log_drift_reports_a_line_that_does_not_reproduce` (`:258`). The issue does not mention this. |
| 764e6068 | history-commands-skip-retired-tags | CONTRADICTED | Whole issue. Quote: "plants a well-formed retired-tag record ... shows that `jit events tail` and `jit events query` succeed". Parser rejects it. Quote: "This repository's log holds retired-tag records (`document_archived`)": 0 such lines today. Failure-lever arms already pin the error path (`tests/cli_issue/failure_lever_registry.toml:266-281`). Recommend rejecting it and rewiring 4466395d and ac8f07c2 to 84e1e27a. |
| 338d1920 | event-log-test-captures-tail | CONTRADICTED | Quote: "the tests' assertions are unchanged". `repository_state_store_tests.rs:865-880` requests tail then whole and asserts `file_bytes(&EVENTS) == log`. That cannot hold over a tail; the test must move to another path. "about 60 references" is now 26 plus `fixture.capture` callers. Symbols otherwise exist. |
| ac8f07c2 | storage-format-history-docs | CONTRADICTED | Quote: REQ-02 "states that no ordinary command reads one" with the DEC-19 reader list, which omits `jit validate`, the `repository_validation` checker and its gate-run read. False unless the owner decision is resolved. Edge on 764e6068 must move. `guarantees.md:635` now also states the claim-parse failure. |
| 7a8773ca | search-excludes-history | CONTRADICTED (Background only; REQs stand) | Quote: "anchored negations do not apply when the search path is absolute". d98b59261 made `search()` run with `current_dir(data_dir)` (`search.rs:145`). Tested on rg 15.2.0: anchored and unanchored negations both exclude all three paths, and an explicit glob still reaches a gate run. Anchored is now preferable because an unanchored `!tmp` also hides any nested `tmp`. DEC-18's rejected option has effectively landed, with absolute result paths kept. |
| 5582e39e | mutation-event-tail-captures | GAP | Missing edge. REQ-01 lists profile and re-init, but those paths capture through `capture_proposed_base` (`commands/profile.rs:941,968,990`; `commands/init.rs:83,117`), whose base forces whole at `validate.rs:369`, and `discover_tail` does not downgrade a whole request (`image.rs:603-606`). Their `validate_repository(&proposed)` calls (`profile.rs:1048`, `init.rs:156`) also reach `load_records` (`validation/repository.rs:419`). Needs an edge on 5b0e3d63 (which reaches cac57c93). The other listed captures exist; `events_action` errors on a tail today (`initialize.rs:729-739`), covered by REQ-01. |
| 725af076 | capture-refuses-whole-event-log | GAP | Executable, subject to the owner-decision carve-out. Nothing deletes what the refusal makes dead: `captured_log_end` whole arm `mutation.rs:624-626`, `repository_state_store.rs:955`, `overlay.rs:145-150`, `validate.rs:366-370`, the `EVENTS` entry `materialize.rs:235` with its filter `validate.rs:426`. @/charter/D-18, @/inv/canonical-cutover. Add a REQ here or one follow-on task. |
| 34dd339e | committed-length-restatements | GAP | Listed sites exist, except `repository_state_store_tests.rs:2817`, now an unrelated test. Unlisted restatements: `validate.rs:365-366`, `record_fidelity.rs:105-106` ("certified torn tail"), `repository_state/initialize.rs:16` ("torn-tail handling"). REQ-01 binds only listed sites, so these survive. |
| d0d73bef | archive-event-reader-removal | GAP | Reader and `is_torn_event_prefix` still at `json.rs:1176,1417`; also add conformance case `issue_store_conformance.rs:529,1226`. Quote: REQ-02 "No event-log parser exists outside `domain::parse_known_events`": `event_log_drift` parses log lines as raw `Value` for ids (`record_fidelity.rs:116-121`). Outside this issue's scope and not touched by any other. |
| 97f5f2b7 | torn-tail-marker-removal | GAP | Symbols all exist. REQ-02 tests only this repository, whose log has 0 `profile_lifecycle` lines. gf2's log has 13 lines carrying `"isolated_torn_tail":false`, none true. After removal those lines stop reproducing, so `event_log_drift` fails `jit validate` on gf2, and 0a6fd04e records 44 open gf2 issues requiring the built-in checker. Needs a gf2 log rewrite or a cutover task. |
| e0d8af37 | archive-evidence-seeding | GAP | Volatile: now 83 records over 58 targets (72 container, 11 document), max 3 per target; the issue says 74 over 50. REQ-03 is structural, so that is harmless. Real gap: gf2 has 36 `artifact_archive_executed` lines and no evidence seeding, so a re-archive there silently loses coverage and residue identities. Part of the format cutover below. |
| dc5fa7f8 | history-boundary (story) | PARTLY-DONE | REQ-01 "one treatment of retired tags" is met by 451929125 (error). The third parser is gone (47a082ef, `storage/memory.rs:479`). Background "three event-log parsers" is history. REQ-02, REQ-03, REQ-04 are open, and REQ-04 depends on the owner decision. |

### Contract findings

| Contract or decision | Fit | Finding |
|---|---|---|
| `event-log-rule` | Amend | Replace "A well-formed record whose tag is not a current `EventTag` (a retired tag) is skipped by every caller" with: an unknown tag is an error naming the line. The rest fits. `parse_known_events` uses `contents.lines()` (`event_log.rs:53`), which cannot see a terminator, so the implementation must change, and every caller passing a lone line needs checking. |
| `event-append-probe` | Fits, with exception | `EventLogTail` and `final_line` match @/issue/c88822a5/decision/D-1 (`image.rs:150-229`). "A non-history command reaches the event log only through `discover_tail`" holds only after the owner decision. |
| `archive-execution-record` | Fits | Fields match `types.rs:1304-1317`. |
| `archive-evidence-record` | Fits | Later moved from store method to image by 65aafaba, as planned. `record_drift` covers index, issues, gate runs and log only, so evidence and verdict records escape the fidelity pass that 0a6fd04e and 952ed1e7 made the guard. |
| `history-paths` | Fits | `EVENTS`, `GATE_RUNS` and `RECORD_SCRATCH_DIR` exist (`path.rs:12,376`). |
| DEC-15 | Superseded | Its rejected option landed. Its rationale is void: the logs were rewritten by 2a504b17 and b01208cb. Epic REQ-12 "no command compacts or rewrites" still holds, because those were one-off scripts. |
| DEC-18 | Stale rationale | See 7a8773ca. |
| DEC-19 | Incomplete | Still correct: `jit events`, `worktree store-divergence` (`worktree.rs:402-403`), `gate status --all/--limit`, server list (`crates/server/src/routes.rs:981`). Missing: `jit validate`, `repository_validation` checker. `snapshot export` is not a reader (`snapshot.rs:449-480`). |
| DEC-21 vs D-17, D-22 | Minor conflict | D-17 and D-22 say "the event log's length counts the repository's commits". An overwrite of an uncommitted tail can leave the length equal or shorter. eaca3f52 is safe because admission compares the whole tail. 578ed10d REQ-06's "did not grow" wording and the charter sentence are imprecise in this edge. An unterminated tail is reachable only through a foreign writer, since a crash mid-apply leaves a record the next session completes. |
| Format version | GAP, epic level | `docs/reference/storage-format.md:443`: "The marker changes whenever an on-disk layout or interpretation changes, and a binary accepts exactly one format." The plan's landing groups assume "An older binary reads the new tree as having no runs, leases or evidence", and no issue bumps `schema_version` or crosses the other repositories. Precedent: 2a504b17 and `dev/archive/3d9a0ce3-lead-friction/active/format3-window.sh`. Needs one cutover task covering this repository, gf2, mealie-fi, inflected-messages and research: format bump, gate-run relayout, evidence seeding, `isolated_torn_tail` line rewrite. mealie-fi and inflected-messages have no profile or archive lines. research was not inspected. Gate-run and lease parts overlap audit-gate-runs and the lease story. |

### Event-log and history consumers no issue covers

| Consumer | Status |
|---|---|
| `validation/record_fidelity.rs:50-137` | Uncovered. Whole-log read, double parse, raw id scan. Blocks 5b0e3d63, cac57c93, 84e1e27a, d0d73bef. |
| `commands/validate.rs:946-959` | Uncovered. Captures every gate-run record for `jit validate`; gate-run story should confirm. |
| `commands/gate_check.rs:1074-1075` | Uncovered. Built-in gate reaches the same computation. |
| `storage/json.rs:1170` (`read_events` takes `.events.lock` shared) | Uncovered, D-22. `jit events` waits on writers, and b697a3f7 REQ-21 does not list it. |
| `docs/reference/worktree-validate.md:341-344` | Uncovered. States event-log fidelity; must follow the owner decision. |
| gf2 `.jit/events.jsonl` | Uncovered. See 97f5f2b7, e0d8af37. |
| `scripts/benchmark-mutation-cost.sh:42,118` | Harness reading its own fixture log; no action. |
| `.agents/skills/jit-execution-lead/scripts/test-land.sh` | Test reads through `git show`; no action. |
| `scripts/migrate_timestamps.py`, `scripts/fix_timestamps.py` | Legacy one-offs reading the log; not runtime. Removal candidates under D-18, no epic impact. |
| `crates/server/src/watcher.rs:92` | Watches `gate-runs`, ignores the log; reads nothing. Per-issue layout still matches. |
| web `api/client.ts:123,128`, mcp-server, `profiles/jit-dogfood/manifest.toml:325` | API only, none, and prose only. No action. |

Failure-lever fixture `tests/cli_issue/failure_probe_fixture.rs:382` writes `{\n`, a terminated line, so those arms survive the new torn rule.

`.agents/skills/jit-manage/SKILL.md:173` names `claims.index.json`. That is lease-story scope; I did not check whether `lease-operator-docs` lists it.

### Overlap with open epics

- **b697a3f7, ordering:** c517b88b and c2930451/97f5f2b7 all edit `compose_event_action`, with no edge between them. c517b88b's Background and Note ("refused with `UncertifiedTornTail`") become false once c2930451 lands. Add an edge c517b88b → 97f5f2b7 or amend the note.
- **b697a3f7, consistent:** 65aafaba → d0d73bef (DEC-03) is present and removes 5e1ba005's method. 2f12d920 → 725af076 is present. c517b88b REQ-03 asks for the tail, compatible with 725af076. Its new captures on export and package tree need no entry in 338d1920.
- **b697a3f7, shared function:** 578ed10d moves `capture_validation_report` onto the read driver, the same code as the owner decision. Whichever lands second rebases. A lock-free validate that keeps a whole-log read is still admitted by tail compare.
- **f4d9502a:** no semantic overlap found. Shares files only (`commands/validate.rs`, `commands/worktree.rs`). I read the first 3,500 characters of the epic body and did not read its child issues. 4334df62 touches `read_exact_store`; not audited.

### Amendment list

1. Decide the owner question; amend DEC-19 and REQ-13 accordingly.
2. 764e6068: reject. Rewire 4466395d and ac8f07c2 to 84e1e27a.
3. 84e1e27a: drop retired-tag skip, require unknown tag stays an error, add `record_fidelity.rs:127` to scope.
4. `event-log-rule`, DEC-15, epic REQ-10, dc5fa7f8 REQ-01: reword to the landed treatment.
5. cac57c93, 5b0e3d63: restate against `record_fidelity.rs` and the decision.
6. 5582e39e: add edge on 5b0e3d63.
7. 338d1920: except the tail-to-whole test at `repository_state_store_tests.rs:865-880`.
8. 725af076: carve-out if needed; delete dead whole-log arms.
9. 34dd339e: add the three unlisted sites.
10. d0d73bef REQ-02: reconcile with the id scan.
11. 97f5f2b7, e0d8af37: add a format-bump and multi-repository cutover task.
12. 7a8773ca: correct Background; allow anchored globs.
13. c517b88b: edge or note.

