# Checkpoint e590fa91 — superseded design retirement: evidence

Story `story:superseded-design-retirement`; children 3d9150d7, 51eb1aa6, b4483ad6,
c88822a5, ec72359b, 33f97287, 8454236c, e346d387, 816df12a, 91168745, be13da79.
Construct map: the epic investigation, §"Claim 9 — REQ-05 construct mapping" (`investigation.md` in the epic artifact directory).
Commands run from the repository root at the HEAD this document is committed in.

## REQ-01 — the ten constructs, row by row

The retired modules are gone:

```console
$ git ls-files crates | rg 'file_transaction|transaction_journal|transaction_recovery|transaction_staging'
$
```

Every construct-map row, counted over `crates mcp-server scripts docs web/src`:

```sh
for t in FileTransactionKernel rollback_repository_actions rollback_repository_action \
  prepare_repository_backup restore_verified_backup remove_redundant_backup backup_authority \
  '"backups"' 'backup: ControlName' transaction_staging stage_bytes stage_authority \
  prepare_staged_data_action create_directory_stage_name '"stages"' data_stage \
  RepositoryBeforeReverseAction RepositoryAfterReverseAction COMPANION_MARKER CompanionDirs \
  create_companion_control open_companion_control remove_companion_control_if_identity \
  sweep_orphan_companions needs_worktree_companion RepositoryBeforeReadSetRevalidation \
  FailurePoint plan_hash PlanHashError FileTransactionOutcome RepositoryTransactionJournal \
  phase_one image_two phase_three has_stable_overlap ListingFingerprint listing_fingerprints \
  document_capture_closure DocumentClosure 'enum DocumentCapture'; do
  printf '%-38s %s\n' "$t" "$(git grep -F -c -e "$t" HEAD -- crates mcp-server scripts docs web/src \
    | awk -F: '{s+=$NF} END{print s+0}')"
done
```

| Term | Construct (original location) | Hits at HEAD | Result |
|---|---|---:|---|
| rollback kernel | `FileTransactionKernel` (`file_transaction.rs:68`) | 0 | absent |
| | `rollback_repository_actions` (`:1669`) | 0 | absent |
| | `rollback_repository_action` (`:1699`) | 0 | absent |
| transaction backup | `prepare_repository_backup` (`:1302`) | 0 | absent |
| | `restore_verified_backup` (`:1779`) | 0 | absent |
| | `remove_redundant_backup` (`:1800`) | 0 | absent |
| | `backup_authority` (`:285`) | 0 | absent |
| | `"backups"` control subdirectory | 0 | absent |
| | `backup: ControlName` journal fields (`transaction_journal.rs`) | 0 | absent; module gone |
| staging directory | `storage/transaction_staging.rs` (`stage_bytes`) | 0 | absent; module gone |
| | `stage_authority` (`:277`) | 0 | absent |
| | `prepare_staged_data_action` (`:1345`) | 0 | absent |
| | `create_directory_stage_name` (`:1229`) | 0 | absent |
| | `"stages"` control subdirectory | 0 | absent |
| | `data_stage`, `data_stage_identity` journal fields | 0 | absent |
| reverse action | `FailurePoint::RepositoryBeforeReverseAction` / `RepositoryAfterReverseAction` (`transaction_recovery.rs:90,93`) | 0 | absent; `FailurePoint` itself has 0 hits |
| | the work: `rollback_repository_action(s)` | 0 | absent (above) |
| | prose (`file_transaction.rs:5,1665,3198`) | — | module gone |
| | `ReverseAcquisitionError` (`storage/guard_order.rs`) | live | unrelated: lock-acquisition order |
| worktree companion | `COMPANION_MARKER` (`:41`) | 0 | absent |
| | `CompanionDirs` (`:269`) | 0 | absent |
| | `create_companion_control` / `open_companion_control` / `remove_companion_control_if_identity` | 0 / 0 / 0 | absent |
| | `sweep_orphan_companions` (free and kernel method) | 0 | absent |
| | `needs_worktree_companion` | 0 | absent |
| apply-time revalidation of the whole read set | block in `JsonMutationSession::apply` (`repository_state_store.rs:516-535`) | — | behaviour gone, see below |
| | mirror in `MemoryMutationSession::apply` (`:613-621`) | — | behaviour gone, see below |
| | `FailurePoint::RepositoryBeforeReadSetRevalidation` | 0 | absent |
| plan hash over captured entries | `plan_hash` (`image.rs:1581`) | 0 | absent |
| | `PlanHashError` (`image.rs:1715`) | 0 | absent |
| | `MaterializationPlan.hash` / `hash()` (`repository_state/mod.rs`) | 0 | `rg -n 'pub fn hash\|hash:' crates/jit/src/repository_state/mod.rs` is empty |
| | `FileTransactionOutcome.plan_hash` | 0 | absent |
| | `RepositoryTransactionJournal.plan_hash` | 0 | absent |
| phased capture | `CaptureSpec::phase_one` (`image.rs:628`) | 0 | absent |
| | `image_one` / `image_two` / `phase_three` in `capture_proposed_base_inner` | 0 / 0 / 0 in production | absent; one `image_one` local remains in `tests/fast_rules/derived_state_repair_tests.rs:138`, a single image, unrelated |
| | `RepositoryImage::has_stable_overlap` (`image.rs:978`) | 0 | absent |
| listing fingerprint | `ListingFingerprint` (`image.rs:226`) | 0 | absent |
| | `RepositoryImage::listing_fingerprints()` | 0 | absent |
| | `RepositoryImage.listings`, `CaptureSpec::discover_listing(s)`, `listings()` | live | now a `DirectoryListing(BTreeSet<String>)` of child names (`image.rs:324-328`): the complete-listing read, carrying no fingerprint |
| mutation document closure | `document_capture_closure` (`validate.rs:1300`) | 0 | absent |
| | `DocumentClosure` (`validate.rs:1247`) | 0 | absent |
| | `DocumentCapture` enum (`validate.rs:1280`) | 0 | absent |
| | `validate_capture_closure` / `ValidationCaptureClosure` (`materialize.rs`) | live | unrelated: the whole-repository validation closure, which stays |

