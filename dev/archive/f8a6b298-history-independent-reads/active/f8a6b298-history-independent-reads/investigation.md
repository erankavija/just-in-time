# Investigation: Restructure storage so reads never scale with project history (f8a6b298)

> Grounding for planning node 5e615dc2. Three scoped investigations against `main` after the 2195c98f breakdown was created (2195c98f manifest keys and issue ids are cited where this epic consumes its types).

## Part: Gate-run storage

## f8a6b298 investigation: gate-run storage

Scope: REQ-01..04, gate-run parts of REQ-12..15. Code read at HEAD `dd6af8f2a` (2026-10-21). Other sessions are editing; `repository_state/path.rs` moved under ae2e3739 (commit `5066bec26`) during this read.

### 1. Claim classification

| Claim / REQ | Verdict | Current evidence |
| --- | --- | --- |
| Background: `gate-runs/<run-id>/result.json` is flat; an issue's runs are found by parsing every run and matching `issue_id` | valid-and-open | `storage/json.rs:957-1040` (`read_gate_runs`, one enumeration of the root, parses each record), `json.rs:1474-1479` (`list_gate_runs_for_issue` filters by `issue_id`). Memory twin: `storage/memory.rs:766-800`, `:934-936`. Path builder `repository_state/mutation.rs:512-520`. |
| Per-issue lookup and `(gate, digest)` reuse lookup both enumerate all runs | valid-and-open | `json.rs:1481-1488`, `memory.rs:938-946` (`find_reusable_gate_run` = `latest_gate_run(read_gate_runs(is_reusable_for))`). Reuse is read through `self.storage`, outside the mutation image (`commands/gate_check.rs:1043-1049`, called at `:928`). |
| Gate evaluation and gate-checker evidence capture put every run record into the image, then filter by issue (`gate_check.rs:273-300`, `mod.rs:660-675`, `mod.rs:1971-1992`) | valid-and-open, qualified | Only for checkers that consume run history: `checker_consumes_run_history` = `Exec { pass_context: true }` (`commands/mod.rs:589-597`). Then `discover_gate_run_results` (now `commands/mod.rs:2048-2069`, not 1971) lists the root and captures every run leaf; `captured_gate_runs` (`gate_check.rs:273-318`) parses all and filters by issue; `captured_precheck_plan` (`mod.rs:622-680`, parse/filter at `:661-677`) likewise. Other checkers capture only the `GATE_RUNS` directory entry and the pre-derived new-run paths (`gate_check.rs:871-880`, `:759-768`). |
| Validation's precheck-history capture does the same (`validate.rs:466`) | valid-and-open | `commands/validate.rs:465-467` calls `discover_gate_run_results` when `discover_issue_closure` (`validate.rs:1164-1215`) finds a pass-context precheck on the target. |
| `issue show` 98 ms with runs, 15 ms without; `issue status` on 20 ids 1.4 s | valid (reproduced, partially) | Measured now: `jit issue show f8a6b298 --json` 0.096 s; `jit issue status <20 ids> --json` 1.65 s. The 15 ms figure was not reproduced (needs a copy without runs). Cause: every show/status builder calls `list_gate_runs` = full scan per issue: `main.rs:1740` (`build_issue_show_response`, used by `issue status` at `:3253,3268`), `:3046` (`issue create --json`), `:3305`, `:3356`, `commands/issue.rs:308` (`issue_status_response`). |
| `docs/reference/storage-records.md` documents the flat layout as deliberate | valid-and-open | Generated from `storage/reference.rs:184-188` ("`gate-runs/` is flat rather than nested per issue") and `:549-556`; rendered at `storage-records.md:60-65,126`. |
| Repository size: 4,638 runs, 47 MB | valid, grown | Now 4,698 run dirs, 47 MB on disk, 16.7 MB of record bytes; 976 issues own runs; max 40 runs per issue, max 29 per (issue, gate), median 1; 584 runs carry a digest, 123 are reused, 395 distinct `(gate, digest)` keys; every run's issue file exists; every record has `schema_version` 1 and `run_id` == directory name; largest record 378 KB. 92 run ids are cited in `dev/`, `docs/` or issue records. |
| REQ-01 one issue's runs open only that issue's records | valid-and-open | All readers above enumerate the root. |
| REQ-02 latest (issue, gate) found directly | valid-and-open | Latest is computed by scanning; five divergent selections exist: `domain/types.rs:1034-1040` (`latest_gate_run`, started_at then run_id), `gate_check.rs:1247-1262` (`get_last_gate_run`, stable sort, last wins on tie), `gate_check.rs:1384-1398` (`>=` keeps first on tie), `output.rs:1715-1718` (`GateView::build`, `max_by_key`), `gate_check.rs:257-271` (`compact_run_history_for_context`). |
| REQ-03 reusable verdict found from (gate, digest) | valid-and-open | See reuse row. |
| REQ-04 image holds only the evaluated issue's runs | valid-and-open | See capture rows. Capture budget is 512 MB (`commands/mod.rs:235`); this repository is at about 1/11 of it, so the full-history capture is also a future hard failure, not only a cost. |
| REQ-12 (gate-run part) no file derived from history; no compaction | currently holds for gate runs | No gate-run index or pointer exists today. The proposed design must not add one that a strict reading calls derived (section 5). |
| REQ-13 (gate-run part) only history-reporting commands read gate-run history | valid-and-open | `issue show`, `issue status`, `issue create --json`, `issue children` (via `issue_status_response`), gate evaluation, transitions with pass-context prechecks, and `validate` with a precheck target all scan. History-reporting gate-run readers: `gate status --all/--limit` (`main.rs:4660-4700`), server `GET /issues/:id/gate-runs` (`crates/server/src/routes.rs:892-906`), web `IssueDetail.tsx:150,168`. |
| REQ-14 (gate-run part) other issues' runs unparseable, commands still succeed | valid-and-open, and one test asserts the opposite | `crates/jit/tests/cli_gate/gate_cli_integration_test.rs:169-200` (`test_issue_show_errors_on_corrupt_gate_run`) plants a corrupt `gate-runs/corrupt-run/result.json` that belongs to no issue and requires `issue show` to fail. It must be rewritten to corrupt the shown issue's own record. Backend tests `json.rs:1932-1965,1986-2064` and `memory.rs:1075-1160` assert that `list_gate_runs_for_issue("any-issue")` fails on an unrelated corrupt run; same fate. |
| REQ-15 (gate-run part) docs describe per-key layout | valid-and-open | `docs/reference/storage-records.md:58-126` (generated, source `storage/reference.rs:175-188,462,549-556`), `docs/reference/storage-format.md:31,291-295`, `docs/concepts/overview.md:99`, rustdoc `domain/types.rs:925,929`, `commands/gate.rs:24`. |

Adjacent finding (event-log scope, affects REQ-13/14 for gate evaluation): `capture_gate_evaluation_image` requests `VirtualPath::EVENTS` whole (`gate_check.rs:764`), so every gate evaluation reads the 3.6 MB log twice; the finalizer needs only the tail (`repository_state/mutation.rs:639-648` accepts a tail). `discover_issue_closure` also requests it whole for broad built-in prechecks (`validate.rs:1197-1199`). Hand to the event-log investigator.

### 2. Prior art

- `docs/reference/storage-records.md` / `storage/reference.rs`: the flat layout and its rationale; regenerated by `scripts/generate-storage-records-reference.sh` (artifact `storage-records-reference`). The freshness test `storage::reference::tests::test_gate_run_fields_match_derived_schema` keys off `GateRunResult`'s schema; `test_render_projects_the_storage_result_path` (`reference.rs:794`) keys off `gate_run_result_relative_path`.
- `docs/reference/storage-format.md:291-295` defers to storage-records; tree at `:31`.
- D-17 redo record, `dev/archive/f1dc843a-redo-mutations/active/breakdown.json`: apply publishes one redo record over the files it writes; the pre-commit check compares only the write set and the log length and tail. Its single-round capture task (REQ-04 there) folded gate-run result capture into the one round, which is where the whole-root capture now lives.
- Audit: `findings-tables.md` row 2 proposes "runs per issue, a latest-per-(issue, gate) pointer and a (gate, digest) reuse pointer in the same redo record". Row 19 flags the hand-rolled no-follow reader in `json.rs:957-1043,1393` against `read_repository_file_nofollow` (`storage/repository_state_store.rs:1810`).
- ae2e3739 (landed `5066bec26`): `RepositoryLayout::tracker_written_paths` (`repository_state/path.rs:537-560`) excludes `ISSUES`, `EVENTS`, `GATE_RUNS`, `INDEX` and locks from every input digest. Any new tracker-written root must join this list or live under `GATE_RUNS`. This repository's `.jit/gates.toml:86,154,343,689` still excludes `.jit/gate-runs` by hand (5a3a886f retires that).

### 3. Consumer sweep

Gate-run API, path or layout references (`gate_run|gate-runs|GateRun|GATE_RUNS|run_record`), counts per file:

- `crates/jit/src`: `commands/gate_check.rs` 153, `storage/reference.rs` 118, `storage/json.rs` 70, `main.rs` 61, `storage/memory.rs` 58, `storage/gate_runs.rs` 35 (tests only), `output.rs` 30, `gate_execution.rs` 24, `repository_state/mutation.rs` 23, `storage/repository_state_store_tests.rs` 22 (conformance `:2176,2380`), `commands/mod.rs` 15, `commands/gate.rs` 14, `domain/types.rs` 13, `storage/mod.rs` 8 (trait `:257-291`), `storage/errors.rs` 7, `repository_state/mod.rs` 3, `repository_state/path.rs` 3, `commands/issue.rs` 2, `commands/validate.rs` 1.
- `crates/jit/tests`: `cli_gate/bracket_coverage_gate_run_test.rs` 10, `fast_rules/local_rule_enforcement_tests.rs` 8 (IssueStore wrapper `:869-885`), `fast_docs_templates/template_apply_atomicity_tests.rs` 7 (IssueStore wrapper `:157-168`), `cli_gate/gate_cli_integration_test.rs` 6, `cli_gate/gate_evaluation_durability_test.rs` 4, `cli_gate/gate_evaluate_exit_code_test.rs` 3, `fast_docs_templates/template_apply_tests.rs` 2 (`:1131,1189`, exclusion path literal), `scratch_build/repository_state_cutover_tests.rs` 1, `cli_gate/main.rs` 1, `cli_gate/gate_evaluate_skip_at_head_test.rs` 1.
- `crates/server/src`: `routes.rs` 13 (`:61-62,887-922`; `get_gate_run` ignores the `:id` segment and loads by run id alone), `watcher.rs` 5 (`:76,99-101,314-315`; matches any `gate-runs` path component, survives nesting).
- `web/src`: `components/Issue/IssueDetail.tsx` 12, `api/client.ts` 5 (`:119-128`, both routes already carry the issue id), `types/models.ts` 2. No change needed if routes keep their shape.
- `mcp-server`, `test-vectors`: 0.
- `scripts/cargo-ci.sh:18` (comment), `contrib/gates/ai-review.sh:114` (comment), `dev/active/9c9ac88e/subsample-repo.py:11,20` (copies `gate-runs/` flat; dev tool).
- Docs: `docs/reference/storage-records.md` 3, `docs/reference/storage-format.md` 2, `docs/concepts/overview.md` 1, `dev/TESTING.md:556,577`.
- `GateVerdictOrigin::Reused(run_id)` (`domain/types.rs:1046-1063`) names a source run by id only; rendered at `main.rs:1663-1664`, documented at `storage/reference.rs:264`. Under per-issue addressing the id alone does not locate the record.

