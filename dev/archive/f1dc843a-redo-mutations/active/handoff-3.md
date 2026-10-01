# Handoff — Mutations commit through a redo record and never scale with project history (f1dc843a) — session 4

**Date:** 2026-09-21
**Session number:** 4 (this file is `handoff-3.md`: the index is one past the
highest existing handoff, and **session 3 left no handoff**. Session 3's record
is `session_3_summary` in `progress.json` — it opened and closed wave 7.)
**Prior handoffs:** `handoff.md` (session 1), `handoff-2.md` (session 2)

## Current state

- Epic: `f1dc843a` — state: backlog (assignee `agent:jit-execution-lead`)
- Wave in progress: **wave 8 of 23** — not yet dispatched. `8d7f4cec` is `ready`.
- Children summary: **20 done, 1 ready (`8d7f4cec`), 27 backlog** of 48 (the 48 counts the epic itself and the two bracket nodes)
- Active claims: **none** (`jit claim list` reports 0 leases)
- Open escalations: **none**
- Progress file: `progress.json` in `dev/active/f1dc843a-redo-mutations/`
- `main` is clean at `1fce64b78`, `cargo-ci` green, `docs-mechanical` green.

## What just happened

This session delivered **no epic waves**. It did the user-requested build-time
work that session 3 was going to do, and measured it.

- **POL-03 landed** (`1fce64b78`): the gate's `CARGO_INCREMENTAL=0` ban is gone,
  with all its enforcement sites, its two gate steps, its three selftest
  scenarios, and every document that stated it. POL-03 named *three* enforcement
  points; there were **four** — `scripts/cargo-ci-selftest.sh` was the fourth.
- Measured payoff, one file under `crates/jit/src/storage/` touched: clippy
  29.3 s → 16.7 s, suite-build 25.1 s → 8.6 s, **full gate 79 s → 56 s**. Cold
  build unchanged (99.5 s → 99.1 s). Suite-clock unchanged across two no-change
  reruns (22.40 / 22.34 s against a 22.33 s baseline), so the budget derivation
  in `dev/benchmarks/rust-build-budgets/README.md` needed no revision.
- **PF-11's diagnosis was wrong about the headline number.** The gate is 78 s
  warm / 128 s cold on an idle host. The "~8 minutes per worker run" was the
  host-wide `flock` serializing a four-worker wave — queue time, not compile time.
- **Measured and rejected: `lld`.** No change on any step. Both arms already link
  with LLD — Rust defaults to `rust-lld` on `x86_64-unknown-linux-gnu`.
- **Measured and rejected: raising the flock to 2 slots.** Two concurrent runs
  took 88 s against 110 s serial (20 %), and **both failed the suite-duration
  budget** (38.3 s / 33.3 s against 30 s). The budget is defined over an idle
  host, so concurrency turns parallel waves red on a checker that says nothing
  about the tree — the exact defect `jit:0708d692` removed. **Keep the flock.**
- **Measured and rejected: tmpfs for worker targets.** Storage medium is not the
  bottleneck: main's target on SATA 79 s, an isolated target on NVMe 78 s.
  `MemAvailable` never fell below 48.06 GB of 62 during a full gate run.
- **sccache: corrected twice, see Traps.** It is **not** broken and **not**
  useless. Its Rust cache key is bound to the absolute target-directory path.
  Same path: 37 s → 7 s, 244/244 Rust hits. Different path: 0.00 % hits, always.
  Every worktree has its own path, so it misses 100 % of the time in the only
  case it was installed for, at ~3 s per cold build and 73 GB of disk.
- Nothing from the investigation is committed except POL-03. All experiment
  target directories under `/data` were removed.

## What to do next

- [ ] **Standing instruction from the user (POL-04, recorded in `progress.json`):
      actively watch how `cargo-ci` behaves and watch for throughput bottlenecks
      in parallel execution.** Concretely, every wave: record each gate run's
      per-step timings and its `suite-clock` in the wave's progress entry, and
      record the wall time from first dispatch to last merge. Flag any
      `suite-clock` above 25 s and any step that moves more than 30 % from the
      figures in "What just happened". Do not let this become a second
      measurement project — it is observation carried alongside the waves.
