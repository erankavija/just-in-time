# Plan: Restructure storage so reads never scale with project history (f8a6b298)

> Planning node: 5e615dc2. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

Every per-key read gets a record addressed by that key, written in the same redo record as the change that produces it (`@/charter/D-17`, `@/charter/D-19`). No index, pointer or cache is derived from a history file (`@/charter/D-18`). Each history file becomes append-only and is read only by history-reporting commands. Issue bodies are written against the post-2195c98f code (`IssueId`, `GateRunId`, `GateKey`, `LeaseId`, the `Event`/`EventBody` envelope, `VirtualPath` constructors) and cite current lines.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | Runs move to `gate-runs/<issue-id>/<run-id>.json`, load by `(issue, run)`, and latest-run readers load the run their gate state names. The repository's runs are rewritten once. | Investigation, gate runs §5a, §5c |
| REQ-02 | `GateState.last_run` names the latest run. The five latest-run selections collapse into one accessor. | Gate runs §1 REQ-02 row |
| REQ-03 | A verdict record per `(gate, digest)` names the source run, and reuse reads only that record inside the image. | Gate runs §5b (R1) |
| REQ-04 | Captures request the `last_run` record of each history-consuming gate on the evaluated issue, and the root listing is deleted. | Gate runs §1 capture rows |
| REQ-05 | A lease read API comes first, then per-issue lease files behind it. Renew, heartbeat and force-evict move to issue addressing. The limit check reads current lease files (DEC-13). | Leases §5.1, §5.2 |
| REQ-06 | Test-only APIs go first, then one private lock-load-change-publish sequence serves every operation, including the mutation guard. | Leases C4–C6 |
| REQ-07 | The lease-file rename or unlink is the commit point, and history is appended after it. An injector matrix proves prior-or-next. | Leases §5.1 option A |
| REQ-08 | The claim event follows the assignee change (DEC-12). The capture is the issue record plus the tail probe (DEC-16). | Leases §5.3 |
| REQ-09 | Archive executions become one domain record type. Per-target evidence records are written in the execute redo record, and this repository's targets are seeded once. | Events §5.3 (A) |
| REQ-10 | One torn-line rule (DEC-15) comes first. Readers then converge on the one parser, which skips retired tags uniformly, and select by typed tag. | Events §5.1 |
| REQ-11 | Unanchored ripgrep negations are built from one history-path declaration. | Events §5.4 (A) |
| REQ-12 | The claims index is deleted, nothing is seeded from history at runtime, and `RedoRecord` refuses a put or delete at the log. | Leases R12c; Events §4 |
| REQ-13 | Validation, mutation captures, claim and archive move to the tail probe, and `CaptureSpec` refuses a whole read of the log. The storage-format reference names the readers (DEC-19). | Events §3.1 |
| REQ-14 | One CLI integration module in `cli_repo_workflow`, with one shared fixture and corruption helper and one test per command family (DEC-20). | Events §5.6 |
| REQ-15 | The generated storage-record reference is updated where the layout changes, plus one section entry for the new records. Hand-written pages are covered by three docs entries. | Gate runs §1 REQ-15; Leases §2 |

## Shared architectural contracts

### `gate-run-record-layout` [plan-fixed] — Per-issue gate-run records

A run record lives at `gate-runs/<issue-id>/<run-id>.json`, built only by `VirtualPath::gate_run_record(&IssueId, &GateRunId)`. 2195c98f's 79aa3aa7 introduces the constructor with `(&GateRunId)`, and this epic adds the issue. An issue's run history is one listing of `gate-runs/<issue-id>/`. Loading one run takes `(issue, run)` and reports a record of another issue as not found. Everything under `gate-runs/` is already a tracker-written path (`repository_state/path.rs:537-560`).

### `gate-state-last-run` [implementation-produced] — Latest run on the gate state

`GateState.last_run: Option<GateRunId>` is serialized only when present. It is set by `gate_state_from_run` in the plan that records the run, and only there. A manual status write keeps the prior value. A gate whose state has no `last_run` has no latest run, even when runs of it exist. Producer: `gate-state-last-run`.

### `gate-verdict-record` [plan-fixed] — Per-key reusable verdict