### 4. Primitive verification

| Property | Delivered by | Gap |
| --- | --- | --- |
| A per-key run record is written inside the mutation's one redo record | `finalize` pass 3 (`mutation.rs:1046-1096`) emits `CreateDirectory` + `WriteFile { expected: Absent }`; `redo_record_for` (`storage/repository_state_store.rs:1000-1012`) folds them into the record, and apply creates missing ancestors (`open_or_create_capability_parent`, `:1690-1700`). | Only the path changes. A nested `gate-runs/<issue>/` needs no explicit `CreateDirectory` because ancestors of puts are created on apply. |
| Interruption leaves prior or next | `@/inv/atomic-writes`, redo record replay on next session open. | None; holds for any file set in one plan. |
| A concurrent writer of the same key gets a retryable conflict | Write-set comparison against `ExpectedPreimage` (`@/inv/derived-state-coherence`); `classify_apply` retry in gate evaluation (`gate_check.rs:999`). | A rewritten key (issue record, any verdict record) must be captured so its preimage is known. |
| Run ids are time-ordered | No. `IdAuthority::uuid_at` (`mutation.rs:146-163`) hashes seed and index and stamps v4 bits. | Directory order is meaningless; "latest" needs a stored reference or a time-ordered name. |
| Command knows the run id before finalize | Pre-derived through `identifier_at` today (`gate_check.rs:871`, `commands/mod.rs:2867-2871`); 2195c98f `gate-run-ids` REQ-04 makes drafts carry it. | None after 79aa3aa7. |
| Path components are safe | Issue ids are UUIDs; `GateKey` lowercase-kebab (488eb3c1); `InputsDigest` 64 lowercase hex (`domain/repository_inputs.rs:357-399`). | None after 488eb3c1. |
| Every recorded run updates the issue's gate state in the same plan | Gate evaluation `gate_check.rs:982-997`; prechecks `commands/mod.rs:2950-2962` with plans at `:2975-2990` and `:3191-3210`. | Manual attestations write state with no run (8673dc5e REQ-04 adds attestation run records). |
| Capture cost per key | `CaptureSpec` exact paths are unbounded; listings count against `max_listings` 256 (`validate.rs:915-920`). | Per-issue listing costs one listing. |

### 5. Design options

#### 5a. Run record placement and the latest run (REQ-01, REQ-02)

- **A. `gate-runs/<issue-id>/<run-id>.json`, and `GateState` names its run.** Add `last_run: Option<GateRunId>` to `GateState` (`domain/types.rs:400-408`), set by `gate_state_from_run` (`gate_check.rs:229-250`) in the same plan that records the run. Latest = issue record (already read) plus one file. History = one listing of the issue's directory. Load by `(issue, run)` is direct, which is what the server route already passes. Adds no file (D-18). Removes the five latest selections. Captures for pass-context checkers need one path per gate instead of a listing.
- **B. `gate-runs/<issue-id>/<gate-key>/<started_at>-<run-id>.json`.** Latest = max name in one per-(issue, gate) listing. No reference to maintain, but the name duplicates `started_at` (two sources), a load by run id needs a listing, and each evaluation spends one listing.
- **C. A per-(issue, gate) `latest.json` pointer next to the history.** A separate file that must stay coherent with the history: the rejected option of D-19. Not recommended.

Recommendation: **A**. On REQ-12, `GateState.last_run` is the issue's own state written by the mutation that produced it. Nothing rebuilds it from the run directory. It names a record rather than summarising history. Behavioural edges for the plan: a gate with no `gates_status` entry reports no latest run, although today `get_last_gate_run` would find one; a manual status write must state whether it keeps or clears `last_run`, and this aligns with 8673dc5e REQ-04.

#### 5b. Reuse key (REQ-03)

- **R1. Verdict record `gate-runs/verdicts/<gate-key>/<digest>.json` holding the source run's address `{issue_id, run_id}`.** An executed run with status passed or failed and a digest writes or replaces it in the same redo record (expected preimage = captured). Reuse = one read plus one run read. Placing it under `gate-runs/` keeps it inside `tracker_written_paths` (`path.rs:541`) and this repository's hand exclusions. A separate root `gate-verdicts/` would need a `path.rs` edit and four `gates.toml` edits.
- **R2. Store executed digest-bearing runs at their (gate, digest) home and have issue history reference them.** This breaks REQ-01 locality and splits one record type across two homes. Rejected.
- **R3. Verdict record holding a full copy of the run.** Duplicates up to 378 KB of stdout. Rejected.

Recommendation: **R1, latest recorded wins**. That matches today's latest-by-start selection, and a forced re-run supersedes the old verdict. To make R1 authoritative rather than derived, reuse consults only the verdict record, so a run that no verdict record names is not a reuse source by definition. The run-history directory is never scanned for reuse.

Coupled change: `GateVerdictOrigin::Reused` should carry the source issue as well as the run, `{source_issue, source_run}`, so provenance resolves directly. This changes the wire shape that 79aa3aa7 is typing.

#### 5c. The 4,698 existing runs (canonical cutover, no migration reader)

- **Relayout (recommended).** A one-off script under `dev/active/f8a6b298/`, run once in the cutover commit and removed when the epic closes, does three things. It moves each `gate-runs/<run>/result.json` to `gate-runs/<issue>/<run>.json` byte for byte. It backfills `last_run` on each issue's `gates_status` entries from the max `(started_at, run_id)`. It writes the 395 verdict records. Issue records use the canonical serializer: sorted keys, pretty-printed, no trailing newline (`mutation.rs:576-597`); Python `json.dumps(indent=2, sort_keys=True, ensure_ascii=False)` reproduces it, verified by `jit validate` and by a byte-diff of the untouched fields. This keeps the 92 cited run ids resolvable, `gate status --all` history, and the reuse cache.
- **Delete.** `git rm -r .jit/gate-runs` loses the history views, the cited evidence, `last_run_at` in `issue show`, and the 395 reuse entries, so the first evaluations re-run checkers. Readiness is unaffected because `gates_status` is authoritative.
- **Hazard either way:** the dogfood binary. An old installed `jit` reading the new tree treats `<issue-id>/` as a run directory, finds no `result.json`, and silently reports no runs (`json.rs:1001`). Its gate evaluations write old-layout records into the new tree. The data commit and `./scripts/install-jit.sh` must land together, and leads in other worktrees must reinstall.

### 6. Architecture fit / invariant check

- Layering: the layout lives in `repository_state` (path constructor, finalize) and `storage` (readers). The `GateState` field is domain data; commands only choose paths to capture. This respects `@/inv/architecture-dependency-direction`.
- `@/inv/convention-convergence`: the JSON and memory readers duplicate no-follow logic (`json.rs:957-1040,1393-1471` vs `memory.rs:343-380,766-800,909-932`). The new reader should go through `read_repository_file_nofollow` and one shared listing primitive, with one backend conformance suite replacing the paired tests in `json.rs:1932-2105` and `memory.rs:1075-1160` (`@/inv/shared-test-contracts`). The five latest selections collapse to `GateState.last_run`.
- `@/inv/derived-state-coherence`: the run record, issue record (`last_run`), verdict record and event commit in one redo record. The verdict record's preimage must be in the record attempt's capture. That attempt is the second session (`gate_check.rs:944-1004`), and the digest is known by then. Moving the reuse read into that image closes today's out-of-image `self.storage` read.
- `@/inv/domain-agnostic`: path components come from ids, gate keys and digests. No vocabulary is added.
- `@/inv/canonical-cutover`: there is no dual-layout reader; old-shape records become unreachable clutter. `captured_gate_run_result_paths` (`mutation.rs:524-549`) and `gate_run_result_relative_path` are removed.
- `@/charter/D-20`: `issue show`, `issue status`, `issue create --json` and `issue children` build through five copies of the show response (`main.rs:1731,3046,3305,3356`, `commands/issue.rs:308`). Each must stop calling `list_gate_runs` and use the latest-run accessor. Converging them is outside this scope but touches the same lines; library-handler issues 10751360 and f87f9b00 own that.

### 7. Sequencing

Upstream 2195c98f keys (issue ids) that must land first:
- `gate-run-ids` (79aa3aa7): `GateRunId`, drafts carrying the id, and `VirtualPath::gate_run_record`. This epic changes that constructor to take `(&IssueId, &GateRunId)`, so record it as a downstream edit, not a conflict.
- `gate-key-registry-type` (488eb3c1) and `gate-key-issue-fields` (417be079): `GateKey` as a path component and as the `gates_status` key.
- `issue-id-flip` (7abcc22d) and `issue-record-path-storage` (0102769f): `IssueId` as a path component and the record-path constructor idiom.
- `gate-run-actor` (faa19def) precedes 79aa3aa7 and needs nothing from this epic.

Other seams and recommended edges:
- 72d89161 (ready) removes `gate_passed_at_head`, one `get_last_gate_run` consumer (`commands/gate.rs:995-1010`), and `tree_dirty`. Edge: 72d89161 before step 2.
- 86d3625d REQ-03 removes `reusable_verdict` (`gate_check.rs:1043-1049`). Edge: before step 3, or fold it into step 3.
- 8673dc5e (attestation run records) after step 2. It then writes into the new layout and sets `last_run`.
- c998dfd3, fd9bfab8 (in progress), 39710b3b and e57095a4 touch run status, findings and history rendering, not layout. No edge needed; workers rebase.
- ae2e3739 is done; R1 under `gate-runs/` needs no change there.

Worker-sized steps, each compiling with clippy, fmt and tests green:

