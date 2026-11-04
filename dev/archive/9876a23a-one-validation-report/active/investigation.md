# Investigation: Compute one validation report and derive every validation view from it (9876a23a)

> Grounding for planning node 2df01664. Two scoped investigations against current `main`, after 842b0ac6 and 3d9a0ce3 changed validation code. Timings were taken with the installed release binary under concurrent host load, so absolute figures are inflated and ratios hold.

## Part: The one report, its selections and its passes

## 9876a23a investigation — one report and its views (REQ-01..07, 10, 11, 12)

Read at HEAD `71ed9d4ce` (validate.rs last touched by `c4762af0e`). Line numbers below are current; the container's background numbers are stale by 400–800 lines in `commands/validate.rs`.

### 1. Pipeline map at current main

#### 1.1 Whole repository: `validate_repository`

Entry `validation/repository.rs:152` (`validate_repository`) and `:183` (`validate_repository_with_materializations`). CLI path `main.rs` validate arm → `CommandExecutor::validate_repository_report_with_divergences` (`commands/validate.rs` `capture_validation_report`, ~`:541`) → one mutation session → `capture_repair_plan` (`:650`) → `capture_validation_base` → `capture_base_reading` (`:211`) → `capture_proposed_base_inner` (`:365`) then `grow_discovering` with `discover_document_references` (`:932`).

Inputs: the captured image only, except `validate_machine_local_claims` (`repository.rs:540`), which does live filesystem I/O under `.git/jit` through `crate::commands::validate_claims_index_at` and reads `crate::test_mode` (env). `collect_rule_findings` reads the wall clock (`chrono::Utc::now()`, `repository.rs:589`).

Passes, in order:

| Pass (`RepositoryValidationPass`) | Code | Parses / reads | Output |
|---|---|---|---|
| EffectiveConfig | `:191-203` (`validate_item_kinds`) | declarations from `validation_declarations_from_image` (`repository_state/mod.rs:507`) | structural error |
| RulesAndSchemas | `:206-220` | same | `rules-file` finding (unattributed) |
| Gates | `:223` | same | — |
| Records | `load_records` `:358-415` | index via `RepositoryIndex::parse`; every issue via `serde_json::from_str`; `events.jsonl` via `parse_known_events` | `Records { issues, event_count }` or structural error |
| RepositoryIntegrity | `validate_integrity` `:417-534`, `validate_machine_local_claims` `:540` | records + document evidence from image + live claims index | structural error |
| NamespaceAndHierarchy | `collect_rule_findings` `:554-598` | `evaluate_local` per issue (one `SchemaEngine` each), `evaluate_graph` over all, `project_plan_content` `:657` | attributed findings |
| (unnamed) | `collect_enforcement_drift_findings` `:600`, `collect_review_placeholder_findings` `:631` | declarations | unattributed findings |
| ItemLinks | `collect_item_link_findings` `:697-843` | `resolve_item_kinds` once; builds one `HashSet<String>` of canonical addresses once per call | attributed `dangling-item-link` findings |
| Projections | `validate_materializations` `:845` | plan from `derive_materialization` (resolves item kinds again, `repository_state/materialize.rs:132,242`) | structural error |

Output: `Result<RepositoryValidationReport, RepositoryValidationFailure>` (`:52,70`); the failure carries the partial report. The executor adds `captured_divergences` (`validate.rs:706`), which parses every issue again through `captured_active_issues` (`commands/mod.rs:2062`).

#### 1.2 Per-issue and whole `run_rules` (`validate.rs:1579`)

Reads through storage and executor caches, not the image: `effective_rules()`, `repo_content_format()` (`commands/mod.rs:2634`), `storage.list_issues()`, `storage.resolve_issue_id`/`load_issue`. Passes:
- `evaluate_local` for the issue (or every issue).
- `evaluate_graph_rules` (`:1493`), which also calls `image_plan_content` (`:1549`) → `capture_validation_image` — **a full whole-repository capture** — then `plan_content_from_image` (`:986`), which re-parses index, issues, config and templates from that image.
- Executor dangling copy `dangling_link_findings` (`:1261`), resolving each qualified label via `resolve_link_label` (`:1314` → `commands/item.rs:526` → `show_item` `:327`).
- Whole arm only: `enforcement_drift_findings` (`:1364`) → `compute_drift_findings` (`:1404`) → `loadable_rule_names` (`:1455`, live `rules_path.exists()` at `:1456`, `ruleset_store::load_ruleset` from disk) and `storage.load_gate_registry()`; and `review_placeholder_findings` (`:1705`, `storage.load_gate_registry()`).
- Unloadable-rules arm (`:1593-1615`): drift + `rules-file` for both arms; placeholder only for the whole arm.

Per-issue filter: findings with `issue_id == I`, plus every `is_config_error()` finding of a rule whose selector matches I (from any firing issue). Drift is excluded unless rules are unloadable.

#### 1.3 `validate_scope` (`validate.rs:1772`)

Storage + caches, plus `image_plan_content` (another full capture) when graph rules apply. Slice = `bracket_scope_ids` (`domain::queries`), breakdown type from the cached template registry. Local rules per slice issue; graph rules filtered by `severity != Off`, `!is_repo_wide_at_transition()`, and selector matching some slice issue; evaluated by `evaluate_graph_scoped(rules, slice, all, …)` (`validation/graph.rs:432`), so the graph context is **the slice** and the resolution index is everything. Then executor dangling over the slice and executor drift. No review-placeholder.

#### 1.4 `explain_rules` (`validate.rs:1934`)

Storage + caches + a capture through `evaluate_graph_rules`. Local findings for the issue; graph messages = attributed to I **or any config error** (`:1975-1980`). Every rule becomes a `RuleOutcome` with `match_failure`/`render_selector`. No dangling, drift or placeholder.

#### 1.5 Single rule: `validate_rule` (`validate.rs:1900`) → `rule_validation_report` (`:1063`)

Image only (already REQ-02 compliant). Parses issues (`captured_active_issues`), `declarations_from_image` and `assemble_config` (config parsed twice), then `plan_content_from_image` (index, issues, config, templates parsed again). `select_graph_rule` (`graph.rs:479`) turns a missing, off, non-graph or non-matching rule into an error finding. `evaluate_graph_rule(rule, [issue], ctx)` with graph context = all issues. Findings keep their own attribution (a coverage finding names the container, not the firing issue).

#### 1.6 Gate checkers (`commands/gate_check.rs`)

Dispatch `execute_builtin_checker_with_repository_view` `:1027`:
- `RepositoryValidation` `:1045` → `validate_repository(image)`; full derivation including `derive_materialization`. `gate_findings_from_report` `:634`.
- `IssueValidation` `:1051` → `captured_issue_rule_report` `:445`. It repeats the per-issue logic over the image. It errors when rules are unloadable (`declarations_from_image` `?`), where the CLI reports `rules-file`. It then calls `repository_rule_report` (`:435`, full `validate_repository`, so a second issue parse and a second materialization derivation) only to keep the `dangling-item-link` findings for I (`:504`).
- `RuleValidation` `:1060` → `rule_validation_report` (shared with the CLI).
- `LabelTargetValidation` `:1074-1120` → `captured_scope_rule_report` `:515`. It repeats the scope logic and then `repository_rule_report` (`:583`) to keep dangling findings in the slice plus all drift.
- Setup prelude (issues, `declarations_from_image`, `assemble_config`, content-format chain, hierarchy, `plan_content_from_image`) appears in `:451-486`, `:522-567` and `validate.rs:1081-1117`.

#### 1.7 Executor-side copies (REQ-05)

| Pass | Executor copy | Validation-layer original | Divergence |
|---|---|---|---|
| dangling-item-link | `validate.rs:1261-1335` | `repository.rs:697-843` | executor resolves per label through `show_item` (storage, alias-canonicalising `canonical_kind_name`); the report pass uses one address set built from the image, and its `@/{kind}` formatting (`repository.rs:786`) does not canonicalise kind aliases. That is a suspected per-view disagreement for alias-form labels, found by reading the code and not by running it. |
| enforcement-drift | `validate.rs:1364-1453` (+ `loadable_rule_names` `:1455`) | `repository.rs:600-629` | executor treats an unparseable `gates.toml` as `SourceState::Unloadable`; the report path fails declaration loading instead. Executor reads live disk. |
| review-placeholder | `validate.rs:1705-1740` | `repository.rs:631-655` | identical logic, storage vs image source. |

Other consumers of the executor copies: `check_invariants` (`commands/invariant.rs:60`) uses `compute_drift_findings`, and `tests/cli_item_validate/invariant_check_cli_tests.rs:225` (`test_check_reports_unloadable_gate_registry`) pins the tolerant-gates behaviour. Issue create (`main.rs:2429`) uses `evaluate_graph_rules`. `gate_check.rs:2896` (test) calls `review_placeholder_findings`.

### 2. Claim classification

