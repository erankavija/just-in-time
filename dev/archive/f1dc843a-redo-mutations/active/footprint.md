# Redo record foundation — delivered footprint

> **Diátaxis Type:** Reference (measurement record)

Evidence for `21e976d1` REQ-06. Each task's delivered production footprint against the
estimate its own Notes state, with a reason for every material difference.

## Reproducing these figures

Every number below is produced by [`footprint.py`](footprint.py), committed beside this
document. Run it from the repository root:

```console
$ python3 dev/active/f1dc843a-redo-mutations/footprint.py 34d37f8be \
    --tasks 835bae3f,555ff4be,faa668c9,35544121,4e818d10,8559f529,d591a1c8
Net production lines per task, 34d37f8be..HEAD

  35544121    +320
  835bae3f    +252
  4e818d10    +250
  faa668c9    +120
  555ff4be    +102
  8559f529     +93
  48e4f65c     +32
  untagged      +8
  d591a1c8      +0

  subtotal over 7 tasks: +1137
```

The script is the authority for the table below; the table restates its output alongside
each task's estimate. `48e4f65c` and the untagged commits are outside this phase and are
excluded from the subtotal, which is why the seven-task figure is quoted rather than the
whole-range total.

At wave 21, `722878d4` reuses this script for ER-01's attributed balance — pass `--tasks`
the redo-record tasks alone to get the subtotal that excludes in-epic remediation work.

## Method

Net production lines, attributed by the `jit:<short-id>` tag each commit carries, over
`34d37f8be..HEAD`. Excluded from the production count: `*_tests.rs` files, anything under a
`tests/` directory, modules declared `#[cfg(test)] mod <name>;` in a `mod.rs` or `lib.rs`
(the script discovers these; `crates/jit/src/storage/redo_conformance.rs` is one), and every
line inside a `#[cfg(test)]` item, whose extent is found by matching braces from the
attribute rather than by cutting the file at the first occurrence.

All three exclusions matter and a simpler method gets them wrong. Cutting each file at its
first `#[cfg(test)]` counts `redo_conformance.rs`'s 447 test lines as production — its gate
is on the declaration in `mod.rs`, not inside the file — and truncates `memory.rs` at line
146, where two `#[cfg(test)]` function attributes sit above 200-odd lines of production
code. That method reported 1428 where the correct figure is 1137.

Net rather than cumulative: a rework commit that rewrites a block counts every rewritten
line as an addition, so cumulative additions across a task's commits measure churn rather
than footprint. `35544121` added 413 production lines across its three rounds and removed
93 of its own, delivering 320.

## Delivered

| task | estimate | delivered | difference |
|---|---|---|---|
| `835bae3f` Redo record vocabulary | 250 | 252 | +2 |
| `555ff4be` Durable redo record write | 120 | 102 | −18 |
| `faa668c9` Write-set and event-log conflict check | 150 | 120 | −30 |
| `35544121` File-backend record application | 350 | 320 | −30 |
| `4e818d10` Memory-backend record application | 200 | 250 | **+50** |
| `8559f529` A record rejects an overlapping operation set | 30 | 93 | **+63** |
| `d591a1c8` Shared interruption conformance suite | — (400 test lines) | 0 | — |
| **total** | | **1137** | ceiling 1,500 |

## Reasons for the material differences

**`8559f529`, +63.** The estimate is the execution lead's drafting error. It came from the
worker analysis that proposed the check, which costed the pairwise path comparison and not
the encapsulation that makes the check binding. Delivering only the comparison produced a
checked constructor beside public fields, which a struct literal bypasses; the criterion
says "rejected at construction", so the fields became private, read accessors were added,
and 43 construction sites across four files moved onto `RedoRecord::new`. The lead told the
worker the number was wrong rather than amending the issue, and the worker reported the real
figure.

**`4e818d10`, +50.** Its code review found the two backends reading one record differently:
puts at `issues/a.json` and `issues` made the in-memory backend overwrite a directory entry
with a file while keeping its child, where the filesystem backend fails `EISDIR`. Closing
that required the memory applier to resolve each path as it runs and refuse a directory
occupying its target, across puts, deletes and the append. The estimate was written before
the divergence was known.

**`835bae3f`, +2.** Within measurement noise of a 250-line estimate; session 1 recorded 248
by a slightly different method. No reason beyond the method.

## Why the criterion was amended rather than the estimates

The children's Notes say "Expected additive footprint is under N production lines". REQ-06
originally elevated those estimates into a contract. Three were missed, two for reasons that
only became visible after the estimate was written.

Updating the three Notes to the delivered figures would have made REQ-06 pass, and would
have made it unfalsifiable: a ceiling rewritten once the outcome is known would have
accommodated any outcome. The epic's owner amended REQ-06 once instead, so the binding
constraint is the phase total and the per-task figures stay estimates that are reported
against. The total holds at 1137 of 1,500 with the estimates untouched.
