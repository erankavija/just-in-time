# Epic complete: Compute one validation report and derive every validation view from it (9876a23a)

**Started:** 2026-11-03 (lead just-in-time-a4; plan audit against 120402067)
**Completed:** 2026-11-04 (lead just-in-time-b9 from wave 7)
**Assignee:** agent:9876a23a-lead

## Summary

Repository validation is one pure function, `validation::report`, over one captured image. Every view (whole, per-issue, explain, rule, scope) and every built-in validation checker calls `ValidationReport::select` or `explain` on it, and one property test owns the selection contract. Capture, issue creation, `validate --fix` and link checking stop repeating work. Per-issue validation went from 1.28 times slower than whole validation to about 13 times faster: 268 ms against 3,632 ms on this repository ([benchmark](benchmark/README.md)).

## Metrics

| Metric | Value |
|---|---|
| Children completed | 45 done, 2 rejected (145b4d57, e16493fc: superseded by the audit) |
| Waves executed | 9, plus the 7516ed50/ce221384 tail and one remediation task |
| Rework cycles | 19 across 11 issues (2b5f452c 3; c92f4011, 5d5a1794, 3acaa5a3, 1f24e3e9 2 each; ea843d26, f968dfe1, 6ba4f623, 612ee9d4, a106d1be, f22147ec 1 each) |
| Escalations | 7 by just-in-time-a4, then 9 by just-in-time-b9 (below); all resolved by the steward |
| Issues created during execution | 2a10917d, 14f49a28 (standalone), e2082ba9 (bug), 4b66bc6b (epic-review remediation) |
| Net production lines (`scripts/production-lines.py --test-support-as-test`, first-parent merges) | **≈ −399** |

Net production lines by wave: 1 +3, 2 −1, 3 −42, 4 +35, 5 +55, 6 −32, 7 +464, 8 −510, 9 −369; then 7516ed50 −2. The audit's estimate was ≈ −1,050; the re-estimate on 2026-11-04 was ≈ −470 (range −380 to −560). The selection machinery (612ee9d4, +286), the duplicate-key-preserving decoder (2b5f452c, +96, steward D-02) and the e2082ba9 fix were unforeseen; the views, checkers and executor removals paid them back.

## Success criteria

- [x] REQ-01 one pure report function — 6ba4f623, 2b5f452c, 612ee9d4
- [x] REQ-02 every view is a selection over one capture; only whole validation reads history — 2c8c13b2, 86f7e6eb, 208edf4c, 612ee9d4
- [x] REQ-03 checkers report what `jit validate` reports for their selection — a106d1be (same-image parity test)
- [x] REQ-04 one test owns the selection contract — 728ecfb0, extended by 208edf4c REQ-05
- [x] REQ-05 no executor pass copies or checker pipelines — 6ac41880, a106d1be, 3acaa5a3
- [x] REQ-06 validation layer names nothing in `commands` — d40d6f74, 8b56c215, 03bf6983; claims-index check removed upstream (DEC-09)
- [x] REQ-07 one address index, item kinds resolved once — 001a9c46, ea843d26, e2082ba9
- [x] REQ-08 per-issue no slower than whole, release build — 5d774061, re-measured on the final tree by 4b66bc6b
- [x] REQ-09 incremental capture specs, base requests once — 2f12d920
- [x] REQ-10 one loader, records parsed once, fidelity included — b3a768b3, 2b5f452c
- [x] REQ-11 each schema compiles once per report — ccd6e1fb
- [x] REQ-12 one plan resolver, one accessor per default — d40d6f74, f968dfe1, 977d01d8
- [x] REQ-13 creation findings from the mutation image, no post-publish read — 5d5a1794, 03c59566, ce221384
- [x] REQ-14 `--fix` in one mutation with one post-fix report — 1f24e3e9
- [x] REQ-15 one probe per distinct URL, timed agent, bounded pool — 7516ed50

Story gates: 420bc076, b3ef68b5 and 91edfa5c passed cargo-ci and holistic-review. Epic gates: repo-validate passed; holistic-review passed on its second run, after 4b66bc6b answered F1 (REQ-08 had been measured on an ancestor).

## Wave execution log

Waves 1–7 are recorded in [handoff.md](handoff.md) and [progress.json](progress.json). From the takeover:

- **Wave 7 close:** 612ee9d4, the report-selection machinery.
- **Wave 8:** the issue/explain, rule and scope views (2c8c13b2, 86f7e6eb, 208edf4c), the gate checkers (a106d1be), the selection property test (728ecfb0), `--fix` in one mutation (1f24e3e9) and incremental capture specs (2f12d920).
- **Wave 9:** the executor pass copies (6ac41880) and executor graph evaluation (f22147ec) removed, the validation layer's `commands` references cleaned out (03bf6983), view docs (78d897a2) and the REQ-08 measurement (5d774061).
- **Tail:** the link-probe pool (7516ed50), command-reference docs (ce221384) and the remediation re-measure (4b66bc6b).

## Key decisions (steward rulings recorded in issues and plan)

- DEC-19: the CLI and the checkers call `ValidationReport::select` directly. The per-view wrappers were removed, and the rule and scope views depend on 2c8c13b2.
- `selection-membership`: a report whose rule pass failed fails every selection but Whole (208edf4c D-02). The owning case lives in the property test.
- 728ecfb0 REQ-03 was restated against the contract: a source-finding union, and explain over matched rules only.
- DEC-04 exception: 7516ed50 no longer waits on f4d9502a's 9ecde705, which adapts when it lands.
- a106d1be D-01: "no gate-run listing" concerns the report's inputs, not the capture that publishes the run.

## Escalations (b9 session)

All were resolved by just-in-time-6a:

- Ledger re-estimate (accepted).
- The four entry-point rulings.
- 728ecfb0 REQ-03 against the contract.
- The rule-pass-failure contract gap.
- The 9ecde705 blocker.
- 1f24e3e9 cargo-ci host noise.
- The 2c8c13b2 pointer-test ownership question.
- package-drift on main (97f5f2b7, not this epic).
- The format-4 window.

## Holistic quality notes

- **The lead broke main.** Landing docs without branch CI let a hand edit to the generated `gate-presets.md` through (62f836117), which blocked every lead's landings until 96dfde725. Rule recorded: docs changes take branch CI, and generated references are edited at their source.
- **Lead briefs caused two rework rounds.** The planner-split instruction for 1f24e3e9 grew lines before rework 2 corrected it. The "no test-only capture helper" instruction for f22147ec duplicated test setup.
- **Property-test coverage.** Per the 208edf4c worker, most random images in the selection property test fail the integrity pass first. Selections are still exercised with findings, but clean Whole reports are rare. A future generator could target integrity-clean images.
- **`issue create --force`.** It records bypassed blocking failures as `local_rule_bypassed` events, not as `findings`. The docs state this, and the epic review accepted it under REQ-13.
