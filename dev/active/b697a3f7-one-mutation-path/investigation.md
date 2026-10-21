# Investigation: Converge previews and coupled derivations on the mutation session (b697a3f7)

> Grounding for planning node 0dc73711. Four scoped investigations against current `main` (about e2f2693d8), after 842b0ac6, 3d9a0ce3 and bedf0962 changed the areas involved. Citations were re-grepped during the run; concurrent commits keep moving lines.

## Part: Mutation-session coherence and bulk update

## b697a3f7 investigation: mutation-session coherence (REQ-01 to REQ-05, REQ-20 subset)

Read at HEAD `e2f2693d8` (2026-10-21). `commands/mod.rs` moved about 35 lines during the investigation. Every `mod.rs` citation below comes from the last grep at `e2f2693d8`. Behaviour marked "reproduced" was observed with the installed `jit` binary, built at 19:21 the same day, in a scratch repository. Each of those observations was also confirmed by reading the code.

### 1. Claim classification

| # | Claim | Class | Current evidence |
|---|---|---|---|
| C1 | Deletion derives dependent Ready transitions inside its own plan | valid | `derive_issue_deletion` `commands/issue.rs:67-150`; promotion through `derive_state_transition` at `:103-140` |
| C2 | Single-issue update and automatic Done run `check_auto_transitions` after the commit. It lists every issue and opens one session per dependent | valid | `issue.rs:539-541` (update), `:844-857` (auto Done), `:860-876` (`list_issues` plus one `auto_transition_to_ready` session per candidate). Callers of `auto_transition_to_done`: `gate.rs:878` (manual pass), `gate_check.rs:1579` (`run_postchecks`) |
| C3 | The bulk path runs no cascade | **invalid-as-stated** | Bulk now runs one post-batch `check_auto_transitions` (`bulk_update.rs:292-296`, commits 59965c223 and 611675591, jit:05d1b34e). It is still a post-commit follow-up, so REQ-02 stays open |
| C4 | Assign, unassign, gate add and gate remove have dedicated captured mutations beside the general update, and the two paths write different events | valid, refined | Dedicated derivation: `apply_captured_issue_mutation` `mod.rs:1604-1700`. Single-issue `issue update` already reuses it (`issue_local_mutations` `mod.rs:974-1004`, folded in `derive_field_update` `mod.rs:1277-1320`), so the divergent path is **bulk** (`bulk: true`, `mod.rs:949-970`). See §4 for the divergences |
| C5 | Three lease-check implementations format the same missing-lease message | valid | `captured_lease_warnings` `mod.rs:776-813` (message `:803`); `check_active_lease`/`require_active_lease` `mod.rs:3337-3420` (message `:3411`); `template_lease_warnings` `template.rs:277-315` (message `:306`) |
| C6 | `require_active_lease` runs before and outside the publishing session at six sites | valid (lines stale) | `issue.rs:712` (assign), `:753` (unassign); `gate.rs:717` (add_gates), `:763` (remove_gates), `:809` (pass_gate), `:974` (fail_gate). It reads `ClaimsIndex::load` without the claims lock and takes its mode from the executor disk cache (`cached_config`, `mod.rs:2558`) |
| C7a | `preview_bulk_update` reimplements write validation | valid | `bulk_update.rs:458-495`, `compute_changes` `:498`, `projected_after_update` `:574`, `validate_update_preview` `:602-633` |
| C7b | Per matched issue it lists every issue (O(M·N)) | valid | `transition_blockers` `mod.rs:3256-3290` calls `self.storage.list_issues()` for each matched issue with a state change (`bulk_update.rs:480`) |
| C7c | It reloads the gate registry per matched issue | valid only when `add_gates` is non-empty | `bulk_update.rs:606-608` |
| C7d | It evaluates only local rules, from the disk cache | valid | `cached_config()` and `effective_rules()` at `bulk_update.rs:625-627` |
| C7e | Apply enforces graph rules the preview skips | valid | Apply goes through `derive_field_update` → `derive_state_transition` (`mod.rs:1075`, graph rules at about `:1130-1250`). Preview never evaluates graph rules |
| C7f | (new) `preview_bulk_update` has no caller | **finding** | No call in `main.rs`, `cli.rs`, the server, MCP, web or tests; `git log -S` shows it was never wired. `BulkUpdatePreview` appears only in `bulk_update.rs` and the `pub use` at `mod.rs:73` |
| C8 | REQ-20: issue-creation stamping has more than one derivation | valid | Finalizer pass 1 `repository_state/mutation.rs:765-797` (stamps `gates_status[*].updated_at`, builds `Event::IssueCreated` inline at `:788`). Pass 2 `:800-848` (re-derives state from dependencies at `:817-821`, no gate stamp, uses `Event::draft_issue_created` at `:835`). The initial Ready decision is also made at `issue.rs:209-210`, `template.rs:1383` and `batch_create.rs:569-573` |
| C9 | REQ-20: claim commands repeat the claims-coordinator opening | valid | Branch, identity, agent, `FileLocker`, `ClaimCoordinator::new` repeated at `commands/claim.rs:100-119` (acquire), `236-246` (heartbeat), `346-359` (release), `430-440` (renew), `485-495` (status), `532-542` (list), `573-583` (force-evict), `641-651` (recovery). A ninth variant with a different identity and agent policy is `claims_mutation_guard` `mod.rs:755-774` (`read_worktree_id`, falls back to the path and `system:lease-check`) |
| C10 | REQ-20: issue loading has two lookups and two treatments of git errors | valid | `JsonFileStorage::load_issue` `storage/json.rs:1176-1214` swallows git errors (`:1201`). `load_issue_or_not_found` `:1216-1254` propagates them as `PathReadError::Other` (`:1235`). f4d9502a `head-tier-removal` (d7cce445) deletes the git tier, which removes the git-error half |
| C11 | bedf0962 mode resolution | **already-done** | bedf0962 is Done. Both captured and cached paths call `JitConfig::enforcement_mode` (`config.rs:1308`). This epic changes only where the check runs |

| REQ | Class | Gap summary |
|---|---|---|
| REQ-01 | valid-and-open | In-plan corrections exist only for delete, `dep add`/`dep rm`/reduce and template edges, and each uses its own call shape. Update, reject, lifecycle transitions, auto Done, bulk and `validate --fix` correct outside the plan or not at all |
| REQ-02 | valid-and-open | Follow-up publishers: `check_auto_transitions` (`issue.rs:540,853`; `bulk_update.rs:295`), `auto_transition_to_done` (`gate.rs:878`, `gate_check.rs:1579`), and the `validate --fix` pass loop (`validate.rs:2090-2130`, one session per correction) |
| REQ-03 | valid-and-open | See §4, including the second derivation of the state field |
| REQ-04 | valid-and-open | Six pre-session sites, one between-session site (template), three message definitions. The set of issue mutations that are lease-checked is not defined |
| REQ-05 | valid-and-open | Preview is a separate pipeline and has no consumer (C7f) |

Totals: already-done 1 (C11), valid-and-open 14 (C1, C2, C4–C10, REQ-01–05; C7 counted once), invalid-as-stated 1 (C3).

**Gaps not in the background (all in scope):**

- **G1. Reject never promotes dependents (reproduced).** `update_issue_state(Rejected)` uses the lifecycle path (`issue.rs:664-682`) and `reject_issue` with a reason uses the field update (`:685-700`). Neither calls the cascade. The dependent stays Backlog and `jit validate` passes, because only Demote is a violation (`validation/repository.rs:505-530`). `docs/concepts/core-model.md:236,808` claims rejection promotes dependents.
- **G2. Leaving a terminal state never demotes Ready dependents (reproduced).** `issue update C --state done`, then `--state in_progress`: dependent D stays Ready, and `jit validate` then reports "Stored readiness must agree with the dependency graph".
- **G3. The promotion semantics differ by site.** Template edge removal promotes directly, without graph rules (`template.rs:1336-1353`). Delete (`issue.rs:103`), `dep rm` (`dependency.rs:605`) and `AutoReady` (`mod.rs:3064`) promote through `derive_state_transition`, which can be graph-rule-blocked and then records `transition_blocked` events. Demotion is always direct (`dependency.rs:964`, `template.rs:1281-1291`).
- **G4. `issue update --state in_progress` skips prechecks (reproduced).** An auto precheck gate whose checker is `false` did not block the transition. Prechecks run only on the lifecycle path (`lifecycle_requires_prechecks` `mod.rs:~885`). The state field therefore has two derivations, each with its own Done→Gated divert (`derive_field_update` `mod.rs:~1380-1405` and the lifecycle path at about `mod.rs:3100-3125`). `docs/reference/cli-commands.md:1047` claims single-issue update enforces prechecks.
- **G5. Lease coverage is inconsistent.** CLI `dep add`/`dep rm` use the Batch variants, which check no lease (`dependency.rs:281`, `cli.rs` DepCommands comment), while the server's `add_dependency` (Single) does. Bulk update (`enforce_lease: false`, `mod.rs:967`) and label edits (`mod.rs:1025`) check none.
- **G6. Lock order constrains where the check can run.** `assign`, `unassign`, `gate add`, `gate evaluate` and `gate fail` are classified repository-first (`cli.rs:3291-3330`, asserted by the test at `cli.rs:3780-3830`). `main.rs:1929-1960` retains the startup session for those commands. `CoordinationOrderGuard::enter` refuses while a repository order guard is held (`storage/guard_order.rs:52-62`). Moving their lease check under `claims_mutation_guard` therefore requires reclassifying them as claims-first.

### 2. Prior-art sweep

- Charter D-20 (`dev/vision/9db27a3a-charter.md:384-397`): "A preview runs the same mutation session as its operation, read-only." D-17 (`:302-`): one redo record per mutation. `@/inv/derived-state-coherence` requires coupled materializations from one read, published through one synced redo record.
- No dedicated read-only session type exists. The established read-only pattern is `with_mutation_session` returning `SessionStep::Done` (`mod.rs:430-470`), used by validation capture (`validate.rs:155-165`), profile plan (`profile.rs:1520`), archive preview (`archive.rs:874`) and documents. It holds the repository write lock during capture.
- A precedent for running the apply derivation over many issues in one session already exists: `confirm_bulk_noop_candidates` (`bulk_update.rs:349-425`) calls `derive_field_update` for every candidate against one capture, then applies an empty plan to replay the conflict check.
- `docs/concepts/core-model.md:232-238,807-809` describe the cascade as happening "in the same operation" via `check_auto_transitions`. That is stale: it is a separate redo record per dependent, and false for reject (G1). `docs/reference/cli-commands.md:1042-1048` says bulk applies "no auto-transitions" (stale since 05d1b34e), says it is "atomic per-issue", and makes the precheck claim (G4).
- `dev/architecture/repository-state-materialization.md` has no preview or readiness section. The `dev/archive/f1dc843a-redo-mutations/active/investigation.md:1092` inventory lists `validate --fix` as four transaction sets, including the cascades. 9876a23a DEC-14 (fix-in-one-mutation) folds those into one plan.
- 9876a23a DEC-04(2) steward ruling: `fix-in-one-mutation` (1f24e3e9) applies `Issue::derive_readiness_correction` to the projected set until no correction remains. **b697a3f7's readiness-derivation task depends on 1f24e3e9** and later converges its fixpoint loop.
- The existing property test `tests/fast_issue/readiness_coherence_tests.rs:213-271` covers dependency edits only, in the Demote direction (`ready_but_blocked`).

### 3. Consumer sweep

Counts are occurrences per file.

| API being changed | Consumers |
|---|---|
| `check_auto_transitions` | `commands/issue.rs` 4 (def `:860`, calls `:540,:853`, test `:1071` plus test name), `commands/bulk_update.rs` 2 (`:295`, doc `:1533`), `docs/concepts/core-model.md` 2 (`:237,:808`) |
| `auto_transition_to_ready` / `_backlog` / `_done`, `publish_captured_auto_transition`, `CapturedLifecycleMutation::{AutoReady,AutoBacklog,AutoDone}` | `issue.rs` 3 defs; `validate.rs:2117-2118`; `gate.rs:878`; `gate_check.rs:1579`; `mod.rs` def `:~2745` plus the variants `:~855-870` and arms `:3064-3100` |
| `should_auto_transition_to_ready` / `derive_readiness_correction` | `domain/types.rs` (def `:687,:700` plus 8 tests); `commands/issue.rs:103`; `dependency.rs:605,:964`; `template.rs:1281,:1342`; `validate.rs:2099-2105`; `validation/repository.rs:513`; `mod.rs:3064-3090` |
| `require_active_lease` / `check_active_lease` | `issue.rs` 2, `gate.rs` 4, `mod.rs` def plus 3 tests (`:4305,:4357,:4380`). Named in df129d9e's executor-kernel list (`dev/active/df129d9e-rust-code-quality/breakdown.json`, `executor-facade-issue-batches`) and rewritten by f8a6b298 c3e3e2ff |
| `captured_lease_warnings` | `mod.rs` def plus 3 calls (`:2630`-area field update, lifecycle preflight and in-session), `issue.rs:630`, `gate.rs:1306`, `dependency.rs:305,:405` |
| `template_lease_warnings` | `template.rs:193,:277` |
| "No active lease" text | `mod.rs:803,:3411`, `template.rs:306`; tests `issue.rs:2176`, `gate_check.rs:3859`, `mod.rs:4360`, `crates/jit/tests/cli_gate/gate_cli_integration_test.rs` 2. Not in docs, MCP or web |
| `coordinates_claims_first` | `main.rs:1929`; `cli.rs` 10 impls (`:3072-3390`), `CLAIM_FIRST_LEAF_COMMANDS` and the inventory test (`cli.rs:3700-3835`) |
| `preview_bulk_update`, `BulkUpdatePreview`, `compute_changes`, `projected_after_update`, `validate_update_preview`, `transition_blockers` | `bulk_update.rs` only (preview has no caller; `BulkUpdatePreview` is used by `test_preview_compute_summary` `:735`). `mod.rs:73` re-export, `transition_blockers` def `mod.rs:3256` |
| `apply_bulk_update` | `main.rs:2971`; `tests/fast_issue/transition_graph_enforcement_tests.rs` 3; `tests/fast_rules/local_rule_enforcement_tests.rs` 4; `bulk_update.rs` tests. df129d9e f79d2510 moves it onto the issues facade |
| `CapturedFieldUpdate::bulk` / the `bulk` flag | `mod.rs:949,:974-977,:1512-1516`; `bulk_update.rs` `confirm_bulk_noop_candidates` and `publish_captured_bulk_update` |
| `CapturedIssueMutation::{Assign,Unassign,AddGates,RemoveGates}` | `mod.rs` enum `:~475`, `issue_local_mutations`, `apply_captured_issue_mutation`; `issue.rs:718,:759`; `gate.rs:723,:768` |
| Creation stamping (`MutationIntent::CreateIssue[Batch]`) | `repository_state/mutation.rs:747-848`; producers `mod.rs` `publish_issue_creation`, `batch_create.rs`, `template.rs` |
| `ClaimCoordinator::new` in production | `claim.rs` 8, `mod.rs:767`. The rest are tests (`claim.rs` 11, `claim_coordinator.rs` 6, `mod.rs:4394`) |
| `load_issue_or_not_found` | `storage/mod.rs:216` (trait default), `json.rs:1216`, `memory.rs:828`, `document.rs` 2, `issue.rs` 1, `tests/fast_docs_templates/template_apply_atomicity_tests.rs` 2 (wrapper) |

Server: `crates/server/src` calls only `add_dependency` (3) among these APIs. mcp-server and web have no consumer: the MCP bridge is generated from the CLI schema, and `issue update` has no `--dry-run`.

### 4. Primitive verification

**One redo record per apply.** `finalize` builds one `MaterializationPlan`. `session.apply` writes one record (`RedoRecord::new` in `repository_state_store.rs:1028`). Corrections added as `MutationIntent`s to the same `finalize` call satisfy REQ-01's "own redo record" with no new mechanism.

**Readiness fixpoint.** A correction moves an issue only between Backlog and Ready. Neither state is terminal, so a correction never changes any other issue's blockedness (`Issue::is_blocked` reads only `is_dependency_met`, `domain/types.rs:~641`). One pass over the projected set is therefore complete. 9876a23a's "until none remains" loop terminates after one productive pass.

**Field divergences (REQ-03).**

| Change | Dedicated or single `issue update` | Bulk |
|---|---|---|
| assign | `issue_claimed` at phase 1 (`mod.rs:~1612-1620`) | `issue_claimed` at phase 4 plus `issue_updated{source:"bulk-update", fields:[...,"assignee"]}` (`mod.rs:1503-1520`) |
| unassign | `issue_updated{source:"issue-unassign", fields:["assignee"]}` | `issue_updated{source:"bulk-update", fields:["assignee"]}` |
| gate add | `gate_added` per key, and **inserts a `Pending` `gates_status` entry** (`mod.rs:~1650-1660`) | `gates_required` push only, no status entry, `issue_updated{fields:["gates"]}` (`mod.rs:~1335-1345`). The stored state differs |
| gate remove | `gate_removed` per key | `issue_updated{fields:["gates"]}` |
| rule validation | Dedicated commands capture only the issue, events and gates (`publish_captured_issue_mutation` `mod.rs:2296-2375`) and run no local or graph rule | `derive_write_validation` runs |
| state | Lifecycle path: prechecks, divert, lease (`mod.rs:2767-3240`) | Field path: divert, no prechecks (G4) |

