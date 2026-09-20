# Whole-log read cost and the single-pass capture specification

> **Diátaxis Type:** Explanation (investigation finding)

Investigation for `7226f00b`. It measures what reading `events.jsonl` into the capture
image costs, enumerates every consumer of those captured bytes, and answers whether
`c88822a5` REQ-03 can keep the event log in the capture base while the epic's REQ-02
holds.

**Verdict: it cannot.** One whole-log read costs $0.89\ \text{ms}/\text{MB}$; REQ-02's
budget is $0.34\ \text{ms}/\text{MB}$. Even a read with zero capture overhead costs
$0.81\ \text{ms}/\text{MB}$, so no implementation of a whole-log read fits.

---

## Method

Hardware: 24 logical cores, SHA-NI present, fixtures on tmpfs, page cache warm.
Binary: `cargo build --release -p jit` at `142e1ad76`.

Three measurements, each reported as a median.

**M1 — whole-log read, isolated (REQ-01).** A throwaway `examples/` binary drove the real
`RepositoryMutationSession::capture` over a fixture, three specs per iteration, 15
iterations, repeated in three passes:

1. the index, config, gate registry and every indexed issue file — the base set
   `c88822a5` REQ-03 keeps, minus the log;
2. the same spec plus `VirtualPath::EVENTS`;
3. `VirtualPath::EVENTS` alone.

(2) − (1) is the log's marginal contribution to a capture round; (3) is the same quantity
reached independently. The two agree to within 1 ms, which is what makes the isolation
credible. The binary also timed `std::fs::read` and `Sha256::digest` over the same file to
establish a floor. It is deleted — nothing under `crates/` is committed (REQ-05).

**M2 — end-to-end scaling.** Six copies of this repository's benchmark fixture with logs
inflated to 1, 2, 4, 6, 8 and 10 times, using the inflation the committed harness performs
(`dev/active/f1dc843a-redo-mutations/bench.sh`). Seven timed pairs per fixture after two
warm-up pairs; median of a changing label update and of the idempotent repeat.

**M3 — the committed harness.** `bench.sh` run unmodified for the headline figure.

## Measured: the whole-log read (REQ-01)

| log | fixture bytes | capture, no log | capture + log | **difference** | log alone |
|---|---|---|---|---|---|
| 1x | 3 370 280 | 14.2 ms | 17.4 ms | **3.2 ms** | 3.0 ms |
| 10x | 33 702 914 | 14.5 ms | 43.2 ms | **28.6 ms** | 29.5 ms |

One capture round pays **3.2 ms at 1x and 28.6 ms at 10x**: a slope of
$0.89\ \text{ms}/\text{MB}$, stable to ±0.4 ms across passes. The base capture is flat,
confirming the difference is the log and not fixture drift.

The cost is not capture overhead. Reading the same file and hashing it, with no capture
machinery at all:

| log | `fs::read` | `Sha256::digest` | floor |
|---|---|---|---|
| 1x | 0.14 ms | 1.38 ms | 1.5 ms |
| 10x | 11.2 ms | 13.8 ms | 25.0 ms |

The floor slope is $0.81\ \text{ms}/\text{MB}$ — 91 % of the measured cost. Capture reads
each file with `read_to_end` and SHA-256s it (`repository_state_store.rs:1469-1480`,
`image.rs:43-51`), and that pair is running at roughly 2.4 GB/s on this machine. There is
no implementation slack here.

## Measured: how many whole-log passes a label update makes

| log MB | changing | idempotent |
|---|---|---|
| 3.22 | 123.3 ms | 76.1 ms |
| 6.44 | 152.1 ms | 82.3 ms |
| 12.87 | 204.1 ms | 97.1 ms |
| 19.31 | 260.0 ms | 108.2 ms |
| 25.74 | 311.2 ms | 117.6 ms |
| 32.18 | 421.8 ms | 128.8 ms |

The idempotent slope is $1.82\ \text{ms}/\text{MB}$ — exactly $2.04 \times 0.89$. The
idempotent path returns before `finalize` (`commands/mod.rs:2541`), so its whole marginal
cost is capture, and it makes **two** whole-log reads: the phase-two spec adds the log
(`commands/validate.rs:411`) and phase three clones that spec (`validate.rs:422`).

The changing slope over 1x–8x is $8.34\ \text{ms}/\text{MB}$. The extra
$6.52\ \text{ms}/\text{MB}$ over the idempotent path is a third whole-log read at apply
(`storage/repository_state_store.rs:528`, compared whole at `:535`), the composer's two
prefix copies (`mutation.rs:1225`, `:1269`) and the whole-file rewrite of the log. The 10x
point sits 60 ms above the line the other five define; the projection below uses the 1x–8x
slope, which is the conservative choice.

