# 5046408a independent design review, revision 2

Verdict: **FAIL — two bounded corrections remain before implementation.** The revised design incorporates D-26 and closes the original missing-target representation problem. It also fixes the consumer authority inventory, removed HEAD-tier statement, and symlink policy. The remaining recovery/policy ordering gap and incomplete exact footprint prevent accepting the full implementation plan.

Authority checked read-only: `git show main:dev/vision/9db27a3a-charter.md`, D-26 at line 479. D-26 requires the original absolute no-follow root and identity, validation before replay, tentative lock selection followed by revalidation, recovery before new root selection, and unchanged D-25 reads. Reviewed the revised `dev/active/5046408a/design.md` and the current implementation; no builds, gates, implementation, tracker edits, or commits.

## Findings

1. **High: the pending-record path still has no defined source for the pre-lock linked-data write decision.**

   `design.md:35` defines coherent declaration capture only when no record is pending. `:39` permits a record peek “only to select locks,” requires recovery before final config selection, and also requires pre-lock linked-data refusal. These obligations do not specify how the config-derived refusal is evaluated when the pending record changes configuration.

   The current boundary (`commands/bootstrap.rs:215`, especially `:224`) calls `ConfigManager::get_write_policy`, which loads and assembles configuration (`config_manager.rs:96`, `config.rs:1223`). The current implementation's raw read cannot be retained as the new protocol: it can observe a config preimage or postimage depending on where replay stopped. Conversely, using the new clean-state admission path would return Busy for every pending record, preventing mutation recovery in a linked data checkout. Replaying first without a defined policy exception violates the stated pre-lock refusal promise. D-26 settles replay targets; it does not itself state a linked-write-policy exception.

   **Required resolution:** specify the pending-record policy view and its admission/revalidation, including a record changing `write_policy` and a partial apply. Two concrete choices are available for the steward:

   - Preserve the pre-lock guard by constructing a tentative *committed declaration postimage* from the same redo record: use its puts/deletes for touched declaration paths and no-follow reads for unchanged declarations, feed the existing assembler, and validate the record/root/unchanged inputs around this candidate and again under the selected locks. This is only a mutation bootstrap candidate, never an admitted reader snapshot. An unstable candidate must retry/conflict, not publish a policy result. The design must say that config postimage supplies policy only; D-26's recorded root supplies replay targets. Do not create a second TOML parser or a second durable authority.
   - Explicitly authorize recovery of already committed work before applying the current linked-write policy to new work, with the corresponding exception to the pre-lock/no-side-effect refusal contract. This is simpler but changes the policy contract and cannot be inferred from “preserve” wording.

   Recommend preserving the guard via a precisely defined committed-declaration candidate if strict preservation remains required. Either choice needs steward acceptance and an owning test. The worker must not choose an undocumented raw-config, Busy-forever, or guard-bypass behavior. Cover config allow→refuse and refuse→allow, partial config application, explicit invocation override, and candidate replacement before locks. Preserve the original record's audit bytes; recovery must not synthesize a second mutation audit event.

2. **Medium: the declared exact footprint omits existing owners that necessarily change under D-26.**

   `design.md:65` requires the in-memory equivalent to enforce the same binding, but `storage/memory.rs` is absent from the concrete source/test lists. Its `MemoryRedoApplier::apply` (`:576`) is an independent current application entry point; it resolves operations and writes entries without checking a layout binding. Its direct worktree-record fixtures also need the new binding contract. Add this file explicitly and keep the common structural/binding rules shared; backend-specific capability checks remain at the appropriate boundary.

   `crates/jit/tests/cli_issue/recover_command_tests.rs:94` constructs a worktree-bearing redo record without a binding. The new invariant necessarily requires updating that fixture, but the file is absent from `design.md:87`. Add it to the footprint as an existing boundary fixture, not a new duplicate replay matrix. If the canonical constructor signature changes for data-only callers too, include the existing `crates/jit/tests/cli_repo_workflow/lease_enforcement_tests.rs:541` call site; an API preserving an explicit data-only construction form can avoid that extra edit.

   Add `crates/jit/tests/cli_repo_workflow/linked_checkout_write_policy_tests.rs` for the finding-1 CLI boundary; it owns the existing refusal-before-session/no-residue and override audit behavior (`:210`). Keep the policy/capture race matrix in the storage/bootstrap owner and test only the CLI's added boundary here. These are contained footprint corrections, not new work streams or Cargo targets.

