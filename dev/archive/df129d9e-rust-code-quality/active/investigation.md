# Investigation: Dispatch layering, executor surface, and build hygiene (df129d9e)

Grounding for [plan.md](plan.md), measured on 2026-10-20 at HEAD 1ed0a049c. Three areas were investigated independently; each section keeps its own claim table, consumer sweep and overlap notes. Line numbers drift as concurrent work edits `main.rs`, so each citation also names its function or match arm. The production-line counter for REQ-10 is [prodlines.py](prodlines.py).

## df129d9e investigation, area 1: CLI error boundary and main.rs dispatch

Scope: REQ-02, REQ-11, REQ-12, REQ-13, REQ-14, exit-code part of REQ-09.
Line numbers are as of 2026-10-20, HEAD 2210e044a plus uncommitted `.jit/` edits only; main.rs is being edited concurrently (2d10f1dd), so every finding also names its function or match arm.

### Claim classification

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1a | main.rs ~9,578 lines | already-true (fact) | `wc -l` = 9578 |
| 1b | `run()` starts ~2355 | valid | `fn run()` main.rs:2355, ends 8782 → **6,428 lines** |
| 1c | single match over ~33 command groups | **invalid-as-stated** | `Commands` enum (cli.rs:40) has **29** variants; `run()`'s dispatch `match command` (main.rs:2655) has 28 arms, of which 3 are no-ops (`Init` 2656 handled earlier at 2542, `Rdeps|List` 2984 normalized at 2633, `Version` 2987 handled at 2390). 25 dispatching arms. "33" is the stale survey count. A second, pre-validation `match &command` (2542-2627) handles `Init`. |
| 1d | functions over 500 lines | valid-and-open | Only `run()` (6,428). Next largest: `error_to_error_code` 353 (55-407), `dep_add_batch_json_error` 123 (441-563). Largest arms inside `run()`: `Commands::Gate` 4217-5584 (1,368), `Commands::Issue` 2988-4049 (1,062), `Commands::Validate` 7265-7824 (560), `Commands::Doc` 5826-6308 (483), `Commands::Claim` 8174-8554 (381), `Commands::Profile` 2659-2983 (325), `Commands::Config` 6763-7080 (318), `Commands::Query` 6387-6703 (317), `Commands::Serve` 7896-8173 (278). `GateCommands::Status` alone is 4611-4906 (296). |
| 2 | ten parallel JSON-error emitters, `handle_json_error!` 18 uses | **valid-and-open, counts invalid-as-stated, list incomplete** | See inventory below. `handle_json_error!` has **13** invocations (18 is the raw text match incl. comments/defs). At least 5 further emitters exist beyond the ten. |
| 3 | 44 inline `exit(json_error.exit_code().code())`, 76 `process::exit` total | valid (counts approximately right) | 44 exact-text matches (4 in pre-`run` helpers, 40 in `run()`). 76 text matches = **75 calls + 1 doc comment** (main.rs:9026, test module). Distribution: 14 in helpers before `run()`, 2 in `main()`, 59 in `run()`. Outside main.rs (non-test): `output_macros.rs:78` (inside `handle_json_error!`, expands 13×), `output.rs:387` and `output.rs:399` (`writeln_safe` / `writeln_safe_stderr`, broken-pipe), plus clap `error.exit()` at main.rs:2371 (parse failure). None in `commands/`, none in `crates/server/src`. |
| 4 | `error_to_error_code` at main.rs:55-407 (~353 lines) in the binary | valid-and-open | 55-407 exact. Downcast list in Primitive verification. Library homes: `ErrorCode` enum output.rs:670, `ErrorCode::ALL` output.rs:774, `ErrorCode::exit_code` output.rs:880, `ExitCode` output.rs:552, `JsonError`/`ErrorDetail` output.rs:465/530, `refine_id_error` output.rs:1162 (already a library downcast-classifier fragment), `JsonError::{issue_not_found, gate_not_found, cycle_detected, transition_blocked, ...}` output.rs:1180-1260. `jit::ExitCode`/`jit::JsonError` re-exported at lib.rs:47. |
| 5 | gate status `GateCheckError` at 4707,4726,4818,4897; issue search SearchError only on JSON path ~7210 | gate status: valid-and-open (lines exact). Search: **invalid-as-stated** in naming | The SearchError branch (7210-7218) is in top-level `Commands::Search` (`jit search`, 7140-7243), not `IssueCommands::Search` (3138-3228), which has no JSON error branch. Full fallback list below: 50+ sites. |
| 6 | server keeps own `PathReadError`→status map (routes.rs ~348) | valid-and-open | `path_read_error_status` routes.rs:348-356; plus 18 blanket `map_err(→ StatusCode)` sites. Disagrees with the CLI classifier (see Consumer sweep). |
| 7 | `process::exit` skips `Drop` for retained session and annotation guard | valid-and-open | Types confirmed below. Impact: in-process restoration skipped; OS releases flock fds at exit. |
| 8 | findings outcomes decide non-zero exit ad hoc | valid-and-open | 14 findings sites, 4 distinct mechanisms (below). |
| 9 | `apply plan --json` without lease prints both `Error:` and JSON | **valid, but it is the asserted contract, not a local defect** | main.rs:2232 `eprintln!("Error: {}", e)` runs unconditionally before `emit_top_level_json_error` (2263). Applies to every propagated error under `--json`. Reproduced (`jit apply plan abcd1234 --json` in scratch repo: stdout envelope, stderr `Error: no template 'plan' ...`, exit 2). Asserted by `test_propagated_config_parse_failure_emits_registered_envelope_and_human_diagnostic` (crates/jit/tests/cli_issue/top_level_failure_envelope_tests.rs:39, asserts stderr starts with `Error: ` at :58), while handler-owned failures assert empty stderr (same file :96, :113, :166, :216, :240). See Needs decision. |

Count: already-done/true 1 (1a); valid-and-open 7 (1b, 1d, 3, 4, 6, 7, 8); valid with inaccurate counts or naming 2 (2, 5); invalid-as-stated 1 (1c); valid but contract-protected 1 (9).

### Prior art

Container a2546471 (archived, `dev/archive/a2546471-json-error-contract/active/`; completion-report.md). Delivered: every `--json`-accepting arm renders the envelope post-dispatch; `ErrorCode` is the only production classification and sole exit-status source; payload-stream purity; census of 100 arms (98 probes + 2 exemptions). Decision D-5: clap parser diagnostics stay outside the envelope (keep that; main.rs:2365-2373).

Census and guard suites (all in the `cli_issue` target, crates/jit/tests/cli_issue/main.rs):
- `failure_probe_coverage_tests.rs` — `test_failure_probe_coverage_matches_clap_arms_and_emits_typed_envelopes` (:258), the definition-derived census; `test_probe_contract_errors_rejects_generic_code` (:446).
- `failure_lever_registry.rs` + `failure_lever_registry.toml` — one lever per reflected command path; exemptions with reasons (e.g. toml:999 `version`).
- `failure_probe_fixture.rs` (1,267 lines) — probe driver.
- `failure_envelope_contract_tests.rs` — canonical envelope shape.
- `payload_stream_purity_tests.rs` — stdout carries exactly one document (stderr not constrained, :49).
- `top_level_failure_envelope_tests.rs` — propagated vs handler-owned envelope, stderr expectations.
- `the removed exit-code subprocess suite`, `command_exit_code_projection_tests.rs` (1,186 lines) — exit-code/code parity.
- In-binary unit tests that test the classifier and move with it: main.rs `mod exit_code_projection_tests` (8949), `mod repository_state_classifier_tests` (9269), `mod lock_timeout_classification_tests` (9527). `mod linked_checkout_write_decision_tests` (8785) tests `linked_checkout_write_decision`.
- Others relevant to REQ-09: `crates/jit/tests/cli_repo_workflow/test_cli_consistency.rs`, `crates/jit/tests/scratch_build/stale_binary_json_exit_tests.rs`, `crates/jit/tests/cli_gate/gate_evaluate_all_test.rs`, `crates/jit/tests/cli_query_graph/scope_validation_tests.rs`, `crates/jit/tests/cli_item_validate/validation_lease_tests.rs`, `crates/jit/tests/cli_repo_workflow/claim_integration_tests.rs`; server route tests in `crates/server/src/routes.rs` `mod tests` (1049+; status asserts at 2011-2025, 2497-2506 call `path_read_error_status` directly).

Contract docs: canonical statement in `docs/reference/cli-commands.md` (~lines 23-40, "For failures reached after successful argument parsing ..."); generated vocabularies `docs/reference/error-codes.md` and `docs/reference/exit-codes.md`. cli-commands.md:506 states "Errors (always shown to stderr)".

Existing precedent for REQ-02 per-group handlers: `run_item`/`run_item_inner` (1783/1812), `run_invariant`/`run_invariant_inner` (1866/1886), `run_project`/`run_project_inner` (1938/1969), `run_query_all` (2002), `run_profile_lifecycle` (975). Each outer fn wraps the inner `Result` with `handle_json_error!`.

### Consumer sweep

#### A. JSON-error emitters in main.rs (production call counts exclude defs, comments, tests)