1. **`GateState` names its run.** Add `last_run`, set it from the draft's run id in gate evaluation and prechecks. Files: `domain/types.rs`, `commands/gate_check.rs`, `commands/mod.rs`, storage-records projection if issue fields are projected. Needs 79aa3aa7 and 417be079. Readers are unchanged.
2. **Per-issue layout cutover (REQ-01, REQ-04, gate-run REQ-15).** `VirtualPath::gate_run_record(&IssueId, &GateRunId)`; finalize writes `gate-runs/<issue>/<run>.json`. The trait becomes `list_gate_runs(&IssueId)` (one listing), `load_gate_run(&IssueId, &GateRunId)` and `latest_gate_run(&IssueId, &GateKey)` (via `last_run`), with one shared reader. Captures in `gate_check.rs:273-318,759-768,817-818`, `mod.rs:657-677,2048-2069,2865-2882` and `validate.rs:465-467` request only `gate-runs/<issue>/<last_run>.json` per pass-context gate. Show and status builders use the latest accessor, as do the server `get_gate_run` route and the reference projection and docs. Rewrite `test_issue_show_errors_on_corrupt_gate_run` and the backend clutter tests. Ships the relayout script and data commit plus a binary reinstall. Large: split readers and captures (2a) from show/status and latest consumers (2b) only if the trait keeps both `list` and `latest` in 2a.
3. **Reuse key (REQ-03).** Verdict record R1 written by finalize for executed passed or failed digest runs; `find_reusable_gate_run(&GateKey, &InputsDigest)` reads one key inside the record image; `Reused` carries the source issue; verdict backfill in the relayout script. Files: `repository_state/mutation.rs`, `storage/{mod,json,memory}.rs`, `commands/gate_check.rs`, `domain/types.rs`, `storage/reference.rs`, `main.rs:1663`.
4. **REQ-14 gate-run part.** The file-backed test plants unparseable `gate-runs/<other-issue>/*.json` and `gate-runs/verdicts/<other-gate>/*`, and shows that show, status, available, one-issue evaluation and claim succeed. Best owned jointly with the event-log and claims parts as one test.

## Part: Leases, the claims log and claim

## f8a6b298 investigation: leases, claims log, claim command

Scope: REQ-05, REQ-06, REQ-07, REQ-08; claims parts of REQ-12, REQ-13, REQ-14, REQ-15. Code read at HEAD `2e0c5ba72`. All paths are relative to the repository root.

### 1. Claim classification

Totals: 18 claims. 1 already-done, 16 valid-and-open, 1 invalid-as-stated.

| # | Claim | Class | Current evidence |
|---|---|---|---|
| C1 | Every append rescans `claims.jsonl` for the next sequence number (`:935-983`) | valid-open | `append_claim_op` `crates/jit/src/storage/claim_coordinator.rs:928-956` calls `get_next_seq` `:959-976`, which reads the whole log and parses every line. |
| C2 | Lease-enforced mutations and synchronized acquisition replay the full log under the claims lock | valid-open | Guard: `lock_claims_for_repository_mutation` `:413-432` → `rebuild_index_from_log_at(now, true)` `:425`. Acquire: `acquire_claim_synchronized` `:694`. Replay: `:1256-1346`. Recovery replays too (`commands/claim.rs:733-742`), which the claim did not list. |
| C3 | The storage reference calls `claims.index.json` a cache derived from the log | **invalid-as-stated** (partly) | The wording is present (`docs/reference/storage-format.md:390,398-400`), but the index is not a cache. It is a second authority. Renew `:1024`, release `:1095`, heartbeat `:833`, force-evict `:1150` and `get_active_leases` `:1201` read only the index, and the index is never reconciled with the log. Synchronized acquire `:694` and the mutation guard `:425` read only the log. Seven lock-free readers (§3) read only the index. A crash between the log append and the index write leaves the two disagreeing. Acquire then repairs it at `:713`, but renew, heartbeat and force-evict on that lease return `LeaseNotFound`. |
| C4 | `ClaimCoordinator` repeats lock-load-evict-append-write in eight operations | valid-open (undercounted) | The eight are `acquire_claim` `:450`, `acquire_claim_with_reason` `:527`, `acquire_claim_synchronized` `:659`, `heartbeat` `:824`, `renew_lease` `:1016`, `release_lease` `:1087`, `force_evict_lease` `:1142` and `get_active_leases` `:1188`. A ninth lock site is the guard `:413`. A tenth hand sequence is in `execute_git_recovery` (`commands/claim.rs:733-749`: verify, rebuild, write, load, evict, write). The steps differ between them: heartbeat, release and force-evict do not evict, and `renew_lease` does not `create_dir_all` the lock directory (`:1018`). |
| C5 | `acquire_claim` and `acquire_claim_with_reason` are called only from tests | valid-open | Production callers: 0. Test callers: `acquire_claim` has 40 in `claim_coordinator.rs`, 5 in proptests, 2 in `commands/claim.rs` tests (`:867,1733`), 2 in `commands/worktree.rs` tests (`:725-726`) and 1 in `commands/mod.rs` test (`:4412`). `acquire_claim_with_reason` has 8, all in coordinator tests. `release_lease` (`:1087`) is also test-only: `jit claim release` goes through `execute_claim_release_by_issue` → `get_active_leases` + `force_evict_lease` (`commands/claim.rs:372-395`). `check_issue_lease` (`commands/claim.rs:559`) has no caller; d759d606 deletes it. |
| C6 | The claim proptests exercise code production never runs | valid-open | `claim_coordinator_proptests.rs` uses `acquire_claim` 5×, `rebuild_index_from_log` 5× and `release_lease` 1×. The concurrency props go through `acquire_claim_when_reached` (`claim_coordinator.rs:1431-1460`) → `acquire_claim`. |
| C7 | `claim` captures and parses the whole event log to decide whether to emit `IssueClaimed` | valid-open | Capture: `commands/claim.rs:189-197` passes `VirtualPath::EVENTS` as a whole-file path. Parse: `captured_events` `repository_state/mutation.rs:674-688` → `domain::parse_known_events`. Decision: `event_tail_reflects_claim` `:693-708`, used in pass 2 `:851-888`. |
| C8 | Assignment has more than one derivation path, and they write different events | valid-open | See §5.3. |
| C9 | Active-lease reads persisting their evictions is tracked separately | **already-done** | d7d0d0db is **Done** (gates passed 2026-10-21; commit `14b223560`). `get_active_leases` writes the index after evicting (`:1199-1203`). Tests: `test_get_active_leases_persists_eviction_of_expired_lease_once` `:2031` and `..._without_expired_leases_appends_nothing` `:2061`. Local log: every lease evicted after the fix has exactly one auto-evict. **Residual leak:** `renew_lease` evicts (`:1025`) and then bails on not-found or not-owner (`:1028-1046`) without writing the index, so the eviction is logged again on the next read. The test-only `acquire_claim` has the same leak (`:460` → `:470`). |
| C10 | Sizes: 1,569 entries, 459 duplicate auto-evictions | valid (data moved) | Now 1,838 lines and 491 `auto-evict` records. Lease `b18eebd7` alone accounts for 291. `claims.index.json` holds 16 leases. |
| R05 | One issue's lease is read and changed without reading the log or other leases; sequence number without reading the log | valid-open | The index is a single file holding every lease (`:167-179`, `:210-225`). Sequence: C1. |
| R06 | One lock-load-change-publish sequence; no test-only API; proptests on the production path | valid-open | C4, C5, C6. |
| R07 | Interruption leaves prior-or-next; no rebuild from the log | valid-open | Every op appends to the log (`:928-956`) and then writes the index (`:877-895`), so an interruption between the two splits them. Rebuilds: `:425`, `:694`, `commands/claim.rs:738`. |
| R08 | `claim` reads no event-log content; one event in the same redo record; a repeat appends nothing | valid-open (partly done) | Already true: the event and the issue write come from one `finalize` call → one plan (`claim.rs:209`, `mutation.rs:852-887`), and a fully reflected claim is a no-op. Open: the event-log read (C7). |
| R12c | Nothing is derived from the claims log; nothing compacts it | valid-open | The index is rebuilt from the log and written by recover (`claim.rs:738-740`) and by synchronized acquire (`claim_coordinator.rs:694,713`). No code compacts the log. `docs/how-to/troubleshooting.md:527-532` tells operators to delete it. |
| R13c | Only history commands read the claims log | valid-open | Current readers: `get_next_seq` (every op), `rebuild_index_from_log_at` (guard, acquire, recover) and `verify_index_consistency` (`:1363-1372`, recover). After restructuring, **no command reads the claims log at all.** |
| R14c | File-backed test with an unparseable claims log | valid-open | No such test exists. Today an unparseable line fails replay at `:1280-1281` (`Failed to parse claim log entry`) and is silently skipped by `get_next_seq`. |
| R15c | References describe the per-key layout; the "derived cache" wording is gone | valid-open | See §2 and §3 (docs). |

### 2. Prior art

- `docs/reference/storage-format.md:377-409` documents the control plane: `claims.jsonl`, `claims.index.json` ("Derived cache", `:390,398`) and `locks/claims.lock`. `:58` names `claims.lock`.
- `docs/concepts/guarantees.md:115-122` covers the lock. `:416-418` gives a layout listing that says heartbeat "updates the index".
- `docs/how-to/troubleshooting.md:229` and `:515-532` say the index can always be rebuilt and give manual index and log deletion steps.
- `docs/how-to/multi-agent-coordination.md:326` has a "Rebuild claims index from log" step. `docs/tutorials/parallel-work-worktrees.md:214` has a diagram node.
- `docs/reference/cli-commands.md:3462` lists "claims index" among validation checks, and `:3539` says recover rebuilds the claims index.
- `docs/reference/claim.md:151` names `claims.jsonl` as the eviction audit trail.
- `scripts/hooks/README.md:75` names the index for strict-mode checks.
- Original design: `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md`, "Claim Acquisition Algorithm" (47 references; the module doc cites it at `claim_coordinator.rs:13`).
- D-17 redo work: `dev/archive/f1dc843a-redo-mutations/active/investigation.md:191,1108` records claims as **outside** the redo record, with one bridge (synchronized acquire → `repository_sync`). `:900` covers recover's rebuild.
- `RedoRecord` addresses "data-root and worktree targets alike and nothing outside them" (`storage/redo_record.rs:47-52`). `.git/jit` is outside those roots.
- 19e7298a (Done) REQ-02 deliberately fixed today's rule: "an issue assigned without its claim event emits exactly the missing one". The REQ-08 decision in §5.3 reverses that criterion on purpose.
- 2195c98f investigation `dev/active/2195c98f-semantic-types/investigation.md:120,152,185` says control-plane files have no `VirtualPath` home, and f8a6b298 ends replay.
- `.gitattributes:3` still declares `.jit/claims.jsonl merge=union`. The path is stale (the log lives in `.git/jit/`), and 7a60f987 removed it only from the init template (`tests/cli_repo_workflow/init_tests.rs:399-421`). `.git/jit/heartbeat/` is an empty leftover directory.

### 3. Consumer sweep

**Coordinator API → production callers** (all under `crates/jit/src/`):

