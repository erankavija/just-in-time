# Capture git-spawn benchmark

Before-and-after evidence for REQ-04 and REQ-08. The fix batches pinned-document inspection and caches the evidence by resolved commit.

## Method

`bench.sh <jit-binary> <label>` copies a prepared repository before each scenario, puts `git-count-wrapper.sh` ahead of the real `git` on `PATH` to log every git invocation, and times one process run as wall clock. Every repository copy lives on tmpfs.

| repository | shape | built from |
| --- | --- | --- |
| `jit1071-445` | 1071 issues, 445 document references, every reference unpinned | `subsample-repo.py <this repo> sub1071 1071`, then `vary-doc-refs.py sub1071 jit1071-445 445` |
| `gf2-copy` | 864 issues, 1497 document references across 42 distinct revisions, mostly file targets | `git clone --no-checkout --shared` of an adopter repository at `e9ac0ebc`, with its `.jit/` copied in |

The subsample copies only `.jit/`, so its references resolve as paths missing from the commit. Its two pinned revisions are absent from the fresh history, so the batched revision resolution falls back to per-revision resolution. The adopter copy exercises file targets, blob reads, and many distinct revisions.

"before" is the installed jit 1.0.0 release binary, built before the fix, measured in one run. "after" is the release build of the fix. Its rows are the median of three runs, and every run spawned the same process count.

## Results

| scenario | before: git processes | before: wall | after: git processes | after: wall |
| --- | ---: | ---: | ---: | ---: |
| `jit1071-445` dependency add, published with `--reduce` | 2361 | 5706 ms | 16 | 517 ms |
| `jit1071-445` dependency add, refused as redundant | 1575 | 3775 ms | 12 | 435 ms |
| `gf2-copy` idempotent label update | 8985 | 24542 ms | 48 | 3498 ms |

REQ-04 holds: the published dependency add spawns 16 git processes (limit 20) and completes in 517 ms (limit 1000 ms).

REQ-08 holds: the adopter update spawns 48 processes for 1497 references across 42 revisions. They are 3 repository-discovery calls, 1 `rev-parse --end-of-options` per capture (2 captures), 42 `ls-tree -r -t -z --full-tree` listings (one per distinct commit, in the first capture only), and 1 `cat-file --batch`. The second capture reuses every cached pair and spawns only its revision resolution.

## Remaining cost

The adopter update's remaining 3.5 s is in-process. A CPU profile of the after binary attributes it mainly to SHA-256 content hashing (13%), path component iteration (13%), and `RootRelativePath::parse` (5%). The issue's notes put these linear terms out of scope.
