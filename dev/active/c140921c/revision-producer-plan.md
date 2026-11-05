# Shared commit revision resolution

**Issue:** c140921c  
**Date:** 2026-11-05  
**Status:** Conditionally approved design; implementation blocked on explicit Git-floor approval and a273 reaching Done and landing.

## Problem Statement

Snapshot source selection must use `GitCheckout::resolve_commit` under @/issue/c140921c/requirement/REQ-01. That producer currently appends `^{commit}` to the original expression. For Git message-search syntax, this changes the expression before Git resolves it. The previous libgit2 behavior resolved the original expression and then peeled the resulting object. Snapshot must retain that behavior through the shared producer, without a private resolver (@/inv/convention-convergence, @/charter/D-20).

The differential investigation exercised Git 2.39.5, Git 2.56.0 and the installed libgit2 1.9.7 C API. Both `:/unique needle` and a literal-LF `:/unique\nbody` selected the intended earlier commit with resolve-then-peel; the current suffix-first command failed with `fatal: Needed a single revision`. A one-process `cat-file --batch-check -Z` resolve-then-peel experiment matched the oracle across twenty cases. The installed oracle is not the exact vendored libgit2 1.9.4 dependency; the earlier a273 Rust oracle independently established the same message-search behavior.

Session evidence is recorded in `/tmp/f4d9502a-lead/c140-revision-producer-investigation.md`, with raw inputs and results in `c140-revision-research/probe.json` and `c140-revision-research/batch-candidate-probe.json` beneath that directory. The earlier oracle is `path-revision-audit-result.md`. These are investigation artifacts, not durable acceptance tests. Git documents [message searches and peeling as distinct revision operations](https://git-scm.com/docs/gitrevisions); libgit2 documents [single-object resolution](https://libgit2.org/docs/reference/main/revparse/git_revparse_single.html).

## Success Criteria

The authoritative c140 criteria remain @/issue/c140921c/requirement/REQ-01, @/issue/c140921c/requirement/REQ-02, @/issue/c140921c/requirement/REQ-03 and @/issue/c140921c/requirement/REQ-04. They are cited rather than copied, following @/inv/single-source-prose. This plan does not amend their tracker source.

The conditionally approved producer correction additionally requires:

- Unary, bulk and the a273 reader use one original-expression-then-peel operation in the Git module.
- Initial absence and an existing object that cannot become a commit remain typed, distinct outcomes. Adapter mappings preserve the contracts below.
- Recognized ambiguity affects only that input in bulk resolution; valid results remain available. Malformed protocol and process failures remain errors.
- Process budgets remain unchanged, including a273's one-child read. Metadata resolution does not allocate arbitrary object bodies.
- The producer owns the regression matrix, while snapshot adds one wiring case. Existing SHA-256, framing and lifecycle coverage remains active.

## Design

### Authorization boundary

Implement this option only after the user explicitly approves the Git >=2.42/runtime proposal and a273 is landed and Done. The frozen six-file a273 decision patch remains unchanged. This document authorizes no floor, runtime, source, issue or manifest mutation. If the user declines that prerequisite, return the design decision to the lead rather than substituting another transport or increasing process counts.

### One session operation

Use the existing `CatFileProcess` response parser and lifecycle for a shared typed commit-resolution operation. Unary `GitCheckout::resolve_commit` and bulk `resolve_commits` use its Types mode. The existing `GitObjectReader` exposes the same operation through its Contents session, allowing a273's path reader to delegate without starting another child.

For each distinct input, send the original expression unchanged. A Commit response already establishes the canonical commit identity. For another existing object type, request `<canonical-object-id>^{commit}` in the same session, and require a Commit response before constructing `CanonicalCommitOid`. Never append peeling syntax to the original expression. Annotated and nested tags use Git's canonical-ID peeling semantics. Missing, invalid and ambiguous inputs do not trigger a different resolver or another process.

Expressions are stdin data, not argv options. Reject NUL before writing; approved `-Z` framing supports LF expressions. Validate canonical SHA-1 or SHA-256 identities before constructing peel requests. Retire the old suffix-first bulk path and per-name subprocess fallback. Do not add revision grammar parsing, a version probe, a cache or a private consumer resolver.

Both modes reuse one header/status parser. Contents mode uses the shared body consumer to drain exactly the declared object length and required separator when resolution needs metadata only. It retains no arbitrary blob payload; memory for draining is bounded independently of object size. Ordinary object reads still collect bytes when their caller requests them. Preserve stderr draining, child cleanup, final exit-status validation and reader reuse after each response. A known per-input failure must not desynchronize the following response.

### Result and error boundaries

The shared operation carries typed outcomes; consumers must not recover distinctions by matching diagnostic strings.

| Observation | Shared resolution meaning | Adapter behavior |
| --- | --- | --- |
| Original input reports missing | Initial resolution failure | Checkout retains `RevisionNotFound`; bulk records failure for that input; a273 retains `CommitNotFound`. |
| Original object exists, canonical-ID peel cannot produce Commit | Existing noncommit | Checkout retains `RevisionNotFound`; bulk records failure for that input; a273 retains its noncommit `Other` error. |
| Recognized ambiguous response | Per-input resolution failure | Bulk preserves valid neighboring results; checkout reports unresolved revision. Ordinary object-read ambiguity remains an error rather than absence. |
| Malformed/unknown response, invalid identity/type, truncated body or wrong separator | Protocol error | Propagate error; never convert to missing or preserve a misleading successful batch. |
| Spawn, I/O or child-exit failure | Process error | Propagate error, including final status after otherwise valid responses; never convert to missing. |

The recognized ambiguity status belongs in the shared parser's typed response representation. Commit-resolution adapters may classify it as unresolved; the ordinary object-reader adapter retains its error boundary. This changes mapping at the adapter, not the parser or transport. The a273 reader must continue reporting ambiguity as an error and must not classify it as an absent commit.

### Exact implementation footprint

Only these four existing files are in the conditional correction scope. Existing root-owned c140 migration edits are preserved.

| File | Bounded change |
| --- | --- |
| `crates/jit/src/storage/git.rs` | Shared session resolution and typed outcomes; unary/bulk delegation; reader exposure; shared bounded body drain; retire obsolete fallback and suffix-first contract/diagnostic wording. |
| `crates/jit/tests/fast_docs_templates/git_module_tests.rs` | Owning producer regression and result/error matrix; update invocation-shape expectations while retaining process, SHA-256, framing and lifecycle assertions. |
| `crates/jit/src/storage/json.rs` | After a273 lands, replace its local resolve/peel control flow with the reader operation; preserve missing/noncommit mapping, canonical identity, bytes and abbreviated label. |
| `crates/jit/src/commands/snapshot.rs` | One message-search wiring case in the existing source-mode test; production keeps the mandated `checkout.resolve_commit(reference)` call. |

No Cargo target, dependency, documentation-floor or Docker change belongs to this implementation footprint. This design document is a separate planning artifact.

### Process budgets

| Operation | Required budget |
| --- | --- |
| Unary commit resolution | 1 process |
| Bulk resolution of distinct inputs | 1 process, independent of input count |
| Base comparison | 2 processes total |
| Existing single pinned-path read | 3 processes total |
| Single-revision pinned inspection | 3 processes per call; 6 across two calls, independent of reference count |
| a273 committed-path read | 1 child for resolution, optional peeling and requested content |

Extra peel requests use the existing child. Multi-revision inspection keeps its existing per-commit tree operations and single content reader in addition to the one resolution process.

### Contract reconciliation for the lead

The following wording needs transparent reconciliation after approval, separately from this design-only edit. Preserve historical delivery records; do not silently rewrite the old migration as though it originally included this correction.

| Authoritative location | Stale or incomplete wording | Required reconciliation |
| --- | --- | --- |
| `dev/active/f4d9502a-git-cli-module/plan.md`, `git-pinned-reads` capability | Methods hold the existing bridge's “logic and semantics.” | State original-expression resolution followed by canonical-object peeling, one shared unary/bulk/session operation and the explicit error boundaries. |
| Same plan, `git-object-reads` capability | “A name containing a newline is rejected before git sees it.” | After the approved a273 cutover, state `-Z`, LF acceptance and NUL rejection; retain one parser and the process budgets. |
| Same plan, DEC-13 and DEC-14 | `--batch`/`--batch-check` protocol lacks the new framing qualification; snapshot uses `resolve_commit` without its corrected semantic contract. | Record approved framing and shared resolution while preserving the existing API and one-reader decision. |
| Same plan, `snapshot-metadata-through-module` summary row; `breakdown.json` node of that name | Two-file migration footprint. | Record the explicitly approved four-file producer correction as a scope amendment, retaining the original migration history and c140 criteria. |
| `breakdown.json`, `pinned-reads-into-module` and `git-object-reads` contracts | “Today's semantics and results” and newline rejection describe the earlier producer. | Distinguish historical preservation from the approved semantic correction and transport cutover. Verify linked issue wording when the lead updates its source. |
| `crates/jit/src/storage/git.rs`, `resolve_commit`, `compare_with_base`, `resolve_commits` | Exact `rev-parse --verify <revision>^{commit}` semantics, matching diagnostic and bulk rewrite/fallback. | Replace within the approved source footprint with the shared operation's actual contract and diagnostics. |
| `git_module_tests.rs`, invocation assertions | Old `rev-parse` and per-name fallback argv. | Assert the shared Types session invocation and unchanged numerical budgets. |

c140 REQ-01 already names the correct API and must remain. Record the correction's acceptance explicitly without weakening c140's ignored-file, no-checkout or metadata requirements, or a273's valid-path, error and one-process guarantees. No issue or manifest is amended by this document.

## Implementation Steps

1. Have the lead confirm the authorization boundary and reconcile authoritative scope/contracts. Start from the landed a273 implementation, preserving existing c140 source changes.
2. Extend the owning producer test with original message search, including LF, and observe RED on the current producer before implementation. Keep the failure output as evidence.
3. Implement the shared session operation and error mapping; route unary and bulk through it, remove the old fallback, and expose it through the existing reader.
4. Delegate a273's local resolution sequence to the shared reader operation. Add the single snapshot wiring case; retain existing consumer tests.
5. Run focused producer and consumer checks, then the required Rust validation. Return changes for lead review and gates. Escalate any need for a fifth source file, a different error policy or a larger process budget before proceeding.

## Testing Approach

Extend `test_git_checkout_canonicalizes_symbolic_abbreviated_and_full_revisions` and the existing producer error/bulk tests rather than creating another suite. Cover message searches with and without LF, nested tags, symbolic/full/abbreviated identities, explicit peel/ancestry, existing noncommits, missing/empty input, ranges and option-shaped names. A mixed batch must prove that recognized ambiguity and missing input preserve valid following results. Test malformed responses and process failures through the shared parser/lifecycle owner, including final status and exact body draining before a following request.

Retain SHA-256 identity coverage, existing framing/payload tests and child cleanup coverage; do not move their ownership into snapshot. Preserve the existing scale tests for one bulk resolution process, three-process pinned inspection and two-process base comparison. Re-run a273's existing LF/message-search/tag/binary and one-child tests after delegation. Add one message-search assertion to `test_determine_snapshot_source_mode_resolves_requested_commit_and_defaults_to_head` to prove snapshot wiring only.

Use the existing focused suite commands in `dev/TESTING.md` and the shared check queue. Required Rust checks remain `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` with zero warnings, and `cargo test`; full gates and commits remain lead-owned. No Cargo execution is needed for this design-only artifact.

## Risks and Open Questions

Option A is conditionally selected; the outstanding decision is the user's Git-floor/runtime approval, followed by a273 completion. No alternative architecture is selected here. The approved error policy and four-file boundary are sufficient for implementation; Rust method/type spelling is an implementation detail, not permission to alter those contracts.

The two-command Git 2.39-compatible alternative proved the semantics but expands process counts, while a `rev-list --no-walk` substitute incorrectly accepts ranges. Neither is an authorized fallback. The single-process design depends on the approved NUL-framed transport, and the bulk ambiguity/body-drain tests must establish that error classification and session alignment coexist. If that cannot be achieved within the approved boundary, report the concrete limitation to the lead before changing architecture.
