# Plan: Compute one validation report and derive every validation view from it (9876a23a)

> Planning node: 2df01664. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

Repository validation becomes one pure validation-layer function, `validation::report`, over a captured validation (the image plus the records the one loader parsed) and the inputs its caller supplies. Every view, CLI or gate checker, captures once, computes that report and selects from it through one membership predicate (`@/charter/D-20`). Shared resolution (records, plan documents, item addresses, item kinds, container pointers, compiled schemas, configuration defaults) moves to one function each before the report is assembled, so the report is built from converged parts (`@/charter/D-18`). The side paths stop repeating work: capture specs grow incrementally, issue creation derives findings in its mutation, `--fix` publishes once, and link checking probes each URL once. Each issue body is written against the code as it stands after the upstream tasks the entry follows, and cites current lines.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | `pure-validation-report` replaces `validate_repository`, its `_with_materializations` twin and the `Result` pair with `report(&CapturedValidation, &ReportInputs)`; the executor's second graph computation goes in `executor-graph-evaluation-removal`, and the checkers' pipelines in `gate-checkers-over-selection`. | Investigation, core §1.1, §4.2, §5.1 |
| REQ-02 | `report-selection` adds finding provenance (with `graph-finding-subject`) and `select`/`explain`; `issue-views-over-selection`, `rule-view-over-selection` and `scope-view-over-selection` make each view capture once and select. | Core §1.2–1.5, §5.2 |
| REQ-03 | `gate-checkers-over-selection` maps each checker to its selection (`checker-selections`) and owns one parity test against `jit validate`. | Core §1.6 |
| REQ-04 | `selection-contract-proptest` checks every selection against an oracle written from `selection-membership`, over generated images covering every finding kind, built with `image-test-builder` and the harness graph strategy (`dag-strategy-in-test-harness`); view tests shrink to wiring (DEC-11). | Core §4.4, §6.1, §6.2 |
| REQ-05 | `invariant-drift-over-declarations` moves the last production consumer off the drift copy; `executor-pass-cases-to-report-tests` moves their test cases onto the report passes; `executor-pass-copies-removal` deletes the copies; checkers stop calling whole validation for subsets. | Core §1.7 |
| REQ-06 | `planning-node-lookup-in-templates`, `built-in-pass-vocabulary`, the claims input injected by `pure-validation-report` (DEC-09), and `validation-layer-tests-off-commands` for tests and rustdoc. | Core §2 row 3, §5.3 |
| REQ-07 | `item-address-index` (one index per report, kinds resolved once on captured declarations); `container-pointer-index` removes the per-pointer normalisation (DEC-10). The per-issue `show_item` path goes with the issue view. | Core §4.6, §5.4; capture C4 |
| REQ-08 | The per-issue view becomes one capture and one report and skips the divergence view and the claims index, so its work is a subset of whole validation's; `validation-cost-measurement` records interleaved release-build pairs on a copy of this repository (DEC-12). | Capture §5 REQ-08 A |
| REQ-09 | `capture-spec-incremental`, then `document-reference-round-batched` (DEC-16). | Capture C1–C3, §4 |
| REQ-10 | `records-loader` converges three loaders; `capture-hands-records` hands capture's parse to the report and divergences. | Core §2 row 5, §4.3 |
| REQ-11 | `compiled-local-rules`: one compiled set per report. | Core §4.5 |
| REQ-12 | `one-plan-document-resolver`; `config-default-accessors` and `command-config-defaults`. | Core §5.6 (counts corrected there) |
| REQ-13 | `create-findings-in-session` derives findings from the mutation image; `create-output-reports-findings` renders them identically and removes post-publish reads (DEC-13). | Capture C6, §5 REQ-13 A |
| REQ-14 | `fix-in-one-mutation` (DEC-14, DEC-04(2), DEC-04(3)). | Capture C7, §4, §5 REQ-14 A |
| REQ-15 | `link-probe-pool` (DEC-15). | Capture C8, §4 |

## Shared architectural contracts

### `selection-membership` [plan-fixed] — What each selection admits

A finding carries its origin (local rule; graph rule, with whether the rule is repository-wide at transition; or a built-in pass), its firing subject, its attribution and whether it is a configuration error.