- `acquire_claim_synchronized`: `commands/claim.rs:138`. Tests: `claim.rs` 3 sites, coordinator 5.
- `heartbeat`: `commands/claim.rs:248`.
- `renew_lease`: `commands/claim.rs:447`.
- `force_evict_lease`: `commands/claim.rs:381` (release-by-issue) and `:638`.
- `get_active_leases`: `commands/claim.rs:373` (release), `:502` (status), `:538` (list) and `:589` (dead `check_issue_lease`).
- `verify_index_consistency`, `rebuild_index_from_log`, `write_index_atomic`, `load_claims_index` and `evict_expired`: `commands/claim.rs:734-748` (`jit recover`).
- `lock_claims_for_repository_mutation`: only through `claims_mutation_guard` (`commands/mod.rs:720-739`). Its callers are `commands/mod.rs:2596` (field update), `:2820` (preliminary lifecycle), `:2885` (lifecycle), `commands/issue.rs:618`, `commands/dependency.rs:264,353` and `commands/gate.rs:1391`. Users of the guard: `captured_lease_warnings` (`mod.rs:741-776`) and `ensure_claim_available` (`mod.rs:778-800`). A coverage test is at `cli.rs:4110`.

**Lock-free readers of `claims.index.json`** (they bypass the claims lock):

| File:line | Purpose | Expiry rule |
|---|---|---|
| `commands/mod.rs:3343` `check_active_lease` | Legacy preflight behind `require_active_lease`, which is called at `gate.rs:726,770,816,1033` and `issue.rs:715,756` | `expires <= now` ⇒ inactive |
| `commands/template.rs:300` | Template lease warnings. This is a parallel copy of `captured_lease_warnings`, and template apply takes no claims guard. | `expires > now` |
| `commands/worktree.rs:252` | `worktree list` active-claim counts | `Lease::is_expired` |
| `commands/validate.rs:2488` | `validate --leases` | `expires_at < now` |
| `commands/validate.rs:2742` (`validate_claims_index_with_paths`) and `validation/repository.rs:542` | Schema and duplicate checks in repository validation | n/a |
| `scripts/hooks/pre-commit:95-102` and `scripts/hooks/pre-push:77-89` | Shell + jq reads of the index and its `stale` field, embedded through `include_str!` at `commands/hooks.rs:7-8`. pre-push matches by prefix (`startswith`). | stored `stale` |

There are four expiry predicates (`claim_coordinator.rs:92`, `mod.rs:3357-3361`, `template.rs:304`, `validate.rs:2496`), and they disagree at `now == expires_at`.

**Tests and fixtures touching the files:**

- `crates/jit/tests/cli_item_validate/validation_lease_tests.rs`: 15, including a `write_claims_index` fixture at `:112`.
- `crates/jit/tests/cli_repo_workflow/claim_integration_tests.rs`: 4 (`:51,177,302,768`).
- `tests/cli_issue/failure_lever_registry.toml`: 7, including the `malformed-claim-index` lever at `:74,84`.
- `tests/cli_issue/failure_probe_fixture.rs:637`, `tests/cli_issue/command_exit_code_projection_tests.rs:760` (2 refs) and `tests/cli_repo_workflow/worktree_cli_tests.rs:48`.
- `tests/cli_repo_workflow/init_tests.rs:419` (3 refs) and `tests/cli_repo_workflow/lock_contention_tests.rs:94` (the claims lock).
- `commands/hooks.rs:288` (`grant_lease` writes the index).

**Other consumers:**

- Server: no lease reader. `crates/server/src/watcher.rs:68,89,327` filters `claims.jsonl` and `claims/` names under the data root. That filter is stale, since claims live in `.git/jit`.
- MCP: none. Its tools are generated from the CLI schema, so a CLI argument change for renew, heartbeat or force-evict flows through `jit --schema`.
- web: none.
- `crates/jit/src/storage/warnings.rs:36` (`SequenceGap`), `DuplicateLease` and `IndexRebuilt` exist only for the index and replay.
- `test_mode.rs:2`.

**Docs:** `storage-format.md` 6, `guarantees.md` 4, `troubleshooting.md` 10, `claim.md` 1, `worktree-validate.md` 3, `cli-commands.md` 2, `multi-agent-coordination.md` 1, `parallel-work-worktrees.md` 1, `scripts/hooks/README.md` 1.

### 4. Primitive verification

| Property | Delivered today? | Evidence and what is needed |
|---|---|---|
| Atomic, durable per-key replace in `.git/jit` | Parts exist | `write_file_atomic_bytes` (`storage/atomic_write.rs:44,144-178`) gives visibility only: no fsync and no directory sync. `write_file_synced` `:55`, `create_file_synced` `:73` and `sync_directory` `:95` exist as `pub(in crate::storage)`, so the coordinator can compose temp-synced → rename → dir-sync. The coordinator's own `write_index_atomic` (`claim_coordinator.rs:877-895`) is a parallel variant with a fixed `.tmp` name and no directory sync, which is a convention-convergence defect. It goes away. |
| Lease state in the repository redo record | **No** | The redo record cannot address `.git/jit` (`redo_record.rs:47-52`), and `RedoRecord`'s single `EventAppend` is tied to the event log. |
| O(1) sequence number | **No** | `get_next_seq` `:959-976` is a full scan. Nothing else allocates a claims sequence. |
| Prior-or-next on interruption | **No** | Log append (`:944-953`) and index publish (`:877-895`) are two commit points. The fixed `claims.index.tmp` also races between processes, although the lock serializes writers. |
| One global serialization point | Yes | `locks/claims.lock` through `FileLocker::lock_exclusive_with_metadata`. It is taken at every op, and the guard holds it across the whole repository mutation (`mod.rs:2885` → session). Ordering is enforced by `CoordinationOrderGuard` (`:414,669`). |
| Interruption seam for lease ops | **No** | `MutationInterruptionInjector` (`storage/mutation_interruption.rs:13-33`) and `RedoInterruptionInjector` (`redo_apply.rs:33-59`) cover sessions and records only. A lease-point injector is needed for the REQ-07 matrix. Its shape can follow `RedoPoint`. |
| Event append without parsing the log | Yes | `CaptureSpec::discover_tail` (`repository_state/image.rs:668-676`) and `EventLogTail::read_tail` (`image.rs:177-193`, `storage/repository_state_store.rs:1515-1528`). Mutations already use them (`commands/validate.rs:413-416`), and `captured_log_end` (`mutation.rs:639-649`) accepts a `FileTail`. The tail read still reads the **final line** to apply the torn-tail rule (`mutation.rs:603-615`). |
| Lookup of a lease by issue key | Blocked by short-id tolerance | `resolved_active_leases` (`:294-319`) and `is_issue_identity_candidate` (`:806-810`) resolve persisted short spellings. 2195c98f `lease-issue-identity` (98404e0c) deletes them. After that, an issue id is a valid file-name key. |
| Lookup of a lease by lease id | n/a in a per-issue layout | Renew, heartbeat and force-evict take `lease_id` (`cli.rs` `ClaimCommands::Renew` and the following variants; `main.rs:8209,8251,8450`). See §5.2. |

### 5. Design options

#### 5.1 Layout and the single lease sequence

**Option A (recommended): one file per active lease, plus a sequence counter and history.**

- `.git/jit/leases/<issue-id>.json` holds that issue's current `Lease`. An absent file means no lease. The file name comes from a canonical `IssueId` through one path constructor on `WorktreePaths`; control-plane paths have no `VirtualPath` home (2195c98f D8).
- `.git/jit/claims.seq` is a single u64 counter, read and rewritten (synced replace) under the lock before each append.
- `.git/jit/claims.jsonl` becomes append-only history that no command reads.
- `claims.index.json` is deleted.
- `stale` is no longer stored. It is computed at read by `Lease::is_stale`, so `stale` leaves the serialized record.

Every lease operation then runs as one private sequence:

```text
with_lease(issue: &IssueId, op) ->
  lock claims.lock (+ CoordinationOrderGuard)
  load leases/<issue>.json          # one file; Absent | Present(lease)
  current = present && !expired(now) ? Some(lease) : None   # expiry = auto-evict of THIS key only
  change = op(current, now)?        # Keep | Put(lease) | Remove(reason)  (pure)
  if expired-present or change != Keep:
      publish lease file            # temp(synced) → rename → fsync dir; or unlink → fsync dir
      seq = bump claims.seq
      append history line(s)        # AutoEvict for the expired key, then the op's own record
  unlock
```

Commit point: the lease-file rename or unlink. An interruption before it leaves the prior lease; after it, the next lease. Nothing rebuilds from the log, and the lease state never depends on it.

What an interruption can do to history, by ordering:

- **State first (recommended):** a crash after the commit loses at most that op's history line. A retried acquire then finds its own lease and reuses it, as `:723-731` already does, so no duplicate is written.
- **History first:** a crash leaves a phantom line, and a retry appends a duplicate. That is the same failure class as the 491 auto-evicts.

How each operation maps onto the sequence:

| Operation | `op(current)` |
|---|---|
| acquire | `None` ⇒ `Put(new)`. Own lease (same agent and worktree) ⇒ `Keep` (convergent retry). Other holder ⇒ error. The TTL=0 reason check runs before the lock. |
| renew | Own ⇒ `Put` with new `expires_at`/`last_beat`. Absent ⇒ `LeaseNotFound`. Not owner ⇒ error. Expired ⇒ the eviction persists before the error, which closes the C9 leak by construction. |
| heartbeat | `Put` with new `last_beat`. Renew already does this for TTL=0 leases (`:1050-1057`), so the two can collapse into one (D-18). |
| release-by-issue and force-evict | `Remove(reason)`, with actor attribution in the history line. |
| auto-evict | The expired branch of the sequence. The listing read (`status`/`list`/`recover`) runs it once per expired key, which preserves d7d0d0db REQ-01..03 (one record per expired lease, persisted). |
| lease check in repository mutations | The guard holds the lock only. After the target resolves from the capture, it reads `leases/<target>.json` per target (`has_active_lease`, `conflicting_lease`). It writes nothing; expired counts as inactive. |
| recover | Lock cleanup, then the eviction pass over the listing. Verify and rebuild are deleted. |

Trade-offs of option A:

- Pros: per-key reads; one commit point; reuses the atomic-write primitives; deletes rebuild, verify, `SequenceGap`, `DuplicateLease` and `IndexRebuilt` (D-18).
- Cons: the history line is best-effort, at most one line lost per crash. `claims.seq` costs a second fsync per op.

**Option B: option A plus a control-plane redo record.**

- `.git/jit/lease.redo` carries the lease put or delete, the seq bump and the history line. The next lease op applies a leftover record, which makes lease state and history exactly consistent.
- Pros: the same commit discipline as D-17.
- Cons: `RedoRecord` and `RedoApplier` would have to gain a control-plane root class and a non-event append target, or a second record type would be needed. That is new mechanism serving only audit fidelity, against D-18.
- Choose B only if the claims history must be exact.

**Option C: keep one authoritative `leases.json` with no log replay. Rejected.** It is O(active leases), not O(history), so D-19 is met. But it reads every other issue's lease on each op, which violates REQ-05 literally.

**Sequence number alternatives.**

- (a) The counter file. Recommended.
- (b) The sequence is the log's byte length before the append, from `metadata().len()` under the lock. It needs no extra file and is monotonic because REQ-12 forbids rewrites. However, a torn previous append can merge into the next line unless the appender reads the last byte, and that byte is log content.
- (c) Drop `seq` and order by line position. This matches D-18 but makes REQ-05's sequence-number clause vacuous, which is risky under a literal review.

