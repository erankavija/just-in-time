# Repository-state quality hardening — plan

**Issue:** 1cc809de (epic), planning node 02dc4bac
**Type:** epic
**Priority:** high
**Date:** 2026-07-23

## Problem Statement

The repository-materialization epic (cdc840ad) delivered a sound transaction kernel whose strongest guarantees are narrated rather than enforced. The adversarial audit `dev/studies/cdc840ad-audit-2026-07-23.md` graded the implementation B+ and its maintainability C+, and recorded:

- **Boundary drift** — the claimed single planning boundary is seven public plan-producing entry points across five error types; the capture/plan/apply/retry protocol is hand-copied at 29 sites with 65 conflict arms and 30 bespoke convergence messages (audit B2-1, B2-2, B2-3).
- **Latent panics (planning-time finding)** — at planning time, two hand-aligned 4-variant action enums were bridged by `unreachable!` on the publication path (B2-4).
- **Narrated guarantees** — no property-based coverage of plan derivation, cutover guards implemented as source-text greps, no multi-threaded contention coverage, stringly path literals with a hand-maintained repair-path mirror (B2-5, B2-6, B2-8, B3).
- **Unmaintained performance claims** — every published timing is a single self-attested measurement; `apply_bulk_update` retains the O(n)-sessions pathology; 693 zero-byte lock files accumulate; nobody has profiled the ~1s per-session cost (C2).
- **Hygiene debt** — dead and over-broad exports, test-only API in the shipped surface, a 3,900-line inline test module, review-round test names, no contributor architecture doc, five advisory-debt items (B2-7, B2-9…B2-12).
- **Presentation debt** — nine falsifiable errors in the showcase deck and an epic-completion "independent" review performed by the building agent itself (A1, A2, A4).

This epic makes the implementation match its claims and makes every published claim trace to a repository artifact.

## Success Criteria

Copied from the epic; the audit finding IDs in parentheses are the traceability map.

- [hard] REQ-01: A single mutation-session combinator owns the capture/plan/apply/retry protocol — retry bound, conflict classification, and terminal conflict error live in one place, and command modules contain no hand-written retry loops or per-site conflict arms. (B2-1, C3-1)
- [hard] REQ-02: Every public plan-producing entry point returns a typed error, and producer failures reach exit-code mapping through typed variants with no stringify-then-downcast round-trip. (B2-2, B2-3, C3-2)
- [hard] REQ-03: Journal-action extraction in the transaction kernel is total; the publication path contains no `unreachable!` panic sites. (B2-4, C3-3)
- [hard] REQ-04: The initialize and profile-application request variants flow through the shared plan-identity tail, or a recorded decision item states why they are exempt. (C3-4)
- [hard] REQ-05: Property-based tests cover plan-hash determinism under action reordering and managed-document splice round-trip. (B2-5, C4-1)
- [hard] REQ-06: The cutover guard enforces publisher visibility restrictions instead of source-text substring matching, while the deleted-module assertions are retained. (B2-6, C4-2)
- [hard] REQ-07: Multi-threaded contention tests cover concurrent mutation-session opening and active-layout reentry rejection. (B3, C4-3)
- [hard] REQ-08: Well-known repository file paths are compile-time constants, and repair-path test coverage derives from the repair planner's own declaration rather than a hand-maintained list. (B2-8, C4-4)
- [hard] REQ-09: A checked-in benchmark harness records repeated runs with machine and cache-state metadata as artifacts at a stable path, and every published timing claim traces to such an artifact. (C2-1)
- [hard] REQ-10: Session-open budget tests bound bulk update to eligible work, matching the guarantee already tested for automatic transitions. (A2-4, C2-2)
- [hard] REQ-11: The per-session cost profile is recorded as an artifact, and the lock-hygiene approach chosen from that profile is implemented so zero-byte lock files no longer accumulate without bound. (C2-3, C2-4)
- [hard] REQ-12: Test-only failure-injection and fixture surfaces are feature-gated out of the default library surface, dead exports are removed, and public items with no external consumer are demoted to crate visibility. (B2-7, B2-10, C5-1, C5-2)
- [hard] REQ-13: The repository-state store's inline test module lives in a sibling file, and tests named after review rounds follow the `test_<function>_<scenario>` convention. (B2-9, B2-11, C5-3, C5-4)
- [hard] REQ-14: A contributor architecture document covers the repository-state/materialization subsystem, and the stale storage-abstraction pointer in the core system design document is swept. (B2-12, C5-5)
- [hard] REQ-15: The five advisory-debt items recorded at the predecessor epic's completion are cleared from the codebase and documentation. (C5-6)
- [hard] REQ-16: A corrected successor presentation of the repository-materialization story exists in which every falsifiable claim traces to a repository artifact, and the predecessor deck is archived with a tombstone pointing to the successor and the audit. (A1, A2, A4, C1)

## Shared architectural contracts

Contracts named here are referenced by the manifest through `contract_refs` and `produces_contracts`; each is defined once and cited rather than restated in issue bodies. An implementation-produced contract has one producing task reachable from every consumer; a plan-fixed contract is settled here and has no producer.

### `mutation-session-contract` [implementation-produced] — Mutation-session retry combinators

The command-layer retry contract designed in *Mutation-session combinator (S1)*: `with_mutation_attempts`/`with_mutation_session`/`classify_apply`/`capture_or_retry`, the `MutationSessionExhausted` terminal error, and the single `MUTATION_SESSION_RETRY_LIMIT` bound. The combinator-foundation task produces it in `crates/jit/src/commands/mod.rs`; the single-session and self-managed migrations consume it. The bulk-update budget work deliberately does not depend on it — it derives the per-mutation session constant from a baseline run so combinator changes cannot invalidate the bound.

### `producer-error-type` [implementation-produced] — Typed producer error family

The `ProducerError` family and the `RepositoryStateError::Producer(#[from] ProducerError)` retyping designed in *Typed-error boundary (S1)*, spanning `materialize.rs`, `projection_render.rs`, `artifact_classifier.rs`, and `profile_apply.rs`. The producer-error-family task produces it; the finalizer error types embed its typed `validate_proposed_layout`, and the exhaustive exit-code match consumes the full variant set.

### `test-support-feature` [implementation-produced] — test-support feature twin pattern

The `test-support` Cargo feature and its feature-on-`pub` / feature-off-`pub(crate)` twin pattern designed in *Test-support feature gating (S4)*. The feature-gating task produces it in `crates/jit/Cargo.toml` and `crates/jit/src/storage/mod.rs`; the test-only rule-serialization publics move onto the same twin pattern.

### `session-cost-artifact-schema` [plan-fixed] — Session-cost benchmark artifact schema

The JSON artifact schema pinned in *Performance contract (S3)* and instantiated by the committed `dev/studies/perf/session-cost-27ffbd2d.json`: top-level `schema_version`/`jit`/`machine`/`corpus`/`method`/`commands`, with per-command `name`/`argv`/`class`/`n`/`min_ms`/`median_ms`/`p95_ms`/`max_ms`/`lock_files_created`. The benchmark harness and the bulk-mutation scenario both emit to this schema.

