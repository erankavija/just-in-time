# Handoff: f8a6b298 history-independent reads

State at 07:35 EET 2026-11-04. 31 of 38 tasks done; progress in `progress.json`.

## Blocked on the user

- Format-4 cutover (655273d9): `scratchpad/f8a6-lead/bracket.sh` in session 8cfb372c was denied by Claude Code's permission check ("Modify Shared Resources"). Nothing changed: all five repos at format 3, old binaries installed, jit-server PID 3361204 running. On the user's yes: re-verify the green sha (was 45e523c3e; worktree `target/format4-green-45e523c3e4a6` has CI-verified release binaries), send just-in-time-6a a 15-minute warning, wait for its writers-stopped confirmation, run the bracket, report validate/claim-list per repo and the server's new pid and argument comparison. Roll forward on a mid-window failure; never reinstall format 3 over a mixed set.

## In flight (results land in their worktrees, uncommitted for Codex)

- 614a6f97 (Claude Opus): issue-addressed renew/heartbeat/force-evict; deletes `find_by_lease_id`.
- 725af076 (Codex Sol, continuation run): capture refuses whole event-log reads; lead decided the two tests owning removed arms change.
- 09ca911e (Codex Luna): storage-record reference docs.
Land each with `land-issue.sh` under `flock -o queue2.lock`.

## Remaining after those

7244ecb2 (lease interruption matrix), 4466395d, ac8f07c2, f0a5b90a, then the stories and epic completion.

## Traps — do not repeat these

- Run queue jobs with `flock -o` (an sccache server inherited a plain flock fd and held queue.lock).
- Authoritative CI uses `TMPDIR=/dev/shm/jit-ci/<lead>` (/tmp ran out of inodes); workers use /mnt/nvme; TIMEOUT/ENOSPC/load faults are host faults: rerun, don't rework.
- Evaluate code-review after the issue's other gates (npm-ci evidence).
- Commit progress.json in the same command that edits it; other sessions' uncommitted files block land.sh.