#### 5.2 Conflicts between REQ-05 and current features

1. **Indefinite-lease limits.** `max_indefinite_leases_per_agent` and `..._per_repo` (`config.rs:1339-1351`, defaults 2 and 10) are enforced at `:744-771` by counting **all** leases. A TTL=0 acquire therefore reads other issues' leases. Options:
   - (i) Amend REQ-05 to exempt a policy count over current leases. That count scales with active leases, not history, so D-19 holds.
   - (ii) Remove the limits (D-18). 5b37a806 then becomes the reclaim path for dead holders.
   - (iii) Keep a separate per-agent indefinite counter. That is a second file that must change atomically with the lease file, so it needs option B.
   - Recommendation: (i).
2. **Lease-id-addressed operations.** `claim renew`, `claim heartbeat` and `claim force-evict` take a lease id, so finding the key requires scanning all lease files. Options:
   - (i) Address these operations by issue id, as `claim release` already does (`cli.rs` Release: "WITHOUT requiring the lease UUID"). The lease id stays an ownership token in output. This changes the CLI, MCP schema and docs.
   - (ii) Keep an immutable `leases/by-id/<lease-id>` pointer, created before the lease file and treated as dangling when its lease does not match. It is not derived from history.
   - (iii) Accept the scan, which is O(active) and violates REQ-05 literally.
   - Recommendation: (i), which conflicts with 8cfbf19d REQ-04 ("Lease-id CLI arguments are parsed once into `LeaseId` at the boundary"). Under (i), f8a6b298 replaces those arguments after 8cfbf19d lands.
3. **Hooks.** Hooks read the index and its stored `stale` field through jq (§3). Once `stale` is computed at read time, jq cannot check it. Options:
   - (i) Hooks call `jit claim status --issue <id> --json`, giving one activeness computation (D-20).
   - (ii) Hooks read `leases/<id>.json` with jq and ignore staleness.
   - Recommendation: (i).

#### 5.3 REQ-08: assignment paths and the claim decision

Every path that writes `Issue.assignee`:

| Path | Code | Event written | Assignment without a matching claim event? |
|---|---|---|---|
| `issue assign`, `issue claim --assign-only` (`main.rs:3822`) and non-bulk `issue update --assignee` (`mod.rs:951`) | `CapturedIssueMutation::Assign` `mod.rs:1600-1611` | `IssueClaimed` only when the assignee changes | No |
| non-bulk `issue update --unassign` | `Unassign` `mod.rs:1613-1627` | **`IssueUpdated{fields:[assignee]}`**, not `IssueReleased` | **Yes, indirectly**: the event tail still ends at the earlier `IssueClaimed` |
| `issue update --bulk --assignee` / `--unassign` | `derive_field_update` `mod.rs:1340-1344,1503-1507` | `IssueClaimed` when it changes to `Some`; unassign writes only `issue_updated` | Same as unassign |
| `issue claim`, `issue claim-next` (`issue.rs:742,813`) | lifecycle `Claim` `mod.rs:3010-3040,3164-3172` | Different assignee ⇒ error. Same assignee and InProgress ⇒ no-op. Otherwise `IssueClaimed` is pushed **unconditionally**, including when a same-assignee Ready issue is promoted and the assignment does not change. | No |
| `issue release` | lifecycle `Release` `mod.rs:3174-3186` | `IssueReleased` when there was an assignee | n/a |
| `claim acquire` | `MutationIntent::ClaimIssue` `mutation.rs:851-888` | The issue is written when the assignee differs, **silently overwriting another assignee with no `IssueReleased`**. `IssueClaimed` is written when the tail does not already show this claim. | n/a |
| `claim release`, force-evict, renew, heartbeat | — | None. The issue stays assigned (`claim.rs:2015` test) | No |
| `issue create`, `batch-create`, template apply | Drafts come from `Issue::new` (`domain/types.rs:529`). `BatchIssueDef` has no assignee field (`batch_create.rs:68-97`), and `template.rs` sets none. | — | Cannot assign |
| archive and revive (`State::Archived` transitions) | `mod.rs:1066-1080,1248-1260` | `issue_state_changed`; the assignee is preserved | No new case |
| `jit archive` (artifact archive) | Does not touch the assignee | — | — |
| Out of band: hand edit, a git merge that brings an issue file without its events, a pre-D-17 torn write | — | — | **Yes** |

Reachable states where the issue is assigned to agent A without a matching claim event:

- **S-a: `claim acquire` A → `issue unassign` → `claim acquire` A.** Today `issue_reflects=false` and `event_reflects=true`, so the issue is reassigned and **no event is written**. That is a live `@/inv/event-log` gap.
- **S-b: out-of-band assignment.** Today the missing `IssueClaimed` is emitted, as 19e7298a REQ-02 specifies.

**Decision that lets `claim` read no event-log content:** the `ClaimIssue` pass emits `IssueClaimed` exactly when the captured issue's assignee differs from the claimant, and in the same plan as the issue write. Otherwise it emits nothing. The capture becomes `{issue record, events tail}` through `discover_tail`. `captured_events` and `event_tail_reflects_claim` are deleted, along with the claim-specific whole-log capture budget.

Behaviour that differs from today:

1. **S-a** now emits one event. This fixes an invariant gap.
2. **S-b** no longer emits a repair event. 19e7298a REQ-02's second clause is retired. Out-of-band assignment is outside the event-log invariant's reach, and canonical-cutover rules out a compatibility repair.
3. A crash-interrupted claim was already atomic through the redo record (`claim.rs:198-212`), so nothing is lost there.

Recommended, if REQ-08's "claim" covers both claim commands: align `issue claim` so that `IssueClaimed` is written only when the assignee changes. A same-assignee promotion from Ready would then emit only `issue_state_changed` (`mod.rs:3166-3172`). This gives one rule (D-20).

Unchanged and out of scope, but worth reporting: `claim acquire` overwrites another agent's assignment without a release event or a refusal (`mutation.rs:857-866`), whereas `issue claim` refuses.

**Interpretation risk.** The append path reads the event log's length and its final line through the tail capture, as every event-appending mutation does. Under a literal reading of "reads no event-log content", that tail read counts as content. Recommended wording: "decides from no event-log record; only the shared append base (length and final line) is read". REQ-14's unparseable log still succeeds, because an unparseable final line is treated as torn and gets a separator newline (`mutation.rs:603-628`).

### 6. Architecture fit and invariant check

- **Layers.** The lease sequence, layout and atomic publish belong in `storage/claim_coordinator.rs`. The `op(current) -> Change` decisions are pure and can live beside `Lease` as domain-style functions. Commands keep their identity, branch and config resolution.
- **D-4.** The guard and every lease op already short-circuit outside git (`mod.rs:725`, `claim.rs:45-63`). The per-key layout stays under `paths.shared_jit`, so nothing changes there.
- **D-21 (one storage-owned git module).** It is violated today by `claim_coordinator.rs:979-998` and `commands/claim.rs:24-63,297-315`. That is out of scope, but the restructure should not add a third git call site.
- **convention-convergence.** Convergence targets:
  - one lease sequence;
  - one expiry/activeness predicate (`Lease::is_active(now)`) replacing the four in §3;
  - `template.rs:290-320` merged into `captured_lease_warnings`;
  - the private `write_index_atomic` replaced by the atomic-write primitives;
  - the legacy lock-free `require_active_lease` preflight, which duplicates the guarded check, deleted or kept as advisory over the same per-key read.
- **atomic-writes.** Per-key publish is temp + rename + directory sync. The claims log grows by `O_APPEND` under the lock; the positioned-append rule applies to the event log only.
- **derived-state-coherence.** It covers repository mutations. The lease file is the only lease authority and nothing is derived from it, so no coupled materialization remains.
- **event-log.** The S-a gap closes. `claim release` and force-evict still change no issue state, so they owe no event.
- **canonical-cutover.** Existing `claims.index.json` and `claims.jsonl` are not migrated. Leases active at cutover are lost, which is acceptable for a machine-local, TTL-bounded control plane; state this in the step. 0abaeba0's rewrite of the log stays a precondition of 2195c98f's strict decode only.
- **shared-test-contracts and orthogonal-tests.** The lease sequence's interruption matrix is owned once at the coordinator. The proptests drive `acquire` through the production entry (`acquire_claim_synchronized` with a no-op `repository_sync`, or a split-out lease step that synchronized acquire composes). Rebuild-centric props are deleted: `prop_index_rebuild_idempotent`, `prop_no_data_loss_in_rebuild` and `prop_staleness_computed_during_rebuild`.

### 7. Related in-flight issues and seams

| Issue | State | Seam | Recommended edge |
|---|---|---|---|
| d7d0d0db | Done | Evictions persist on reads. Option A keeps this per key and closes the renew leak. | none |
| 5b37a806 "Recovery never reclaims an indefinite lease whose holder has died" | **Ready**, assigned `agent:claude` | Changes recover's eviction pass and adds a reclamation event | 5b37a806 depends on S1, or is folded into S1 |
| 083c9a91 "Claim and recover operations behind a claims facade" (df129d9e) | Backlog | Moves every `commands/claim.rs` entry point | 083c9a91 depends on S1 and S2, so it wraps the final API |
| d759d606 "Drop the unused storage type parameter..." (df129d9e) | Ready | Edits `claim.rs:421,469,618` and deletes `check_issue_lease` | Either order. Trivial conflict; land it first if it is ready. |
| 0abaeba0 `claims-log-canonical-rewrite` | Backlog | Data rewrite of the log, needed by 2195c98f strict decode | Upstream through 2195c98f |
| 98404e0c `lease-issue-identity` | Backlog | Deletes short-id tolerance (`:294-319,806-810`). That is the precondition for a per-key lookup by issue. | S1 depends on it |
| 8cfbf19d `lease-worktree-ids` | Backlog | Types `lease_id`/`worktree_id`; its REQ-04 parses lease-id CLI args, which §5.2.2(i) would replace | S1 depends on it |
| a50e9be9 `agent-identity-assignee` | Backlog | `Lease.agent_id` and `ClaimCoordinator` agent become `Assignee` | S1 depends on it |
| 6e8b9f31 `system-actor-constants` | Backlog | `system:*` actors in `claim.rs` and `mod.rs:729` | S1 depends on it |

### 8. Sequencing

Every step lands after the 2195c98f container. Each one leaves fmt, clippy and tests green.

**S1: Per-issue lease records through one lease sequence.**

