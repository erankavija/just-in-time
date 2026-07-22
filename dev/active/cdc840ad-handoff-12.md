# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 12

**Date:** 2026-07-22T16:43:09+03:00
**Session number:** 12
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` through `dev/active/cdc840ad-handoff-11.md`

## Current state

- Epic: `cdc840ad` — state: backlog
- Wave in progress: wave 5 of 9
- Children summary: 4 done, 1 in_progress, 6 backlog, 0 rejected
- Active claims: `49adf23b` is claimed by `agent:worker` since 2026-07-19T16:45:06Z
- Open escalations: None. The rules/init cleanup escalation was resolved by the invoker's option 1 authorization.
- Repository: clean `main` at `81d6cace`
- Progress file: `dev/active/cdc840ad-progress.json` (reflects the above)

## What just happened

- Accepted and committed atomic archive transaction cutover as `a5beffef`; deleted archive staging/rollback predecessors. The commit was net -756 lines.
- Accepted and committed captured graph/snapshot repository export cutover as `ecb28939`; repository-contained snapshot payload is capped at 128 MiB before eager buffering, external publication streams, and post-commit durability failures surface as warnings.
- Confirmed no process constantly holds substantial RAM. Remaining snapshot memory cost is transient: owned intent/finalizer/plan structures can hold multiple bounded payload copies.
- Deleted the dead rules publisher cluster, `RuleMembershipSync`, `CommandExecutor::init`, always-empty `FreshInitResult.warnings`, writer-only tests, and obsolete control-plane helpers.
- Independent review found canonical re-init had not inherited the deleted rules write-through behavior and cumulative in-memory fixtures were red; escalation opened because `49adf23b` had reached its rework limit.
- Invoker selected option 1. The correction reset the counter, composed rules/header/membership/schemas from final profile-overlaid config/rules with captured-base preimages, restored pure parser/splice regressions, and migrated exactly the 13 `fast_rules` plus 2 server fixtures.
- Two independent final reviews passed. Validation passed: init 6/6, rules-document 9/9, `fast_rules` 167/167, server document API 5/5, strict workspace Clippy, formatting, and diff checks. Full workspace retained only the established unrelated 7 failures: one dangling raw-ID dependency test and six sandbox socket-bind failures.
- Committed the accepted rules/init predecessor cleanup as `81d6cace`; commit delta is +532/-1894, net -1362.

## What to do next

- [ ] Resume wave 5 on `49adf23b`; re-read every prior handoff's **Traps — do not repeat these** section before dispatch.
- [ ] Make the next package the broad fixture migration required to delete `IssueStore::init`: remove the trait method, JSON partial publisher, memory no-op, and two forwarding test-double implementations; migrate the roughly 199 JSON/memory/server fixture calls to explicit canonical repository-state setup or exact malformed-state preimages. Do not add a generic legacy-init compatibility helper.
- [ ] Keep that package focused on initialization fixtures. Run affected Rust/server suites, doctests, strict Clippy, formatting, and absence scans before independent review.
- [ ] Then delete the remaining repository-publishing `IssueStore` methods and implementations/test doubles (`save_issue`, `restore_issue_verbatim`, `save_gate_registry`, `append_event`, `write_repo_file`, and any surviving peers) after migrating their fixture callers to aggregate typed state. Keep read/query and external-control-plane capabilities only.
- [ ] Run the full §2 live-tree absence scans and cumulative gates only after all final IssueStore publisher/test-fixture debt is gone. `49adf23b` remains in progress and its formal gates remain pending.
- [ ] Do not start wave 6 (`661d6be2`) until `49adf23b` is done.

## Traps — do not repeat these

- **All unresolved traps in handoff 11 and earlier remain binding.** Load the full handoff chain; do not rely only on this summary.
- **Do not retain `IssueStore::init` for tests or replace it with another generic scaffold helper.** JSON performs a real partial multi-file publication while memory does nothing; the mismatch is the bad abstraction. Migrate normal fixtures to canonical repository-state setup and seed malformed/partial cases explicitly.
- **Do not assume `CommandExecutor::init` initialized an in-memory repository or attached layout.** Its memory storage call was a no-op and it could not mutate the executor's layout. The red `fast_rules`/server fixtures required captured config/rules/schema bytes and explicit `with_layout`, not restoration of the wrapper.
- **Do not delete filesystem-writer tests without preserving their pure semantic cases.** The first cleanup attempt lost simultaneous add/drop, indented `[[rules]]`, and `[[ruleset]]` false-positive coverage. Keep byte-transform behavior in `repository_state::rules_document`; delete only wrapper mechanics.
- **Do not derive re-init rules from the captured base config when a profile supplies final config/rules.** The dogfood profile adds hierarchy entries and unique `brackets`; composition must use final profile-overlaid authority while every action retains the captured base preimage.
- **Do not reintroduce separate schema/rules writes on re-init.** The corrected finalizer excludes raw generated-schema actions, invokes the existing pure materializer once, replaces same-path raw actions, and rebases the derived actions into the one recovered plan.
- **Do not claim snapshot directory publication is instantaneously observer-atomic.** D17 promises one captured recoverable delta and convergence, not a whole-tree rename. External directories use Linux no-replace rename; repository directories use the transaction kernel.
- **Do not describe the snapshot RAM issue as a resident leak.** There is no long-lived holder. Repository-contained outputs are capped at 128 MiB payload but can transiently occupy multiple owned copies; external outputs remain the low-memory path.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Active issue: `jit issue show 49adf23b`
- Plan: `dev/active/cdc840ad-plan.md`
- Progress: `dev/active/cdc840ad-progress.json`
- Prior handoff: `dev/active/cdc840ad-handoff-11.md`
- Accepted commits: `a5beffef`, `ecb28939`, `81d6cace`
- Historical rules decisions with supersession notes: `dev/active/af4c901a-derive-default-rules-at-load.md`, `dev/active/d74a9ed1-write-through-namespace-unique-membership.md`
- Installer worktree: `.agents/worktrees/lead-install-clean`
