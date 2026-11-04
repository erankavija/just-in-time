# Handoff 2: f8a6b298 history-independent reads

Outgoing lead: just-in-time-8c (session 8cfb372c). Incoming: just-in-time-52. Written 2026-11-04 ~08:40 EET. This supersedes `handoff.md` where they differ.

## Task state

Done: 32 of 38 impl tasks. Progress, dispatch log, rework counts and escalations: `progress.json`.

| Task | State | Notes |
|---|---|---|
| 09ca911e storage-record reference docs | landing (my queue job) | committed `ab4647e0e` on its branch |
| 725af076 capture refuses whole event-log reads | landing after 09ca911e | committed `fa48c2012`; lead decided the two tests owning removed whole-log arms change (materialize closure, overlay) |
| 7244ecb2 lease interruption matrix | worker told to stop and commit | if it reports partial, its branch holds the WIP; finish or redispatch |
| 4466395d history isolation test | backlog | |
| ac8f07c2 storage-format history docs | backlog | 1630015d already updated the storage-format directory tree |
| f0a5b90a lease operator docs | backlog | owns user docs still showing `<lease-id>` (614a6f97 left claim.md, cli-command-grammar.md, multi-agent-coordination.md, troubleshooting.md, guarantees.md, parallel-work-worktrees.md to it) |
| 655273d9 format cutover | ready, blocked on the user | see below |
| stories 735949ef, 5ee85aed, dc5fa7f8; epic | backlog | close after children; then epic completion (Section 10) |

Check the two queue jobs: their output is in `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/8cfb372c-a2e4-4cea-b109-0e52c5da8739/scratchpad/f8a6-lead/land-09ca911e.txt` and `land-725af076.txt`. If a land ends with code-review failed or a gate not passed, the issue stays in_progress; handle it per the review protocol.

## Format-4 window (655273d9)

The user approved the cutover of all five repositories (via just-in-time-6a, recorded in `progress.json` notes). Conditions: inspect `research` first; stop and report if any repo is not a format-3 jit tracker; each repo's `.jit` clean and revertible with git; coordinate live sessions (gf2-4e in gf2); reinstall only after all five are rewritten.

The bracket's run was denied by the outgoing session's Claude Code permission check ("Modify Shared Resources"). Nothing changed. The incoming lead needs the user's permission in its own session, and must not route around a denial.

Tooling (on main): `dev/active/f8a6b298-history-independent-reads/format4-rewrite.py`, `format4-window.sh` (`install` and `rewrite` modes), `format4-dry-run.sh`, `format4-rewrite-test.py`; dry-run record `format4-dry-run.md`. Dry run at 07:05 on copies of all five passed every check.

Plan agreed with just-in-time-6a:

1. Pre-bracket, no writers stopped: choose the green sha (was `45e523c3e`; use main HEAD at the time if main moved in a way that matters). In a fresh detached worktree `target/format4-green-<sha12>` run `./scripts/cargo-ci.sh` under `flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock`, then `cargo build --release --locked -p jit -p jit-server` there. A verified tree for `45e523c3e` exists at `target/format4-green-45e523c3e4a6` with release binaries.
2. Send just-in-time-6a a 15-minute warning; wait for its confirmation that every writer (just-in-time-88, just-in-time-a4, gf2-4e, any session in the other repos, the dashboard) stopped and committed `.jit`.
3. Bracket: the outgoing lead's script is `scratchpad/f8a6-lead/bracket.sh` (path above); copy and adapt it. Steps: re-check each repo is format 3 with a clean `.jit` and record its HEAD; stop the jit-server with `jit serve --stop` (format-3 binary); copy `~/.cargo/bin/jit` and `jit-server` to a rollback directory (`mktemp -d ~/.cache/jit-format4-rollback.XXXXXX`); install the verified release binaries with `install -m 755` (atomic, works on a running executable); run `format4-window.sh rewrite <repo> <head>` for each repo in turn; after just-in-time's rewrite validates, restart the server; finally `jit validate` in all five.
4. Server: running before as PID 3361204, `jit-server --data-dir /home/vkaskivuo/Projects/just-in-time/.jit --bind 0.0.0.0:3000 --web-dir /home/vkaskivuo/Projects/just-in-time/web/dist`, cwd the repo. Restart from the repo root with `jit serve --port 3000 --web-dir /home/vkaskivuo/Projects/just-in-time/web/dist` and compare the new process's arguments with `ps`. It is a tracker writer: stop it inside the bracket.
5. Report each repo's validate and claim-list result and the server's new PID and argument comparison to just-in-time-6a, which releases writers and lifts the install freeze.

