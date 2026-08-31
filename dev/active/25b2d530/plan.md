# Implementation plan: a gate evaluation executes its checker at most once

> **Diátaxis Type:** Explanation

The change plan for issue 25b2d530. Line references are to the tree at the time of writing.

## The defect

`CommandExecutor::check_gate_with` (crates/jit/src/commands/gate_check.rs:877-1035) wraps
capture + checker execution + publication in ONE `with_mutation_attempts("gate evaluation", ...)`
closure. Each attempt:

1. opens a mutation session, captures a gate-evaluation image, derives issue/registry/gate/
   runs and a `CapturedGateEvaluation { evidence, prompt }`;
2. executes the checker UNLESS the `cached` result from a previous attempt was derived over
   evidence equal to this attempt's evidence (gate_check.rs:937-972);
3. re-captures a final image, and returns `AttemptOutcome::Retry` when the final evidence
   differs from the evidence the cached result was derived over (gate_check.rs:1005-1007).

So whenever the repository moves while the checker runs, the closure retries, the evidence
no longer matches, and the whole checker chain re-executes. A completed verdict (status,
findings, report text) is discarded with no run record and no event. For a checker that takes
minutes (an AI reviewer), drift over that window is near certain, so the cost doubles or
triples and a real verdict is thrown away.

The same defect exists on the lifecycle precheck path: `publish_captured_lifecycle_mutation`
(crates/jit/src/commands/mod.rs:2631-2740) reuses `cached_precheck` only while
`cached.target_id == expected_target && cached.evidence == plan.evidence`, so precheck
checkers re-execute on the same drift.

## Required behaviour

A completed checker run's verdict is recorded exactly once, against the input image captured
before the run. Post-run drift is ANNOTATED on the run record; it never triggers
re-execution and never discards a finished verdict. Retrying publication (capture/apply
races) stays, because it is cheap and never re-runs the checker.

## Implementation

### 1. New domain vocabulary — crates/jit/src/domain/types.rs

Add next to `GateVerdictOrigin` (which it mirrors in shape and spirit):

```rust
/// Whether the evaluation inputs a verdict was derived over still matched the
/// repository when the run was recorded.
///
/// A checker is launched against one captured repository image. `InputsMoved`
/// says the repository changed under a checker that had already started, so the
/// verdict is the checker's own, taken over the state captured before it ran,
/// rather than over the state the record was published against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateEvaluationStability {
    /// The evaluation inputs held still from capture through recording.
    #[default]
    Stable,
    /// The evaluation inputs moved while the checker ran.
    InputsMoved,
}
```

Add to `GateRunResult` as the LAST field (declaration order matters, see 2):

```rust
    /// Whether the evaluation inputs this verdict was derived over still
    /// matched the repository when the run was recorded.
    #[serde(default)]
    pub evaluation_stability: GateEvaluationStability,
```

Re-export it wherever `GateVerdictOrigin` is re-exported (crates/jit/src/domain/mod.rs and
any `pub use` chain). Follow the existing export shape exactly.

### 2. Record-field inventory — crates/jit/src/storage/reference.rs

Add `GateRunField::EvaluationStability` as the last variant, bump `ALL` to 22 entries in
declaration order, add the `as_str` arm `"evaluation_stability"`, and write its
`description()` text in the register's voice (see the `Origin` and `TreeDirty` entries for
tone; state what the field records and why a reader cares — a verdict over inputs that moved
is the checker's verdict over the state captured before it started, not over the state the
record was published against). Update the presence column the same way `origin` is handled
(always present). Update `sample_gate_run` and `sample_gate_run_minimal`.

There are conformance tests in that file (around lines 600-660) holding `GateRunField::ALL`
against the schemars schema of `GateRunResult`; they must pass without weakening.

### 3. Every `GateRunResult` struct literal