`bench.sh` unmodified: changing label update 124 ms at 1x, 423 ms at 10x — **3.41x**.

## Projected: REQ-02 after the epic (REQ-02 of this issue)

The epic's own criteria fix the post-retirement shape. Assumptions, each traceable:

- `b4483ad6` lands the positioned append, so the composer's prefix copies and the
  whole-file log rewrite are gone.
- Epic REQ-05 removes "apply-time revalidation of the whole read set" and the "plan hash
  over captured entries", so the apply-time whole-log read is gone.
- Epic REQ-04 states the conflict check as "event-log length or tail", which
  `ensure_write_set_unchanged` already implements (`repository_state_store.rs:877-909`).
- `c88822a5` collapses the attempt to one round, keeping the log in the base spec.

Under those, **one** whole-log read remains. Decomposing the measured 1x changing update
at the measured slopes:

$$
T_{\text{base}} = 123.3 - 8.34 \times 3.22 = 96.4\ \text{ms}
$$

$$
T(1\text{x}) = 96.4 + 0.89 \times 3.22 = 99.3\ \text{ms}
\qquad
T(10\text{x}) = 96.4 + 0.89 \times 32.18 = 125.0\ \text{ms}
$$

**Projected difference: +25.8 ms, or +26 %.** REQ-02 requires under 10 %.

Stated as a budget, REQ-02 allows $0.10 \times 99.3 = 9.9\ \text{ms}$ across a 28.96 MB
increase, i.e. $0.34\ \text{ms}/\text{MB}$. A whole-log read costs $0.89$, and its floor
costs $0.81$. **REQ-02 is unreachable while any mutation reads the log whole** — by 2.6x
against the measurement and 2.4x against the hardware floor.

With the log contributing only its length and a bounded tail, $T(10\text{x}) \approx
T(1\text{x})$ and the difference falls under 1 %. Nothing between those two designs helps:
the log is read whole or it is not.

## Enumerated: consumers of the captured event-log bytes (REQ-03)

Eleven production sites read the log's bytes out of a `RepositoryImage`. Sites that only add
`VirtualPath::EVENTS` to a `CaptureSpec` are listed after them; they consume nothing.

| # | `file:line` | function | what it takes from the bytes | needs | on label update |
|---|---|---|---|---|---|
| 1 | `repository_state/mutation.rs:695` | `captured_events` | `parse_known_events` over every line | whole log | no — `ClaimIssue` arm only (`:884`) |
| 2 | `repository_state/mutation.rs:1225` | `compose_event_action` | `prefix.to_vec()`, then `compose_events` (`:1269`) | whole log | **yes** |
| 3 | `repository_state/mutation.rs:1230` | `prefix_has_torn_tail` (`:645`) | the final line only | bounded tail | **yes** |
| 4 | `repository_state/mutation.rs:1273` | `expected_of` → `ExpectedPreimage::File` | the entry identity: SHA-256 and byte size | whole log (to compute) | **yes** |
| 5 | `storage/repository_state_store.rs:535` | `JsonMutationSession::apply` | re-reads the set, compares entries whole | whole log | **yes** |
| 6 | `storage/repository_state_store.rs:898` | `ensure_write_set_unchanged` | `EventLogTail::of_bytes` — length + last 4096 | length + bounded tail | not yet wired |
| 7 | `repository_state/archive.rs:465` | `captured_archive_events` | `parse_known_events` over every line | whole log | no — archive |
| 8 | `commands/migrate.rs:88` | `backfill_lifecycle_timestamps` | `parse_known_events` over every line | whole log | no — `jit migrate` |
| 9 | `validation/repository.rs:428` | `load_records` | `parse_known_events`, keeps `.len()` | whole log | no — validate, or a broad precheck gate |
| 10 | `repository_state/initialize.rs:861` | `events_action` | `Absent` vs `File { .. }` | presence only | no — `jit init` |
| 11 | `storage/memory.rs:502` | `InMemoryStorage::load_events` | parses every line | whole log | in-memory backend |

`compose_event_action` has a second caller, `finalize_audit_append` (`mutation.rs:1315`),
which the init and profile-lifecycle finalizers use; it consumes the same prefix under the
same classification and is not on the label-update path.

The plan hash reads the *captured* log only as its identity — `CapturedBytes` serializes as
its precomputed digest (`image.rs:135-139`). It does, however, SHA-256 the *composed* bytes
a `WriteFile` action carries (`image.rs:1346`, `:1606`), so a changing label update today
hashes the whole rewritten log a second time. That cost sits in the
$6.52\ \text{ms}/\text{MB}$ residual above, not in the capture read; epic REQ-05 removes
the plan hash and `b4483ad6` shrinks the action's bytes to the appended lines.

