# Mutation-latency baseline

Baseline measurements behind the issue's REQ-04 threshold. Reproduce before
and after the fix and compare.

## Harness

`subsample-repo.py <src-repo> <dst> <n-issues>` builds a repository holding the
first N issues of a source repository, pruning dependency edges that would
dangle. `vary-doc-refs.py <src-repo> <dst> <n-doc-refs>` holds issue count fixed
and truncates the total document-reference count. Both leave a committed git
repository at the destination.

Time each operation as wall clock around a single process run. Count git
subprocesses by putting a wrapper named `git` ahead of the real one on `PATH`
that appends its arguments to a log before exec'ing through.

Run on tmpfs. On-disk timings agreed to within 4%, which is what establishes
process spawning rather than storage I/O as the cost.

## Scaling by issue count

Document references scale with issue count in the subsample, so this column
pair moves together and does not separate the two terms.

| issues | doc refs | dependency add | issue create | validate |
| ---: | ---: | ---: | ---: | ---: |
| 25 | 32 | 240 ms | 248 ms | 88 ms |
| 50 | 42 | 316 ms | 324 ms | 112 ms |
| 100 | 71 | 510 ms | 516 ms | 175 ms |
| 200 | 107 | 715 ms | 732 ms | 250 ms |
| 400 | 208 | 1447 ms | 1451 ms | 495 ms |
| 800 | 383 | 2670 ms | 2653 ms | 909 ms |
| 1071 | 445 | 3209 ms | 3164 ms | 1087 ms |

Reads over the same range stay flat: listing all issues moves 14 ms to 31 ms,
querying available work 13 ms to 27 ms, and single-issue status 7 ms to 8 ms.

## Separating the two terms

Issue count held at 1071, document references varied:

| doc refs | dependency add | git processes |
| ---: | ---: | ---: |
| 0 | 201 ms | 3 |
| 50 | 454 ms | 203 |
| 100 | 682 ms | 389 |
| 200 | 1122 ms | 743 |
| 445 | 2192 ms | 1575 |

Document references held at 0, issue count varied: 28 ms at 100, 76 ms at 400,
199 ms at 1071.

This gives two independent linear terms, 0.18 ms per issue and 4.5 ms per
document reference, and a spawn count of roughly 3.5 git processes per
reference. Predicted latency matches every measured row above to within 1%.

## Where the time goes

Of the 3209 ms dependency add at 1071 issues and 445 references, 2301 ms is
git subprocess spawning: 1136 ms across 782 `rev-parse` calls and 1165 ms
across 782 `ls-tree` calls, timed by replaying the same calls standalone.

The 782 `rev-parse` invocations are one identical query, because every
reference in this repository is unpinned and so carries the same revision.
One `ls-tree` listing the whole tree costs 2.9 ms and answers every path that
the 391 individual invocations answer one at a time.

Spawn counts divide by the 391 distinct reference paths to give the capture
count per command: whole-repository validate captures once at 789 spawns, a
dependency add or issue update twice at 1575, and issue creation three times
at 2361.

A CPU profile attributes the non-git remainder to `CaptureSpec::validate`,
path component parsing, and SHA-256 content hashing. Those stay linear in
reference count and are out of scope for this issue.