**Lease check (REQ-04).** `captured_lease_warnings` with a guard taken before `open_mutation_session` and held across it is the in-session form. The guard's snapshot comes from `lock_claims_for_repository_mutation` (`claim_coordinator.rs:410`). That snapshot is the "captured claims evidence": claims are control-plane state under `.git/jit/` and have no `VirtualPath` (2195c98f DEC-14 per f8a6b298 `lease-record-layout`). With git absent the guard is `None` (`mod.rs:760-762`), so warn mode warns and strict mode refuses. That matches D-4, and it differs from `require_active_lease`, which fails on `require_worktree_paths`/`ClaimsIndex::load` instead.

**Bulk read-only session (REQ-05).** `QueryFilter::filter_issues` is pure (`query_engine/mod.rs:65-71`), so matching can run over captured issues. Apply currently matches on an unlocked `list_issues` (`bulk_update.rs:241-242`). Matched issues are applied sequentially, each in its own session. Issue k therefore sees issues 1..k-1 already updated: B depends on A and both are matched with `--state done` gives A Done, then B Done. A preview over the unprojected image would report B as blocked. **Equality requires folding in apply order over a projection that advances with each success, including its readiness corrections.**

**Diverted Done.** `derive_field_update` publishes Gated and then returns `error_after_apply` (`mod.rs:~1395-1405`, published at the end of `publish_captured_field_update`), so the issue changes **and** lands in `errors` (`bulk_update.rs:286-288`). The equality relation must define this outcome (§5).

### 5. Design options

#### REQ-01/02: one readiness derivation

**Domain (pure, `domain/readiness.rs` or `domain/queries.rs`):**

```rust
pub fn readiness_corrections(
    base: &[Issue],
    projected: &[Issue],
    scope: ReadinessScope,
) -> Vec<(IssueId, ReadinessCorrection)>
```

It runs over `projected` and calls `Issue::derive_readiness_correction` once per candidate. `ReadinessScope` has two values:

- `Affected`: issues whose dependency set changed or which are new, plus the direct dependents of issues whose effective terminality changed between `base` and `projected` (including deleted issues).
- `All`: for `validate --fix`.

**Commands (one helper beside `derive_state_transition`):** `readiness_correction_intents(base, projected, evidence) -> (Vec<MutationIntent>, Option<TransitionBlockedError>)`. It applies Promote and Demote and is called by every plan that changes a state or a dependency: delete, dep add/remove/reduce, template edges, field update (single and bulk), lifecycle State/AutoDone, and `validate --fix`. Then `check_auto_transitions`, `auto_transition_to_ready`/`_backlog` and the `AutoReady`/`AutoBacklog` variants are deleted. G1 and G2 close as a consequence.

- Option A (recommended): Promote goes through `derive_state_transition`, so graph rules apply, as three of the four sites do today. Demote is direct, because the graph forbids Ready and no rule can keep it. The template promote changes behaviour (G3).
- Option B: both directions are direct, with no graph rules on derived readiness. This is simpler, but it drops today's `transition_blocked` events for promotions.

**Scope:**

- Option A (recommended): `Affected`. It is deterministic, has no side effect on unrelated drift, and is O(N) over the captured issues the base already holds.
- Option B: `All` everywhere. It is simpler, but a `dep add` on X would silently repair unrelated drift on Y.

**Auto Done:**

- Minimum: the AutoDone lifecycle plan carries its own corrections, which satisfies REQ-02.
- Fuller: manual `pass_gate` derives AutoDone inside its `SetManualGateStatus` plan. That needs a full base capture instead of the narrow one at `mod.rs:2296-2375`.
- `run_postchecks` keeps a separate AutoDone, because checkers run outside the lock.

Recommend the minimum, with the manual-pass fold as an optional follow-on.

**Initial readiness at creation** (REQ-20 stamping): drafts are born Backlog, and the same derivation promotes them. That removes the four initial-Ready writers (C8). Local rules must see the final state; `issue.rs:200-215` comments on this. So the correction runs before `derive_write_validation`.

#### REQ-03: one derivation per field

- Option A (recommended): the bulk path stops folding. `CapturedFieldUpdate::bulk` composes `issue_local_mutations` like single update does, and the `bulk` flag and the `"bulk-update"` source are removed. That gives one event vocabulary: `gate_added`/`gate_removed` per key, `issue_claimed`, `issue_updated{fields:["assignee"]}` for unassign. The dedicated commands publish through `publish_captured_field_update`, which gives them full capture, rule validation and in-session lease checking. `publish_captured_issue_mutation` keeps only `SetManualGateStatus`.
- Option B: keep two publishers and share only `apply_captured_issue_mutation`. This is less churn, but the dedicated commands keep skipping rules, so "same issue state" fails whenever a rule rejects the change.

The state field also needs a decision (G4). Either `issue update --state` routes through the lifecycle `State` request (prechecks, divert), or the precheck step moves into the shared derivation. Recommend the latter: one state derivation, with prechecks as a captured precondition for any Ready→InProgress transition.

**Test:** one property test over (initial issue, field change). It asserts that the dedicated command, single `issue update` and bulk `--filter id:…` produce equal stored issues and equal event bodies, ignoring id and timestamp.

#### REQ-04: one in-session lease check

- Define the message once as a typed error `LeaseRequiredError { issue_id }` in `errors.rs`. Strict mode returns it; warn mode renders its `Display`. This replaces `anyhow!` (`mod.rs:807`) and gives the classifier a code.
- Delete `require_active_lease` and `check_active_lease`.
- assign and unassign arrive through REQ-03 Option A, with `enforce_lease`. gate add and remove do too. pass and fail need a guard in their own session (`SetManualGateStatus`), and so does auto pass through `check_gate_with`.
- Template takes the guard before its second session and checks inside it, folding its preflight session.
- Reclassify `issue assign`, `issue unassign`, `gate add`, `gate remove`, `gate evaluate`, `gate evaluate-many` and `gate fail` as claims-first in `cli.rs`, and update the inventory test (G6).

**The lease-checked set** must be decided:

- Option A (recommended): every user-requested mutation of an existing issue's record checks the issues it targets. That includes bulk (per matched issue) and CLI `dep add`/`dep rm` (closing G5). System-derived writes are excluded: readiness corrections of dependents, `validate --fix`, archive, recovery. Each derivation returns its lease targets, as template does today (`DerivedTemplateApply::lease_targets`).
- Option B: keep today's set (the background's sites plus template) and only move it into sessions. This is less behaviour change, but it violates the literal "every issue mutation".

**Division of work:**

- **bedf0962** (Done): mode resolution. No edge.
- **f8a6b298 c3e3e2ff `lease-reader-migration`**: moves `require_active_lease`/`check_active_lease` onto `read_active_lease` and `Lease::is_active`, and makes template warnings come from `captured_lease_warnings` (its REQ-02).
- **f8a6b298 15009590 `lease-mutation-guard-through-lock`**: replaces the guard's internals (`ClaimsMutationGuard` → `LeaseLock::load`) behind `claims_mutation_guard`.
- **This epic**: where the check runs (in-session, every listed mutation), the reclassification, deleting the pre-session check, and the message defined once.

**Order:** our REQ-04 task depends on c3e3e2ff, so the template copy is already gone and we delete rather than migrate. That edge delays only us. With 15009590 there is no edge: the call-site interface `claims_mutation_guard(paths) -> Option<guard>` plus `captured_lease_warnings(mode, targets, issues, guard)` is unchanged by it, and the only overlap is textual, in `mod.rs:755-813`.

The reverse edge (c3e3e2ff after ours) would spare c3e3e2ff from migrating a function we delete. It delays f8a6b298, so it is flagged for the steward, not proposed.

#### REQ-05: bulk preview

- **Option A: delete the preview.** It has no consumer (C7f), and deleting it is the D-18 answer. Delete `preview_bulk_update`, `BulkUpdatePreview`, `compute_changes`, `projected_after_update`, `validate_update_preview` and `transition_blockers` (about 200 lines). REQ-05 as written would then be unmeetable, so the criterion must be rewritten. Needs a decision.
- **Option B (recommended if a preview is wanted): one fold, two views.** A pure `derive_bulk_update(image, captured_issues, filter, ops, force, context) -> BulkDerivation` matches over the captured issues. It then folds in match order: for each issue, `derive_field_update` against the current projection, plus readiness corrections, plus lease targets. Each success advances the projection, and each outcome is recorded as `Changed`, `Unchanged`, `Rejected { changed: false }`, or `Diverted { changed: true }` (the Gated case).
  - **B1:** apply keeps one session per issue. Preview equals apply only if no other writer intervenes and the fold mirrors per-issue re-capture. That is two compositions of one derivation.
  - **B2 (recommended):** apply is one session. The plan is the union of every non-rejected issue's intents, giving one redo record. Preview runs the same function and returns `SessionStep::Done`. Equality holds by construction (D-20). `confirm_bulk_noop_candidates` and its `C*J+1` session budget are deleted.
  - The cost of B2: batch-atomic publication replaces per-issue publication (the docs line "Atomic per-issue" changes), and a conflict retries the whole batch.
  - Expose the preview as `jit issue update --filter … --dry-run`, matching `--dry-run` on the other mutating commands (`cli.rs:400,764`).

**Property test (harness layer, `InMemoryStorage`, in `bulk_update.rs` tests):**

- **Generators:**
  - 3 to 8 issues with random states (all seven, including Archived with an origin).
  - An acyclic dependency graph (edges only to lower indices).
  - Labels from a small declared namespace.
  - Gates from a registry with one manual and one auto gate, with random `gates_status`.
  - A config with one enforcing local rule and one enforcing graph rule.
  - A filter from a small grammar (`state:`, `label:`, `priority:`, AND/OR).
  - `UpdateOperations` drawn from: any state, adds and removes over the vocabulary plus one undeclared label or gate, assignee or unassign, priority.
- **Assertions:**
  - The preview changes no storage byte (the image before and after compares equal).
  - Then apply runs on the same storage. Preview's changed ids equal the ids whose stored record changed. Preview's failing ids equal `result.errors` ids. Preview's unchanged ids equal `result.skipped`.
- **Fixtures:** seed through `commands/test_helpers.rs` (`seed_issue`, `seed_gate_registry`, `seed_repo_file`). The 9876a23a `image-test-builder` (6100b960) is optional, so no edge is needed.

### 6. Architecture fit and invariant check