| Claim / REQ | Verdict | Evidence |
|---|---|---|
| BG executor copies (`run_rules`, `--scope`, `--explain` via storage + caches, three copied passes) | valid-and-open | §1.2–1.4, §1.7. Lines now `:1579`, `:1772`, `:1934`, `:1261`, `:1364/1404`, `:1705`. |
| BG gate-checker copies, `validate_repository` called only to keep subsets, `rule_validation_report` third prelude | valid-and-open | `gate_check.rs:435,445,515,504,583`; `validate.rs:1063` |
| BG inverted dependency (`repository.rs` imports constants, `find_planning_node`, `planning_node_plan_path`, `validate_claims_index_at` from `commands`) | valid-and-open | `repository.rs:542,622,646,682,683,769,832` (prod); tests also at `:902-966,1006,1236,1263` |
| BG per-issue 3× slower; `show_item` per label | valid-and-open (mechanism confirmed). Re-timed with the installed release binary, which may lag HEAD: whole 1.51 s, `<id>` 4.79 s, `--scope` 1.16 s, `--explain` 0.51 s, `--rule` 0.45 s over 1,567 issue files. | `validate.rs:1314`; `item.rs:72` (regex per kind per call via `resolve_item_kinds`), `item.rs:327-420` (`project_items()` re-indexes all project sources per project-scoped label; `resolve_issue_id` + `load_issue` per issue-scoped label) |
| BG repeated parsing: 4 parses, 3 loaders, 3 index readers | valid-and-open, restated. Whole `jit validate` parses issues 3× through 3 loaders: `effective_issues` (`validate.rs:789`, in capture `:478`), `load_records` (`repository.rs:358`) and `captured_active_issues` (`mod.rs:2062`, via `captured_divergences`). Index readers: `effective_index_ids` (`validate.rs:774`, private `IndexIds` struct), `RepositoryIndex::parse` (`repository.rs:359`), `storage::json::parse_repository_index` (`mod.rs:2069`). The gate `IssueValidation` path adds a 4th issue parse. | — |
| BG `SchemaEngine` per issue (`local.rs:317`) | valid-and-open | `local.rs:317`. `rule_schema` desugar also reruns per issue (`local.rs:302-305`). |
| BG plan-document resolution written three times | valid-and-open | `validate.rs:949` (`plan_document_paths`), `validate.rs:986/1008` (`plan_content_from_image`/`_with_issues`), `repository.rs:657` (`project_plan_content`). `plan_content_with_issues` also serves mutations: `commands/mod.rs:2686,3176`, `dependency.rs:403`, `bulk_update.rs:377`, `issue.rs:90` (`plan_content_from_image`). |
| BG content-format chain at 9 sites | invalid-as-stated: there are **12** `Option<ValidationConfig>` → Markdown chains plus **1** re-implementation of the parse. Chains: `commands/mod.rs:1057,1189,1443,1478,2464,2638`, `validate.rs:1080`, `repository.rs:564,710`, `gate_check.rs:461,541`, `template.rs:1123`. Re-implementation: `repository_state/projection_render.rs:245-260` (own case-insensitive token match, own error). | — |
| BG strictness chain at 3 sites | invalid-as-stated: **4** sites: `commands/mod.rs:1063,1196`, `template.rs:1127`, `bulk_update.rs:629`. | — |
| REQ-01 one pure report function | valid-and-open: `validate_repository` is image-based but impure (live claims index, env `test_mode`, wall clock). Two entry functions exist (`validate_repository`, `validate_repository_with_materializations`). | `repository.rs:152,183,540,589` |
| REQ-02 views = filtered report, image only | valid-and-open, except `--rule`, which already reads only the image | §1.2–1.5 |
| REQ-03 checkers = same subset as `jit validate` | valid-and-open: CLI per-issue and gate `IssueValidation` disagree on unloadable rules; CLI `--scope` and gate scope agree in logic but compute it twice | §1.6 |
| REQ-04 one selection-contract test | valid-and-open: no such test; no image generator exists | §4.4 |
| REQ-05 no executor copies; no checker calls whole validation for a subset | valid-and-open | §1.7, `gate_check.rs:504,583` |
| REQ-06 validation imports nothing from `commands` | valid-and-open; 7 production import sites (§2 row 3). `find_planning_node` depends on `commands::template_expand::declared_container_label` (`template_expand.rs:626`, `pub(super)`), which depends on `InterpolationContext` (`:528`). | — |
| REQ-07 one address index per report, kinds once | already-done for the report's item-link pass (`repository.rs:703,713-749`); open for the executor path (removed by REQ-05) and for "kinds once per report" (resolved again in `materialize.rs:132,242` during the same validate) | — |
| REQ-10 one parse, one loader | valid-and-open | row 5 |
| REQ-11 schema compiled once per report | valid-and-open | `local.rs:317` |
| REQ-12 one plan resolver; one accessor each | valid-and-open (counts corrected above) | — |

Counts: already-done 0 (REQ-07 is partly done); valid-and-open 16; invalid-as-stated 2 (the content-format count and the strictness count). Not re-measured: the background's absolute timings, which were taken on another copy.

### 3. Prior art

- Charter `dev/vision/9db27a3a-charter.md:384-396` (D-20): "Validation computes one report over the captured repository and filters it for an issue, a scope, an explanation, or a gate checker." The rejected alternative is scoped variants that drift. It is the governing text for REQ-02/03.
- `docs/concepts/validation-engine.md` documents local/graph split and repo-wide exclusion at transition (`:42-58`), dangling `satisfies:` always-on (`:121-124`).
- View contracts: `docs/reference/cli-commands.md:3313-3376` (modes, `--scope` "excluding whole-repository rules", exit codes), `docs/reference/worktree-validate.md:333-412`, `docs/reference/gate-presets.md:31-105` (four built-in checkers), `docs/how-to/validation-rules.md:52,106-113` (explain lists every rule).
- `dev/architecture/repository-state-materialization.md` covers capture/materialization. I found no study or note on validation performance under `dev/`.
- Rustdoc on `evaluate_graph_scoped` (`graph.rs:420-431`) and the ef0065ad comment at `graph.rs:975-979` record why scope uses slice context. The selection redesign must keep their outcome (§5.2).

### 4. Primitive verification

#### 4.1 Existing report and finding types (reuse)

- `engine::Finding { rule, severity, message }` (`engine.rs:106`). Keep it as the leaf.
- `graph::GraphFinding { issue_id, finding, is_config_error }` (`graph.rs:262`). It records **attribution** but not the **firing subject**. Attribution differs from subject for label-coverage with `container-from-label` (`graph.rs:970-1055`, finding attributed to the container), and per-subject config errors are unattributed (`graph.rs:985-1013`). Selection by rule+issue (REQ-02 `--rule`) and precise per-issue config errors need the subject. **Needed:** add `subject: Option<IssueId>` set by each evaluator (`evaluate_one` arms `graph.rs:610-680`). Label-uniqueness stays subject-less (`graph.rs:719`).
- `report::ReportedFinding { issue_id, rule, severity, message }` (`report.rs:23`). This is the wire shape of every validate JSON output. Keep it as the projection.
- `RuleReport`, `RuleOutcome`, `ExplainReport` (`report.rs:58,97,128`). Keep them as view outputs.
- `RepositoryValidationReport`/`Failure` (`repository.rs:52,70`). Consumers: `gate_check.rs` (2), `init.rs` (1), `profile.rs` (2), `validate.rs` (10), `main.rs`.

#### 4.2 Purity

The report function needs `now` injected and the claims check moved out (`repository.rs:540-552`, `:589`). Everything else reads the image.

#### 4.3 Single parse

There is no shared loader. `capture_proposed_base_inner` already returns `(image, issues)` (`validate.rs:365-520`), but validation drops the issues (`capture_base_reading` `:211`). Capture's lenient parse (`effective_issues`) runs over the *overlay-effective* bytes before `apply_overlay`, so it equals a parse of the overlaid image.

#### 4.4 Generators for REQ-04

- I found no proptest strategy for `RepositoryImage`, `Issue` sets with labels, or `RuleSet`.
- Reusable pieces:
  - `graph/hierarchy.rs:1121` `arbitrary_dag` (acyclic by lower-index dependencies);
  - `readiness_coherence_tests.rs:213` `dependency_edits`;
  - `claim_coordinator_proptests.rs:53` `issue_id_strategy`.
- Pure image construction exists only as private copies: `repository_state/mod.rs:1240` `layout()` and `:1437` `image_with`, `mutation.rs:1420`, `archive.rs:631`, `document/reference.rs:263,330`. Each builds a synthetic `RepositoryLayout` + `RepositoryImage::close` (`image.rs:708`).
- **Needed:** one shared test-support builder `image_from_files(files) -> RepositoryImage` (converging those 3+ copies, `@/inv/shared-test-contracts`), plus a strategy producing a config (`[item_kinds]` with one issue-scoped and one project-scoped kind, a `type` hierarchy with a breakable template), a fixed small rules.toml menu (one local schema rule, label-coverage with `container-from-label`, label-uniqueness, dependency-shape; each severity), and 2–7 issues over `arbitrary_dag` with labels drawn from valid, dangling and unqualified link forms.

#### 4.5 Schema compile once

`SchemaEngine` caches by schema key (`engine.rs:177-183,408-420`), but its lifetime is one `evaluate_local` call (`local.rs:317`). An engine owned per report delivers REQ-11 without other changes. The engine is `!Sync` (RefCell), which is fine for a single-threaded report.

#### 4.6 Address index

The report's item-link pass already uses one set (`repository.rs:713-749`), with issue-scope prefix resolution by linear scan per label (`:800-814`). Extracting it as a value type is enough. Kinds are resolved per call (`repository.rs:703`) and again inside `derive_materialization`.

### 5. Design

#### 5.1 Report

