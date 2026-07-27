# Code-Smell Hunt Report: Boundary Leaks & Stringly-Typed Code

> **Spec document for epic `7095769d`** (Code-smell cleanup). Produced by the `jit-smell-hunt`
> multi-agent workflow: 6 Sonnet finders (one per code area) → per-finding Opus adversarial
> verification → Opus synthesis. 39 agents total; 27 findings survived verification (consolidated
> into 24 entries below). Each child task of the epic maps to one of the fix clusters here.

24 confirmed smells (all adversarially verified true): 5 boundary leaks and 19 stringly-typed. The dominant pattern is **closed-set domain enums encoded as `String`** (missing `FromStr`/`ValueEnum`, `to_lowercase()` dispatch, Debug-formatted map keys) and **storage-internal paths/filenames hardcoded in command and output layers**. Worst offenders by file: `main.rs` (6 findings, the monolith concentrates error-classification and gate/blocked-reason string parsing), `output.rs`/`commands/graph.rs` (the `by_state` stringly map spanning producer→DTO→consumer), `validation/` (4 findings), and `config.rs` (2 enum-as-String configs).

## Highest leverage fixes

1. **`impl FromStr for GateStage` and `GateMode` in `domain/types.rs`** (mirror existing `State`/`Priority`/`ContentFormat` impls). Kills 3 sites at once: `commands/gate.rs:608-617`, `main.rs:1886-1906`, and unblocks the typed-field fix below. Pair with `#[arg(value_enum)]` on the CLI so clap rejects bad input at parse time.

2. **Type `GateDefinition.stage`/`.mode` as `GateStage`/`GateMode`** (`output.rs:1222-1244`) instead of `String`. Add `JsonSchema` to `GateMode` (`GateStage` already has it). Deletes the fragile `format!("{:?}",…).to_lowercase()` conversion and collapses the duplicated enum→string match arms at `main.rs:2755-2762` and `2814-2821`. Combined with fix #1 this removes the entire `GateStage`/`GateMode` stringly cluster.

3. **Key `by_state` by `State`, not `String`.** Change `DependencySummary.by_state` (`output.rs:727-733`) and `IssuesInfo.states` (`snapshot.rs:56`) to `HashMap<State, usize>`; update producers `commands/graph.rs:200-222` and `snapshot.rs:274` to `entry(node.state)`, and consumer `main.rs:2672` to `.get(&State::Done)`. `State` is `Copy + Eq + Hash + Serialize(snake_case)`, so JSON stays valid — and this also fixes the latent Debug-vs-serde divergence (`"inprogress"` vs `"in_progress"`). Resolves two findings spanning four files.

4. **Introduce typed `thiserror` variants for `IssueNotFound`/`GateNotFound`/`CycleDetected` and downcast instead of substring-scanning.** `GraphError::CycleDetected`/`NodeNotFound` already exist (`graph.rs:26-33`) and reach `main.rs` downcastable; add `IssueNotFoundError`/`GateNotFoundError` at the `anyhow!` origins in `storage/memory.rs`. Extends the existing downcast chain and removes the brittle `error_msg.contains(...)` dispatch at **5 sites**: `main.rs:74-104` (`error_to_exit_code`) and `main.rs:173, 1298, 1737-1742, 2231-2236`. Single highest-count stringly cluster.

5. **Type config enums and add `Deserialize` + `FromStr`.** `WorktreeConfig.mode`/`enforce_leases` → `WorktreeMode`/`EnforcementMode` (`config.rs:741-795`) collapses **four** duplicated dispatch tables (incl. `EffectiveConfig` env-var branches at 1183-1193, 1226-1235) and fixes a real TOML-vs-env case-sensitivity divergence. Same pattern for `ItemKindConfig.scope` → a `KindScopeConfig` enum (`config.rs:322`), mirroring the sibling `SourceOfTruth` field so bad tokens fail at parse time.

6. **Add `const TYPE_NAMESPACE` + shared `fn type_label_value(&[String]) -> Option<&str>` in `labels.rs`/`domain`.** Promote the already-correctly-shaped private helper at `breakdown.rs:602-609` and reuse it across the 5 sites (`issue.rs:37`, `batch_create.rs:555`, `breakdown.rs:607/619/665`, `validate.rs:607,827`), replacing the raw `starts_with("type:")`/`strip_prefix("type:")` bypasses.

## Boundary leaks

| File:lines | Sev | Fix |
|---|---|---|
| `commands/mod.rs:964-974` (`check_active_lease`) | medium | Stop hardcoding `claims.index.json` + manual read/deserialize; delegate to `ClaimCoordinator::load_claims_index()` as `claim.rs:600` already does. |
| `commands/validate.rs:1653-1664` (`check_worktree_exists`) | medium | Remove hardcoded `.jit/worktree.json` + `"worktree_id"` field; add a read-only `read_worktree_id(worktree_root) -> Result<Option<String>>` in `storage::worktree_identity` deserializing into `WorktreeIdentity`. |
| `storage/claim_coordinator.rs:963,1052,1066` (also `lock_cleanup.rs`, `worktree_identity.rs`) | low | Storage emits `eprintln!` diagnostics; propagate a typed `Vec<StorageWarning>` (e.g. `SequenceGap`, `IndexRebuilt`, `TempCleanupFailed`) up to the output layer, or route via `tracing::warn!`. |
| `main.rs:323-326` (`print_gate_run_details`) | low | `.jit/gate-runs/{run_id}/result.json` path literal duplicates `storage/json.rs:662-684`; expose a `result_path()` from `JsonFileStorage` and print that. (Do NOT put a path on the pure `GateRunResult` domain type.) |
| `validation/serialize.rs:122-222` (`scaffold_default_rules`, `regenerate_type_hierarchy_schema`) | low | Raw `std::fs` writes + hardcoded `.jit/` paths bypass storage; route persistence through a storage write method (a jit-root-relative variant) while validation produces only content. |