`gate-runs/verdicts/<gate-key>/<inputs-digest>.json` holds `{issue_id, run_id}` for the latest recorded run of that gate over that digest that executed its checker and passed or failed. The finalizer writes or replaces it in the run's redo record, with the captured preimage as the expected value. Reuse reads only this record and the run it names. A run that no verdict record names is never a reuse source.

### `reused-verdict-provenance` [implementation-produced] — Reused verdict names issue and run

`GateVerdictOrigin` is internally tagged on `derivation`. `Reused { source_issue: IssueId, source_run: GateRunId }` serializes as `{"derivation":"reused","source_issue":…,"source_run":…}`, and a record without `source_issue` fails to decode. The old shape is rewritten by the repository's run rewrite in the same landing group. Producer: `gate-verdict-provenance`.

### `lease-record-layout` [plan-fixed] — Per-issue lease records

An active lease is `.git/jit/leases/<issue-id>.json`, named by one constructor on `WorktreePaths`. Control-plane paths have no `VirtualPath` home (2195c98f DEC-14). An absent file means no lease. `.git/jit/claims.seq` holds the last sequence number as a decimal u64. `.git/jit/claims.jsonl` is append-only history that no command reads. `claims.index.json` does not exist. A lease record stores no `stale` field; readers report `Lease::is_stale(now)`. The lease-file rename or unlink is the commit point. The sequence bump and the history lines follow it, so a crash loses at most one history line and never lease state.

### `lease-read-api` [implementation-produced] — One lock-free lease read path

`Lease::is_active(now)` means neither expired nor stale. `read_active_lease(&WorktreePaths, &IssueId, now)` and `read_active_leases(&WorktreePaths, now)` in `storage/claim_coordinator.rs` are the only lock-free lease reads. They read the index until `lease-per-issue-records` reimplements them over lease files with the same signatures. Producer: `lease-read-api`.

### `lease-sequence` [implementation-produced] — One lease mutation sequence

A private `with_lease(&IssueId, op)` runs these steps in order: claims lock with `CoordinationOrderGuard`, load the lease file, evict it if expired, then `op(current, now) -> LeaseChange { Keep | Put(Lease) | Remove(reason) }` (pure). A change or an eviction is then published (temp file synced, rename, directory sync; or unlink, directory sync), followed by the `claims.seq` bump and the history append. Every operation, the repository-mutation guard's per-target read included, goes through it. Producer: `lease-per-issue-records`.

### `lease-id-scan-bridge` [implementation-produced] — Interim lease-id lookup

Until renew, heartbeat and force-evict are addressed by issue (DEC-11), they find a lease by listing the lease directory. This is a named temporary bridge with one producer, `lease-per-issue-records`, and one remover, `lease-issue-addressed-ops`, which depends on the producer. Its removal criterion: no lease operation lists the lease directory to find a lease by id (`@/inv/canonical-cutover`).

### `event-log-rule` [plan-fixed] — One event-log parser

`domain::parse_known_events` is the only parser, and it skips blank lines. A final line without a terminating newline is an uncommitted append and is ignored. A well-formed record whose tag is not a current `EventTag` (a retired tag) is skipped by every caller. Any other malformed line is an error that names the line. Appending over an unterminated final line prefixes a newline. Callers select events by typed tag.

### `event-append-probe` [plan-fixed] — Bounded end-of-log probe

A non-history command reaches the event log only through `CaptureSpec::discover_tail`: the length plus the bytes back to the final line's start, per owner decision `@/issue/c88822a5/decision/D-1`. It parses no record. "Reads no event-log content" in REQ-08 and REQ-13 means exactly this (DEC-16).

### `archive-execution-record` [implementation-produced] — Archive execution payload

`domain::artifact_execution::ArchiveExecution` holds the target, destination root, publications, reference changes, planned deletions and the reconciling flag. `EventBody::ArtifactArchiveExecuted` wraps it with its bytes unchanged. The archive folds take `&[ArchiveExecution]`. Producer: `archive-execution-record-type`.

### `archive-evidence-record` [plan-fixed] — Per-target archive evidence

`archive-evidence/<target-key>.json` holds the target and its ordered executions. The key is `container/<issue-id>` or `document/<sha256 of the normalized path>`, built by one `VirtualPath` constructor. The directory joins `tracker_written_paths`. Execute writes `prior + this` in the redo record of its event. Planner, finalizer and preview each read it once.