```rust
// validation/report.rs (pure)
pub fn report(image: &RepositoryImage, now: DateTime<Utc>) -> ValidationReport;

pub struct ValidationReport {
    pub passes: Vec<RepositoryValidationPass>,
    pub structural: Option<StructuralFailure>,   // replaces the Result<Report, Failure> pair
    pub findings: Vec<ValidationFinding>,
    records: ParsedRecords,                        // index + issues, parsed once
    rules: RuleSet,                                // for selector matching / explain
    templates: TemplateRegistry,                   // for scope boundary
}
pub struct ValidationFinding {
    pub finding: Finding,                // rule, severity, message
    pub origin: FindingOrigin,           // Local | Graph{repo_wide: bool} | BuiltIn(BuiltInPass)
    pub subject: Option<IssueId>,        // firing issue; None for rule-level or repository findings
    pub issue: Option<IssueId>,          // attribution (wire `issue_id`)
    pub config_error: bool,
}
pub enum BuiltInPass { RulesFile, DanglingItemLink, EnforcementDrift, ReviewPlaceholder }
```

`BuiltInPass` replaces the three `commands` constants (REQ-06) and gives each a rule-name token. Its spelling should come from the 2195c98f wire-vocabulary macro.

Finding classes:

| Class | Findings | Subject / attribution |
|---|---|---|
| Per-issue | local rule findings; graph findings of coverage, label-reference, dependency-shape, gate-recency, type-hierarchy, criteria-*; `dangling-item-link` | attributed issue (+ subject for graph) |
| Per-rule | graph config errors not tied to a firing issue (bad `id-pattern`, bad key type) | subject None, `config_error` |
| Per-subject config error | pointer missing/ambiguous/none (`graph.rs:985-1013`) | subject = firing issue, unattributed |
| Repository-wide | `label-uniqueness`; `enforcement-drift`; `review-placeholder`; `rules-file` | none |
| Structural (not findings) | records, integrity, projections | `structural` |

#### 5.2 Selection

```rust
pub enum Selection { Whole, Issue(IssueIdPrefix), Scope(IssueIdPrefix), Rule { issue: IssueIdPrefix, rule: String } }
impl ValidationReport {
    pub fn select(&self, s: &Selection) -> Result<RuleReport, SelectionError>;   // RuleReport stays the wire view
    pub fn explain(&self, issue: &IssueIdPrefix) -> Result<ExplainReport, SelectionError>; // = select(Issue) grouped over every rule
}
```

Membership predicates, defined once (`Selection::contains(&ValidationFinding, &ResolvedSelection)`):
- **Whole**: everything; `structural` reported.
- **Issue(I)**: local/graph/dangling with `issue == I`; config errors of rules whose selector matches I with `subject ∈ {I, None}`; `rules-file`. Drift, placeholder, label-uniqueness and structural are excluded. Wire `issue_id` is rewritten to I, as today (`validate.rs:1658-1661`, `gate_check.rs:499`).
- **Scope(C)**: S = `bracket_scope_ids(C, issues, breakdown_type)`. Includes local with `issue ∈ S`; graph from rules with `!repo_wide` and `issue ∈ S`; config errors with `subject ∈ S`, or rule-level config errors of rules matching some member of S; dangling with `issue ∈ S`; drift; `rules-file`. Excludes placeholder and structural.
- **Rule(I, R)**: `select_graph_rule` failure → one error finding (as today); otherwise findings of R with `subject == I`, plus rule-level config errors of R.
- **Explain(I)**: every rule → `RuleOutcome`; messages from the Issue(I) selection grouped by rule.

Behaviour changes the plan must state (see Needs decision):
1. **Scope graph context becomes the whole repository.** Today `evaluate_graph_scoped` uses the slice as graph context (`graph.rs:432-454`, `:548-558`), so coverage candidates and dependency-shape/label-reference targets outside the bracket are invisible. Under restriction they are visible. The ef0065ad skip (container outside the slice) is preserved by attribution: the finding names the out-of-slice container and is filtered out (`test_scope_skips_unrelated_breakdown_whose_container_is_out_of_scope`, `scope_validation_tests.rs:732`).
2. Per-issue and explain config errors narrow from "any config error of an applicable rule" to subject ∈ {I, None}.
3. Per-issue drift (today only when rules are unloadable) is always excluded; `rules-file` is always included. Gate `IssueValidation` stops erroring on unloadable rules.
4. Scope keeps excluding the placeholder; whole keeps it.

Options for the report API:
- **A (recommended)**: one `ValidationReport` with `structural: Option<…>` replacing the `Result<Report, Failure>` pair. D-18: one type, and selection never has to unwrap a failure.
- **B**: keep `RepositoryValidationReport`/`Failure` and add the `findings` provenance inside `rule_report`. Smaller diff, but every view keeps matching on two shapes.

Options for ParsedRecords and REQ-10:
- **A (recommended)**: capture returns `CapturedValidation { image, records }` built by the one loader over the overlay-effective byte source. `report` takes that pair, so REQ-01's signature is `report(&CapturedValidation, now)`. One parse covers capture, report, divergences and the gate checkers.
- **B**: `report(image)` parses, and capture stops parsing issues (it would then need a second round to discover plan and document paths). Simpler signature, one more capture round, and capture still needs the issues.

Divergences (`captured_divergences`) become a report pass or a view over `records`. This removes the third issue parse.

#### 5.3 REQ-06 homes

| Item | Today | Home |
|---|---|---|
| `DANGLING_LINK_RULE`, `ENFORCEMENT_DRIFT_RULE`, `REVIEW_PLACEHOLDER_RULE` | `commands/validate.rs:12,19,23`; consumers `gate_check.rs` (5), `commands/mod.rs` (2), `repository.rs` (10), tests | `validation::report::BuiltInPass` (closed vocabulary) |
| `find_planning_node`, `planning_node_plan_path`, `PLAN_DOC_LABEL` | `validate.rs:2284,2332,2323`; re-export `commands/mod.rs:96` | `crate::templates` (pure; already owns `GraphTemplate`, `RoleBindings`, `plan_doc_location`), as `GraphTemplate::planning_node`/`plan_document(...)` |
| `declared_container_label` + container token interpolation | `commands/template_expand.rs:626,528-600` (pure, `pub(super)`) | move with it into `templates` (template_expand imports only domain/templates/config, `:24-33`); `template.rs:15,916` keep calling it |
| `validate_claims_index_at` | `validate.rs:2457`; caller `repository.rs:542` | out of the report. Either the executor runs it beside the report and feeds a structural input, or 98184c64 deletes it (§7) |

#### 5.4 REQ-07 address index

Option **A (recommended)**: `domain::item::AddressIndex { addresses: HashSet<CanonicalAddress>, issue_prefixes }`, built once by the report from parsed issues, resolved kinds and project sources (the current `repository.rs:713-749` body). It exposes `resolve(value, kinds, project_name) -> Result<(), Detail>`. Kinds are resolved once on `CapturedRepositoryDeclarations` and shared with `derive_materialization` (`materialize.rs:132,242`). Canonicalise kind aliases there to close the suspected divergence in §1.7.

Option B: an index keyed to `AddressableItem`, which also serves `show_item`. That gives one resolver for both `jit item show` and validation (D-20), but it is a larger scope that REQ-07 does not demand.

#### 5.5 REQ-11

`validation::local::LocalRules` (or `CompiledLocalRules`) holds the desugared `(rule, schema, key)` list and one `SchemaEngine`, built once per report. It provides `evaluate(&self, issue, format) -> LocalEvaluation`. `evaluate_local` becomes `LocalRules::new(rules).evaluate(...)` for single-issue write paths. Callers: `commands/mod.rs:1066,1446,1481,2467`, `template.rs:1131`, `bulk_update.rs:633`, `gate_check.rs:464,546`, `validate.rs` (4), and tests (§6).

#### 5.6 REQ-12

- `JitConfig::content_format() -> ContentFormat` and `JitConfig::strictness() -> Strictness`. Both are infallible once 42bcaa6e and 96bb0941 type the fields.
- Replace the 12 chains, the `projection_render.rs:245` re-implementation, and the 4 strictness sites. `CommandExecutor::repo_content_format` (`mod.rs:2634`) is deleted.
- One plan resolver, `templates::plan_document(container, issues_by_id, templates) -> Option<PlanDocument { path, planning_done }>`. It serves capture discovery (`validate.rs:949`), the report (`repository.rs:657`) and the mutation callers of `plan_content_with_issues` (§2).

### 6. Consumer sweep (files × refs)