### `virtualpath-const-representation` [implementation-produced] — Cow-backed VirtualPath constants

The `Cow<'static, str>`-backed `VirtualPath` and its `pub const` well-known-path associated items designed in *Path constants and repair targets (S2)*. The Cow-representation task produces it in `crates/jit/src/repository_state/path.rs`; the well-known-path call-site migration consumes the constants.

### `repair-target-authority` [plan-fixed] — Derived repair-target authority

The authority relationship pinned in *Path constants and repair targets (S2)*: `repair_target_paths` enumerates the complete drift-independent set of paths repair may materialize (default-ruleset targets, configured-projection targets, profile-target keys; obsolete-schema deletions excluded), and every non-delete `derive_repair` action target must be a member of that set on the same inputs. `derive_repair` is not refactored to consume it, since it needs composed bytes per target that a path set cannot supply.

## Design

### Story structure

Five stories, each a checkpoint for its criterion cluster per the story-as-checkpoint pattern. Implementation children live under their story; downstream stories depend on the story node, not on individual tasks.

```mermaid
graph TD
    B[breakdown 24bab642] --> P[planning 02dc4bac]
    S3[S3 Performance contract<br/>REQ-09..11] --> B
    S1[S1 Planning-boundary collapse<br/>REQ-01..04] --> B
    S2[S2 Testable guarantees<br/>REQ-05..08] --> S1
    S4[S4 Hygiene and docs<br/>REQ-12..15] --> S1
    S5[S5 Presentation succession<br/>REQ-16] --> S3
    S5 --> S2
    S5 --> S4
    E[epic 1cc809de] --> S5
```

*(Every arrow in this document — mermaid and prose "→" alike — reads "depends on", matching the direction of a jit dependency edge. The epic depends only on the sink story S5; the graph stays transitively reduced, which jit enforces on edge insertion.)*

- **S3 Performance contract** starts immediately and in parallel with S1: the lock-hygiene decision is already made from the planning-time profile (PD-1, epic D-7), so S3 implements it, builds the benchmark harness, and fixes the bulk-update session budget. The benchmark harness (REQ-09) is a new `scripts/` entry with artifacts under a stable repository path; budget tests (REQ-10) mirror `test_check_auto_transitions_opens_sessions_only_for_eligible_backlog_issues`.
- **S1 Planning-boundary collapse** is the highest-leverage change: a mutation-session retry contract in `crates/jit/src/commands/mod.rs` retiring the 30 copy-paste retry loops; typed errors for the two `anyhow` plan producers (`finalize_gate_registry_edit`, `finalize_archive_execution`) and the six `anyhow` producer signatures in `materialize.rs`; total journal-action extraction in `file_transaction.rs`; folding `Initialize`/`ApplyProfile` into the shared plan-identity tail or recording the exemption decision. The combinator and error designs are pinned in the two subsections below.
- **S2 Testable guarantees** follows S1 so property tests and contention tests target the settled surface: proptest for `plan_hash` reorder-invariance and splice round-trip, visibility-based cutover guard, contention tests over `open_mutation_session` and `ActiveLayoutTracker`, `VirtualPath` associated consts with `repair_paths()` derived from the repair planner's declaration.
- **S4 Hygiene and docs** follows S1 because the public-surface demotions (REQ-12) must not race the boundary refactor: feature-gate `FailurePoint`/`TransactionFailureInjector`/`test_support`, delete `issue_draft`, demote the 13 over-broad exports, split the store's inline test module, rename review-round tests, write the contributor architecture doc, sweep `core-system-design.md`, clear the four non-lock advisory-debt items (the zero-byte-lock item is cleared by S3's lock-hygiene implementation).
- **S5 Presentation succession** is last: the corrected retelling deck re-derives its figures from S3's artifacts and the final tree, and the predecessor deck moves to the archive with a tombstone.

**Leaf wiring rule.** A story node is a checkpoint: it depends on its own task sinks and completes only when they do. Story-level prerequisite edges are *mirrored onto task sources* so leaves are actually blocked, not just their checkpoints: every S1 and S3 task source depends on the breakdown node B (releasing the fan-out only when both breakdown gates pass); every S2 and S4 task source depends on the S1 story node; every S5 task source depends on the S2, S3, and S4 story nodes. Intra-story edges stay between siblings. With this rule no leaf can become Ready before the breakdown gate and its upstream story checkpoints are Done, and jit's transitive reduction keeps the redundant story-to-story edges only where they are not already implied.

### Mutation-session combinator (S1)