- [ ] **Open wave 8: `8d7f4cec`** (Publication failure injection repoint).
      Single-issue wave, `ready`, gates `cargo-ci` + `code-review`. Opus tier.
      Before dispatch: `jit doc add 8d7f4cec dev/active/f1dc843a-redo-mutations/plan.md --doc-type design`
      — `jit doc list 8d7f4cec` is currently **empty**, and session 1's trap
      records a review round lost to exactly that omission.
- [ ] Wave 9 (`0938d99e`) depends on `8d7f4cec`; wave 10 (`4319256e`) on
      `0938d99e`; wave 11 (`10ba2e35`); wave 12 (`b0ee2dd6`). The spine is
      strictly serial from here to wave 13, which is the next 4-way fan-out
      (`ccea63de`, `ead8f86d`, `79252d23`, `e6e25f9a` — all documentation).
- [ ] Carry **ER-01** into `722878d4` (wave 21). The exclusion list has grown:
      `ce053478`, `19e7298a`, `2d4fda72`, `48e4f65c`, `7226f00b`, **and this
      session's POL-03 commit `1fce64b78` (−139 Rust lines)**. `8559f529` is NOT
      excluded. Reuse `footprint.md`'s method; `footprint.py --tasks` exists.
- [ ] `c88822a5` (wave 16) carries three implied consequences in its Notes —
      per-path read mode in `CaptureSpec`, a captured entry carrying length+tail,
      `expected_of` no longer deriving the log preimage from a whole-log digest.
      Verify all three at review; they are the load-bearing part of epic REQ-02.
- [ ] `8b6a9e34` (wave 22) still owns SP-01's live half (`dev/TESTING.md` citing
      an archived benchmark path plus the `transaction-benchmark-evidence`
      contract LD-4 retires). **Note:** this session rewrote `dev/TESTING.md`'s
      incremental section; re-read the file before assuming SP-01's line numbers.
- [ ] SP-07 (`ELOOP` vs `Unaddressable`) remains unclosable by design; it
      surfaces in the completion report.

## Traps — do not repeat these

Session 1's eleven traps and session 2's ten remain in force in full
(`handoff.md`, `handoff-2.md`). None was resolved this session. New traps:

- **Do NOT delete a line range from a Rust file without diffing its top-level
  item list.** Removing the two gate-ban tests from
  `build_profile_policy_tests.rs` by `sed -i '116,218d'` also took
  `const SUITE_CLOCK_VARIABLE`, which two *later* tests use. The gate failed
  with three `E0425`s. **Diff the item inventory before and after:**
  `diff <(git show HEAD:<file> | grep -E '^(fn|const|static|impl|struct|#\[test\])') <(grep -E '^(fn|const|static|impl|struct|#\[test\])' <file>)`.
- **Do NOT run two things that write the same output file.** Two gate runs were
  redirected to one `verify.txt`; the second truncated it, the first then
  appended its summary, and the file ended up showing run 1's verdict under
  run 2's timing line. The exit codes were right and the file was a lie. Give
  every run its own output path.
- **Do NOT start a second `cargo-ci` before the first has exited.** The first
  run kept the host `flock` while finishing its remaining steps after a compile
  error, so the second queued behind it and the wave lost ~4 minutes. A failing
  step does not end the run — `cargo-ci.sh` continues through failures by design.
- **Do NOT read an sccache hit rate from `--show-stats` without checking the
  server's age.** The counters reset when the server restarts on idle timeout.
  A "3 % hit rate" was read from a freshly-started server and used to call
  sccache dead weight; the A/B test later showed the real behaviour is 100 % or
  0 % depending on target-dir path. **Stop and restart the server, run a known
  workload, then read the counters.**
- **A cold-target gate run can report `suite-clock` above 40 s without anything
  being wrong with the tree.** Two runs in freshly seeded target dirs reported
  43.0 s and 43.3 s and failed the budget; the same targets, once settled,
  reported 55 s per full run and passed. This is LD-7's signature, worse than the
  31.8 s previously recorded, because the page cache was under pressure from the
  experiment itself. **Re-run once warm before treating a suite-duration failure
  as a verdict about the change.** This is a live risk for a worker's *first*
  gate run in a fresh worktree — watch it under POL-04.