- Executor copies and views: `crates/jit/src/commands/validate.rs` (copies plus 12 unit tests at `:3127-3392`); `main.rs:2429` (issue create), `main.rs` validate arm; `commands/invariant.rs:60` (+4 unit tests `:110-157`); `gate_check.rs` (46 refs to view/checker names).
- Rule-name constants: `gate_check.rs` 5, `commands/mod.rs` 2, `validation/repository.rs` 10 (including tests).
- Report types: `ReportedFinding`/`RuleReport` in `gate_check.rs` 14, `validate.rs` 32, `main.rs` 1, `validation/repository.rs` 18; `RepositoryValidation*` in `gate_check.rs` 2, `init.rs` 1, `profile.rs` 2, `validate.rs` 10.
- `evaluate_local`: `commands/mod.rs` 4, `template.rs` 1, `bulk_update.rs` 1, `gate_check.rs` 2, `validate.rs` 4, `repository.rs` 1; tests `local.rs` (~25), `repository_state/default_rules.rs` (11), `tests/fast_docs_templates/content_format_dispatch.rs` (8), `tests/fast_rules/example_rulesets_tests.rs` (14), `tests/fast_rules/default_rules_registry_derivation_tests.rs` (6). REQ-11 keeps the `evaluate_local` signature, so these are unaffected under §5.5.
- Integration tests per view: `tests/cli_query_graph/scope_validation_tests.rs` (33), `tests/cli_item_validate/validate_cli_rule_tests.rs` (13), `tests/fast_rules/validate_rule_runner_tests.rs` (11), `tests/cli_gate/bracket_coverage_gate_run_test.rs` (2), `tests/cli_item_validate/validate_drift_builtin_tests.rs` (5), `tests/cli_item_validate/invariant_check_cli_tests.rs` (6), `src/gate_presets/reference.rs` (16).
- Server: `crates/server/src/routes.rs:230` uses `get_status` only and is unaffected. mcp-server generates its validate tool from the schema (`validate_cli_rule_tests.rs:919` pins it); the JSON shape is unchanged if `ReportedFinding` stays the wire type. `web/` has no validation consumer. `scripts/jit-validate.sh` passes arguments through. `test-vectors/` has no validation vectors.
- Docs to update when semantics change (§5.2): `docs/reference/cli-commands.md:3313-3376`, `docs/reference/worktree-validate.md:333-412`, `docs/reference/gate-presets.md:31-40`, `docs/concepts/validation-engine.md:42-58`.

#### 6.1 Tests that move to the selection test (`@/inv/orthogonal-tests`)

Selection membership is proven once by the REQ-04 property test plus a few example tests in the same module. These tests re-prove view filtering and should fold into it, or shrink to wiring:
- `validate.rs`:
  - `test_dangling_link_findings_*` (7, `:3127-3292`), `test_dangling_invariant_link_reports_finding` `:3303` and `test_validate_silent_fails_on_dangling_link` `:3315` → item-link matrix owned by the report pass (`repository.rs` tests `:1203,1240`);
  - `test_run_rules_whole_repo_surfaces_dangling_link` `:3340` → selection;
  - `test_run_rules_accepts_short_id_prefix` `:3369` and `test_explain_rules_accepts_short_id_prefix` `:3392` → one selection-resolution test.
- `tests/fast_rules/validate_rule_runner_tests.rs`: `test_per_issue_surfaces_relevant_graph_finding` `:108`, `test_whole_repo_collects_local_findings_for_all_issues` `:94`, `test_explain_lists_matched_rules_and_outcomes` `:137` → selection. Keep the skip-reason rendering tests (`:185-255`) as `match_failure`/explain unit tests.
- `tests/cli_query_graph/scope_validation_tests.rs`: `test_scope_excludes_repo_wide_label_uniqueness` `:493`, `test_scope_does_not_pull_in_plan_or_upstream` `:539`, `test_scope_boundary_is_template_driven_custom_breakdown_type` `:601`, `test_scope_skips_unrelated_breakdown_whose_container_is_out_of_scope` `:732`, `test_scope_dangling_brackets_pointer_still_fails_precisely` `:786` → selection; `test_rule_validation_*` `:307-434` → selection.
  - The external-plan tests `:909-1085,1196,1221` → one plan-resolver owner test.
  - CLI exit/JSON tests `:1335-1400` stay as wiring.
- `tests/cli_item_validate/validate_cli_rule_tests.rs`: `test_validate_id_reports_malformed_graph_rule_not_passed` `:692`, `test_validate_explain_marks_malformed_graph_rule_as_failed` `:715`, `test_validate_rule_cli_*` `:750-849` → selection. The CLI output tests stay.
- `gate_check.rs`: `test_rule_validation_checker_ignores_unrelated_validation_failures` `:2963`, `test_rule_validation_checker_reports_missing_and_ambiguous_targets` `:3062`, `test_label_target_validation_*` `:2908,2933`, `test_repository_validation_warns_for_sorted_placeholder_gate_keys` `:2881` → each checker proves only `checker(image) == report(image).select(sel)` (REQ-03 wiring).
- `tests/cli_gate/bracket_coverage_gate_run_test.rs:288` `test_selected_rule_ignores_historical_rejected_bracket_target` → selection.

#### 6.2 REQ-04 property shape

Selection is a filter, so the test must check relations rather than recompute views. Over generated `CapturedValidation` and generated selections (Whole, Issue for each id and a non-matching prefix, Scope for each breakable container, Rule for each pair of graph rule and issue), assert:
1. `select(s) ⊆ project(whole)`.
2. A finding of `whole` is in `select(s)` iff `s.contains(finding)`. `contains` is the single definition.
3. The union of all Issue selections plus repository-scoped findings equals the whole set of attributed and repository findings.
4. `explain(I)` FAIL rules are exactly the rules in `select(Issue I)`.
5. Gate checker output for (image, gate) equals `select` for the gate's selection (REQ-03); this may be a separate harness test.

Place the test in a `cfg(test)` module inside `crates/jit/src/validation/`. That adds no Cargo target (`@/inv/bounded-rust-build-footprint`).

### 7. Architecture fit and other-epic seams

- **Dependency direction**: the report and the selection are pure validation layer; the executor only captures and renders; gate checkers call `select`. This fixes `@/inv/architecture-dependency-direction` at `repository.rs:542-683`.
- **Convention convergence**: one loader, one plan resolver, one address index, one local evaluator, and one image test builder.
- Other impure inputs in validation/: `crate::config_manager::namespaces_from_config` (pure function in an I/O module) and `crate::test_mode` (`repository.rs:541`; also targeted by f368ce3d).

Seams with recommended edge directions (A → B means A waits on B):

| Other task | Seam | Edge |
|---|---|---|
| 842b0ac6 `keep-one-enforcement-drift` e1af39db (ready) | renames `enforcement_drift_tolerant` at the executor copy we delete and at `repository.rs:610` | our REQ-05 step → e1af39db (842b0ac6 executing; no delay) |
| 842b0ac6 `strict-rule-declarations` cd7b6c4a (ready) | graph evaluator config-error paths and tests | our subject-provenance step → cd7b6c4a |
| 842b0ac6 `remove-reference-builders` 81e7cdc3 (ready), `remove-repository-state-cfg-twins` 69af08af (ready) | `local.rs` tests; `validate.rs` | our first steps → those (or simply → 842b0ac6 epic) |
| f8a6b298 `lease-index-validation-removal` 98184c64 | deletes `validate_machine_local_claims`/`validate_claims_index_at` | **flag**: either our pure-report step → 98184c64 (we may wait on a large epic), or we inject the input and 98184c64 later deletes the injection. Recommend the injection; do **not** add 98184c64 → ours. |
| f8a6b298 `validation-event-count-removal` cac57c93 | `load_records` event parse, `event_count` | **flag**: same file region as our loader step. No default edge; whichever lands second rebases. |
| f8a6b298 `validation-skips-event-log` 5b0e3d63 | capture base (`validate.rs:403-416`) | REQ-09/10 seam; no edge from my scope |
| f4d9502a `lease-validation-lists-once` f649c144, `branch-drift-through-module` 47d62472, `layout-fallback-removal` d68150e2 | `validate_leases`, `validate_branch_drift`, layout in validate.rs | no report overlap; no edge |
| 39710b3b (ready, no epic) "Record built-in checker faults as error runs" | same dispatch `gate_check.rs:1027-1140` and `validate.rs:1063` | **flag**: recommend 39710b3b lands first; our checker step → 39710b3b |
| 525bd61a (ready, no epic) "Re-validate after zero-fix --fix" | REQ-14 subsumes it | out of my scope; recommend absorbing or rejecting it in favour of the REQ-14 task |
| rust-code-quality 5962427b (validation facade), 06ee9b67 (validate handler), 9cc3056a (findings renderer) | they move the same methods and main.rs arm | **flag**: an edge from theirs onto ours delays them; recommend steward ordering them after 9876a23a |
| f368ce3d (domain-agnostic, test switches) | `repository.rs:541` `test_mode` | dissolves if the claims check leaves the report |
| b697a3f7 (previews), 16c53cc2 (graph engine) | REQ-13 / `validate_integrity` graph queries | no edge from my scope |

2195c98f keys this scope needs first (the whole epic is already upstream via 9876a23a → 2195c98f):

| Our step | 2195c98f key (issue) |
|---|---|
| all report/selection types (issue ids) | `stage-validation-issue-ids` 1c5da41d, `issue-id-flip` 7abcc22d, `prefix-resolution` c400730c |
| loader | `issue-record-path-commands` 855d027d, `well-known-paths-storage` e1a6c8c2, `well-known-paths-commands` 945f65f3 |
| integrity pass | `graph-node-result-refs` bf87bbdb |
| hierarchy in report and checkers | `hierarchy-config-constructor` 62a0085a, `hierarchy-config-callers` f8f80eea |
| subject provenance in graph evaluators | `typed-label-coverage` 7e8f1525, `typed-label-reference` af9c277d, `typed-dependency-shape` 5613ef50, `compiled-id-pattern` b48d7b2a |
| `BuiltInPass`, `Severity`/`RuleScope` in findings | `rule-severity-scope-vocabulary` 6826f966 (and the wire-vocabulary macro) |
| gate finding mapping | `gate-finding-vocabulary` 72dfc0d7 |
| REQ-07 kinds once | `item-kind-id-pattern` bc1955a8, `kind-scope-vocabulary` b4fe7887, `scoped-issue-id-arguments` c1c86312 |
| REQ-12 accessors | `content-format-vocabulary` 42bcaa6e, `validation-strictness-vocabulary` 96bb0941 |