### `history-paths` [implementation-produced] — Declared history paths

`repository_state/path.rs` declares the data-root history paths once: the event log, the gate-run directory and the redo scratch directory. Default search excludes them, and the storage-format reference names the same set. Producer: `search-excludes-history`.

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| gate-state-last-run | Gate state names the run that produced it | task | Each recorded gate run's id is stored on the issue's gate state for that gate. | gate-run-record-layout | REQ-02, DEC-05, DEC-01 | touches 12 | — | — |
| gate-run-latest-captures | Gate captures hold only the evaluated issue's latest runs | task | Gate captures request the evaluated issue's latest run of each gate by path and list no gate-run root. | gate-state-last-run, gate-run-record-layout | REQ-04, DEC-05 | touches 5 | gate-run-cutover | gate-state-last-run, validation-skips-event-log, mutation-event-tail-captures |
| gate-run-load-by-issue | Gate-run records load by issue and run id | task | One gate-run record loads by its issue id and run id through storage, the executor and the server route. | gate-run-record-layout | REQ-01, DEC-05, DEC-01 | touches 8 | — | — |
| gate-latest-run-reads | Latest-run readers load the run their gate state names | task | Readers of an issue's latest gate runs load one record per gate, named by its gate state. | gate-state-last-run | REQ-01, REQ-02, DEC-05 | touches 6 | gate-run-cutover | gate-state-last-run, gate-run-load-by-issue |
| gate-verdict-provenance | Reused verdicts name their source issue and source run | task | A reused verdict's provenance names its source issue and source run. | — | REQ-01, DEC-07, DEC-01 | touches 6 | gate-run-cutover | — |
| gate-verdict-records | Reusable verdicts are found through a per-key verdict record | task | Gate reuse reads one per-key verdict record written with the run that holds the verdict. | gate-verdict-record, reused-verdict-provenance | REQ-03, DEC-06 | touches 9 | gate-run-cutover | gate-verdict-provenance, gate-run-load-by-issue, gate-run-latest-captures |
| gate-run-per-issue-layout | Gate-run records live under their issue | task | Gate-run records are stored per issue, and an issue's run history is one directory listing. | gate-run-record-layout, gate-state-last-run | REQ-01, REQ-15, DEC-05 | touches 13 | gate-run-cutover | gate-latest-run-reads, gate-verdict-records |
| gate-run-records-relayout | This repository's gate runs are rewritten into the per-issue layout | task | This repository's recorded gate runs, latest-run references and verdict records match the per-issue layout. | gate-run-record-layout, gate-verdict-record, gate-state-last-run, reused-verdict-provenance | REQ-01, DEC-08 | creates 1, touches 2 | gate-run-cutover | gate-run-per-issue-layout |
| lease-hooks-status | Git hooks check leases through the claim status command | task | The shipped git hooks decide lease ownership through the claim status command. | — | REQ-12, DEC-09, DEC-01 | creates 1, touches 5 | — | — |
| lease-production-acquire-tests | Claim tests acquire leases through the production path | task | Claim unit tests and property tests acquire and evict leases only through production entry points. | — | REQ-06, DEC-10, DEC-01 | touches 5 | — | — |
| lease-read-api | Lease readers share one read path and one activeness rule | task | Lock-free lease readers use one storage read path and one activeness predicate. | — | REQ-05, DEC-09, DEC-01 | touches 5 | — | — |
| lease-per-issue-records | Each lease is one record under one lease sequence | task | Leases are per-issue records changed only through one lock-load-change-publish sequence. | lease-record-layout, lease-read-api | REQ-05, REQ-06, REQ-07, REQ-12, DEC-02, DEC-03, DEC-09, DEC-10, DEC-13 | touches 12 | lease-cutover | lease-production-acquire-tests, lease-read-api, lease-hooks-status |
| lease-index-validation-removal | Repository validation stops checking the claims index | task | Repository validation holds no claims-index check and the index type is gone. | — | REQ-12, DEC-09 | touches 4 | — | lease-per-issue-records, validation-skips-event-log |
| lease-issue-addressed-ops | Renew, heartbeat and force-evict address a lease by issue | task | Renew, heartbeat and force-evict find the lease from the issue id directly. | lease-sequence, lease-id-scan-bridge | REQ-05, DEC-11, DEC-03 | touches 6 | — | lease-per-issue-records |
| lease-interruption-matrix | An interrupted lease operation leaves the prior or the next lease | task | One matrix test proves an interrupted lease operation leaves the prior or the next lease. | lease-sequence, lease-record-layout | REQ-07, DEC-09 | touches 1 | — | lease-issue-addressed-ops |
| lease-records-rewrite | This checkout's leases are rewritten into lease records | task | This checkout's active leases are lease records and its claims index is gone. | lease-record-layout | REQ-05, DEC-14 | creates 1, touches 3 | lease-cutover | lease-per-issue-records |
| claim-event-from-assignee | A claim emits its event exactly when the assignee changes | task | Claims decide their claim event from the issue's assignee and parse no event-log record. | event-append-probe | REQ-08, DEC-04, DEC-12, DEC-16, DEC-01 | touches 4 | — | — |
| event-torn-line-rule | An unterminated final event line is an uncommitted append | task | The event log has one torn-line rule and the profile marker protocol is gone. | event-log-rule | REQ-10, DEC-15, DEC-01 | touches 11 | — | — |
| validation-skips-event-log | Repository validation reads no event log | task | Repository validation and broad prechecks read only the event log's bounded end. | event-append-probe | REQ-13, DEC-19, DEC-01 | touches 2 | — | — |
| mutation-event-tail-captures | Mutations capture the event log's end, not the whole log | task | Event-appending mutations capture only the event log's bounded end. | event-append-probe | REQ-13, DEC-19 | touches 7 | — | event-torn-line-rule |
| archive-execution-record-type | Archive executions are one domain record type | task | Archive evidence folds read one domain execution record type shared with the event. | — | REQ-09, DEC-17, DEC-01 | touches 8 | — | event-torn-line-rule |
| archive-evidence-records | Archive evidence is a per-target record | task | Archive preview and execute read a target's evidence from its per-target record once. | archive-evidence-record, archive-execution-record, event-append-probe | REQ-09, DEC-17, DEC-03 | touches 12 | archive-evidence-cutover | archive-execution-record-type, validation-skips-event-log |
| archive-evidence-seeding | This repository's archived targets get evidence records | task | This repository's archived targets each have an evidence record matching the event log. | archive-evidence-record | REQ-09, DEC-17 | creates 1, touches 1 | archive-evidence-cutover | archive-evidence-records |
| event-log-reader-convergence | Event-log readers share one parser and select events by typed tag | task | Event-log reads go through one parser that skips retired tags and selects by typed tag. | event-log-rule | REQ-10, DEC-15 | touches 5, uncertain | — | claim-event-from-assignee, archive-evidence-records |
| capture-refuses-whole-event-log | Captures and redo records refuse whole-log access | task | The capture layer and the redo record admit only a tail probe and an append on the event log. | event-append-probe | REQ-12, REQ-13, DEC-19 | touches 3 | — | mutation-event-tail-captures, claim-event-from-assignee, archive-evidence-records |
| search-excludes-history | Default search reads no history file | task | A search without an explicit file selection excludes the declared history paths. | — | REQ-11, DEC-18, DEC-01 | touches 2 | — | — |
| storage-records-per-key-reference | The storage-record reference describes verdict and archive-evidence records | task | The generated storage-record reference covers verdict and archive-evidence records. | gate-verdict-record, archive-evidence-record | REQ-15, DEC-06, DEC-17 | touches 2 | — | gate-run-per-issue-layout, archive-evidence-records |
| history-isolation-test | Ordinary commands succeed over unparseable history | task | A CLI integration module proves ordinary commands succeed over unparseable history files. | — | REQ-14, DEC-20, DEC-16 | creates 1, touches 1 | — | gate-run-per-issue-layout, lease-issue-addressed-ops, capture-refuses-whole-event-log, event-log-reader-convergence |
| storage-format-history-docs | The storage-format reference describes per-key records and history files | task | The storage-format and concepts pages describe per-key records and name history files and their readers. | gate-run-record-layout, gate-verdict-record, lease-record-layout, archive-evidence-record, history-paths | REQ-12, REQ-13, REQ-15, DEC-19 | touches 3 | — | gate-run-records-relayout, lease-index-validation-removal, lease-issue-addressed-ops, capture-refuses-whole-event-log, event-log-reader-convergence, search-excludes-history, storage-records-per-key-reference |
| lease-operator-docs | Lease command and recovery docs match per-issue lease records | task | Lease command, recovery and testing pages match issue-addressed lease commands and lease records. | lease-record-layout | REQ-12, REQ-15, DEC-11 | touches 9 | — | lease-interruption-matrix, lease-index-validation-removal |
| gate-and-search-command-docs | Gate provenance and search docs match per-key reads | task | Command docs show reused-verdict provenance with its source issue and the default history exclusion of search. | reused-verdict-provenance, history-paths | REQ-11, DEC-07, DEC-18 | touches 2 | — | gate-run-records-relayout, search-excludes-history |
| per-key-gate-runs | Gate runs are addressed per issue, per gate and per inputs digest | story | Gate-run records are stored and read per issue, per gate and per inputs digest. | — | REQ-01, REQ-02, REQ-03, REQ-04 | — | — | gate-run-records-relayout |
| per-key-leases | Leases are per-issue records under one lease sequence | story | Leases are per-issue records under one sequence, and claims decide their event from the assignee. | — | REQ-05, REQ-06, REQ-07, REQ-08 | — | — | lease-records-rewrite, lease-operator-docs, claim-event-from-assignee |
| history-boundary | Only history-reporting commands read history files | story | Only history-reporting commands read history files, and the boundary is enforced, documented and tested. | — | REQ-09, REQ-10, REQ-11, REQ-12, REQ-13, REQ-14, REQ-15 | — | — | history-isolation-test, storage-format-history-docs, gate-and-search-command-docs, archive-evidence-seeding |