Failure policy: a failure before the first rewrite → restore the rollback binaries. After a rewrite has committed → roll forward: keep the format-4 binary, stop, report, fix and rerun the failed repo (a refusal changes nothing; a partial rewrite converges on rerun). Never reinstall format 3 over a mixed set.

Repositories (all `--git-common-dir` = `<repo>/.git`; all format 3, `.jit` clean at the 07:15 check except just-in-time's live tracker files):

| Repo | Quirks |
|---|---|
| /home/vkaskivuo/Projects/just-in-time | tracks `.jit/gate-runs`; 25 leases; archive evidence 58; the jit-server runs here |
| /home/vkaskivuo/Projects/gf2 | gitignores `.jit/gate-runs` (rewritten but uncommitted by design); 13 profile torn-tail marker lines; 2 stale leases; live lead gf2-4e |
| /home/vkaskivuo/Projects/mealie-fi | tracks gate-runs; no archives or leases |
| /home/vkaskivuo/Projects/inflected-messages | tracks gate-runs; no archives or leases |
| /home/vkaskivuo/Projects/research | gitignores `.jit/gate-runs`; no archives or leases |

## Queue tooling

- Landing: `scratchpad/f8a6-lead/land-issue.sh <sid>` merges main into the worktree, runs cargo-ci on the merged tree under the turnstile and the exclusive full-CI lock, runs `land.sh`, evaluates gates (code-review last), and marks done when all pass. Run it as `flock -o scratchpad/f8a6-lead/queue2.lock land-issue.sh <sid>` (always `-o`: an sccache daemon once inherited a plain flock fd and held the queue lock).
- Authoritative CI and gate runs use `TMPDIR=/dev/shm/jit-ci/f8a6-lead-<sid>`, removed after the run (never `/dev/shm/sccache-tmp`); `CARGO_INCREMENTAL=0`. The sccache guard: if `ps -C sccache` finds no daemon, set `CARGO_CI_NO_SCCACHE=1` and tell just-in-time-6a.
- Workers' own test runs use `TMPDIR=/mnt/nvme/jit/tmp/f8a6-lead`; their sync-heavy TIMEOUTs there are host artifacts.
- Host faults (cargo-ci exit 75, load budget, TIMEOUT-only, ENOSPC) are reruns, not verdicts; a gate that recorded such a fault is re-run with `--force`.
- Codex workers cannot write `.git`: never let them merge; the lead commits and merges main.
- `progress.json` edits are committed in the same command; uncommitted files outside `.jit` block every lead's `land.sh`.

## Open review items

- Plan reviews (5e615dc2 plan-review, b06cee74 coverage-preview and breakdown-review) passed at 04:58 after the DEC-24 absorption and contract edits; rerun them if the plan changes again.
- Decisions recorded on issues: dd53527f D-1 (claims-log sequence scan accepted until 2f563673), 2f563673 D-1 (53d19851 absorbed) and D-2 (lease-id-scan-bridge, now removed by 614a6f97), 97f5f2b7 D-1 (atomic-writes wording).
- `surfaced_pitfalls` in `progress.json` must be reconciled against the epic criteria at completion.

## Traps — do not repeat these

The traps in `handoff.md` still apply. Also: a cleanup that deletes a per-run TMPDIR breaks the shared sccache daemon if the run started it; the guard above prevents that.
