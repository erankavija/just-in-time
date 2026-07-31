# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 2

**Date:** 2026-07-31
**Session number:** 2
**Prior handoffs:** `handoff.md` (session 1, same directory). Its trap section remains in force except where a trap below records a resolution.

## Current state

- Epic: `e204e63d` — state: backlog. Claimed by nobody; the session-1 claim was not renewed.
- Wave in progress: **none. Wave 0 of 8 is ready to dispatch and nothing has been dispatched.**
- Children summary: 75 issues carry the epic label — 14 done, 1 in_progress (`6f8f02ba`, reopened for the generator retarget), 12 ready, 48 backlog. 8 rejected (they carry no epic label by design; see trap on membership residue).
- Active claims: none.
- Open escalations: none awaiting an answer. 17 owner decisions were taken this session and are all recorded as `@/issue/e204e63d/decision/D-7` … `D-23`.
- **The planning bracket is fully passed.** `7eecace8` done (plan-review), `237b15be` done (coverage-preview, breakdown-review). `@/charter/D-3` is satisfied and implementation may be dispatched.
- Progress file: `progress.json` in this directory.

## What just happened

- **The epic's scope roughly quadrupled on an owner ruling.** "Clean jit before 1.0. No embedded configurations, be it template or others." The container now covers packaging, projection, profile extraction, package composition, and every declarative configuration compiled into the binary.
- Seventeen owner decisions recorded, `D-7` … `D-23`. The load-bearing ones: `D-13` (the binary carries no declarative configuration; bare `jit init` writes the structural minimum), `D-14` (two packages ship, `jit-dogfood` declaring a dependency on `jit-default`; composition enters v1.0), `D-16` (both packages ride in the release archive), `D-19` (no runtime version check), `D-21` (this checkout's own path constants are inside the boundary).
- `@/charter/D-8` amended twice — first to a discovered repository-local profile, then to composable packages. Projected into `AGENTS.md` both times.
- Audit `7cbefe7c` dispatched and completed: 27 sites classified, 17 leave, 10 stay. It **rejected three of the nine sites in the lead's grep survey** and added nine the grep could not reach. Report at `dev/active/7cbefe7c/findings.md`.
- Plan and manifest amended five times. `plan-review` failed four rounds (1 → 4 → 9 → 3 findings) and passed on round 5, owner-authorised past the rework limit. `breakdown-review` failed once on four graph-side findings, then passed.
- Manifest grew 21 → 72 entries, 101 edges. Graph reconciled to it after every round: 42 issues created, 8 rejected, ~20 retargeted in place.
- Epic criteria amended: REQ-04 restated, REQ-06 struck (number retired), REQ-01 restated over the packaged declaration, REQ-08 restated over composed packages, REQ-07…REQ-10 added.
- **No implementation work was dispatched and none has landed.** Every commit this session is `chore` or `plan`.

## What to do next

- [ ] Re-read `dev/active/7cbefe7c/findings.md` §5 (the ordering) and the plan's risks table before dispatching. The ordering has one *silent* failure edge; see traps.
- [ ] Dispatch wave 0. Twelve issues are `jit query available` right now: `a30d704d`, `ae435979`, `4c40e165`, `5dd0df5b`, `09aea8b3`, `3e340587`, `510297e4`, `0439da41`, `fdae1023`, `9bdf8025`, `f6c23c09`, `28e1c647`. They are genuinely independent — but see the trap on the overlap table before choosing which to run concurrently.
- [ ] `6f8f02ba` is `in_progress` and holds no work of its own: it reopened because REQ-01's authority moved. It closes behind `scaffold-template-removal` (`daddfc0b`), which carries the generator retarget. Do not dispatch it as a task.
- [ ] Reinstall the binary before running any gate on merged work — `crates/jit/src/**` and `profiles/jit-dogfood/**` are declared build inputs and every wave touches them.

## Traps — do not repeat these

All session-1 traps remain in force. Read `handoff.md`'s trap section in full; the flock/`cargo-ci.sh`, dirty-install, three-dot-diff, and `pgrep` traps all still apply. New this session:

- **Do NOT run `plan-review` before the plan covers every settled decision it cites.** Round 2 failed on "the plan cites D-13 and D-14 but does not decompose them" — true, and unfixable until audit `7cbefe7c` landed. The owner's sequence was amendment → audit → one re-plan; the lead gated between the first two and burned a round. A decision recorded on the container is a decision the plan must implement or explicitly hand to a named successor.
- **Do NOT reconcile the graph by comparing only edges among manifest issues.** That is what the lead did, and `breakdown-review` caught two live issues still depending on rejected ones plus thirteen roots missing their `→ 237b15be` bracket edge. Walk every edge of every issue, including edges pointing outside the manifest.
- **Do NOT remove the last edge from a rejected issue.** It becomes isolated and `jit validate` then fails repository integrity with exit 4 — not a warning. `e103d7a4` hit this. Give a rejected issue an edge to the breakdown node rather than deleting it.
- **Do NOT substitute issue ids into the manifest and assume they survive.** The lead did this twice; the planning agent rewrote the manifest from its own copy both times and the ids were lost, which nearly caused eleven duplicate issues to be created. Keep the key→id map in a file the lead owns and re-derive it every round; treat the manifest's `issue:` refs as advisory.
- **Do NOT accept a declared "footprint uncertainty" as a resolution.** `plan-review` rejected two of them. When an entry says a count was estimated rather than read, the count is wrong: `explicit-taxonomy-test-fixture` declared six call sites and the real figure was **64 across 28 files**.
- **Do NOT let `a62d444d`'s classifications stand unchecked against `D-13`.** That investigation predates the boundary ruling. Its §2.2/§10.3 called `preview_coverage_rule` engine-shaped and needing a home; it inserts the literal `"brackets"` as a label namespace and has no production caller. An issue was created on the stale classification and had to be rejected — and its criteria would have **passed review while the defect survived**.
- **Do NOT trust a per-pair overlap table without checking what its generator excluded.** The planner's generator silently filtered delivered entries; the reviewer named a pair on `commands/validate.rs` it could not have found. Unfiltered, the count went 46 → 70 pairs.
- **Do NOT read an idle notification as a stalled agent.** The lead did, and messaged a planning agent mid-flight asking why it had done nothing. The working tree was clean because the agent had not written yet.
- **Membership residue:** a rejected issue keeping its `epic:`/`milestone:`/`story:` labels is a `plan-review` finding. Stripping them is correct and produces advisory `orphan-leaf` warnings, which are the honest state — `validate_orphans` (`crates/jit/src/domain/type_taxonomy.rs:420`) has no state filter, so this cannot be cleared by archiving.

## Open questions needing invoker input

None blocking. Two standing observations the next lead should raise if they become live:

- The container is very large for one `holistic-review`: 72 entries, 8 waves, 7 stories. Splitting the configuration-boundary work into a successor epic was offered and declined (`D-10`, reaffirmed when the scope widened). Raise it again only if the epic gate proves unworkable, not as a preference.
- `@/issue/e204e63d/decision/D-19` ships the resolver with no runtime compatibility check. The exposure is narrow — the release archive keeps package and binary together — but it is real for an adopter who keeps a package across a `jit` upgrade. The owner took it knowingly with the consequence stated.

## Reference artefacts

- Epic: `jit issue show e204e63d` — carries `D-7` … `D-23` and the nine live criteria (REQ-06 is retired, not reused).
- Plan: `dev/active/e204e63d-derived-package-sources/e204e63d-plan.md` — passed plan-review at round 5.
- Manifest: `dev/active/e204e63d-derived-package-sources/e204e63d-breakdown.json` — 72 entries, 101 edges, authoritative.
- **Boundary audit:** `dev/active/7cbefe7c/findings.md` — the inventory, the ordering, the per-issue verdicts, the costs with no route back. Cited as `A<n>.<m>` in the plan.
- Profile-extraction investigation: `dev/active/a62d444d/findings.md` — cited as `F<n>.<m>`. **Predates `D-13`; check its classifications before relying on them.**
- Container investigation: `dev/active/e204e63d-derived-package-sources/e204e63d-investigation.md` — cited as `C`/`Q`/`S`.
- Progress file: `dev/active/e204e63d-derived-package-sources/progress.json` — gate history, owner rulings, graph state.