```mermaid
flowchart LR
    N0["gate-state-last-run: Gate state names the run that produced it"]
    N1["gate-run-latest-captures: Gate captures hold only the evaluated issue's latest runs"]
    N2["gate-run-load-by-issue: Gate-run records load by issue and run id"]
    N3["gate-latest-run-reads: Latest-run readers load the run their gate state names"]
    N4["gate-verdict-provenance: Reused verdicts name their source issue and source run"]
    N5["gate-verdict-records: Reusable verdicts are found through a per-key verdict record"]
    N6["gate-run-per-issue-layout: Gate-run records live under their issue"]
    N7["gate-run-records-relayout: This repository's gate runs are rewritten into the per-issue layout"]
    N8["lease-hooks-status: Git hooks check leases through the claim status command"]
    N9["lease-production-acquire-tests: Claim tests acquire leases through the production path"]
    N10["lease-read-api: Lease readers share one read path and one activeness rule"]
    N11["lease-per-issue-records: Each lease is one record under one lease sequence"]
    N12["lease-index-validation-removal: Repository validation stops checking the claims index"]
    N13["lease-issue-addressed-ops: Renew, heartbeat and force-evict address a lease by issue"]
    N14["lease-interruption-matrix: An interrupted lease operation leaves the prior or the next lease"]
    N15["lease-records-rewrite: This checkout's leases are rewritten into lease records"]
    N16["claim-event-from-assignee: A claim emits its event exactly when the assignee changes"]
    N17["event-torn-line-rule: An unterminated final event line is an uncommitted append"]
    N18["validation-skips-event-log: Repository validation reads no event log"]
    N19["mutation-event-tail-captures: Mutations capture the event log's end, not the whole log"]
    N20["archive-execution-record-type: Archive executions are one domain record type"]
    N21["archive-evidence-records: Archive evidence is a per-target record"]
    N22["archive-evidence-seeding: This repository's archived targets get evidence records"]
    N23["event-log-reader-convergence: Event-log readers share one parser and select events by typed tag"]
    N24["capture-refuses-whole-event-log: Captures and redo records refuse whole-log access"]
    N25["search-excludes-history: Default search reads no history file"]
    N26["storage-records-per-key-reference: The storage-record reference describes verdict and archive-evidence records"]
    N27["history-isolation-test: Ordinary commands succeed over unparseable history"]
    N28["storage-format-history-docs: The storage-format reference describes per-key records and history files"]
    N29["lease-operator-docs: Lease command and recovery docs match per-issue lease records"]
    N30["gate-and-search-command-docs: Gate provenance and search docs match per-key reads"]
    N31["per-key-gate-runs: Gate runs are addressed per issue, per gate and per inputs digest"]
    N32["per-key-leases: Leases are per-issue records under one lease sequence"]
    N33["history-boundary: Only history-reporting commands read history files"]
    N0 --> N1
    N18 --> N1
    N19 --> N1
    N0 --> N3
    N2 --> N3
    N4 --> N5
    N2 --> N5
    N1 --> N5
    N3 --> N6
    N5 --> N6
    N6 --> N7
    N9 --> N11
    N10 --> N11
    N8 --> N11
    N11 --> N12
    N18 --> N12
    N11 --> N13
    N13 --> N14
    N11 --> N15
    N17 --> N19
    N17 --> N20
    N20 --> N21
    N18 --> N21
    N21 --> N22
    N16 --> N23
    N21 --> N23
    N19 --> N24
    N16 --> N24
    N21 --> N24
    N6 --> N26
    N21 --> N26
    N6 --> N27
    N13 --> N27
    N24 --> N27
    N23 --> N27
    N7 --> N28
    N12 --> N28
    N13 --> N28
    N24 --> N28
    N23 --> N28
    N25 --> N28
    N26 --> N28
    N14 --> N29
    N12 --> N29
    N7 --> N30
    N25 --> N30
    N7 --> N31
    N15 --> N32
    N29 --> N32
    N16 --> N32
    N27 --> N33
    N28 --> N33
    N30 --> N33
    N22 --> N33
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| DEC-01 Upstream 2195c98f | Chosen: each root depends on the specific 2195c98f task it consumes. Rejected: one edge to the whole epic. External dependency 79aa3aa7 → `gate-state-last-run`, `gate-run-load-by-issue`, `gate-verdict-provenance`. External dependency 417be079 → `gate-state-last-run`. External dependency 98404e0c → `lease-production-acquire-tests`, `lease-read-api`. External dependency 855d027d → `claim-event-from-assignee`, `validation-skips-event-log`. External dependency e98ae5a4 → `claim-event-from-assignee`, `event-torn-line-rule`. External dependency 3ee3a4d1 → `archive-execution-record-type`. External dependency 945f65f3 → `mutation-event-tail-captures`. External dependency 18dc6e3e → `search-excludes-history`. External dependency 2195c98f → `lease-hooks-status`, which consumes nothing specific. 8cfbf19d, a50e9be9, 0102769f, d72bf093 and 641bd14e are reached transitively, so a direct edge would be rejected as redundant. |
| DEC-02 Eviction persistence | Chosen: `lease-sequence` persists every eviction it makes, which keeps d7d0d0db's property and closes the `renew_lease` leak (`claim_coordinator.rs:1025-1046`). |
| DEC-03 Cross-epic seams | Chosen: external dependency 5b37a806 → `lease-per-issue-records` (recovery reclaim lands on the old layout first). 083c9a91 depends on `lease-issue-addressed-ops`, the final lease-API entry. b697a3f7 REQ-06 depends on `archive-evidence-records`; record this for the steward. Rejected: folding 5b37a806 in. |
| DEC-04 19e7298a REQ-02 | Chosen: reverse it. A claim no longer repairs a missing claim event (steward-approved). |
| DEC-05 Latest run | Chosen: `GateState.last_run` plus `gate-runs/<issue>/<run>.json`. Rejected: time-ordered run filenames, which read other runs' names, and a `latest.json` pointer, an extra file. |
| DEC-06 Reuse key | Chosen: `gate-verdict-record`, latest recorded wins. Rejected: scanning runs by digest. |
| DEC-07 Provenance | Chosen: `Reused` carries the source issue and run. This changes the wire shape after 79aa3aa7 types `GateRunId`. Rejected: run id only. |
| DEC-08 Existing runs | Chosen: one idempotent rewrite task, `gate-run-records-relayout`, in landing group `gate-run-cutover` with the reinstall of the dogfood binary. Rejected: deleting history, since 92 run ids are cited. No migration reader remains. |
| DEC-09 Lease layout | Chosen: `lease-record-layout`. `claims.index.json` is a second authority, not a cache (investigation C3), and is deleted. Rejected: one `leases.json`, which reads other leases. |
| DEC-10 One sequence | Chosen: `lease-sequence`. Test-only APIs are deleted first, so the property tests drive production. |
| DEC-11 Issue addressing | Chosen: renew, heartbeat and force-evict take an issue id. 8cfbf19d first types these lease-id arguments as `LeaseId`, and `lease-issue-addressed-ops` replaces them; that is where the churn falls. Rejected: a per-lease-id pointer file. |
| DEC-12 Claim event | Chosen: `IssueClaimed` exactly when the assignee changes, for both `claim acquire` and `issue claim`. Fixes acquire → unassign → acquire emitting nothing. |
| DEC-13 Limit check | Chosen: the indefinite-lease limits read current lease records only (REQ-05 amended). |
| DEC-14 Checkout leases | Chosen: one idempotent rewrite, `lease-records-rewrite`, in landing group `lease-cutover`. No reader of the old index remains. |
| DEC-15 Parser rule | Chosen: `event-log-rule`. Retired-tag records are skipped uniformly, since history is append-only (REQ-12) and this log holds 7 `document_archived` records, plus 1 `lifecycle_timestamps_backfilled` once 2492ab29 retires its tag. Rejected: erroring on retired tags, which would require rewriting history. The marker protocol is removed. Kept: `@/issue/c88822a5/decision/D-1`. d72bf093 REQ-03 (byte-identical re-serialization of this log) is unaffected: no line here carries `isolated_torn_tail`. |
| DEC-16 Content | Chosen: `event-append-probe`. The bounded probe is allowed, and no record is parsed. |
| DEC-17 Archive evidence | Chosen: `archive-evidence-record`, plus one seeding task in landing group `archive-evidence-cutover`. Rejected: evidence on the `.jit-container` marker or on the issue record. |
| DEC-18 Search | Chosen: exclusion by construction of the default selection from `history-paths`. Rejected: running `rg` from the data root, which changes result paths. |
| DEC-19 History readers | Chosen: `jit events`, `jit worktree store-divergence`, `gate status --all` and `--limit`, and the server gate-run list. Validation drops its log parse, and the whole-log mutation captures move to the probe. |
| DEC-20 REQ-14 test | Chosen: a module in `cli_repo_workflow` with one shared file-backed fixture and no new target (`@/inv/bounded-rust-build-footprint`). |
| Investigation seams | External dependency 72d89161 → `gate-latest-run-reads`. 47a082ef (memory parser), 2492ab29 (migrate removal), 86d3625d (`reusable_verdict` removal) and d759d606 (claim command signatures) are children of 842b0ac6, on which every 2195c98f root depends, so every entry here already follows them and a direct edge would be rejected as redundant. Reverse edge: 8673dc5e depends on `gate-run-per-issue-layout`; add it with `--reduce`, because it makes 8673dc5e → 72d89161 redundant. 083c9a91 → `lease-issue-addressed-ops` likewise needs `--reduce` (it subsumes 083c9a91 → d759d606). |
| Hook lease checks | Chosen: hooks call `jit claim status`, giving one activeness computation (`@/charter/D-20`). This lands before the layout, so no hook ever reads a lease file. Rejected: jq over lease files, which cannot compute staleness. |
| Landing groups | `gate-run-cutover` (captures, latest reads, provenance, verdicts, layout, rewrite), `lease-cutover` and `archive-evidence-cutover`. Each lands with its data task and a binary reinstall. An older binary reads the new tree as having no runs, leases or evidence. |
| Shared-file contention | `commands/gate_check.rs`, `commands/mod.rs`, `storage/claim_coordinator.rs`, `storage/json.rs`, `storage/memory.rs` and `repository_state/mutation.rs` are edited by several entries. Edges order every same-function edit: gate-run entries chain through the layout, lease entries chain through the sequence, and event entries chain through the parser and the capture refusal. The remaining footprint advisories are distinct functions; workers rebase. |

## Investigation sources

- [Investigation](investigation.md) holds the consumer sweeps, primitive verification and option analyses for gate runs, leases and claims, and the event log, archive, search and history boundary.