- Files:
  - `storage/claim_coordinator.rs` (the sequence, the layout, `claims.seq`, the history append, a lease interruption injector, removal of the index, rebuild, verify, `acquire_claim`, `acquire_claim_with_reason`, `release_lease`, `evict_expired` and `write_index_atomic`; the unit tests are retargeted);
  - `storage/claim_coordinator_proptests.rs`;
  - `storage/worktree_paths.rs` (lease path constructor);
  - `storage/warnings.rs` (delete `SequenceGap`, `DuplicateLease`, `IndexRebuilt`);
  - `commands/claim.rs` (recover loses rebuild and verify; `RecoveryReport.index_rebuilt` goes away; release, renew, heartbeat and force-evict use the sequence);
  - `commands/mod.rs:720-800` (the guard reads per key);
  - `scripts/hooks/pre-commit`, `scripts/hooks/pre-push` and `commands/hooks.rs:288` (§5.2.3).
- Bridge so the step lands green: `ClaimsIndex::load(paths)` is re-implemented as a listing of `leases/` with the same signature, so its six readers compile unchanged. It is a named temporary bridge (canonical-cutover) that S2 removes.
- 2195c98f keys: `lease-issue-identity` (98404e0c), `lease-worktree-ids` (8cfbf19d), `agent-identity-assignee` (a50e9be9), `system-actor-constants` (6e8b9f31), `issue-id-flip` (7abcc22d).
- Possibly too large for one worker. If it must split: S1a covers the sequence, layout, coordinator ops, tests and proptests. S1b covers recover, the guard, the hooks and the deletion of the test-only APIs.

**S2: Lease readers converge on the per-key and listing reads.**

- Files:
  - `commands/mod.rs:3329-3420` (`check_active_lease`/`require_active_lease`);
  - `commands/template.rs:290-320`;
  - `commands/worktree.rs:252`;
  - `commands/validate.rs:2482-2530,2726-2770`;
  - `validation/repository.rs:536-550` (the duplicate and schema checks are impossible by construction and are deleted);
  - one `Lease::is_active`;
  - deletion of `ClaimsIndex`;
  - tests: `tests/cli_item_validate/validation_lease_tests.rs`, `tests/cli_issue/failure_lever_registry.toml` (`malformed-claim-index` becomes a malformed lease record), `failure_probe_fixture.rs:637`, `command_exit_code_projection_tests.rs:760`, `claim_integration_tests.rs:177,302,768`, `worktree_cli_tests.rs:48`.
- 2195c98f keys: as S1, plus transitively `stage-validation-issue-ids` and `stage-dispatch-issue-ids`.

**S3: `claim acquire` decides its event from the issue record alone (REQ-08).** Independent of S1 and S2.

- Files:
  - `commands/claim.rs:184-197` (tail capture, `VirtualPath::issue_record`);
  - `repository_state/mutation.rs:674-708,851-888` (delete `captured_events` and `event_tail_reflects_claim`; typed tag instead of `"issue_claimed"` at `:873`);
  - optionally `commands/mod.rs:3164-3172` (`issue claim` alignment);
  - tests to rewrite: `mutation.rs:2116` `test_claim_missing_event_only_emits_event`, `claim.rs:1780` `test_claim_acquire_idempotent_over_split_and_full_reflection`, `claim.rs:1811` (keep; it is the large-log property), and a new S-a test.
- 2195c98f keys:
  - `event-envelope` (d72bf093);
  - `stage-repository-state-issue-ids` (641bd14e);
  - `event-tag-vocabulary` (e98ae5a4);
  - `issue-record-path-storage` (0102769f), and `issue-record-path-commands` (855d027d) for the `claim.rs:192` `format!`;
  - `issue-id-flip` (7abcc22d).

**S4: Claims part of REQ-13/14/15 and the docs.**

- REQ-14 check: replace `.git/jit/claims.jsonl` with unparseable lines and show that `claim acquire`, renew and release succeed. Fold this into the container-wide REQ-14 test that the events and gate-runs parts own.
- Docs:
  - `docs/reference/storage-format.md:377-409`: the per-key layout, the seq counter, the history log and which commands read it (none).
  - `docs/concepts/guarantees.md:410-420`.
  - `docs/how-to/troubleshooting.md:220-235,505-535`.
  - `docs/how-to/multi-agent-coordination.md:326`, `docs/reference/cli-commands.md:3462,3539`, `docs/tutorials/parallel-work-worktrees.md:214` and `docs/reference/claim.md:151`.
  - `scripts/hooks/README.md:75`; the `crates/server/src/watcher.rs:68-95` comment and filter; `.gitattributes:3`.
  - CLI help and the schema, if §5.2.2(i) is chosen.

## Part: Event log, archive evidence, search and the history boundary

## f8a6b298 investigation: event log, archive evidence, search, history-reader boundary, cross-cutting proof

Scope: REQ-09, REQ-10, REQ-11, REQ-12 (event-log part), REQ-13 (all three history files), REQ-14 (test design), REQ-15 (docs). Code read at HEAD `2e0c5ba72`, with other sessions editing concurrently. `commands/validate.rs` and `repository_state/image.rs` shifted while I read them, so line numbers there are ±20.

### 1. Claim classification

| # | Claim / REQ | Verdict | Evidence (current) |
|---|---|---|---|
| C1 | "Execute reads [events.jsonl] twice: once through storage (`commands/archive.rs:248`) and once from the captured image (`:713`, `repository_state/archive.rs:31`, `456`)" | **invalid-as-stated** | `:248` belongs to `plan_archive_target` (`archive.rs:178`), which is the **preview and candidates** path. Execute (`execute_archive_target`, `archive.rs:839`) never calls storage. It captures the whole log once (`capture_archive_base`, `validate.rs:343`, which passes `mutation=false` and so reaches `discover_paths([EVENTS])` at `validate.rs:416`). It then parses that log **twice from the image**: `archive.rs:713` (planner) and `repository_state/archive.rs:31` (finalizer), both through `captured_archive_events` (`repository_state/archive.rs:456`). Preview reads the log once per target, and `archive_candidates` (`archive.rs:158`) plans every terminal container, so it reads the whole log N times. The underlying problem is real. |
| C2a | `parse_known_events` certifies a torn line only by the immediately following profile marker | valid-and-open | `domain/event_log.rs:42-76`, `certifies_preceding_torn_tail` `:78` |
| C2b | "…and it keeps records with retired tags" | **invalid-as-stated** | The parser **skips** retired or unknown tags: it neither returns nor rejects them (`event_log.rs:66-73`, push only when `EventTag::ALL` matches). The trait doc says the same (`storage/mod.rs:232-237`). The real divergence is that the memory backend errors on these records while file storage skips them. |
| C2c | The file backend's archive reader is a second parser with its own torn rule and a raw tag literal | valid-and-open | `storage/json.rs:1357-1393` (`read_artifact_archive_events`), literal `:1382`, rule `is_torn_event_prefix` `:1595`. That rule accepts **any** `{`-prefixed line that hits EOF, anywhere in the file. |
| C2d | The in-memory backend has a third parser that fails on retired tags | valid-and-open | `storage/memory.rs:480-492` (`load_events`, strict `Event` per line, `from_utf8_lossy`). Already tracked as **47a082ef** (ready). |
| C2e (missed) | A fourth, writer-side rule | open | `repository_state/mutation.rs:606` `prefix_has_torn_tail` plus `MutationError::UncertifiedTornTail` (`:477`, raised at `:1206`). It refuses **every** mutation except profile apply when the final line is unterminated and not JSON. Verified: `jit issue update` fails with "uncertified torn tail". |
| C3 | `jit search` with no `--glob` greps the whole data root | valid-and-open | `main.rs:7071-7090` builds `SearchOptions{file_pattern: glob, file_patterns: []}`. `search.rs:116-150` passes the data dir to `rg` with no exclusion. Verified: a garbage line in `events.jsonl` is returned as a match. The server (`crates/server/src/routes.rs:295-322`) always passes `issues/*.json` plus linked-document globs, so only the CLI is affected. |
| REQ-09 | Archive evidence is read once, with no log read | valid-and-open | Rows C1 and §5.3 |
| REQ-10 | One parser, one torn rule, typed selection | valid-and-open | Rows C2a–e. String tag comparisons remain at `json.rs:1382`, `commands/events.rs:25` (`e.get_type() != et`) and `mutation.rs:780,827,836,873` (`PendingEvent.tag` literals). 2195c98f **e98ae5a4** removes the last two groups (its REQ-02 and REQ-03). |
| REQ-11 | Search without a selection reads no history | valid-and-open | Row C3 |
| REQ-12 (event log) | Nothing is derived from the log and nothing rewrites it | valid-and-open, half done | The rewrite half is already structural: the delta rejects any non-append action on the log (`repository_state/image.rs:~1297-1300`), and redo apply only writes at the offset (`storage/redo_apply.rs:308-320`). The one **derivation** from history is `jit migrate lifecycle-timestamps` (`commands/migrate.rs:45-110`), which writes issue fields from the log. **2492ab29** removes it. |
| REQ-13 | Only history-reporting commands read history files | valid-and-open; **much wider than the background states** | §3.1. Ten mutation capture sites read `events.jsonl` **whole** (as unparsed bytes) on every run. |
| REQ-14 | File-backed corruption test | open | §5.6. Today `jit claim acquire` fails on a garbage log line, while `jit issue claim`, `issue update` and `issue show` succeed (verified in a scratch repo). |
| REQ-15 | Docs | open | §2 and §3.3 |
| Doc claim (found) | `docs/concepts/guarantees.md:638`: "issue operations … read only the log's length and final line" | **false today** | Most issue mutations capture the log whole (§3.1). Only `capture_mutation_base` and `capture_mutation_preflight` callers read a tail. |