The audited surface is 30 hand-copied retry loops across 16 command files (29 `for _ in 0..8` plus `template.rs:176` under `TEMPLATE_RETRY_LIMIT`), 65 `RetryableConflict` match arms, and 30 bespoke "did not converge" messages. The sites split into **22 pure single-session** loops and **8 self-managed multi-session** loops that, within one attempt, release a preflight session and then acquire a `.git/jit` claims guard, run an external checker subprocess, or revalidate a cross-reopen invariant before opening the apply session (`dependency.rs:260,396`; `issue.rs:552`; `gate.rs:1357`; `mod.rs:2141,2353`; `gate_check.rs:909`; `template.rs:176`). A combinator-held session cannot serve the second group: it would hold `.repo-write.lock` across `claims_mutation_guard` (inverting today's lock order into a deadlock hazard) and across multi-second subprocess runs.

Resolution: one owner of the retry contract, two entry points that share it, and a session never held across between-session work:

```rust
const MUTATION_SESSION_RETRY_LIMIT: usize = 8;

#[derive(Debug, thiserror::Error)]
#[error("{operation} did not converge after {attempts} capture conflicts")]
pub struct MutationSessionExhausted { operation: &'static str, attempts: usize }

enum AttemptOutcome<T> { Done(T), Retry }

// The single apply-conflict classifier — the only place a RetryableConflict on
// apply is interpreted; no command module writes an arm.
fn classify_apply<T>(outcome: Result<impl Sized, RepositoryStateStoreError>, value: T)
    -> Result<AttemptOutcome<T>>;

// Retry driver: owns the bound and terminal error, holds NO session, so a
// closure may open/release sessions, take a claims guard, and run a subprocess
// between them exactly as today.
fn with_mutation_attempts<T>(operation: &'static str,
    attempt: impl FnMut() -> Result<AttemptOutcome<T>>) -> Result<T>;

// Session-passed convenience for the 22 pure single-session sites: opens one
// fresh recovered session per attempt, applies Apply(plan) via classify_apply.
enum SessionStep<T> { Apply(MaterializationPlan, T), Done(T), Retry }

fn with_mutation_session<S: RepositoryStateStore, T>(
    store: &S, layout: &RepositoryLayout, operation: &'static str,
    attempt: impl FnMut(&mut dyn RepositoryMutationSession) -> Result<SessionStep<T>>,
) -> Result<T>;  // implemented on top of with_mutation_attempts + classify_apply
```

`with_mutation_attempts` owns the bound (one named constant replacing every literal `8`) and the typed terminal `MutationSessionExhausted` (replacing all 30 bespoke bails; mapped to today's generic exit code in `main.rs`). `classify_apply` owns apply-conflict classification. Capture-conflict classification stays centralized in the existing `capture_*` helpers that already fold `RetryableConflict` into `Ok(None)`; the few sites with inline capture arms (`gate_check.rs:746,770`; `claim.rs:214`; `config.rs:317`) adopt a shared `capture_or_retry` helper. The 22 single-session sites use `with_mutation_session` (read-only sites return `Done`); `claim.rs:212` is a free function, which is why the entry points are free `fn`s generic over the store. The 8 self-managed sites use `with_mutation_attempts` directly, keeping their present structure — preflight, release, guard/subprocess, the same cross-reopen equality checks as today (`template.rs:217-227`, `mod.rs:2430-2444,2491`, `gate_check.rs:997`) returning `Retry`, then apply via `classify_apply`; guards are closure locals whose `Drop` runs after apply, preserving lock ordering byte-for-byte. Result: no `for _ in 0..8` and no per-site conflict arm anywhere in command modules — REQ-01 holds for all 30 sites with no exemptions. The refactor is behavior-preserving: the 35 failure-injection points and ~20 interruption tests pass unchanged, and conflict-classification unit tests move onto `classify_apply`/`with_mutation_attempts`.

### Typed-error boundary (S1)

Producer failures currently stringify through `RepositoryStateError::Producer(String)` (34 sites; sink at `repository_state/mod.rs:583` via `format!("{error:#}")`) and are re-typed by the runtime downcast chain `projection_producer` (`mod.rs:590-598`); the exit-code boundary (`main.rs:104,783`) probes with `downcast_ref` + `is_profile_target_conflict`. The design removes every string and every downcast:

1. New `ProducerError` (thiserror), each variant carrying its typed source: `MissingCapture(&'static str)`, `MalformedUtf8(#[from] FromUtf8Error)`, `UnknownProjection(String)`, `UnknownKind { projection, kind }`, plus transparent `#[from]` for `CaptureError`, `RepositoryLayoutError`, `ConfigurationDeclarationError`, `InvariantConfigError`, `ItemError`, `RuleConfigError`, `ProjectionError`, `ManagedDocumentError`. The six `materialize.rs` producers (`read_text`, `assemble_config`, `assemble_config_from_declarations`, `render_capture_closure`, `validate_capture_closure`, `compose_configured_projections`) **and** the two internal `anyhow` leaves — `render_projection_body` (`projection_render.rs:62`, whose injected read-closure becomes `FnMut(&str) -> Result<Option<String>, ProducerError>`) and `validate_proposed_layout` (`artifact_classifier.rs:515`) — change to `Result<_, ProducerError>`.
2. `RepositoryStateError::Producer(String)` becomes `Producer(#[from] ProducerError)`. `fn producer`, `fn projection_producer`, `fn is_profile_target_conflict`, and the twelve `.map_err(…producer…)` sites (`mod.rs:405,415,428,431,436,520,612,617,645,648,678,683`) are deleted in favor of `?`. The inline `Producer(format!)` sites (`mod.rs:501,508`) become `ProducerError::ConfigParse` and a new `RepositoryStateError::Overlay(#[from] OverlayError)`. The profile-application producer (`profile_apply.rs`, whose `Producer(String)`/`producer` sites live at `:397,:435,:469,:678` plus the `profile_registry_error` helper) joins the same retyping with profile-specific variants — `ProfileRegistryNotFile { target }`, `ProfileRegistryParse { target, #[source] source }`, `ProfileContributionConflict { identity, registry }` — and `compose_default_ruleset`, already typed, stays as is.
3. `finalize_gate_registry_edit` returns `Result<_, RepositoryStateError>` via a new `GateRegistryEditError { MissingEdit, MultipleEdits, DeclarationMismatch }` variant, with `#[from]` variants added for its eight `?`-propagated leaf classes (`MutationError`, `GateDeclarationError`, `OverlayError`, `SeedError` join the existing `Delta`/`PlanHash`/`Layout`).
4. `finalize_archive_execution` returns `Result<_, RepositoryStateError>` via a new `ArchiveExecutionError` enumerating the archive-specific classes found at their construction sites — `MissingDestination`, `MissingContentIdentity`, `ContentIdentityChanged`, `NonFileSource{role}`, `UnsafeOccupant{role}`, `MissingPlannedDocument`, `RelinkTargetsPinned`, `RelinkStale`, `MissingCapturedIssue`, `MalformedIssueJson(#[source] serde_json::Error)`, `IssueIdMismatch`, `CapturedIssueCacheLost`, `MissingChangedIssue` — plus `#[from]` for its typed leaves (`EventLogError`, `PlanError`, the retyped `validate_proposed_layout`); shared leaves reach exit mapping via the `RepositoryStateError` `#[from]` variants.
5. `main.rs::error_to_exit_code` replaces the two downcast probes with one **exhaustive** match over `RepositoryStateError` (no `_ =>` arm): projection/managed-document/profile-conflict/ambiguous-ownership/gate-registry-edit/archive-execution variants → validation-failed exit; capture/parse producer variants → generic exit; layout variants → invalid-argument exit. The profile `--json` boundary keeps its single sanctioned `downcast_ref::<RepositoryStateError>` on the anyhow CLI transport, but `profile_json_error` (`main.rs:770-791`) switches from the deleted `is_profile_target_conflict` bool to matching the `ProfileTargetConflict` variant (and the new profile producer variants map to `PROFILE_CONFLICT`/`PROFILE_ERROR` codes explicitly) — the round-trip being removed is stringify-then-downcast *inside* `repository_state`, not the boundary conversion itself.

**Totality mechanism and acceptance criterion:** deleting `Producer(String)` and the sink/downcast functions removes anyhow's blanket `From<E: Error>` from every producer signature, so each `?` must resolve to a concrete `#[from]` variant — an unmapped failure class is a compile error — and the exhaustive `main.rs` match means a future variant cannot default to the wrong exit code. Two mechanisms make the boundary string-free, and together they are REQ-02's acceptance criterion, checked after the typed-error work lands: (a) **compiler-enforced totality** — no function in `crates/jit/src/repository_state/` carries an `anyhow` type in its signature; (b) **grep-enforced message-freedom** — no error value is constructed with `format!`, and `String`-typed variant fields carry raw identifiers captured by value (a projection name, a registry target), never a pre-rendered message; all rendering lives in `Display` impls. The variant inventory above records every surface verified during planning (the six `materialize.rs` producers, two internal leaves, two finalizers, `profile_apply.rs`); any bridge it does not name — including the captured-declaration assembly and the initialization/profile-application paths — fails criterion (a) or (b) mechanically rather than by list membership.

### Cutover guard (S2)

The current guard is the substring test `tests/provenance_contract/repository_state_cutover_tests.rs:39-67`, which greps the production halves of the four cutover command modules for 14 forbidden substrings — defeated by reformatting or by moving a raw write one file over. The property it approximates: only the mutation-session API may publish into repository roots. The visibility replacement: `FileTransactionKernel` (+ `TransactionControlLocation`) and the in-repository writers `write_file_atomic`/`write_file_atomic_bytes` tighten from `pub(crate)` to `pub(in crate::storage)` — their only callers (`repository_state_store.rs`, `user_config_store.rs`, `file_transaction.rs`) live in `crate::storage`, so repository-owned publishers compile unchanged while `crate::commands` can no longer name them. The three external-export wrappers (`write_external_export_atomic`, `publish_external_file_noreplace`, `publish_external_directory_noreplace`) move to a `storage::external_publish` submodule re-exported `pub(crate)`: their legitimate callers (`commands/snapshot.rs`, `commands/graph.rs`) publish to caller-chosen paths outside repository roots (charter D-4), and the module name states that intent. The low-level rename primitive `rename_noreplace_cap` is a repository transaction primitive, not an export API: it stays `pub(in crate::storage)` alongside the in-repository writers, reachable by the external-publish wrappers only through their storage-internal implementation. The deleted-module and renderer-owner tests (`:69-139`, `:141-161`) are retained verbatim; only the substring test is deleted. Residual raw `std::fs` publishing, which visibility cannot restrict, keeps exactly one narrow retained assertion in the same provenance-contract suite — scanning only the four cutover modules for the four `std::fs` publisher calls (`fs::write`, `fs::rename`, `File::create`, `OpenOptions::new`) — a deliberate, recorded exception to the visibility mechanism; the `clippy.toml` disallowed-methods alternative is rejected because it applies crate-wide and `commands/snapshot.rs` legitimately stages exports with raw `std::fs` into temp directories.

### Path constants and repair targets (S2)

`VirtualPath` is `String`-backed through `RootRelativePath::Descendant(String)` (`path.rs:13-19`), which blocks `const`. The design changes the payload to `Cow<'static, str>`: `Cow::Borrowed` is const-constructible, so well-known paths become `pub const` associated items (`VirtualPath::GATES`, `EVENTS`, `CONFIG`, `INDEX`, `RULES`, …) and the ~100 fallible `VirtualPath::data("literal")?` sites become infallible const references. Parsing/deserialization produce `Cow::Owned`; `Ord`/`Hash`/`Eq` semantics are unchanged (`Borrowed("x") == Owned("x")`), so const and parsed keys collide correctly in the `BTreeMap`-keyed image. Because consts bypass the reserved-path `checked()` guard (`path.rs:173-186`), one test iterates every associated const (`VirtualPath::ALL_KNOWN`) asserting round-trip equality through the fallible constructor, proving each const canonical and non-reserved. Repair-target coverage stops mirroring by hand, with the authority designed around two verified facts: every family in `derive_repair` (`repository_state/mod.rs:660-701`) is *drift-filtered* — projections skip when bytes already match (`materialize.rs:351-357`), the default ruleset writes only on diff (`materialize.rs:432,456-459`), profile targets push only when the entry differs (`mod.rs:680-697`) — so a set derived from walked actions would enumerate only currently-drifting paths; and the repair test's 11 literals (`tests/fast_rules/derived_state_repair_tests.rs:85-110`) mix repair *write-targets* with three fixture-*input*-only paths (`index.json`, `events.jsonl`, the profile record, which `derive_repair` never writes). The design: a new `repository_state::repair_target_paths(image, declarations, profiles) -> Result<BTreeSet<VirtualPath>>` enumerates the complete drift-independent set of paths repair may materialize — default-ruleset targets guarded by the capture spec (via `serialized_default_ruleset(config).schema_files`), configured-projection targets (via `require_target`), and the keys of `compose_profile_targets` per claims — while obsolete-schema *deletions* stay excluded by definition (their targets are occupant-proven paths deliberately outside the declared set, `materialize.rs:498-512`). `derive_repair` is **not** refactored to consume it (it needs composed bytes per target, which a path set cannot supply); the authority relationship is enforced by a unit test asserting every non-delete action target of `derive_repair` is a member of `repair_target_paths` on the same inputs. The repair test keeps a three-literal fixture-input tail (`repair_paths() = repair_target_paths(…) ∪ {index.json, events.jsonl, profile record}`) and gains a full-drift coverage assertion: seed every target stale and assert the repair delta's write/set-mode target set equals `repair_target_paths(…)` exactly. `repair_target_paths` is an ordinary public API of `repository_state` — a declarative introspection surface in the same spirit as validation — so the in-crate subset test and the integration coverage test both call it without `test-support` gating and without any dependency on the test-support feature-gating work.

### Performance contract (S3)

A new `scripts/benchmark-session-cost.sh` (naming after `scripts/benchmark-rust-build.sh`) builds the release binary, generates a fixture repository of a recorded issue count, and times each scenario with ≥3 warmup and ≥20 measured runs, emitting one JSON artifact at the stable path `dev/studies/perf/session-cost-<jit-commit>.json` (commit in the filename so history accumulates; the planning-time profile `session-cost-27ffbd2d.json` is the first instance and the schema template). Schema fields: `schema_version`; `jit{version,commit,dirty,profile}`; `machine{cpu_model,logical_cpus,ram_kb,kernel,measurement_fs,repo_real_fs}`; `corpus{issue_count}`; `method{samples_per_command,warmup_runs,warm,cache_state,timing_clock}`; `commands[]{name,argv,class,n,min_ms,median_ms,p95_ms,max_ms,lock_files_created}`; `mutation_syscall_summary{task_clock_ms,user_s,sys_s,page_faults,context_switches}`. Scenarios: baseline (`--version`), single read (`issue show`), read-all (`query available`, `issue list`), and mutation (`issue update --priority`); the bulk-mutation scenario is added by the bulk-update task after its fix lands, so its numbers measure the fixed behavior. Mutable scenarios have a reset protocol: the harness materializes a fresh fixture repository per measured sample (regenerate or restore from a pristine copy), so no sample observes a prior sample's mutations, and warmup runs use throwaway fixtures of the same shape. Cache-state contract: warm-only (cold needs root to drop the page cache and is not assumed); warm = page cache primed by the warmup runs against the pristine copy; timings are comparable only across artifacts sharing `measurement_fs`. Citation rule: every published timing cites artifact path + command name + stat (e.g. `session-cost-<commit>.json › issue_update_mutation › median_ms`); an uncited number is a staleness defect per `@/inv/single-source-prose`. The planning profile also pins the optimization target: a title-only mutation costs ~3.7 s median, dominated by system time and ~545k minor page faults (memory materialization of two full captures), not lock I/O and not subprocesses.

### Test-support feature gating (S4)

The injection seam is production code — `file_transaction.rs` threads `&dyn TransactionFailureInjector` through ~121 `repository_check` sites and both storage backends hold an `Arc<dyn TransactionFailureInjector>` defaulting to `NoTransactionFailures` — so the definitions stay unconditional; what gets feature-gated is the *public exposure*. A `test-support` feature in `crates/jit/Cargo.toml` gates: the sole public re-export of `TransactionFailurePoint`/`TransactionFailureInjector`/`NoTransactionFailures` (`storage/mod.rs:66-68`, with a `#[cfg(not(feature))] pub(crate) use` twin so internal resolution never changes), `mod test_support` (`seed_issue_fixture`), `pub mod test_helpers` (`commands/mod.rs:64`), the conflict-injection branch at `repository_state_store.rs:527-532` plus its `memory.rs` backing machinery (from `#[cfg(test)]` to `#[cfg(feature = "test-support")]`), and the file backend's only public fault-injection path, `JsonFileStorage::with_repository_state_failures` (`storage/json.rs:314`), with the same feature-on-`pub` / feature-off-`pub(crate)` twin pattern (the memory backend's injection twins are already `#[cfg(test)] pub(crate)` and need no change).

Tests keep compiling through a self dev-dependency — `[dev-dependencies] jit = { path = ".", features = ["test-support"] }` in `crates/jit`, and the same feature on `crates/server`'s dev-dependency — so `cargo test --workspace` unifies the feature into test builds while `cargo build`/release drops the surface. This adds zero integration-test targets and requires **no CI or gate-config changes**: `cargo-ci.sh` picks the feature up via the dev-dependency, and the standalone `clippy --lib` gate keeps verifying the feature-off public surface. It also dissolves the S3/S4 ordering conflict over the failure-injection probe: in-crate tests (including the S3 budget test) compile with the feature regardless of which story lands first.

### Bulk-update session budget (S3)

REQ-10 is a behavior fix plus its locking test. Today `apply_bulk_update` (`commands/bulk_update.rs:161-197`) opens preflight and publication sessions for **every** filter-matched issue before knowing whether anything changes (cost 2·|matched|), while `check_auto_transitions` already demonstrates the correct shape: a cheap in-memory eligibility prefilter that can only skip, never authorize, plus an authoritative recheck inside the session. The design: a pure `UpdateOperations::is_provable_noop(&Issue) -> bool` prefilter over the already-listed issues, deliberately **Issue-local and no-op-only**: it skips an issue exactly when every operation's target value already holds (state equal, labels present/absent, priority, gate and assignee state), which is decidable from the listed `Issue` alone. Potentially *invalid* updates are **not** prefiltered — they still open sessions so the same authoritative in-session validation produces the same per-issue error results as today; repository-dependent validity never moves out of the session, and the prefilter only skips, never authorizes (the same TOCTOU property as the auto-transitions guard). The bound: sessions opened = C·|{matched issues that are not provable no-ops}|, where C is the per-mutation session constant (currently 2: preflight + publication); the test derives C from a single-issue baseline run rather than hardcoding it, so S1's combinator work cannot invalidate it. The test reuses the `SessionOpenCounter` probe counting the once-per-session-open failure point exactly as the auto-transitions budget test does, seeding K matched issues of which K−J already hold their target values, and asserting C·J, not C·K.

### Decisions pinned at epic creation

Recorded in the epic description (D-1…D-7): single-epic scope, milestone v1.0 with the production-readiness epic depending on this one, deck succession (corrected retelling only, predecessor archived with tombstone), audit imported as the evidence source, independent holistic review as a required epic gate, benchmark harness as a maintained contract, and lock-hygiene design chosen from profiling data before fan-out.

### Plan decisions

- **PD-1 (lock hygiene, satisfies epic D-7): eliminate the per-issue sidecar read lock; the surviving lock set is O(1).** Decided from the planning-time session-cost profile `dev/studies/perf/session-cost-27ffbd2d.json` (summary in the sibling `.md`). The profile shows the zero-byte locks originate on the *shared read path* — `load_issue` shared-locks a `<id>.lock` sidecar on every read (`storage/json.rs:918-919`, created O_CREAT at `storage/lock.rs:286-294`, never removed; 692 sidecars for 665 issues; a read-all command such as `jit query available` creates all 665) — so the initially considered lazy creation was refuted: deferring creation still yields one sidecar per issue ever read, O(issues) growth. The sidecar is redundant: issue JSON is published by atomic temp+rename (`atomic_write.rs:365`), so readers cannot observe torn files; writers serialize on `.jit/.repo-write.lock`; `list_issues` already holds `.index.lock` shared around the load loop (`json.rs:1028-1030`). `load_issue` therefore reads directly (defense-in-depth, if retained, takes the shared lock on the existing repo-scoped `.index.lock`). Retained fixed locks stay O_CREAT-without-O_EXCL and are never unlinked, so flock coordination remains race-safe. Post-fix bound: ~6 fixed lock files (`.repo-write`, `.index`, `.gates`, `.events`, per-worktree bootstrap, claims), independent of issue count; one-time deletion of orphaned sidecars is cleanup, not correctness.

- **PD-2 (successor-deck assets, resolves audit A4-2): vendor presentation assets under the deck folder.** The predecessor deck loads reveal.js, KaTeX, and three font families from CDNs and cannot present air-gapped; the successor vendors reveal.js and fonts locally (and drops KaTeX entirely per A4-1), so the deck is self-contained and its rendering is reproducible from the repository alone.

### Gate assignment

Per repository convention, gates are need-based per footprint: Rust-touching children carry `cargo-ci` and `code-review`; documentation children carry `doc-review` and `docs-mechanical`; children touching `mcp-server/` carry `mcp-ci` — the REQ-15 advisory-debt sweep edits `mcp-server/lib/tool-generator.js:161,173,179`, so the S4 child owning that item carries it. Stories carry the union of their children's footprint baselines. The epic carries `repo-validate` and `holistic-review` (independent reviewer distinct from the building agent, epic D-5).

### Breakdown overview

Generated from `dev/active/1cc809de-breakdown.json`; do not edit by hand.

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| planning-boundary-collapse | Collapse the planning boundary to a single mutation-session contract | story | A single mutation-session contract and a fully typed error boundary collapse the copy-pasted planning surface. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | — | — | self-managed-multi-session, exit-mapping-string-free-sweep, total-journal-action-extraction |
| enforced-guarantees | Convert narrated materialization guarantees into enforced tests | story | Property, contention, visibility, and path tests convert narrated materialization guarantees into enforced checks. | — | plan-story-structure | — | — | plan-hash-reorder-property-test, managed-document-splice-property-test, visibility-cutover-guard, contention-tests, virtualpath-callsite-migration, repair-target-coverage |
| performance-contract | Establish an artifact-backed performance contract | story | A checked-in benchmark harness, a bounded session budget, and lock-hygiene make performance claims artifact-backed. | — | plan-story-structure | — | — | lock-hygiene-sidecar, bulk-mutation-benchmark-scenario |
| hygiene-and-docs | Clear repository-state hygiene debt and document the subsystem | story | Feature-gating, surface reduction, test-module hygiene, docs, and debt clearance harden the repository-state subsystem. | — | plan-story-structure | — | — | public-surface-demotions, test-only-publics-gating, store-test-split-renames, architecture-doc-sweep, remove-dead-command-parameter, mcp-schema-generator-warnings, cli-error-code-doc-correction, fail-closed-manual-ci-script |
| presentation-succession | Replace the repository-materialization showcase with a corrected successor | story | A corrected successor deck traces every falsifiable claim to an artifact and the predecessor deck retires behind a tombstone. | — | plan-story-structure | — | — | predecessor-deck-tombstone |
| retry-combinator-foundation | Mutation-session retry combinator foundation | task | A single mutation-session retry contract centralizes the retry bound, apply-conflict classification, and the terminal error. | — | plan-mutation-session-combinator | touches crates/jit/src/commands/mod.rs; touches crates/jit/src/main.rs; touches crates/jit/src/storage/repository_state_store.rs; uncertain: conflict-classification unit tests may need a crate-internal RetryableConflict constructor in crates/jit/src/storage/repository_state_store.rs | — | — |
| single-session-site-migration | Single-session retry call-site migration | task | The pure single-session retry loops and inline capture arms move onto the shared mutation-session combinators. | mutation-session-contract | plan-mutation-session-combinator | touches crates/jit/src/commands/mod.rs; touches crates/jit/src/commands/archive.rs; touches crates/jit/src/commands/batch_create.rs; touches crates/jit/src/commands/claim.rs; touches crates/jit/src/commands/config.rs; touches crates/jit/src/commands/dependency.rs; touches crates/jit/src/commands/document.rs; touches crates/jit/src/commands/gate.rs; touches crates/jit/src/commands/gate_check.rs; touches crates/jit/src/commands/init.rs; touches crates/jit/src/commands/issue.rs; touches crates/jit/src/commands/migrate.rs; touches crates/jit/src/commands/profile.rs; touches crates/jit/src/commands/project.rs; touches crates/jit/src/commands/validate.rs; uncertain: dependency.rs, issue.rs, gate.rs, gate_check.rs, and mod.rs also hold self-managed multi-session sites that are out of scope here; template.rs holds only a self-managed site and is not touched by this task | — | retry-combinator-foundation |
| self-managed-multi-session | Self-managed multi-session retry migration | task | The eight self-managed multi-session sites move onto the shared retry driver with lock ordering preserved. | mutation-session-contract | plan-mutation-session-combinator | touches crates/jit/src/commands/dependency.rs; touches crates/jit/src/commands/issue.rs; touches crates/jit/src/commands/gate.rs; touches crates/jit/src/commands/mod.rs; touches crates/jit/src/commands/gate_check.rs; touches crates/jit/src/commands/template.rs | — | single-session-site-migration |
| producer-error-family | Typed producer error family for repository-state | task | Producer failures carry a typed ProducerError instead of stringifying through Producer(String). | — | plan-typed-error-boundary | touches crates/jit/src/repository_state/mod.rs; touches crates/jit/src/repository_state/materialize.rs; touches crates/jit/src/repository_state/projection_render.rs; touches crates/jit/src/repository_state/artifact_classifier.rs; touches crates/jit/src/repository_state/profile_apply.rs | — | — |
| finalizer-error-types | Typed finalizer errors for gate-registry edit and archive execution | task | The gate-registry-edit and archive-execution finalizers return typed RepositoryStateError values. | producer-error-type | plan-typed-error-boundary | touches crates/jit/src/repository_state/mod.rs; touches crates/jit/src/repository_state/archive.rs | — | producer-error-family |
| exit-mapping-string-free-sweep | Exhaustive exit-code mapping and residual error-string sweep | task | Producer failures map through an exhaustive RepositoryStateError match and residual untyped errors are retired. | producer-error-type | plan-typed-error-boundary | touches crates/jit/src/main.rs; touches crates/jit/src/repository_state/mod.rs; touches crates/jit/src/repository_state/export.rs; touches crates/jit/src/repository_state/initialize.rs; uncertain: the residual anyhow/format! sweep may touch any other file under crates/jit/src/repository_state/ that still carries an untyped signature or a format!-constructed error value, for example index.rs, overlay.rs, default_rules.rs, projection.rs, rules_document.rs, rule_serialize.rs, or rules_gates_projection.rs | — | finalizer-error-types, retry-combinator-foundation, plan-identity-tail |
| total-journal-action-extraction | Total journal-action extraction in the transaction kernel | task | Journal-action extraction returns a typed mismatch error instead of unreachable panics on the publication path. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | touches crates/jit/src/storage/file_transaction.rs; touches crates/jit/src/storage/transaction_journal.rs | — | — |
| plan-identity-tail | Plan-identity tail for initialize and profile-application requests | task | Initialize and profile-application requests flow through the shared plan-identity tail. | — | plan-story-structure | touches crates/jit/src/repository_state/mod.rs | — | — |
| plan-hash-reorder-property-test | Plan-hash reorder-invariance property test | task | A property test pins plan-hash invariance under permutation of the action-set input order. | — | plan-story-structure | creates crates/jit/src/repository_state/image_plan_hash_property_tests.rs; touches crates/jit/src/repository_state/image.rs | — | planning-boundary-collapse |
| managed-document-splice-property-test | Managed-document splice round-trip property test | task | A property test pins the managed-document splice round-trip through the public renderer. | — | plan-story-structure | creates crates/jit/src/repository_state/managed_document_property_tests.rs; touches crates/jit/src/repository_state/managed_document.rs | — | planning-boundary-collapse |
| visibility-cutover-guard | Visibility-enforced cutover guard | task | Compile-time publisher visibility, not source-text substring matching, enforces the repository-root cutover guard. | — | plan-cutover-guard | creates crates/jit/src/storage/external_publish.rs; touches crates/jit/src/storage/mod.rs; touches crates/jit/src/storage/file_transaction.rs; touches crates/jit/tests/provenance_contract/repository_state_cutover_tests.rs | — | test-support-feature-gating |
| contention-tests | Multi-threaded contention tests for mutation sessions and layout reentry | task | Real-thread contention tests exercise concurrent session opening and layout-reentry rejection over the shared store. | — | plan-story-structure | creates crates/jit/src/storage/repository_state_store_contention_tests.rs; touches crates/jit/src/storage/repository_state_store.rs | — | planning-boundary-collapse |
| virtualpath-cow-representation | Cow-backed VirtualPath with well-known path constants | task | VirtualPath becomes Cow-backed so well-known paths are compile-time const associated items. | — | plan-path-constants-repair-targets | touches crates/jit/src/repository_state/path.rs | — | planning-boundary-collapse |
| virtualpath-callsite-migration | Migrate well-known path sites to VirtualPath constants | task | The fallible well-known VirtualPath::data literal sites become infallible const references. | virtualpath-const-representation | plan-path-constants-repair-targets | touches crates/jit/src/repository_state/image.rs; touches crates/jit/src/repository_state/materialize.rs; touches crates/jit/src/repository_state/initialize.rs; touches crates/jit/src/repository_state/mod.rs; touches crates/jit/src/repository_state/mutation.rs; touches crates/jit/src/repository_state/export.rs; touches crates/jit/src/repository_state/path.rs; touches crates/jit/src/repository_state/profile_apply.rs; touches crates/jit/src/repository_state/overlay.rs; touches crates/jit/src/repository_state/archive.rs; touches crates/jit/src/storage/gate_runs.rs; touches crates/jit/src/validation/repository.rs; touches crates/jit/src/commands/gate.rs; touches crates/jit/src/commands/mod.rs; touches crates/jit/src/commands/template.rs; touches crates/jit/src/commands/gate_check.rs; touches crates/jit/src/commands/validate.rs; touches crates/jit/src/commands/document.rs; touches crates/jit/src/commands/project.rs; touches crates/jit/src/commands/profile.rs; touches crates/jit/src/commands/migrate.rs; touches crates/jit/src/commands/init.rs; touches crates/jit/src/commands/config.rs; touches crates/jit/src/commands/claim.rs; touches crates/jit/src/commands/batch_create.rs; uncertain: the exact set is the production sites that construct a well-known path; test-fixture construction of arbitrary non-well-known paths in unit-test modules is excluded, and a given file is touched only if it references one of the new well-known-path constants | — | virtualpath-cow-representation |
| repair-target-coverage | Derived repair-target coverage authority | task | Repair-path coverage derives from a repair_target_paths authority instead of a hand-maintained list. | repair-target-authority | plan-path-constants-repair-targets | touches crates/jit/src/repository_state/mod.rs; touches crates/jit/tests/fast_rules/derived_state_repair_tests.rs | — | planning-boundary-collapse |
| benchmark-harness | Benchmark harness and first session-cost artifact | task | A checked-in harness records repeated timed runs as JSON artifacts at a stable per-commit path. | session-cost-artifact-schema | plan-performance-contract | creates scripts/benchmark-session-cost.sh; creates dev/studies/perf/session-cost-<jit-commit>.json | — | — |
| bulk-update-eligibility-prefilter | Bulk-update eligibility prefilter and session budget | task | Bulk update opens sessions only for matched issues that are not provable no-ops. | — | plan-bulk-update-session-budget | touches crates/jit/src/commands/bulk_update.rs; touches crates/jit/src/commands/issue.rs | — | single-session-site-migration |
| bulk-mutation-benchmark-scenario | Bulk-mutation benchmark scenario | task | The benchmark harness records a bulk-mutation scenario measuring the fixed bulk-update behavior. | session-cost-artifact-schema | plan-bulk-update-session-budget, plan-performance-contract | touches scripts/benchmark-session-cost.sh; uncertain: the emitted artifact at dev/studies/perf/session-cost-<jit-commit>.json gains a bulk-mutation entry when the harness is re-run | — | benchmark-harness, bulk-update-eligibility-prefilter |
| lock-hygiene-sidecar | Lock-hygiene: eliminate the per-issue sidecar read lock | task | Removing the per-issue sidecar read lock keeps the surviving fixed lock set O(1) in issue count. | — | plan-pd1-lock-hygiene | touches crates/jit/src/storage/json.rs; touches crates/jit/src/storage/lock.rs | — | — |
| test-support-feature-gating | Feature-gate the test-support library surface | task | A test-support Cargo feature drops the failure-injection and fixture APIs from the default public surface. | — | plan-test-support-feature-gating | touches crates/jit/Cargo.toml; touches crates/jit/src/storage/mod.rs; touches crates/jit/src/storage/json.rs; touches crates/jit/src/storage/repository_state_store.rs; touches crates/jit/src/commands/mod.rs | — | planning-boundary-collapse |
| public-surface-demotions | Delete a dead export and demote the repository-state public surface | task | The dead export is removed and the over-broad publics are demoted to crate visibility. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | touches crates/jit/src/repository_state/mod.rs; touches crates/jit/src/repository_state/mutation.rs; touches crates/jit/src/repository_state/managed_document.rs; touches crates/jit/src/repository_state/initialize.rs; touches crates/jit/src/repository_state/default_rules.rs; touches crates/jit/src/repository_state/materialize.rs; touches crates/jit/src/repository_state/path.rs | — | test-support-feature-gating |
| test-only-publics-gating | Gate the test-only rule-serialization publics behind test-support | task | SerializedRuleSet, SchemaFile, and serialize_ruleset move behind the test-support feature. | test-support-feature | plan-test-support-feature-gating | touches crates/jit/src/repository_state/rule_serialize.rs; touches crates/jit/src/repository_state/mod.rs | — | test-support-feature-gating |
| store-test-split-renames | Split the store test module to a sibling file and fix review-round test names | task | The oversized inline store test module moves to a sibling file and review-round test names follow the convention. | — | plan-story-structure | creates crates/jit/src/storage/repository_state_store_tests.rs; touches crates/jit/src/storage/repository_state_store.rs | — | planning-boundary-collapse |
| architecture-doc-sweep | Write the repository-state architecture doc and sweep the stale storage pointer | task | A contributor architecture document covers the repository-state subsystem and the stale storage pointer is swept. | — | plan-story-structure | creates dev/architecture/repository-state-materialization.md; touches dev/architecture/core-system-design.md | — | planning-boundary-collapse |
| remove-dead-command-parameter | Remove the dead _command output parameter | task | The unused _command parameter is removed from the JSON output constructors and their call sites. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | touches crates/jit/src/output.rs; touches crates/jit/src/output_macros.rs; touches crates/jit/src/main.rs; uncertain: the output macros forward the command argument into JsonOutput::success and JsonError::new, so removing it updates macro-invocation sites across the crates/jit/src/commands/ modules broadly | — | planning-boundary-collapse |
| mcp-schema-generator-warnings | Clean up MCP schema-generator warnings | task | MCP schema generation completes without circular-reference, missing-definition, or unrecognized-ref warnings. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | touches mcp-server/lib/tool-generator.js; uncertain: the MCP schema generator tests under mcp-server/ may need updating alongside the generator fix | — | planning-boundary-collapse |
| cli-error-code-doc-correction | Correct the CLI project-render error-code documentation | task | The CLI reference describes the typed error-code split for project render under --json as implemented. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | touches docs/reference/cli-commands.md | — | planning-boundary-collapse |
| fail-closed-manual-ci-script | Make the manual CI script fail closed on skipped audits | task | The manual CI script no longer reports success while a security audit is skipped or failing. | — | plan-story-structure, dev/studies/cdc840ad-audit-2026-07-23.md | touches scripts/test-ci-manual.sh | — | planning-boundary-collapse |
| successor-deck | Corrected successor deck for the repository-materialization story | task | One vendored successor deck re-derives its figures from repository artifacts and the final issue tree. | — | plan-pd2-deck-assets | creates dev/presentations/1cc809de/ | — | enforced-guarantees, performance-contract, hygiene-and-docs |
| predecessor-deck-tombstone | Archive the predecessor deck behind a tombstone | task | The predecessor deck moves into the feature archive and a tombstone remains at its original path. | — | plan-story-structure | creates dev/archive/features/cdc840ad/showcase/; touches dev/presentations/cdc840ad/ | — | successor-deck |

```mermaid
flowchart LR
    N0["planning-boundary-collapse: Collapse the planning boundary to a single mutation-session contract"]
    N1["enforced-guarantees: Convert narrated materialization guarantees into enforced tests"]
    N2["performance-contract: Establish an artifact-backed performance contract"]
    N3["hygiene-and-docs: Clear repository-state hygiene debt and document the subsystem"]
    N4["presentation-succession: Replace the repository-materialization showcase with a corrected successor"]
    N5["retry-combinator-foundation: Mutation-session retry combinator foundation"]
    N6["single-session-site-migration: Single-session retry call-site migration"]
    N7["self-managed-multi-session: Self-managed multi-session retry migration"]
    N8["producer-error-family: Typed producer error family for repository-state"]
    N9["finalizer-error-types: Typed finalizer errors for gate-registry edit and archive execution"]
    N10["exit-mapping-string-free-sweep: Exhaustive exit-code mapping and residual error-string sweep"]
    N11["total-journal-action-extraction: Total journal-action extraction in the transaction kernel"]
    N12["plan-identity-tail: Plan-identity tail for initialize and profile-application requests"]
    N13["plan-hash-reorder-property-test: Plan-hash reorder-invariance property test"]
    N14["managed-document-splice-property-test: Managed-document splice round-trip property test"]
    N15["visibility-cutover-guard: Visibility-enforced cutover guard"]
    N16["contention-tests: Multi-threaded contention tests for mutation sessions and layout reentry"]
    N17["virtualpath-cow-representation: Cow-backed VirtualPath with well-known path constants"]
    N18["virtualpath-callsite-migration: Migrate well-known path sites to VirtualPath constants"]
    N19["repair-target-coverage: Derived repair-target coverage authority"]
    N20["benchmark-harness: Benchmark harness and first session-cost artifact"]
    N21["bulk-update-eligibility-prefilter: Bulk-update eligibility prefilter and session budget"]
    N22["bulk-mutation-benchmark-scenario: Bulk-mutation benchmark scenario"]
    N23["lock-hygiene-sidecar: Lock-hygiene: eliminate the per-issue sidecar read lock"]
    N24["test-support-feature-gating: Feature-gate the test-support library surface"]
    N25["public-surface-demotions: Delete a dead export and demote the repository-state public surface"]
    N26["test-only-publics-gating: Gate the test-only rule-serialization publics behind test-support"]
    N27["store-test-split-renames: Split the store test module to a sibling file and fix review-round test names"]
    N28["architecture-doc-sweep: Write the repository-state architecture doc and sweep the stale storage pointer"]
    N29["remove-dead-command-parameter: Remove the dead _command output parameter"]
    N30["mcp-schema-generator-warnings: Clean up MCP schema-generator warnings"]
    N31["cli-error-code-doc-correction: Correct the CLI project-render error-code documentation"]
    N32["fail-closed-manual-ci-script: Make the manual CI script fail closed on skipped audits"]
    N33["successor-deck: Corrected successor deck for the repository-materialization story"]
    N34["predecessor-deck-tombstone: Archive the predecessor deck behind a tombstone"]
    N7 --> N0
    N10 --> N0
    N11 --> N0
    N13 --> N1
    N14 --> N1
    N15 --> N1
    N16 --> N1
    N18 --> N1
    N19 --> N1
    N23 --> N2
    N22 --> N2
    N25 --> N3
    N26 --> N3
    N27 --> N3
    N28 --> N3
    N29 --> N3
    N30 --> N3
    N31 --> N3
    N32 --> N3
    N34 --> N4
    N5 --> N6
    N6 --> N7
    N8 --> N9
    N9 --> N10
    N5 --> N10
    N12 --> N10
    N0 --> N13
    N0 --> N14
    N24 --> N15
    N0 --> N16
    N0 --> N17
    N17 --> N18
    N0 --> N19
    N6 --> N21
    N20 --> N22
    N21 --> N22
    N0 --> N24
    N24 --> N25
    N24 --> N26
    N0 --> N27
    N0 --> N28
    N0 --> N29
    N0 --> N30
    N0 --> N31
    N0 --> N32
    N1 --> N33
    N2 --> N33
    N3 --> N33
    N33 --> N34
```
<!-- jit:breakdown-overview:end -->

## Implementation Steps

1. **Planning node 02dc4bac** — finalize this document, including the profile-backed lock-hygiene decision (PD-1); pass plan review.
2. **Breakdown node 24bab642** — decompose into the five stories and their children with `satisfies:REQ-NN` coverage labels; pass coverage preview and breakdown review.
3. **Fan-out** — S3 and S1 in parallel; S2 and S4 after S1; S5 after S2, S3, S4.
4. **Epic completion** — all stories done, `repo-validate` and the independent holistic review pass.

## Testing Approach

- Combinator refactor (S1) is behavior-preserving: the existing 35 failure-injection points and ~20 interruption tests must pass unchanged; conflict-classification unit tests move to the combinator.
- New property tests (S2) per `@/inv/semantic-test-assertions`; contention tests use real threads over a shared store, not injection.
- Budget tests (S3) count session opens via the existing `TransactionFailurePoint` probe, deterministic and machine-independent.
- Harness artifacts (S3) record n>1 runs, machine identity, and the warm-only cache-state contract; the successor deck (S5) cites artifact paths for every number.
- Suite topology respects `@/inv/bounded-rust-build-footprint`: the store test split moves a module to a sibling file without adding integration-test targets.

## Risks and Open Questions

- **Lock-hygiene is decided from planning-time data** (PD-1: sidecar elimination, profile-backed); the residual risk is an undiscovered reader that relies on the sidecar for more than torn-read protection — the lock-hygiene review must check every `load_issue` caller before removing the lock.
- **REQ-04 may end in a recorded exemption** if `Initialize`/`ApplyProfile` genuinely cannot share the plan-identity tail; the exit is a decision item, not silent scope drift.
- **Boundary refactor blast radius** — the combinator touches 16 command files; mitigated by landing it as mechanical per-file conversions behind an unchanged public behavior contract, verified by the untouched interruption suite.
- **Feature-gating test support** (REQ-12) is designed to require no CI or gate-config changes (self dev-dependency unifies the feature into test builds); the S4 child still verifies `cargo-ci` and the standalone `clippy --lib` gate behave as designed before landing.
- **Timing figures are machine-specific**; the harness records machine identity rather than pretending portability, and the deck quotes figures with their recorded environment.