- **Domain purity.** `readiness_corrections` is pure over `&[Issue]` and belongs in `domain/`. Graph-rule gating stays in `commands/`, where `derive_state_transition` already sits.
- **`@/inv/derived-state-coherence`.** Today it is violated by post-commit corrections (C2, C3) and by missing corrections (G1, G2). The design restores it.
- **`@/inv/event-log`.** G1 and G2 leave missing `issue_state_changed` events. The REQ-03 divergences make the log depend on the invocation path.
- **`@/inv/convention-convergence`.** It is violated by three lease checks, two state derivations (G4), two promotion semantics (G3) and the preview pipeline.
- **Lock order.** REQ-04 requires LeaseLock (or today's claims lock) first, then the repository lock. The CLI classification must follow (G6). Taking the claims guard inside a retained session fails at runtime (`guard_order.rs:52-62`).
- **`@/inv/orthogonal-tests`.**
  - The readiness matrix is owned once by a domain unit or proptest on `readiness_corrections`.
  - Each consumer gets one wiring test: its plan contains the correction in the same record, observed by counting `MutationPoint::SessionOpen` or redo applies, as `issue.rs`'s `SessionOpenCounter` does.
  - The existing CLI-free proptest in `readiness_coherence_tests.rs` extends to both directions and to state, delete and bulk edits.
- **D-19.** The scoped correction reads only captured current issues, never history.

### 7. Sequencing

Each step leaves the workspace building, clippy-clean and tested. Every step follows 2195c98f, the epic's current dependency; the specific keys are listed.

| # | Step | Files | Needs (external) | Edge notes |
|---|---|---|---|---|
| 1 | Pure `readiness_corrections` plus `ReadinessScope`, with unit tests and a proptest (both directions, scope minimality) | `domain/types.rs` or a new `domain/readiness.rs`, `domain/mod.rs` | 2195c98f `issue-id-flip` 7abcc22d (IssueId in the signature) | — |
| 2 | Command helper `readiness_correction_intents`. Rewire delete, dep add/remove/reduce and template edges onto it (G3 decision) | `commands/mod.rs`, `issue.rs`, `dependency.rs`, `template.rs` | 2195c98f `event-envelope` d72bf093 and `stage-graph-command-issue-ids` 804a8605 | — |
| 3 | Corrections in the field-update, lifecycle (State/AutoDone/Claim/Release) and bulk per-issue plans. Delete `check_auto_transitions`, `auto_transition_to_ready`/`_backlog`, `AutoReady`/`AutoBacklog`. Add G1/G2 regression tests and session-count tests. Fix docs `core-model.md:232-238,807-809` and `cli-commands.md:1042-1044` | `mod.rs`, `issue.rs`, `bulk_update.rs`, `gate.rs`, `gate_check.rs`, two docs pages, `tests/fast_issue/readiness_coherence_tests.rs` | 2195c98f `stage-lifecycle-command-issue-ids` 75951558, `stage-dispatch-issue-ids` cd8adfc1 | Touches the same methods as df129d9e 27f0a575/2e0f276b; flag |
| 4 | `validate --fix` readiness through the helper with `ReadinessScope::All`, replacing 9876a23a's fixpoint loop | `validate.rs` | 9876a23a **1f24e3e9** `fix-in-one-mutation` | Steward ruling: ours → 1f24e3e9 |
| 5 | REQ-03: bulk composes `issue_local_mutations`; the `bulk` flag and `"bulk-update"` source go; dedicated assign/unassign/gate add/remove publish through the field update; property test over the three paths. The state-field unification (G4) is a separate step, 5b | `mod.rs`, `issue.rs`, `gate.rs`, `bulk_update.rs` | 2195c98f `gate-key-issue-fields` 417be079, `event-tag-vocabulary` e98ae5a4, `stage-gate-command-issue-ids` 8e82fa49 | Steps 3 and 5 both edit `derive_field_update`; land 3 first |
| 6 | REQ-04: `LeaseRequiredError`; delete `require_active_lease`/`check_active_lease`; in-session checks for gate pass/fail/auto-pass and template; reclassify `cli.rs`; decide the lease set (G5) | `errors.rs`, `mod.rs`, `gate.rs`, `gate_check.rs`, `template.rs`, `dependency.rs`, `cli.rs`, `main.rs` (none expected), tests that assert the message | 2195c98f `agent-identity-assignee` a50e9be9, `lease-issue-identity` 98404e0c (transitively via c3e3e2ff) | ours → f8a6b298 **c3e3e2ff**; no edge with 15009590 |
| 7 | REQ-05: `derive_bulk_update` fold; one-session apply (B2) and preview (`--dry-run`), or deletion (Option A); delete the preview helpers, `transition_blockers` and `confirm_bulk_noop_candidates`; proptest; docs for bulk semantics | `bulk_update.rs`, `mod.rs`, `cli.rs`, `main.rs`, `docs/reference/cli-commands.md`, the two integration test files | Steps 3, 5, 6 | df129d9e f79d2510 moves `apply_bulk_update` onto a facade; flag |
| 8 | REQ-20 creation stamping: one `stamp_created_issue(draft, id, now)` for both finalizer passes, one created-event constructor, initial readiness via step 1 | `repository_state/mutation.rs`, `issue.rs`, `template.rs`, `batch_create.rs` | 2195c98f `issue-draft-identity` 5fac48b8, `issue-record-path-commands` 855d027d | ours → 842b0ac6 **2492ab29** (`remove-lifecycle-backfill`, same finalizer region `mutation.rs:379,~941-980`) and → 9876a23a **5d5a1794** (`create-findings-in-session`, `publish_issue_creation`) |
| 9 | REQ-20 claims-coordinator helper: one `open_claim_coordinator` for the 8 `claim.rs` sites; decide whether `claims_mutation_guard` shares its identity/agent policy | `commands/claim.rs`, `mod.rs` | 2195c98f `lease-worktree-ids` 8cfbf19d, `agent-identity-assignee` a50e9be9 | ours → f4d9502a **f6eaba42** (`claims-head-lookup`, same 8 sites) and → f8a6b298 **614a6f97** (`lease-issue-addressed-ops`, the final lease-API entry, which rewrites these ops). df129d9e 083c9a91 (claims facade) moves the same functions; flag |
| 10 | REQ-20 issue loading: one lookup (`load_issue` maps `load_issue_or_not_found`'s NotFound to `IssueNotFoundError`) in both backends | `storage/json.rs`, `storage/memory.rs`, `storage/mod.rs` | 2195c98f `stage-storage-issue-ids` (look up by title) | ours → f4d9502a **d7cce445** (`head-tier-removal` deletes the git tier, so the git-error half becomes already-done) |

Seams with other in-flight work:

- **df129d9e** (Backlog): 27f0a575 (issues facade), fd2073b7 (gates facade), f79d2510 (bulk/label facade), 2e0f276b (mutations return the issue), 083c9a91 (claims facade). They move or re-sign the same methods as steps 3, 5, 6, 7 and 9. There is no correctness edge; whichever epic lands second rebases. Flag for the steward.
- **459bb995** (842b0ac6, config defaults, executing): touches `cached_config` users, including `require_active_lease`. No edge, because step 6 deletes that caller.

## Part: Archive planning, destinations and snapshot export

## b697a3f7 investigation: archive and snapshot

Scope: REQ-06, REQ-07, REQ-08, REQ-18 (artifact classifier and domain imports), REQ-19, and three REQ-20 items: archive destination with no legacy short-id fallback, one scope type for snapshot and link check, one safe-relative-path validator. Code read at HEAD `fb08b419b`. All paths are under `crates/jit/` unless stated.

### 1. Claim classification

| # | Claim / REQ | Class | Current evidence |
|---|---|---|---|
| C1 | `plan_archive_target` repeats the execute planner `capture_archive_plan` step for step | valid-and-open | Preview: `src/commands/archive.rs:184-309` (generic `impl<S: IssueStore>`, `:155`). Execute: `capture_archive_plan` `:511-834` (`impl CommandExecutor<JsonFileStorage>`, `:510`). The two run the same steps in the same order: config, hierarchy, issues, target resolution, policy, legacy/preferred destination, inventory remap, pinned evidence, inventory, archive events, discovery, blockers, classification facts, residue identities, classify. The preview reads through `IssueStore` and disk. The execute planner reads through the captured image. |
| C2 | About 800 lines of storage-only helpers in `storage/artifact_planning.rs` and `storage/artifact_discovery.rs` | valid-and-open | 709 + 86 = 795 lines. Production code: `resolve_container_destination` `artifact_planning.rs:28-81`, `collect_artifact_classification_facts` `:83-145`, `collect_citation_scan_evidence` `:163-198`, `inspect_artifact_evidence*` `:246-443`, `discover_archive_artifacts` `artifact_discovery.rs:41-60`, `collect_closure_evidence` `:62-86`. Exports: `storage/mod.rs:11-12,49-51`. **Not all of it is preview-only.** Execute also calls `collect_citation_scan_evidence` (`commands/archive.rs:813`), so that function and its no-follow walker must be relocated, not deleted. `JsonFileStorage::inspect_artifact_location` (`storage/json.rs:702-726`) has no production caller. Only its own test calls it (`json.rs:292-334`). |
| C3 | `archive_candidates` plans each candidate from scratch, re-reading configuration, all issues and the event log each time | valid-and-open | `archive.rs:158-170` maps each id to `plan_archive_target`. Each call does `config_manager.load` (`:185`), `get_hierarchy_config` (`:186`), `list_issues` (`:187`), `resolve_issue_id` (`:194`) and `read_artifact_archive_events` (`:254`), which parses the whole log. Measured here: `jit archive candidates --json` takes **87 s** for 190 candidates over 1589 issues and 18482 event lines, about 0.46 s per candidate. |
| C4 | Container-destination evidence is acquired by hand twice | valid-and-open | `storage/artifact_planning.rs:28-81` (preview) and `commands/archive.rs:542-651` (execute, inline). |
| C5 | The `.jit-container` marker name is formatted at more than five sites | valid-and-open | 8 production sites: `domain/artifact_classifier.rs:451,592`; `storage/artifact_planning.rs:53,123`; `repository_state/archive.rs:123`; `commands/archive.rs:607,641,771`. Test sites: `commands/archive.rs:1569,1574,2245,2258`; `artifact_classifier.rs` tests; `repository_state/archive.rs:800,807`; `tests/cli_repo_workflow/archive_preview_cli_tests.rs` (3). The user docs name the marker at `docs/reference/cli-commands.md:337,373`. |
| C6 | Embedded-owner discovery walks reachability once per document reference and resolves every reference again on each walk | valid-and-open | `discover_embedded_owners` `domain/artifact_discovery.rs:501-534` calls `reachable_parsed_paths` (`:536-554`) once per (issue, unpinned document). Two issues that link the same document walk it twice. `resolve_reference` runs at three sites over the same parsed references: closure expansion `:261`, discovery `:413`, and every reachability walk `:548`. `ParsedArtifact` stores raw `references: Vec<String>` (`:31-35`). |
| C7 | The classifier imports `repository_state::ProducerError` and matches it with a wildcard arm | valid-and-open | `artifact_classifier.rs:783-790` (`_ => None` at `:789`). `validate_proposed_layout` / `validate_proposed_artifact_layout` return `repository_state::ProducerError` (`:835-910`, constructors `:865,897`). Test sites: `:2785,3021`. The variants are declared at `repository_state/mod.rs:327-340`. The consumer is `repository_state/archive.rs:29` (`ArchiveExecutionError::Producer`), with a test at `:1019`. |
| C8 | Two missing-evidence helpers in the classifier return different error types | already-done (residue open) | 45b442f3 (842b0ac6 `collapse-container-destination-states`, commit `edfa85f6e`) removed the anyhow twin. Only `required_resolution_evidence` remains (`:487-496`). Residue: `classify_entry` builds `ArtifactEvidenceError::IncompleteEvidence` inline over the `locations` map (`:950-954`), outside the helper. |
| C9 | Domain code uses `anyhow` | valid-and-open, narrower than stated | The only uses are `domain/types.rs:14,156,181,216`: the `FromStr` of `State`, `Priority` and `ContentFormat`. 2195c98f re-declares all three through the vocabulary macro: 2ac50581 `lifecycle-state-vocabulary`, 182350ce `priority-vocabulary` and 42bcaa6e `content-format-vocabulary`, on 3f026bb1 `wire-vocabulary-macro`. Its manifest does not state the macro's `FromStr` error type, so it cannot be verified that this closes the gap. Other untyped domain errors are `Result<_, String>` at `domain/key_reference.rs:30` and `domain/repository_inputs.rs:266,283`. |
| C10 | (not in background) Domain imports from `repository_state` beyond the classifier | valid-and-open | `domain/types.rs:13` imports `repository_state::RootRelativePath`; `:9-12` gives the reason. It is used at `types.rs:1157-1171` (`Directory(RootRelativePath)`). Under REQ-18 this import must also go. No other `domain/` or `graph/` file imports `repository_state`, `storage` or `commands`. |
| C11 | Snapshot export re-serializes issues outside the canonical serializer | valid-and-open | `commands/snapshot.rs:453` writes `serde_json::to_string_pretty(issue)` of issues from `storage.list_issues`/`load_issue` (`:99-118`). The canonical serializer is `repository_state::serialize_issue` (`repository_state/mutation.rs:554`), which goes through `canonicalize_json` (`:580-600`) and sorts object keys. Exported bytes can therefore differ from the stored bytes in key order, and any non-canonical stored record is rewritten. |
| C12 | Snapshot export copies only `config.toml` and `gates.toml` | valid-and-open | `snapshot.rs:462-473` reads those two through `read_repository_file_nofollow` on the data root, not from an image. It omits `rules.toml`, `templates.toml`, `invariants.toml` and `config/gate-presets/`. It enumerates issues through `IssueStore`. No captured image exists in export assembly; the only session is `publish_repository_export` (`commands/mod.rs:2241-2260`), which captures the destination only. |
| C13 | There are two safe-relative-path validators (`repository_inputs.rs:454` and `RootRelativePath::parse`) | **invalid-as-stated: there are three** | (a) `domain::repository_inputs::is_safe_relative_path` `:473-490`, with consumers `repository_inputs.rs:102`, `profile/package.rs:872` and `profile/package_archive.rs:643`. (b) `RootRelativePath::parse` `repository_state/path.rs:82-104`, with about 50 call sites in 19 files (§3). (c) `storage::validate_repo_relative_path` `storage/path_errors.rs:57-74`. Rule (c) is laxer: it accepts `.`, empty segments, `:`, `\` and control characters. Consumers of (c): `storage/json.rs:710,1444,1486`, `storage/memory.rs:250`, `storage/git_revision.rs:694`, `commands/document.rs:410`, `commands/archive.rs:416` (paired with (b) at `:417`), `storage/artifact_planning.rs` (6), and the rustdoc at `validation/graph.rs:105`. (a) and (b) agree except on empty input (root vs reject). |
| C14 | `SnapshotScope` duplicates `DocumentScope` | valid-and-open | `snapshot.rs:141-187` (`All`, `Issue(String)`, `Label{namespace,value}`; `parse` returns anyhow) versus `document/scope.rs:23-50` (`All`, `Issue(String)`; typed `DocumentScopeParseError`). Issue selection by scope is also written twice: `commands/snapshot.rs:99-118` and `commands/document.rs:1113-1120`. |
| C15 | REQ-20: archive destination resolution has a legacy short-id fallback | valid-and-open | Domain: `artifact_classifier.rs:414-485`, fallback at `:474-482`. Acquisition: `artifact_planning.rs:64-70` and `commands/archive.rs:540,596-597,621-628`. The policy is documented at `docs/reference/cli-commands.md:380-385`. Tests: `artifact_classifier.rs:2380-2560`, including `test_resolve_container_destination_adopts_a_preexisting_identifier_only_directory_without_relocating_it` at `:2440`. Data check: this repository has 3 bare short-id directories under `dev/archive/` (`7d3a3a47`, `a4e3cfb0`, `a9b5dd08`) and each carries a matching `.jit-container`, so the marker branch wins first and removing the fallback needs no data migration here. |
| R6 | REQ-06 | valid-and-open | C1–C3. |
| R7 | REQ-07 | valid-and-open | C4, C5. |
| R8 | REQ-08 | valid-and-open | C6. |
| R18 | REQ-18 (classifier part) | valid-and-open | C7, C8 residue, C9, C10. |
| R19 | REQ-19 | valid-and-open | C11, C12. |

Counts: already-done 1 (C8, with an open residue); valid-and-open 19 (C1–C7, C9–C12, C14, C15, R6–R19); invalid-as-stated 1 (C13).

### 2. Prior-art sweep

- `@/charter/D-20` (`dev/vision/9db27a3a-charter.md:384-396`): "A preview runs the same mutation session as its operation, read-only." This favours reusing the mutation session over a new read-session type.
- Precedent for a read-only use of the mutation session: `capture_validation_image_with` (`src/commands/validate.rs:146-165`) opens `with_mutation_session` and returns `SessionStep::Done`. Validation is classified as no-recovery-dispatch (`src/cli.rs:3034-3053`) but still opens a session. The archive preview can follow the same pattern without changing `ArchiveCommands::requires_recovery_dispatch` (`src/cli.rs:3211-3217`).
- `dev/archive/f1dc843a-redo-mutations/active/investigation.md:1098` records the preview as having "no session, no repo lock". A session-based preview takes the repository write lock and applies a leftover redo record at open, as validation already does.
- Commit `c53e50205` (65059cd2) deliberately moved the citation scan out of the guarded capture. The comment at `commands/archive.rs:808-812` says citations are advisory and kept out of the capture budget. Keep that decision: preview and execute share the one scan function, so their plans still agree.
- f8a6b298 `DEC-17` (`dev/active/f8a6b298-history-independent-reads/plan.md:333`) sets per-target `archive-evidence/<target-key>.json`. Its `DEC-03` (`:319`) records that "b697a3f7 REQ-06 depends on `archive-preview-reads-evidence`".
- 842b0ac6 `DEC-follow-ups` (`dev/active/842b0ac6-greenfield-cutover/plan.md:218`) passes the short-id destination fallback to this epic.
- 2195c98f plan `:74` keeps the path identity in `repository_state/path.rs`. Moving `RootRelativePath` into `domain` (§5.3) goes beyond that plan, and lands after it.
- `docs/reference/cli-commands.md:342-385` is the adopter description of destination resolution. `:3579-3606` covers snapshot export and does not enumerate the `.jit/` files it copies. `:2841-2846` covers `doc check-links --scope all|issue:<ID>`.

### 3. Consumer sweep

Archive planning APIs to delete or replace (REQ-06/07):

| API | Consumers (file: count) |
|---|---|
| `CommandExecutor::plan_archive_target` | `src/commands/archive.rs`: 3 (`:160`, `:174`, `:179`) |
| `archive_candidates` | `src/main.rs:5498`; `tests/fast_docs_templates/harness_demo.rs:510` (harness, `InMemoryStorage`, writes `config.toml` to disk at `:486-493`, which a memory-backend capture will not see); `tests/cli_repo_workflow/archive_preview_cli_tests.rs` |
| `preview_archive_document` / `preview_archive_container` | `src/main.rs:5533,5565`; `tests/fast_issue/short_hash_tests.rs:307-308` (harness); `archive_preview_cli_tests.rs` (5 references across both methods); `src/commands/archive.rs` tests (36 `fn test_` in the module) |
| `storage::discover_archive_artifacts` | `src/commands/archive.rs:255`; `tests/fast_docs_templates/artifact_discovery_tests.rs` (13 references, 22 tests, 844 lines). These tests own the discovery properties: format matrix, symlink no-follow, pinned roots, navigation targets. |
| `storage::collect_artifact_classification_facts` | `src/commands/archive.rs:288` |
| `storage::resolve_container_destination` | `src/commands/archive.rs:221` |
| `storage::collect_citation_scan_evidence` | `src/commands/archive.rs:813` (execute; stays); its own tests `artifact_planning.rs:630-709` (4) |
| `inspect_artifact_evidence` | `storage/artifact_planning.rs` (internal), `storage/artifact_discovery.rs:14,79`, `storage/json.rs:711` |
| `JsonFileStorage::inspect_artifact_location` | test only: `storage/json.rs:292-334` |
| `inventory_issues_pinned_evidence` + `GitRevisionResolver::new` in archive | `commands/archive.rs:90-105,190,248` (f4d9502a d9ba0029 rewrites these) |
| `IssueStore::read_artifact_archive_events` | trait default `storage/mod.rs:257-266`, file backend `storage/json.rs:1295-1330`, preview `commands/archive.rs:254`, 17 test sites in `commands/archive.rs:1058-2231` (f8a6b298 d0d73bef deletes all of these). f8a6b298 5e1ba005 adds a new `IssueStore` evidence-read method in both backends and two test wrappers (`tests/fast_docs_templates/template_apply_atomicity_tests.rs`, `tests/fast_rules/local_rule_enforcement_tests.rs`). Under REQ-06 that method loses its only caller (the preview) and must be deleted again. |
| `ArtifactListingScope`, `ArtifactEvidence` (domain) | domain classifier/discovery; `commands/archive.rs` image evidence; storage helpers. These are kept. |

Domain imports (REQ-18): `RootRelativePath::parse` consumers per file: `commands/archive.rs` 1, `commands/init.rs` 1, `commands/profile.rs` 8, `commands/snapshot.rs` 6, `profile/application.rs` 2, `repository_state/archive.rs` 1, `repository_state/export.rs` 6, `repository_state/image.rs` 1, `repository_state/initialize.rs` 2, `repository_state/materialize.rs` 1, `repository_state/mutation.rs` 2, `repository_state/package_tree.rs` 2, `repository_state/path.rs` 11, `repository_state/profile_apply.rs` 4, `storage/artifact_planning.rs` 2, `storage/json.rs` 1, `storage/memory.rs` 2, `storage/redo_record.rs` 1, `storage/repository_state_store.rs` 1. `crates/server`, `mcp-server`, `web`, `scripts`, `test-vectors`: none.

`ProposedLayout*` / `validate_proposed_layout`: `domain/artifact_classifier.rs` (production 5, tests 2), `repository_state/mod.rs:327-340`, `repository_state/archive.rs:29,1019`.

Scope types (REQ-20): `SnapshotScope`: `src/snapshot.rs` (22 incl. tests), `src/commands/snapshot.rs` 14, `src/main.rs:7753-7756`, rustdoc `src/validation/graph.rs:85`. `DocumentScope`/`DocumentScopeParseError`: `src/document/scope.rs` 20, `src/document/mod.rs:27`, `src/main.rs:5429-5434`, `src/commands/document.rs:1100-1114`, `src/output/failure.rs:315` (downcast for the exit class). CLI arguments are `src/cli.rs:386` (`scope: Option<String>`) and `:2027`. Docs: `docs/reference/cli-commands.md:2846` and the snapshot flag table `:3595`. mcp-server and web carry no scope literals.

Snapshot (REQ-19): `export_snapshot` is called by `src/main.rs:7774` and tests in `tests/cli_issue/snapshot_export_tests.rs`. That file asserts `.jit/issues` exists (`:80`), and a symlinked `.jit/config.toml` must fail the export without output (`:521-552`); a capture-based read must preserve that. Unit tests are at `src/commands/snapshot.rs:775-1080`.

Legacy destination fallback (REQ-20): domain `resolve_container_destination` is called at `commands/archive.rs:649` and wrapped at `storage/artifact_planning.rs:76`. Domain tests are at `artifact_classifier.rs:2380-2560`. Docs: `docs/reference/cli-commands.md:380-385`.

### 4. Primitive verification

| Property | Delivered today? | Evidence / what is needed |
|---|---|---|
| A read-only session over the execute planner | Yes, by precedent | `with_mutation_session` + `SessionStep::Done` (`commands/mod.rs:436-471`), as validation uses it. `capture_archive_plan` already takes `&mut dyn RepositoryMutationSession` and returns `(image, plan)` without applying. Needed: move preview, candidates and execute into `impl<S: IssueStore + RepositoryStateStore>`. `InMemoryStorage` implements the store (`storage/repository_state_store.rs:498`), so the harness keeps working. The memory capture reads only its in-memory state (`:1212-1263`) and reports pinned evidence as unavailable, so the harness candidates test must seed configuration into memory. |
| Preview plan == execute plan over the same image | Not yet | One function makes it hold by construction. The owning test should take one image, run the planner for preview and for execute, and compare the plans. `ArchiveExecutionResult` does not carry the plan, so the test needs to call the planner directly (`pub(crate)`). Citations are read outside the image by the same function in both paths (`archive.rs:813`), so they agree. |
| Candidates in one session | Not yet | Needed: split `capture_archive_plan` into a once-per-session base derivation (config, namespaces, hierarchy, captured issues, `archive.rs:516-525`) and a per-target planner over the growing image. Two risks. (1) Each `extend` deep-clones the image (`repository_state_store.rs:606,757`; REQ-13). Growing one image across 190 targets therefore costs on the order of targets × extends × image size. Land REQ-13 first, or measure. (2) The image budget: the archive base allows 2^16 listings (`commands/validate.rs:384-393`) and 512 MiB (`commands/mod.rs:265`) shared across all candidates. |
| Once-per-target archive evidence | After f8a6b298 | Today execute parses the whole log through `captured_archive_events` (`archive.rs:719`, `repository_state/archive.rs:31`), and preview parses it through `read_artifact_archive_events`. After 7fc08dff the image holds the per-target record, and the planner reads it once. |
| Pinned evidence batched | Execute: yes; preview: no | The execute planner asks the capture for pinned evidence (`archive.rs:670-700`), and the capture batches the requests (`storage/repository_state_store.rs:1130-1150`, `inspect_pinned_targets`). The preview runs three git processes per document (`archive.rs:90-105`). REQ-06 makes the preview use the batched path, which is what f4d9502a d9ba0029 REQ-01/02 asks for. |
| One destination-evidence request | No | After REQ-06 only the execute-side inline block remains (`archive.rs:542-651`). Extract it into one function. Without the legacy fallback, the function requests the archive root, its child listing, and child markers; the legacy path drops out. |
| Marker declared once | No | Add one domain constant for the marker name and one path constructor (`fn container_marker_path(root) -> String`), used by the domain (2 sites), the commands (2 after extraction) and `repository_state/archive.rs:123`. |
| References resolved once per plan | No | `parse_artifact` (`artifact_discovery.rs:612`) should store `(reference, ReferenceResolution)` pairs, since `resolve_reference` is a pure function of the parent path and reference (`:633`). Discovery and reachability then read the stored resolution. The closure parses each path once, but it re-enters across capture rounds (`archive.rs:720-737`); it already defers unparsed paths, so storing resolutions on `ParsedArtifact` covers all three sites. |
| Reachability once per distinct root | No | Collect the distinct normalized document roots first, then compute `reachable_parsed_paths` once per root into a `BTreeMap<root, BTreeSet<path>>`, then fan out per issue. |
| Domain purity | No | C7, C10. `ProducerError` needs a domain-owned layout-violation type. `RootRelativePath` needs a domain home. |
| Snapshot from one image | No | The building blocks exist: `captured_active_issues(&image)` (`commands/mod.rs:2062`) and image `File` entries holding the stored bytes. Needed: one list of declaration files. Today it is implicit in the base capture (`commands/validate.rs:369-377`: CONFIG, INVARIANTS, RULES, GATES, TEMPLATES) and has no layout-level name. A symlinked declaration is captured as `RepositoryEntry::Symlink` and must fail the export, preserving `snapshot_export_tests.rs:521-552`. |

### 5. Design options

#### 5.1 Read-only archive session (REQ-06)
- **A (recommended).** Reuse the mutation session and return `SessionStep::Done`. This follows the validation precedent and the D-20 wording, and adds no new mechanism (D-18). Cost: preview and candidates hold the repository write lock for the run and apply a leftover redo record at open. I sent this to b6-inv-session for alignment with REQ-05.
- **B.** A new `RepositoryReadSession` supertrait (capture and extend only) under shared locks. That is a second session mechanism: it cannot apply a leftover redo record without the write lock, so it would need a retry-or-escalate path. Rejected under D-18 unless REQ-05 needs it anyway.

#### 5.2 Candidates
- **A (recommended).** One session and one base derivation, with each candidate planned on the growing image. This is the literal REQ-06 shape and removes the per-candidate config, issue and log re-reads. Sequence it after REQ-13 (image sharing) so per-extend clones do not dominate.
- **B.** One session per candidate. This is simpler on budgets but contradicts "one read-only session".

#### 5.3 Domain types for paths and layout violations (REQ-18, REQ-20 safe paths)
- Layout violation: add `domain::artifact_classifier::ProposedLayoutViolation { TargetAbsent{target}, EdgeUnavailable{reference,parent,resolved,available} }`. The classifier matches it exhaustively. `repository_state` replaces its two `ProducerError` variants with one `#[from]` wrapper; `ArchiveExecutionError` keeps reporting it.
- Relative paths:
  - **A (recommended).** Move `RootRelativePath` into `domain` (a pure module). `repository_state::path` builds `VirtualPath` on it. Delete `is_safe_relative_path` (its callers use `RootRelativePath::parse` and reject `Root`). Delete `validate_repo_relative_path` (storage maps the parse error to `PathReadError::InvalidPath`). Behaviour change: storage path reads reject `.`, `//`, `:`, `\` and control characters, which they accept today (`storage/path_errors.rs:57-74`).
  - **B.** Keep the type in `repository_state` and copy the validator into domain. That keeps two forms and the import at `types.rs:13`, so it fails REQ-18 and REQ-20.

#### 5.4 Snapshot source (REQ-19)
- **A (recommended).** Capture issues (the `ISSUES` listing plus records) and the declaration set in one read-only session, then write each entry's stored bytes. Publishing keeps its own session (`publish_repository_export`).
- **B.** Read and publish in one session when the destination is inside the repository. This saves a session but couples two export paths, and the external destination branch still needs A.
- The declaration set should be one layout constant (e.g. `VirtualPath::DECLARATIONS`) that both the base capture (`validate.rs:369-377`) and snapshot read. Open question: whether `config/gate-presets/` belongs in the set. REQ-11 makes presets storage bytes keyed by that path, so including it keeps "every declaration file" literal.

#### 5.5 Scope type (REQ-20)
- **A (recommended).** One domain `IssueScope { All, Issue(IssueId), Label{namespace,value} }` with one `FromStr` and a typed error, and one pure selection function over issues. Link check gains `label:`.
- **B.** One type, with link check rejecting `Label` at its boundary. This keeps a second parse rule.

#### 5.6 Legacy destination fallback
Delete the `legacy_root` parameter. Resolution takes the archive root and returns the marker match, a conflict, or the preferred root. `artifact_destination_root`'s container arm stays only if `classify_artifacts`' default (`artifact_classifier.rs:667-670`) still needs it. Prefer making `destination_root` mandatory on `ArtifactClassificationInventory` and dropping that arm. Update `docs/reference/cli-commands.md:380-385`.

### 6. Architecture fit / invariant check

- `@/inv/architecture-dependency-direction`: deleting the `storage::artifact_*` planners removes storage-side calls into domain planning. Moving `RootRelativePath` and the layout violation into domain makes `domain → repository_state` imports zero (C7, C10).
- `@/inv/convention-convergence`: one planner, one destination-evidence function, one marker constant, one path validator, one scope type.
- `@/inv/orthogonal-tests`: the 22 storage-level discovery tests (`tests/fast_docs_templates/artifact_discovery_tests.rs`) must be re-homed, not duplicated. Pure discovery properties go to domain tests with evidence maps. No-follow and symlink classification belong to the capture primitive's own tests (REQ-16 owner), plus one command-level preview test.
- `@/inv/derived-state-coherence`: execute already plans and publishes in one session. REQ-06 changes reads only.
- `@/inv/single-source-prose`: update the adopter docs listed in §3 in the same change as each behaviour.
- `@/charter/D-19`: candidates stop parsing the log per target once f8a6b298's evidence records land.

### 7. Sequencing

Each step leaves the workspace green. All steps follow 2195c98f (an epic-level dependency).

1. **S-08: references resolved once, reachability once per root (REQ-08).** Files: `src/domain/artifact_discovery.rs`. 2195c98f: 3ee3a4d1 `stage-domain-issue-ids` (edits `:537`). Edge: S-08 ← d5966ba2 (lead-friction, Ready; edits `artifact_discovery.rs:314-330,501-520`). Independent of S-06, so it can run in parallel.
2. **S-18a: layout-violation domain type and single evidence helper (REQ-18).** Files: `src/domain/artifact_classifier.rs`, `src/repository_state/mod.rs`, `src/repository_state/archive.rs`. Edge: ← 4575a03d (v1-release bug, Ready; classifier). 2195c98f: 1f6d3754 `plan-error-derive`.
3. **S-18b/20-paths: `RootRelativePath` into domain, one validator; typed domain errors (REQ-18, REQ-20).** Files: `src/repository_state/path.rs`, `src/domain/types.rs`, `src/domain/repository_inputs.rs`, `src/domain/key_reference.rs`, `src/profile/package.rs`, `src/profile/package_archive.rs`, `src/storage/path_errors.rs`, `src/storage/{json,memory,git_revision,mod}.rs`, `src/commands/document.rs`, `src/validation/graph.rs`, plus the 19 `RootRelativePath::parse` files in §3. 2195c98f: 18dc6e3e `well-known-paths-repository-state`, 945f65f3 `well-known-paths-commands`, and 2ac50581/182350ce/42bcaa6e for the anyhow `FromStr`. Edge: ← 63c11d08 (842b0ac6 `single-domain-module-paths`, which edits the `repository_inputs` re-exports).
4. **S-06a: preview through the execute planner (REQ-06).** Files: `src/commands/archive.rs`; harness test `tests/fast_issue/short_hash_tests.rs`. Preview runs `capture_archive_plan` in a Done-only session, and the impl bounds become `IssueStore + RepositoryStateStore`. 2195c98f: c1c86312 `scoped-issue-id-arguments` (`ArchiveTarget::Container` holds `IssueId`), c400730c `prefix-resolution`, f8f80eea `hierarchy-config-callers`. Edges: ← d0d73bef (f8a6b298 `archive-event-reader-removal`, which transitively covers 5e1ba005 per the steward note); ← d9ba0029 (f4d9502a `archive-pinned-reads-batched`; see Needs decision); ← d5966ba2.
5. **S-06b: candidates in one session; delete storage planners (REQ-06).** Files: `src/commands/archive.rs`, `src/storage/mod.rs`, `src/storage/json.rs` (`inspect_artifact_location` and its test), `src/storage/memory.rs`, delete `src/storage/artifact_discovery.rs` and `src/storage/artifact_planning.rs`, new `src/storage/citation_scan.rs` (scan plus its no-follow walker, on the REQ-16 primitive module), the 5e1ba005 evidence method in both backends and in `tests/fast_docs_templates/template_apply_atomicity_tests.rs` and `tests/fast_rules/local_rule_enforcement_tests.rs`, re-homed `tests/fast_docs_templates/artifact_discovery_tests.rs`, `tests/fast_docs_templates/harness_demo.rs`. Edges: ← S-06a; ← the REQ-13 step (image sharing); ← the REQ-16 step if the walker moves onto shared primitives.
6. **S-07/20-dest: one destination-evidence function, marker constant, no legacy fallback (REQ-07, REQ-20).** Files: `src/commands/archive.rs`, `src/domain/artifact_classifier.rs` (+ tests `:2380-2560`), `src/repository_state/archive.rs`, `docs/reference/cli-commands.md:337,373,380-385`. Edge: ← S-06b.
7. **S-19: snapshot from one image (REQ-19).** Files: `src/commands/snapshot.rs` (`enumerate_issues`, `copy_jit_state`, `assemble`, `export_snapshot`), `src/repository_state/path.rs` (declaration set), `src/commands/validate.rs:369-377`, `tests/cli_issue/snapshot_export_tests.rs`. 2195c98f: 855d027d `issue-record-path-commands`, 945f65f3, c1c86312. Edge: ← 7cd27db8 (f4d9502a `snapshot-blobs-through-module`, which threads one object reader through `assemble`; it is downstream of c140921c `snapshot-metadata-through-module`).
8. **S-20-scope: one scope type (REQ-20).** Files: `src/snapshot.rs`, `src/document/scope.rs`, `src/document/mod.rs`, `src/main.rs:5429-5434,7753-7756`, `src/output/failure.rs:315`, `src/commands/document.rs:1100-1120`, `src/commands/snapshot.rs:99-118`, `src/cli.rs` help text, `src/validation/graph.rs:85`, `docs/reference/cli-commands.md:2846,3595`. 2195c98f: c1c86312. Edge: ← S-19 (same `enumerate_issues`).

Other in-flight neighbours:
- df129d9e a015dcbb ("Archive and snapshot methods behind an archive facade") moves these methods by their current storage bounds. 3cf69ecf and 182f4c03 are their handler moves.
- cd1da611 (slug) does not touch archive destinations.

## Part: Configuration, executor caches, presets, gate evidence and image sharing

## b697a3f7 investigation: configuration, executor caches, presets, gate evidence, image sharing

Scope: REQ-09, REQ-10, REQ-11, REQ-12, REQ-13, and the REQ-20 items: declaration modules only parse, status summary, type extraction, edit distance, write-if-changed constructor.
Code read at HEAD `e2f2693d8` (HEAD moved during the investigation: 459bb995's rework `3bfbfc1af` merged as `4568aea4f`; the line numbers below are re-read after that merge).

### 1. Claim classification

| # | Claim / REQ | Class | Current evidence |
|---|---|---|---|
| C1 | Executor keeps four disk-backed `OnceLock` caches (rules, configuration) | valid-and-open | `commands/mod.rs:2127` `rules`, `:2138` `effective_rules`, `:2144` `config`, `:2147` `namespaces`. Loaders: `rules()` `:2478` → `storage::ruleset_store::load_ruleset(self.storage.root(), …)` `:2487`; `effective_rules()` `:2521` (probes `rules.toml` on disk at `:2525`); `cached_config()` `:2558` → `config_manager.load()` → `JitConfig::load`; `cached_namespaces()` `:2572`; `repo_content_format()` `:2593`. A fifth `OnceLock`, `worktree_paths` `:2168`, is the injected worktree authority, not configuration. |
| C2 | `JitConfig::load` and the image declaration path are two assembly pipelines | valid-and-open, understated | At least six assemblies of a `JitConfig`: (1) `JitConfig::load` `config.rs:1392` (filesystem: config + `TemplateRegistry::load` + `validate_item_kinds` + `InvariantRegistry::load`); (2) `materialize::assemble_config` / `assemble_config_from_declarations` `repository_state/materialize.rs:67,79` (image: config + invariants only, **no templates, no item-kind validation**); (3) `validation_declarations_from_image` `repository_state/mod.rs:504` (image: (2) + templates `:523`); (4) `commands/validate.rs:748` `effective_config` + `:767` `empty_materialization_config` (overlay bytes: config + invariants); (5) `repository_state/initialize.rs:228-230` (config, empty invariants); (6) `commands/template.rs:546` `template_config_from_image` (+ templates, absent config → empty) and `:574` `template_declarations_from_image` (a third rule-set assembly beside `repository_state/mod.rs:537-552` and `CommandExecutor::effective_rules`). The user/system layers also go through `JitConfig::load` (`config.rs:1578,1584`), which then reads `templates.toml`/`invariants.toml` from `~/.config/jit` and `/etc/jit`. Absent `config.toml`: (1) empty config, (3) `MissingCapture` error, (6) empty config. |
| C3 | `ConfigurationDeclarations` mirrors five fields of the config it wraps | valid-and-open | `declarations/mod.rs:30-42` (`parsed` plus `hierarchy`, `namespaces`, `item_kinds`, `projections`, `documentation`, all cloned at `:65-77`). Production readers of the mirror fields: only `repository_state/profile_apply.rs:2869` (`.projections`); test readers `declarations/mod.rs` tests, `commands/init.rs:1180` (test), `tests/cli_repo_workflow/init_tests.rs:689`. `materialization_config` (`:54`) clones `parsed` again. |
| C4 | `get_hierarchy_config` re-reads three TOML files, 13 call sites | valid-and-open | `config_manager.rs:263-275` → `ConfigManager::get_namespaces` → `JitConfig::load` (config, templates, invariants). 12 production sites + 1 test (`config_manager.rs:386`); see §3. |
| C5 | Declarations are parsed more than once within one mutation | valid-and-open | `publish_captured_lifecycle_mutation`: `commands/mod.rs:2808` (preflight image), `:2911` and `:3102` (same session image, same attempt). Same-image double parses (`declarations_from_image` + `assemble_config`): `issue.rs:75-76` (and `:625` before it in `delete_issue`, so three parses), `dependency.rs:384-385`, `gate.rs:1049`+`:1072`, `gate.rs:1301`+`:1329`, `project.rs:142`+`:159`, `gate_check.rs:461-462`, `:523-524`, `validate.rs:1078-1079`, `template.rs:344` (`template_config_from_image` + `template_declarations_from_image`). In capture, `validate.rs:450` re-parses config on every discovery step of `capture_proposed_base_inner`. |
| C6 | Gate presets have three readers; `PresetManager` does raw filesystem I/O on a hardcoded path; the memory backend copies its listing | valid-and-open, understated | Four parsers of preset files: `gate_presets/manager.rs:24-63` (JSON backend via `storage/json.rs:1463-1471`, `fs::read_dir` + nix `O_NOFOLLOW` read at `:98`), `storage/memory.rs:206-242`, `commands/gate.rs:262` `parse_captured_gate_presets` (image, used by apply), `commands/template.rs:690` `parse_template_presets` (image). Two listing helpers: `gate.rs:244` `listed_gate_preset_paths`, `template.rs:620` `listed_json_paths`. Hardcoded path `manager.rs:25,95` although `VirtualPath::GATE_PRESETS` exists (`repository_state/path.rs:344`). `memory.rs:904-916` duplicates `PresetManager::list_presets` (`manager.rs:72-85`). `gate preset show/list` read storage (disk), while `gate preset apply` reads the image: two sources for one preset. Only `load_presets_from_custom_files` (`gate_presets.rs:160`) is shared. |
| C7 | Precheck evidence capture is a parallel variant of gate-evaluation capture | valid-and-open | `captured_precheck_plan` `commands/mod.rs:657-747` + `PrecheckEvidence` `:566` vs `captured_gate_evaluation` `gate_check.rs:321-425` + `GateEvaluationEvidence` `:80`. They differ: evaluation stores the gate definition and pinned evidence and adds Exec document paths; precheck stores `VirtualPath::GATES` instead of the definition, reads no documents (by decision `@/issue/8454236c/decision/D-1`), and re-parses every run record at `:704-722` (`execute_captured_prechecks` parses them again through `captured_gate_runs`, `gate_check.rs:1462`). Prechecks record no `inputs_digest` and never reuse a verdict, while evaluation does (`gate_check.rs:877-895`). |
| C8 | Prompt size limit enforced twice | valid-and-open | Evaluation checks at capture (`gate_check.rs:385-392`) and again at context build (`:1265-1272`). Precheck capture reads the prompt without a size check (`mod.rs:731-738`), so prechecks check it only at context build. |
| C9 | `check_gate_with` re-derives the issue, registry and gate from the same image several times | valid-and-open | Inputs session: `capture_gate_evaluation_image` resolves them at `gate_check.rs:746-748`, the attempt closure does it again at `:858-860`, and `registry.gates.get` runs a third time at `:883-886`. Record session: inside `capture_gate_evaluation_image` again, then `:921-923`. |
| C10 | Each capture round deep-clones the whole image | valid-and-open | `storage/repository_state_store.rs:599` (JSON) and `:750` (memory) `self.captured = Some(image.clone())`. `RepositoryImage` owns its maps (`repository_state/image.rs:698-704`), and `CapturedBytes` owns its `Vec<u8>` (`:105-108`), so the clone copies every byte. |
| C11 | `MaterializationPlan::new` clones the image again | valid-and-open | `repository_state/mod.rs:719-727` (`image: image.clone()`). |
| C12 | `ensure_session_image` compares byte for byte | valid-and-open | `repository_state_store.rs:832-843`: `captured != Some(image)` over the derived `PartialEq` (`image.rs:697`). The session's captured image is used only for this check (`:558`, `:717`). |
| C13 | `InvariantRegistry::load` reads files although declarations are documented filesystem-free | valid-and-open | `declarations/invariants.rs:144-156` against `declarations/mod.rs:1`. `templates.rs:432-450` `TemplateRegistry::load` does the same. Production caller of both: only `JitConfig::load` (`config.rs:1412,1418,1443,1457`). The precedent is `storage/ruleset_store.rs:19`. |
| C14 | `StatusSummary` stores the Backlog count under `open` | already-done (naming) | The field has been `backlog` since `f7340f63f` (842b0ac6/66bd4b5c): `commands/mod.rs:1816-1825`; the JSON message and human lines say `backlog` (`main.rs:6264-6273`). |
| R20a | Declaration modules only parse; file reads live in storage | valid-and-open | C13. |
| R20b | Status summary sits beside the query commands, reports `backlog` | valid-and-open (location only) | The struct is at `commands/mod.rs:1816` and `get_status` at `commands/validate.rs:2024`; the query commands are in `commands/query.rs`. Side findings: the server has a parallel `StatusResponse` that drops `gated`/`rejected` (`crates/server/src/routes.rs:219-244`). The web `StatusSummary` type (`web/src/types/models.ts:115`) matches nothing the server returns, and `client.getStatus` has no caller. Human `jit status` omits `gated` (`main.rs:6271-6277`). |
| R20c | Type extraction from labels has one normalisation | valid-and-open | `domain/type_taxonomy.rs:199` `extract_type` trims and lowercases; `labels.rs:169` `type_value_of` (and `type_label_value(s)` `:193,204`) returns the raw value. The label grammar allows uppercase values (`labels.rs:40`), so `type:Epic` is a known `epic` to `detect_validation_issues` (`type_taxonomy.rs:303`) and membership warnings (`:338,387`) but unknown to the graph and validation code built on `type_value_of`. Test `type_taxonomy.rs:506` asserts the lowercasing. |
| R20d | Edit distance has one implementation | valid-and-open | `domain/type_taxonomy.rs:209` `levenshtein_distance` (used `:257`); `validation/engine.rs:889` `levenshtein` (used `:801`; its comment `:886-888` defends the copy). |
| R20e | Write-if-changed action has one constructor | valid-and-open, understated (6 sites, not 5) | `materialize.rs:363-377` (configured projection), `:444-458` (data-root ignore), `:541-553` (default schemas), plus a string-compare variant at `:517`; `repository_state/mod.rs:1046-1060` (profile repair); `repository_state/mutation.rs:1009-1022` (gate registry edit); `repository_state/package_tree.rs:271-296` (through `TreeFileDisposition`). `profile_apply.rs:1938` (`ProfileTargetDisposition`) is the same comparison as a disposition. The plain constructor is `RepositoryAction::write_file` (`image.rs:1138`). |
| REQ-09 | One pure assembly from bytes; filesystem and capture call it; no mirror fields | valid-and-open | C2, C3. |
| REQ-10 | No executor caches; read paths from captured image; hierarchy from declarations; one parse per attempt | valid-and-open | C1, C4, C5; §3. |
| REQ-11 | Backends supply preset bytes by path; one pure listing and lookup; no preset I/O outside storage | valid-and-open | C6. |
| REQ-12 | One evidence capture with the prompt limit; resolve issue, registry and gate once | valid-and-open | C7–C9. |
| REQ-13 | Share the image without copying; identify it by token | valid-and-open | C10–C12. |

What changed today in these areas:
- 459bb995 (`4590bd483`, then the rework `3bfbfc1af`) stated config defaults once and **added `JitConfig::content_format()`/`strictness()`** (`config.rs:1370,1381`). It also moved `gate_check.rs` and `template.rs` readers onto them. That delivers most of 9876a23a's `config-default-accessors` (977d01d8) REQ-01. `CommandExecutor::repo_content_format` (`mod.rs:2593`) now only forwards to it.
- 5e846743 (`abc794762`) touched only `agent_config.rs` and `config.rs`'s test-only agent types.
- Neither changed `JitConfig::load`, `parse_configuration` or `ConfigurationDeclarations`.
- 3d9a0ce3 today:
  - 72d89161 (`1c36f0c8e`) added `VerdictSource`, `declared_inputs_digest` (`gate_check.rs:986`, which reads live worktree files outside the image) and `reusable_verdict` (`:1013`, a storage scan outside the image) to `check_gate_with`.
  - ae2e3739 (`5066bec26`) changed the digest roots.
  - 8673dc5e (`0179a6820`, `d9f1d644d`, `e2f2693d8`, in progress) records attestations as runs and changed `compact_run_history_for_context`.
  - None of them unified the precheck and evaluation evidence or the prompt limit.
- f11514dd (842b0ac6, `f2826a737`) removed linked-worktree evidence from capture.

### 2. Prior-art sweep

- Charter `dev/vision/9db27a3a-charter.md:384-393` (D-20): one computation per capability; a preview runs the same mutation session read-only; "a rule, derivation, or parser exists once". D-17 (`:302`): a mutation reads only the registries and issues its derivation consults.
- `dev/architecture/repository-state-materialization.md`:
  - `:83-100`: the image is the single closed input, grown by `extend`, never re-read.
  - `:140-145`: "apply refuses a plan whose preimages disagree with the session's image". The image-recognition prose to sweep when REQ-13 lands.
  - `:163,179`: `MaterializationPlan::new(image, delta)` is the shared tail.
  - `:229-230`: `declarations_from_image` / `validation_declarations_from_image` are the canonical bundle.
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-3.md:26` planned to drop the whole-`JitConfig` field from `ConfigurationDeclarations` and narrow consumers to components, the opposite direction to the one recommended in §5. `cdc840ad-handoff-11.md:127` accepted the executor `OnceLock` caches as "read-only conveniences" that REQ-10 now retires.
- `@/issue/8454236c/decision/D-1`: the mutation path captures no issue-linked document, so a broad precheck's stability is judged only over mutation-base inputs. A unified evidence function must keep this distinction (§5, REQ-12).
- No user doc (`docs/`) mentions the caches, `PresetManager` or the assembly paths. `docs/reference/gate-presets.md:9` and `docs/reference/cli-commands.md:2328,2348` describe `.jit/config/gate-presets/<name>.json` and stay valid.

### 3. Consumer sweep

Executor cache readers (production; tests separate):
- `commands/mod.rs`: 4 cache fields `:2127-2147`, `ConfigManager::new` `:2174`, `rules()` `:2478`, `effective_rules()` `:2521`, `cached_config()` `:2558`, `cached_namespaces()` `:2572`, `repo_content_format()` `:2593`, and `require_active_lease` `:3402` (cache read; REQ-04 deletes it). Owning tests `:4094`, `:4148` (and `:4251`, `:4276`) assert the caching itself and go.
- `commands/validate.rs` (16): `:1270, 1403, 1455` (a direct `load_ruleset` from disk), `:1461, 1494, 1505, 1507, 1584, 1608, 1774, 1775, 1790, 1841, 1933, 1934`, plus `get_hierarchy_config` `:1141`. Almost all of these sit in executor validation passes that 9876a23a deletes (`executor-pass-copies-removal` 6ac41880, `executor-graph-evaluation-removal` f22147ec).
- `commands/graph.rs` (7): `:235, 316, 404, 409, 431, 497, 584`.
- `commands/item.rs` (5): `:73, 135, 287, 365, 481`.
- `commands/bulk_update.rs` (3): `:624, 625, 627` (`validate_update_preview`; REQ-05 rewrites it). The same function also calls `load_gate_registry()` from disk at `:608`.
- `commands/document.rs` (3): `:165, 289, 345` (`get_hierarchy_config`).
- `commands/archive.rs` (3): `:159, 185` (`config_manager.load()`), `:186`. REQ-06 rewrites archive planning.
- `commands/template.rs` (1): `:100`.
- `commands/query.rs` (1): `:58` (`config_manager.get_namespaces()`).
- `commands/profile.rs` (1, test): `:4634`.
- `main.rs` (3): `:1766` (`get_write_policy`), `:5872`, `:6076` (`get_namespaces`). `:6020,6030` `JitConfig::load` are `config validate` file checks.
- `crates/server/src/routes.rs` (5): `:178` (`get_hierarchy_config`), `:933, 959, 1004` (`get_namespaces`), `:972` (`get_hierarchy_icons`), all through the `pub config_manager` field (`mod.rs:2121`). The server's executor is one long-lived `Arc` (`crates/server/src/main.rs:76`), so any cached reader it reaches never refreshes. No current route reaches a `OnceLock` reader.
- Integration tests:
  - `effective_rules()`: `tests/fast_rules/default_rules_registry_derivation_tests.rs` (5), `effective_rules_tests.rs` (3), `project_render_harness_tests.rs` (1).
  - `JitConfig::load`: `tests/cli_item_validate/invariant_registry_story_tests.rs` (12), `tests/fast_docs_templates/templates_loader_tests.rs` (8 including `TemplateRegistry::load`), `tests/fast_rules/config_loading_tests.rs` (3), `tests/cli_repo_workflow/project_config_tests.rs` (1), `tests/fast_docs_templates/archive_path_layout_tests.rs` (1), plus `src/test_utils.rs:1916`.
- Other disk declaration reads beside the caches, also in REQ-10's "read paths" reading:
  - `IssueStore::load_gate_registry` (`storage/gate_store.rs:23`) at `gate.rs:823` (`pass_gate`), `:988` (`fail_gate`), `:1009` (`list_gates`), `:1239` (`show_gate_definition`, a server route), `gate_check.rs:1368, 1407, 1568`, `validate.rs:1414, 1704`, `bulk_update.rs:608`.
  - `load_ruleset` (`storage/ruleset_store.rs:19`; production callers `mod.rs:2487`, `validate.rs:1455`).

Config assembly consumers (REQ-09):
- `parse_configuration` / `ConfigurationDeclarations` / `materialization_config`:
  - `repository_state/mod.rs:233, 423, 433, 442, 510, 889, 1446-1448`
  - `repository_state/materialize.rs:67-92, 887, 931`
  - `repository_state/initialize.rs:228-230`
  - `repository_state/profile_apply.rs:2867`
  - `commands/validate.rs:748-770, 451, 1018`
  - `commands/init.rs:366, 1180, 1390`
  - `commands/template.rs:603`
  - `tests/cli_repo_workflow/init_tests.rs:4, 71, 689`
- `assemble_config` (image) callers: `issue.rs:76, 609, 625`, `dependency.rs:304, 385`, `gate.rs:321, 1049, 1288, 1301`, `archive.rs:519`, `project.rs:142`, `template.rs:550`, `validate.rs:1079`, `gate_check.rs:462, 523`, `profile_apply.rs:1628, 1841`, `initialize.rs:698`, `materialize.rs:795`.
- `JitConfig::load` production: `config_manager.rs:49`, `config.rs:1578, 1584, 1590` (layers), `main.rs:6020, 6030`.

Gate presets (REQ-11):
- Source: `gate_presets/manager.rs` (whole file), `gate_presets.rs:14-20` (re-export), `storage/mod.rs:351, 358` (trait methods), `storage/json.rs:1463-1471`, `storage/memory.rs:206-242, 904-923`, `commands/gate.rs:244-275, 292, 1249-1258, 1312`, `commands/template.rs:620, 690-704, 401`, `main.rs:4542, 4584`.
- Tests: `tests/fast_docs_templates/template_apply_atomicity_tests.rs:179-183` and `tests/fast_rules/local_rule_enforcement_tests.rs:899-907` (trait forwarders), `template_apply_tests.rs:716, 864, 975, 997`, `planning_preset_tests.rs:283-301`, `gate.rs:1961-2017`, `manager.rs` tests.
- Generated reference: `gate_presets/reference.rs`, `generated_artifacts.rs:111-116`, `docs/reference/gate-presets.md`. These stay.

Gate evidence (REQ-12): `commands/mod.rs:557-747` (`CapturedTransitionEvidence`, `PrecheckEvidence`, `PrecheckValidationView`, `CapturedPrecheckPlan`, `CachedPrecheckExecution`, `captured_precheck_plan`; consumer `publish_captured_lifecycle_mutation` `:2767-3110`), `commands/gate_check.rs:15, 80-102, 196, 273, 321-425, 690-981, 1246-1310, 1434-1555`. Test helper `mod.rs:3648` `capture_precheck_plan`.

Image (REQ-13): `image.rs:697-861` (`RepositoryImage`, `close`, `into_answers`); `repository_state_store.rs:396, 406, 558, 599, 717, 750, 832-843`; `repository_state/mod.rs:571-727`; `overlay.rs:53-92` (deep-clones the base maps, outside the criterion's literal scope); `RepositoryImage::close` callers `repository_state_store.rs:1210, 1263`, `overlay.rs:92` and test builders (`archive.rs:638`, `export.rs:334, 471`, `initialize.rs:1328`, `mod.rs:1263, 1325, 1424, 1436`, `document/reference.rs:349`). Equality-dependent tests: `repository_state_store_tests.rs:368, 406, 2120` (JSON image equals memory image), `materialize.rs:1042, 1152`.

REQ-20 items:
- Status: `main.rs:6261`, `crates/server/src/routes.rs:233, 1402`.
- `extract_type`: 16 uses, all in `type_taxonomy.rs`. `type_value_of`/`type_label_value(s)`: about 90 uses in about 30 files (largest: `labels.rs` 21, `commands/issue.rs` 15, `tests/fast_issue/transition_graph_enforcement_tests.rs` 9).
- Edit distance: 2 definitions, 2 callers.
- Write-if-changed: the 6 sites in R20e.

### 4. Primitive verification

| Property | Delivered today? | Evidence / what is needed |
|---|---|---|
| One pure config assembly from bytes | No | Six assemblies (C2). The pure pieces exist: `parse_configuration` (`declarations/mod.rs:65`), `TemplateRegistry::from_toml_str`, `InvariantRegistry::from_toml_str`, `validate_item_kinds` (`config.rs:1505`). Needed: one function over `(config, templates, invariants)` bytes that fixes absent-file semantics, template-type checking and item-kind validation once. |
| No disk-backed executor cache | No | C1. Needed: every reader takes a captured `CapturedRepositoryDeclarations`. |
| Read paths read declarations from an image | No | No read-only session exists. `RepositoryStateStore` offers only `open_mutation_session` (`repository_state_store.rs:224`), which takes the bootstrap, repository and events write locks (`:445-470`). `jit validate` already reads through a mutation session it never applies (`validate.rs:556`), and that is the existing read pattern. |
| Hierarchy derived from declarations, no disk read | No | `get_hierarchy_config` reads disk (`config_manager.rs:263`). `validation_declarations_from_image` computes `namespaces` (`repository_state/mod.rs:533`) and then discards it; every hierarchy user recomputes `namespaces_from_config` + `repository_state::hierarchy_config` (`default_rules.rs:520`, which silently falls back to empty). 2195c98f 62a0085a supplies `HierarchyConfig::from_declarations`. |
| One declaration parse per mutation attempt | No | C5. The capture helpers return `MutationBase { image, issues }` (`validate.rs`), with no parsed declarations, so every derivation re-parses. |
| One preset reader, pure list/lookup | No | C6. A pure parse exists (`load_presets_from_custom_files`); listing and lookup are written three times. |
| One evidence capture with the prompt limit | No | C7, C8. |
| Issue/registry/gate resolved once | No | C9. `capture_gate_evaluation_image` already resolves all three and returns only `(image, target_id)` (`gate_check.rs:690-814`). |
| Image shared, not copied | No | C10, C11. `extend` already moves entries (`into_answers`, `image.rs:844-860`); only the session's retained copy and the plan's copy clone. |
| Session recognizes its image by identity | No | C12. The session's copy serves only this check, so a token replaces it without loss. Semantic change: a byte-identical image from another session is refused today only if bytes differ, and with a token it is always refused. No production path builds a plan from a preflight image: `preflight.image` is read only (`gate.rs:1288`, `issue.rs:609`, `mod.rs:2803`). |
| One type normalisation / one edit distance / one write-if-changed | No | R20c–R20e. |

### 5. Design options

**REQ-09: one assembly.**
- **A (recommended).** Delete `ConfigurationDeclarations`. Add one pure `declarations::assemble_configuration(ConfigurationSources { config: Option<&[u8]>, templates: Option<&[u8]>, invariants: Option<&[u8]> }) -> Result<JitConfig, ConfigurationDeclarationError>`. It parses the config, populates templates (checked against the hierarchy types), runs `validate_item_kinds`, populates invariants, and treats an absent config as empty.
  - `CapturedRepositoryDeclarations.config` becomes the only parse, and `RepositoryDeclarations.configuration` becomes `&JitConfig`.
  - `assemble_config`, `assemble_config_from_declarations`, `materialization_config`, `validate.rs` `effective_config`/`empty_materialization_config`, `template_config_from_image` and the `initialize.rs:228` path collapse into calls.
  - The filesystem loader reads the three files and calls it. User and system layers pass `templates`/`invariants` as `None`, which ends today's reading of `~/.config/jit/templates.toml`.
- **B.** Keep `ConfigurationDeclarations` as narrowed components and drop `parsed` (cdc840ad direction). `JitConfig` is still needed whole, so B grows the bundle back into a mirror of every field. Rejected (D-18).
- **C (optional hardening, not recommended here).** Split a `Deserialize` raw `ConfigToml` from an assembled `JitConfig` without `Deserialize`, so a half-assembled config (templates and invariants `#[serde(skip)]`, `config.rs`) cannot exist. Costs about 30 `toml::from_str::<JitConfig>` test sites and doc examples in `config.rs`.
- Decision inside A: item-kind and template strictness. Today validation reports a malformed `[item_kinds]` as a finding (`validation/repository.rs:192`), because the image path skips `validate_item_kinds`. One strict assembler turns that finding into a declaration-parse structural failure, as templates and gates already are. 9876a23a's report work owns how structural failures render.

**REQ-09/REQ-20: where the filesystem loader lives.**
- **A (recommended).** `storage/` (sibling of `storage/ruleset_store.rs`, for example `storage/configuration_store.rs`). Delete `InvariantRegistry::load` and `TemplateRegistry::load`. This satisfies REQ-20's "file reads in storage".
- **B.** Keep the filesystem read in `config.rs`. 459bb995's notes say "Configuration file I/O stays in config.rs" (layered user/system loading). B keeps that but leaves templates/invariants reading in non-storage code unless moved anyway. Under A, `EffectiveConfig::load`'s user/system probing can stay in `config.rs`: those layers are not repository declarations.

**REQ-10: how read paths get an image.**
- **A (recommended).** One executor helper `capture_declarations(&self) -> CapturedRepositoryDeclarations`. It opens the session the epic defines as "read-only" (D-20: the operation's mutation session, never applied, as REQ-05/06 previews use), captures `CONFIG, INVARIANTS, RULES, GATES, TEMPLATES` plus rule-schema paths, and parses them through the one assembler. Reads then serialize behind writers, as `jit validate` already does.
- **B.** A new lock-free read session in storage. Better server concurrency, but it needs new torn-read and leftover-redo-record rules. That is more mechanism (D-18) and belongs with the preview read-only session decision, not here.
- The bundle gains `namespaces: LabelNamespaces` (already computed at `repository_state/mod.rs:533`) and a `hierarchy() -> Result<HierarchyConfig, ConfigError>` accessor over 62a0085a's constructor. It is computed on call rather than eagerly, so an invalid hierarchy fails only hierarchy users, matching f8f80eea's propagation.
- `ConfigManager`'s disk readers (`load`, `get_namespaces`, `get_hierarchy_icons`, `get_write_policy`, `get_project_name`) and `get_hierarchy_config` are deleted. The pure helpers (`namespaces_from_config`, `write_policy_from_config`, `invocation_write_override`) stay.

**REQ-10: one parse per attempt.** Return the parsed bundle with the base: extend `MutationBase` (or the `CapturedValidation` that 9876a23a's capture-hands-records introduces) with `declarations`, and thread it to every derivation. The preflight image is a different capture, so "once per attempt" holds strictly only if the preflight's parse is reused when the declaration entries' `EntryIdentity` values are unchanged. Otherwise the honest form is "once per captured image" (Needs decision 1).

**REQ-11: presets.**
- **A (recommended).** Delete `IssueStore::list_gate_presets`/`get_gate_preset` and `PresetManager`. `gate preset list/show` capture `GATE_PRESETS` (listing plus `*.json`) through the REQ-10 capture helper, so the storage backend supplies the bytes keyed by the gate-presets path through the session. One pure `gate_presets::presets_from_image` (listing, then `load_presets_from_custom_files`) plus pure `list`/`get` over its map serves show, list, apply and template apply. Show and apply then read one source, and the backend I/O and nix code go.
- **B.** Replace the two trait methods with `IssueStore::gate_preset_files() -> Vec<(String, Vec<u8>)>` and keep a disk read for list/show. It keeps two sources (storage for show, image for apply) and a trait method; rejected.

**REQ-12: evidence.**
- One `captured_gate_evidence(image, issue, gates: &[(&GateKey, &GateDefinition)]) -> Result<CapturedGateEvidence { evidence, prompts }>`. It enforces `MAX_PROMPT_FILE_SIZE` once at capture and deletes the check at `gate_check.rs:1265-1272`.
- The evidence records each gate definition, so precheck's `VirtualPath::GATES` input becomes the definitions.
- Documents and pinned evidence are taken only where the capture spec holds them, the same filter `PrecheckEvidence::matches` uses (`mod.rs:586-603`), so `@/issue/8454236c/decision/D-1` is preserved without a precheck variant.
- `capture_gate_evaluation_image` returns a bound struct `{ image, issue, registry, gate }`.
- Moving the precheck size check to capture changes when it fails: before any precheck runs, instead of at that gate.

**REQ-13: image identity.**
- **A (recommended).** `RepositoryImage { data: Arc<ImageData>, token: ImageToken }`. `close` mints the token from a process-wide counter, and clones share it. `PartialEq` compares data only, which keeps the JSON==memory and plan==image tests. The session stores `Option<ImageToken>`. `extend` moves entries through `Arc::try_unwrap`, copying only when the caller kept another handle. `MaterializationPlan::new` clones the `Arc`. Literal fit to "identity token".
- **B.** The session holds the `Arc` and checks with `Arc::ptr_eq`, taking it before `extend`. One fewer type, but a pointer is not obviously a "token" under literal review.

**REQ-20 type normalisation.**
- **A (recommended).** Exact values everywhere: delete `extract_type`; `type_value_of` is the one extractor. This matches canonical-token cutovers such as 66bd4b5c and `@/inv/canonical-cutover`. The unknown-type finding with the edit-distance suggestion already offers the lowercase fix.
- **B.** Lowercase everywhere: change `type_value_of` and every hierarchy lookup. This widens accepted input and diverges from the label grammar's case-sensitive values.

**REQ-20 edit distance.** Keep the single-row version (`validation/engine.rs:889`) and move it to a pure shared home (for example `domain/text.rs`, or `labels.rs` beside the suggestion helpers). Validation depends inward on domain, so `engine.rs`'s reason for a copy (`:886-888`) does not hold.

**REQ-20 write-if-changed.** `RepositoryAction::write_if_changed(entry: &RepositoryEntry, path, owner, bytes, mode) -> Option<RepositoryAction>` in `image.rs` beside `write_file`. It is the one place that decides "unchanged" (file with equal bytes and mode). Use it at the six sites. `package_tree.rs` and `profile_apply.rs:1938` keep their disposition enums, derived from the same predicate (`RepositoryEntry::holds(bytes, mode)`).

### 6. Architecture fit and invariants

- `@/inv/architecture-dependency-direction`:
  - The assembler in `declarations/` and the preset listing in `gate_presets.rs` are pure.
  - File reads move into `storage/`.
  - `get_hierarchy_config(storage)` (`config_manager.rs:263`), which takes an `IssueStore` to read disk, is deleted.
  - `Arc` in `repository_state/image.rs` adds no I/O.
  - The hierarchy derivation is the domain constructor.
- `@/inv/convention-convergence`: removes the six assemblies, four preset readers, two evidence captures, two edit distances, two type extractors and six write-if-changed copies.
- `@/inv/derived-state-coherence`: the record publication path is unchanged. Reads stop depending on per-process state that can disagree with the captured image, which is what the cache fields do today: a cached config feeds lease mode at `mod.rs:3402` while the session reads another.
- `@/inv/domain-agnostic`: the preset path comes from `VirtualPath::GATE_PRESETS`, not a literal.
- `@/inv/orthogonal-tests`:
  - Delete the cache-owning tests (`mod.rs:4094, 4148, 4251, 4276`).
  - Add one owning test for the assembler's absent-file and strictness cases, one conformance case for preset listing through the session on both backends (it replaces `manager.rs` tests and the paired memory tests), and one test that a byte-identical image from another session is refused.
- `@/charter/D-19`: unaffected. Run-history reads in evidence are f8a6b298's (0bd2c797).

### 7. Sequencing

Upstream by epic edge: all of 2195c98f. Keys this scope needs:
- REQ-09: `validation-strictness-vocabulary` 96bb0941, `content-format-vocabulary` 42bcaa6e, `item-kind-id-pattern` bc1955a8 (config decoding).
- REQ-09 storage move and REQ-11: `well-known-paths-storage` e1a6c8c2 (literals in `config.rs`, `invariants.rs`, `templates.rs`, `gate_presets/manager.rs`, `memory.rs`).
- REQ-10: `hierarchy-config-constructor` 62a0085a and `hierarchy-config-callers` f8f80eea.
- Write-if-changed: `repository-action-owner` 179dcef9 (owner enum).
- REQ-12: `gate-key-registry-type` 488eb3c1 and `gate-run-ids` 79aa3aa7.
- Story nodes: `typed-declarations` 0461a6f7 covers 62a0085a, f8f80eea and bc1955a8; `wire-vocabularies` 83d04069 covers 96bb0941, 42bcaa6e and 179dcef9; `issue-identity` e2651d77 covers 79aa3aa7 and 488eb3c1.

Each step leaves the workspace building, clippy-clean and green:

1. **image-identity (REQ-13).**
   - Files: `repository_state/image.rs`, `storage/repository_state_store.rs`, `repository_state/mod.rs`, `repository_state/overlay.rs`, `storage/repository_state_store_tests.rs`, `dev/architecture/repository-state-materialization.md:140-145`.
   - No cross-epic edge needed. 9876a23a `capture-spec-incremental` 2f12d920 edits `CaptureSpec` in the same file (`image.rs:566-686`) but a different struct; if the steward wants it serial, put this step after 2f12d920.
2. **write-if-changed constructor (REQ-20).** Files: `image.rs`, `materialize.rs`, `repository_state/mod.rs`, `mutation.rs`, `package_tree.rs`, `profile_apply.rs`.
3. **suggestion primitives (REQ-20 edit distance + type extraction).** Files: `domain/type_taxonomy.rs`, `validation/engine.rs`, `labels.rs`. Needs Needs decision 2.
4. **status beside query (REQ-20).**
   - Files: `commands/mod.rs`, `commands/validate.rs`, `commands/query.rs`, `main.rs`, `crates/server/src/routes.rs` (import only).
   - df129d9e `handlers-query-status` 24a982b7 moves the same `jit status` arm; no edge either way (flag).
5. **one config assembly (REQ-09).**
   - Files: `declarations/mod.rs`, `config.rs`, `repository_state/{mod,materialize,initialize,profile_apply}.rs`, `commands/{validate,template,init}.rs`, the tests in §3.
   - After 842b0ac6 `strict-config-toml-loading` fbb661f2 (which itself follows 459bb995) and `unify-lease-limit-resolution` 34361448 (`ConfigLoader`/claim path). Both edges run ours → theirs.
6. **declaration file reads in storage (REQ-20).**
   - New `storage/` loader. Delete `InvariantRegistry::load` and `TemplateRegistry::load`. `JitConfig::load` becomes the storage read plus the step-5 assembler.
   - Files: `storage/`, `config.rs`, `declarations/invariants.rs`, `templates.rs`, test files `templates_loader_tests.rs`, `invariant_registry_story_tests.rs`, `config_loading_tests.rs`.
7. **captured read declarations (REQ-10a).**
   - Add the capture helper and the bundle's `namespaces` and `hierarchy()`. Migrate `graph.rs` (7), `item.rs` (5), `document.rs` (3), `template.rs:100`, `query.rs:58`, `main.rs` (3), `crates/server/src/routes.rs` (5). Delete `get_hierarchy_config` and the `ConfigManager` disk readers.
   - Needs 62a0085a and f8f80eea. Uses the read-only session decision from the REQ-05/06 investigator (Needs decision 3).
   - Split if large: 7a for jit commands, 7b for server and `main.rs`.
8. **captured gate presets (REQ-11).**
   - Files: `gate_presets.rs`, delete `gate_presets/manager.rs`, `storage/{mod,json,memory}.rs`, `commands/{gate,template}.rs`, `main.rs:4542,4584`, the two test forwarders.
   - After step 7. Ours after 842b0ac6 `remove-test-only-gate-entry-points` 86d3625d and `remove-gate-auto-fields` fe761000 (both edit `gate_presets.rs`).
9. **executor caches removed (REQ-10b).**
   - Delete the four `OnceLock`s, `rules()`, `effective_rules()`, `cached_config()`, `cached_namespaces()` and `repo_content_format()`. Migrate the `tests/fast_rules` sites (9 in 3 files).
   - Files: `commands/mod.rs`, the remaining readers, `storage/ruleset_store.rs` (if `load_ruleset` loses its last production caller, keep it test-only or delete it).
   - After: this epic's REQ-04 step (`require_active_lease`), REQ-05 step (`validate_update_preview`) and REQ-06 step (`archive.rs:159-186`); 9876a23a `command-config-defaults` 145b4d57, `executor-pass-copies-removal` 6ac41880 and `executor-graph-evaluation-removal` f22147ec (these delete most `validate.rs` readers). Every edge runs ours → theirs.
10. **one declaration parse per attempt (REQ-10c).**
    - Thread the bundle through the base. Remove the double parses in C5.
    - Files: `commands/{mod,issue,dependency,gate,project,template}.rs`, `commands/validate.rs` (capture types).
    - After 9876a23a `records-loader` b3a768b3 and `capture-hands-records` 2b5f452c (same return types), and after this epic's REQ-04 step (same preflight sites `issue.rs:609`, `gate.rs:1288`). Leave `gate_check.rs:461-462, 523-524` to 9876a23a `gate-checkers-over-selection` a106d1be, or follow it.
11. **one gate evidence capture (REQ-12).**
    - Files: `commands/gate_check.rs`, `commands/mod.rs:557-747, 2767-3110`.
    - After 3d9a0ce3 `39710b3b` (built-in checker faults, same file) and `8673dc5e` (in progress, same file). After f8a6b298 `gate-run-latest-captures` 0bd2c797 and `gate-verdict-reuse-lookup` 6c0f8739, which rewrite the run-history capture in both evidence functions and move reuse into the image. All of these edges run ours → theirs.

9876a23a / 2195c98f ordering on the shared code:
- 2195c98f, upstream of everything here: 96bb0941 → 42bcaa6e; 62a0085a → f8f80eea; bc1955a8; e1a6c8c2; 179dcef9; 488eb3c1; 79aa3aa7.
- 9876a23a 977d01d8 `config-default-accessors` → 145b4d57 `command-config-defaults` → our step 9. 977d01d8's REQ-01 is now mostly already-done (459bb995 rework `3bfbfc1af`, `config.rs:1370,1381`); flag this to 9876a23a's lead.
- 9876a23a b3a768b3 `records-loader` → … → 2b5f452c `capture-hands-records` → our step 10.
- 9876a23a 2f12d920 `capture-spec-incremental` and our step 1: no edge needed (different struct). Optionally ours after.
- 9876a23a 6ac41880 and f22147ec → our step 9.

Edges that would delay another epic (flag only, not proposed as defaults):
- fbb661f2 after our step 5, so strictness lands in one parse path instead of two.
- f8a6b298 0bd2c797 after our step 11, so it rewrites one evidence function instead of two.
- 2195c98f e1a6c8c2 edits `gate_presets/manager.rs:25,95`, which step 8 deletes. The epic edge makes this churn unavoidable unless e1a6c8c2's REQ-01 list drops `manager.rs`.
- df129d9e has no edge to this epic. Its tasks move the same methods mechanically:
  - `fd2073b7` and `1ac651c7`: preset list/show, step 8.
  - `a24b11aa`: `check_gate_with`, step 11.
  - `0bc448fb`: expects the public `rules`/`effective_rules`/`config_manager` that steps 7 and 9 delete, and server `get_hierarchy_config` access.
  - `24a982b7`: `jit status`, step 4.
  - Whichever lands second rebases.

## Part: Storage primitives, redo apply and atomic publication

## b697a3f7 investigation: storage primitives and redo apply

Scope: REQ-14, REQ-15, REQ-16, REQ-17, REQ-18 (general domain imports; the artifact classifier belongs to the archive investigator), and four REQ-20 items: process liveness, no-follow repository reads, ancestor walking, initialization errors. Code read at HEAD `fb08b419b`.

### 1. Claim classification

| # | Claim / REQ | Class | Current evidence |
|---|---|---|---|
| C1 | Redo apply walks each parent chain three times | valid-and-open | Pre-pass `resolve` calls `open_capability_parent` and discards the handle (`storage/redo_apply.rs:137-143,196-205`). Each operation calls `resolve` again (`:214,234,291,310`) and then walks a third time with `open_or_create_capability_parent` (`:215,235`) or `open_capability_parent` (`:292,311-312`). `ensure_write_set_unchanged` adds a fourth walk per target before commit (`repository_state_store.rs:877-888` → `inspect_capability_entry` `:1403`). |
| C2 | Two `lstat` calls per component | valid-and-open | `open_capability_parent` lstats each component (`repository_state_store.rs:1551`), then `open_child_dir_nofollow` lstats it again (`:1610`) before the open and an `fstat` (`:1621`). `open_or_create_capability_parent` does the same (`:1584` then `:1592`→`:1610`). |
| C3 | Parent directory fsynced after every operation | valid-and-open | put `redo_apply.rs:239`, delete `:296` (on removal), make-directory `:217` (on creation), each created ancestor `repository_state_store.rs:1587`, record removal `redo_apply.rs:333`. The event-log file sync (`:320`) and staged-file sync (`:268`) are file-data syncs and must stay per operation. |
| C4 | No-follow primitives live in the session module | valid-and-open | `repository_state_store.rs:1533-1897` (parent walks, `open_child_dir_nofollow`, `open_regular_file_nofollow`, `open_absolute_dir_nofollow`, both repository-file readers, identity helpers). `CapabilityRoots` is also there (`:311-377`). The file is 2059 lines. |
| C5 | Import cycle with `redo_apply.rs:17` | valid-and-open | `redo_apply.rs:17-20` imports `open_capability_parent`, `open_child_dir_nofollow`, `open_or_create_capability_parent`, `CapabilityRoots` and `RepositoryStateStoreError` from the session module. The session imports `RedoApplier` and `RedoApplyError` back through `storage::` (`repository_state_store.rs:18-21`). `init_publish.rs:13-16` imports from both. A second cycle, not named in the REQ: the session imports `memory::{append_events, MemoryRepositoryState}` (`:17`), and `memory.rs:49,202` imports `repository_state_store::ActiveLayoutTracker`. |
| C6 | Leaf-open sequence written four times | valid-and-open | The sequence is lstat, refuse symlink or non-file, no-follow open, fstat recheck. It appears in `open_regular_file_nofollow` (`repository_state_store.rs:1631-1647`), `read_repository_file_nofollow` (`:1694-1712`), `read_confined_relative` (`:1770-1783`) and, partially, `inspect_capability_leaf` (`:1447-1467`, no fstat recheck). `json.rs` also pre-lstats before calling `open_regular_file_nofollow`, which lstats again (`json.rs:114-122,986-1012,1385-1390`). A fifth copy outside storage is `profile/nofollow.rs:31-38`, which adds `O_NONBLOCK` and is guarded as profile's single opener (`tests/scratch_build/repository_state_cutover_tests.rs:1576-1584`). |
| C7 | Record commit uses ambient path I/O | valid-and-open | `RedoRecord::commit(&self, data_root: &Path)` (`redo_record.rs:208-217`) calls `create_scratch` → `std::fs::create_dir` and `adopt_mode` → `std::fs::metadata`/`set_permissions` (`:346-366`), then `create_file_synced` and path-based `sync_directory` (`atomic_write.rs:55-95`). Callers pass paths: `repository_state_store.rs:614` (`layout.data_root()`) and `:659` (`init_publish::stage_path(..)`). The data root is not revalidated between `revalidate_layout_capabilities` (`:566`) and this path-based commit, so a root swapped in that window receives the record. |
| C8 | Five atomic-write implementations | valid-and-open (undercounted) | (1) `atomic_write.rs:144-180`, no fsync, temp name `.{name}.{pid}.{seq}.tmp`, permissions parameter always `None`; `write_file_atomic_bytes` has no non-test caller, and `write_file_atomic` has one (`user_config_store.rs:62`). (2) `claim_coordinator.rs:871-890`, fixed temp name, optional fsync, no directory sync. (3) `external_publish.rs:48-90`, with its own `COUNTER` and the same name format (`:31,69-70,278-284`), plus no-replace file and directory variants (`:114-230`). (4) `worktree_identity.rs:233-251` uses `NamedTempFile`, reopens it by path for `write_file_synced`, then `persist` and a path-based `sync_directory`. (5) `serve.rs:84-91`. (6) The record put, `redo_apply.rs:233-247` (uuid stage name, synced). |
| C9 | Server PID file written without fsync | valid-and-open | `serve.rs:84-91`: `fs::write` then `fs::rename`, with no file or directory sync. |
| C10 | Claims index uses fixed temp name `claims.index.tmp` | valid-and-open, and worse than stated | `claim_coordinator.rs:873-874`. `jit recover` calls `write_index_atomic` without the claims lock (`commands/claim.rs:689,697`), so it can race a locked lease operation on the same temp file. f8a6b298 removes both (see §7). |
| C11 | Two PID-liveness primitives with different non-unix policies | valid-and-open | `serve.rs:118-138` (`is_process_alive`, non-unix returns `false`) and `storage/lock_cleanup.rs:15-44` (`process_exists`, non-unix returns `true`). Both read `kill(pid, 0).is_ok()`, so a live process owned by another user (`EPERM`) reads as dead. This latent bug lets `cleanup_stale_locks` (`lock_cleanup.rs:112`) remove a live foreign lock. |
| REQ-14 | Open each distinct parent once per record | valid-and-open | C1, C2. |
| REQ-15 | Coalesced directory fsync, conditional on a proof | valid-and-open; the proof does not exist | Existing interruption tests: `redo_conformance.rs:282-482` (matrix on both backends) and the `redo_apply.rs` tests. They interrupt in-process, so every completed rename stays visible in the page cache. They pass with or without any directory fsync and cannot prove the coalescing safe. See §4. |
| REQ-16 | One no-follow module, no cycle, leaf-open once, commit through capabilities | valid-and-open | C4–C7. |
| REQ-17 | One publication function with a durability argument; claims index, worktree identity and PID file through it; unique temp names; PID synced | valid-and-open, target shifts | C8–C10. After f8a6b298, `claims.index.json` no longer exists; see §5 D4. |
| REQ-18 (general imports) | Domain imports nothing from `repository_state`; typed errors | valid-and-open | `domain/types.rs:13` imports `RootRelativePath` for `ProfileOrigin::Directory` (`:1171`). `anyhow` appears only in `domain/types.rs:14,156,181,216` (`FromStr` for `State`, `Priority`, `ContentFormat`). `artifact_classifier.rs:783-897` (`ProducerError`) belongs to the archive investigator. |
| REQ-20 liveness | One guarded primitive, one non-unix policy | valid-and-open | C11. Consumers: `lock_cleanup.rs:112`; `serve.rs:538,661,704`; `main.rs:7043,7049`. |
| REQ-20 no-follow reads | Through the shared read primitive | valid-and-open | Hand-rolled walks: `read_exact_store` (`json.rs:75-165`), `read_gate_runs` (`:957-1043`), `load_gate_run_result` (`:1333-1410`). The gate-run pair is already planned by f8a6b298 `gate-run-shared-reader` (0851fae4), whose REQ-01 names `read_repository_file_nofollow`. `read_exact_store` remains for b697a3f7. |
| REQ-20 ancestor walking | One walker | valid-and-open | Canonical `VirtualPath::ancestor_directories` (`repository_state/path.rs:383-397`). Copies: `initialize.rs:1201-1215` (`ancestor_dirs`), `archive.rs:369-389` (loop inside `create_ancestors`), and `storage/memory.rs:407-420` (`ensure_ancestor_dirs`, which also silently drops a parse failure). Present-or-create matches repeat the same action at `initialize.rs:1161-1188`, `archive.rs:390-405` and `mutation.rs:1037`. |
| REQ-20 initialization errors | Keep their types | valid-and-open | `InitializationError::RuleMaterialization(String)` (`initialize.rs:144`) is filled with `to_string()` at `:479,487,499,697,699,701,736,894,1144` and by `rule_materialization_error` (`:1149-1150`). `Config(String)` (`:126`) is filled at `:229,241,243,245`. Exit codes do not change: `output/failure.rs:125-140` classifies on the outer `RepositoryStateError::Initialization` before any chain scan. |

Counts: already-done 0; valid-and-open 20 (C1–C11 and nine REQ rows); invalid-as-stated 0. Two qualifications are in §5: REQ-17's "claims index" names a file that f8a6b298 deletes, and REQ-15's crash safety can be shown only under a crash model, not observed on a real filesystem.

### 2. Prior-art sweep

- **Charter D-17** (`dev/vision/9db27a3a-charter.md:302-`) puts "machine-local control files (worktree identity, server pid and log, claims)" outside the lock and the record. REQ-17's sites therefore form one class, separate from repository publication. D-18 favours one publication core.
- **f1dc843a plan** (`dev/archive/f1dc843a-redo-mutations/active/plan.md:140-175`): `redo-record-shape` (OD-7, idempotent re-application) and `record-commit-point` (the record is synced with its directory before the first live write; the extension avoids the `.tmp` sweep). REQ-03 there: the crash matrix is proven once against the mechanism on both backends (`redo_conformance.rs`).
- **Architecture note** `dev/architecture/repository-state-materialization.md:349-376` states "rename it into place, and sync the parent" (`:368-369`). REQ-15 makes this text stale. `docs/concepts/guarantees.md:69-82` and `docs/reference/storage-format.md:88-110` describe only the record's commit sync and stay valid.
- **Init publisher** (`storage/init_publish.rs:51-87,131-143`) already uses the coalesced pattern: it stages a tree, runs `sync_tree` once, then renames. It also pays redundant per-ancestor syncs through `open_or_create_capability_parent` (`:62,68`) before `sync_tree` repeats them.
- **Profile's opener** (`profile/nofollow.rs:1-38`) is a documented single opener with `O_NONBLOCK`, so opening a FIFO cannot hang. With non-blocking options, storage's leaf-open can drop its leading `lstat` safely.
- **Invariants:** `@/inv/atomic-writes` and `@/inv/derived-state-coherence` (`.jit/invariants.toml:42-49`) require only that a record "the next session completes" exists. Neither requires per-operation directory durability.
- **Audit rows** (findings-tables 15, 19, 20, 33, storage-json 19, repo-state 20) match §1, with stale line numbers.

### 3. Consumer sweep

| API / file being changed | Consumers (file: count) |
|---|---|
| `open_child_dir_nofollow` | `storage/json.rs` 7, `repository_state_store.rs` 6, `artifact_planning.rs` 4, `external_publish.rs` 3, `init_publish.rs` 3, `redo_apply.rs` 2 |
| `open_absolute_dir_nofollow` | `json.rs` 4, `repository_state_store.rs` 4, `external_publish.rs` 3, `artifact_planning.rs` 2, `test_support.rs` 1 |
| `open_regular_file_nofollow` | `json.rs` 5, `artifact_planning.rs` 2, `repository_state_store_tests.rs` 2, `repository_state_store.rs` 1 |
| `open_capability_parent` / `open_or_create_capability_parent` | `redo_apply.rs` 4 / 3, `repository_state_store.rs` 3 / 0, `init_publish.rs` 0 / 3 |
| `read_repository_file_nofollow` / `read_repository_file_confined` | `commands/snapshot.rs` 2 / `json.rs` 2 |
| `capability_dir_identity` | `external_publish.rs` 2, `repository_state_store.rs` 1 |
| `CapabilityRoots` | `repository_state_store.rs` 13, `redo_apply.rs` 11, `redo_conformance.rs` 2, `repository_state_store_tests.rs` 2, `tests/scratch_build/repository_state_cutover_tests.rs` 2 (marker `"CapabilityRoots"`, `:733`) |
| `RepositoryStateStoreError` (pattern-matched where primitives move) | `json.rs` 9 (matches `::Io` NotFound at `:76-83,958-965,1341-1348`), `init_publish.rs` 10, `redo_apply.rs` 2, `lock.rs` 4, `commands/mod.rs` 16, `main.rs` 4, plus others unaffected |
| `RedoPoint` | `redo_apply.rs` 29, `redo_conformance.rs` 23, `memory.rs` 18, `repository_state_store_tests.rs` 6, `mutation_interruption.rs` 6, `commands/profile.rs` 2, `commands/gate_check.rs` 2, `tests/cli_repo_workflow/profile_publication_tests.rs` 2, `profile_concurrency_tests.rs` 2 |
| `RedoRecord::commit` | `repository_state_store.rs` 2 (`:614,659`); tests: `redo_record.rs` 7, `redo_apply.rs` 2, `redo_conformance.rs` 1 |
| `write_file_atomic` / `write_file_synced` / `create_file_synced` / path `sync_directory` | `user_config_store.rs` 2 / `worktree_identity.rs` 2 / `redo_record.rs` 2 / `redo_record.rs` 3 + `worktree_identity.rs` 2. All lose their callers after S2+S6. |
| `sync_capability_directory` | `redo_apply.rs` 5, `init_publish.rs` 3, `repository_state_store.rs` 2 |
| `write_index_atomic` | `claim_coordinator.rs` 9 production + 2 test, `commands/claim.rs` 2. All are deleted by f8a6b298 53d19851. |
| `write_identity_atomic` | `worktree_identity.rs` 2 (`:163,178`) |
| `write_pid_file` / `pid_file_path` | `serve.rs` (writer `:84`, tests `:767,1461`); `pid_file_path(` is the classifier marker in the cutover scan (`repository_state_cutover_tests.rs:681`) |
| `is_process_alive` / `process_exists` | `serve.rs` 3 production + 6 test, `main.rs` 2 / `lock_cleanup.rs` 1 production + 5 test |
| `ancestor_dirs`, `create_ancestors` loop, `ensure_ancestor_dirs` | `initialize.rs` 3 / `archive.rs` 2 / `memory.rs` 5 |
| `InitializationError::{RuleMaterialization,Config}` | `initialize.rs` only; `output/failure.rs:822,904` (test names `UnexpectedOccupant` only) |
| `RootRelativePath` (if moved to domain) | 23 files: `repository_state/path.rs` 44, `export.rs` 12, `commands/profile.rs` 10, `commands/snapshot.rs` 8, `profile_apply.rs` 6, `package_tree.rs` 5, `repository_state_store.rs` 4, `mutation.rs` 4, `domain/types.rs` 3, `memory.rs` 3, `artifact_planning.rs` 3, `materialize.rs` 3, `initialize.rs` 3, `profile/application.rs` 3, `gate_execution.rs` 3, `redo_record.rs` 2, cutover tests 2, and 1 each in `json.rs`, `repository_state/mod.rs`, `image.rs`, `archive.rs`, `commands/init.rs`, `commands/archive.rs` |
| Docs and prose | `dev/architecture/repository-state-materialization.md:368-369` (REQ-15); `docs/reference/storage-format.md:48` names `server.pid.json` (unchanged); `storage/atomic_write.rs:1-17` and `external_publish.rs:1-19` module docs (REQ-17) |
| `mcp-server`, `web`, `scripts`, `test-vectors` | No consumers of these primitives (grep: no hits). |

**Structural guards these steps must keep green** (`tests/scratch_build/repository_state_cutover_tests.rs`):

- `REPOSITORY_STATE_PUBLISHERS` (`:633-655`) admits whole modules by definition: `pub struct RedoRecord {`, `pub struct RedoApplier {` and `fn publish_staged_data_root(`. A writing function moved into a non-publisher module, such as a new `capability_fs.rs`, is admitted only through classified callers. Moving `commit` there would flag its session caller, so the commit belongs in `redo_apply.rs`.
- `NON_REPOSITORY_WRITES` markers (`:659-700`) classify control-file writers by text in the writing function: `shared_jit`, `pid_file_path(`, `WorktreeIdentity` and `UserConfigRoot`. Callers of a shared `publish_file` keep them; the helper is classified through its callers (`:1132`).
- `IN_ROOT_DIRECTORIES` (`:731-742`) needs `CapabilityRoots` or `RECORD_SCRATCH_DIR` in a directory-creating function.
- `test_profile_package_reads_build_no_follow_options_in_one_place` (`:1576-1584`) pins profile's opener to `profile/nofollow.rs`.

### 4. Primitive verification

- **Interruption safety of coalesced fsyncs.** Not delivered by any existing test. Every test interrupts the pass in-process, so every completed rename and unlink remains visible: those tests reach only states where a prefix of operations has run. A crash that loses an unsynced directory entry can leave any subset of the non-durable operations persisted. That covers renames, unlinks, created directories and created ancestors, and it is the state coalescing exposes. The property needed and its proof design:
  1. **Model.** File data is durable after `sync_all`. A directory entry is durable after its directory's `fsync` returns. Any subset of entries not yet synced may persist. The record is durable before the first live write (`redo_record.rs:208-216`) and removed only after the pass.
  2. **Ordering, by construction.** Every distinct parent touched is fsynced, including the parent of each created ancestor, the event log's directory and each made directory's parent, and all those fsyncs return before `check(BeforeRecordRemoval)` and the record unlink. A crash therefore leaves either the record (replay) or every operation durable.
  3. **Replay from an arbitrary subset.** New property `prop_replay_converges_from_any_subset_of_persisted_operations` in `redo_conformance.rs`, run on both backends. For a generated record and initial state, for every subset S of its operations (up to 7 operations, at most 128 subsets; cap cases), seed the state, apply the sub-record of S uninterrupted (the persisted state), optionally create the ancestors of puts outside S (an entry persisted without its child), and seed event-log residue. Then commit the full record, apply it uninterrupted, and assert `published()` equals the clean-application result. The existing `snapshot` comparison must ignore `.redo.*.tmp` stages left by a lost rename; the sweep test (`redo_apply.rs:917-925`) owns their reclamation.
  4. The proof holds because record operations are pairwise path-disjoint (`RedoRecord::new`/`parse`, `redo_record.rs:116-130,413-428`) and each is idempotent (make-dir tolerates a dir, put overwrites via stage and rename, delete tolerates absence, append seeks and truncates to offset plus length, `redo_apply.rs:213-321`). They commute, so any subset followed by the full record converges.

  This is a proof under the model; no test can observe real power loss. Staged-file `sync_all` (`:268`) and event-log `sync_all` (`:320`) must stay per operation: a rename that persists must name synced bytes.
- **Distinct parents opened once (REQ-14).** Needs a per-record handle map keyed by `(RepositoryRootClass, parent RootRelativePath)`, built in the pre-pass (preserving "refuse a symlinked ancestor before any live write", owned by `redo_apply.rs:1034-1064`) and extended when an operation creates an absent ancestor. A shared-prefix walk opens `a`, `a/b` and `a/b/c` once each. Recommended observation: the map itself counts the opens it performs (per instance, so no process-wide counter and no conflict with `@/inv/orthogonal-tests`). A unit test asserts that the count equals the number of distinct ancestor directories of the record's targets. Dropping the leading `lstat` in `open_child_dir_nofollow` (`repository_state_store.rs:1610`) is safe only with non-blocking open options (profile's precedent), because `fstat` (`:1621`) then refuses non-directories.
- **Unique temp names per writer.**
  - uuid, unique: redo `staged_name` (`redo_apply.rs:245-247`, owned by `test_staged_name_is_unique_across_calls` `:905`).
  - Unique within a process by counter, but two independent counters, `atomic_write.rs:27` and `external_publish.rs:31`, produce one name format: `atomic_write.rs:155` and `external_publish.rs:70,278-284`.
  - `NamedTempFile`, random: `worktree_identity.rs:242`.
  - Fixed, not unique: `claim_coordinator.rs:874` (`claims.index.tmp`) and `serve.rs:87` (`server.pid.pid.tmp`, from `with_extension("pid.tmp")` on `server.pid.json`).

  One generator (uuid, as `staged_name`) satisfies REQ-17.
- **PID file synced.** Not delivered (C9).
- **Record commit through capabilities.** Not delivered (C7). The session already holds `roots.data` (`repository_state_store.rs:313-316`). The absent-root path needs the stage `Dir`, which `stage_new_data_root` opens (`init_publish.rs:58`) but does not return. Mode adoption can use `Dir::dir_metadata` and `Dir::set_permissions` instead of `std::fs` (`redo_record.rs:357-366`).
- **Domain purity.** `repository_state` is imported at `domain/types.rs:13` and in `artifact_classifier` (other investigator). `anyhow` is used in `domain/types.rs` only. Domain also imports `config`, `declarations`, `document`, `profile`, `errors` and `labels` (`domain/types.rs:6,8,929,1238-1248`; `domain/item.rs:42-45`; `domain/projection.rs:46`). REQ-18 does not name these; 2195c98f 5c232305 moves `ProfileId`.
- **Liveness.** No guarded primitive treats `EPERM` as alive (C11). The pid-safety guard (`i32::try_from` plus positive check) is written four times: `serve.rs:126-131`, `lock_cleanup.rs:27-30`, `serve.rs:677-683` (SIGTERM) and `gate_execution.rs:343-349` (group kill).

### 5. Design options

**D1. Module for no-follow primitives (REQ-16).**
- (a) **`storage/capability_fs.rs`.** It owns `CapabilityRoots`, one parent walk with a create/no-create mode returning the per-record handle map, `open_child_dir_nofollow`, `open_absolute_dir_nofollow`, the single leaf-open (non-blocking options), both repository-file readers rebuilt on the leaf-open, identity helpers, `sync_capability_directory` and `write_staged_file`. It has its own `CapabilityFsError { UnsafeTarget, Io }`, with `From` into `RepositoryStateStoreError` and `RedoApplyError`. `redo_apply` and the session both depend on it, and `redo_apply` imports nothing from the session.
- (b) Move the primitives into `redo_apply.rs`. This puts reads (`json.rs`, `snapshot.rs`) behind the applier module.
- (c) Keep the primitives in the session module and move only `CapabilityRoots`. This leaves the cycle.
- **Recommend (a).**

Profile's opener stays as a cited exception unless the steward wants profile to import storage (see Needs decision).

**D2. Record commit (REQ-16).**
- (a) **`commit_record(data: &Dir, record) -> Result<()>` in `redo_apply.rs`**, beside `RedoApplier`, so the publisher module set is unchanged and `redo_record.rs` imports nothing from `atomic_write`.
- (b) A method on `CapabilityRoots`, which fails the cutover writer scan (see §3).
- **Recommend (a).** `create_file_synced` and the path-based `sync_directory` then lose their last callers; delete them.

**D3. One publication function (REQ-17).**
- (a) Path-based `publish_file(path, bytes, Durability)` in `atomic_write.rs`, as the audit proposes. Redo puts and external exports keep their own stage-and-rename.
- (b) **Capability core plus thin path entry.** `publish_in(parent: &Dir, leaf, bytes, mode, Durability)` holds the uuid stage name, `create_new` no-follow write, optional file sync, rename and optional directory sync. `publish_file(path, ..)` opens the parent and calls it. Redo `put_file` calls the core with `Durability::Deferred` (directory sync coalesced per REQ-15). `write_external_export_atomic` calls the core after its parent revalidation.
- (c) As (b), with external publication left separate because of its parent-identity revalidation.
- **Recommend (b).** Durability values: `Full` (file plus directory), `Deferred` (file only; the caller syncs the directory) and `None` (no current user; omit it unless user config keeps its unsynced behaviour). Caveat: the path entry must open the parent *following* symlinks, because user config (`~/.config` is often a dotfile symlink) and `.git/jit` paths are not repository roots. `open_absolute_dir_nofollow` would refuse them.

**D4. What "claims index" means after f8a6b298.**
- (a) **Map it to the lease records** `.git/jit/leases/<issue>.json` and `.git/jit/claims.seq`, which `LeaseLock::apply` publishes (f8a6b298 plan `:56,64`; 2f563673 REQ-02: synced temp, rename, directory sync). Add a b697a3f7 step that routes them through `publish_file` after f8a6b298's 2f563673 and 53d19851. This edge runs from our step to theirs, so f8a6b298 is not delayed.
- (b) Edge from 2f563673 onto our publication step, so f8a6b298 writes lease files through `publish_file` from the start. This delays f8a6b298; flag it for the steward.
- **Recommend (a)**, with the REQ-17 wording amended at breakdown to "lease records" (escalate).

**D5. Liveness primitive (REQ-20).**
- (a) **Keep `storage/lock_cleanup::process_exists` as the one primitive.** Delete `serve::is_process_alive`; serve already depends on storage. Treat `EPERM` as alive. Non-unix policy: alive (the conservative choice: a live lock is never deleted and a duplicate server is never started). Platform behaviour is untested, since CI is Linux-only (`.github/workflows/*`: all `ubuntu-latest`).
- (b) A tri-state `Liveness { Alive, Dead, Unknown }`, with each caller mapping `Unknown`. That is two policies, against the REQ.
- (c) A crate-level `process` module that also owns a `signal_target(u32) -> Option<Pid>` guard, also used by `serve.rs:677` and `gate_execution.rs:343`. This makes `@/inv/pid-safety` structural but goes beyond the REQ.
- **Recommend (a)**, with (c)'s shared conversion if the steward wants pid-safety enforced once.

**D6. `RootRelativePath` out of `repository_state` (REQ-18).**
- (a) **Move the type and its parse error into the domain** (for example next to `domain::identity` from 2195c98f), and update 23 importing files with no re-export (`@/inv/canonical-cutover`).
- (b) A domain-local wrapper, which duplicates the validator against REQ-20's "one safe-relative-path validator".
- **Recommend (a).** Also coordinate with the REQ-20 safe-path item, which merges `domain/repository_inputs.rs:454` into the same type and belongs to another investigator.

### 6. Architecture fit / invariant check

- `@/inv/architecture-dependency-direction`: D1(a) and D2(a) remove the session/redo cycle. The session↔memory cycle (C5) remains; name it as known residue or add a step that moves `ActiveLayoutTracker` into the session-neutral module.
- `@/inv/atomic-writes`: REQ-15 coalescing is consistent with the invariant text, which requires a replayable record and no per-operation durability, once the ordering rule (§4) holds. Update the module rustdoc (`redo_apply.rs:1-8`) and the architecture note (`:368-369`) in the same change.
- `@/inv/orthogonal-tests`: the subset property is the mechanism's owning test in `redo_conformance.rs`. The REQ-14 open count is per instance; no global counter. Consolidate the duplicated liveness tests (`serve.rs:814-830`, `lock_cleanup.rs:150-175`) into the primitive's single set.
- `@/inv/convention-convergence`: `profile/nofollow.rs` is a second no-follow opener. Converge it or cite it as an exception.
- `@/inv/essential-code-prose`: REQ-15's fallback ("rustdoc states why") applies only if the proof is rejected.
- Layer: `publish_file` and the liveness primitive stay in storage; commands (`serve.rs`) call storage. That is allowed and removes `serve.rs`'s direct file writes, which `atomic_write.rs:8-10` prohibits.

### 7. Sequencing

Each step compiles, passes clippy and leaves tests green.

| Step | Change | Files | Requires first |
|---|---|---|---|
| S1 | Extract `storage/capability_fs.rs` (D1a) with `CapabilityFsError`; one parent walk; single non-blocking leaf-open; readers rebuilt on it; `CapabilityRoots` moved. No behaviour change. | `storage/{capability_fs (new), repository_state_store, redo_apply, init_publish, json, external_publish, test_support, mod}.rs`, `artifact_planning.rs` unless REQ-06 deleted it first, `repository_state_store_tests.rs`, `redo_conformance.rs` | 842b0ac6 `remove-storage-cfg-twins` (ec190bd5, ready); edge from S1 onto it, cheap. No 2195c98f key. |
| S2 | Record commit through capabilities (D2a); `init_publish::stage_new_data_root` returns the stage `Dir`; delete `create_file_synced` and path `sync_directory` if unused. | `redo_apply.rs`, `redo_record.rs`, `repository_state_store.rs`, `init_publish.rs`, `atomic_write.rs`, tests in `redo_record.rs`/`redo_apply.rs`/`redo_conformance.rs` | S1 |
| S3 | REQ-14 per-record parent-handle map with per-instance open count and test. | `redo_apply.rs`, `capability_fs.rs` | S1 |
| S4 | REQ-15 proof: the subset-crash property (§4) on both backends. | `redo_conformance.rs`, `redo_apply.rs` test fixtures, `memory.rs` test fixtures | S3 |
| S5 | REQ-15: coalesce directory syncs per record before `BeforeRecordRemoval`; no-sync walk mode for redo (and init, which `sync_tree` covers); update rustdoc and the architecture note. If S4 is rejected, S5 instead adds the rustdoc stating why per-operation syncs remain. | `redo_apply.rs`, `capability_fs.rs`, `init_publish.rs`, `dev/architecture/repository-state-materialization.md` | S4 |
| S6a | REQ-17 core: `publish_in` / `publish_file` with `Durability` and uuid stage names (D3b). Convert redo `put_file`, external export, user config, worktree identity and the PID file (synced). Delete `write_file_atomic*`, `write_file_synced` and the external `COUNTER`. | `atomic_write.rs` or `capability_fs.rs`, `redo_apply.rs`, `external_publish.rs`, `user_config_store.rs`, `worktree_identity.rs`, `commands/serve.rs`; cutover test only if markers move | S1, S5 (`Deferred` for redo puts). 2195c98f: `lease-worktree-ids` (8cfbf19d), `control-file-path-constants` (5491edd0), `well-known-paths-storage` (e1a6c8c2). Epic edge 2195c98f→b697a3f7 covers them. |
| S6b | REQ-17 lease records and `claims.seq` through `publish_file` (D4a). | `storage/claim_coordinator.rs` | S6a; f8a6b298 `lease-per-issue-records` (2f563673) and `lease-index-machinery-removal` (53d19851). Cross-epic edge from S6b onto them; does not delay f8a6b298. |
| S7 | REQ-20 liveness (D5a): one primitive, `EPERM` alive, non-unix alive; delete `serve::is_process_alive`; one test set. | `storage/lock_cleanup.rs`, `commands/serve.rs`, `main.rs` | None. Merge seam with df129d9e d4373d07 / 8c46fc85 (move the `main.rs:7043-7049` foreground code into `serve.rs`); recommend no edge. If the steward wants ordering, S7 waits on d4373d07, which delays only us. |
| S8 | REQ-20 no-follow reads: `read_exact_store` through the shared reader. | `storage/json.rs` | S1; f8a6b298 `gate-run-shared-reader` (0851fae4) owns `read_gate_runs`/`load_gate_run_result`. Edge from S8 onto 0851fae4 so the REQ-20 item completes; does not delay f8a6b298. 2195c98f `stage-storage-issue-ids` (89e7a273), via the epic edge. |
| S9 | REQ-20 ancestor walking: `ancestor_dirs`, the `create_ancestors` loop and `ensure_ancestor_dirs` replaced by `VirtualPath::ancestor_directories`, with one missing-directory-action helper. | `repository_state/{initialize, archive, path, mutation}.rs`, `storage/memory.rs` | Coordinate with the archive investigator's REQ-06/07 steps on `repository_state/archive.rs`. No 2195c98f key. |
| S10 | REQ-20 initialization errors typed: `RuleMaterialization(#[source] Box<RepositoryStateError>)` and typed `Config` sources. | `repository_state/initialize.rs` | None. |
| S11 | REQ-18 general: `RootRelativePath` and its error into the domain (D6a); 23 importers. `domain/types.rs` `FromStr` errors typed. | `domain/types.rs`, `repository_state/path.rs`, the 23 files in §3 | 2195c98f `issue-id-types` (3e353abd, creates `domain::identity`), `wire-vocabulary-macro` (3f026bb1), `lifecycle-state-vocabulary` (2ac50581), `priority-vocabulary` (182350ce), `content-format-vocabulary` (42bcaa6e). Those keys re-declare `State`/`Priority`/`ContentFormat`; if the macro's `FromStr::Err` is typed, `anyhow` leaves the domain with no b697a3f7 work. Also follow 5c232305 (ProfileId into the domain), which touches `domain/types.rs:1248`. |

**f8a6b298 / f4d9502a ordering on the shared code, exactly:**

1. **f8a6b298 `committed-length-append` (c2930451).** It changes `repository_state/mutation.rs` only and cites `redo_apply.rs:302-320` as background. No textual overlap with S3/S5, so no edge.
2. **f8a6b298 `redo-record-refuses-log-rewrite` (b32a720e).** It edits `RedoRecord::new`; S2 moves `commit` out of the same file. This is a merge seam only, so no edge.
3. **f8a6b298 `lease-lock-handle` (a085ca6a) → … → `lease-per-issue-records` (2f563673) → `lease-index-machinery-removal` (53d19851).** These rewrite and then delete `write_index_atomic`. S6a must not touch `claim_coordinator.rs`. S6b follows 53d19851.
4. **f8a6b298 `lease-interruption-matrix` (7244ecb2)** models its seam on `RedoInterruptionInjector`. S3–S5 leave `RedoPoint` unchanged, so no edge.
5. **f4d9502a `init-identity-head-lookup` (732f7afb)** edits `commands/mod.rs:3338-3374` (`initialize_worktree_identity`), not `worktree_identity.rs::write_identity_atomic`. f4d9502a `claims-head-lookup` (f6eaba42) edits `claim_coordinator.rs` branch lookup (`:974-993`). Neither overlaps S6a, so no edge. The textual neighbours of S6a's worktree-identity change are 2195c98f 8cfbf19d and 5491edd0, covered by the epic edge.
6. **Optional (flag; delays f8a6b298):** 2f563673 → S6a, so that lease files are born on `publish_file`.

Other in-flight seams:
- 842b0ac6 `remove-storage-cfg-twins` (ec190bd5, ready): S1 after it.
- 842b0ac6 `remove-linked-worktree-evidence` (f11514dd), `converge-memory-event-parsing` (47a082ef) and `single-file-lock-poll-loop` (8397eb74) are done; their effects are visible in the current code.
- df129d9e d4373d07 / 8c46fc85 (serve): S7 merge seam.
- f8a6b298 0851fae4: S8 edge.