Tally: invalid-as-stated **2** (C1, C2b), valid-and-open **11** (C2a, C2c, C2d, C3, and REQ-09–15, where REQ-12's rewrite half is already done), already-done **0**, unverifiable **0**.

### 2. Prior-art sweep

- **D-17 redo epic** (`dev/archive/f1dc843a-redo-mutations/active/`):
  - `plan.md:182-187` (`positioned-event-append`): the append is offset plus bytes, the composer may prefix a separator newline, and "Torn-tail certification is preserved".
  - `plan.md:19` (REQ-04): the conflict check uses length plus the last 4 KiB "as bytes … because the torn-tail protocol admits an incomplete final line".
  - Owner decision **`@/issue/c88822a5/decision/D-1`**: the tail is bounded by the final line and re-reads back to the last newline "so torn-tail certification judges the whole final line".
  - Completion report REQ-02 (log-size-independent mutation cost) was proven on the label-update fixture only. The whole-log capture sites in §3.1 never adopted the tail mode.
- **Torn-tail marker origin**: profiles epic `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md:87`. The `ProfileLifecycle.isolated_torn_tail` field is at `domain/types.rs:~1585`.
- **This repository's log**: 16.4k lines, 0 unparseable lines, 0 `isolated_torn_tail:true` markers, last byte `\n`. It holds 7 records with the retired tag `document_archived`, so retired-tag skipping is load-bearing here. It also holds 74 `artifact_archive_executed` records over 50 distinct targets, at most 3 per target.
- **`docs/reference/storage-records.md`** is generated from `crates/jit/src/storage/reference.rs` and checked by a golden test, so it is edited there and regenerated. Its event-log paragraph (`reference.rs:~532`) says "Reading parses the file line by line, skipping blank lines" and states no torn or retired rule. Its gate-run paragraph and the `issue_id` field row say that "gate-runs/ is flat rather than nested per issue"; that belongs to the gate-runs investigator.
- **`docs/reference/storage-format.md`**:
  - `:26`, `:31`: layout tree.
  - `:98-104`: publication contract (the log is extended, not replaced).
  - `:234-259`: the `jit migrate lifecycle-timestamps` backfill, which **2492ab29** removes.
  - `:283-295`: event-log and gate-run sections (pointers only).
  - `:389-406`: claims log and the "Derived cache" index, owned by the claims investigator.
  - It has no section naming history files or their readers. REQ-13 requires one.
- Other docs that state log-read behaviour:
  - `docs/concepts/guarantees.md:638`, `:645`: false today (row above).
  - `docs/concepts/guarantees.md:163-170`, `:413-426`: still accurate.
  - `docs/reference/cli-commands.md:3721-3745` (`jit search`): needs the default-exclusion sentence.
  - `docs/reference/cli-commands.md:2871+` (`jit events`): its "reads the log" wording stays.
  - `docs/how-to/multi-agent-coordination.md:326` ("Rebuild claims index from log"): claims investigator.
  - `docs/concepts/overview.md:96-99`: tree only.
  - `docs/reference/events.md`: generated; it changes only if `isolated_torn_tail` is removed.

### 3. Consumer sweep

#### 3.1 Readers of `events.jsonl` today (production code)

Each reader is listed with the command that reaches it, the read mode and the target classification.

| Site | Command(s) | Mode | Class / target |
|---|---|---|---|
| `commands/events.rs:7,18` → `IssueStore::read_events` (`json.rs:1344`, `memory.rs:900`) | `jit events tail`, `jit events query` | whole, parsed | **history-reporting; keep** |
| `commands/worktree.rs:347-348` → `read_exact_store` (`json.rs:76-170`, parse `:161`) | `jit worktree store-divergence` | both stores whole, parsed | Reports event-record divergence between checkouts. **Decision:** classify it as history-reporting (recommended) or drop event comparison. |
| `commands/archive.rs:248` → `read_artifact_archive_events` (`json.rs:1357`; trait default `storage/mod.rs:249`) | `jit archive candidates`, `archive document`, `archive container` (preview) | whole, second parser | REQ-09: per-key evidence |
| `validate.rs:343,416` → `archive.rs:713` and `repository_state/archive.rs:31` | `archive … --execute` | whole capture, parsed ×2 | REQ-09 |
| `commands/claim.rs:193` capture → `mutation.rs:854` `captured_events` (`:675`) | `jit claim acquire` (`synchronize_claim_repository_state`) | whole, parsed | REQ-08 (claims investigator); tail after |
| `validation/repository.rs:406-409` (`event_count`, which nothing reads) via `validate.rs:416` (`mutation=false`) | `jit validate`, `validate --fix` | whole, parsed | **Not history-reporting**: stop reading (§5.5) |
| `validate.rs:884` (`discover_issue_closure`, broad built-in precheck) | `jit gate evaluate`/`check` of an issue whose precheck is a broad built-in checker (e.g. this repo's `repo-validate`) | whole, parsed by validation | Same as the row above. It also blocks REQ-14's gate evaluation when the gate is broad. |
| `commands/migrate.rs:55,80` | `jit migrate lifecycle-timestamps` | whole, parsed | Removed by **2492ab29** |
| `commands/mod.rs:2301` (`publish_captured_issue_mutation`, 6 callers) | issue lifecycle and field mutations routed through it | **whole bytes, unparsed** | → tail |
| `commands/document.rs:311,371,958` | `doc add`, `doc remove`, asset rescan | whole bytes | → tail |
| `commands/gate_check.rs:~753` (`capture_gate_evaluation_image`) | `gate evaluate`/`check` | whole bytes | → tail |
| `commands/gate.rs:296` (`captured_gate_preset_fixed_paths`), `:1097` (`publish_gate_registry_mutation`) | gate preset apply, gate define/remove | whole bytes | → tail |
| `commands/template.rs:388` | template apply | whole bytes | → tail |
| `commands/profile.rs:2059,2090` | `profile apply`/`refresh` | whole bytes; needs the final line for the marker | → tail |
| `repository_state/initialize.rs:424` | `jit init` (re-init over an existing root) | whole bytes | → tail |
| Tail readers (already compliant): `validate.rs:415` via `capture_mutation_base` and `capture_mutation_preflight` | batch_create, bulk_update, dependency, issue, config, init, profile, mod.rs callers | length plus tail | keep |
| `storage/repository_state_store.rs:916-955` (`ensure_write_set_unchanged_against`), `:1077` (`live_event_log_tail`) | every publish that appends | length plus tail | keep |
| `crates/server/src/watcher.rs:89` | server | ignores `events.jsonl` | fine |

`.git/jit/claims.jsonl` and `gate-runs/` readers are inventoried in the claims and gate-runs parts. History-reporting commands I identified for them:
- `jit gate status <id> [gate] --all/--limit` reads gate-run history.
- `jit recover` (`commands/claim.rs:734-741`) rebuilds the claims index from the log. REQ-07 forbids that.
- `jit worktree list`/`info` call `count_claims_for_worktree` (`worktree.rs:142`).

#### 3.2 Parser and archive-evidence consumers

- `parse_known_events` callers:
  - production: `json.rs:161`, `json.rs:1354`, `repository_state/archive.rs:465`, `mutation.rs:686`, `validation/repository.rs:407`, `commands/migrate.rs:80`
  - tests: `domain/event_log.rs:120-159` and `storage/repository_state_store_tests.rs:1911,2124,2536`
- `read_artifact_archive_events`:
  - production: `commands/archive.rs:248`
  - tests: `commands/archive.rs`, 15 call sites between `:1050` and `:2226`
  - test double: `tests/fast_docs_templates/template_apply_atomicity_tests.rs:154` forwards `read_events`
- `captured_archive_events`: `commands/archive.rs:713`, `repository_state/archive.rs:31`, re-exported at `repository_state/mod.rs:27`.
- Folds over `Event::ArtifactArchiveExecuted`:
  - `recorded_archived_copies` (`domain/artifact_discovery.rs:165`): keyed by target only, across destination roots, with a later record superseding an earlier one.
  - `archive_coverage` (`repository_state/archive.rs:560`): keyed by target and destination root.
  - `apply_recorded_residue_identities` (`commands/archive.rs:107`): keyed by target and destination root.
- `EventLogError` and `ArchiveExecutionError::EventLog` (`repository_state/archive.rs:~548`).

#### 3.3 Search consumers

- `jit::search::search`: the CLI (`main.rs:7071-7100`) and the server (`routes.rs:309`, `322`, always globbed).
- `SearchOptions.file_patterns` is used only by the server.
- Tests: `search.rs:401-460`, the corpus-based argv tests from 34999aad (done).
- MCP and web reach search only through the CLI or server. No other consumers.

### 4. Primitive verification

| Property | Status | Evidence / need |
|---|---|---|
| Event append is positioned at the captured length and idempotent on replay | **holds** | `redo_apply.rs:308-320`: seek, write, `set_len`, sync. Memory backend at `memory.rs:~548`. |
| A jit crash cannot leave a torn log line | **holds under D-17** | The record commits before the append, and replay rewrites `[offset, offset+len)` and truncates. A torn line can therefore come only from an external writer or a pre-redo log, and this repository has none (§2). The marker protocol has no producer left. |
| The append isolates a non-newline final line | holds | `compose_events` (`mutation.rs:~620`) prefixes `\n` when `final_line` is non-empty |
| Concurrent-change detection on the log is bounded by the final line | holds, with a caveat | `EventLogTail::read_tail` (`image.rs:~170-190`) reaches back to the last newline. A huge unterminated final line is read whole. |
| A per-key archive record is written in the same redo record as the event | **mechanism exists** | `finalize_archive_execution` already merges its own `WriteFile` actions with `finalize_delta`'s issue and event actions into one `RepositoryDelta` (`repository_state/archive.rs:~315-322`). The evidence record is one more `WriteFile`. |
| Interruption leaves prior or next | holds | Redo commit point (`@/inv/atomic-writes`); nothing new is needed |
| Whole reads of the log are impossible outside history commands | **missing** | `CaptureSpec::discover_paths` (`image.rs:~555`) accepts `VirtualPath::EVENTS`. Proposal: refuse `EVENTS` in `discover_paths` and offer only `discover_tail` (one place enforces REQ-13 for the capture layer). |
| No action rewrites the log | holds | `validate_action`: "the event log changes only by a positioned append" (`image.rs:~1297`). `RedoRecord::new` still admits a put or delete at `EVENTS` (`redo_record.rs:115-130` checks only overlap). Optionally tighten it in the same place. |

### 5. Design options

#### 5.1 One event-log rule (REQ-10)

- **A (recommended): strict lines, typed selection, skip non-current tags.**
  - A blank line is skipped.
  - A line must be a JSON object with a string `type`. Otherwise it is an `EventLogError` that names the line.
  - The tag goes through `EventTag`'s exact-token parse (2195c98f wire-vocabulary-macro **3f026bb1**). A known tag decodes or errors. An unknown or retired tag is skipped.
  - No torn-line tolerance. The `isolated_torn_tail` field, `certifies_preceding_torn_tail`, `prefix_has_torn_tail`, `UncertifiedTornTail` and `is_torn_event_prefix` are all deleted (D-18), along with the marker-reordering block (`mutation.rs:~1199-1232`).
  - The appender keeps only the separator-newline rule. A corrupted final line is then isolated, and history commands report it.
  - Consistent with `guarantees.md:638` ("never silently skipped").
- **B: tolerate any EOF-truncated object prefix anywhere** (today's `json.rs:1595` rule) and keep the rest of A. One rule with no marker, but it tolerates a state that no longer has a producer.
- **C: keep the marker rule everywhere.** Smallest diff, but it keeps profile apply as the only path that can repair, and it keeps the writer-side refusal that breaks REQ-14 for an unterminated garbage tail.

Notes:
- Removing `isolated_torn_tail` changes `ProfileLifecycle` serialization. Old lines still decode because `Event` cannot use `deny_unknown_fields`, given the 2195c98f envelope's `flatten`. However, **d72bf093** REQ-03 asserts that every line in this repository's log re-serializes to identical bytes. If that assertion survives as a permanent test, it breaks.
- Location: `domain/event_log.rs` (pure). Both backends' `read_events` and every history command call it. 47a082ef already moves the memory backend there.

#### 5.2 Event capture for appends (REQ-13 for mutations)

- **A (recommended): tail-only capture of `EVENTS`.**
  - Every mutation uses `discover_tail`, and `CaptureSpec` refuses `EVENTS` as a whole path.
  - Keep `@/issue/c88822a5/decision/D-1` (tail reaches back to the final line). With 5.1-A the final line is needed only for the separator decision, so the reach-back could shrink to a fixed window plus the last byte. That **reverses an owner decision**, so it is listed under Needs decision and not assumed.
- **B: probe only the length and last byte.** Minimal I/O, but the conflict check would lose its 4 KiB tail comparison, which is part of the D-17 REQ-04 contract.

Under either option, appending reads the log's end only and never its records. REQ-08 and REQ-14 should define "reads no event-log content" as "parses no records", with the bounded end probe allowed. Otherwise no append is possible.

#### 5.3 Per-key archive evidence (REQ-09)

The evidence is **not** on the archived issue record today:
- The issue record keeps only relinked `documents[].path` (`DocumentReference`, `types.rs:829`).
- The `.jit-container` marker holds only the container id (verified on `dev/archive/*/.jit-container`).
- Document targets have no issue at all.

Consumers need `(target, destination_root) → publications{source → (destination, content_identity)}` plus `reference_changes`, and `target → latest destination per source` across roots.

- **A (recommended): `archive-evidence/<target-key>.json` in the data root.**
  - Key: `container/<IssueId>` or `document/<sha256(normalized path)>`. The record carries the target for verification.
  - Body: the ordered list of that target's executions. Each entry is the existing `ArtifactArchiveExecuted` payload (`target`, `destination_root`, `publications`, `reference_changes`, `planned_deletions`, `reconciling`). Factor that payload into one struct shared by the event body and the record.
  - The three folds keep their logic and take `&[ArchiveExecution]` instead of `&[Event]`.
  - The record grows only with reruns of that one target (at most 3 here).
  - `finalize_archive_execution` writes `prior + this` whenever `event_needed`, in the same redo record as the event.
  - The planner reads the record once from the image and hands it to the finalizer, so execute reads it once. With b697a3f7 REQ-06, preview reads it through the same planner; until then, through one `IssueStore` read.
  - Deleted: `read_artifact_archive_events` (trait and json.rs), `captured_archive_events`, `ArchiveExecutionError::EventLog`. `capture_archive_base` switches to a tail capture of the log.
- **B: fold the evidence into the destination's `.jit-container` marker**, with a sidecar for documents. This puts evidence in the worktree archive tree, changes the marker's identity contract (`artifact_classifier.rs:248,459,600`), and needs a separate sidecar shape for documents. Rejected.
- **C: put it on the container issue record.** No home for document targets, and it inflates every issue read. Rejected.

With no migration (`@/inv/canonical-cutover`), this repository's 50 archived targets get no evidence record, so reruns over them lose coverage and residue identity. Recommend a one-off **checkout-data task** that seeds the 50 records from this log with a throwaway script, analogous to 2195c98f `claims-log-canonical-rewrite` (0abaeba0). It is not product code. Listed under Needs decision.

#### 5.4 Search exclusion (REQ-11)

Verified with ripgrep 15.2:
- Anchored negations (`!/events.jsonl`) do **not** apply when the search path is absolute.
- Unanchored negations (`--glob '!events.jsonl' --glob '!gate-runs'`) exclude both, keep absolute result paths, and a user `--glob '*.json'` still selects gate-run files.

- **A (recommended):** when no glob is given, `search.rs` appends unanchored negations built from one declaration of history paths (`VirtualPath::EVENTS` and `GATE_RUNS` leaf names, exported beside them in `repository_state/path.rs`). The same declaration feeds the storage-format reference list. The claims log is outside the data root, and `tmp/` (redo scratch, which can hold appended event bytes) should be excluded the same way.
- **B:** run `rg` with `current_dir(data_root)` and anchored globs. This is exact, but result paths become relative, which changes output.
- **C:** pass explicit non-history entries as path arguments. This needs a listing and changes semantics for new files.

Seam: **0f8efc37** REQ-08 ("ripgrep runs only over linked documents") may later remove data-root grepping entirely. **d05b5639** and **98cc2097** move the CLI call site. Putting the exclusion inside `search.rs` survives both.

#### 5.5 Validation and the log

`event_count` (`validation/repository.rs:58`) has no reader. Recommend deleting the event-log parse from repository validation (D-18):
- `jit validate` and broad built-in prechecks then read no history.
- `capture_proposed_base_inner(mutation=false)` uses the tail too, which removes its whole-log branch.
- Log integrity stays observable through `jit events tail` (history-reporting).

#### 5.6 REQ-14 test design

Location: one module `crates/jit/tests/cli_repo_workflow/history_isolation_tests.rs` plus a `mod` line in that suite's `main.rs`. This adds no Cargo target (`@/inv/bounded-rust-build-footprint`, `dev/TESTING.md:168-214`).

Shape:
- CLI subprocess against a git-initialised `TempDir` (git is needed for leases).
- Setup: one shared fixture creates issue A (target) and issue B, defines a gate with a trivial `exec` checker (`true`), runs it on B, and acquires and releases one lease so the claims log exists.
- Corruption, by a shared `corrupt_history(repo)` helper:
  - replace `.jit/events.jsonl` with unparseable lines, the **last one unterminated** to pin the append rule;
  - replace `.git/jit/claims.jsonl` with unparseable lines;
  - overwrite B's gate-run records with unparseable bytes.
- One test per command family (`@/inv/orthogonal-tests`): reads (`issue show`, `issue status`, `query available`), `gate evaluate A`, `claim acquire`, and lease acquire/renew/release. Each owns its `TempDir`.

Notes:
- The gate must not be a broad built-in precheck unless §5.5 lands; with §5.5 it can be.
- REQ-14's `claim` maps to `jit claim acquire` (the `ClaimIssue` intent path, `claim.rs:193`); `jit issue claim` already passes.
- The capture-layer refusal of a whole `EVENTS` read (§4) is the cheap structural owner of "mutations parse no log". The CLI test proves composition only.

### 6. Architecture fit / invariant check

- The parser stays pure in `domain/event_log.rs`. The archive-evidence record type and folds belong in `domain/` (pure). The path constructor goes in `repository_state/path.rs`, and reading and writing go through capture and finalize (`@/inv/architecture-dependency-direction`).
- `@/inv/event-log`: archive still appends `ArtifactArchiveExecuted`. The evidence record is co-produced in the same redo record, not derived from the log (REQ-12 holds).
- `@/inv/derived-state-coherence`: evidence is a write target, so it gets the write-set conflict check automatically (`ensure_write_set_unchanged`).
- `@/inv/convention-convergence`: one parser and one capture mode for the log, with no private readers.
- `@/inv/canonical-cutover`: torn-tail compatibility is removed (5.1-A). The archive-evidence gap for old targets is handled as checkout data, not migration code.
- `@/inv/single-source-prose`: the history-path list in the docs comes from the declaration in §5.4. Generated pages (`storage-records.md`, `events.md`) are edited at their source in `storage/reference.rs` and `event_catalog.rs`.

### 7. Sequencing

External prerequisites (recommended edges into f8a6b298's steps):
- **47a082ef** (ready): memory backend through `parse_known_events`, with a cross-backend test. Edge: S1 depends on it.
- **2492ab29**: removes `migrate`. Edge: S2b and S6 depend on it. Otherwise S2b must convert `migrate.rs:55`.
- 2195c98f keys:
  - `event-envelope` **d72bf093** and `event-body-view` **18bcafb8**: they rewrite the matcher in `event_log.rs`.
  - `event-tag-vocabulary` **e98ae5a4**: typed `PendingEvent.tag` and the events `--event-type` parse.
  - `wire-vocabulary-macro` **3f026bb1**: exact-token `EventTag` parse.
  - `stage-domain-issue-ids` **3ee3a4d1**: `PlanTarget` id typed.
  - `issue-id-flip` **7abcc22d**.
  - `issue-record-path-storage` **0102769f**: constructor pattern for `VirtualPath::archive_evidence`.
  - `well-known-paths-repository-state` **18dc6e3e**, `well-known-paths-storage` **e1a6c8c2**: `json.rs` `EVENTS_FILE` literal.
- **b697a3f7** (sibling epic, no edge today): its REQ-06 (preview runs the execute planner) touches the same archive functions. Recommend that S3 depends on b697a3f7's archive-preview entry, or that S3 lands first and b697a3f7 rebases. Needs decision.

Steps (each keeps clippy and tests green):

| Step | Change | Files | Needs |
|---|---|---|---|
| S1 | One rule (5.1-A): rewrite `parse_known_events`, delete the marker protocol and writer refusal, delete `is_torn_event_prefix`. The archive reader temporarily calls `parse_known_events` and filters by typed variant. | `domain/event_log.rs`, `domain/types.rs`, `domain/event_catalog.rs`, `repository_state/mutation.rs`, `storage/json.rs`, `commands/profile.rs` (torn tests ~5247), `docs/reference/events.md` (regenerated), `storage/reference.rs` | 47a082ef, d72bf093, e98ae5a4, 3f026bb1 |
| S2a | Validation stops parsing the log (5.5), and the non-mutation base captures the tail | `validation/repository.rs`, `commands/validate.rs` (~416, ~884) | — |
| S2b | Switch the whole-log captures to tail | `commands/{mod,document,gate,gate_check,template,profile}.rs`, `repository_state/initialize.rs` | 2492ab29 |
| S3 | Archive evidence record (5.3): domain type and folds, path constructor, planner reads once, finalize writes it; delete `read_artifact_archive_events`, `captured_archive_events` and the json.rs reader; archive base uses the tail. Plus the separate checkout-data seeding task. | `domain/artifact_discovery.rs`, `domain/artifact_execution.rs`, `domain/types.rs`, `repository_state/{archive,path,mod}.rs`, `commands/archive.rs`, `storage/{mod,json,memory}.rs` | 3ee3a4d1, 7abcc22d, 0102769f; b697a3f7 seam |
| S4 | `CaptureSpec` refuses a whole `EVENTS` read; optionally `RedoRecord` refuses a put or delete at `EVENTS` | `repository_state/image.rs`, `storage/redo_record.rs` | S2a, S2b, S3, and the claims part's REQ-08 step (claim must stop parsing first) |
| S5 | Search exclusion (5.4) with a corpus unit test in `search.rs` | `search.rs`, `repository_state/path.rs`, `docs/reference/cli-commands.md` | 18dc6e3e (path constants) |
| S6 | Docs (REQ-13 and REQ-15): storage-format "History files" section naming the readers; event-log reading rule in `reference.rs`, regenerated; fix `guarantees.md:638,645`. Gate-run and claims wording comes from the sibling parts. | `docs/reference/storage-format.md`, `storage/reference.rs`, `docs/reference/storage-records.md`, `docs/concepts/guarantees.md`, `docs/concepts/overview.md` | after all layout steps |
| S7 | REQ-14 test (5.6) | `tests/cli_repo_workflow/{history_isolation_tests.rs,main.rs}` | last: needs the gate-runs per-key layout and the lease sequence from the sibling parts |