## Stringly-typed / missed type safety

Covered by Highest-Leverage fixes #1–#6 (not repeated): `commands/gate.rs:608-617`, `main.rs:1886-1906`, `output.rs:1222-1244` + `main.rs:2755-2821` (GateStage/GateMode); `output.rs:727-733` + `commands/graph.rs:200-222` + `snapshot.rs` + `main.rs:2672` (by_state); `main.rs:74-104,173,1298,1737-1742,2231-2236` (error classification); `config.rs:741-795` + `config.rs:322-323` (config enums); `breakdown.rs`/`validate.rs`/`issue.rs`/`batch_create.rs` (`type` namespace).

Remaining:

| File:lines | Sev | Fix |
|---|---|---|
| `main.rs:3394-3406` (blocked-reason re-parse) | medium | `query_blocked` formats `"dependency:…"`/`"gate:…"` then CLI re-parses via `splitn(2,':')` with a silent `_ => Dependency` fallback. Return `Vec<(Issue, Vec<BlockingReason>)>` with a `BlockingReason { Dependency{…}, Gate{…} }` enum from `domain/queries.rs`; CLI maps to `BlockedReason` without splitting. |
| `commands/document.rs:753-776` (`check_document_links`) | medium | `scope == "all"` / `strip_prefix("issue:")` inline protocol. Introduce `DocumentScope { All, Issue(String) }` with `FromStr`, parse at CLI call site, match — mirroring the existing `SnapshotScope` pattern (`snapshot.rs:140-183`). |
| `validation/graph.rs:98-102` (`is_config_error`) | medium | Detects config-error kind via `message.starts_with(CONFIG_ERROR_PREFIX)`; this drives a real transition guard (`commands/mod.rs:692-708`). Add `is_config_error: bool` (or `FindingKind` enum) to `GraphFinding`, set in `config_error()`, read the field. |
| `validation/report.rs:39-49` (`ReportedFinding.severity`, `RuleOutcome.severity`/`scope`) | medium | String fields where `Severity`/`Scope` enums exist; predicates round-trip via `.token()`. Derive `Serialize(rename_all="snake_case")` on both enums, type the fields, reduce `is_error()`/`has_errors()` to `== Severity::Error`. JSON stays byte-identical. |
| `query_engine/parser.rs:24-29` (`QueryCondition::State`/`Priority`) | medium | Raw `String` for closed enums; evaluator's `from_str().unwrap_or(false)` fails open (`state:notastate` silently matches nothing). Change variants to `State(domain::State)`/`Priority(domain::Priority)`, parse in `parse_condition` with `map_err`. Leave `Label`/`Assignee` as String (open values). |
| `commands/graph.rs:172-185` (`export_graph(format: &str)`) | low | `format.to_lowercase()` match over dot/mermaid/json. Add `GraphExportFormat` enum (clap `ValueEnum`) consumed directly; clap rejects bad input at parse time. |
| `validation/rules.rs:439-494` (`StatePredicate`) | low | Stores `Vec<String>` for closed `State` set; `matches()` round-trips `State→token→.to_lowercase()`. Parse tokens to `BTreeSet<State>` at `validate()` time (via the predicate's own whitelist, not `State::from_str` which accepts extra aliases); `matches()` becomes a `contains` check. |
| `query_engine/parser.rs:129-136` + `lexer.rs` (`Token::Filter` field) | low | `match field.as_str()` over state/label/priority/assignee literals, validated nowhere structurally. Add `FilterField` enum + `FromStr` parsed in the lexer; parser match becomes exhaustive. |
| `domain/types.rs:205-207` (`Issue.created_at`/`updated_at`) | low | Raw RFC-3339 `String` while sibling `GateState.updated_at` and all Event/`GateRunResult` timestamps use `DateTime<Utc>`. Change both to `DateTime<Utc>` (chrono serde default emits RFC 3339); constructors already build a `DateTime`. |
| `domain/types.rs:163-168,184` (`assignee`/`updated_by`) | low | `Option<String>` holding `{type}:{identifier}`, validated inconsistently (`bulk_update.rs:530` checks, `issue.rs:554`/`claim.rs:99` don't). Introduce `Assignee { kind: String, identifier: String }` with `FromStr`→typed error / `Display`; use `Option<Assignee>`. Centralizes the duplicated split. |
| `domain/projection.rs:150-160` (`group_labels`) | low | Inline `split_once(':')` duplicates `crate::labels::parse_label` (already re-exported into `domain/`). Fold over `parse_label(label).ok()`. |
| `gate_execution.rs:52-59` (`JIT_STAGE` string) | low | Inline `GateStage→"precheck"/"postcheck"` match is a second source of truth vs serde. Add `as_str()`/`Display` on `GateStage` in `domain/types.rs`; use `stage.as_str()`. |