26 construction sites across crates/jit/src (gate_execution.rs, storage/gate_runs.rs,
storage/reference.rs, output.rs including a DOCTEST at output.rs:2426-2452, main.rs,
commands/mod.rs, storage/repository_state_store_tests.rs, and test modules). Add
`evaluation_stability: GateEvaluationStability::Stable` (or `Default::default()` where the
site already uses that idiom for neighbouring fields).

### 4. Output projection — crates/jit/src/output.rs

`GateRunSummary` gains `pub evaluation_stability: crate::domain::GateEvaluationStability`
(always serialized, like `origin`), mapped in `GateRunSummary::build`, with a doc comment in
the style of the neighbouring `origin`/`inputs_digest` comments. Update the doctest.

### 5. Web type mirror — web/src/types/models.ts

`GateRunSummary` there mirrors the Rust projection. Add
`export type GateEvaluationStability = 'stable' | 'inputs_moved';` next to `GateVerdictOrigin`
with the same comment style, and add the field to the `GateRunSummary` interface.

### 6. THE FIX — crates/jit/src/commands/gate_check.rs `check_gate_with`

Restructure the single retry closure into three phases so the checker call site sits OUTSIDE
every retry loop. Do not keep a `cached: Option<CachedGateEvaluation>` mutable across attempts.

Phase 1 — capture, retryable. `with_mutation_attempts("gate evaluation inputs", ...)`:
open a session, `capture_gate_evaluation_image(...)`; `None` => `AttemptOutcome::Retry`;
otherwise derive `(image, issue, issues, registry, runs, captured)` exactly as the current
code does and return them as `AttemptOutcome::Done`. The session is dropped before the
phase returns (same block scoping the current code uses), so no session is held across the
checker.

Phase 2 — execute, NOT retryable, runs exactly once. Resolve the gate from the phase-1
registry, compute `declared_inputs_digest`, take `reusable_verdict` under
`VerdictSource::ReuseUnchangedInputs` as today, and otherwise call `execute_captured_gate`
with the phase-1 image. Set `result.inputs_digest`. This code must not be inside any
`with_mutation_attempts` closure.

Phase 3 — publish, retryable. `with_mutation_attempts("gate evaluation record", ...)`:
open a session, `capture_gate_evaluation_image(...)`; `None` => `Retry`. Derive
`final_issue`, `final_registry`, `final_gate`, `final_runs`, `final_captured` exactly as
today. Then, instead of retrying on evidence mismatch:

```rust
let stability = if final_captured.evidence == captured.evidence {
    GateEvaluationStability::Stable
} else {
    GateEvaluationStability::InputsMoved
};
```

Write `stability` onto the draft result before building the intents, then build and apply
the same three intents (`UpdateIssue`, `RecordGateRun`, `RecordEvent`) as today. An apply
conflict still returns `Retry` — and because the verdict is already in hand, the retry
re-captures and re-publishes without touching the checker.

Keep every other behaviour: bound-target resolution, run-id minting, the returned
`GateRunResult` carrying `run_id`, the `VerdictSource::Execute` path, error propagation.

Update the doc comments on `check_gate` / `check_gate_with` so they state the new contract:
the checker executes at most once per invocation, and a verdict is recorded against the
image captured before the checker started, with drift reported on the record. Sweep for any
other comment or doc in the crate that describes the old re-execution behaviour and fix it
in this change.

### 7. Lifecycle prechecks — crates/jit/src/commands/mod.rs

In `publish_captured_lifecycle_mutation`, change the `cached_precheck` reuse condition from
`cached.target_id == expected_target && cached.evidence == plan.evidence` to
`cached.target_id == expected_target` alone, so prechecks execute at most once per
transition. When the cached evidence differs from the current attempt's `plan.evidence`,
set `evaluation_stability = GateEvaluationStability::InputsMoved` on the precheck runs that
get recorded (the `precheck_runs` copies feeding the intents), leaving `Stable` otherwise.
Keep the `target_id` guard and every other retry trigger (`expected_target` change,
`current_precheck_required` change, lease-mode change) as they are. Update the comment on
`CachedPrecheckExecution` (mod.rs:514) to state the new rule.