## Prior findings: closure table

| Prior finding | Status | Revision-2 evidence |
| --- | --- | --- |
| 1. Missing durable original-root authority | **Closed at design level.** | D-26 and `design.md:37–39` select one typed root path/identity binding in the same record, require it exactly for worktree operations, reject unsupported wire versions, refuse moved/replaced/symlinked roots, and replay against the original root before selecting another. Footprint correction in new finding 2 remains. |
| 2. Configuration-to-layout admission | **Partly closed; pending-policy branch remains open.** | `:35` retains the data observation through consumer capture, revalidates mutation selection under locks, restarts on movement, and covers claim-first/retained handoffs. `:39` gives ordered candidate/revalidate/replay/reselect sequencing. New finding 1 is the missing branch needed to make that protocol executable for linked data stores. |
| 3. Consumer authority and projections | **Closed at design level.** | `:45–54` makes document content/history/assets consistently code-relative; keeps guard/coordination/data identity distinct; preserves code root in projected layouts; permits data-only primary access instead of fake/equal-root layouts; makes worktree output match coordination branch/root. |
| 4a. Unsupported committed-HEAD fallback claim | **Closed.** | `:54` explicitly preserves local/primary fallback and absence of the HEAD issue tier, consistent with `storage/json.rs:1116`, `:2448`. |
| 4b. Ambiguous symlink canonicalization | **Closed.** | `:31` rejects every symlinked ancestry component, uses no-follow evidence, retains containment checks, and wraps errors with setting source. Git observation alias caching remains separate. |

## Protocol and scope assessment

Apart from finding 1, the proposed lock sequence is consistent: tentative data-only record observation; recorded worktree bootstrap lock first when required; data bootstrap/repository/events locks next; reread record and root identities under serialization; replay only the validated record; release recovery guards before selecting and admitting a different layout. Data-only records need no fabricated code root. Record replacement requires dropping guards and restarting, never acquiring a second worktree lock while holding data locks. Missing-root failures must be mapped to the typed recovery error even if opening that root or its bootstrap lock fails before the final under-lock validation.

The clean-state read handoff preserves D-25 only when the original config-selection observation participates in admission of the actual consumer capture. The plan now states this explicitly and names storage as the retry/admission owner. A changed token must not be discarded merely because a later capture is internally consistent. Claim-first reopen and retained-session tests are correctly listed. The same requirement applies to server operations using the opened executor; implementation review must verify that a stale root cannot be paired with newly read config and that failed admission returns the typed bounded conflict.

The root binding must be derived on both ordinary publication and the existing absent-data-root stage publication (`storage/repository_state_store.rs:967`). The revised “derive from the committing session” contract covers both; its test-first implementation must preserve that fact. Worktree-root identity preflight occurs before *all* replay operations, including data puts, as D-26 requires. No new history scan, root flag, sidecar, relocation search, or Git policy is needed.

The authority table preserves the steward's data-history guard ruling. Opposing data/code histories in both directions, non-Git data invoked from Git code, and non-Git code with Git data are in the planned wiring tests. Common-dir/lease/claims authority remains data or the preexisting relocated coordination context; a third independent code context does not replace it. Document and projected-primary tests cover the prior concrete inconsistency. Profile checks are assigned to the checkout owning affected paths; implementation should partition by existing declared authorities without adding per-consumer discovery.

Process bounds remain correct as stated at `:60`: ordinary one probe, external bootstrap at most two, external traversal $N+1$, independently selected data/code contexts at most one probe each, and a possible third relocated coordination context. Count failed contexts and changed selection contexts; reuse the invocation cache. Do not count unrelated descendants as aliases or conflate HEAD operations with layout probes.

REQ-01–08 are all represented in settings, precedence, default parity, split-root opening, checker/evidence, coordination, typed errors and the two canonical documentation homes. Acceptance remains design-level only: findings 1–2 must be resolved, then implementation and configured checks must establish the criteria. The existing owning test matrix is appropriately layered, uses child-only environment and shared fixtures, and creates no new Cargo target. The missing footprint owners above should be added before the steward accepts that footprint.

## Next action

Resolve the pending-record linked-policy branch and add the omitted concrete files. Then re-review those bounded changes and obtain steward exact-footprint acceptance. D-26's original-root choice does not need to be reopened; no user approval or manifest expansion is required.