### 8. Sequencing

Each step compiles, passes clippy, and leaves tests green.

1. **Planning-node and plan-document resolver move (REQ-06b, REQ-12b)**.
   - Move `declared_container_label`, `InterpolationContext::for_container`, `find_planning_node`, `planning_node_plan_path` and `PLAN_DOC_LABEL` into `templates.rs`, and add `plan_document(...)`.
   - Route `validate.rs:949,986,1008`, `repository.rs:657` and the 5 mutation callers through it.
   - Files: `templates.rs`, `commands/template_expand.rs`, `commands/template.rs`, `commands/validate.rs`, `commands/mod.rs`, `commands/dependency.rs`, `commands/bulk_update.rs`, `commands/issue.rs`, `validation/repository.rs`.
   - Needs 1c5da41d, 7abcc22d.
2. **Config accessors (REQ-12a)**.
   - Add `JitConfig::content_format()`/`strictness()`; replace the 12+1 and 4 sites.
   - Files: `config.rs`, `commands/mod.rs`, `validate.rs`, `repository.rs`, `gate_check.rs`, `template.rs`, `bulk_update.rs`, `repository_state/projection_render.rs`.
   - Needs 42bcaa6e, 96bb0941.
3. **Compiled local rules (REQ-11)**.
   - Files: `validation/local.rs` and `engine.rs`; callers keep `evaluate_local`.
   - Needs 6826f966; follows 81e7cdc3.
4. **Built-in pass vocabulary (REQ-06a)**.
   - Add `BuiltInPass` in `validation/report.rs`; delete the three constants.
   - Files: `validate.rs`, `gate_check.rs`, `commands/mod.rs`, `repository.rs`, tests.
   - Needs 6826f966.
5. **Graph finding subject (provenance)**.
   - Add `GraphFinding.subject`, set by each evaluator.
   - Files: `validation/graph.rs`.
   - Needs 7e8f1525, af9c277d, 5613ef50, b48d7b2a; follows cd7b6c4a.
6. **Address index and kinds once (REQ-07)**.
   - Files: `domain/item.rs`, `validation/repository.rs`, `repository_state/mod.rs` (declarations carry kinds), `repository_state/materialize.rs`.
   - Needs bc1955a8, b4fe7887, c1c86312.
7. **One loader and pure report (REQ-01, REQ-10)**.
   - Add `ParsedRecords` and `CapturedValidation`; capture returns records.
   - The report takes `now`; the claims check is injected by the executor (or removed if 98184c64 landed); `captured_divergences` reads records.
   - The new `ValidationReport` (option A) replaces `Result<Report, Failure>`.
   - Files: `validation/repository.rs` → `validation/report.rs`, `commands/validate.rs` (capture, `capture_validation_report`, `repair_derived_state`), `commands/init.rs`, `commands/profile.rs`, `commands/mod.rs` (`captured_active_issues` → loader), `main.rs`.
   - Needs 855d027d, e1a6c8c2, 945f65f3, bf87bbdb, 62a0085a, f8f80eea.
8. **Selection type and REQ-04 property test**.
   - Add `Selection`, `select` and `explain`, the shared image test builder (converging `repository_state/mod.rs:1437`, `mutation.rs:1420`, `archive.rs:631`), and the generators.
   - Files: `validation/report.rs`, new `validation/selection_tests.rs` (cfg(test)), `test_utils.rs` or `repository_state` test support.
9. **CLI views over the selection (REQ-02, REQ-05)**.
   - `run_rules`, `explain_rules`, `validate_rule` and `validate_scope` become capture → report → select.
   - Delete `dangling_link_findings`, `enforcement_drift_findings`, `compute_drift_findings`, `loadable_rule_names`, `review_placeholder_findings`, `image_plan_content` and `rule_validation_report`.
   - `check_invariants` drift moves onto the pure drift function over captured declarations (decide on unloadable gates, below).
   - `evaluate_graph_rules` stays only for issue create until the REQ-13 step.
   - Migrate the §6.1 tests.
   - Files: `validate.rs`, `invariant.rs`, `main.rs`, the test files in §6.1, and docs §6.
   - Follows e1af39db.
   - This can split into 9a (issue/explain/rule) and 9b (scope + docs).
10. **Gate checkers over the selection (REQ-03, REQ-05)**.
    - Delete `repository_rule_report`, `captured_issue_rule_report` and `captured_scope_rule_report`; each checker selects from one report over its captured image.
    - Add the checker-equals-CLI wiring test.
    - Files: `gate_check.rs`, `gate_presets/reference.rs` tests.
    - Follows 39710b3b if that is accepted.

## Part: Capture, performance and the side paths

## 9876a23a investigation — capture, performance and side paths (REQ-08, 09, 13, 14, 15)

Code read at HEAD `3f7ce9165`. Measurements used `~/.cargo/bin/jit` (release, built from main `582b1cc92`) on a copy of this repository made with `git clone --shared` plus `rsync -a .jit/` under `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/15b668cb-1cdd-4d8f-9b20-8f416eb492eb/scratchpad/v9/bench/repo` (1,573 issues, 565 document references, 561 of them unpinned, 457 distinct document paths). The host ran other agent sessions: the 1-minute load average was 7.5–9.8 throughout. Every figure is the median of 5 runs. Absolute values are inflated by the load. Ratios are the usable signal.

### 1. Claim classification

