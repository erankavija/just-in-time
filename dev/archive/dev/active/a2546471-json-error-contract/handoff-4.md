# Handoff — Structured failure reporting across the machine-readable CLI surface (a2546471) — session 4

**Date:** 2026-07-28T22:28:00+03:00
**Session number:** 4
**Prior handoffs:** `dev/active/a2546471-json-error-contract/handoff.md`, `dev/active/a2546471-json-error-contract/handoff-2.md`, `dev/active/a2546471-json-error-contract/handoff-3.md`

## Current state

- Epic: `a2546471` — state: ready
- Wave in progress: wave 9 of 9; implementation waves are closed, final holistic review is blocked
- Children summary: 5 direct children done, 0 in progress, 0 backlog/ready, 1 rejected
- Active claims: epic `a2546471` remains assigned to `agent:jit-execution-lead`; no child is actively being worked
- Open escalations: epic rework exceeded after holistic review found the `config validate --json` non-envelope failure result
- Progress file: `progress.json` in this directory reflects the above

## What just happened

- Completed Wave 7 census rework `4437d897`, then closed story `7491da4a` with fresh cargo and code-review gates.
- Completed documentation leaf `ec029993` and Wave 8 story `9eadd135`; fixed its one missing configuration-reference link in rework cycle 1.
- First epic holistic review found pre-dispatch Clap parser errors outside the envelope.
- Created `8f5e5921`, then stopped and rejected it before any edits after the invoker reaffirmed that plan decision D-5 is authoritative.
- Created and completed documentation leaf `d780dfc1`, narrowing the canonical adopter promise to post-dispatch failures; both documentation gates passed.
- Aligned epic REQ-01/02/03/04/05/06/07 and non-goals with the invoker-authorized post-dispatch plan boundary; removed stale live arm counts from the plan in favor of the generated census.
- Second plan-aligned holistic review found `config validate --json` emits `{valid:false,...}` and exits 1 after dispatch rather than emitting an error envelope; its registry exemption preserves the gap.
- `repo-validate` passed on every epic attempt. Holistic review remains failed at commit `5510e0fa` with run `1bcf3734-606a-4a1d-9c6d-e8f3b20788fb`.
- All epic worker worktrees and temporary merged branches were reclaimed. Unrelated `lead-install-clean` and `steward-v1-readiness` worktrees remain untouched.

## What to do next

- [ ] Obtain the invoker decision on the open rework-exceeded escalation below.
- [ ] If one additional narrow rework is authorized, create a leaf task under `a2546471` to convert invalid `config validate --json` results into a classified validation error envelope, remove its exemption, and update the canonical registry/conformance and exit-projection tests.
- [ ] Run that leaf's configured cargo and code-review gates, perform the six-tier lead review, close it, and rerun epic `repo-validate` plus `holistic-review`.
- [ ] If the invoker instead keeps the validation-result exemption, obtain explicit approval to amend the epic criteria and authoritative plan with that specific exception before rerunning review.
- [ ] After epic gates pass, produce the completion report, transition the epic to Done, archive the active artifact bundle, link the archived plan and completion report, validate, and report.

## Traps — do not repeat these

- **Do not implement JSON envelopes for Clap parser failures.** The invoker reaffirmed authoritative plan decision D-5; `8f5e5921` was rejected with no code changes. Parser errors, help, and version remain pre-dispatch non-goals.
- **Do not narrow adopter prose without aligning the tracker contract.** Holistic run `ad491cd8-d877-4f1c-b2b5-9ff2756986c7` rejected documentation-only scoping because the epic criteria still said every failure. The invoker-authorized criteria now explicitly say post-dispatch.
- **Do not treat every registry exemption as proof that the container criterion is met.** `config validate` has a constructible post-dispatch invalid-config outcome, exits nonzero, and emits a non-envelope result; holistic run `1bcf3734-606a-4a1d-9c6d-e8f3b20788fb` rejected its exemption under REQ-02/04/05/07.
- **Do not use the standalone unscoped `scripts/docs-mechanical.sh` result as this epic's gate.** It reports unrelated pre-existing repository link/citation debt; configured scoped documentation gates passed. `repo-validate` is the repository-wide completion authority and passed.
- **Do not leave verifier-created `target/debug/incremental` before cargo gates.** Remove that exact rebuildable cache after each merge verifier run; the bounded-footprint gate requires incremental state absent.
- Re-read all prior handoffs' trap sections; they remain in force unless explicitly resolved there.

## Open questions needing invoker input

- Question: May the lead take one additional narrow epic rework cycle to repair the post-dispatch `config validate --json` failure result?
  - Context: the epic has consumed the configured maximum of two rework cycles; holistic review now identifies one new high-severity post-dispatch gap that conflicts with the authoritative plan's general envelope rule and its D-7 treatment of non-envelope failure payloads.
  - Options: (A) authorize one additional rework to emit a classified validation error envelope and replace the exemption; (B) keep the exemption and explicitly amend the epic criteria and plan with a `config validate` exception; (C) reject/stop the epic.
  - Recommendation: Option A, because it preserves the plan's post-dispatch contract and avoids introducing a command-specific exception.

## Reference artefacts

- Epic: `jit issue show a2546471`
- Design docs: `dev/active/a2546471-json-error-contract/investigation.md`, `dev/active/a2546471-json-error-contract/investigation-guard-docs.md`
- Planning docs: `dev/active/a2546471-json-error-contract/plan.md`, `dev/active/a2546471-json-error-contract/breakdown.json`
- Progress and handoff: `dev/active/a2546471-json-error-contract/progress.json`, `dev/active/a2546471-json-error-contract/handoff-4.md`
- Failed holistic evidence: `.jit/gate-runs/1bcf3734-606a-4a1d-9c6d-e8f3b20788fb/result.json`
- Canonical registry exemption: `crates/jit/tests/cli_issue/failure_lever_registry.toml` (`config validate`)
