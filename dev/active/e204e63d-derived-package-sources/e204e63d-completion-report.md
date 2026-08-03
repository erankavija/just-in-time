# Epic Complete: Derived profile assets and projected policy documentation (e204e63d)

**Started:** 2026-07-31
**Completed:** 2026-08-04
**Assignee:** agent:jit-execution-lead

## Summary

Every declarative configuration left the binary and now reaches a repository as
package content: two packages ship (`jit-default` carrying the generic vocabulary,
`jit-dogfood` carrying this project's workflow and declaring a dependency on it), the
packaged asset tree is assembled from the repository files it mirrors rather than kept as a
second checked-in copy, and the three facts that were previously held in agreement by hand —
the plan-template declaration, the shipped area classification, and the packaged live assets —
are each derived from one authority.

## Metrics

| Metric | Value |
|---|---|
| Issues under the epic | 80 / 80 complete |
| Immediate children | 11 / 11 complete |
| Waves executed | 7, plus one remediation wave at the container gate |
| Rework cycles | 30, across 25 issues |
| Escalations | 3 |
| Owner rulings recorded | 36 |
| Issues created during execution | 6 |
| Sessions | 10 |

## Success Criteria

REQ-06 was retired by D-7 rather than renumbered, so the live set is nine.

- [x] **REQ-01** — area lists obtained from the packaged default profile's `[documentation]`
      declaration; neither adopter document keeps a hand-maintained copy — `67da1d70`,
      `daddfc0b`, `f77fd51b`. Both documents carry the lists inside a
      `jit:shipped-documentation-policy` region; mechanical check M7 fails a stale one.
- [x] **REQ-02** — a change to either plan-template declaration without the other fails the
      suite, comparing two parsed declarations — `e1a18372`, `097b9f48`.
- [x] **REQ-03** — the packaged live-asset tree is derived, not a second checked-in copy —
      `7d038e97`, `d94f6849`, `3e340587`, `9943c488`, `26f503cc`. `git ls-files profiles`
      lists only the two manifests, two install assets, and one region source.
- [x] **REQ-04** — two assemblies produce identical content and digests — `ebbb254f`.
      Verified at close: two runs of `assemble-package.sh` produced byte-identical trees
      and the same SHA-256 over the sorted per-file digests.
- [x] **REQ-05** — every live-source root is declared with bounding exclusions, and an
      unclaimed file under a declared root fails the suite — `39c34568`, `0cf1f351`,
      `9d451c98`, `0db28190`, `4873b32f`.
- [x] **REQ-07** — the binary compiles in no workflow profile or preset, and
      `@/invariant/domain-agnostic` carries no sanctioned exception — `ff1bbada`, `aa258222`,
      `6fef2d7c`. `include_dir`, `BuiltinPresets` and `ProfileOrigin::Embedded` are absent
      from the crate.
- [x] **REQ-08** — a package and its declared dependencies are discovered from repository-local
      locations and applied without a jit source checkout — `09aea8b3`, `5d12a79f`, `5dd0df5b`,
      `7835fdc0`, `6c47b99a`, `c7058cac`, `e4ba28c5`, `4c40e165`, `6013cd81`, `8d534ee7`,
      `cd9a17f0`. Verified at close in a throwaway repository that is not a jit checkout:
      `jit init --profile jit-dogfood --from packages/jit-dogfood` applied both packages, the
      dependency resolving from `packages/jit-default` without being named.
- [x] **REQ-09** — the release that removes the embed carries the package in the same release —
      `9de22aa7`, `6013cd81`. `release-artifacts.yml` stages both package directories into the
      archive that carries the binaries.
- [x] **REQ-10** — `jit validate --fix` restores profile-owned targets by re-resolving from the
      provenance record's source path, and fails loudly when it cannot — `8371bd46`. Verified
      at close in both directions: a deleted target was repaired from the recorded location,
      and with the package moved away the command failed naming the record, the profile and
      the unreadable location.

## Wave Execution Log

**Waves 1–6** (sessions 1–9) — the planning bracket, the boundary audit, package assembly and
reproducibility, the live-source declaration and its completeness walk, package composition and
provenance, the release-archive work, and the embed-and-preset removal.

**Wave 7** (session 10) — `26f503cc`, `62ef09b6`, `d198030e`: the citation checker's package
exclusions, the command reference's profile vocabulary, and the bridge's curated descriptions.
Three issues, one rework round.

**Remediation wave** (session 10) — `cd9a17f0` and `e522a8e1`, opened from two blocking findings
at the `7af6eb3d` container gate. Both passed every gate with no rework.

**Epic close** (session 10) — `65280aff`, the unreleased changelog reconciliation.

## Key Decisions

- **Model choice per dispatch, not by rule.** Codex `gpt-5.6-luna` took first attempts where the
  sites and the correct end state were both named; Opus took work that had to establish an
  outcome or run a suite codex's sandbox blocks. The `d198030e` rework went to Opus on the
  second ground, not the first.
- **A reviewer's repair site is not automatically the right one.** `code-review` F1 on
  `d198030e` diagnosed the failure correctly and pointed at a CLI `about` string that
  `62ef09b6` had already aligned a shipped page to. Editing it would have falsified a landed,
  gated page from inside a different issue. The rework brief carried the constraint against the
  reviewer's implied site, with the reason, and the next round passed.
- **Whole-surface audit over batch-fixing.** After `doc-review` found one unrunnable command per
  round, the remaining fix audited every documented profile invocation at once. Three of the
  five locationless forms were deliberate — each framed as the record-backed case — and were
  left alone; changing them would have made the page state something false.
- **Independent verification of every outcome criterion.** Criteria phrased as outcomes were
  checked by running the command, not by reading the code that implements it. This found that a
  documented "preferred setup" command fails in a fresh repository, and it distinguished a
  correct package source from an assembled package when validating the fix.

## Escalations

Three, all resolved by owner ruling and recorded in `progress.json` under `escalations` and
`owner_rulings`.

1. **`779ea308` — issue scope (criteria).** The planned build-input constant required ~60
   hand-written path literals reconciled against the packaged targets by assertion: the exact
   defect class this container removes. Resolved by declaring the inventory once and matching it
   with the same covering rule a gate's declared inputs use.
2. **`39c34568` — issue scope plus a new work item.** D-6 requires categorical exclusions rather
   than path literals, but one declared root had no categorical split. Resolved by relocating the
   packaged gate machinery so each declared root is a directory where packaging is the rule
   (`9d451c98`).
3. **`c7058cac` — epic decision plus issue scope.** Two criteria disagreed about whether the
   `invariant` item kind belongs to the default package. Resolved by amending D-20: initialization
   creates the invariant registry, so `invariant` is default vocabulary and the workflow package
   contributes entries into it.

## Issues Discovered During Execution

- `9d451c98` — Relocate the packaged gate machinery out of the script directory (escalation 2).
- `65ff0f38` — Regenerating a checked-in generated artifact has one invocation form (owner-directed).
- `7cbefe7c` — Inventory the binary's declarative configuration and sequence its removal (D-18).
- `cd9a17f0` — Make the reference's first-time profile initialization runnable as written
  (`7af6eb3d` container gate, both reviewers).
- `e522a8e1` — Take the remaining hierarchy type names out of the bridge's instructions
  (`7af6eb3d` container gate, `code-review` F2).
- `65280aff` — Reconcile the unreleased changelog against the release it describes (epic close).

## Holistic Quality Notes

- **The container checkpoints earned their cost.** `fdae1023` passed its own `mcp-ci` and
  `code-review` while leaving a hierarchy type name in shipped agent-facing text. Only the
  container's whole-surface review caught it — the same class of gap this epic exists to close.
- **Two vacuous assertions were found by reading diffs rather than by gates**, both earlier in
  the epic. The guard added by `e522a8e1` was therefore checked by reintroducing the exact text
  it forbids and observing the failure, and the citation-checker exclusion was checked by
  confirming that four ids inside the excluded subtree are dangling in this repository, so the
  exclusion is load-bearing rather than decorative.
- **Ordering, not caveats, resolved the two review failures that came from sequence.** The
  session-9 rule — an issue removes the falsehoods its own change creates, and a successor is
  never the first place a predecessor's falsehood is corrected — held for the rest of the epic
  and shaped both the `d198030e` rework constraint and the changelog reconciliation.
- **The `holistic-review` gate passed with one low advisory**: aggregate timing and
  disk-reclamation figures in the linked handoffs carry no reproducible measurement. It affects
  no hard criterion, and the operational claims in this report are limited to ones with a
  recorded gate run or a command result behind them.

## Follow-ups Not Filed

These were surfaced during execution and are outside every criterion of this epic. They have no
container yet; placing them is the owner's call.

- `jit issue create` has no `--description-file` or stdin form, unlike `jit issue update`.
- `jit gate` cannot evaluate one gate across several issues.
- `scripts/verify-commit-builds.sh` reports a cold workspace build implausibly fast and may be
  vacuous; a merge that breaks only test targets passes it.
- `cargo-ci` is not deterministic under host load: a graceful-shutdown drain assertion fails
  under a concurrent build.
- A packaged contribution can duplicate a repository registry entry with nothing binding them;
  two instances occurred in this container.
- The dogfood-guidance region has two carriers — the package's region source and the rendered
  region in `AGENTS.md` — with nothing binding them.
- `scripts/assemble-package.sh` takes one positional destination and has no `--help`, so
  `--help` assembles a tree into a directory of that name.
- The stale-binary guard reports the same commit on both sides of its message when the working
  tree is merely dirty in a build-input path.
- `crates/jit/tests/fast_docs_templates/ai_review_verdict_tests.rs` carries a `(REQ-11)` tag
  naming a requirement id local to a different epic.
