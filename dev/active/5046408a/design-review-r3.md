# 5046408a independent design review, revision 3

Verdict: **PASS — design review.** Both revision-2 findings are closed. No unresolved design finding remains from this review. Steward acceptance of the exact footprint is still required before implementation, as the design explicitly states.

Reviewed `dev/active/5046408a/design.md` in the assigned isolated worktree and read the authoritative charter from `main` at `59a4306a1ee466be5164964baab4756c9d85596a`. D-26 is at `dev/vision/9db27a3a-charter.md:480`; D-27 is at `:502`. This review makes no claim that the design has been implemented or tested.

## Finding closures

1. **R2 finding 1 — pending-record policy source/order: CLOSED.**

   `design.md:39` now explicitly applies D-27: mutation/recovery startup completes the already committed record under D-26, preserving its original audit bytes without another override event. Only then does coherent recovered configuration plus the explicit invocation override govern new work, before its mutation session opens. Clean-store refusal still precedes mutation effects; pending-store refusal may follow only completion of the older commit. The declaration-postimage overlay and partial-config policy read are expressly excluded. This is the authoritative policy exception R2 required, rather than an inferred bypass.

   `design.md:81` owns both allow→refuse and refuse→allow transitions, partial config application, explicit override, and original audit-byte preservation. The CLI boundary at `:88` checks clean refusal and recovery-before-refusal without duplicating the storage race matrix.

2. **R2 finding 2 — incomplete exact footprint: CLOSED.**

   `design.md:67` explicitly includes `storage/memory.rs` and shared structural/binding validation with backend-specific capability checks. `:90` includes that file, the existing worktree-record fixture in `cli_issue/recover_command_tests.rs`, and the owning `cli_repo_workflow/linked_checkout_write_policy_tests.rs` boundary. The data-only construction form preserves the existing lease-enforcement call site; any unavoidable change there requires steward footprint acceptance. Existing suites and fixtures remain the owners, with no new Cargo test target.

## Earlier findings retained closed

| Original finding | Status and evidence |
| --- | --- |
| Missing durable original-root authority | **Closed.** D-26 and `design.md:37–39` require one typed absolute no-follow root/identity binding in the same record, exactly for worktree-bearing operations; ordinary and absent-root stage publication derive it from their committing session. Unsupported wire versions fail explicitly. |
| Configuration-to-layout admission | **Closed at design level.** `:35` retains the selection observation across consumer capture, revalidates clean mutation selection under locks, and routes newly appearing records back through recovery instead of applying them under a candidate layout. D-27 closes the previously missing policy branch. Claim-first reopen and retained sessions share the handoff. |
| Data/code/document/projected authority | **Closed.** `:45–56` separates code checker/evidence/drift/document authority from data history and data/relocated coordination; projected primary reads preserve code-root meaning or use data-only access rather than a fake overlapping layout. |
| Removed committed-HEAD issue fallback | **Closed.** `:56` preserves local/primary storage fallback and expressly excludes reintroducing a HEAD issue tier. |
| Symlink canonicalization ambiguity | **Closed.** `:31` rejects symlinks throughout ancestry using no-follow evidence; source-aware errors and pure containment checks remain intact. Git observation alias reuse does not alter storage-root policy. |

## Coherence assessment

The proposed boundaries are now consistent:

- **Read:** no-follow data declaration capture through the existing parser; carry its record/event-tail/root-identity observation into admission of the actual consumer capture; bounded retryable conflict on movement or a pending record. No writer locks or recovery, preserving D-25.
- **Pending mutation/recovery:** tentative record observation selects locks only. For worktree-bearing records, acquire the recorded worktree bootstrap lock before data/repository/events locks, then reopen and validate the record and roots. A changed observation releases guards and restarts without acquiring a second worktree lock under data locks. Data-only records need no fabricated worktree capability. Original-root validation precedes replay writes, including data puts; missing/moved/replaced/symlinked roots preserve the record and yield typed recovery failures.
- **New work after recovery:** release recovery guards; select from recovered config and invocation inputs; evaluate coherent linked-data policy; acquire the new session in normal lock order and revalidate the selection. A new pending record returns to the recovery path. A retained session is never silently rebound. Recovery does not create authorization or audit bytes for new work.

D-26 and D-27 therefore remove both the original target ambiguity and the config/policy/recovery cycle without adding a second durable authority, configuration parser, or declaration overlay. Storage remains the shared owner of capability checks, admission, retry and replay. The implementation review must verify the stated handoff also holds for server operations and claim-first reopening; these are existing planned obligations, not new scope.

The authority inventory preserves the steward's data-history guard ruling and non-Git boundaries. Claims/leases use the data repository's common directory when present; independently selected code cannot replace it. Documents use code consistently, worktree identities/output use coordination consistently, and projected data reads do not treat data checkout roots as code roots.

The process bounds at `design.md:62` remain sound: one ordinary probe; at most two external-bootstrap probes; external traversal $N+1$; one probe per distinct configured context, including a possible independent relocated coordination context. Failed and changed contexts count; exact/canonical/top-level observations are reused. HEAD/evidence operations remain separate from discovery budgets.

REQ-01–08 remain covered by the settings/precedence/default behavior, nested/separate-store opening, code checker/evidence, data coordination, typed invalid-root errors, and canonical storage-format/deployment documentation. The test matrix assigns representation, replay/crash, capability, admission, policy and consumer properties to their existing owning layers; environment tests remain explicit-input or child-only. Required Rust checks and configured gates remain implementation obligations.

## Next action

Steward accepts the revised exact footprint, then implementation may proceed under this design and D-26/D-27. No further design decision, user approval, manifest change, or unrelated Git-policy work is requested by this review.

Only this review artifact was written. No source/design/tracker edits, commits, builds or gates were performed.