| # | Claim (background / REQ) | Class | Current evidence |
|---|---|---|---|
| C1 | `discover_document_references` calls `CaptureSpec::discover_paths` once per document reference | valid-and-open | `crates/jit/src/commands/validate.rs:941-954` (audit line 1259 is stale). It also calls `discover_pinned` once per reference, which is already incremental. |
| C2 | Each call clones the spec and revalidates every requested path | valid-and-open | `crates/jit/src/repository_state/image.rs:566-578` (`discover_paths`: `self.clone()`, then `candidate.validate()`), `:581-589` (`discover_tail`), `:604-612` (`discover_listings`), `:660-686` (`validate`: depth check and `ensure_semantic` over every path and listing, plus `validate_pinned_request` over **every pinned request**). `discover_pinned` (`:620-629`) is already per-entry. |
| C3 | The discover closure re-applies `base(spec)` every round | valid-and-open | The `base` closure is at `validate.rs:393-418`. `discover` calls it first on every round (`validate.rs:436-440`). Each `base` call makes 2–3 `discover_*` calls, so it clones and validates the whole spec 2–3 times per round. |
| C4 | 541 references over more than 1,200 paths; `CaptureSpec::validate` is about 20% of the inclusive profile | valid-and-open, figures stale | There are now 565 references and more than 2,100 spec paths (1,573 issue records plus registries and plan documents). My `perf record` of `jit validate` (release binary, no frame pointers, so inclusive figures undercount) attributes about 4.4% to `CaptureSpec::validate` and about 3.5% to `BTreeSet<VirtualPath>` clone and drop, so about 8% in total. The top cost is now **`domain::issue_id::normalise`, 29% inclusive** (`crates/jit/src/domain/issue_id.rs:84-89`), reached from `resolve_container` (`crates/jit/src/validation/graph.rs:1306-1316`). It allocates a lowercase copy of every issue id for every `container-from-label` comparison, which makes it O(labels × issues). The audit does not name it. It belongs to REQ-07 or REQ-08 (graph scope; see §7, step R0). |
| C5 | "Capture runs twice per validate" | **invalid-as-stated** | Plain `jit validate` opens **one** mutation session and captures once (`validate.rs:552-591` → `capture_repair_plan` `:654` → `capture_validation_base` `:195` → `capture_base_reading` `:211-238`). The capture grows through several `extend` rounds. Two things do run twice: (a) the fixed-point loop `grow_discovering` (`commands/mod.rs:352-370`) runs the discover closure once more after the last extend to confirm nothing changed, so document-reference discovery runs **twice** (`validate.rs:234-237`), and closure planning (config, index and rules parse plus `validate_capture_closure`) runs **3 times**; (b) `validate --fix` captures the whole validation image at least 3 times (`validate.rs:43`, `:613`, `:85`) plus once per fix mutation. The background cannot be confirmed as worded; it reads as (a). |
| C6 | Human `issue create` lists every issue and evaluates every graph rule after the commit, recapturing a whole validation image; 373 vs 55 ms with `--quiet`; JSON computes no warnings | valid-and-open, figures stale | `crates/jit/src/main.rs:2428-2441` (audit lines 3071-3077 are stale): `storage.list_issues()` and then `executor.evaluate_graph_rules` (`validate.rs:1502-1544`). That function reads the disk caches (`effective_rules`, `cached_namespaces`, `repo_content_format`) and `image_plan_content`, which captures the **whole validation image** (`validate.rs:1558`). Measured now: human 629 ms, `--quiet` 76 ms, `--force` 111 ms, `--json` 267 ms. The JSON path computes no rule findings, but it also reads storage after publish: `storage.load_issue`, `get_dependencies_enriched` and `list_gate_runs` (`main.rs:2404-2415`). Local-rule warnings returned by `create_issue` go to **stderr** in both modes (`main.rs:2397-2399`). |
| C7 | `validate --fix` runs a full validation first and discards it, then publishes each hierarchy fix and each readiness correction as its own mutation | valid-and-open | `validate.rs:41-45`: `validate_silent()` runs and its result is used only when `!fix`. Hierarchy: one `publish_captured_field_update` per `ReplaceType` (`validate.rs:1196-1218` → `apply_type_fix` `:1223-1238`), each reading `get_hierarchy_config` and `list_issues` from disk (`:1154-1155`). Readiness: one lifecycle mutation per correction (`validate.rs:2125-2131` → `issue.rs:816-839` → `publish_captured_lifecycle_mutation` `commands/mod.rs:2813`), and each of those opens a preflight session and a base session. This repeats for up to 10 passes, each calling `storage.list_issues()` (`validate.rs:2094-2148`). Transitive reduction is **already one mutation** (`dependency.rs:246-256`, `ReduceAll` → pure `derive_dependency_repair` `:505-507`), and so is derived-state repair (`validate.rs:605-648`). Total publications: H + R (× passes) + 2. Also: with fixes applied, the post-fix `validate_silent()` (`:85`) keeps only the error; with zero fixes it does not re-validate at all (this is open bug 525bd61a). |
| C8 | Link checking probes external URLs serially, with a new HTTP agent per asset and no deduplication | valid-and-open | `crates/jit/src/commands/document.rs:1062-1081` (`validate_external_url`: `ureq::Agent::new_with_defaults()` per call, HEAD request) and `:1249`, called inside the per-document, per-asset loop (`:1180-1291`). The default `ureq` 3.2 config has **no timeout** (`timeout_global` defaults to `None`, ureq-3.2.0 `src/config.rs:663-672`). `validate_external_url` never returns `Err`, so the `url_validation_failed` warning branch (`document.rs:1264-1277`) is dead. |
| REQ-08 | `validate <id>` ≤ whole `validate` | valid-and-open | Measured: whole `validate` 1,534 ms, `validate 9876a23a` 4,926 ms (3.2×, the same ratio as the audit's 0.80/2.36 s); `--scope 9876a23a` 1,745 ms; `--explain` 881 ms. |
| REQ-09 | Incremental spec additions; batched document-reference round | valid-and-open | C1–C3. |
| REQ-13 | Create findings from the mutation's image, same in human and JSON, no read after publish | valid-and-open | C6. |
| REQ-14 | `--fix`: no pre-report, one mutation, always one post-fix report | valid-and-open | C7. |
| REQ-15 | Each distinct URL once, one agent, timeout, bounded concurrency | valid-and-open | C8. |

Counts: already-done 0, valid-and-open 12 (stale figures in C4 and C6), invalid-as-stated 1 (C5).

### 2. Prior-art sweep

- **D-20 rationale** (`dev/vision/9db27a3a-charter.md:384-396`): "a per-issue view becomes slower than the whole report it approximates" names REQ-08's symptom. "A preview runs the same mutation session as its operation, read-only" governs `--fix --dry-run` (REQ-14).
- **Capture model** (`dev/architecture/repository-state-materialization.md:80-100`): one pass, grown through `RepositoryMutationSession::extend`, "nothing is read twice". Under `@/issue/8454236c/decision/D-1`, issue-linked documents and pinned evidence are read by validation, not by mutations. That decision matters for REQ-13: the create mutation's image does **not** hold document references.
- **Batching precedent already in the code**: `discover_listings` documents the quadratic hazard of one-at-a-time growth (`image.rs:595-603`), and `discover_pinned` validates only the new request (`image.rs:614-629`). Document link checking builds its spec with one batched `discover_paths(worktree)` (`document.rs:1435-1456`). REQ-09 applies the same pattern to `discover_paths`, `discover_tail` and `discover_listings`.
- **Measurement harnesses**: `scripts/benchmark-mutation-cost.sh` (zsh; `git clone --shared` plus `rsync .jit`, load gating, median of 3; committed run `dev/benchmarks/publication-cost-722878d4/README.md`, cited from `dev/TESTING.md:458`) is the precedent for "measured on this repository, release build, recorded as evidence". `scripts/benchmark-session-cost.sh` (+ selftest) times a generated 665-issue corpus, not this repository. `scripts/rust-build-budget.sh` enforces target count, executable bytes and suite duration; it has no runtime-latency budget. `dev/studies/perf/session-cost-27ffbd2d.md` is a historical profile. No "benchmark harness" issue is open: searching for "benchmark" finds only f9977efd, fb2dc5c3, c780514e, caceed7d and 58282882, and none of them is about runtime latency.
- **Docs**: `docs/reference/cli-commands.md:3316-3390` (validate modes, JSON envelope, `--fix --json` error envelope; the `--fix` success body `{valid: true, fixes_applied, dry_run, message}` is built inline in `main.rs:2818-2832`), `:2899-2920` (`doc check-links`: no timeout or concurrency wording), `:957-990` (`issue create`; the `--orphan` row is at `:979`), and `docs/reference/exit-codes.md` (check-links exit 0/1/2, changed by f37d228e).
- **JSON warnings precedent**: `ClaimResponse { #[serde(flatten)] issue, warnings }` (`crates/jit/src/output.rs:1341-1346`) and `GateEvaluateManyResult.warnings` (`output.rs:2474`). Validation's machine shape for findings is `ReportedFinding` (`validation/report.rs`), serialized under `rule_findings` (`main.rs:2896`).
- **TLS test precedent**: `crates/jit/tests/cli_query_graph/remote_document_tls_tests.rs` (rcgen and rustls loopback server; the module doc says it drives an agent "configured exactly like" `validate_external_url`, so that doc becomes stale when the agent config changes). `check_links_tests.rs:11-31` has an offline loopback HTTP server, which a request-counting dedup test can reuse.

### 3. Consumer sweep

Counts are occurrences per file (`grep`), excluding the defining file unless noted.

- **`CaptureSpec::discover_paths` / `discover_tail` / `discover_listing(s)` / `discover_pinned`** (signatures unchanged by REQ-09; only cost changes): production in `commands/validate.rs` 13, `commands/gate_check.rs` 6, `commands/document.rs` 6, `commands/archive.rs` 5, `commands/mod.rs` 4, `commands/gate.rs` 3, `commands/template.rs` 3, `commands/profile.rs` 2, `commands/init.rs` 1, `commands/migrate.rs` 1, `commands/project.rs` 1, `repository_state/overlay.rs` 2, `repository_state/package_tree.rs` 3, `repository_state/export.rs` 1, `repository_state/profile_apply.rs` 3 (tests), `repository_state/mutation.rs` 1 (test), `repository_state/image.rs` 15 (9 in tests, after `:1435`), `document/reference.rs` 2, `storage/repository_state_store_tests.rs` 10, `tests/fast_docs_templates/template_apply_atomicity_tests.rs` 1. Other per-item loops with the same quadratic shape: `gate_check.rs:774-781` (a precheck's documents, one `discover_paths` per document), `overlay.rs:67` (`apply_overlay`, one `discover_paths` per override; production callers in `config.rs` 2, `init.rs` 2, `profile.rs` 3, `validate.rs` 1, `initialize.rs` 4, `repository_state/mod.rs` 2, `profile_apply.rs` 3).
- **`discover_document_references`**: `validate.rs:236` only.
- **`evaluate_graph_rules`** (the create-warnings path): `main.rs` 1 (`:2430`), `validate.rs` 4, `tests/fast_rules/cli_warnings_integration_tests.rs` 1 (`warnings_for` helper `:77-85`, used by 4 tests that assert create-time `orphan-leaf` and `strategic-consistency` findings through the executor), `tests/fast_rules/config_loading_tests.rs` 1.
- **`create_issue`** (if REQ-13 changes its return value): about 390 call sites per 8b7f2973's background. The server calls it in `crates/server/src/lib.rs` 1 and `crates/server/src/routes.rs` 4. The CLI `--orphan` flag is exercised in `tests/cli_issue/quiet_mode_tests.rs` 2, `tests/cli_query_graph/label_hierarchy_e2e_test.rs` 1 and `tests/cli_query_graph/graph_json_tests.rs` 2. The `issue create` JSON schema is `IssueShowResponse` (`crates/jit/src/schema.rs:542-548`). The MCP bridge generates `jit_issue_create` from that schema (`mcp-server/curated-tools.json:18`). Docs: `docs/reference/cli-commands.md:957-990`.
- **`validate_with_fix`**: `main.rs` 1, `validation/repository.rs` 3 (tests `:884,1227,1246`), `tests/fast_issue/state_transition_fix_tests.rs` 8, `transitive_reduction_validation_tests.rs` 6, `readiness_coherence_tests.rs` 3. CLI `--fix` tests: `tests/cli_item_validate/type_hierarchy_fix_tests.rs` 7, `derived_state_repair_tests.rs` 4, `validate_cli_rule_tests.rs` 2, `tests/cli_repo_workflow/steering_scenarios.rs` 1. `repair_derived_state` is also called by `jit recover` (`commands/claim.rs:733`). `validate_silent`: `issue.rs` 2, `main.rs` 1, and test files `label_constraints_tests.rs` 2, `dep_add_atomic_tests.rs` 2, `graph_rule_validation_tests.rs` 6, `readiness_coherence_tests.rs` 3, `transitive_reduction_validation_tests.rs` 7, `fast_rules/validation_tests.rs` 10. The readiness helpers `auto_transition_to_ready` and `auto_transition_to_backlog` are also used by `issue.rs` (11 occurrences), `bulk_update.rs` 2 and `dependency.rs` 1, so the per-fix publishers stay. Docs mentioning `validate --fix`: `docs/reference/cli-commands.md` 4, `worktree-validate.md` 3, `how-to/multi-agent-coordination.md` 3, `how-to/custom-gates.md` 2, and 1 each in `concepts/guarantees.md`, `concepts/hierarchy-resolution.md`, `how-to/dependency-management.md`, `how-to/deployment.md`, `reference/events.md`, `reference/profiles.md` and `reference/storage-format.md`.
- **`validate_external_url` / `check_document_links`**: `document.rs` (definition and 1 call), `main.rs` 1, `tests/cli_query_graph/check_links_tests.rs` 6, `tests/fast_issue/short_hash_tests.rs` 1, and the module doc of `tests/cli_query_graph/remote_document_tls_tests.rs`. `crates/jit/Cargo.toml:60-71` has a comment saying `Agent::new_with_defaults()` is "the only call site"; it becomes stale. Docs: `cli-commands.md` 2, `configuration.md` 1, `exit-codes.md` 3.
- No consumers in crates/server, web or test-vectors for the capture-spec, fix or link-check internals.

### 4. Primitive verification

- **Incremental spec validation (REQ-09).** Every check in `CaptureSpec::validate` (`image.rs:660-686`) is either per entry (depth ≤ `max_depth`, `ensure_semantic`, `validate_pinned_request`) or a count (`listings.len() ≤ max_listings`). Validating only the added entries is therefore exactly equivalent to revalidating the whole spec. The fields are private, `CaptureSpec` derives `Serialize` but not `Deserialize`, and every mutator goes through these methods, so the invariant "every held entry is valid" holds by construction. `RepositoryImage::close` re-runs `spec.validate()` once (`image.rs:705`). That is linear and can stay, or become a debug assertion. All-or-nothing semantics are kept by validating the new entries before inserting any of them, so the clone is unnecessary. The only cross-field coupling is `discover_paths` removing a path from `tails`, which is local to the entry.
- **Batched document-reference round (REQ-09).** `DocumentReferenceRequests::for_reference` (`document/reference.rs:70-81`) is pure, so all requests of a round can be computed first and added with one `discover_paths(worktree…)` plus per-entry `discover_pinned` (already incremental). An optional `discover_pinned_many` would keep the round's spec additions symmetric.
- **Hoisting `base` (REQ-09).** When `start` is `None`, `base` is already applied before the first capture (`validate.rs:507-509`), so re-applying it in every round is a no-op. When `start` is `Some(image)`, the spec must grow once. Applying `base` to `start.capture_spec().clone()` and extending once before `grow_discovering` removes the per-round call and the `if spec != image.capture_spec() { return Ok(()) }` guard (`validate.rs:438-440`).
- **Validation views timing (REQ-08).** No existing automated mechanism bounds command latency. The build-budget script bounds suite time only (`scripts/rust-build-budget.sh:23-27`). An automated bound needs either a release build in CI or a debug-build proxy, and both are noisy at the 1–5 s scale on a shared host.
- **Create findings from the mutation image (REQ-13).**
  - `publish_issue_creation` (`commands/mod.rs:2418-2503`) already holds `MutationBase { image, issues }`; it discards `issues` at `:2433`. It also holds `declarations` (`:2445`), the final draft and the context timestamp.
  - `capture_mutation_base` → `discover_issue_closure` captures plan documents (`validate.rs:882`), so `plan_content_with_issues(&image, &issues ∪ {new}, …)` (`validate.rs:1017`) can supply graph-rule plan content without a capture.
  - The transition path already evaluates graph rules purely from the same kind of evidence: `derive_state_transition` with `CapturedTransitionEvidence` (`mod.rs:557-563`, `:1150-1211`, which calls `evaluate_graph` with `context.timestamp()`).
  - The returned plan's delta holds the record exactly as stored. `apply_overlay(&image, plan overrides)` (`repository_state/overlay.rs:53`) yields the post-mutation image.
  - Two gaps. (a) The mutation image reads the event log as a tail and has **no document references** (decision `8454236c/D-1`), so a full REQ-01 report over it would produce false findings on document checks. (b) The JSON response also needs `get_dependencies_enriched` and `list_gate_runs`. Both are empty by construction for a new issue: no dependencies, and gate runs are keyed by issue id.
- **One mutation for all fixes (REQ-14).**
  - Pure derivations exist for every fix: `generate_fixes` over `detect_validation_issues` (domain, `domain/type_taxonomy`); `derive_dependency_repair(issues, dry_run)` (`dependency.rs:505-507`); `Issue::derive_readiness_correction` (`domain/types.rs:669`), whose cascade needs a pure fixpoint over the projected issue set (b697a3f7 REQ-01 asks for exactly that); and `derive_materialization(image, RepairDerivedState{…})` (`validate.rs:681-692`).
  - Composition: `finalize(layout, image, context, intents)` (`repository_state/mutation.rs:719`) turns the issue edits into a delta. `RepositoryDelta::new` (`repository_state/image.rs:1239-1256`) accepts a concatenated action list and rejects duplicate targets, so issue edits (`issues/*.json`, `events.jsonl`) and derived-state repair (rules, schemas, projection targets) merge into one delta when their paths are disjoint.
  - Constraints: each changed issue must appear in one `UpdateIssue` (fold label, dependency and state edits per issue before `finalize`); an `AppendEvents` from repair would collide with the finalizer's append (unverified whether repair ever emits one; `validate.rs:631` handles the case). There is no existing "merge two plans" primitive, so a small one is needed in `repository_state`.
  - The post-fix report can be computed over `apply_overlay(base, delta)` inside the same session, before publish. That one computation serves `--dry-run` and apply alike (D-20 preview), and it answers 525bd61a, since the zero-fix path then still reports the report.
  - Base read mode: fixes need the mutation reading of the event log (a tail), and validation currently reads it whole. f8a6b298 `validation-skips-event-log` (5b0e3d63) makes both read the tail.
- **HTTP client (REQ-15).** `ureq = { version = "3", default-features = false, features = ["rustls", "gzip"] }` (`crates/jit/Cargo.toml:71`), locked at 3.2.0. `Agent::config_builder().timeout_global(Some(d))…build()` then `Agent::new_with_config` (ureq-3.2.0 `src/agent.rs:112,123`; `src/config.rs:669`) gives one agent with a timeout. `Agent` is `Clone` over shared `Arc`s and `Send + Sync` (`src/agent.rs:89-98`; `src/lib.rs:1356-1362`), so `std::thread::scope` workers can share one agent with no new dependency. The error type has `StatusCode(u16)` and `Timeout(_)` (`src/error.rs:14,36`). Setting `http_status_as_error(false)` lets the status be classified, and `Timeout`, `HostNotFound` and `ConnectionFailed` become the `url_validation_failed` warning class, which makes the dead branch live. `tokio` is a workspace dependency, but `crates/jit` uses none (only the root `Cargo.toml:53` and the server crate), so std threads are the no-new-dependency option.

### 5. Design options

**REQ-09 incremental capture spec**
- A (recommended): per-entry `fn validate_request(&self, path) -> Result` plus count checks. `discover_paths`, `discover_tail` and `discover_listings` collect the new entries, validate them, then insert; no clone. Add a `discover_pinned` batch variant only if a caller needs one. The REQ-09 test is a proptest showing that incremental acceptance equals whole-spec revalidation (the semantic contract), plus a unit test that a refused batch leaves the spec unchanged.
- B: keep the clone-and-validate pattern but have callers batch. Cheaper diff, but quadratic wherever a caller grows one entry at a time (`overlay.rs:67`, `gate_check.rs:777`). It also fails REQ-09's first clause.
- Batching round (both options): `discover_document_references` gathers requests then one call. Apply the same to `gate_check.rs:774-781` and `overlay.rs:67` (one `discover_paths` over all overrides).

**REQ-08 enforcement**
- A (recommended): scripted measurement recorded as evidence. Generalize `scripts/benchmark-mutation-cost.sh` (fixture copy, load gate and median already exist) with read scenarios: `validate`, `validate <id>`, `validate --scope <C>`, `validate <id> --explain`, `issue create` (human, json, quiet), `validate --fix --dry-run`. Alternatively add one sibling script reusing its functions. The closing task commits `dev/benchmarks/9876a23a-validation-views/README.md` plus `raw/`. This follows the 722878d4 precedent and D-18 (one harness).
- B: automated wall-clock bound (a CLI test comparing the two timings). Debug build, shared-host noise, and added suite time against the measured suite budget (`@/inv/bounded-rust-build-footprint`). Not recommended.
- C: automated structural guard. REQ-04's selection contract already proves `validate <id>` = filter(whole report). If REQ-02 makes `validate <id>` call the one report function once, latency ≤ whole + filter follows structurally, and the script in A is the measured confirmation. No counting instrumentation exists to assert "one report computation" mechanically; adding one is new test-support mechanism, which D-18 cautions against.

**REQ-13 create findings**
- A (recommended): inside `publish_issue_creation`, project `issues ∪ {final_draft with id}` and evaluate the issue-scoped advisory passes for the new issue: local-rule warnings (already computed) and graph rules, through the same pure graph evaluation `derive_state_transition` uses, with `plan_content_with_issues` over the mutation image and `context.timestamp()`. Return the findings with the created issue. Once REQ-01 exists, this becomes "the one report function's issue-scoped passes over the overlaid mutation image, selected to the new issue", with document-reference passes excluded because the mutation image does not read them (8454236c D-1).
- B: make the create mutation capture the full validation closure (document references, whole issue set) and run the whole report, selected to the new issue. That is exact D-20 conformance, but every create pays validation capture cost: measured `validate` at 1.5 s against create at 76 ms. Not recommended.
- JSON contract (output change): a new `IssueCreateResponse { #[serde(flatten)] issue: IssueShowResponse, findings: Vec<ReportedFinding> }`, mirroring `ClaimResponse` and reusing validation's finding shape (`rule`, `issue_id`, `severity`, `message`). Register it in `schema.rs:545` in place of `IssueShowResponse`, and document it at `cli-commands.md:957`. Local-rule warnings move from stderr into `findings` (human mode renders them through the same list). `--orphan` remains a presentation filter applied to both outputs. Whether `--force` suppresses findings must be decided (today it suppresses them in human mode only). Because new-issue dependencies and gate runs are empty by construction, the JSON path drops `load_issue`, `get_dependencies_enriched` and `list_gate_runs` (`main.rs:2404-2409`); that requires `create_issue` to return the created issue (= 8b7f2973 REQ-01/02).

**REQ-14 fix mode**
- A (recommended): one session. Capture the validation base, then derive the fixes over a projected issue set in this order: hierarchy type fixes, transitive reduction, the readiness fixpoint, then derived-state repair from declarations. Fold the issue edits per issue, `finalize` them, concatenate with the repair actions through `RepositoryDelta::new`, compute the report over `apply_overlay(base, delta)`, then `Apply` (or return in dry-run). Output always carries that report, so the `--fix` JSON gains the report fields and its exit status follows the post-fix report. That is an output contract change, documented at `cli-commands.md:3316-3390`. `jit recover` keeps calling the derived-state-only repair, or calls the composed fix; decide which.
- B: keep the separate publishers but run them under one held session. Publishers each open their own session (`with_mutation_session`) and the redo record is per session, so this does not deliver "one mutation". Not viable without restructuring.

**REQ-15 link checking**
- A (recommended): before the per-document loop, collect `BTreeSet<&str>` of external URLs and probe them with one agent (`timeout_global`, `http_status_as_error(false)`) from a fixed pool of `std::thread::scope` workers (constant bound, e.g. 8) that pull from a shared index (`AtomicUsize`). Return `BTreeMap<url, ProbeOutcome {Reachable, Unreachable(status), Failed(reason)}>` and read the map inside the loop. Tests: the loopback server in `check_links_tests.rs` counts requests (two documents naming one URL produce one request), and a server that accepts and never answers gives a `url_validation_failed` warning within the timeout.
- B: rayon or tokio. A new dependency against `@/inv/bounded-rust-build-footprint`. Rejected.

### 6. Architecture fit / invariant check

- REQ-09 is local to `repository_state` (pure capture declaration); no layer change.
- REQ-13 moves rule evaluation into the command-layer derivation inside the session. It keeps `@/inv/derived-state-coherence` (one read under the lock) and removes the post-publish capture and the disk caches from that path (`effective_rules`, `cached_namespaces`), in line with b697a3f7 REQ-10. The CLI then only renders the findings (`@/inv/architecture-dependency-direction`).
- REQ-14 satisfies `@/inv/atomic-writes` (one redo record) and D-20 (dry-run and apply share one session and one report). The hierarchy fix currently reads `get_hierarchy_config` from disk (`validate.rs:1154`); A derives it from image declarations, as 2195c98f `hierarchy-config-callers` (f8f80eea) requires.
- REQ-15 stays in the command layer. The timeout and pool size are mechanism constants, so `@/inv/domain-agnostic` is unaffected. `@/charter/D-4` is unaffected.
- `@/inv/orthogonal-tests`: REQ-09 equivalence is owned once in `image.rs` unit tests; the REQ-15 dedup and timeout tests are owned once in `check_links_tests.rs`; REQ-14 needs one harness test for "one publication carries every fix class". Existing per-class fix tests stay as wiring. A redo-record or event-count observation proves "one mutation".
- Stale prose to sweep: `Cargo.toml:60-64` ("the only call site"), the `remote_document_tls_tests.rs:6-12` module doc, `cli-commands.md` for `--fix` output and the `issue create` JSON, and the `validate_with_fix` doc comment (`validate.rs:31-40`).

### 7. Sequencing (this scope)

The 9876a23a epic depends on the whole 2195c98f epic, so every 2195c98f key lands first. The keys each step touches are named so the plan can cite them.

| Step | Change | Files | 2195c98f keys whose code it edits | Neighbour seams and edge |
|---|---|---|---|---|
| R0 (REQ-08 prerequisite, graph scope; hand to the core investigator) | Resolve `container-from-label` against a normalised-id index built once per evaluation, not `IssueIdPrefix::matches` per issue | `validation/graph.rs:1306-1316`, `domain/issue_id.rs` | `prefix-resolution` (already landed: `IssueIdPrefix`), `stage-validation-issue-ids` (1c5da41d) | 16c53cc2 (graph traversal, unplanned) owns graph query algorithms; flag to the steward. |
| S1 (REQ-09a) | Incremental `discover_paths`, `discover_tail` and `discover_listings`; equivalence proptest | `repository_state/image.rs` | `well-known-paths-repository-state` (18dc6e3e) touches the same module's constants only | f8a6b298 `capture-refuses-whole-event-log` (725af076) adds an EVENTS refusal to `discover_paths` (`image.rs:566`). **No edge by default**: the regions merge trivially, and the refusal becomes one per-entry check. An edge either way would delay one side; flag for the steward. |
| S2 (REQ-09b) | One batched document-reference round; `base` applied once; batch `gate_check.rs:774-781` and `overlay.rs:67` | `commands/validate.rs:211-238,393-510,941-954`, `commands/gate_check.rs`, `repository_state/overlay.rs` | `issue-record-path-commands` (855d027d) rewrites `validate.rs:292` and the `gate_check.rs` issue paths in the same functions | f8a6b298 `validation-skips-event-log` (5b0e3d63) rewrites `base` at `validate.rs:411-412` and `:880`. Prefer S2 after 5b0e3d63 (an edge from ours onto theirs does not delay their epic). If f8a6b298 is scheduled later, use no edge; flag. Depends on S1. |
| S3 (REQ-15) | Deduplicated, pooled, timed URL probing; dedup and timeout tests; Cargo.toml and TLS-test doc sweep | `commands/document.rs:1058-1081,1180-1291`, `tests/cli_query_graph/check_links_tests.rs`, `Cargo.toml` comment | none | f4d9502a `asset-check-through-module` (9ecde705) edits `document.rs:1157,1176` in the same function, and f37d228e (ready) edits exit-code lines `:1347-1353`. Different regions: **no edge**. c9103fcf (df129d9e) moves the `check-links` main.rs arm only. |
| S4 (REQ-13) | In-session advisory findings for the new issue; `IssueCreateResponse` with `findings`; no post-publish reads; schema and docs | `commands/mod.rs:2418-2503`, `commands/issue.rs:167-225`, `main.rs:2384-2441`, `output.rs`, `schema.rs:545`, `docs/reference/cli-commands.md`, `tests/fast_rules/cli_warnings_integration_tests.rs` | `issue-draft-identity` (5fac48b8: the draft carries its id, which changes `:2432`), `stage-lifecycle-command-issue-ids` (75951558), `stage-validation-issue-ids` (1c5da41d), `issue-record-path-commands` (855d027d, `mod.rs:2438`), `content-format-vocabulary` (42bcaa6e) and `validation-strictness-vocabulary` (96bb0941) for the defaults in `derive_write_validation` (`mod.rs:1054-1065`) | df129d9e: 8b7f2973 ("Issue creation returns the created issue") and 3f816619 (issue write handlers) edit the same arm and return value. **Needs decision** (below). Best after REQ-01 lands, so the findings come from the report's issue-scoped passes. |
| S5 (REQ-14) | One-session `--fix`: pure fix derivations, readiness fixpoint, delta concatenation, report over the overlay, output contract | `commands/validate.rs:41-95,1149-1238,2060-2148`, `commands/dependency.rs` (expose `derive_dependency_repair`), `repository_state` (delta merge), `main.rs:2801-2840`, docs | `hierarchy-config-callers` (f8f80eea), `stage-validation-issue-ids` (1c5da41d), `stage-lifecycle-command-issue-ids` (75951558), `event-envelope` (d72bf093) for the drafted events | 525bd61a (ready, unparented) is subsumed by REQ-14's "always reports": mark it satisfied or reject it as superseded; its REQ-03 index-rebuild decision stays open. b697a3f7 REQ-01 (unplanned) wants the same pure readiness derivation, so S5 should own it and b697a3f7 consume it (an edge from b697a3f7 onto S5 delays b697a3f7; flag). It needs 5b0e3d63 for tail-read compatibility, or it captures the tail itself. Depends on REQ-01 (report function). |
| S6 (REQ-08) | Generalize the benchmark harness with validate, create and fix scenarios; commit the measured run | `scripts/benchmark-mutation-cost.sh` (or a sibling), `dev/benchmarks/9876a23a-validation-views/` | none | Last; depends on REQ-02 and REQ-07 (the core scope), R0, S2, S4 and S5. |

Each step leaves the workspace compiling with clippy and tests green: S1 changes no signature, S2 only reorders spec growth, S3 is internal, and S4/S5 change output contracts and their tests in the same step.