### 8. Docs

- Regenerate `docs/reference/storage-records.md` with `scripts/generate-storage-records-reference.sh`
  (do not hand-edit the generated file).
- `docs/reference/cli-commands.md`: under the `jit gate evaluate` **Behavior** section, add a
  short bullet group stating that a checker executes at most once per evaluation, that its
  verdict is recorded even when the repository changed while it ran, and that the run record
  and `--json` gate-run output report `evaluation_stability` as `stable` or `inputs_moved`.
  Keep it factual and in the page's existing voice; the repository has a `single-source-prose`
  invariant, so state the fact once and do not duplicate the generated field table.

## Tests (write these; test-first)

In `crates/jit/src/commands/gate_check.rs` the following existing `#[cfg(unix)]` tests encode
the OLD behaviour and must be rewritten to the new contract (keep their fixtures and races,
change the assertions, and rename them so the name describes the new property):

- `test_check_gate_reexecutes_when_prompt_changes_during_checker` (line ~4088): checker-count
  must now be `"x"` (one execution), exactly one recorded run, and that run's
  `evaluation_stability == InputsMoved`. The context the checker saw is the PRE-run prompt
  (`"old prompt"`), not the mutated one.
- `test_check_gate_rebases_final_update_when_issue_changes_during_checker` (line ~4122):
  checker-count `"x"`; the checker's context carries the pre-run title `"Test"`; the STORED
  issue still ends at `"Changed concurrently"` (publication rebases onto the final image);
  the recorded run is `InputsMoved`.
- `test_check_gate_reexecutes_when_direct_dependency_changes` (line ~4152): checker-count
  `"x"`, recorded run `InputsMoved`, context carries the pre-run dependency title.
- `test_check_gate_reexecutes_when_gate_definition_changes_during_checker` (line ~4194):
  checker-count `"x"`, one recorded run, `InputsMoved`, context carries the pre-run gate title.
- `test_check_gate_reexecutes_for_new_same_issue_run_but_not_unrelated_run` (line ~4307):
  first half — checker-count `"x"`, run `InputsMoved`; second half (unrelated run) —
  checker-count `"x"`, run `Stable`. Rename to describe drift classification, not re-execution.
- `test_check_gate_reuses_checker_result_after_apply_only_conflict` (line ~4364): keep as is
  and additionally assert the recorded run is `Stable` — an apply-only conflict is not drift.

Add:

- A test that an evaluation whose inputs moved records exactly ONE gate-run record, ONE
  `GatePassed`/`GateFailed` event, and one `gates_status` update — the REQ-02 property.
- A test that a FAILING verdict from a checker whose inputs then move is recorded (status
  `Failed`, findings preserved, issue left non-passed) rather than discarded. This is the
  production failure being fixed, so it must be covered explicitly.
- A lifecycle test in `crates/jit/src/commands/mod.rs` (or wherever the precheck tests live)
  that a transition whose precheck checker mutates the repository executes that checker once
  and records its run once, marked `InputsMoved`. Use the existing precheck fixtures
  (`precheck_evidence_fixture`, `capture_precheck_plan`) as your starting point; if a
  file-backed repository is needed, follow the `setup_file_repository` pattern in
  gate_check.rs.

The existing `test_precheck_evidence_detects_*` tests in mod.rs stay unchanged — they test
drift DETECTION, which is still required, only its consequence changed.

Test naming follows `test_<function>_<scenario>` and describes the scenario in full.
Assert semantic properties, not field inventories. Do not add `#[ignore]` to anything: the
repository has a `no-ignored-tests` invariant.

## Validation

```
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --workspace --all-targets      # must be zero warnings
cargo test --workspace                      # whole suite, not only your new tests
cd web && npm run lint && npm run build && npm test
```

Also run `scripts/generate-storage-records-reference.sh` and confirm the regenerated doc is
the one committed. Run `scripts/docs-check-projections.sh` and `scripts/docs-check-links.sh`
if they exist and are cheap.