- **Do NOT assume a linker or compiler-cache change is inert because a
  measurement says so, without checking the tool was actually engaged.** The
  `lld` arm measured identical to baseline; the reason was not that lld is slow
  but that `rust-lld` was **already the default**. `readelf -p .comment <exe>`
  reports `Linker: LLD ...` and settled it in one command.
- **Do NOT state a mechanism for a measurement you have not isolated.** Two
  wrong mechanisms were asserted about sccache this session before the right one
  was measured: (a) that `RUSTC_WRAPPER` was splitting Cargo fingerprints between
  gate and interactive builds — checked and **not supported**, all `libc`
  fingerprints carried an identical `rustc` hash; (b) that incremental
  compilation poisoned the sccache key — **refuted**, `CARGO_INCREMENTAL=0` still
  gave 0.00 % Rust hits. The actual mechanism is absolute-path binding, proven by
  two cold builds at the *same* `CARGO_TARGET_DIR` (0 % then 100 %).
- **The auto-mode classifier refuses edits that remove CI enforcement.** The
  POL-03 deletion was blocked as "CI Bypass" when attempted as one scripted
  multi-cut edit. A recorded user decision in `progress.json` is **not** live
  authorization. Get the user's confirmation in-session, then edit file by file.

## Open questions needing invoker input

- Question: **Does `crates/jit` get split along its layer boundaries, and if so,
  when?**
  - Context: the user has put project restructuring on the table. `crates/jit` is
    197,195 src lines in one compilation unit. The gate is now 56 s: ~25 s
    compile, 22.3 s test execution, 4.5 s doctest, 1.6 s fmt. A split attacks
    only the 25 s, and a `storage/` edit would still rebuild `commands/`
    (53,604 lines, the largest layer) and the CLI — roughly a third of the crate
    would be skipped, not most of it. Layer sizes: `domain/`+`graph/` 22,003;
    `validation/`+`query_engine/` 11,852; `storage/` 29,232; `commands/` 53,604;
    `main.rs`/`cli.rs`/`output.rs`/`config.rs`/`schema.rs` 23,715.
  - Options: (A) profile the suite (`scripts/profile-test-suite.sh` exists) and
    measure the split's real upside before deciding; (B) split now; (C) defer
    past the epic.
  - Recommendation: **A, and not B now.** The epic has 27 open issues concentrated
    in `storage/` and `commands/` — the two layers a split would move — so B
    collides with every remaining wave, which is what PF-11 already concluded. The
    suite is now the larger term of the two, so measuring it first is what tells
    you whether a split is even the right lever.

## Reference artefacts

- Epic: `jit issue show f1dc843a`
- Plan: `dev/active/f1dc843a-redo-mutations/plan.md` (contracts, OD-1..OD-10, LD-1..LD-11 — **LD-11 places kernel retirement in the cutover phase**)
- Investigation: `dev/active/f1dc843a-redo-mutations/investigation.md` (long — grep, do not read whole)
- Breakdown manifest: `dev/active/f1dc843a-redo-mutations/breakdown.json`
- Progress + all findings: `dev/active/f1dc843a-redo-mutations/progress.json` (PF-11..PF-13, POL-03/POL-04, ER-01)
- Footprint method and REQ-06 evidence: `dev/active/f1dc843a-redo-mutations/footprint.md`, `footprint.py`
- **Build-time derivation (this session):** `dev/benchmarks/rust-build-budgets/README.md`, "The gate's incremental ban, retired 2026-09-21"
- REQ-02 measurement (SP-05): `dev/active/7226f00b/whole-log-read-cost.md`
- Benchmark harness: `dev/active/f1dc843a-redo-mutations/bench.sh`
- Conformance suite: `crates/jit/src/storage/redo_conformance.rs`
- REQ-06 baseline: `git diff --shortstat 34d37f8be -- '*.rs'`
