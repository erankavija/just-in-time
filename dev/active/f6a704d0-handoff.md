# Handoff — Generic issue & label validation engine (f6a704d0) — session 1

**Date:** 2026-06-07
**Session number:** 1
**Prior handoffs:** None

## Current state

- Epic: `f6a704d0` — state: backlog (blocked by remaining children; cannot go in_progress until they're done — `jit issue claim` auto-transitions and fails while blocked, so the epic was never claimed; this is expected, not an error).
- Wave in progress: wave 6 of 7.
- Children summary: 9 done, 2 in_progress (a0f0f342, a6daa05d), 1 pending (0abaddc0).
  - Done: 2f50a3b0 proj, 2fa7c882 rules-loader, 7df61f62 core, 00525fe0 htmlxml, 6297d67b sugar, 33f23ec7 keyword, a7176f28 graph, 25ad2a02 local, b8ba1b10 validate.
- Active claims: a0f0f342, a6daa05d, 0abaddc0 claimed as agent:claude (claimed this session).
- Open escalations: None awaiting input (3 resolved this session — see below).
- Progress file: `dev/active/f6a704d0-progress.json`.
- main HEAD: `5a83744`. Working tree clean. All wave 1-5 work merged + gate-green on main.

## What just happened

- Waves 1-5 complete: all 9 children merged to main, each passing cargo-ci + code-review gates + the 6-tier lead review. Heavy review iteration throughout (local-eval 25ad2a02 took 6 code-review rounds + an adversarial review).
- Wave 6 dispatched (defaults a0f0f342 + sdd a6daa05d) in parallel worktrees.
- a6daa05d (sdd): worker done -> code-review FAIL (req-derivation not enforced; schema missing requirement/scenario structure) -> rework merged -> adversarial review (found a concept-doc overclaim, fixed) -> code-review round 3 FAIL (req/satisfies rules used `scope = "global"`). LEAD-FIXED on main (commit `5a83744`): changed the two reference rules to `scope = "linked"` + added cross-epic isolation test; 21 example tests pass. **NOT yet re-gated.**
- a0f0f342 (defaults): worker done -> code-review FAIL (unknown-type check dropped; orphan/strategic not default rules; namespace-registry parity) -> rework merged (added `Assertion::TypeHierarchy` graph kind reusing `validate_orphans`/`validate_strategic_labels`; unknown-type rule; etc.) -> adversarial review found registry write-block parity bug (needed BOTH `enforce_namespace_registry` AND `reject_malformed_labels`) + `default:` prefix collision risk; LEAD-FIXED + tests -> code-review round 3 FAIL (residual INLINE label-format + uniqueness enforcement in command code = not single-source-of-truth; label_regex parity). Dispatched **rework attempt 2 (consolidation)** -> worker reported done on branch `worktree-agent-a0f0f342` @ `fcf3039` (removed inline validate_label/uniqueness from create/update/bulk/add_label + labels.rs helper; split label-format into canonical-always-enforced + label_regex-write-only; flipped namespace-unique to enforce=true; live-repo safety test). **fcf3039 is NOT merged, NOT gated.** Its worktree dir was removed to free disk; the branch ref is preserved.
- Process change adopted (user directive): run an ADVERSARIAL reviewer before every code-review gate. Saved as memory `feedback-adversarial-review-before-gate`.
- Added a dedicated gate `cargo-ci-features` (user-approved) required on 00525fe0.
- Fixed a pre-existing flaky `httpbin.org` network test (now a loopback mock) that was intermittently failing cargo-ci and consistently failing code-review in the offline reviewer sandbox (commit on main, user-approved approach "point at local mock").

## What to do next

- [ ] **Land a0f0f342 (defaults consolidation):** `git merge --no-ff worktree-agent-a0f0f342` (branch @ fcf3039) into main; resolve any conflicts; run cargo-ci; run an ADVERSARIAL reviewer (focus: residual inline enforcement gone? write-path parity exact? live-repo not newly blocked?); fix findings; then `jit gate pass a0f0f342 cargo-ci` and `jit gate pass a0f0f342 code-review`; on pass `jit issue update a0f0f342 --state done`. (rework_counts a0f0f342 = 2 = MAX_REWORK_ATTEMPTS — if the code-review gate FAILS again, ESCALATE per escalation-policy entry 5 rather than dispatching a 3rd rework.)
- [ ] **Re-gate a6daa05d (sdd):** the linked-scope fix is committed on main (5a83744) but not gated. Run `jit gate pass a6daa05d cargo-ci` + `code-review`; on pass mark done. (rework_counts a6daa05d = 1.)
- [ ] **Wave 7 — init (0abaddc0):** depends on a0f0f342 (done). Read its description; dispatch (single-issue wave) in a worktree; it scaffolds `.jit/rules.toml` in `jit init` + does the one-time config->rules migration (the legacy `[validation]`/`NamespaceConfig` value/pattern/required/unique fields were intentionally KEPT by a0f0f342 as the data source — 0abaddc0 migrates and removes them; loader must warn-on-removed-keys, never hard-error).
- [ ] **Epic completion (Section 10):** once all children done, verify success-criteria coverage, run `jit gate check-all f6a704d0` (epic gates: cargo-ci, code-review), produce completion report, mark epic done, archive progress file.

## Traps — do not repeat these

- **`jit gate pass <id> <gate>` exits 0 even when the gate VERDICT is FAIL.** Always read the recorded status: `jit gate check <id> <gate> --json` -> `.status`; for failures read `.jit/gate-runs/<run_id>/result.json` `.stdout`. (Memory: `project-jit-gate-pass-exit-code`.)
- **The `code-review` gate's verdict parser reads the LAST non-blank line.** If the reviewer emits prose AFTER `VERDICT: PASS`, the parse fails and the gate is recorded FAILED ("Could not extract VERDICT"). If a code-review FAILs with no `## Issue` findings and the body looks like a PASS, check the trailing line — re-run the gate (non-deterministic; usually places the verdict last on retry).
- **Do NOT trust a cargo-ci "passed" poll without confirming a FRESH run_id.** A stale prior run can read as passed while the new run is still in flight (this masked a real failing unit test once this session). Capture the prior run_id and wait until `run_id` changes before trusting the status.
- **Sub-agent worktrees share one disk (96% full at peak).** Always prune merged worktrees (`git worktree remove`) promptly; concurrent `cargo test` across worktrees on the shared `target/` can cause rlib-collision / "Disk quota exceeded" noise. Dispatch script anchors worktrees on current main HEAD; main must be clean before dispatch.
- **Validator cache keying is settled — do NOT reopen.** The engine caches compiled validators by the schema's CANONICAL SERIALIZED STRING (true identity, `engine.rs::schema_key`), NOT by rule name and NOT by a u64 hash. The decision record §5.2 AND issue 7df61f62's criterion were amended (user-approved) from "once per rule" to "once per distinct schema". Three review rounds were lost here; the contradiction is resolved.
- **defaults migration parity is subtle — preserve it exactly.** Legacy enforcement had ALWAYS-ON pieces (inline `validate_label` canonical format + inline unique-namespace reject, both hard on write regardless of flags) AND CONFIG-GATED pieces (`label_regex` write-only gated by reject_malformed; namespace-registry block gated by `enforce_namespace_registry && reject_malformed_labels`; namespace values/pattern/required = validate-time error, never block write). The consolidation (fcf3039) maps these to: `default:label-format` canonical enforce=true always; `default:label-format-custom` label_regex enforce=reject_malformed (write-only); `default:namespace-unique:<ns>` enforce=true; `default:namespace-registry` enforce = `enforce_namespace_registry && reject_malformed_labels`. When reviewing fcf3039, verify these mappings and that THIS repo's loose config (all write-block flags false) does not newly block normal `jit issue create/update/claim` (else the lead's own workflow breaks). There is a `parity_live_repo_loose_config_does_not_block_normal_labels` test for exactly this.
- **Two parity tests previously baked in the WRONG registry behavior** (asserting block on `enforce_namespace_registry` alone). Fixed this session; if you see a registry parity test asserting single-flag blocking, it is wrong — block requires BOTH flags.
- **SDD examples live under `docs/examples/`, NOT the live `.jit/`.** Writing example rules to the repo's real `.jit/rules.toml` would activate them on this repo. Keep examples inert under docs.
- **Run an adversarial reviewer BEFORE the code-review gate** (user directive; memory `feedback-adversarial-review-before-gate`). The adversary catches what the gate would, in one pass. Note: this session's wave-6 adversary missed the residual-inline-enforcement finding — when reviewing migrations/refactors, explicitly instruct the adversary to check for DUPLICATE/residual enforcement in command code, not just the new code.

## Open questions needing user input

None. (User said "stop here" mid-wave-6; resume per "What to do next".)

## Reference artefacts

- Epic: `jit issue show f6a704d0`
- Decision record: `dev/active/f6a704d0-validation-engine.md` (amended §5.2 this session)
- Implementation plan: `dev/active/f6a704d0-validation-engine-plan.md`
- Progress file: `dev/active/f6a704d0-progress.json`
- Unmerged work: branch `worktree-agent-a0f0f342` @ `fcf3039` (defaults consolidation rework — merge + gate next).
- Dispatch/leak-check scripts: `~/.claude/skills/project-lead/scripts/`