- **Whole**: every finding, and the structural failure.
- **Issue(I)**: findings attributed to I from local rules, graph rules and the item-link pass; configuration errors of graph rules whose selector matches I, when their subject is I or absent; and `rules-file`. It excludes `enforcement-drift`, `review-placeholder`, unattributed findings and the structural failure. Kept findings are reported with I as their issue.
- **Scope(C)**: S = `bracket_scope_ids(C, issues, breakdown type of C's template)`. Local-rule and item-link findings attributed to a member of S; graph findings attributed to a member of S from rules not repository-wide at transition; configuration errors whose subject is in S, or subject-less ones of rules matching a member of S; `enforcement-drift`; `rules-file`. It excludes `review-placeholder` and the structural failure. Rules are evaluated over the whole repository (DEC-06).
- **Rule(I, R)**: a selection error from `select_graph_rule` is one error finding naming R; otherwise R's findings whose subject is I, with their own attribution, plus R's subject-less configuration errors; `rules-file` when the rule set is unloadable.
- **Explain(I)**: one outcome per rule; a matched rule fails exactly when the Issue(I) selection holds a finding of it.

A prefix resolves against the report's records through the shared resolver, with today's not-found and ambiguity errors. A report without parsed records fails every selection but Whole with its structural error. Sources: DEC-05, DEC-06, DEC-07; investigation core §5.2.

### `checker-selections` [plan-fixed] — Built-in checker to selection