**Apply-time revalidation, by behaviour.** `JsonMutationSession::apply`
(`crates/jit/src/storage/repository_state_store.rs:552`) runs
`ensure_session_image`, `ensure_delta_is_captured` (each action's `ExpectedPreimage`
against the session's own image — no live read), `revalidate_layout_capabilities`
(root identity only), then `publish_through_record` or `publish_absent_data_root`.
Both publishers call `ensure_write_set_unchanged` (`:884`), whose shared body
`ensure_write_set_unchanged_against` (`:923`) reads live only
`record.make_directories()`, `record.puts()` and `record.deletes()`, and the event
log as length plus tail. No path the mutation read without writing is reread.
`MemoryMutationSession::apply` (`:718`) follows the same order through
`ensure_memory_write_set_unchanged` (`:898`). `ExpectedPreimage` is the live
write-target conflict check `@/issue/e346d387/decision/D-1` keeps.

**Phased capture, by behaviour.** `capture_proposed_base_inner`
(`commands/validate.rs:335`) grows one image through `extend`;
`test_capture_mutation_base_reads_in_one_round` (`commands/validate.rs:4631`)
counts the session's capture rounds and owns the one-round property.

## REQ-02 — one publication path beside the redo record

Source guard: `crates/jit/tests/scratch_build/repository_state_cutover_tests.rs`.

- `REPOSITORY_STATE_PUBLISHERS` (`:697`) names exactly three definitions:
  `pub struct RedoRecord {`, `pub struct RedoApplier {` (the redo record, written
  and applied) and `fn publish_staged_data_root(` (`storage/init_publish.rs`), with
  the reason "an absent data root cannot be created through the record, whose home
  is a scratch directory inside that root".
- `NON_REPOSITORY_WRITES` (`:720`) names the nine classes whose target lies outside
  repository state.
- `scan_filesystem_writers` (`:867`) classifies every production function that
  mutates the filesystem; one in no publisher module and no class is unclassified.
- `test_repository_state_is_written_only_by_the_redo_record_and_the_absent_root_publisher`
  (`:977`) fails on any unclassified writer.
  `test_repository_state_writer_scan_flags_unclassified_writers_wherever_their_path_comes_from`
  (`:990`) is its red-first proof over source fragments.

Demonstration of the failure mode: appending this third publisher to
`crates/jit/src/commands/mod.rs`, then reverting it (`git checkout -- crates/jit/src/commands/mod.rs`):

```rust
pub fn scratch_third_publisher(data_root: &std::path::Path) {
    let _ = std::fs::write(data_root.join("index.json"), b"{}");
}
```

```console
$ cargo test -q -p jit --test scratch_build test_repository_state_is_written_only_by
running 1 test
repository_state_cutover_tests::test_repository_state_is_written_only_by_the_redo_record_and_the_absent_root_publisher --- FAILED
...
filesystem writes to an unclassified target in {"crates/jit/src/commands/mod.rs::scratch_third_publisher"}; a mutation reaches the repository's roots only through its session's redo record (`@/charter/D-17`), and any other write names the non-repository class it belongs to
...
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 42 filtered out; finished in 1.52s
```

After the revert:

```console
$ cargo test -q -p jit --test scratch_build test_repository_state_
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out; finished in 1.54s
```

## REQ-03 — no declaration, projection, output, schema or bridge names a retired mechanism

Terms: journal, rollback / roll back, backup, staging, reverse action, companion,
transaction, recovered session, plan hash, fingerprint, phase one / three, overlap.

```console
$ rg -n -i 'journal|roll ?back|backup|staging|stage|reverse action|companion|plan.hash|fingerprint|phase one|phase three|overlap|transaction|recovered' \
    .jit/config.toml .jit/invariants.toml .jit/rules.toml .jit/gates.toml .jit/templates.toml .jit/reference .jit/schemas | cut -c1-120
.jit/gates.toml:6:stage = "postcheck"            (and 19 more gate `stage =` keys)
.jit/invariants.toml:43:statement = "File replacements use the temp-file and atomic-rename pattern, and the event log grows by a positioned append at the length the mutation read. A mutation commits when its redo record is synced, ...
```

Both are current vocabulary: a gate's `stage` key, and `@/inv/atomic-writes`
stating the redo record. No `.jit/` edit is needed.

Generated projections and adopter docs:

```console
$ rg -n -i 'journal|roll ?back|backup|reverse action|companion|recovered session|plan hash|listing fingerprint|phase (one|three)|stable overlap|transaction' AGENTS.md CLAUDE.md docs/
```

Every hit is unrelated: gate examples ("rollback plan"), operator tar backups
(`docs/how-to/deployment.md`, `troubleshooting.md`), "companion `templates.toml`"
in example rule comments, `journalctl`. `docs/how-to/deployment.md:205` states a
restored root "carries no pending redo record" — current vocabulary.

Command schema and help (built from this HEAD):

```console
$ target/debug/jit --schema | rg -i -c 'journal|roll ?back|backup|staging|reverse action|companion|transaction|recovered session|plan.hash|overlap|phase one|phase three'
$ echo $?
1
$ target/debug/jit --schema | rg -i -o '[a-z_ ]*fingerprint[a-z_ ]*' | sort -u
base_fingerprint
file asset by its region identifier and fingerprinted over its source body and mode
 historical fingerprints
 Its value is fingerprinted
separated fingerprint of the contribution when it was published
s mode belongs to its base fingerprint rather than its identity
$ target/debug/jit recover --help | head -1
Run recovery: apply a leftover redo record, then the repair routines
```

Before this checkpoint the schema carried "Whether an application published a
transaction." five times and "Whether the capture published a transaction." once
(`ProfileApplicationStatus` rustdoc, `crates/jit/src/profile/application.rs`); fixed
in b6b84dbbb. The remaining `fingerprint` hits are the profile ownership claim's
`base_fingerprint`, a live and unrelated concept. `crates/jit/tests/cli_issue/recover_command_tests.rs:178`
owns the absence of the retired `transactions_recovered` output key.

MCP bridge: `mcp-server/index.js` loads tool definitions from `jit --schema` at
runtime, so the schema fix reaches it. Its hand-written text:

```console
$ rg -n -i 'journal|roll ?back|backup|staging|reverse action|companion|transaction|recovered session|plan.hash|fingerprint|overlap' mcp-server --glob '!node_modules'
$
```

`mcp-server/index.js:66` ("jit_recover cleans up stale locks and corrupted state")
names no retired mechanism and matches `jit recover --help`.

## REQ-04 — one mechanism in contributor material and Rust prose

Fixed in b6b84dbbb (SP-29 and the sweep it led to): every Rust site naming the
publication a "transaction" now names the mutation session, its redo record, or
the publication — including `storage/mod.rs:1`, `commands/mod.rs:3209`,
`main.rs:230,2574,2582,2588`, module docs of `json.rs`, `gate_store.rs`,
`ruleset_store.rs`, `rule_serialize.rs`, `projection.rs`,
`rules_gates_projection.rs`, `initialize.rs`, `package_capture.rs`. The store's
catch-all `RepositoryStateStoreError::Transaction` is now `Publication`,
`execute_archive_transaction` is `execute_archive_target`, and four test names
lose the word. `dev/TESTING.md:398` now states a redo-record round trip instead of
"the storage journal".

```console
$ rg -n -i 'transaction|journal|roll ?back|rolled back|backup|companion|reverse action|plan hash|listing fingerprint|stable overlap' crates --type rust | rg -v 'FILE_FLAG_BACKUP|gate_check.rs:1425|default_rules.rs:318|label_filter_repeatable_query_tests.rs:5'
crates/jit/tests/cli_issue/recover_command_tests.rs:178:        value.get("transactions_recovered").is_none(),
```

The excluded hits are the Windows `FILE_FLAG_BACKUP_SEMANTICS` flag and three
generic uses of "companion"; the one remaining line asserts the retired key's
absence.

SP-19 is already closed, by 72afc1f23 (`jit:b7c99dcb`): no guard, constant, test or
prose reserves `.jit-bootstrap` or `.jit-bootstrap/*`
(`rg -n 'jit-bootstrap[^.]|jit-bootstrap"' crates/` is empty). The live
`.jit-bootstrap.lock` (`repository_state/path.rs:16`, `BOOTSTRAP_LOCK_FILE`) is the
worktree bootstrap lock serializing sessions that share one worktree over the init
publisher's sibling stage; `is_bootstrap_lock` (`:28`) refuses it as a mutation
target because publication by stage-and-rename would replace the inode its holders
lock. It is unaffected.

`dev/` sweep (excluding `dev/archive/`, `dev/active/`, and measured benchmark
artifacts): `dev/architecture/repository-state-materialization.md` hits are the
current mechanism (root-capability revalidation, init's stage directory);
`dev/vision/9db27a3a-charter.md:306-357` is `@/charter/D-17` naming what it
supersedes; `dev/architecture/cli-and-mcp-strategy.md:308,637` (a batch `--atomic`
idea), `dev/studies/multi-agent-parallelism-analysis.md` (an `updated_at`
rollback bug) and `dev/benchmarks/rust-build-efficiency/report.md:30` (a harness
file backup) are unrelated.

The journal-shaped benchmark harness is retired here, delivering 8b6a9e34 REQ-02
early: `scripts/benchmark-transaction.sh` is deleted (its Rust helper and
`LD_PRELOAD` fsync shim were heredocs inside it, so no other file existed for it),
and `dev/TESTING.md` no longer cites it, the `transaction-benchmark-evidence`
contract, or the publication-cost figures sourced from that artifact. No test,
gate or docs-mechanical policy referenced either.

```console
$ rg -n "benchmark-transaction|transaction-benchmark|TRANSACTION_BENCH" -g '!dev/archive/**' -g '!dev/benchmarks/**' -g '!dev/active/**' .
$ echo $?
1
```

Under `dev/active/` the names remain only in the epic's planning record
(`f1dc843a-redo-mutations/plan.md` LD-4, `investigation.md`, `breakdown.json`,
`progress.json`, the handoffs), which records the decision to retire them.

Reading order (`dev/architecture/repository-state-materialization.md` §"Where to
start reading"):

```console
$ cd crates/jit/src && ls repository_state/mod.rs repository_state/image.rs repository_state/materialize.rs \
    repository_state/projection.rs repository_state/rules_document.rs repository_state/mutation.rs \
    storage/repository_state_store.rs storage/redo_record.rs storage/redo_apply.rs storage/init_publish.rs
repository_state/image.rs
repository_state/materialize.rs
repository_state/mod.rs
repository_state/mutation.rs
repository_state/projection.rs
repository_state/rules_document.rs
storage/init_publish.rs
storage/redo_apply.rs
storage/redo_record.rs
storage/repository_state_store.rs
```

Every file exists, as do the other files the note cites
(`storage/mutation_interruption.rs`, `storage/redo_conformance.rs`,
`commands/mod.rs`, `commands/validate.rs`, `commands/profile.rs`).

## REQ-05 — build, suite, test-target budget

Run at 936b921a5 (the prose fixes plus the REQ-06 script). clippy runs with
`-D warnings`; `excluded-tests` enforces no skipped or ignored test; `budget`
enforces the integration-target count. A first run on a cold build failed only
because the `recorded-failure-corpus` nextest setup script timed out at 60 s while
waiting for the build-directory lock; the immediate rerun is below.

```console
$ BUSY_HOST_OVERRIDE=1 scripts/cargo-ci.sh
  ✓ fmt: ok (1608 ms)
  ✓ clippy: ok (198 ms)
  ✓ suite-build: built in 510 ms, warmed 17 executables (2070725936 bytes) in 72 ms (585 ms)
  ✓ test: 4711 passed, 0 failed, 0 skipped (36969 ms)
  ✓ doctest: 63 passed, 0 failed, 0 ignored (4851 ms)
  ✓ suite-clock: 41838 ms
  ✓ excluded-tests: excluded-tests: nextest skipped=0, doctest ignored=0 (8 ms)
  ✓ budget: integration-targets=10/12 active-executables=14 bytes=1739334912/2147483648 profile=line-tables-only dep-features=pruned suite-duration=41838ms (80 ms)
```

## REQ-06 — net Rust lines over the delivered children

`dev/active/e590fa91/phase-net.py` sums `git show --numstat` `*.rs` rows over each
child's non-merge commits tagged `jit:<child>`; `.jit/`-only commits touch no `*.rs`
path and contribute zero. Measured at 2b86baff8 (the checkpoint's anchor):

```console
$ python3 dev/active/e590fa91/phase-net.py 2b86baff8
child      commits    added  removed      net
3d9150d7        10      732      264     +468
51eb1aa6        11       19      137     -118
b4483ad6        12      365      279      +86
c88822a5        21     2125     2157      -32
ec72359b        10      177     1223    -1046
33f97287         4      176      469     -293
8454236c         8      150      164      -14
e346d387         8       78       79       -1
816df12a         7        0        0       +0
91168745         6       21       15       +6
be13da79         4        0        0       +0
total                  3843     4787     -944
REQ-06 MET: removes more than it adds
```

Production-only comparison (`footprint.py`, test code excluded, base 34d37f8be):

```console
$ python3 dev/active/f1dc843a-redo-mutations/footprint.py --tasks 3d9150d7,51eb1aa6,b4483ad6,c88822a5,ec72359b,33f97287,8454236c,e346d387,816df12a,91168745,be13da79
  91168745      +6
  b4483ad6      +5
  e346d387      +2
  c88822a5     -28
  8454236c     -56
  51eb1aa6     -65
  33f97287    -170
  ec72359b    -496

  subtotal over 11 tasks: -802
  no commits found for: 3d9150d7, 816df12a, be13da79
```

3d9150d7 (source guards) touched only test code; 816df12a and be13da79 touched no
Rust. The checkpoint's own prose commit is outside both figures.
