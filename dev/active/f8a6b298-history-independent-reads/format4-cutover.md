# Format 4 cutover

The record of the window that moved five repositories to repository format 4
(655273d9). It ran on 2026-11-04 from 10:40:12 to 10:41:56 EET, using
[format4-window.sh](format4-window.sh) once per repository with the
[format4-rewrite.py](format4-rewrite.py) at this commit's tree. Before the
window, [format4-dry-run.sh](format4-dry-run.sh) passed on fresh copies of all
five repositories with the binary below.

## Preconditions

- **Approval:** the user approved the cutover for all five repositories, with no
  confirmation needed per repository. The approval was relayed by the steward
  just-in-time-6a, and the user also gave it directly in the lead's session.
- **Quiet:** just-in-time-6a stopped every tracker writer and confirmed it,
  including the dashboard and the jit-server. The bracket's own check found each
  repository at format 3 with no uncommitted `.jit` path.
- **Binary:** `jit` and `jit-server` were built in release mode at `a05567fde`.
  `./scripts/cargo-ci.sh` passed in that tree. The previous binaries were copied
  to a rollback directory before `install -m 755` replaced them.

## Per repository

Each rewrite was followed by `jit validate` and `jit claim list` with the
installed binary, and then by one commit of the tracked `.jit` paths.

| Repository | HEAD before | Rewrite commit | Runs moved | Verdict records | Evidence records | Torn-tail markers removed | Leases after | `validate` |
|---|---|---|---|---|---|---|---|---|
| just-in-time | `aec53b2f3` | `ecdb2e473` | 6626 | 1067 | 58 | 0 | 3 | pass |
| gf2 | `85050dff6` | `321f632ac` | 5997 | 74 | 14 | 13 | 2 | pass |
| mealie-fi | `4c0c509d5` | `f1572e900` | 414 | 0 | 0 | 0 | 0 | pass |
| inflected-messages | `5738b57` | `0652d60` | 644 | 0 | 0 | 0 | 0 | pass |
| research | `05e1c7f` | `7b7e9e9` | 449 | 0 | 0 | 0 | 0 | pass |

gf2 and research gitignore `.jit/gate-runs`. Their rewritten run and verdict
records are on disk but are not part of the rewrite commit. In each repository,
the rewrite moved every lease in the pre-window lease index under
`<common-dir>/jit` into its own lease record, dropping no expired lease.
`jit claim list` then reported that same number of leases.

## After the window

- **Indexes:** all five `index.json` files read 4, and `jit validate` reports 0
  errors in each.
- **Run ids:** every run id cited in this repository's `dev/`, `docs/` and issue
  records still resolves. Of the 6626 run ids in the pre-window tree, 3699 are
  cited, and each resolves to `.jit/gate-runs/<issue-id>/<run-id>.json`.
- **Server:** jit-server stopped as PID 3361204 and restarted from this
  repository as PID 3274476. Both processes had the arguments
  `--data-dir <repo>/.jit --bind 0.0.0.0:3000 --web-dir <repo>/web/dist`, and
  `/api/health` answered 200.
- **Rollback:** no rollback was taken and no repository was pushed.