Out of scope — these read the log from disk, never from an image:
`storage/json.rs:158` and `:1347` (`fs::read_to_string` then `parse_known_events`), and
`commands/events.rs:7` and `:18` (`IssueStore::read_events`, behind `jit events tail` and
`jit events query`). `repository_state/initialize.rs:1122` filters `RepositoryAction`s by
path rather than reading a captured entry.

Declaration only — these name the log so the finalizer has a prefix and a preimage, and
read nothing: `commands/validate.rs:411`, `commands/mod.rs:2202`, `commands/claim.rs:193`,
`commands/gate.rs:296` and `:1119`, `commands/gate_check.rs:790`,
`commands/template.rs:388`, `commands/document.rs:295`, `:375` and `:957`,
`commands/migrate.rs:45`, `commands/profile.rs:2065`,
`repository_state/initialize.rs:428`. `crates/server` has no consumer;
`server/src/watcher.rs:89` only filters the filename out of change notifications.

**The label-update path holds no semantic consumer.** Its four sites are #2 (a copy the
positioned append deletes), #3 (the last line), #4 (a conflict preimage) and #5 (a
conflict check) — and #6 already answers #4 and #5 with length and tail. Every site that
genuinely needs the whole log (#1, #7, #8, #9, #11) belongs to another command.

## Answer: can `c88822a5` REQ-03 stand as written? (REQ-04)

**No.** As written it commits every mutation to a whole-log read, and the measurement puts
that read 2.6x over REQ-02's budget with no implementation slack beneath it.

The amendment is to make the base specification take the log as **length and a bounded
tail**, and to let the commands that parse it ask for the whole log themselves:

> REQ-03: The capture's base specification reads the registries, the index, **the event
> log's length and a bounded tail**, the indexed issue files, and the plan documents a
> transition rule consults; a caller's named paths and listings are added to that one
> round. **A command that parses the log names it whole in its own capture set.**

This needs a per-path read mode in `CaptureSpec` and a captured entry carrying length and
tail — `EventLogTail` (`repository_state_store.rs:928`) is that value, today private and
derived from bytes the capture already holds. It also requires #4 to stop deriving the
event log's expected preimage from a whole-log SHA-256, which is a production change
`c88822a5` or `b4483ad6` must carry; `ensure_write_set_unchanged` is the replacement and
epic REQ-04 already specifies it.

**Consumers that block the whole log disappearing entirely** — and so must keep asking for
it explicitly: `mutation.rs:695` (claim reflection), `archive.rs:465`, `migrate.rs:88`,
`validation/repository.rs:428`, `memory.rs:502`. None is on the label-update path, so none
blocks REQ-02. `mutation.rs:695` computes a tail-shaped property
(`event_tail_reflects_claim`, `:713`, scans in reverse for the most recent assignment
event) but over an unbounded reach, so it is not a tail read today.

## Two premises of `c88822a5` that do not hold at `142e1ad76`

Both affect its line-balance and its round count; neither changes the verdict.

- "The apply-time round is already gone." It is not.
  `JsonMutationSession::apply` still re-captures the entire read set and compares it whole
  (`repository_state_store.rs:526-538`). It is the third whole-log read a changing label
  update pays, and epic REQ-05 is what retires it.
- `ensure_write_set_unchanged` (`faa668c9`) has **no production caller** — only
  `repository_state_store_tests.rs:5108`. The length-and-tail comparison exists but
  nothing reaches it yet.

## Measured, projected, and undetermined

Measured: every number in *Method*, *Measured: the whole-log read* and *Measured: how many
whole-log passes*, including the 0.89 and 0.81 ms/MB slopes and the idempotent path's two
capture reads, which the slope ratio $1.82 / 0.89 = 2.04$ establishes directly. The
changing path's third read is read from the code (`repository_state_store.rs:528`) and is
consistent with its residual slope; it is not separately timed, because no tracing
facility on this host could count the opens.

Projected: the 99.3 ms / 125.0 ms pair and the +26 % difference. They assume the four
bullets under *Projected*, a log-independent base unchanged by the epic's other work, and
that the per-pass slope holds under one round rather than three. The first assumption is
the load-bearing one: if the epic lowers the base below 96.4 ms, the 10 % budget shrinks
and the gap widens.

Undetermined: whether `c88822a5`'s single round removes fixed per-round overhead large
enough to lower the base materially. It would have to fall below 26 ms for a whole-log
read to fit the budget, which the 14.2 ms flat base capture makes implausible, but only
building the single-pass capture settles it. The 10x changing-label outlier — 60 ms above
the line the other five points define — is also unexplained; it is consistent with the
34 MB whole-file rewrite crossing a writeback threshold, and the positioned append removes
that write either way.
