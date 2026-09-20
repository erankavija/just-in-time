# Capture budgets that declare the event log

Survey taken while closing `48e4f65c`, which removed the 64 MiB ceiling from the
document capture. It records every remaining `CaptureBudget` in the workspace whose
`CaptureSpec` declares `VirtualPath::EVENTS` and still states a `max_bytes` ceiling,
so the decision on how far REQ-02's wording reaches rests on measured sites rather
than recollection.

## The sites

| Budget | `max_listings` | `max_bytes` | `max_depth` | Where the log enters | Spec built at |
| --- | --- | --- | --- | --- | --- |
| `commands/gate.rs:248` `GATE_PRESET_CAPTURE_BUDGET` | 64 | 512 MiB | 16 | `captured_gate_preset_fixed_paths()` `gate.rs:296` | `gate.rs:315`, `gate.rs:344` |
| `commands/gate.rs:1108` (gate registry mutation) | 64 | 256 MiB | 16 | `registries()` `gate.rs:1119` | `gate.rs:1129` |
| `commands/gate_check.rs:781` (gate evaluation image) | 256 | 512 MiB | 32 | declared path list `gate_check.rs:790` | `gate_check.rs:794` |
| `commands/template.rs:33` `TEMPLATE_CAPTURE_BUDGET` | 2 | 512 MiB | 16 | `template_fixed_paths()` `template.rs:388` | `template.rs:406`, `template.rs:415` |
| `commands/validate.rs:1253` `MUTATION_CAPTURE_BUDGET`, via the derived budget `validate.rs:356` | 256, or 65536 for an archive scan | 512 MiB | 32 | `spec.discover_paths([VirtualPath::EVENTS])` `validate.rs:411` | `validate.rs:410` |

`MUTATION_CAPTURE_BUDGET` also serves `capture_mutation_preflight` (`validate.rs:253`,
`:269`), which declares only the config, the index, and candidate issue records; that
use does not reach the log.

Two budgets state a hand-picked 64 MiB ceiling and declare no event-log path, so
neither is exposed to log growth: `commands/config.rs:304` and `commands/init.rs:350`
each declare `VirtualPath::CONFIG` alone.

## Why these differ from the one `48e4f65c` removed

Every site above requests listings. `CaptureBudget`'s own contract
(`repository_state/image.rs:585-604`) puts capture growth in exactly one place —
expanding a complete listing names children the declaration did not — and `max_bytes`
is one of the two dimensions that bound it, charging every listed child's name
alongside the bytes read. Where `max_listings > 0`, deleting `max_bytes` deletes a
live bound, not a dead restatement.

The document and issue-mutation budgets request no listing at all
(`commands/mod.rs:141-164`). Nothing there can grow beyond what the caller declared,
so their `max_bytes` bounded only the payload whose size is the project's own history.
That is what made it removable in one line.

A replacement bound for the five sites therefore has to bound listing expansion
directly rather than through a byte total that the event log also draws on. The
`max_listings` column above is the input to that design.

## Exposure

Measured on this repository at 2026-09-20, after nine months of continuous
multi-agent dogfooding (`.jit/events.jsonl` first committed 2025-12-18):

- event log: 3.4 MiB over 14,946 events, so roughly 239 bytes per event
- `.jit/issues`: 5.5 MiB across 1,151 records

Against that baseline:

- the 64 MiB ceiling `48e4f65c` removed sat at about 19x the current log
- 256 MiB is about 75x, or roughly 1.12M events
- 512 MiB is about 150x, or roughly 2.25M events

So none of the five is in plausible reach for a repository of this project's shape and
intensity: the exposure is one to two orders of magnitude away, where the document
ceiling was within one.

Two qualifications keep that from being headroom anyone should spend:

- The ceiling charges the whole capture, not the log alone
  (`repository_state/image.rs:899`). The gate-preset, template, and validate captures
  declare `VirtualPath::ISSUES` and its listing, so every issue record and every listed
  child's name counts against the same number. On this repository that is already 5.5
  MiB of issue records before the log is read, and it grows with the project too.
- The archive-scan variant (`validate.rs:356-362`) raises `max_listings` to 65536, and
  every listed child's name is charged, so its byte total is the furthest from being a
  statement about the log.

## Disposition

The five sites are unchanged by `48e4f65c`, which is scoped to the ceiling nothing can
grow into. They are the same defect class deferred by an order of magnitude, and unlike
the document ceiling they are load-bearing until listing expansion is bounded on its
own terms.