| Emitter | Def | Prod calls | Behavior |
|---|---|---|---|
| `command_json_error(err, code)` | 574 | 47 | caller-chosen code; only override is lock timeout |
| `claim_json_error(err, fallback)` | 612 | 7 | classifier, fallback when `GenericError` |
| `validate_fix_json_error` | 625 | 1 | classifier + `{:#}` message |
| `profile_json_error` | 1245 | 7 | own downcast cascade, fallback `ProfileError` |
| `profile_result` | 1302 | 7 | prints `profile_json_error` and exits (not in claim's ten) |
| `gate_pass_json_error` | 688 | 2 | own downcast cascade + details/suggestions, fallback `GateError` |
| `render_gate_pass_error` | 665 | 2 | prints `gate_pass_json_error` and exits (not in claim's ten) |
| `dep_add_batch_json_error` | 441 | 1 | per-edge cascade, fallback `DependencyError` |
| `invalid_argument(msg, json)` | 874 | 25 | JSON branch prints + exits 2 |
| `resolve_gate_key_for` | 848 | (exit site 860) | prints `InvalidArgument` + exits (not in claim's ten) |
| `exit_recovery_failed` | 599 | 2 | `RecoveryFailed`; human path `exit(1)` |
| `render_validation_json` | 636 | 8 | findings envelope or success |
| `emit_top_level_json_error` | 2263 | 1 (main) | classifier + InvalidLabelPattern suggestion + LinkedCheckoutWriteRefused details (not in claim's ten) |
| `handle_json_error!` | output_macros.rs:70 | 13 | `refine_id_error` + print + exit |
| `dependency_batch_error_code` | 424 | 1 | dominant-edge classifier |
| inline `JsonError::new(...)` | — | 1903 (`run_invariant_inner`), 1212 (`render_profile_agreement`), 3785 (`IssueCommands::Delete`), 4848 (`GateCommands::Status` transposed args), 5066 (`GateCommands::Add`), 5530 (`GateCommands::Preset`) | ad hoc |

`verb_hint_error` (1387, 13 calls) returns a typed error and propagates; it does not emit.

#### B. Fallback-code substitution sites (REQ-13)

Code chosen by call site rather than error type (function / arm, line):
- `gate_pass_json_error` 698/722/738/754 (`GateError` fallback 754).
- `dep_add_batch_json_error` 535 (`ValidationFailed`), 546 (`DependencyError`).
- `resolve_gate_key_for` 858 (`InvalidArgument`).
- `profile_json_error` 1251 (`ProfileNotFound` for any `NotFoundError`; classifier says `IssueNotFound`), 1264, 1273, 1280, 1293, 1297, 1299 (`ProfileError` for every other `RepositoryStateError` and unknown error; classifier maps `RepositoryStateError` variants to ValidationFailed/InvalidArgument/GenericError at main.rs ~162-175).
- `run_item_inner` 1804 (`ItemCommandFailed`), `run_invariant_inner` 1878 (`InvariantCommandFailed`), `run_project_inner` 1958/1960 (`ValidationFailed`/`ProjectCommandFailed`).
- `IssueCommands::Show` 3295 (`ItemNotFound`), 3381 (`issue_not_found` for any error); `IssueCommands::Status` 3410; `IssueCommands::Children` 3459; `IssueCommands::Progress` 3492 (all `JsonError::issue_not_found` for any error, via `handle_json_error!`).
- `IssueCommands::Update` 3691 (`GenericError` fallback) — **2d10f1dd overlap**.
- `IssueCommands::Delete` 3785 (`DeletionNotConfirmed` hard-coded; type already classifies to the same code).
- `DepCommands::Add` 4155, `DepCommands::Rm` 4209 (`DependencyError`).
- `GateCommands::Define` 4322, `Update` 4463, `List` 4503 (`GateError`, exit 6 = AlreadyExists class), `Add` 5066/5068 (`GateNotFound`/`GateError`), `Fail` 5338 (`GateError`).
- `GateCommands::Status` 4707, 4726, 4818, 4897 (`GateCheckError`, exit 3) — **8673dc5e overlap**.
- `GateCommands::Preset` 5381, 5450, 5574 (`PresetError`), 5530 (`failure_code`).
- `DocCommands` arm at 5987 (`InvalidArgument`), `Commands::Config` 6934 (`InvalidArgument`), `Commands::Hooks` 7121 (`HooksInstallError`).
- `Commands::Search` 7210-7218 (`RipgrepNotFound`/`SearchFailed`) — **34999aad adjacency**.
- `Commands::Validate` render calls 7406, 7452, 7518, 7623 (`GenericError`), 7760 (`ValidationFailed`/`GenericError`).
- `ClaimCommands::{Acquire 8216, Release 8261, Renew 8307, Heartbeat 8340, Status 8429, List 8502, ForceEvict 8545}` (`claim_json_error` fallback).
- `Commands::Worktree` 8598, 8651, 8707 (`WorktreeInfoError`/`WorktreeListError`/`WorktreeStoreDivergenceError`).
- `Commands::Serve` 7942, 7987, 8162: use `error_to_error_code` on both paths — consistent, not a fallback.

#### C. Human vs JSON exit divergence (REQ-13)

Mechanism: in every site of list B shaped `if json { print; exit(fallback) } return Err(e)`, the human path reaches `main()` → `error_to_exit_code` (410) while JSON exits with the fallback's class. They diverge whenever the classifier returns a different class. Concrete divergent classes (from `ErrorCode::exit_code`, output.rs:880-930):
- `GateCheckError`, `DependencyError`, `PresetError`, `Claim{Acquire,Release,Renew,Heartbeat,ForceEvict}Error` → exit 3; classifier on an unclassified/IO failure → 1 or 10.
- `GateError` → exit 6; human → classifier (1/3/10...).
- `SearchFailed`/`RipgrepNotFound` → 1; human IO failure → 10.
- `JsonError::issue_not_found` fallback (Show/Status/Children/Progress, `gate_pass_json_error` 745) → 3 for any error; human IO → 10, parse → 1.
- `profile_json_error` `ProfileError` → 1; classifier may give 4 or 2 for `RepositoryStateError` variants.
- `exit_recovery_failed`: JSON `RecoveryFailed` → 1; human `exit(1)`; consistent, but bypasses classifier.

Human-path-only exits that bypass the classifier: 4283 (`GateCommands::Define` `--env` parse, `exit(2)`; the same error under JSON propagates via the closure too — no JSON branch), 4863 (transposed args `exit(2)`), 7427/7497/7536/7558/7582/7591/7821 (validate `exit(1)`).

#### D. Findings outcomes (REQ-12, claim 8)

| Command (fn / arm) | Line | Decision | JSON | Human |
|---|---|---|---|---|
| `profile diff` (`render_profile_difference`) | 1129-1156 | `conflicted > 0` | `render_validation_json(ValidationFailed)` | `exit(ValidationFailed)` 1156 |
| `profile check` (`render_profile_agreement`) | 1191-1242 | `diverged > 0` | `JsonError::new(ValidationFailed)` + exit 1216 | `exit(ValidationFailed)` 1242 |
| `invariant check` (`run_invariant_inner`) | 1890-1925 | `result.has_drift()` | `JsonError` + exit 1906 | exit 1924 |
| `dep add` batch rejected | 4145 | typed `DependencyBatchRejectedError` | `dep_add_batch_json_error` exit | propagates → classifier |
| `gate status-all` (`GateCommands::StatusAll`) | 4907-5006 | `!all_passed` | **success envelope then exit 4** | exit 4 (5005) |
| `gate evaluate-many` | 5200-5302 | first failing entry's `gate_pass_json_error` code | **success envelope then `exit(code)`** 5302 | same exit |
| `gate preset apply` | 5520-5550 | `!errors.is_empty()` → `failure_code` | `JsonError` + exit | exit 5550 |
| `doc check-links` | 6240-6306 | library raw `i32` (`commands/document.rs:1291`: 1 errors, 2 warnings-only) | **success envelope then `exit(result.exit_code)`** | same — **f37d228e overlap** |
| `config validate` | 7040-7054 | `has_errors` | `render_validation_json` | exit 7053 |
| `validate --scope` | 7344-7381 | `report.has_errors()` | `ValidationFailed` | exit 4 |
| `validate --rule` | 7392-7427 | has_errors | `GenericError` | exit 1 |
| `validate --explain` | 7441-7497 | has_errors | `GenericError` | exit 1 |
| `validate <id>` | 7507-7536 | has_errors | `GenericError` | exit 1 |
| `validate --branch-drift/--leases` | 7540-7623 | per check | `GenericError` | exit 1 (7558/7582/7591) |
| `validate` full | 7700-7821 | integrity error / `rules_failed` | `ValidationFailed` or `GenericError` | `return Err(integrity)` or `exit(1)` |

Three findings commands (status-all, evaluate-many, check-links) emit a **success** envelope with a non-zero exit, so the exit code does not equal an envelope code. REQ-12's "findings outcomes as values" must choose the envelope for them.

#### E. Server error-to-status mappings (crates/server/src)

- `routes.rs:348` `path_read_error_status(&PathReadError)`: NotFound/CommitNotFound→404, InvalidPath/OutsideRepoRoot→400, Other→500. Callers: 366 `get_document_by_path`, 606 `get_document_content`, 636 `get_document_raw`, 689 `get_document_raw_by_path`, 743 `get_raw_wildcard`, 797 `get_document_history`, 845 `get_document_diff`. These executor methods return typed `Result<_, PathReadError>` (commands/document.rs:554, 572, 618, 660, 706, 795), not anyhow.
- Blanket maps (any error → one status): 500 at 91 `list_issues`, 168/180 `get_graph`, 235 `get_status`, 287/292/311/325 `search_issues`, 866 `list_gates`, 939 `get_strategic_types`, 965/978 `get_hierarchy`, 1010 `get_namespaces`; 404 at 102 `get_issue`, 881 `get_gate_definition`, 902 `list_gate_runs`, 919 `get_gate_run`.
- `embedded.rs:86/99/115` 404 for absent static assets (not error classification; out of scope).
- Conversion style: handlers return `Result<Json<T>, StatusCode>`, log with `tracing::error!`, return a bare status with empty body. No error envelope over HTTP. `lib.rs`, `main.rs`, `shutdown.rs`, `sse.rs`, `watcher.rs` contain no status mapping.
- CLI/server disagreement today: `OutsideRepoRoot` → CLI `GenericError` (exit 1) vs server 400; `PathReadError::Other` → CLI recurses on inner vs server 500; `NotFound` → CLI `RepositoryNotFound` (a file-not-found is labelled repository-not-found); `get_issue` maps IO failure → 404 where CLI would say IoError.

#### F. Drop skipped by `process::exit` (claim 7)

- `retained_mutation_session: Option<RetainedMutationSessionGuard>` (main.rs:2498, assigned 2523; `take()` at 8005 in `Commands::Serve`), from `JsonFileStorage::open_and_retain_mutation_session` (storage/json.rs:586). `impl Drop for RetainedMutationSessionGuard` storage/json.rs:245 → `release_inner`. The retained session holds `RepoWriteGuard`s (storage/repository_state_store.rs:390-392; `impl Drop for RepoWriteGuard` storage/repo_lock.rs:342) and `LayoutReentry`/`RepositoryOrderGuard` (Drop at repository_state_store.rs:192, guard_order.rs:43). `LockGuard` Drop (storage/lock.rs:151) also removes a `.lock.meta` file when it has one.
- `_linked_checkout_write_override: Option<DispatchMutationAnnotation>` (main.rs:2539), `impl Drop for DispatchMutationAnnotation` commands/mod.rs:188 restores the thread-local annotation.
- Every `exit` inside `run()` and helpers skips these. Practical impact today: OS closes fds so flocks release; thread-local restoration is moot at process end; any `.lock.meta` cleanup is skipped. Returning to `main()` before exiting fixes all of it.

#### G. Human-path rendering (claim 9)

- `main()` (2208-2252): `Err(e) => { eprintln!("Error: {}", e); emit_top_level_json_error(&e).unwrap_or_else(|| error_to_exit_code(&e)) }` (2230-2234). `{}` prints the outermost anyhow context only, no cause chain (`validate_fix_json_error` alone uses `{:#}`).
- `emit_top_level_json_error` reads raw argv for `--json` (2264) — REQ-15 overlap.
- `ActionableError` (errors.rs:34) bakes causes/remedies into Display text (errors.rs:75-95), so under JSON its remedies land in `message`, not `suggestions`.
- Other human error printers: 604 (`exit_recovery_failed` render failure), 4282, 4861, 7946, 7991, 8166, plus `eprintln!("Recovery failed: ...")` 608.

### Primitive verification

- No unified library error type. `JitError` does not exist. `anyhow::Error` is the boundary: `commands/mod.rs:117` `use anyhow::{anyhow, Context, Result}`; executor methods return `anyhow::Result`. Typed errors are ~40 independent `thiserror` structs/enums wrapped in anyhow. Exception: document read methods return `Result<_, PathReadError>` directly.
- Downcasting today: `anyhow::Error::downcast_ref::<T>()` matches the root error or any `.context()` layer, but not errors reachable only via `std::error::Error::source()`. The classifier uses `error.chain()` explicitly for `toml::de::Error`/`serde_json::Error` (main.rs ~223-227) and `std::io::Error` (~232-240), and recurses for `PathReadError::Other` and per-edge `DependencyBatchRejectedError`. `jit::storage::is_lock_timeout` is a library helper already.
- Order is semantic: lock timeout first, batch first, `io::Error` chain search before `GraphError`/not-found downcasts, ambiguous before invalid prefix. Moving must preserve order.
- Types downcast by `error_to_error_code`: `errors::{DependencyBatchRejectedError, LinkedCheckoutWriteRefusedError, TransitionBlockedError, ValidationFailedError, RedundantDependencyError, NotFoundError, AlreadyExistsError, LeaseNotFoundError, InvalidLabelPatternError, InvalidArgumentError, DeletionNotConfirmedError}`; the then claim-specific Git requirement error; `commands::{GatePassFailed, GateNotRequiredError, ManualGateAttestationRequiredError, ProfileApplyError, ProfileResolutionError, ProfileDependencyError, BatchValidationError, MutationSessionExhausted}`; `repository_state::{ProjectionError, ManagedDocumentError, RepositoryStateError, RepositoryIndexError}`; `profile::ProfileClaimError`; `storage::{RepositoryFormatTooNewError, IssueNotFoundError, GateNotFoundError, GateRunNotFoundError, PresetNotFoundError, RepositoryNotFoundError, GateAlreadyExistsError, AmbiguousIdError, InvalidIdPrefixError, PathReadError}` + `storage::is_lock_timeout`; `declarations::{GateStageParseError, GateModeParseError}`; `document::DocumentScopeParseError`; `templates::TemplateConfigError`; `GraphError`; std `io::Error`, `FromUtf8Error`, `Utf8Error`; `toml::de::Error`; `serde_json::Error`. Not downcast: `search::SearchError` (only in the `Commands::Search` arm), `ActionableError`.
- Details/suggestions currently built at call sites that a type-only classifier must reproduce from the error value: `GatePassFailed`/`GateNotRequiredError`/`ManualGateAttestationRequiredError` (gate_pass_json_error 688-760), `IssueNotFoundError` (carries `id`, storage/errors.rs:25 — so `issue_not_found(id)` can derive from it), `TransitionBlockedError` (`JsonError::transition_blocked` output.rs:1253), `LinkedCheckoutWriteRefusedError`, `InvalidLabelPatternError` suggestion, `RepositoryStateError::ProfileTargetConflicts` details, `DependencyBatchRejectedError` details, `AmbiguousIdError`/`InvalidIdPrefixError` details (`refine_id_error`), `SearchError::RipgrepNotInstalled` suggestion, profile `NotFoundError` suggestion. The transposed gate-status case (4835-4866) has no type; it needs one.

### Architecture fit

- Put `classify` in `output` (output.rs or a new `output/failure.rs` submodule; output.rs is 3,999 lines). Reasons: output.rs already owns `ErrorCode`, `ExitCode`, `JsonError`, `ErrorDetail`, and a downcasting fragment (`refine_id_error`, output.rs:1162), and already references `crate::storage` and `crate::commands` (output.rs:1163, 1915, 2883). `errors.rs` is the wrong home: it is imported by `domain/` (1 file) and `storage/` (4 files), so a classifier there that downcasts `commands::`/`repository_state::` types would point a pure-layer dependency outward (`@/inv/architecture-dependency-direction`). errors.rs already imports `crate::output::ErrorCode` (errors.rs:743), so the vocabulary lives in output.
- Signature: `pub fn classify(error: &anyhow::Error) -> JsonError` (code, message, details, suggestions; exit via `ErrorCode::exit_code`), or a `Failure` value with those four fields plus `exit_code()`. It must accept `&anyhow::Error` for the recursion cases. For the server's `PathReadError` routes, either convert with `anyhow::Error::from` or make the executor return anyhow there.
- Server: depends on `jit` (crates/server/Cargo.toml:28) and can import `jit::output::{classify, ErrorCode}`. It still needs one `ErrorCode → http::StatusCode` projection; `http` types should stay out of the jit crate.
- Single exit site: `main()` keeps the one `std::process::exit` (currently 2251; the broken-pipe exit at 2241 and the `output.rs:387/399` broken-pipe exits are further sites REQ-12 must fold or explicitly exempt). Handlers return `Result<Outcome, anyhow::Error>`, where `Outcome` carries a findings exit code or envelope.

### Invariant check

- `@/inv/architecture-dependency-direction`: classifier in output (adapter vocabulary) respects it; in errors.rs it would not (see above).
- `@/inv/semantic-types`: `doc check-links` carries a raw `i32` exit code in `LinkCheckResult` (commands/document.rs:1291-1303) — should be a semantic type or derived outcome.
- `@/inv/convention-convergence`: 15 parallel emitters and 50+ fallback sites are the defect class this invariant names.
- `@/inv/orthogonal-tests`: the classifier tests in main.rs (`exit_code_projection_tests`, `repository_state_classifier_tests`, `lock_timeout_classification_tests`) should move with the classifier into the library as its owning unit tests; CLI suites then test only wiring.
- `@/inv/canonical-cutover`: once `ErrorCode` fallback codes (`GateCheckError`, `GateError`, `DependencyError`, `PresetError`, `Claim*Error`, `Worktree*Error`, `ItemCommandFailed`, `InvariantCommandFailed`, `ProjectCommandFailed`, `HooksInstallError`, `SearchFailed`, `RecoveryFailed`, `ProfileError`) are no longer produced, they become dead vocabulary; removing them changes `ErrorCode::ALL` (47), generated `docs/reference/error-codes.md`, and the `failure_lever_registry.toml` expectations.
- `@/inv/single-source-prose`: `docs/reference/error-codes.md`/`exit-codes.md` are generated; cli-commands.md:506 "Errors (always shown to stderr)" conflicts with the handler-owned empty-stderr behavior.

### Overlaps with in-flight issues

| Issue | State | main.rs regions touched | Other files |
|---|---|---|---|
| 2d10f1dd atomic update/reject | InProgress | `IssueCommands::Update` (3497-3770, incl. fallback site 3691), `IssueCommands::Reject` (3943-3978) | commands layer (new executor op) |
| 34999aad search argv | InProgress | none expected; `Commands::Search` (7140-7243) is adjacent (its SearchError classification is a REQ-13 site) | `search.rs:116-162`, `crates/server/src/routes.rs` `search_issues` (278-345) |
| 16d63038 id resolution | Ready | possibly the transposed-args guard in `GateCommands::Status` (4835-4866, calls `storage().resolve_issue_id`); error types `AmbiguousIdError`/`InvalidIdPrefixError` classified at main.rs ~320-335 and `refine_id_error` output.rs:1162 | storage/json.rs:1264, storage/memory.rs:848, commands/gate_check.rs:737, validate.rs:262, mod.rs:2000, dependency.rs:458, template.rs:911 |
| f37d228e link-check warnings exit | Ready | `DocCommands::CheckLinks` (6240-6307, `exit(result.exit_code)` 6306) | commands/document.rs:1291, docs/reference/exit-codes.md |
| 8673dc5e manual attestation in gate status | Ready | `GateCommands::Status` (4611-4906) — holds all four `GateCheckError` sites | gate run storage/commands |

Sequencing: 2d10f1dd and 8673dc5e edit the same arms that REQ-02 extraction moves; land them first or extract those groups last. f37d228e decides the check-links findings exit; REQ-12 should consume its outcome rather than both editing `exit_code`.

## df129d9e investigation, area 2: success output, schema, CLI argument surface, stubs, human rendering

Snapshot: HEAD `1ed0a049c` (2026-10-20 23:40), `crates/jit/src/main.rs` 9,578 lines (production part ends at the `#[cfg(test)]` module at 8784; a second test module is at 1549-1632). Line numbers drift because other sessions are editing main.rs, so each citation also names its function or match arm. Schema figures come from `target/debug/jit --schema` (built 2026-10-20 23:20). The installed `~/.cargo/bin/jit` dates from Oct 3, is stale, and was not used.

### Claim classification

| # | Claim | Verdict |
|---|---|---|
| 1 | main.rs inline JSON branches and serialization; bodies authored in arms | valid-and-open (counts restated) |
| 2 | 62 response structs; per-query family = one envelope ×8; 33 `count` structs | valid-and-open (the 8 query structs are dead) |
| 3 | Bare payloads outside `JsonOutput::success` | valid-and-open (more sites than claimed) |
| 4 | 74/120 leaves fall back to `CommandResponse`; 46 come from a hand table | valid-and-open (exact) |
| 5 | 108 `json: bool` fields; argv scans at ~1388 and ~2264 | valid-and-open (exact) |
| 6 | Filter strings parsed ad hoc at ~20 sites, with no shared group | valid-and-open (about 14 sites) |
| 7 | 14 hidden stubs duplicate `COMMAND_HINTS`; schema ignores `is_hide_set` | valid-and-open (13 command stubs + 1 hidden flag) |
| 8 | Human rendering duplicated (issue list, validation, gate status, archive) | valid-and-open (more copies than claimed) |
| 9 | Dead output API hidden behind blanket `allow(dead_code)` | invalid-as-stated (the items are dead, but the mechanism is wrong) |

Totals: already-done 0, valid-and-open 8, invalid-as-stated 1.

#### 1. main.rs presentation and serialization
Counts cover production code only, with the test modules excluded:
- 171 lines of `if json`, 9 of `if !json` and 2 of `if *json`, for 182 lines in total.
- 55 `json!(`, 50 `serde_json::to_*` (30 `to_value`, 17 `to_string_pretty`, 3 `to_string`), 106 `JsonOutput::success` and 155 `to_json_string`.
- 63 `OutputContext::new`.

The epic's "155 / 145" figures do not reproduce exactly. The substance holds.

**Inline `json!` success bodies** (match arm, then line):
- `IssueCommands::Delete` 3808
- `DepCommands::Add` 4100, `DepCommands::Rm` 4170
- `GateCommands::Define` 4310, `Remove` 4587, `Add` 5023, `Evaluate` 5106, `EvaluateAll` 5156 and 5165, `Fail` 5321
- `PresetCommands::List` 5354 (`count`/`presets`), `Apply` 5502-5512, `Create` 5565
- `EventCommands::Tail` 5792 and `Query` 5813 (`count`/`events`)
- `LabelCommands::Values` 6737 (`namespace`/`values`/`count`)
- `ConfigCommands::Show` 6804 (a whole effective-config tree built inline), `Get` 6910, `Set` 6956, `Validate` 7027 (its validation logic is also inline, with a local `struct ValidationResult`)
- `HooksCommands::Install` 7088
- `Commands::Validate` 7604, 7618, 7658, 7717, 7747
- `Commands::Recover` 7837
- `MigrateCommands::LifecycleTimestamps` 7878
- `Commands::Serve` (8 sites, see §3)
- `ClaimCommands::Acquire` 8195, `Release` 8232, `Renew` 8285, `Heartbeat` 8323, `Status` 8361 (`count`/`leases`), `List` 8446 (`count`/`leases`), `ForceEvict` 8526
- `WorktreeCommands::Info` 8563

The `json!` calls in `dep_add_batch_json_error`, `gate_pass_json_error`, `emit_top_level_json_error`, `IssueCommands::Delete` 3789 and `GateCommands::Status` 4852 are failure paths and belong to area 1.

**Response structs built inline in arms:**
- `run_query_all` 2031 and 2038
- `Commands::Init` 2610
- `IssueCommands::Search` 3201 and 3210, `Show` 3258, `Status` 3423, `Claim` 3913, `ClaimNext` 4023
- `GateCommands::List` 4482, `Status` 4668/4676/4748/4781, `StatusAll` 4956/4961, `EvaluateMany` 5229/5263/5277
- `GraphCommands::Deps` 5596, `Rdeps` 5643, `Roots` 5667
- `QueryCommands::Available` 6453/6462, `Blocked` 6531/6548 (the blocked-reason mapping is built inline, about 6500-6530), `Strategic` 6587/6596, `Closed` 6632/6641
- `LabelCommands::Namespaces` 6714
- `Commands::Search` 7167
- `WorktreeCommands::List` 8615, which adds a `warnings` key by `map.insert` at 8618
- `WorktreeCommands::StoreDivergence` 8668
- `print_apply_result` 784

**Macros:** `output_data!` is used once (`IssueCommands::Show` 3373).

#### 2. output.rs struct inventory
`output.rs` has 65 `pub struct` declarations outside its tests (the test modules are at 235 and 2901). 43 of them are named `*Response`.

The 8 per-query structs are `ReadyQueryResponse` 1455, `BlockedQueryResponse` 1462, `AssigneeQueryResponse` 1503 (`assignee: String`), `StateQueryResponse` 1511 (`state`), `PriorityQueryResponse` 1519 (`priority`), `LabelQueryResponse` 1527 (`pattern`), `StrategicQueryResponse` 1535 and `ClosedQueryResponse` 1542. **All 8 are dead.** They are referenced only inside output.rs, by tests at 3827 and 3888. The live query arms already share `IssueListResponse`/`IssueListFullResponse` (`available`, `strategic`, `closed`, `all`, `issue list`, `list`) and `BlockedListResponse`/`BlockedFullListResponse` (`blocked`). No live query command echoes its filter. For REQ-04 this means deleting the dead family and adopting a generic envelope; nothing has to be merged.

There are exactly 33 `count` structs in output.rs. Each is listed with its collection key and any extra fields:

| line | struct | collection key | extras |
|---|---|---|---|
| 1353 | IssueListResponse | issues (MinimalIssue) | |
| 1368 | IssueListFullResponse | issues (Issue) | |
| 1378 | IssueSearchResponse | issues | query |
| 1391 | IssueSearchFullResponse | issues | query |
| 1430 | BlockedListResponse | issues (MinimalBlockedIssue) | |
| 1438 | IssueShowListResponse | issues | |
| 1446 | IssueStatusListResponse | issues | |
| 1455 | ReadyQueryResponse (dead) | issues | |
| 1462 | BlockedQueryResponse (dead) | issues | |
| 1472 | BlockedFullListResponse | issues (BlockedIssue) | |
| 1503 | AssigneeQueryResponse (dead) | issues | assignee |
| 1511 | StateQueryResponse (dead) | issues | state |
| 1519 | PriorityQueryResponse (dead) | issues | priority |
| 1527 | LabelQueryResponse (dead) | issues | pattern |
| 1535 | StrategicQueryResponse (dead) | issues | |
| 1542 | ClosedQueryResponse (dead) | issues | |
| 1553 | GraphDownstreamResponse | dependents | issue_id |
| 1624 | GraphDepsTreeResponse | nodes | issue_id, depth, summary |
| 1647 | GraphRootsResponse | roots | |
| 1736 | GraphTreeResponse | nodes | root |
| 1768 | DivergenceResponse | divergences | |
| 2090 | StateCount | (not a list; `count` is a per-state value) | state |
| 2116 | StateRollup | by_state | total, done, rejected, open, percent |
| 2240 | IssueChildrenResponse | issues | container, dangling |
| 2612 | GateCheckAllResponse | gates (count = gates.len()) | results (a second array), passed, total, not_run, all_passed |
| 2648 | GateEvaluateManyResponse | results | |
| 2662 | GateRunHistoryResponse | results | |
| 2715 | GateListResponse | gates | |
| 2801 | ArtifactConformanceResponse | artifacts | areas |
| 2867 | NamespacesResponse | namespaces | |
| 2874 | SearchResponse | results | query |
| 2882 | WorktreeListResponse | worktrees | (+warnings injected in main.rs) |
| 2890 | WorktreeStoreDivergenceResponse | divergences | checkout_store, reference_store |

`count` structs outside output.rs that are also list payloads:
- `ProfileCollection<T>` (`profile/application.rs:24`, key `profiles`). This is already a generic envelope (prior art).
- `DocumentListResult` (`commands/mod.rs:1734`, documents, issue_id)
- `AssetListResult` (`commands/mod.rs:1750`, assets, issue_id, document_path, summary, warnings)
- `ItemListResult` (`commands/item.rs:24`, items)
- `ProjectRenderResult` (`commands/project.rs:49`, projections)
- `InvariantCheckResult` (`commands/invariant.rs:29`, findings)

There are also 5 inline `json!` count bodies in main.rs: presets, events ×2, values and leases ×2.

The following carry a `count` but are not list envelopes:
- `ProjectionRenderReport.count` (`commands/project.rs:39`)
- `ProfileAgreement.count` (`profile/agreement.rs:255`, a per-entry count)
- `snapshot.rs:55` (IssuesInfo) and `snapshot.rs:66` (DocumentsInfo), nested in the snapshot manifest
- `StateCount`

Across all list payloads the collection keys are: issues, dependents, nodes, roots, divergences, by_state, results, gates, artifacts, namespaces, worktrees, profiles, documents, assets, items, projections, findings, presets, events, values, leases.

#### 3. Bare serialized payloads outside `JsonOutput::success`
`JsonOutput` is not a real envelope. `to_json_string` (output.rs:451) serializes the data and only injects a top-level `message`. A bare print therefore differs from it only by lacking `message`.

The complete list:
- `IssueCommands::BatchCreate`: dry run at 3107 (`outcome`), and the real run at 3128, which prints the `{key: id}` map raw and deliberately carries no `message` (code comment at 3120).
- `ArchiveCommands::Candidates` 6313.
- `ArchiveCommands::Document`: execute 6326, preview 6348.
- `ArchiveCommands::Container`: execute 6358, preview 6380.
- `Commands::Serve`, 8 sites: stop 7919, stop not-running 7933, status 7958, status not-running 7978, already running 8023, foreground 8080, started 8118, already running after a race 8148.
- `GraphCommands::Export` with `--json`/`--format json`/`batch` prints the export document raw (5770 for `batch`; the `export_graph` string otherwise). Its `--json` is format sugar, and the schema declares the raw shapes (`GraphExportSummaryResponse`/`Full`).
- `jit --schema` (`run()` 2380) prints the schema document. It is not a command success payload.
- Quirk: the `GateCommands::Status` "not run" branches print `JsonOutput::<Option<()>>::success(None)`. The data is `null`, which is not an object, so `with_message` drops the message silently and stdout is the bare text `null`. This happens 3 times: findings 4693, flat 4804 and default 4883.
- `EventCommands::Tail`/`Query` print raw `serde_json::to_string(&event)` lines in **human** mode (5800, 5821). This is intended.

#### 4. Schema
`CommandSchema::generate()` is at schema.rs:162. It walks `Cli::command()` through `extract_command_with_path` (271) and `extract_command_with_path_hidden` (275). Output schemas come from `get_output_schema_for_command(cmd_path)` (schema.rs:456), which matches a `"group_sub"` string to `schema_for!(T)`, using schemars 0.8.22 `JsonSchema` derives on the output.rs structs. The fallback at 744 is `_ => (None, "CommandResponse")`.

Measured from the generated schema:
- 120 leaves (with 21 groups that also carry an `output`).
- 46 leaves are typed.
- 74 fall back to `CommandResponse`: 61 real leaves and 13 hidden stubs.

The 61 real fallback leaves:
- label values, snapshot export
- dep rm, dep add
- rdeps, recover, serve
- issue batch-create, issue progress, issue delete
- query count, query divergence
- graph rdeps
- claim release, heartbeat, status, acquire, renew, list, force-evict
- archive candidates, document, container
- config get, validate, show-hierarchy, set, show
- migrate lifecycle-timestamps
- gate remove, add, update, show, evaluate-all, evaluate, status, define, fail
- gate preset list, apply, create, show
- doc diff, list, assets list, add, history, show, check-links, remove
- invariant check, project render
- item list, show, search, resolve
- worktree info
- events tail, query
- validate, hooks install

Drift evidence in the hand table: the key `"graph_downstream"` (schema.rs ~681) matches no path. `downstream` is only a clap alias (`cli.rs:2224`), and the path is `graph_rdeps`. As a result `graph rdeps` and `rdeps` fall back to `CommandResponse` even though `GraphDownstreamResponse` exists and main.rs emits it. The table also types group paths: `"query"` (bare `jit query`, line 623).

Supporting functions:
- `builtin_global_flags()` (schema.rs:209) adds clap's synthesized `--help`/`--version` to `global_options`.
- `hidden_commands()` (schema.rs:228) is a hardcoded set of 35 `"group_sub"` paths. It sets `Command.hidden` ("hidden from default MCP tool listing"), which propagates to children. It is unrelated to clap's `hide`: no stub is in the set, and all 35 are excluded in `mcp-server/curated-tools.json`.

Type sources outside output.rs:
- `crate::visualization::{GraphExportSummaryResponse, GraphExportFullResponse}`
- `crate::profile::*Result`
- `build_info::VersionInfo`
- `domain::Issue`
- `generate_types()` (schema.rs:754): ErrorResponse, ProfilePackageModel, GateChecker, Issue, State, Priority

#### 5. `--json` surface
- `cli.rs` has 108 `json: bool` fields: all 107 real leaves plus the bare `Commands::Query` parent (cli.rs ~244). The 13 stubs have none.
- `global = true` is already used once, for `Cli.quiet` (cli.rs:28). That is precedent.

There are two raw-argv scans:
1. `verb_hint_error` (main.rs:1388) scans the stub's captured `trailing_var_arg` args for `--json`, because the stubs declare no json field.
2. `emit_top_level_json_error` (main.rs:2264) scans `std::env::args()` because an error propagated from `run()` no longer carries the parsed flag, and pre-dispatch failures (repo discovery, `invocation_write_override`, `storage.validate()`) happen after parsing but before any arm extracts `json`.

Clap parse failures never reach either scan. `run()` calls `error.exit()` at 2371 after `unknown_subcommand_hint`. The documented contract agrees: `docs/reference/cli-commands.md:33` says "Clap parser diagnostics raised before dispatch remain outside this envelope guarantee." `reject_parent_query_filters` (main.rs:2139) treats a parent-level `--json` as a misplaced filter, so `jit query --json available` exits 2 today. A global flag makes that invocation valid, which is a deliberate REQ-09 change.

#### 6. Issue filter inventory
No `#[derive(Args)]`, `#[command(flatten)]` or typed `value_parser` exists for filters. The only typed CLI values are `CountDimension` (cli.rs:2538), `GraphExportFormat` (commands/graph.rs:13) and the two enums in declarations/gates.rs.

All filter fields are `String`/`Vec<String>` declared with `#[arg(short, long)]`:

| command | fields (cli.rs line) |
|---|---|
| `Commands::List` (alias, rewritten to `IssueCommands::List` at main.rs ~2637) | state 82, assignee 86, priority 90, label 96 (+full) |
| `Commands::Query` (bare) | state 225, assignee 229, priority 233, label 240 (+full) |
| `IssueCommands::List` | state 1214, assignee 1218, priority 1222, label 1228 |
| `QueryCommands::All` | state 2369, assignee 2373, priority 2377, label 2383 |
| `QueryCommands::Available/Blocked/Strategic/Closed` | priority + label (2400/2406, 2422/2428, 2444/2450, 2467/2473) |
| `QueryCommands::Count` | label 2511 |
| `IssueCommands::Search` | state 790, assignee 793, priority 796, `labels` (long `label`) 801 |
| `IssueCommands::Update --filter`, `ClaimNext --filter` | query-language expression strings (981, 1201), not field filters |

Where the strings are parsed:
- In main.rs:
  - `run_query_all` 2013 (`State::from_str`) and 2016 (`Priority::from_str`)
  - `IssueCommands::Search` 3167 (`labels::validate_label`, mapped to `InvalidArgumentError`), 3174 and 3175
  - `QueryCommands::Available` 6442, `Blocked` 6488, `Strategic` 6576, `Closed` 6621 (`Priority::from_str` in each)
- In the executor:
  - `commands/query.rs:278` `validate_label_pattern` (`InvalidLabelPatternError`, accepts `ns:*`)
  - assignee is never parsed; it is compared raw at query.rs:104, search.rs:56 and issue.rs:234

That is about 14 sites. Mutation-value parses (create 3009, update 3584-3585 and 3707/3712) are not filters.

Behavioural differences a shared group must resolve:
- **Label matching differs.** `issue search --label ns:*` exits 2 with "invalid --label filter" (an exact label is required and matched exactly). `query all --label ns:*` matches by pattern. Verified by running both.
- **Error code differs.** Search uses `INVALID_ARGUMENT`; query patterns use `INVALID_LABEL_PATTERN` (main.rs:306 and 2269).
- **Comma splitting differs.** `--label` in filters has no `value_delimiter` (create and update do). `docs/reference/cli-commands.md:462-476` claims comma separation works for `--label` generally.
- **Error path changes.** An invalid `--state` today returns an `INVALID_ARGUMENT` JSON envelope with exit 2 (verified). A clap `value_parser` would turn it into a clap diagnostic with no JSON envelope (see the Needs-decision item on parse errors).

**2195c98f boundary.** That epic's REQ-11 covers the single State/Priority enums, REQ-12 says "query assignee filters are `Assignee` values", REQ-13 covers query-language value lists, and REQ-18 confines raw strings to boundaries "CLI arguments…". REQ-16 here is structural: one flattened group, parsed once at the CLI boundary. They overlap where the group's fields become `State`/`Priority`/`Assignee`/label-pattern types. 2195c98f introduces no `LabelPattern` type. Proposed split:
- df129d9e builds the `IssueFilterArgs` group with `value_parser` over existing `FromStr` types (`State`, `Priority`, `domain::Assignee`).
- 2195c98f owns any new vocabulary types and the executor and query-language signatures.

Also, the `GraphCommands::Tree` human renderer truncates ids with `&id[..8.min(..)]` (main.rs ~5694 and ~5701). That is a 2195c98f REQ-03 site inside a renderer REQ-21 may move.

#### 7. Hidden stubs and hints
`hide = true` appears 14 times in cli.rs:

- **Verb-hint subcommand stubs (13), each `#[command(hide = true, trailing_var_arg = true)]`:**
  - `IssueCommands::Rm` 1087, `Remove` 1094, `Complete` 1102, `Edit` 1110
  - `DepCommands::Remove` 1288, `Delete` 1295
  - `GateCommands::Rm` 1509, `Delete` 1516
  - `DocCommands::Rm` 1987, `Delete` 1994
  - `LabelCommands::Add` 2569, `Rm` 2577, `Remove` 2585
- **Hidden flag stub (1):** `Commands::Validate.divergence` (cli.rs:410). It is handled in the `Commands::Validate` arm at ~7313 and is outside the verb-hint table.
- **No other hidden items.** No legitimately hidden command or flag exists.

`COMMAND_HINTS` (main.rs:1324) has 14 entries: the 13 stubs plus `("invariant","list")`. That last entry is served only by the parser-level path `unknown_subcommand_hint` (1365), which shows that path works today: it prints `Hint: …` to stderr, then the clap error, then exits 2. Verified with `jit invariant list --json`.

`verb_hint_error` (1387) is dispatched from `IssueCommands::Rm..Edit` 3819-3827, `DepCommands::Remove/Delete` 4214-4215, `GateCommands::Rm/Delete` 4609-4610, `DocCommands::Rm/Delete` 5931-5932 and `LabelCommands::*` 6754-6760. Because dispatch runs after `storage.validate()`, a stub invoked outside a repository reports `REPOSITORY_NOT_FOUND` (exit 3) instead of the hint (verified).

The schema ignores hiding in both places:
- `extract_command_with_path_hidden` never calls `is_hide_set`, so the 13 stubs appear as leaves with description "Hidden stub: …" and `CommandResponse`.
- `extract_flag` never calls it either, so `validate --divergence` appears in `validate.flags`.

clap 4.5.53 provides `Command::is_hide_set` (clap_builder command.rs:4229) and `Arg::is_hide_set` (arg.rs:4499).

#### 8. Human rendering duplicates
Each unified renderer will have to pick one behaviour, and every difference below becomes a REQ-09 change to record.

**Issue list.** 8 sites, 6 formats:

| site | header (`print_info`, suppressed by -q) | row | footer |
|---|---|---|---|
| `run_query_all` ~2048 (issue list / list / query all / bare query) | "All issues (filtered):" | `␣␣{id} \| {title} \| {state:?} \| {priority:?}` | "\nTotal: N" |
| `IssueCommands::Search` ~3221 | "Found N issue(s):" | `{id} \| {title} \| {state:?} \| {priority:?}` (no indent) | none |
| `GraphCommands::Rdeps` ~5653 | "Reverse dependencies of {id}:" | `␣␣{id} \| {title}` | none |
| `GraphCommands::Roots` ~5676 | "Root issues (no dependencies):" | `␣␣{id} \| {title}` | none |
| `QueryCommands::Available` ~6473 | "Available issues (unassigned, unblocked):" | `␣␣{id} \| {title} \| {priority:?}` | Total |
| `QueryCommands::Strategic` ~6606 | "Strategic issues:" | same as Available | Total |
| `QueryCommands::Closed` ~6651 | "Closed issues (Done or Rejected):" | `␣␣{id} \| {title} \| {state:?}` | Total |
| `QueryCommands::Blocked` ~6555 | "Blocked issues:" | priority row + `␣␣␣␣- {reason}` lines | Total |

**Validation report.** The findings line `{❌|⚠} [{rule}] {message}` has 4 copies in `Commands::Validate`: scope ~7364, rule ~7412, per-issue ~7524 and repository ~7774. The differences:
- Scope prints "✓ Scope validation passed" after warnings-only findings. Rule and per-issue print nothing in that case.
- The failure summary goes to stderr in scope, rule and per-issue.
- The repository variant prints its header before the findings, then "\nWarnings: N" and a divergence advisory.
- Exit codes differ: scope exits 4, rule and per-issue exit 1, and repository exits 1 or returns `Err`.

`ConfigCommands::Validate` (~7040) has its own `Errors:` / `✗` / `✓` block. `render_validation_json` (636) is already shared on the JSON side.

**Gate run status.**
- The "Gate '{k}' has not been run yet for issue {id}. Use 'jit gate evaluate' to run it." message has 3 identical copies in `GateCommands::Status` (findings 4693, flat 4804, default 4883) and one human-only copy in `GateCommands::StatusAll` (~4991), where it sits beside "Gate '{k}' status: passed|failed".
- The pass line "Passed gate '{k}' for issue {id}" appears in `GateCommands::Evaluate`, `EvaluateAll` and `EvaluateMany`. 72d89161 removed the already-passed skip and its text (D-24).
- `print_gate_run_details` (1633) and `render_gate_pass_error` (665) are already shared.
- `print_issue_show_human` shows "(last run: never run)" (2101), a separate view.

**Archive.** The execute-summary block (`Archive execution complete: …` plus the `warning:` lines) is byte-identical in `ArchiveCommands::Document` (~6327) and `Container` (~6359). The preview and candidates views already use `output::render_archive_plan` and `render_archive_candidates`.

#### 9. Dead output API
No blanket `allow(dead_code)` exists. `output_macros.rs` has none. `output.rs` has targeted allows on `JsonError` (463), `impl JsonError` (469), `ErrorDetail` (528), `ExitCode` (553), `impl ExitCode` (592) and the constructor `impl JsonError` (1179).

The 8 query structs avoid the lint because they are `pub` items of the library crate (`lib.rs:24 pub mod output`), not because of an allow. The `#[macro_export]` macros avoid it the same way.

Dead items (no use outside their own definition or tests):
- macros `output_message!` and `output_json!` (`output_data!` has 1 use)
- `JsonError::invalid_state`, `invalid_priority` and `gate_validation_failed`; `JsonError::from_code_text` is used only by tests (output.rs 3361 and 3647)
- `OutputContext::print_error`, `is_quiet` and `is_json`
- `ExitCode::all_codes_documentation` (622), which is the only caller of `ExitCode::description`
- the 8 query response structs

### Prior art
- `ProfileCollection<T>` (`profile/application.rs:24`) is a generic count-wrapped envelope with the fixed key `profiles`, and it derives `JsonSchema`. It is the existing convention; a `ListEnvelope` should converge with it (convention-convergence).
- `GraphExportSummaryResponse`/`GraphExportFullResponse` and the issue list, search and blocked unions show how a schema states `--full` variants as a `oneOf` (schema.rs ~600-660 and ~700).
- `render_archive_plan` and `render_archive_candidates` (output.rs:26 and 166), plus `IssueStatusResponse::to_line` and `StateRollup::to_lines`, are human renderers that output.rs already owns.
- `render_validation_json` (main.rs:636) is a shared JSON validation emitter.
- `Cli.quiet` with `global = true` is precedent for a global flag.

### Consumer sweep
**Schema producers and consumers:**
- `crates/jit/src/schema.rs`, with inline tests 1199-1935, including `test_hidden_field_set_for_known_commands`, `test_hidden_propagates_to_children`, `test_schema_exposes_global_flags`, `test_graph_deps_schema_matches_tree_response` and `test_profile_show_schema_is_a_count_wrapped_collection`
- `crates/jit/src/main.rs` (`--schema` in `run()` ~2378), `lib.rs`, `cli.rs`, `domain/event_catalog.rs`, `output.rs`, `visualization.rs`
- `crates/jit/tests/fast_docs_templates/schema_tests.rs`. Its `test_schema_has_json_flags` pins per-command `json` flags; `test_schema_has_standardized_json_output_types` pins type names.
- `crates/jit/tests/cli_repo_workflow/integration_schema.rs`, `help_cross_reference_tests.rs`, `profile_acceptance_tests.rs`, `serve_cli_tests.rs`
- `crates/jit/tests/cli_issue/failure_probe_coverage_tests.rs`. `reflected_json_arm_paths_from` (~73-92) derives the census from schema leaves whose `flags` contain `json`. REQ-15 breaks this derivation, which must then become "every executable leaf".
- `command_exit_code_projection_tests.rs`, `gate_field_contract_test.rs`
- `cli_item_validate/validate_cli_rule_tests.rs`, `scratch_build/repository_state_cutover_tests.rs`
- MCP server:
  - `mcp-server/lib/schema-loader.js` runs `jit --schema`.
  - `lib/tool-generator.js` turns every flag into a tool input property (so `json` becomes a property today), sets `_hidden` from `cmd.hidden`, and builds `outputSchema` from `output.success_schema`.
  - `lib/cli-executor.js:118-141` adds `--json` only when the leaf has a `json` flag. A global flag needs `global_options` or unconditional injection.
  - `test-unit.js`: the json-injection tests at 779-851; the hidden guard at 542-547; `curationCoverage` uncurated and stale at 504-508.
  - `test-integration.js`.
  - `curated-tools.json`: its `exclude` holds 13 "Alias stub" entries, which become stale when the stubs are removed, so the coverage test fails until they are deleted. It also states the policy "A command hidden in the CLI schema is always excluded".
- `.github/workflows/release-artifacts.yml:316-327` runs `jit --schema` and `issue list --json`, and asserts the top-level `count == len(issues)`.
- `.gitignore:17` lists `mcp-server/jit-schema.json`.
- No committed schema snapshot or golden file exists. `web/` and `crates/server` do not consume the schema; the server uses `jit::output::GateRunSummary` (routes.rs:19).

**Success-output contract tests:**
- `cli_issue/list_envelope_tests.rs`. It owns the `{count, <plural key>}` property and lists the keys in its header.
- `cli_issue/`: `issue_create_json_contract_test.rs`, `issue_show_shape_test.rs`, `issue_show_projection_test.rs`, `issue_status_projection_tests.rs`, `issue_update_lean_response_test.rs`, `issue_search_json_tests.rs`, `quiet_mode_tests.rs`, `verb_hint_tests.rs`, `command_alias_tests.rs`, `batch_create_tests.rs`, `snapshot_export_tests.rs`, `payload_stream_purity_tests.rs`
- `cli_query_graph/`: `query_json_tests.rs`, `graph_json_tests.rs`, `label_query_json_tests.rs`, `label_filter_repeatable_query_tests.rs`
- `cli_gate/`: `gate_status_all_lean_response_test.rs`, `gate_status_history_flat_test.rs`
- `cli_repo_workflow/archive_preview_cli_tests.rs`
- The output.rs unit tests (2901+)

`verb_hint_tests.rs` asserts that the hint appears in the `--json` error envelope (lines ~41-91) for all 13 stubs.

**Contract documentation:**
- `docs/reference/cli-commands.md` §"CLI JSON contracts" (lines 5-35). It covers the success payload plus `message`, the failure envelope, and the clap-diagnostic exemption. It does **not** state the list envelope.
- The list envelope is stated in `README.md:165` and `docs/concepts/design-philosophy.md:67`, and per command in cli-commands.md (e.g. 667, 1442, 1491, 1590, 2005), `labels.md:369 and 616`, `profiles.md:374` and `worktree-validate.md:258`.
- Wrong-verb hints are documented in `cli-commands.md:160-197`, which says "Under `--json`, the same hint is in `error.message` with code `INVALID_ARGUMENT`".
- `--json` is already documented as a global option (`cli-commands.md:490`).

### Primitive verification
- clap 4.5.53 provides `global = true`, which is used for `quiet`. `Command::ignore_errors` exists (command.rs:1222), but `Parser::get_matches_with` still returns the error for an invalid subcommand (parser.rs:57-63). Partial matches are therefore unavailable, so a global `--json` cannot be recovered from a failed parse without an argv scan.
- clap also provides `Command::is_hide_set`/`Arg::is_hide_set` and `ArgMatches::get_flag`.
- schemars 0.8.22 supports manual `impl JsonSchema` (`schema_name`, `json_schema(&mut SchemaGenerator)`), which a dynamic-key envelope needs.
- serde cannot rename a field from a type parameter. A dynamic collection key needs a hand-written `Serialize` (`serialize_map`) or `#[serde(flatten)]` of a single-entry map. Const generics over `&'static str` are unstable (`adt_const_params`), so a key must be carried by a marker type: `trait CollectionKey { const KEY: &'static str; }`.
- A key per element type does not work: `MinimalIssue` appears under `issues`, `roots` and `dependents`, so the key must belong to the response, not the item.
- `JsonOutput::with_message` drops the message for non-object data (`null`, arrays). A typed `Success<T> { #[serde(flatten)] data: T, message: Option<String> }` would expose `message` in the derived schema, which no schema declares today. `flatten` fails for non-object `T`, which matters for the gate not-run `null` case.

### Architecture fit
1. **Typed handler responses.** Convert each leaf variant into a tuple variant over a `#[derive(Args)]` struct. REQ-02 and REQ-16 need this conversion anyway.
   - Add a trait in `commands/` or a new dispatch module: `trait CommandHandler { type Response: Serialize + JsonSchema + output::Render; fn run(self, ctx) -> Result<Self::Response>; }`.
   - `main.rs` keeps one emit site, `output::emit(&resp, json, quiet)`. It covers REQ-01, REQ-18 and REQ-21 at once.
2. **Schema without a hand table (REQ-17).** Each group enum exposes `fn visit_responses(v: &mut impl FnMut(&'static str, SchemaFn))`. A declarative macro generates it together with the group's dispatch `match`, so dispatch and schema cannot drift. Variant names map to clap names via kebab-case or an explicit `#[command(name)]`.
   - `generate()` walks `Cli::command()`, skips `is_hide_set()` commands and flags (REQ-20), and looks each path up in the visitor map.
   - A coverage test fails if any visible leaf lacks a response.
   - `hidden_commands()` and `builtin_global_flags()` stay where they are (REQ-05).
   - A lighter alternative: each Args struct implements `CommandHandler` and the group lists `visit::<CreateArgs>()`. That list is co-located and coverage-tested, but it is still a list.
3. **List envelope.** In output.rs, define `pub struct ListEnvelope<K: CollectionKey, T, E = ()> { items: Vec<T>, extra: E }`.
   - `Serialize` writes `count`, then `K::KEY`, then flattens `extra`.
   - A manual `JsonSchema` produces an object with `count`, `K::KEY` and the properties of `E`.
   - Extras are needed for query, issue_id/depth/summary, container/dangling, by_state totals, areas, checkout_store, warnings and the `results` of GateCheckAll.
   - `ProfileCollection<T>` becomes `ListEnvelope<Profiles, T>`.
   - The `--full` variants stay a `oneOf`. That needs `ListEnvelope<Issues, Either>` or two instantiations under a union schema.
   - Exception to decide: `GateCheckAllResponse` counts `gates` but also carries `results`, and `StateRollup.count` counts `by_state`.
4. **Rendering.** `trait Render { fn render_human(&self, out: &OutputContext) -> io::Result<()> }` lives in output.rs. Shared renderers to add: `render_issue_rows(header, rows, columns, footer)`, `render_findings(report)`, `render_gate_not_run(key, id)`, `render_archive_execution(result)`.
5. **Global `--json`.** Add `#[arg(long, global = true)] json: bool` on `Cli`, delete the 108 fields and give handlers a context carrying `json`/`quiet`. That leaves:
   - `verb_hint_error`, which disappears together with the stubs;
   - `emit_top_level_json_error`, which reads `cli.json` threaded out of `run()`. That requires `run()` to return the flag together with the error, for example `run() -> (OutputMode, Result<()>)`.

### Invariant check
- **architecture-dependency-direction:** REQ-01 wants payload types owned by output.rs, but `ItemListResult`, `DocumentListResult`, `AssetListResult`, `ProjectRenderResult`, `InvariantCheckResult`, `ProfileCollection` and `DivergenceResponse` (output.rs, consumed by `commands/graph.rs` and `validate.rs`) are command or domain results serialized directly. An output.rs constructor that wraps them satisfies "type or constructor". Moving them into output.rs would invert the dependency direction for commands.
- **domain-agnostic:** `hidden_commands()` hardcodes the MCP listing policy in the engine schema. It is redundant with `curated-tools.json` (D-18 candidate), but REQ-05 pins it.
- **semantic-types:** filter strings, `assignee` compared raw, `GateEvaluateManyResult.status: String` ("passed").
- **single-source-prose:** the list-envelope rule has no canonical home (README and design-philosophy only). Per-command `[--json]` mentions (51 in cli-commands.md) go stale under a global flag.
- **canonical-cutover:** remove the stubs, the dead query structs and the dead macros outright. Also delete the stale `curated-tools.json` stub entries and the stale `graph_downstream` key.
- **orthogonal-tests:** `list_envelope_tests.rs` owns the envelope property. A `ListEnvelope` unit test should own serialization and schema agreement, and the CLI suites keep wiring only. `failure_probe_coverage` must stop deriving its arm set from per-leaf `json` flags.
- **convention-convergence:** converge `ProfileCollection` with `ListEnvelope`, and converge the `JsonOutput` message injection with the derived schema.

### Overlaps
- **3754095f (emoji markers to words, Ready).** It touches:
  - every validation renderer (`Commands::Validate` ~7361-7800 ❌/⚠/✓, `ConfigCommands::Validate` ✗/✓)
  - `DocCommands::CheckLinks`, `AssetCommands::List`
  - `ClaimCommands::*` (8205-8535), `SnapshotCommands::Export`
  - `IssueCommands::Create`/`Update`, `Commands::Apply`, `PresetCommands::Apply`, `HooksCommands::Install`
  - `render_profile_agreement`, `run_invariant_inner`
  - possibly `DependencyTreeNode::state_symbol` (output.rs:1607, ✓/○)

  This collides directly with REQ-21's validation renderer.
- **8673dc5e (gate status shows manual attestation as never run, Ready).** It touches the `GateCommands::Status` latest-run view (the `get_last_gate_run` `Ok(None)` branch ~4883 and the "not run" message), the history view (`list_gate_runs`), and agreement with `StatusAll`. That is exactly the "gate not run" renderer REQ-21 unifies, and the `null` JSON quirk.
- **34999aad (search rg argv, InProgress by agent:worker).** It touches `search.rs:116-162` and `crates/server/src/routes.rs` ~300-325, but no renderer. It is adjacent to REQ-19 (search through `CommandExecutor`) and to the `Commands::Search` arm (7140), which another area owns.
- **2195c98f (semantic types).** See §6 for the REQ-16 boundary and the REQ-03 `[..8]` renderer sites.

## df129d9e investigation, area 3: executor surface, main.rs bypass, build hygiene

Measured at HEAD `1ed0a049c` (main checkout). `main.rs` is edited concurrently (2d10f1dd in progress), so each site is named by function and match arm as well as by line.

Production-line counter: [prodlines.py](prodlines.py). The method inventory below was produced by a throwaway counter over `impl … CommandExecutor` blocks.

### Claim classification

| # | Claim | Verdict |
|---|---|---|
| 1 | `CommandExecutor` has ~163 pub methods in 28 impl blocks across 15 files | **valid-and-open; the numbers are wrong as stated** |
| 2 | `main.rs` makes storage calls (3831-3985), calls `jit::search` (7150), has its own foreground serve (8053-8088) | **valid-and-open**; the sweep below finds more sites |
| 3 | No `[lints]` table and no `clippy.toml`; the zero-warning bar exists only in the gate script | **valid-and-open** |
| 4 | `tests/common/harness.rs` is `#[path]`-included into each of the 10 suite targets | **invalid-as-stated**: it is included into 4 of the 10 targets |
| 5 | The budget script has no `main.rs` ceiling; REQ-10 needs a reproducible per-module counter | **valid-and-open**; the repo has no counter and the survey method was not recorded |
| 6 | `@/inv/bounded-rust-build-footprint` is enforced | **already-done**: enforced by `@/gate/cargo-ci` through `rust-build-budget.sh` |

#### 1. Executor surface (REQ-03)

Counts at HEAD:
- 179 `pub fn` and 16 `pub(crate) fn` (195 methods).
- 28 `impl … CommandExecutor` blocks across 25 files, not 15.

At the survey commit 21f9e563 the same counter finds 150 `pub` and 11 `pub(crate)` methods (161), 28 blocks and 25 files. "163" therefore counts pub plus pub(crate), and "15 files" is wrong. The surface grew by 34 methods after the survey.

Per-file inventory, grouped by family (`pub` methods; `pub(crate)` methods are marked *(c)*):

- **core** `commands/mod.rs:2082`:
  - `new`, `with_layout`, `with_worktree_paths`, `require_worktree_paths`, `require_layout`, `repository_layout`, `publish_repository_export`, `storage`, `initialize_worktree_identity`
  - *(c)* `plan_repository_export`, `rules`, `effective_rules`, `require_active_lease`
  - `pub config_manager: ConfigManager` field (`mod.rs:2032`)
- **issue lifecycle**:
  - `issue.rs:155` (17): `create_issue`, `list_issues`, `show_issue`, `get_dependencies_enriched`, `issue_status_response`, `issue_children`, `issue_progress`, `update_issue`, `update_issue_resolved`, `confirm_deletion_allowed`, `delete_issue`, `update_issue_state`, `assign_issue`, `claim_issue`, `unassign_issue`, `release_issue`, `claim_next`
  - `labels.rs:5` (3): `get_issue`, `add_label`, `list_label_values`
  - `bulk_update.rs:202` (2): `apply_bulk_update`, `preview_bulk_update`
  - `batch_create.rs:224` (2): `validate_batch_from_json`, `batch_create_from_json`
  - `migrate.rs:19` (1): `backfill_lifecycle_timestamps`. 842b0ac6 REQ-08 deletes it.
- **dependency/graph**:
  - `dependency.rs:91` (6): `add_dependency`, `add_dependency_with_policy`, `remove_dependency`, `add_dependencies`, `add_dependencies_with_policy`, `remove_dependencies`
  - `graph.rs:74` (9): `publish_graph_export`, `build_dependency_tree`, `show_dependencies_with_depth`, `show_rdeps_with_depth`, `show_roots`, `export_graph`, `export_graph_batch`, `resolve_hierarchy_tree`, `detect_divergences`
- **query/search**:
  - `query.rs:8` (15): `query_ready`, `query_blocked`, `query_by_assignee`, `query_by_state`, `query_by_priority`, `query_by_label`, `query_by_labels`, `query_strategic`, `query_closed`, `query_all`, `query_available`, `query_blocked_filtered`, `query_strategic_filtered`, `query_count_by_state`, `query_closed_filtered`
  - `search.rs:5` (2): `search_issues`, `search_issues_with_filters`
  - `events.rs:5` (2): `tail_events`, `query_events`
- **gates**:
  - `gate.rs:694` (19): `add_gate`, `add_gates`, `remove_gates`, `pass_gate`, `pass_all_gates`, `pass_gate_many`, `fail_gate`, `list_gates`, `add_gate_definition`, `define_gate`, `update_gate`, `remove_gate_definition`, `show_gate_definition`, `list_gate_presets`, `show_gate_preset`, `apply_gate_preset`, `list_gate_runs`, `get_gate_run_result`, `create_gate_preset`
  - `gate_check.rs:674` (5 + 3c): `check_gate`, `check_gate_with`, `get_last_gate_run`, `get_last_gate_runs_for_issue`, `get_required_gate_statuses_for_issue`, plus *(c)* `checker_repo_root`, `real_repo_layout`, `run_postchecks`
- **documents** `document.rs:94` (16): `add_document_reference`, `list_document_references`, `resolve_issue_artifact_directory`, `report_artifact_conformance`, `remove_document_reference`, `show_document_content`, `document_history`, `document_diff`, `read_document_content`, `read_document_bytes`, `get_document_history`, `get_document_diff`, `get_linked_document_paths`, `read_path_bytes`, `list_document_assets`, `check_document_links`
- **templates/breakdown**:
  - `template.rs:96` (3 + 1c): `container_anchor`, `apply_template`, `apply_template_with`, *(c)* `template_roles`
  - `breakdown.rs:117` (2): `bracket_breakdown`, `bracket_breakdown_with_template`. 842b0ac6 REQ-01 deletes them.
- **validation** `validate.rs:32` (4 + 6c), bound `IssueStore + RepositoryStateStore`, and `validate.rs:1466` (14 + 1c):
  - `validate_with_fix`, `validate_silent`, `validate_repository_report`, `validate_repository_report_with_divergences`
  - `dangling_link_findings`, `enforcement_drift_findings`, `compute_drift_findings`, `loadable_rule_names`, `validate_integrity_silent`, `evaluate_graph_rules`, `image_plan_content`, `run_rules`, `review_placeholder_findings`, `validate_scope`, `validate_rule`, `explain_rules`, `get_status`, `validate_branch_drift`, `validate_leases`
  - the *(c)* `capture_*` family
- **items/invariants/project**:
  - `item.rs:66` (4 + 1c): `list_items`, `search_items`, `show_item`, `resolve_link_label`, plus *(c)* `item_kinds`
  - `invariant.rs:39`: `check_invariants`
  - `project.rs:116`: `project_render`
- **config** `config.rs:229`: `set_config`, `get_config`
- **snapshot** `snapshot.rs:45`, `:667`: `determine_snapshot_source_mode`, `export_snapshot`
- **archive**:
  - `archive.rs:155` (3): `archive_candidates`, `preview_archive_document`, `preview_archive_container`
  - `archive.rs:504`, `impl CommandExecutor<JsonFileStorage>` (2): `execute_archive_document`, `execute_archive_container`
- **init/profile**, both `impl CommandExecutor<JsonFileStorage>`:
  - `init.rs:73` (4): `initialize_profiled_repository[_from_sources]`, `initialize_fresh_repository[_from_sources]`
  - `profile.rs:538` (29): `resolve_profile_*`, `list_recorded_profiles`, `validate_recorded_profiles`, `capture/pack/add_profile_*` with `plan_*` twins, `show_profiles`, `plan_profile[s][_with_inputs|_from_sources]`, `apply_profile[_with_inputs|_from_sources]`, `diff/reconfigure/upgrade_profiles_from_sources` with `plan_*` twins, `resolve_profile_closure`, `resolve_profile_graph`, `validate_profile_selection`, `apply_profile_package[_with_inputs]`

Families that are not on the executor at all, all free functions taking `&storage`:
- claims: `commands/claim.rs:80-618`, the `execute_claim_*` and `execute_recover` functions
- serve: `commands/serve.rs`, free functions `start_server`, `stop_server`, `server_status`, and others

**Executor state** (`commands/mod.rs:2030-2081`). `CommandExecutor<S: IssueStore>` holds:
- `storage: S`
- `pub config_manager: ConfigManager`
- `OnceLock` caches: `rules`, `effective_rules`, `config`, `namespaces`, `worktree_paths`
- `layout: Option<RepositoryLayout>`

Impl blocks narrow `S` in three ways:
- `IssueStore` (most blocks)
- `+ RepositoryStateStore` (`project.rs:116`, `snapshot.rs`, `validate.rs:32`, `create_issue`'s where-clause)
- concrete `JsonFileStorage` (archive execute, init, profile)

**Cross-family coupling** inside commands/ counts `self.<method>(` calls into another file's methods, production halves only:
- Calls into `mod.rs` (the shared kernel of sessions, rules and config caches) dominate: validate 24, profile 22, issue 14, gate 11, document 10, item 5, bulk_update 5, and 4 or fewer from the rest.
- Family-to-family calls are rare, at most 5 per pair: init→profile 5, breakdown→dependency 3, and 1-2 for the rest.
- So the families are leaves over one kernel (mod.rs plus the validate `capture_*` helpers).

### Prior art

- No prior study on splitting `CommandExecutor`, capability traits, `[lints]`, or extracting the harness. The only hits are the df129d9e entry in `dev/active/9db27a3a-v1-0/9db27a3a-progress.json:194` and an unrelated profiles brief.
- `dev/TESTING.md:81-162` and `dev/archive/4b7c06d0-test-suite-performance/TESTING.md:83-143` document the `#[path = "../common/harness.rs"] mod harness;` convention. REQ-07 must sweep that prose in `dev/TESTING.md`.
- `dev/benchmarks/rust-build-budgets/README.md` derives the three budgets. Its rule is "declared once in the script, derived here", so a new `main.rs` ceiling must add its derivation there too.
- `dev/archive/f1dc843a-redo-mutations/active/footprint.py` counts net production lines per commit and per task. It excludes `*_tests.rs`, `#[cfg(test)] mod x;` files and brace-matched `#[cfg(test)]` items. It does not report per-module totals, and it keeps comments and blank lines.
- `dev/active/df129d9e-rust-code-quality/` exists and is empty.
- No line counter is installed (`tokei`, `scc`, `cloc` are absent).

### Consumer sweep

#### `CommandExecutor` consumers outside `commands/`

Production:
- `crates/jit/src/main.rs:2529`: the one CLI construction, `CommandExecutor::new(storage.clone()).with_layout(..).with_worktree_paths(..)`. Every arm uses it.
- `crates/jit/src/lib.rs:45`: `pub use commands::{CommandExecutor, CommitInfo}`.
- `crates/server/src/main.rs:78`: `Arc::new(CommandExecutor::new(storage).with_layout(layout))`.
- `crates/server/src/routes.rs:29`: `AppState<S>.executor: Arc<CommandExecutor<S>>`. Production handlers (routes.rs before 1048) call read-only methods from 4 families plus the kernel:
  - issue: `list_issues` (:86, :166), `show_issue` (:100)
  - validation: `get_status` (:233)
  - documents: `get_linked_document_paths` (:285), `read_path_bytes` (:371, :687, :741), `read_document_content` (:604), `read_document_bytes` (:634), `get_document_history` (:795), `get_document_diff` (:843)
  - gates: `list_gates` (:864), `show_gate_definition` (:876), `list_gate_runs` (:898), `get_gate_run_result` (:914)
  - kernel: `repository_layout` (:290), `storage()` (:178, feeding `jit::config_manager::get_hierarchy_config`), and the pub field `config_manager` (`get_namespaces` :934/:960/:1005, `get_hierarchy_icons` :973)
- The server also calls `jit::search::search` directly (`routes.rs:309,322`).

Library test code:
- `test_utils.rs:555,924,991`
- `commands/test_helpers::memory_executor` (`test_helpers.rs:180`, cfg test or `test-support`)
- `profile/repository_package.rs:1612,1626,1699` (tests)
- `validation/repository.rs:966-969` (tests)
- server tests: `server/src/lib.rs:99,140`, `routes.rs:1057-1113`, `server/tests/server_integration/{document_api,graceful_shutdown}_tests.rs`

Integration tests:
- 38 files construct an executor with 46 `CommandExecutor::new` calls (`crates/jit/tests/**`, `crates/server/tests/**`).
- `tests/common/harness.rs:15` exposes `pub executor: CommandExecutor<InMemoryStorage>`.

Doc links only: `gate_execution.rs:136`, `output.rs:1870,1915`, `validation/{mod.rs:7,graph.rs:81-94}`, `domain/item.rs:904`, `repository_state/default_rules.rs:6,518`, `errors.rs:528`. Renaming or moving methods breaks these intra-doc links.

#### `main.rs` storage and library bypass (REQ-19)

The table gives each site's function or arm, its line at HEAD, the call, and what the executor would need to provide instead.

| fn / arm | line | call | executor needs to return |
|---|---|---|---|
| `print_apply_result` (called from `Commands::Apply` :4080) | 782 | `storage.load_issue(id)` per created role | `apply_template` returns the created `Issue`s by role (or `TemplateApplyResponse`) |
| `run` prelude | 2488-2491 | `JsonFileStorage::new`, `configure_worktree_paths`, `discover_repository_layout` | a library constructor that the CLI and the server share (0f8efc37 REQ-01) |
| `run` prelude | 2504-2513 | `RepositoryStateStore::open_mutation_session(&storage, ..)`, `storage.open_and_retain_mutation_session` | session bootstrap behind the constructor |
| `run` repo check | 2626 | `storage.validate()` | executor/constructor validation |
| `IssueCommands::Create` | 3044 | `storage.load_issue(&id)` | `create_issue` returns `Issue` (today `(String, Vec<String>)`) |
| `IssueCommands::Create` | 3073 | `storage.list_issues()` → `evaluate_graph_rules` | an executor method returning graph findings for one issue |
| `IssueCommands::Update` | 3578 | `storage.resolve_issue_id` (early id for gate/assignee steps) | removed by 2d10f1dd's single-mutation update |
| `IssueCommands::Update` | 3663 | `storage.load_issue(&full_id)` | `update_issue_resolved` returns `Issue` (today `(String, Vec<String>)`) |
| `IssueCommands::Assign` | 3831, 3838 | `resolve_issue_id`, `load_issue` | `assign_issue` resolves internally and returns `Issue` (today `Vec<String>`) |
| `IssueCommands::Claim` | 3860, 3871, 3907 | `resolve_issue_id`, `load_issue` ×2 | `claim_issue` / `assign_issue` return `Issue` (today `Vec<StorageWarning>`) |
| `IssueCommands::Unassign` | 3928, 3935 | `resolve_issue_id`, `load_issue` | `unassign_issue` returns `Issue` |
| `IssueCommands::Reject` | 3947, 3966 | `resolve_issue_id`, `load_issue` | one reject op returning `Issue` (2d10f1dd REQ-03 creates that op) |
| `IssueCommands::Release` | 3981, 3985 | `resolve_issue_id`, `load_issue` | `release_issue` returns `Issue` (today `()`) |
| `IssueCommands::ClaimNext` | 4020 | `storage.load_issue(&id)` | `claim_next` returns `Issue` (today `(String, Vec<StorageWarning>)`) |
| `GateCommands::Status` (transposed-argument guard) | 4834, 4836 | `executor.storage().resolve_issue_id` ×2 | an executor predicate (e.g. `issue_exists`/`resolve_issue`); also passes through the `storage()` accessor |
| `Commands::Search` | 7150, 7161 | `jit::search::search(&jit_dir, ..)` | an executor search method (overlaps 0f8efc37 REQ-08) |
| `Commands::Search` | 7191 | `storage.load_issue(issue_id)` for titles | search results carry the issue title |
| `Commands::Recover` | 7830 | `commands::claim::execute_recover(&storage, ..)` | claims are free functions over storage, not executor methods |
| `Commands::Claim` arms | 8186, 8229 (+ status/renew/force-evict) | `execute_claim_*(&storage, ..)` | same as `Commands::Recover` |
| `Commands::Serve` fg | 8011-8091 | `read_pid_file`, `is_process_alive`, `find_available_port`, `find_server_binary`, `spawn_with_listener`, `std::process::Command::new`, `child.wait()`, `std::process::exit(foreground_exit_code)` | a `commands::serve::run_foreground(opts) -> ForegroundOutcome` beside `start_server` (`serve.rs:533`) |
| `Commands::Serve` daemon/stop/status | 7905-7995, 8093-8170 | already call `serve.rs` (`start_server`, `stop_server`, `server_status`) | only rendering (bare `json!` payloads) remains, which is REQ-01/18 work |

Other direct library use in `run`:
- `ConfigManager::new(jit_dir).get_write_policy()` (:2334)
- `ConfigManager::invocation_write_override()` (:2409, :7016)
- `discovery::discover_jit_dir` (:2427-2447)
- `WorktreePaths::detect_for_data_root` (:2466)
- `jit::commands::graph::compute_dependency_summary` (:5594, :5622)
- `run()` starts with a `#[cfg(feature = "test-support")]` hook into `jit::test_utils::prepare_nextest_profiled_repository_fixture` (:2356-2362). That is a test-only entry point in the binary.

Other observations:
- `std::fs::read_to_string` at :1409 and :3093 (BatchCreate `--from-json`).
- The CLI's `jit search` runs ripgrep over the data dir only, while the server's `/api/search` covers the worktree plus linked docs. They are two different searches today.

### Primitive verification

#### Lints (REQ-06)

- No `[lints]` or `[workspace.lints]` in `Cargo.toml`, `crates/jit/Cargo.toml` or `crates/server/Cargo.toml`.
- No `clippy.toml`, `.cargo/config`, or `rust-toolchain` file.
- `#![deny(unsafe_code)]` sits on four crate roots only: `crates/jit/src/lib.rs:6`, `crates/jit/src/main.rs:14`, `crates/server/src/lib.rs:6`, `crates/server/src/main.rs:6`. Integration-test crates (`tests/*/main.rs`) are not covered.
- No `unsafe` block exists anywhere in `crates/`. `listenfd` holds the only `FromRawFd`, which lives in a dependency.

Gate and CI bars disagree:
- Gate (`scripts/cargo-ci.sh`, `run_step clippy`): `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo-ci-features` gate: `cargo clippy -p jit --features html,xml --all-targets -- -D warnings`.
- Gate `clippy`: `cargo clippy --lib -- -D warnings`.
- GitHub CI (`.github/workflows/ci.yml:152`): `cargo clippy --all-targets --all-features -- -D warnings`. CI uses `--all-features` and the gate does not.

MSRV:
- `rust-version = "1.97"` in `[workspace.package]`, host cargo 1.97.0.
- `[lints]` and `[workspace.lints]` with `lints.workspace = true` are stable since 1.74, so both forms are available.

Probe in a scratch crate:
- `[lints.rust] warnings = "deny"` alone makes `cargo clippy` fail on clippy warn-level lints (`len_zero`, `useless_vec`). It is equivalent to `-D warnings`, so no separate `[lints.clippy] all = "deny"` is needed.
- It also makes plain `cargo build`/`cargo test` fail on any rustc warning.

Proposed encoding:

```toml
[workspace.lints.rust]
warnings = "deny"
unsafe_code = "deny"
```

Each member then declares `[lints] workspace = true`. After that the four `#![deny(unsafe_code)]` attributes are redundant and should be removed (convention-convergence).

`#[allow(clippy::too_many_arguments)]` appears 16 times, plus 5 other local allows. They keep working under `[lints]`.

#### Harness (REQ-07)

`#[path = "../common/harness.rs"] mod harness;` is declared in only 4 targets:
- `tests/fast_docs_templates/main.rs:6`
- `tests/fast_issue/main.rs:6`
- `tests/fast_rules/main.rs:5`
- `tests/cli_query_graph/main.rs:8`

`cli_query_graph` uses only `harness::seed_memory_issue`.

There are 10 integration targets, with no `[[test]]` entries (auto-discovered `tests/*/main.rs`):
- jit: `cli_gate`, `cli_issue`, `cli_item_validate`, `cli_query_graph`, `cli_repo_workflow`, `fast_docs_templates`, `fast_issue`, `fast_rules`, `scratch_build`
- server: `server_integration`

`tests/common/` contains only `harness.rs` (336 lines, `#![allow(dead_code)]` because it is included per target).

Harness dependencies are all reachable through `jit`'s `test-support` feature:
- `jit::commands::test_helpers::memory_executor`, `cfg(any(test, feature = "test-support"))` at `commands/mod.rs:66-68`
- `jit::test_mode::enable`
- `InMemoryStorage::{new, rooted_at, add_data_file}`, all `pub`

`crates/jit/Cargo.toml` already enables that feature for every test build through the self dev-dependency `jit = { path = ".", features = ["test-support"] }`. Nothing `pub(crate)` or `#[cfg(test)]`-only blocks extraction.

Two shapes:
1. **Move into the library's existing test-support surface** (`jit::test_utils` or `commands::test_helpers`). No new crate, compiled once inside the `jit` lib build that every target already links, and converges with the existing `test_utils.rs` (1,918 lines) / `test_helpers.rs` fixture homes.
2. **New workspace member `crates/jit-test-support`** with `jit = { path = "../jit", features = ["test-support"] }`, added as a dev-dependency of `crates/jit`. This adds a lib unit but no test target, so the integration-target count stays 10. The dev-dependency cycle is legal for integration tests. The crate cannot be used from `jit`'s own `#[cfg(test)]` unit tests because that would link a second `jit` instance.

Current budget line (latest cargo-ci gate run):

```
integration-targets=10/12 active-executables=14 bytes=1743471112/2147483648 profile=line-tables-only dep-features=pruned suite-duration=21661ms
```

#### Budget script (REQ-08)

`scripts/rust-build-budget.sh` (263 lines) declares its budgets as `readonly` constants at lines 41-43:
- `MAX_INTEGRATION_TARGETS=12`
- `MAX_EXECUTABLE_BYTES=2 GiB`
- `MAX_TEST_SUITE_SECONDS=30`

Checks:
- Target count from `cargo metadata`.
- Unique `profile.test` executable bytes from the artifact stream.
- Policy checks against `--root` manifests: debug profile, `jsonschema`/`ureq` features.

Errors accumulate in `errors[]`. On failure the script prints `rust-build-budget: FAILED` and a bullet list on stderr and exits 1, and no summary line is emitted.

On success it prints one line (:263):

```
rust-build-budget: integration-targets=.. active-executables=.. bytes=../.. profile=.. dep-features=.. suite-duration=..
```

`cargo-ci.sh` `summarize_pass budget` greps `rust-build-budget: integration-targets=.*`, strips the prefix and stores the rest in the gate summary. `summarize_fail budget` tails the combined output. Appending a `main-rs-lines=N/MAX` field to the success line keeps that parser working.

A ceiling fits as:
- a fourth `readonly MAX_MAIN_RS_CODE_LINES` beside the others
- an `errors+=` check that reads `$ROOT/crates/jit/src/main.rs`, so it stays injectable through `--root`

Fixture tests live in `crates/jit/tests/scratch_build/rust_build_budget_checker_tests.rs` (fixture `--root`, 14 tests). Their fixture roots would need a `crates/jit/src/main.rs`.

The derivation goes in `dev/benchmarks/rust-build-budgets/README.md`.

Current `main.rs`:
- 9,579 physical lines; `run()` is 6,428 lines (2355-8782); the next largest is `error_to_error_code` at 353 lines.
- `#[cfg(test)]` blocks at 1549-1631 and 8784-9578.
- 7,418 code lines excluding cfg(test) blocks, `//` comments of every kind, and blanks.

#### REQ-10 counting method

The survey method is not recorded anywhere in `dev/`. The survey wording ("excluding `#[cfg(test)]` blocks, test-only sibling modules, doc comments, and blank lines") does not reproduce 63,100 at 21f9e563:

| Definition | Lines at 21f9e563 |
|---|---|
| Doc comments only | 66,372 |
| All `//` comments stripped | 63,281 |
| All `//` comments stripped and modules gated `cfg(any(test, feature = "test-support"))` excluded | **63,118** |

The last definition matches. `prodlines.py` implements it. It excludes:
- blank lines
- all comment lines
- `#[cfg(test)]` and `#[cfg(any(test, feature="test-support"))]` items (brace-matched)
- files declared by such a `mod x;` (including `#[path]`)
- files with an inner `#![cfg(test|test-support)]`
- `*_tests.rs`, `*_proptests.rs` and `*_property_tests.rs` files

Command:

```
python3 dev/active/df129d9e-rust-code-quality/prodlines.py <REV> --strip-line-comments --test-support-as-test
```

Baseline per top-level module (HEAD → 21f9e563):

| module | HEAD | 21f9e563 |
|---|---|---|
| commands | 19,843 | 17,292 |
| repository_state | 10,325 | 7,887 |
| storage | 9,326 | 9,829 |
| main | 7,418 | 6,603 |
| domain | 7,347 | 6,316 |
| profile | 4,370 | 904 |
| validation | 2,763 | 2,614 |
| output | 1,782 | 1,396 |
| cli | 1,661 | 1,519 |
| declarations | 1,303 | 1,270 |
| document | 1,174 | 942 |
| config | 1,152 | 1,205 |
| schema | 912 | 866 |
| graph | 781 | 748 |
| errors | 728 | 730 |
| gate_presets | 421 | 616 |
| gate_execution | 406 | 309 |
| query_engine | 373 | 373 |
| templates | 353 | 353 |
| hierarchy_templates | removed | 349 |
| visualization | 160 | 177 |
| config_manager | 141 | 114 |
| snapshot | 141 | 141 |
| search | 129 | 130 |
| labels | 100 | 100 |
| agent_config | 79 | 79 |
| output_macros | 68 | 68 |
| runtime_defaults | 47 | 47 |
| lib | 42 | 40 |
| type_icons | 42 | 59 |
| build_info | 23 | 42 |
| tar_format | 12 | n/a |
| test_mode | 3 | n/a |
| **TOTAL** | **73,425** | **63,118** |

This baseline is a moving target: 2,530 commits since the survey, and 842b0ac6 deletes code. Record the "before" figure at this epic's first commit.

### Architecture fit (REQ-03 shapes)

**A. Per-family facade structs borrowing the executor.** `executor.issues() -> IssueOps<'_, S>`, `.gates()`, `.documents()`, `.queries()`, and so on. Methods move from `impl CommandExecutor` into `impl<S: IssueStore> IssueOps<'_, S>`. The kernel (`mod.rs` sessions, rules and config caches, `validate.rs` `capture_*`) stays on `CommandExecutor` as `pub(crate)`.

- Fits the measured coupling: families call the kernel, rarely each other.
- Keeps per-block storage bounds (`JsonFileStorage`-only profile/init/archive and `RepositoryStateStore` blocks become bounds on the facade impl).
- Lets the server `AppState` hold narrow handles, or keep an `Arc<CommandExecutor>` and pass `executor.documents()` to handlers.
- Cost: every call site changes from `executor.x()` to `executor.family().x()` (main.rs, 38 test files, server). A facade alone does not stop a consumer from holding the whole executor.

**B. Per-family extension traits implemented on `CommandExecutor<S>`.** `trait IssueCommands`, `trait GateCommands`, and so on. Consumers take `&impl GateCommands`.

- Narrowing is enforced by the signature, and fakes become possible.
- Costs:
  - Generic-bound proliferation: `S`-dependent bounds become trait where-clauses, and the `JsonFileStorage`-only blocks need separate traits.
  - Every caller must import the trait.
  - Methods that are generic or `impl Trait` hinder `dyn` use.
  - No existing fake implementations need it: the server tests use `InMemoryStorage`.
  - It risks a parallel abstraction (convention-convergence).

Recommendation: A. Also bring claims (`commands/claim.rs` free functions) and serve into the same family scheme, and make `config_manager` non-`pub` (the server reads it at 4 sites).

### Invariant check

`@/invariant/bounded-rust-build-footprint`:
- Registry entry: `.jit/invariants.toml:82-86`, `kind = "enforced"`, `enforced-by = "@/gate/cargo-ci"`.
- Gate: `.jit/gates.toml:23-37`, `./scripts/cargo-ci.sh`, whose `budget` step runs `scripts/rust-build-budget.sh`.
- Policy tests mirrored in `crates/jit/tests/scratch_build/{build_profile_policy_tests,dependency_feature_policy_tests,rust_build_budget_checker_tests}.rs`.
- Derivation: `dev/benchmarks/rust-build-budgets/README.md`.
- Cited in `crates/jit/Cargo.toml` (glob dep comment), `crates/server/tests/server_integration/main.rs:5`, `dev/TESTING.md:260`, `profiles/jit-dogfood/manifest.toml:427-430` and `docs/reference/rules-and-gates.md:38`.
- REQ-08 extends what this invariant's budget script enforces. The statement names targets and executable bytes, not source lines, so the `main.rs` ceiling sits beside it as gate policy unless the invariant text is widened.
- Related invariants:
  - `@/inv/convention-convergence` favours moving the harness into the existing test-support surface, and `[lints]` replacing the per-root `#![deny]`.
  - `@/inv/essential-code-prose` covers the doc links to executor methods listed above.

### Overlaps

| Issue | State | Overlap |
|---|---|---|
| 2d10f1dd (atomic update/reject) | InProgress, agent:worker, edits in another worktree | Its Notes put "one executor operation" for update and reject in the command layer. That removes the `IssueCommands::Update` early `resolve_issue_id` (:3578) and turns the `Reject` arm (:3947-3966) into one call. The REQ-19 "return the updated `Issue`" change touches the same two arms and executor methods, so sequence REQ-19's issue-arm work after 2d10f1dd lands and have that new op return `Issue`. |
| 16d63038 (full-length id resolution) | Ready | REQ-03 makes one shared normalisation/prefix function behind `resolve_issue_id` in storage (`json.rs:1264`, `memory.rs:848`), gate_check, preflight, dependency and template. REQ-19 moves the CLI's `resolve_issue_id` calls (Assign/Claim/Unassign/Reject/Release, GateStatus guard) behind executor methods. No double ownership if 16d63038 owns the resolver and df129d9e only relocates calls; sequencing either way works, but REQ-19 tests should use the resolver 16d63038 lands. |
| 842b0ac6 (dead code/shims) | Backlog, being planned | Deletes executor methods: `bracket_breakdown*` and `template_roles` (REQ-01), `backfill_lifecycle_timestamps` and the `Commands::Migrate` arm (REQ-08), `validate_integrity_silent` (REQ-01), `add_gate_definition` (test-only, REQ-10). It also removes `main.rs` regions (config show 6800-6900, claim turbofish 8278/8353/8519), output dead code (REQ-15), and `allow(dead_code)` in `memory.rs`. It shrinks the REQ-03 surface and moves the REQ-10 baseline, so land it, or freeze the before-count, first. Its investigation (`dev/active/842b0ac6-greenfield-cutover/investigation.md:211,296,394`) already flags `main.rs` contention with df129d9e. |
| 0f8efc37 (server on core mechanisms) | Backlog | REQ-01: one core constructor for storage, layout and worktree paths, used by both CLI and server. That is the `run()` prelude 2466-2531 plus server `main.rs:68-78`. REQ-07: every server handler goes through one executor helper, which touches `AppState.executor` and the REQ-03 shape. REQ-08: exactly one issue search, with ripgrep only over linked docs. That directly overlaps REQ-19 "search runs through `CommandExecutor`" (`Commands::Search` :7150 and server `routes.rs:309,322`). REQ-09: one document read set. REQ-10 removes the server `/status` route (`get_status` consumer). **Double-ownership risk on search and on the constructor.** |
| 2195c98f (semantic types) | Backlog | REQ-01/02 retype `IssueStore` (`load_issue(&IssueId)`, `resolve_issue_id` via `IssueIdPrefix`). REQ-19's new executor return types (`Issue`) and any facade/trait signatures (REQ-03) are the APIs it retypes. Do the REQ-03 reshaping first and 2195c98f retypes once, or vice versa, but not interleaved. REQ-18 raw strings at boundaries applies to the new executor entry points. |

## Audit findings folded into this epic

The repository-wide v1.0 code audit of 2026-10-20 produced these rows, which the epic's REQ-11 to REQ-21 encode. Manifest entries cite them by id; the investigation sections above verify each against the code. Where an audit count differs from a measurement above, the measurement is authoritative.

| Id | Finding | Locations | Proposal |
|---|---|---|---|
| A3 | CLI error boundary: ten JSON-error emitters, 44 inline exits and 76 `process::exit` calls, per-site fallback codes, and a 355-line classifier living in the binary that the server copies | main.rs:55-2263; output_macros.rs:68; server routes.rs:350 | `JsonError::from_error` in the library; handlers return `Err` and `main` renders once |
| A24 | CLI arguments: 108 per-command `json: bool` fields (argv is scanned for `--json`); filter strings parsed ad hoc at about 20 sites; no flattened filter arguments | cli.rs; main.rs:1388,2264 | A global `--json`; flattened `IssueFilterArgs`; clap `value_parser` |
| A25 | Output contract: the command→output-type table covers 46 of 120 leaves; serve, archive and batch-create bypass the envelope; 33 hand-written `count` structs | schema.rs:456,894; output.rs:1353; main.rs:3107,6313,7919 | Typed response per handler plus a registry; a generic `ListEnvelope` |
| A26 | main.rs bypasses `CommandExecutor`: it resolves and reloads issues after mutations, calls search directly, and has its own foreground-serve path | main.rs:3831-3981,7161,8010-8096 | Mutations return the updated `Issue`; `serve --fg` moves into commands |
| S13 | Hidden CLI stubs (14) duplicate `COMMAND_HINTS`, so the hint can never fire; the schema ignores `is_hide_set`; `hidden_commands()` is a hand-kept list | main.rs:1324-1387; cli.rs:409,1087; schema.rs:228,286 | Delete the stubs; derive `hidden` from clap |
| S14 | main.rs rendering duplication: issue-list rendering 5 times, the validate report block 3 times, "gate not run" 4 times, identical archive renderers | main.rs:2002,3184,4690-4988,6324-6585,7337-7462 | Render helpers in output.rs; clap `conflicts_with` |
