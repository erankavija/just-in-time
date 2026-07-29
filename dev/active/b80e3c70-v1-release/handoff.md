# Handoff — Version 1.0 release (`b80e3c70`) — session 2

**Date:** 2026-07-29

**Prior handoff:** session 1, superseded by this file

**Main baseline before this handoff commit:** `b63835b1`

**Progress authority:** `dev/active/b80e3c70-v1-release/progress.json`

## Current state

- Epic `b80e3c70` remains `backlog`, assigned to `agent:jit-execution-lead`, and blocked by sink child `bb03df0a` as designed.
- Execution is paused in wave 1, sub-wave 1a. Wave 1b and later waves have not been dispatched.
- Six of the eight wave-1a issues are Done:
  - `7edd2fe8` — required CI on every PR
  - `8fa7261b` — web production dependency advisories
  - `bb302dbd` — MCP production dependency advisories
  - `f289ff18` — pinned document link checking
  - `eeee8a1a` — docs checker default footprint
  - `985ab96d` — removed unused plan-doc resolver
- Two wave-1a issues are still `in_progress` with passing non-review gates and failed code review after the configured two rework rounds:
  - `f04f7888` — graceful server shutdown
  - `c7f8ebc7` — archival citation-warning boundaries
- Main is clean. All four rework worktrees were verified clean and fully merged, then removed; their branches remain available.
- The only other registered worktrees are the pre-existing `lead-install-clean` and `steward-v1-readiness` worktrees.

## Exceptional rework approved

The execution-lead policy permits two normal rework rounds. Both issues below reached that limit with distinct, substantive findings. On 2026-07-29 the invoker explicitly authorized one exceptional third round for each issue.

### `f04f7888`

- `cargo-ci`: passed at merge `c61439c7`.
- Final `code-review`: failed.
- Remaining finding: the observer starts its deadline before `axum-server` starts the handle's own drain timer. If the serving task observes the notification late, the observer can report `ForcedClosed` and log a forced-close count while the connection is still inside the handle's legitimate drain window.
- Prepared resolution: remove the second deadline domain. Call `handle.graceful_shutdown(None)` to stop acceptance and begin indefinite graceful draining, own the one configured timeout in JIT, sample `connection_count` at that exact boundary, and call public `handle.shutdown()` only when survivors remain. This makes the sampled count and force-close action share one boundary.
- Approved execution: one exceptional third rework round using a frontier model at high reasoning because the fix must align two asynchronous deadline domains and add deterministic timing coverage.
- Alternatives: explicitly accept the inaccurate log/count, or defer/remove the issue from the v1.0 epic. Accepting the known defect is not recommended.

### `c7f8ebc7`

- `cargo-ci`: passed at merge `55beb5b7`.
- Final `code-review`: failed.
- Remaining finding: `is_citation_start` treats every prefix ending in `:-` as shell-default syntax, so a longer unrelated path such as `notes:-dev/active/design.md` still produces a moving-source warning, violating REQ-04.
- Prepared resolution: recognize shell defaults only in proven parameter-expansion context such as `${NAME:-...}`, with explicit counterexamples for raw `notes:-...`, concatenated longer paths, malformed/unclosed expansions, and valid variable-name forms.
- Approved execution: one exceptional third rework round using a focused high-reasoning implementation model. Shell-default recognition must prove actual parameter-expansion context rather than accepting the raw two-character suffix.
- Alternatives: explicitly accept the false-positive edge case, or defer/remove the issue from v1.0. Accepting a known REQ-04 violation is not recommended.

## User-directed archive-layout finding

The concern about duplicated directory depth is confirmed production behavior, not merely a defensive c7 example:

- `artifact_mirror_destination(destination_root, source)` appends the entire repository-relative source path.
- With this repository's `development_root = "dev"` and `archive_root = "dev/archive"`, outputs include `dev/archive/<container>/dev/active/plan.md`.
- Tests and documentation intentionally describe this as an on-disk mirror.
- Searches for “archive destination”, “mirror source path”, and “duplicated development root” found no open issue that owns correcting the shape.

This is recorded under `surfaced_pitfalls` in `progress.json` and now owned by issue `625cc07f`, **Strip the development root from container archive destinations**. Its description was reviewed against `.jit/reference/content-standards.md` before commit. The graph orders `c7f8ebc7 → 625cc07f → ef118aea`, so citation parsing stabilizes first and the later execution/reporting work validates the corrected canonical layout. Do not let c7's matching logic normalize or endorse the current shape.

## Validation and evidence notes

- Every implementation/rework merge was followed by `scripts/verify-commit-builds.sh` using the workspace-backed verifier cache.
- Configured gates were evaluated sequentially from the main checkout.
- `jit validate --json` passed with zero errors, warnings, or membership divergences after each completed issue transition.
- `985ab96d`'s initial cargo run hit one shutdown timing flake; the focused test passed immediately. Its next run passed all 4,082 tests but failed only because that diagnostic run created `target/debug/incremental`; after removing exactly that regenerable cache, the definitive cargo gate passed.
- `eeee8a1a` had two no-output reviewer-service failures. After the service recovered, its mechanical gate and code review both passed and the issue completed.
- External code-review egress was explicitly approved by the user.

## Next steps

1. Commit the recorded approvals, issue `625cc07f`, its DAG wiring, and progress/handoff updates as isolated lead state.
2. Create fresh worktrees from current main and dispatch exceptional-round `f04f7888` and `c7f8ebc7` workers with the complete cumulative finding history. Preserve the normal rework counts of two and record this attempt separately.
3. Merge each result separately, verify the exact commit, reinstall through `scripts/install-jit.sh`, and rerun required gates sequentially.
4. Complete both issues only after all criteria, linked artifacts, stale-behavior sweeps, and deferred-marker audits pass.
5. Run `jit validate`, commit JIT state separately, and then dispatch wave 1b (`f9e42a43`, `625cc07f`). Re-read `f9e42a43`'s predecessor note in `progress.json`; `f289ff18` moved its error construction into `crates/jit/src/document/reference.rs`.
6. Dispatch `ef118aea` only after `625cc07f` completes, then run whole-tree issue `a122b9b3` alone.

## Operational traps retained from session 1

- Workers must not edit `.jit/`, transition issues, or run gates. Only the lead writes JIT state.
- Never evaluate gates in parallel. Always provide the main checkout as explicit cwd.
- Install the dogfood binary only through `scripts/install-jit.sh`; the stale-binary guard rejects dirty or old provenance.
- Use a workspace-backed `VERIFY_COMMIT_CACHE`; `/tmp` previously produced linker bus errors and quota failures.
- Focused Rust diagnostics can create `target/debug/incremental`; remove only that exact regenerable cache before cargo-ci when necessary.
- Keep JIT state commits separate from implementation/artifact commits.
- Do not infer completion from a worker's idle notification; inspect the branch and wait for the final report.
- The epic cannot be claimed while its sink dependency is unmet; assignment without transition is intentional.
