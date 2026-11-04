# Format 4 dry run

The dry run behind [format4-rewrite.py](format4-rewrite.py) and
[format4-window.sh](format4-window.sh), as run on 2026-11-04 at 06:49. It was
reproduced by [format4-dry-run.sh](format4-dry-run.sh):

```
format4-dry-run.sh ~/Projects/just-in-time ~/Projects/gf2 ~/Projects/mealie-fi \
  ~/Projects/inflected-messages ~/Projects/research
```

The driver read each repository at the HEAD shown below and copied its `.jit` and
`.git/jit` into `target/format4-dry/`. It ran the rewrite twice on each copy. It
then ran the `jit` built from the tree at 02df7867e, which is repository format 4.
The counts are a snapshot and change as the repositories receive new tracker data.

| repo | HEAD | runs | source_issue | run_id set | verdicts | evidence | markers removed | leases | claims.seq |
|---|---|---|---|---|---|---|---|---|---|
| just-in-time | 214d3d32e | 6551 | 301 | 2 | 1034 | 58 | 0 | 25 | 3832 |
| gf2 | 4c696312c | 5994 | 41 | 2495 | 73 | 14 | 13 | 2 | 273 |
| mealie-fi | 4c0c509d5 | 414 | 0 | 272 | 0 | 0 | 0 | 0 | 0 |
| inflected-messages | 5738b57 | 644 | 0 | 422 | 0 | 0 | 0 | 0 | 0 |
| research | 05e1c7f | 449 | 0 | 162 | 0 | 0 | 0 | 0 | 0 |

Column meanings:

- **runs:** run records moved to `gate-runs/<issue-id>/<run-id>.json`.
- **source_issue:** reused verdicts given `source_issue`.
- **run_id set:** gate states given the run ID of their latest run.
- **verdicts:** verdict records written.
- **evidence:** archive evidence records written.
- **leases:** lease records that `jit claim list` reports.

In all five copies:

- The rewrite refused nothing.
- The second run changed no byte.
- `jit validate` exited 0, including its record-fidelity pass.
- `jit gate status` passed both checks on the gate with the most runs, on the issue with the most runs. The latest view exited 0, and the `--all` view listed every run of that gate.
- `jit claim list` exited 0.

The archive container preview of a seeded target exited 0 in just-in-time and gf2. The other three repositories log no archive executions, so nothing was seeded there to preview.