`repository_validation` reports Whole (findings and structural failure); `issue_validation` reports Issue(gated issue); `rule_validation { rule }` reports Rule(gated issue, rule); `label_target_validation { label_namespace }` reports Scope(target named by the gated issue's single label in that namespace), and a missing or repeated label stays the checker's own finding. A run's status follows the built-in fault classification: a configuration fault records `error`, content findings `failed`. `review_placeholder` is not a report view.

### `validation-report` [implementation-produced] — The one report

`validation::report(&CapturedValidation, &ReportInputs) -> ValidationReport` in the validation layer is the only report computation. `CapturedValidation` pairs a captured image with the records the loader parsed (`CapturedValidation::from_image` parses them when the caller has none). `ReportInputs` carries the evaluation instant and the claims-index problems the executor read over its own worktree paths. `ValidationReport` holds the passes reached, the structural failure, and the findings with a wire projection to `RuleReport`. Producer: `pure-validation-report`.

### `report-selection` [implementation-produced] — Selection operation

`ValidationReport::select(&Selection) -> Result<RuleReport, _>` and `ValidationReport::explain(issue) -> Result<ExplainReport, _>`, implementing `selection-membership` through one predicate over findings with provenance. Producer: `report-selection`.

### `graph-finding-subject` [implementation-produced] — Firing subject

`GraphFinding.subject: Option<IssueId>` is the issue whose rule application fired the finding; `None` for `label-uniqueness` and rule-level configuration errors. Producer: `graph-finding-subject`.

### `parsed-records` [implementation-produced] — One records loader

One `repository_state` function parses the index and every indexed record from a byte source into parsed records, checks embedded ids, reconciles a captured `.jit/issues` listing, and returns typed errors. Producer: `records-loader`.

### `built-in-pass` [implementation-produced] — Built-in pass names

`BuiltInPass` in the validation layer, declared through the wire-vocabulary macro, names `rules-file`, `dangling-item-link`, `enforcement-drift` and `review-placeholder`. Producer: `built-in-pass-vocabulary`.

### `planning-node-lookup` [implementation-produced] — Planning node in templates

`crate::templates` owns `find_planning_node`, `planning_node_plan_path`, the plan-document label and `declared_container_label` with the container-token interpolation. Producer: `planning-node-lookup-in-templates`.

### `plan-document-resolver` [implementation-produced] — One plan resolver

One `crate::templates` function maps a container, the issues by id and the template registry to its plan-document reference and whether its planning node is done. Producer: `one-plan-document-resolver`.

### `config-default-accessors` [implementation-produced] — Configuration defaults

`JitConfig::content_format()` (Markdown when undeclared) and `JitConfig::strictness()` (loose when undeclared). Producer: `config-default-accessors`.

### `compiled-local-rules` [implementation-produced] — Compiled local rules

A compiled local-rule set owns the desugared schemas and one `SchemaEngine` and evaluates many issues; `evaluate_local` keeps its signature over a one-issue set. Producer: `compiled-local-rules`.

### `address-index` [implementation-produced] — Item address index

A domain address index over issue-embedded and project items resolves canonical and issue-scoped addresses; `CapturedRepositoryDeclarations` carries the item kinds resolved once. Producer: `item-address-index`.

### `image-test-builder` [implementation-produced] — Test images

`jit::test_harness` builds a closed `RepositoryImage` from any `RepositoryEntry` values (files, absences, directories, symlinks, tail reads), listings and pinned document evidence keyed by revision and path, over a derived or caller-supplied capture spec and a fixed or caller-supplied layout. Producer: `image-test-builder`.

### `incremental-capture-spec` [implementation-produced] — Spec growth

`CaptureSpec::discover_paths`, `discover_tail` and `discover_listings` validate only added entries and the listing count, clone nothing, and leave the spec unchanged on refusal. Producer: `capture-spec-incremental`.

### `claims-index-input` [implementation-produced] — Interim claims-index input

`ReportInputs` carries the claims-index problems the executor reads over its own worktree paths, with today's test-mode skip, while the machine-local claims-index check exists. It is a named temporary bridge: producer `pure-validation-report`; its one remover is f8a6b298's 98184c64, whose REQ-03 removes every validation-report input carrying claims-index problems and the executor code that fills it (DEC-09). Removal criterion: neither `ReportInputs` nor any executor code names the claims index.

### `creation-findings` [implementation-produced] — Creation findings

Issue creation returns `findings` (wire `ReportedFinding`): the new issue's local-rule findings and the graph-rule findings attributed to it, from the mutation's captured issues plus the new record, the mutation timestamp and plan content from the mutation image. Producer: `create-findings-in-session`.

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| planning-node-lookup-in-templates | Planning-node lookup lives in the template module | task | Planning-node and container-label resolution move from commands into crate::templates. | — | REQ-06, DEC-05 | touches 6 | — | — |
| one-plan-document-resolver | One resolver for plan-document references | task | Capture discovery, plan-content projection and repository validation resolve plan documents through one function. | planning-node-lookup | REQ-12, DEC-05 | touches 3 | — | planning-node-lookup-in-templates |
| config-default-accessors | Configuration accessors for content format and strictness | task | JitConfig states the content-format and strictness defaults once, and the validation layer reads them there. | — | REQ-12 | touches 3 | — | — |
| command-config-defaults | Commands read content format and strictness through the configuration accessors | task | Command-layer readers of the content-format and strictness defaults go through the JitConfig accessors. | config-default-accessors | REQ-12 | touches 6 | — | config-default-accessors |
| built-in-pass-vocabulary | Built-in validation pass names are one validation-layer vocabulary | task | The built-in pass rule names become a closed vocabulary in the validation layer. | — | REQ-06, DEC-05 | touches 8 | — | config-default-accessors |
| compiled-local-rules | Local rules compile each schema once per evaluation set | task | Repository validation evaluates its issues through one compiled local-rule set, compiling each schema once. | — | REQ-11 | touches 3 | — | config-default-accessors |
| graph-finding-subject | Graph findings record their firing subject | task | Each graph finding records the issue whose rule application produced it, separately from its attribution. | — | REQ-02, DEC-07 | touches 1 | — | — |
| container-pointer-index | Container pointers resolve through an index built once per graph evaluation | task | Graph evaluation resolves container-from-label pointers through one id index instead of normalising candidate ids per pointer. | — | REQ-07, REQ-08, DEC-10 | touches 2 | — | graph-finding-subject |
| item-address-index | Item links resolve against one address index with item kinds resolved once | task | Repository validation resolves item links through one address index, and resolves item kinds once from the captured declarations. | — | REQ-07, DEC-10 | touches 5 | — | built-in-pass-vocabulary |
| records-loader | One loader parses the index and issue records | task | The index and issue records are parsed by one loader shared by validation, capture and mutation bases. | — | REQ-10, DEC-05 | creates 1, touches 4 | — | — |
| pure-validation-report | One pure function computes the validation report | task | A single pure validation-layer function computes the whole-repository report from a captured image and injected inputs. | parsed-records, address-index, compiled-local-rules, plan-document-resolver, built-in-pass | REQ-01, REQ-06, DEC-05, DEC-09 | touches 9 | — | records-loader, item-address-index, compiled-local-rules, one-plan-document-resolver |
| capture-hands-records | Validation capture hands its parsed records to the report | task | Whole-repository validation parses its records once, during capture, and the report reads them. | validation-report, parsed-records | REQ-10, DEC-05 | touches 2 | — | pure-validation-report |
| image-test-builder | Shared test fixture for repository images | task | Tests close a RepositoryImage over declared entries, listings and pinned evidence through one shared fixture. | — | REQ-04, DEC-11 | touches 1 | — | — |
| image-builder-copies-converge | Repository-state and document test images use the shared builder | task | Core repository-state and document test modules build images through the shared builder and keep no private copy. | image-test-builder | REQ-04, DEC-11 | touches 6 | — | image-test-builder |
| profile-test-images-converge | Export, initialization and profile test images use the shared builder | task | Export, initialization and profile-application test modules build images through the shared builder. | image-test-builder | REQ-04, DEC-11 | touches 3 | — | image-test-builder |
| report-selection | Validation views are selections of the report | task | ValidationReport answers whole, issue, scope, rule and explain selections through one membership predicate. | validation-report, graph-finding-subject, selection-membership, built-in-pass | REQ-02, DEC-05, DEC-06, DEC-07 | touches 4 | — | pure-validation-report, graph-finding-subject, command-config-defaults |
| dag-strategy-in-test-harness | The acyclic-graph proptest strategy lives in the test harness | task | The acyclic dependency-graph proptest strategy moves to the library test harness for unit-test modules to share. | — | REQ-04, DEC-11 | touches 2 | — | — |
| selection-contract-proptest | One property test owns the selection contract | task | A proptest over generated images checks each selection against an oracle written from the membership contract. | report-selection, image-test-builder, selection-membership | REQ-04, DEC-11 | creates 1, touches 1 | — | report-selection, image-test-builder, dag-strategy-in-test-harness |
| issue-views-over-selection | Per-issue and explain validation select from the report | task | jit validate <id>, the whole-repository rule report and --explain capture once and select from the one report. | report-selection, validation-report, selection-membership | REQ-02, REQ-08, DEC-07, DEC-11 | touches 3 | — | report-selection, capture-hands-records |
| rule-view-over-selection | Single-rule validation selects from the report | task | jit validate <id> --rule <rule> captures once and selects the rule's findings for the issue from the one report. | report-selection, validation-report, selection-membership | REQ-02, DEC-11 | touches 2 | — | report-selection, capture-hands-records |
| scope-view-over-selection | Scope validation selects from the report | task | jit validate --scope evaluates rules over the whole repository and restricts the report to the container's bracket scope. | report-selection, validation-report, selection-membership | REQ-02, DEC-06, DEC-11 | touches 2 | — | report-selection, capture-hands-records |
| invariant-drift-over-declarations | Invariant check computes drift from captured declarations | task | jit invariant check computes drift through the report's drift function over one captured image. | validation-report | REQ-05, DEC-08 | touches 3 | — | report-selection |
| gate-checkers-over-selection | Built-in validation gate checkers select from the report | task | Each built-in validation checker computes the one report over its image and returns its selection. | report-selection, validation-report, checker-selections, selection-membership | REQ-03, REQ-05, DEC-03, DEC-11 | touches 2 | — | rule-view-over-selection |
| executor-pass-cases-to-report-tests | Report pass tests cover the executor-pass cases | task | The report's item-link, drift and placeholder tests assert each case the executor-pass unit tests cover. | validation-report, image-test-builder | REQ-05, DEC-11 | touches 1 | — | pure-validation-report, image-test-builder |
| executor-pass-copies-removal | Remove the executor-side validation pass copies | task | The executor's dangling-link, drift and placeholder passes and their unit tests are deleted. | validation-report | REQ-05 | touches 2 | — | issue-views-over-selection, scope-view-over-selection, invariant-drift-over-declarations, gate-checkers-over-selection, executor-pass-cases-to-report-tests |
| validation-layer-tests-off-commands | Validation-layer tests and prose stop naming commands | task | Nothing under crates/jit/src/validation names a commands item, tests and rustdoc included. | image-test-builder, validation-report | REQ-06 | touches 4 | — | executor-pass-copies-removal |
| validation-view-docs | Validation and gate references describe views as report selections | task | The validation, gate and invariant references describe views as selections of one report. | selection-membership, checker-selections | REQ-02, REQ-03, DEC-06, DEC-07, DEC-08 | touches 6 | — | issue-views-over-selection, scope-view-over-selection, invariant-drift-over-declarations, gate-checkers-over-selection |
| capture-spec-incremental | Capture specs validate only added entries | task | Growing a capture spec validates only the entries a call adds, without cloning the spec. | — | REQ-09, DEC-16 | touches 1 | — | — |
| document-reference-round-batched | Validation capture requests a round's document references as one batch | task | Each validation capture round adds its document-reference paths in one batch and applies the base requests once. | incremental-capture-spec | REQ-09, DEC-16 | touches 1 | — | capture-spec-incremental, capture-hands-records |
| link-probe-pool | Link checking probes each distinct URL once through one timed agent | task | Document link checking probes each distinct external URL once, through one HTTP agent with a timeout and bounded threads. | — | REQ-15, DEC-15 | touches 4 | — | — |
| create-findings-in-session | Issue creation derives its findings inside its mutation | task | Issue creation returns the new issue's advisory findings, derived from the image its mutation captured. | — | REQ-13, DEC-13 | touches 4 | — | command-config-defaults |
| create-output-reports-findings | Issue creation output shows its findings, with no read after the mutation commits | task | issue create shows the returned findings identically in human and JSON output with no read after the mutation commits. | creation-findings | REQ-13, DEC-13 | touches 7 | — | create-findings-in-session |
| executor-graph-evaluation-removal | Remove the executor's graph evaluation and the scoped graph entry points | task | Graph findings come only from the report's evaluation and the mutation derivations; the partial entry points are gone. | validation-report | REQ-01 | touches 5 | — | create-output-reports-findings, validation-layer-tests-off-commands, container-pointer-index |
| fix-in-one-mutation | One mutation and one post-fix report for validate --fix | task | --fix derives its fixes from one capture, commits them in one mutation and always shows one post-fix report. | validation-report | REQ-14, DEC-14, DEC-04 | touches 12 | — | capture-hands-records |
| side-path-output-docs | Command references describe create findings, fix reporting and link probing | task | The command reference states the create, fix and link-check behaviour once. | creation-findings | REQ-13, REQ-14, REQ-15, DEC-13, DEC-14, DEC-15 | touches 3 | — | create-output-reports-findings, fix-in-one-mutation, link-probe-pool |
| validation-cost-measurement | Measure validation views on this repository | task | A scripted release-build run of interleaved pairs on a copy of this repository records per-issue against whole validation. | — | REQ-08, DEC-12 | creates 1, touches 1 | — | issue-views-over-selection, scope-view-over-selection, document-reference-round-batched, create-output-reports-findings, fix-in-one-mutation, container-pointer-index |
| report-core | One pure validation report over parsed records | story | One pure report function over once-parsed records, shared resolvers and compiled rules. | — | REQ-01, REQ-06, REQ-07, REQ-10, REQ-11, REQ-12 | — | — | command-config-defaults, container-pointer-index, capture-hands-records |
| report-views | Every validation view is a selection of the report | story | Every validation view and built-in validation checker is a selection of the one report. | — | REQ-02, REQ-03, REQ-04, REQ-05, REQ-06 | — | — | image-builder-copies-converge, profile-test-images-converge, selection-contract-proptest, validation-view-docs, validation-layer-tests-off-commands |
| report-side-paths | Capture, creation, fix and link checking stop repeating work | story | Capture, issue creation, --fix and link checking do their work once, and the measured views meet the bound. | — | REQ-08, REQ-09, REQ-13, REQ-14, REQ-15 | — | — | executor-graph-evaluation-removal, side-path-output-docs, validation-cost-measurement |

```mermaid
flowchart LR
    N0["planning-node-lookup-in-templates: Planning-node lookup lives in the template module"]
    N1["one-plan-document-resolver: One resolver for plan-document references"]
    N2["config-default-accessors: Configuration accessors for content format and strictness"]
    N3["command-config-defaults: Commands read content format and strictness through the configuration accessors"]
    N4["built-in-pass-vocabulary: Built-in validation pass names are one validation-layer vocabulary"]
    N5["compiled-local-rules: Local rules compile each schema once per evaluation set"]
    N6["graph-finding-subject: Graph findings record their firing subject"]
    N7["container-pointer-index: Container pointers resolve through an index built once per graph evaluation"]
    N8["item-address-index: Item links resolve against one address index with item kinds resolved once"]
    N9["records-loader: One loader parses the index and issue records"]
    N10["pure-validation-report: One pure function computes the validation report"]
    N11["capture-hands-records: Validation capture hands its parsed records to the report"]
    N12["image-test-builder: Shared test fixture for repository images"]
    N13["image-builder-copies-converge: Repository-state and document test images use the shared builder"]
    N14["profile-test-images-converge: Export, initialization and profile test images use the shared builder"]
    N15["report-selection: Validation views are selections of the report"]
    N16["dag-strategy-in-test-harness: The acyclic-graph proptest strategy lives in the test harness"]
    N17["selection-contract-proptest: One property test owns the selection contract"]
    N18["issue-views-over-selection: Per-issue and explain validation select from the report"]
    N19["rule-view-over-selection: Single-rule validation selects from the report"]
    N20["scope-view-over-selection: Scope validation selects from the report"]
    N21["invariant-drift-over-declarations: Invariant check computes drift from captured declarations"]
    N22["gate-checkers-over-selection: Built-in validation gate checkers select from the report"]
    N23["executor-pass-cases-to-report-tests: Report pass tests cover the executor-pass cases"]
    N24["executor-pass-copies-removal: Remove the executor-side validation pass copies"]
    N25["validation-layer-tests-off-commands: Validation-layer tests and prose stop naming commands"]
    N26["validation-view-docs: Validation and gate references describe views as report selections"]
    N27["capture-spec-incremental: Capture specs validate only added entries"]
    N28["document-reference-round-batched: Validation capture requests a round's document references as one batch"]
    N29["link-probe-pool: Link checking probes each distinct URL once through one timed agent"]
    N30["create-findings-in-session: Issue creation derives its findings inside its mutation"]
    N31["create-output-reports-findings: Issue creation output shows its findings, with no read after the mutation commits"]
    N32["executor-graph-evaluation-removal: Remove the executor's graph evaluation and the scoped graph entry points"]
    N33["fix-in-one-mutation: One mutation and one post-fix report for validate --fix"]
    N34["side-path-output-docs: Command references describe create findings, fix reporting and link probing"]
    N35["validation-cost-measurement: Measure validation views on this repository"]
    N36["report-core: One pure validation report over parsed records"]
    N37["report-views: Every validation view is a selection of the report"]
    N38["report-side-paths: Capture, creation, fix and link checking stop repeating work"]
    N0 --> N1
    N2 --> N3
    N2 --> N4
    N2 --> N5
    N6 --> N7
    N4 --> N8
    N9 --> N10
    N8 --> N10
    N5 --> N10
    N1 --> N10
    N10 --> N11
    N12 --> N13
    N12 --> N14
    N10 --> N15
    N6 --> N15
    N3 --> N15
    N15 --> N17
    N12 --> N17
    N16 --> N17
    N15 --> N18
    N11 --> N18
    N15 --> N19
    N11 --> N19
    N15 --> N20
    N11 --> N20
    N15 --> N21
    N19 --> N22
    N10 --> N23
    N12 --> N23
    N18 --> N24
    N20 --> N24
    N21 --> N24
    N22 --> N24
    N23 --> N24
    N24 --> N25
    N18 --> N26
    N20 --> N26
    N21 --> N26
    N22 --> N26
    N27 --> N28
    N11 --> N28
    N3 --> N30
    N30 --> N31
    N31 --> N32
    N25 --> N32
    N7 --> N32
    N11 --> N33
    N31 --> N34
    N33 --> N34
    N29 --> N34
    N18 --> N35
    N20 --> N35
    N28 --> N35
    N31 --> N35
    N33 --> N35
    N7 --> N35
    N3 --> N36
    N7 --> N36
    N11 --> N36
    N13 --> N37
    N14 --> N37
    N17 --> N37
    N26 --> N37
    N25 --> N37
    N32 --> N38
    N34 --> N38
    N35 --> N38
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| DEC-01 Upstream 2195c98f | Chosen: each root depends on the specific 2195c98f tasks whose code it edits, reduced so no edge is implied by another; the epic-level 9876a23a → 2195c98f edge stays. Rejected: one edge per root onto the whole epic, which holds every entry until 2195c98f closes. External dependency 855d027d → `planning-node-lookup-in-templates`, `config-default-accessors`, `graph-finding-subject`, `image-test-builder`, `dag-strategy-in-test-harness`. External dependency 42bcaa6e → `config-default-accessors`. External dependency 7e8f1525 → `graph-finding-subject`. External dependency e1a6c8c2 → `records-loader`. External dependency f8f80eea → `pure-validation-report`. External dependency 72dfc0d7 → `pure-validation-report`. External dependency bc1955a8 → `item-address-index`. External dependency b4fe7887 → `item-address-index`. Reached transitively, so a direct edge would be rejected as redundant: 1c5da41d, 7abcc22d, c400730c, 0102769f, 5fac48b8, 75951558, 804a8605, d72bf093 and c1c86312 (through 855d027d), 96bb0941 and 3f026bb1 (through 42bcaa6e), 5613ef50, af9c277d, b48d7b2a and 6826f966 (through 7e8f1525), 62a0085a (through f8f80eea), and every 842b0ac6 task, e1af39db, cd7b6c4a, 81e7cdc3 and 69af08af included (every 2195c98f root depends on 842b0ac6). |
| DEC-02 f8a6b298 and f4d9502a neighbours | Chosen: our entries wait where they edit a function those tasks edit. External dependency 5b0e3d63 → `records-loader` (`load_records`), which `capture-hands-records` (capture base) follows; `document-reference-round-batched` and `fix-in-one-mutation` follow it through that entry. External dependency c3e3e2ff → `pure-validation-report` (claims-index reader). External dependency 725af076 → `capture-spec-incremental` (`discover_paths`). External dependency 725af076 → `image-builder-copies-converge` (test image helpers). No edge onto f649c144 or 47d62472: no entry edits `validate_leases`, `check_worktree_exists` or `validate_branch_drift`. No edge onto 98184c64 (DEC-09; see the claims row). Rejected: edges from those tasks onto ours, which would delay other epics. |
| DEC-03 39710b3b | Chosen: external dependency 39710b3b → `gate-checkers-over-selection`; the checker entry keeps its fault classification (`checker-selections`). Rejected: landing first and changing the dispatch under 39710b3b. |
| DEC-04 Steward rulings | (1) External dependency 8b7f2973 → `create-findings-in-session`; `create-output-reports-findings` follows it. (2) `fix-in-one-mutation` publishes every fix through one mutation with the existing `Issue::derive_readiness_correction` applied to the projected set until none remains; no parallel readiness derivation. Edge for b697a3f7's planner: `fix-in-one-mutation` → b697a3f7's readiness-derivation task (that task depends on `fix-in-one-mutation` and converges its fixpoint loop later). Rejected: introducing the pure cascade here. (3) 525bd61a is rejected as a duplicate; `fix-in-one-mutation` REQ-03 covers the zero-fix case with a test that fails today. (4) No edges onto 5962427b, 06ee9b67 or 9cc3056a (see Integration notes). External dependency 9ecde705 → `link-probe-pool`. No edge onto f37d228e, which is done. 725af076 and 5b0e3d63 are wired under DEC-02. |
| Integration notes (DEC-04(4)) | df129d9e wave-4 tasks and ours touch the same surfaces with no edge; whoever lands second adapts. If 5962427b's validation facade exists when a view or report entry lands, the entry's methods live on `ValidationOps` and keep the facade's bounds; if 06ee9b67's validate handler exists, the view, `--fix` and whole-report entries edit `crates/jit/src/dispatch/validate.rs` instead of the `main.rs` arm and return its findings outcome; if 9cc3056a's findings renderer exists, human findings render through it. 3f816619 moves the `issue create` arm to a handler: `create-output-reports-findings` edits whichever exists. Profile-lifecycle tasks of 3d9a0ce3 may rewrite `commands/profile.rs`; `pure-validation-report` changes only its report call and error variant there. |
| DEC-05 One report | Chosen: `validation-report` with selections (`report-selection`), reusing `Finding`, `ReportedFinding`, `RuleReport`, `RuleOutcome` and `ExplainReport`. Rejected: keeping the `Result<Report, Failure>` pair and filtering per view, which leaves every view matching two shapes. |
| DEC-06 Scope as restriction | Chosen: rules evaluate over the whole repository and scope restricts by attribution (`selection-membership`); stated with a test that fails today in `scope-view-over-selection`. Rejected: evaluating rules over a sliced image, a second computation. |
| DEC-07 Narrowed configuration errors | Chosen: issue and explain selections keep configuration errors whose subject is the issue or absent; `rules-file` always in, `enforcement-drift` always out; `GraphFinding` gains `subject`. Asserted in `report-selection` and, with tests that fail today, `issue-views-over-selection`. Rejected: today's "any configuration error of an applicable rule", which reports other issues' pointer errors. |
| DEC-08 Unloadable `gates.toml` | Chosen: the report fails with a typed error naming the file, and `invariant-drift-over-declarations` makes `jit invariant check` do the same, flipping `test_check_reports_unloadable_gate_registry`. Rejected: tolerating it as unloadable drift in one surface only. |
| DEC-09 Claims-index check | Chosen: `pure-validation-report` takes the claims-index problems as a `ReportInputs` field computed by the executor over its own worktree paths, so the validation layer imports nothing from `commands` regardless of when f8a6b298 deletes the check. The input is the bridge `claims-index-input`. 98184c64 REQ-03 removes every validation-report input carrying claims-index problems and the executor code that fills it, which makes it the one remover of `claims-index-input` in either landing order; `pure-validation-report` REQ-02 holds while the check exists. Rejected: waiting on 98184c64, which would leave REQ-06's claims clause vacuous and hold the core behind the lease-store rewrite. |
| DEC-10 Address index | Chosen: `item-address-index` extracts the existing set into a domain index and resolves item kinds once on captured declarations; `container-pointer-index` builds one id index per graph evaluation. Rejected: an index keyed to `AddressableItem` that also serves `show_item`, a wider scope than REQ-07. |
| DEC-11 Selection test ownership | Chosen: `selection-contract-proptest` owns the contract; per-view tests shrink to wiring in each view entry; `image-test-builder`, `image-builder-copies-converge` and `profile-test-images-converge` give one image builder, accepting any `RepositoryEntry`, listings, pinned evidence and a derived or caller-supplied spec, that every test module closing an image uses; `RepositoryImage`'s own `close` tests and the production closes in `apply_overlay` and storage capture are not helpers (`@/inv/shared-test-contracts`). The proptest's oracle is written from `selection-membership`, independent of the production predicate. Rejected: per-view example matrices. |
| DEC-12 REQ-08 evidence | Chosen: `validation-cost-measurement` generalises `scripts/benchmark-mutation-cost.sh` and records a release-build run in `dev/active/9876a23a-one-validation-report/benchmark/`: at least five interleaved `validate`/`validate <id>` pairs, the median pair difference judged against zero, the load threshold stated. A miss is a blocking finding against `issue-views-over-selection`. Rejected: an automated wall-clock bound in the suite (debug build, shared-host noise, suite budget). |
| DEC-13 Creation findings | Chosen: findings from the mutation's captured image (`creation-findings`), identical in human and JSON output, not suppressed by `--force`; JSON gains `IssueCreateResponse.findings`. Document-reading passes are not evaluated, since a mutation image holds no issue documents. Rejected: capturing the full validation closure on every create. |
| DEC-14 One-mutation `--fix` | Chosen: no pre-fix report; hierarchy, reduction, readiness and derived-state fixes fold into one plan from one capture; one post-fix report always rendered, its exit status authoritative; `--dry-run` runs the same derivation. `jit recover` keeps its derived-state-only repair. Rejected: separate publishers under one held session, which still writes one redo record per publisher. |
| DEC-15 Link probing | Chosen: one `ureq` agent with a request timeout, distinct URLs deduplicated, a fixed `std::thread::scope` pool; transport failures and timeouts stay `unreachable_url` errors and the never-produced `url_validation_failed` branch goes. Rejected: rayon or tokio (new dependency); reclassifying transport failures as warnings, a behaviour change REQ-15 does not ask for. |
| DEC-16 Incremental capture | Chosen: per-entry validation in `capture-spec-incremental`; one batched worktree request per document-reference round and the base applied once in `document-reference-round-batched`. Other one-at-a-time callers (`gate_check.rs` precheck documents, `overlay.rs`) become linear through the incremental spec and are not batched. Rejected: batching callers over the clone-and-revalidate spec. |
| Investigation corrections | Background C5 is corrected in the epic: one capture per plain validate, document-reference discovery twice per capture, several captures in `--fix`. Content-format and strictness counts are stated structurally in `command-config-defaults`. REQ-07 is partly done: the report already builds one address set. |
| Behaviour changes | Each is stated with its asserting test in the owning entry: scope sees the whole repository and reports `rules-file` for an unloadable rule set; the label-target checker sees the whole repository; issue and explain configuration errors, and the issue checker's, narrow; issue views include `rules-file` and exclude drift; the rule view and the issue checker report `rules-file` for an unloadable rule set; `jit invariant check` fails on an unloadable gate registry; `issue create` reports findings in JSON and `--force` no longer hides them; `--fix` renders and exits by its post-fix report; an unresponsive URL fails after the timeout instead of stalling. |
| Terminal sizing | Each terminal delivers one class with at most four criteria. Entries kept whole for a build reason state it in Background: `records-loader` (private loaders), `pure-validation-report` (signatures that only compile together), `fix-in-one-mutation` (return value and renderer). The deletions of public executor and graph functions are separate entries because the build does not flag uncalled public items; test relocations (`executor-pass-cases-to-report-tests`, `dag-strategy-in-test-harness`, the image-builder convergences) are separate from the entries that need them. |
| Shared-file contention | `commands/validate.rs`, `validation/repository.rs`, `commands/gate_check.rs` and `commands/mod.rs` are edited by several entries in distinct functions; same-function edits are ordered by edges (configuration chains before the views and checkers, the item-link pass through `built-in-pass-vocabulary` then `item-address-index`, the drift pass through `report-selection` then `invariant-drift-over-declarations`, `rule_validation_report` through `rule-view-over-selection` then `gate-checkers-over-selection`). `image-test-builder` and `dag-strategy-in-test-harness` add distinct items to `test_harness.rs`. `planning-node-lookup-in-templates` and `built-in-pass-vocabulary` both edit the re-export lines at `commands/mod.rs:96-97`; workers rebase. |

## Investigation sources

- [Investigation](investigation.md): pipeline map, claim classification, primitive verification, consumer sweeps and option analyses for the report and its views, and for capture, creation, `--fix` and link checking.

## Background corrections at a860614ac (execution lead)

Factual line corrections with criteria unchanged, given to workers at dispatch. The full audit is in [audit.md](audit.md).

- d40d6f74: `find_planning_node` `commands/validate.rs:2352`, `PLAN_DOC_LABEL` `:2391`, `planning_node_plan_path` `:2400`, re-export `commands/mod.rs:107`; `declared_container_label` `commands/template_expand.rs:613`, `InterpolationContext` `:517`; callers `validate.rs:1013,1088`, `validation/repository.rs:655-656`, `commands/template.rs:883`; test `validate.rs:2599`.
- 7eb443b2: `GraphFinding` `validation/graph.rs:199-245`; `for_issue` `:214`, `unattributed` `:226`, `issue_finding` `:661`, `config_error` `:670`; `evaluate_one` `:522`, `evaluate_label_uniqueness` `:612`, `evaluate_label_coverage` `:733`, `resolve_container` call `:776`.
- bd2534bd: `arbitrary_dag` `graph/hierarchy.rs:1084`; proptest `crates/jit/Cargo.toml:113`; `lib.rs:39-40`.
- 6100b960: `RepositoryImage::close` `image.rs:780`; private builders at `repository_state/mod.rs:1080,1277`, `mutation.rs:1319`, `archive.rs:621-655`, `document/reference.rs:266-351`; `initialize.rs` no longer closes an image.
