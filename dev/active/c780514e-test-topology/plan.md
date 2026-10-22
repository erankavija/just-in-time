# Plan: Give each test property one owning test on shared fixtures and contracts (c780514e)

> Planning node: 7c885165. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

Fixtures land first, then the per-target migrations onto them; owners gain missing cases before re-proofs are deleted. Each entry lands green. Per-test-bound relief (REQ-11 key, REQ-13 outliers) starts in the first wave with no upstream edge.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | `exit-code-projection-ownership`: the projection suite derives its verified set from its binding cases and absorbs the two row-less parity cases. The 25 unique `exit_code_tests` cases move in three families (envelopes, claim and delete, arguments) before the two re-proof files are deleted. One sweep per CLI target strips projected exit-code assertions in numeric, multi-line, symbolic and helper form, and keeps error-class evidence through the envelope code. | Investigation §1 #1–#3, §D.1, §D.2 |
| REQ-02 | `domain-query-ownership`: extract the `query available` composition and order, and the auto-transition decision, into pure domain functions. Then cut `query_tests`, the label and filter CLI files, and the executor label tests to wiring cases, cut cycle re-proofs, move six `harness_demo` cases and delete the suite. | §1 #4–#8, §5, §D.3 |
| REQ-03 | Add missing finalization cases to `repository_state`, then delete the storage determinism section. | §1 #9 |
| REQ-04 | `list-envelope-coverage`: sixteen list-emitting commands found from their emission sites, not the schema, gain typed `ListEnvelope` outputs (six inline bodies plus a schema key, then nine command-layer results). One schema-driven test then replaces the 19 per-command cases with a runtime guard against untyped list output, and the per-command re-assertions go. | §1 #10, §4, §D.4 |
| REQ-05 | `cli-process-fixture` (wave 1), then one spawn migration per file family (23 families across the eight CLI target slices), plus a separate JSON-id migration per target for the files that scrape human-readable output (D-06). Git setup adoptions are split per test boundary (library, server, `fast_docs_templates`). File-backed and memory-backed setups each get a shared-constructor foundation, then adoptions per boundary across the whole workspace: the three in-process suites, the in-process helpers of three CLI suites, two library module families plus the library file-backed modules, and the server crate. The mechanical sweep in §D.6 classifies every candidate helper and names its owner. Files whose properties move elsewhere are excluded from migration and handled by their owner entries. | §1 #11–#17, §C, §D.6 |
| REQ-06 | Wave-1 foundations: shared `fixture_issue`, shared `TaxonomyRepo`, and product initialisation over any store (followed by the shared memory executor built on it). One adoption per test boundary then retires each copy: library and four suites for `fixture_issue`, two CLI suites for `TaxonomyRepo`, `fast_rules` for the executor. The memory-backed adoptions in the suites, the library and the server remove every private memory executor and literal index body. | §1 #18, §4, §D.6 |
| REQ-07 | `shipped-item-kinds-fixture` derivation (wave 1), then one adoption each for the harness tables, command unit tests, `cli_item_validate` and `fast_rules`. | §1 #20–#21 |
| REQ-08 | `issue-store-conformance` (absorbs the three ad-hoc tests), then the in-memory path-read fix, then backend-test pruning. | §1 #22–#24 |
| REQ-09 | The receipt env lock goes with the runner hook; an audit puts unguarded environment readers under `EnvGuard`. | §1 #25, §4 |
| REQ-10 | `script-self-test-gates`: every repository shell-script self-test runs as a gate (investigation §D.7). The ten gates are registered at planning (D-05, bootstrap) and required by their tasks and by the container. The three that the default suite wraps leave it, and seven that ran nowhere or only in CI gain gates, including three skill-script suites under `.agents/`. The corpus setup's nested cargo test is replaced by an in-suite fixture, and every skip-and-pass path fails instead: the cargo and node skips, the root-only lock tests, and the read-bit and actionlint skips inside the docs-check and workflow-contract self-tests. | §1 #26–#28, #32, §D.5, §D.7 |
| REQ-11 | `input-keyed-fixture-cache`, then removal of the hidden subcommand, receipts and setup scripts. | §1 #29–#32, §A |
| REQ-12 | Final footprint report under `suite-measurement-protocol`. | §1 #34, §C |
| REQ-13 | Early relief: input key, in-memory rule-warning tests, short-deadline server drain, doctest pruning. Final suite-clock report. | §A; D-01 |

## Shared architectural contracts

### `cli-process-fixture` [implementation-produced] — Shared CLI process fixture

A test-support module compiled under `cfg(any(test, feature = "test-support"))`. It owns a temporary repository: uninitialised, `jit init`-ed, or one from shared test support handed to it (for example a `TaxonomyRepo`). It locates the target's `jit` binary through one shared construction path, because `CARGO_BIN_EXE_jit` exists only while an integration target compiles. It yields a `std::process::Command` with every inherited `JIT_*` variable removed, and per-process overrides are set through the fixture. It initialises git on request through a test-support function that in-process tests also call, and it reads created ids from `--json` output. It adds no integration target (`@/inv/bounded-rust-build-footprint`).

### `shipped-item-kinds-fixture` [implementation-produced] — Shipped item kinds from the package

Test support reads the item-kinds contribution of `profiles/jit-default/manifest.toml` and exposes it in the shapes harness, unit and suite tests need. A deliberately custom kind is a minimal literal that says it is custom. Engine code gains nothing (`@/inv/domain-agnostic`).

### `file-setup-constructors` [implementation-produced] — File-backed in-process setups

Shared test support constructs an initialised file-backed repository from caller-supplied configuration, an executor over a file-backed root with its discovered layout, and a temporary declaration directory from caller-supplied files. Adoptions in the in-process suites, the in-process helpers of the CLI suites, the library test modules and the server crate replace the private helpers that investigation §D.6 classifies as setup.

### `memory-declaration-seeding` [implementation-produced] — Memory stores from declarations

Shared test support builds a memory store and executor from caller-supplied configuration, rules and templates, on top of the shared memory executor, which product initialisation sets up. Scenario declarations stay in tests as data. A test writes no index body and restates no value that product initialisation already writes. Scenario values, for example `enforce_leases = "off"`, reach the shared constructor as declarations. Adoptions cover the in-process suites, the in-process helpers of the CLI suites, the library test modules and the server crate. Investigation §D.6 lists each helper with its owner.

### `input-keyed-fixture-cache` [implementation-produced] — Input-keyed fixture cache

The profiled-repository cache key is a digest of the captured package closure (path, bytes, mode), a digest of the `jit` crate source (`src/`, `Cargo.toml`, `build.rs`) and a setup-contract token. No executable's bytes or path are part of it. Entries are built in-process on first use under the existing no-replace publication, and publishing collects the profile's older entries. The recorded-failure corpus uses the same mechanism, keyed on the failure-lever registry and the crate source digest.

### `issue-store-conformance` [implementation-produced] — IssueStore conformance suite

One test module written against the `IssueStore` trait, with reader and writer cases run against each backend through one construction seam. It is the suite named by 89e7a273 REQ-04 (D-03).

### `exit-code-projection-ownership` [plan-fixed] — Projection suite owns exit codes

`CommandSchema::generate().command_exit_codes` is the binding source. Each row has exactly one binding case in both invocation forms, and the completeness check derives its verified set from those cases. The wildcard rows bind every command. Outside the suite, a subprocess test asserts success or failure and the property it adds (verdict, envelope code, remediation text, empty stdout). It names a numeric exit code only for a command and code pair with no row. Where a code was a test's only evidence of the error class on its path, the test asserts that class another way, such as the `--json` envelope's `error.code`. Recorded failure-corpus statuses are data, not assertions.

### `list-envelope-coverage` [plan-fixed] — One list-envelope test

List-emitting commands are derived from emission sites: every serialized struct with a numeric `count` field and every inline `json!` body with a `"count"` key (investigation §D.4). Each one that counts a sibling collection becomes a `ListEnvelope` instantiation, declared as its command's output schema in `schema.rs`. §D.4 names the counts that are not list envelopes. The test enumerates list commands from the generated schema. It holds one invocation per command, covering the commands of the 19 removed cases and the §D.4 inventory. It fails naming any schema list command without an invocation, and asserts `count == len` through the CLI process fixture. Independently of the schema, it also fails when an invocation's JSON carries `count` beside an array while that command's schema is the untyped fallback.

### `domain-query-ownership` [plan-fixed] — Domain owns query and transition semantics

Filter composition, priority order and the auto-transition decision are pure functions in `domain/`, with unit and property tests there. Cycle detection stays owned by `graph/`. Commands call these functions. CLI tests hold one wiring case per subcommand or flag, and the harness holds one cascade wiring case.

### `script-self-test-gates` [plan-fixed] — Script self-tests as gates

This covers every shell-script self-test in the repository (investigation §D.7):

- three that the default suite wraps today: `scripts/cargo-ci-selftest.sh`, `scripts/profile-test-suite.sh --self-test` and the `scripts/rust-build-budget.sh` cases, which move into the script;
- seven that ran nowhere or only in CI: `scripts/docs-check-selftest.sh`, `scripts/generate-shipped-policy-regions-selftest.sh`, `scripts/workflow-contract-selftest.sh`, `scripts/benchmark-session-cost-selftest.sh`, and the skill suites `test-standards-scan.sh`, `test-standards-fix.sh` (`.agents/skills/jit-project-lead/scripts/`) and `test-manifest-tool.sh` (`.agents/skills/jit-planning-lead/evals/`).

Shell files that test no script (CI replays, product walkthroughs, image smoke, the MCP test runner) are recorded as such in §D.7.

`scripts/setup-recorded-failure-corpus.sh --self-test` is deleted together with its script. Each self-test runs as a `.jit/gates.toml` gate whose `[gates.inputs]` cover the script and what it exercises. The ten gate keys are registered at planning (steward ruling D-05, bootstrap): `cargo-ci-selftest`, `profile-suite-selftest`, `build-budget-selftest`, `docs-check-selftest`, `shipped-policy-regions-selftest`, `workflow-contract-selftest`, `session-cost-benchmark-selftest`, `standards-scan-test`, `standards-fix-test` and `manifest-tool-test`. Each gate task requires its own gate and makes it pass, and the container c780514e requires all ten. Only c780514e's issues require them. Where a script still has a skip branch, its gate's checker also fails on a printed `SKIP:` line, so that gate starts out failing until its task lands. `build-budget-selftest` starts out failing until its script gains `--self-test`. Any non-zero exit fails the gate, including exit 2 for a missing tool. No default-suite test wraps a script self-test. No test or self-test passes by printing a skip notice, whether a tool is missing or a host precondition such as an enforced read bit is absent: it establishes the precondition or fails. Investigation §D.5 lists each skip-and-pass path and its owner, and the matches that are not test skips. New gates carry `mode` and no deprecated `auto` key.

### `suite-measurement-protocol` [plan-fixed] — How suite cost is measured

`scripts/cargo-ci.sh` runs under the build turnstile. Warm-target and post-rebuild runs are recorded with commit, host load and command. Per-test timings come from `scripts/profile-test-suite.sh` on a warm target at load ≤ 10. Before-figures come from the investigation (§A, gate run `8e22dd96` at `0896fc29b`). Results go to `dev/benchmarks/test-topology-c780514e/README.md`.

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| cli-process-fixture | Shared fixture for spawning jit in CLI tests | task | One shared test-support fixture spawns jit over a temporary repository it owns, with inherited JIT_* variables removed. | — | REQ-05, REQ-09, D-02 | creates 1, touches 3 | — | — |
| env-reading-tests-guarded | Tests read shared process environment only under the environment guard | task | Each test that reads a variable another test sets holds the shared environment guard. | — | REQ-09 | touches 1, uncertain | — | fixture-setup-hook-removal |
| exit-code-binding-completeness | Exit-code projection suite proves a binding behind each row | task | The projection suite fails when a projected exit-code row lacks a binding case in both invocation forms. | exit-code-projection-ownership | REQ-01, D-04 | touches 1, uncertain | — | cli-issue-failure-corpus-spawn-migration |
| exit-code-reproof-deletion | Exit-code re-proof suites are deleted | task | exit_code_tests.rs and the parity suite are gone; the projection suite alone binds exit codes. | exit-code-projection-ownership | REQ-01, D-04 | touches 3 | — | exit-code-envelope-case-moves, exit-code-claim-delete-case-moves, exit-code-issue-prefix-case-moves, exit-code-dependency-guard-case-moves, exit-code-gate-preset-case-move |
| query-available-domain-order | Available-work filtering and priority order become a pure domain query | task | query available's filter composition and priority order are one pure domain function with unit and property tests. | domain-query-ownership | REQ-02 | touches 3 | — | — |
| auto-transition-domain-decision | Auto-transition readiness is decided by a pure domain function | task | A pure domain function decides which dependents a terminal transition makes ready. | domain-query-ownership | REQ-02 | touches 3 | — | query-available-domain-order |
| harness-demo-case-moves | Unique harness demonstration cases move beside their owners | task | The six harness-demonstration cases with no other owner have owning tests in the library modules they exercise. | shipped-item-kinds-fixture | REQ-02 | touches 4 | — | harness-shipped-item-kinds, shared-memory-executor, unit-test-shipped-item-kinds |
| harness-demo-deletion | The harness demonstration suite is deleted | task | harness_demo.rs is gone, and contributor guidance cites owning tests instead. | — | REQ-02 | touches 3 | — | harness-demo-case-moves |
| finalization-owner-gaps | Repository-state tests own each finalization case | task | Each finalization case the storage determinism section covers has an owning repository_state test. | — | REQ-03 | touches 1 | — | — |
| finalization-reproof-deletion | Storage tests stop re-proving mutation finalization | task | The storage determinism section is gone, and storage tests cover only backend-added behaviour. | — | REQ-03 | touches 1 | — | finalization-owner-gaps |
| issue-store-conformance-suite | Shared IssueStore conformance suite runs against both backends | task | One reader and writer conformance suite runs unchanged against JsonFileStorage and InMemoryStorage. | — | REQ-08, D-03 | creates 1, touches 2 | — | finalization-reproof-deletion |
| memory-backend-path-reads | In-memory path reads serve the store's own commit-pinned state | task | InMemoryStorage path reads, decoding and commit-pinned reads pass the shared conformance suite. | issue-store-conformance | REQ-08 | touches 3, uncertain | — | issue-store-conformance-suite |
| backend-specific-test-pruning | Backend tests keep only backend-specific behaviour | task | JSON and in-memory storage tests assert only behaviour their own backend has. | issue-store-conformance | REQ-08 | touches 3 | — | memory-backend-path-reads |
| profiled-fixture-input-key | Profiled-repository fixture keyed only on its inputs | task | The profiled-repository fixture key derives from package bytes and crate source, and stale entries are collected. | — | REQ-11, REQ-13, D-01 | touches 8 | — | — |
| fixture-setup-hook-removal | Fixture setup runs inside the suite with no runner hook | task | The product binary has no hidden fixture subcommand, and the runner configuration has no profiled-fixture setup script. | input-keyed-fixture-cache | REQ-11, REQ-09 | touches 6 | — | profiled-fixture-input-key |
| failure-corpus-in-suite | Recorded-failure corpus built in the suite under an input key | task | The recorded-failure corpus comes from the input-keyed fixture cache, with no nested cargo test or setup script. | input-keyed-fixture-cache | REQ-10, REQ-11 | touches 9 | — | fixture-setup-hook-removal, cli-issue-failure-corpus-spawn-migration |
| rule-warning-tests-down-a-layer | Create-time rule warnings proven in-process without a profiled repository | task | The four create-time warning cases run in memory against the workflow profile's rules. | suite-measurement-protocol | REQ-13, D-01 | touches 2, uncertain | — | — |
| doctest-substep-bound | Doctest substep fits within the per-test bound | task | The doctest substep takes under 5 s warm at load 10 or less, with each removed example's property owned by a unit test. | suite-measurement-protocol | REQ-13, D-01 | touches 8, uncertain | — | — |
| cargo-ci-selftest-gate | cargo-ci self-test runs as a declared gate | task | The cargo-ci self-test runs as its own gate over the script, outside the default suite. | script-self-test-gates | REQ-10 | touches 5 | — | — |
| profile-suite-selftest-gate | Suite-profiler self-test runs as a declared gate | task | The suite profiler self-test runs as its own gate over the script, outside the default suite. | script-self-test-gates | REQ-10 | touches 2 | — | — |
| build-budget-selftest-gate | Build-budget checker self-test runs as a declared gate | task | The build-budget checker's cases run as the script's own self-test under a declared gate. | script-self-test-gates | REQ-10 | touches 4 | — | — |
| dependency-policy-tool-failure | Dependency-feature policy tests fail when cargo is missing | task | The dependency-feature policy tests use the running toolchain's cargo and fail rather than skip. | script-self-test-gates | REQ-10 | touches 1 | — | — |
| build-footprint-report | Build-footprint figures before and after the epic are reported | task | A benchmark note records integration targets and active test-executable bytes before and after. | suite-measurement-protocol | REQ-12 | creates 1 | — | exit-code-envelope-ownership, query-graph-ownership, storage-contracts, shared-test-fixtures, failure-corpus-in-suite, doctest-substep-bound, cargo-ci-selftest-gate, profile-suite-selftest-gate, build-budget-selftest-gate, dependency-policy-tool-failure, mcp-parity-tool-failure, docs-check-selftest-gate, shipped-policy-regions-selftest-gate, workflow-contract-selftest-gate, session-cost-benchmark-selftest-gate, standards-scan-test-gate, standards-fix-test-gate, manifest-tool-test-gate, lock-tests-deterministic-precondition |
| suite-clock-report | Suite clock before and after the epic is reported | task | The benchmark note records the suite clock before and after, and a per-test profile with no test over 5 s. | suite-measurement-protocol | REQ-13, D-01 | touches 1 | — | build-footprint-report |
| exit-code-envelope-case-moves | Blocked-transition and startup envelope cases move beside their owners | task | Blocked-transition and startup-envelope cases live in the error-envelope suites with their non-exit assertions. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 2 | — | exit-code-binding-completeness |
| exit-code-claim-delete-case-moves | Claim and confirmed-delete cases move beside their owners | task | Claim and confirmed-delete cases live in the claim and lifecycle suites with their non-exit assertions. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 2 | — | exit-code-binding-completeness, cli-claim-worktree-spawn-migration, cli-workflow-json-ids |
| exit-code-reassertion-cli-query-graph | cli_query_graph tests stop re-asserting projected exit codes | task | The cli_query_graph suite asserts only properties it adds, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 5 | — | label-query-cli-wiring-reduction |
| exit-code-reassertion-cli-item-validate | cli_item_validate tests stop re-asserting projected exit codes | task | The cli_item_validate suite asserts only properties it adds, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 5 | — | exit-code-binding-completeness, cli-registry-render-spawn-migration, cli-validate-json-ids |
| inline-list-output-types | Inline list bodies in main.rs become typed list envelopes | task | Six inline list bodies emit typed list envelopes, and graph rdeps declares its schema. | list-envelope-coverage | REQ-04, D-04 | touches 3 | — | — |
| command-list-output-types | Command-layer list results become typed list envelopes | task | Command-layer list results for nine commands are list envelopes declared as their output schemas. | list-envelope-coverage | REQ-04, D-04 | touches 7 | — | inline-list-output-types |
| list-envelope-coverage-test | One test proves the list-envelope count for each list-emitting command | task | A single schema-driven test checks count against collection length for each list-emitting command. | list-envelope-coverage, cli-process-fixture | REQ-04, D-04 | touches 2 | — | cli-process-fixture, command-list-output-types |
| mcp-parity-tool-failure | MCP parity test fails when node is missing | task | The MCP parity test fails rather than skips when node is missing. | script-self-test-gates | REQ-10 | touches 1 | — | cli-validate-json-ids |
| server-shutdown-test-bound | Server shutdown force-close test stays under the per-test bound | task | Drain entitlement is proven with a short deadline, and no server test waits the real five-second drain. | suite-measurement-protocol | REQ-13, D-01 | touches 2 | — | — |
| shared-taxonomy-repo | Shared taxonomy repository type for CLI tests | task | Shared test support defines the taxonomy-initialised CLI test repository and its label helpers. | — | REQ-06, D-02 | touches 1 | — | — |
| shared-fixture-issue | Shared issue-construction fixture | task | crate::test_utils defines the one issue-construction fixture. | — | REQ-06, D-02 | touches 1 | — | — |
| fast-issue-fixture-issue-adoption | fast_issue uses the shared issue fixture | task | The fast_issue suite calls the shared issue fixture and defines no copy. | — | REQ-06, D-02 | touches 1 | — | shared-fixture-issue |
| fast-docs-templates-fixture-issue-adoption | fast_docs_templates uses the shared issue fixture | task | The fast_docs_templates suite calls the shared issue fixture and defines no copy. | — | REQ-06, D-02 | touches 1 | — | shared-fixture-issue |
| fast-rules-fixture-issue-adoption | fast_rules uses the shared issue fixture | task | The fast_rules suite calls the shared issue fixture and defines no copy. | — | REQ-06, D-02 | touches 1 | — | shared-fixture-issue |
| cli-query-graph-fixture-issue-adoption | cli_query_graph uses the shared issue fixture | task | The cli_query_graph suite calls the shared issue fixture and defines no copy. | — | REQ-06, D-02 | touches 1 | — | shared-fixture-issue |
| store-generic-initialisation | Product initialisation runs over any repository-state store | task | Product initialisation runs over any repository-state store with unchanged file-backed behaviour. | — | REQ-06, D-02 | touches 1 | — | — |
| shared-memory-executor | Memory executor initialised through product initialisation | task | The shared memory executor is set up by product initialisation, and TestHarness builds on it. | — | REQ-06, D-02 | touches 2, uncertain | — | store-generic-initialisation |
| fast-rules-memory-executor-adoption | fast_rules uses the shared memory executor | task | The fast_rules suite uses the shared memory executor and defines no copy. | — | REQ-06, D-02 | touches 6 | — | shared-memory-executor |
| shipped-item-kinds-derivation | Shared test support derives shipped item kinds from the default package | task | Shared test support derives shipped item-kind declarations from the default profile package. | — | REQ-07, D-02 | touches 1 | — | — |
| harness-shipped-item-kinds | Harness and test taxonomy take shipped item kinds from the derivation | task | TestHarness and test_taxonomy take shipped item kinds from the package derivation. | shipped-item-kinds-fixture | REQ-07, D-02 | touches 3 | — | shipped-item-kinds-derivation |
| unit-test-shipped-item-kinds | Command unit tests take shipped item kinds from the default package | task | Item, validate and domain item unit tests read shipped item kinds from the shared derivation. | shipped-item-kinds-fixture | REQ-07 | touches 3 | — | shipped-item-kinds-derivation |
| cli-item-validate-shipped-item-kinds | Decision-kind CLI tests take shipped item kinds from the default package | task | Decision-kind CLI tests read shipped item kinds from the shared derivation. | shipped-item-kinds-fixture | REQ-07 | touches 2 | — | shipped-item-kinds-derivation, cli-item-kind-spawn-migration |
| fast-rules-shipped-item-kinds | Projection-render tests take shipped item kinds from the default package | task | Projection-render harness tests read shipped item kinds from the shared derivation. | shipped-item-kinds-fixture | REQ-07 | touches 1 | — | shipped-item-kinds-derivation, fast-rules-memory-setup-adoption |
| server-git-fixture-adoption | Server unit tests initialise git through the shared helper | task | Server route tests initialise git repositories through the shared helper. | cli-process-fixture | REQ-05 | touches 1 | — | cli-process-fixture |
| fast-docs-templates-git-fixture-adoption | Git revision tests initialise git through the shared helper | task | Git-revision suite tests initialise git repositories through the shared helper. | cli-process-fixture | REQ-05, D-02 | touches 1 | — | cli-process-fixture |
| file-backed-setup-constructors | Shared constructors for file-backed in-process test repositories | task | Shared test support constructs file-backed repositories, executors and declaration directories. | — | REQ-05, D-02 | touches 1 | — | — |
| fast-rules-file-setup-adoption | fast_rules file-backed tests use the shared constructors | task | fast_rules file-backed tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 4 | — | file-backed-setup-constructors, rule-warning-tests-down-a-layer |
| fast-docs-templates-file-setup-adoption | fast_docs_templates file-backed tests use the shared constructors | task | fast_docs_templates file-backed tests use the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 3 | — | file-backed-setup-constructors |
| memory-declaration-seeding | Shared memory executor accepts declarations as data | task | Shared test support builds memory stores and executors from declarations passed as data. | — | REQ-05, REQ-06, D-02 | touches 1 | — | shared-memory-executor |
| fast-issue-memory-setup-adoption | fast_issue memory-backed tests use the shared seeding | task | fast_issue memory-backed tests build stores and executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 5 | — | memory-declaration-seeding |
| fast-rules-memory-setup-adoption | fast_rules memory-backed tests use the shared seeding | task | fast_rules memory-backed tests build stores and executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 5 | — | memory-declaration-seeding, fast-rules-memory-executor-adoption |
| fast-docs-templates-memory-setup-adoption | fast_docs_templates memory-backed tests use the shared seeding | task | fast_docs_templates memory-backed tests build stores and executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | memory-declaration-seeding |
| exit-code-issue-prefix-case-moves | Issue-show prefix cases move beside their owner | task | The issue-show prefix cases live in the status projection tests with their non-exit assertions. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 1 | — | exit-code-binding-completeness, cli-issue-show-json-ids |
| exit-code-dependency-guard-case-moves | Dependency and query argument-guard cases move beside their owners | task | Dependency and query argument-guard cases live beside the tests of those commands. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 2 | — | exit-code-binding-completeness, cli-graph-json-ids, cli-label-query-json-ids |
| exit-code-gate-preset-case-move | Gate preset registry case moves beside its owner | task | The gate preset registry case lives in the gate preset tests with its message assertion. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 1 | — | exit-code-binding-completeness, cli-apply-template-spawn-migration |
| cli-issue-list-envelope-reassertion-removal | cli_issue tests stop re-asserting the list-envelope count | task | No cli_issue test re-asserts count against collection length. | list-envelope-coverage | REQ-04 | touches 2 | — | list-envelope-coverage-test, cli-issue-update-spawn-migration, cli-issue-show-json-ids |
| cli-query-graph-list-envelope-reassertion-removal | cli_query_graph tests stop re-asserting the list-envelope count | task | No cli_query_graph test re-asserts count against collection length. | list-envelope-coverage | REQ-04 | touches 1 | — | list-envelope-coverage-test, cli-links-search-spawn-migration |
| cli-repo-workflow-list-envelope-reassertion-removal | cli_repo_workflow tests stop re-asserting the list-envelope count | task | No cli_repo_workflow test re-asserts count against collection length. | list-envelope-coverage | REQ-04 | touches 3 | — | list-envelope-coverage-test, cli-serve-artifact-spawn-migration, cli-profile-json-ids, cli-claim-worktree-spawn-migration |
| query-domain-owner-gaps | Domain query tests own each property the query re-proofs assert | task | domain/queries.rs owns each query property the CLI and executor re-proofs assert. | domain-query-ownership | REQ-02 | touches 1 | — | query-available-domain-order |
| query-cli-wiring-reduction | Query CLI tests assert one wiring case per subcommand and flag | task | query_tests.rs keeps one wiring case per query subcommand or flag on the shared CLI process fixture. | domain-query-ownership, cli-process-fixture | REQ-02, REQ-05 | touches 2 | — | query-domain-owner-gaps, cli-process-fixture, shared-taxonomy-repo |
| label-query-cli-wiring-reduction | Label and filter CLI tests assert one wiring case per subcommand and flag | task | Label and filter CLI tests keep one wiring case per subcommand or flag, and the domain owns label semantics. | domain-query-ownership | REQ-02 | touches 4 | — | query-domain-owner-gaps, exit-code-dependency-guard-case-moves |
| label-query-harness-reduction | Label and strategic executor tests keep one wiring case each | task | Executor label and strategic tests keep one wiring case per entry point, and the domain owns the semantics. | domain-query-ownership | REQ-02 | touches 2 | — | query-domain-owner-gaps, fast-rules-memory-setup-adoption |
| auto-transition-wiring-reduction | Workflow CLI tests keep one auto-transition wiring case | task | One CLI case shows completing a dependency readies its dependent; the domain owns the rules. | domain-query-ownership | REQ-02 | touches 1 | — | auto-transition-domain-decision, cli-workflow-json-ids |
| auto-transition-harness-owner | Lifecycle harness tests stop re-proving the auto-transition cascade | task | Lifecycle harness tests assert timestamps only, and one harness test owns the cascade wiring. | domain-query-ownership | REQ-02 | touches 1 | — | auto-transition-domain-decision |
| batch-cycle-adapter-reproof-removal | Batch-create stops re-proving keyed cycle detection | task | Batch-create keeps at most one adapter wiring test, and graph tests own keyed cycle detection. | domain-query-ownership | REQ-02 | touches 1 | — | — |
| cli-cycle-reproof-removal | Workflow CLI tests stop re-proving dependency cycle rejection | task | integration_test.rs holds no dependency cycle re-proof. | domain-query-ownership | REQ-02 | touches 1 | — | cli-workflow-json-ids |
| docs-check-selftest-gate | Documentation checker self-test runs as a declared gate | task | The documentation checker self-test runs as its own gate over the checkers. | script-self-test-gates | REQ-10 | touches 2, uncertain | — | — |
| shipped-policy-regions-selftest-gate | Shipped-policy region generator self-test runs as a declared gate | task | The shipped-policy region generator self-test runs as its own gate over the generator. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| workflow-contract-selftest-gate | Workflow-contract harness self-test runs as a declared gate | task | The workflow-contract self-test runs as its own gate over the harness and its vectors. | script-self-test-gates | REQ-10 | touches 2, uncertain | — | — |
| session-cost-benchmark-selftest-gate | Session-cost benchmark self-test runs as a declared gate | task | The session-cost benchmark self-test runs as its own gate over the benchmark script. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| standards-scan-test-gate | Standards-scan test suite runs as a declared gate | task | The standards-scan test suite runs as its own gate over the scanner. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| standards-fix-test-gate | Standards-fix test suite runs as a declared gate | task | The standards-fix test suite runs as its own gate over the fixer. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| manifest-tool-test-gate | Breakdown-manifest tool test suite runs as a declared gate | task | The breakdown-manifest tool test suite runs as its own gate over the tool and its fixtures. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| lock-tests-deterministic-precondition | Read-only lock tests establish their precondition on any host | task | The four read-only lock tests run their assertions on any host and never pass by skipping. | — | REQ-10 | touches 1 | — | — |
| library-item-validate-memory-setup-adoption | Item and validate unit tests use the shared memory seeding | task | Item and validate unit tests build memory executors through the shared seeding, with no literal index. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | memory-declaration-seeding, harness-demo-case-moves |
| server-memory-setup-adoption | Server tests use the shared memory seeding | task | Server tests build memory stores through the shared seeding, with no literal index. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | memory-declaration-seeding, server-git-fixture-adoption |
| server-file-setup-adoption | Server tests use the shared file-backed constructors | task | Server tests build file-backed repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 3 | — | file-backed-setup-constructors, server-memory-setup-adoption, server-shutdown-test-bound |
| cli-item-validate-in-process-setup-adoption | cli_item_validate in-process tests use the shared constructors | task | The cli_item_validate in-process tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, REQ-06, D-02 | touches 3 | — | file-backed-setup-constructors, cli-item-validate-shipped-item-kinds |
| cli-query-graph-in-process-setup-adoption | cli_query_graph in-process tests use the shared constructors | task | The cli_query_graph in-process tests build repositories and executors through the shared constructors. | file-setup-constructors, memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | file-backed-setup-constructors, memory-declaration-seeding, cli-label-query-spawn-migration, cli-links-search-spawn-migration |
| cli-repo-workflow-in-process-setup-adoption | cli_repo_workflow in-process tests use the shared constructors | task | The cli_repo_workflow in-process tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, REQ-06, D-02 | touches 1 | — | file-backed-setup-constructors, cli-profile-spawn-migration |
| exit-code-envelope-ownership | Exit codes and list envelopes each have one owning test | story | command exit codes and the list-envelope count each have one owning test | — | REQ-01, REQ-04 | — | — | exit-code-reproof-deletion, cli-issue-list-envelope-reassertion-removal, cli-query-graph-list-envelope-reassertion-removal, cli-repo-workflow-list-envelope-reassertion-removal, exit-code-reassertion-gate-evaluate, exit-code-reassertion-gate-status, exit-code-reassertion-issue-create, exit-code-reassertion-issue-show, exit-code-reassertion-issue-errors, exit-code-reassertion-cli-query-graph, exit-code-reassertion-cli-item-validate, exit-code-reassertion-profile, exit-code-reassertion-checkout, exit-code-reassertion-workflow |
| query-graph-ownership | Query, transition and cycle semantics are proven at the domain and graph layers | story | query filters, auto-transitions and cycle detection are owned by domain and graph tests, with CLI and harness tests reduced to wiring | — | REQ-02 | — | — | query-cli-wiring-reduction, auto-transition-wiring-reduction, auto-transition-harness-owner, batch-cycle-adapter-reproof-removal, cli-cycle-reproof-removal, harness-demo-deletion, label-query-cli-wiring-reduction, label-query-harness-reduction |
| storage-contracts | Storage behaviour is proven once per contract | story | finalization is proven in repository state and every IssueStore backend runs one conformance suite | — | REQ-03, REQ-08 | — | — | backend-specific-test-pruning |
| shared-test-fixtures | Tests run on shared fixtures and guards | story | CLI and in-process tests use one definition of each fixture and reach process state only through the shared guards | — | REQ-05, REQ-06, REQ-07, REQ-09, D-02 | — | — | cli-gate-definition-json-ids, cli-gate-status-json-ids, cli-issue-show-json-ids, cli-issue-update-json-ids, cli-issue-failure-corpus-spawn-migration, cli-issue-error-output-json-ids, cli-graph-json-ids, cli-label-query-json-ids, cli-init-discovery-spawn-migration, cli-document-history-spawn-migration, cli-worktree-json-ids, cli-apply-template-spawn-migration, cli-profile-json-ids, cli-config-spawn-migration, cli-serve-artifact-spawn-migration, cli-workflow-json-ids, library-command-git-fixture-adoption, library-storage-git-fixture-adoption, fast-docs-templates-git-fixture-adoption, library-fixture-issue-deletion, fast-issue-fixture-issue-adoption, fast-docs-templates-fixture-issue-adoption, fast-rules-fixture-issue-adoption, fast-rules-shipped-item-kinds, fast-rules-file-setup-adoption, fast-docs-templates-file-setup-adoption, fast-issue-memory-setup-adoption, fast-docs-templates-memory-setup-adoption, env-reading-tests-guarded, library-item-validate-memory-setup-adoption, library-archive-file-setup-adoption, library-gate-claim-file-setup-adoption, library-init-profile-file-setup-adoption, server-file-setup-adoption, cli-item-validate-in-process-setup-adoption, cli-query-graph-in-process-setup-adoption, cli-repo-workflow-in-process-setup-adoption |
| suite-cost | The default suite runs within its time and footprint budgets | story | the default suite runs no nested cargo build or tool-skipping test, fixtures are keyed on inputs, and footprint and clock are reported | — | REQ-10, REQ-11, REQ-12, REQ-13, D-01 | — | — | suite-clock-report |
| cli-gate-evaluate-spawn-migration | gate evaluation tests spawn jit through the shared process fixture | task | The gate evaluation tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-gate-status-spawn-migration | gate status and key tests spawn jit through the shared process fixture | task | The gate status and key tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-gate-definition-spawn-migration | gate definition tests spawn jit through the shared process fixture | task | The gate definition tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| cli-gate-definition-json-ids | gate definition and evaluation tests read created ids from JSON output | task | The gate definition and evaluation tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 3 | — | cli-gate-evaluate-spawn-migration, cli-gate-definition-spawn-migration |
| cli-gate-status-json-ids | gate status and key tests read created ids from JSON output | task | The gate status and key tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 5 | — | cli-gate-status-spawn-migration |
| cli-issue-create-spawn-migration | issue create and batch tests spawn jit through the shared process fixture | task | The issue create and batch tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, REQ-06, D-02 | touches 6 | — | cli-process-fixture, shared-taxonomy-repo |
| cli-issue-show-spawn-migration | issue show and status tests spawn jit through the shared process fixture | task | The issue show and status tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, REQ-06, D-02 | touches 6 | — | cli-process-fixture, shared-taxonomy-repo |
| cli-issue-update-spawn-migration | issue search and update tests spawn jit through the shared process fixture | task | The issue search and update tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, REQ-06, D-02 | touches 6 | — | cli-process-fixture, shared-taxonomy-repo |
| cli-issue-show-json-ids | issue show and rollup tests read created ids from JSON output | task | The issue show and rollup tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 5 | — | cli-issue-show-spawn-migration |
| cli-issue-update-json-ids | issue mutation tests read created ids from JSON output | task | The issue mutation tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 5 | — | cli-issue-create-spawn-migration, cli-issue-update-spawn-migration |
| cli-issue-failure-corpus-spawn-migration | failure-envelope contract tests spawn jit through the shared process fixture | task | The failure-envelope contract tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-issue-error-output-spawn-migration | error-output tests spawn jit through the shared process fixture | task | The error-output tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-issue-output-mode-spawn-migration | output-mode and export tests spawn jit through the shared process fixture | task | The output-mode and export tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 3 | — | cli-process-fixture |
| cli-issue-error-output-json-ids | error-output and export tests read created ids from JSON output | task | The error-output and export tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 2 | — | cli-issue-error-output-spawn-migration, cli-issue-output-mode-spawn-migration |
| cli-item-kind-spawn-migration | item-kind tests spawn jit through the shared process fixture | task | The item-kind tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-validate-spawn-migration | validate tests spawn jit through the shared process fixture | task | The validate tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-registry-render-spawn-migration | registry and render tests spawn jit through the shared process fixture | task | The registry and render tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| cli-validate-json-ids | validate tests read created ids from JSON output | task | The validate tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 2 | — | cli-validate-spawn-migration |
| cli-graph-spawn-migration | graph and dependency tests spawn jit through the shared process fixture | task | The graph and dependency tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, REQ-06, D-02 | touches 6 | — | cli-process-fixture, shared-taxonomy-repo, cli-query-graph-fixture-issue-adoption |
| cli-label-query-spawn-migration | label and query tests spawn jit through the shared process fixture | task | The label and query tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, REQ-06, D-02 | touches 6 | — | cli-process-fixture, shared-taxonomy-repo, cli-query-graph-fixture-issue-adoption |
| cli-links-search-spawn-migration | link, scope and search tests spawn jit through the shared process fixture | task | The link, scope and search tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, REQ-06, D-02 | touches 4 | — | cli-process-fixture, shared-taxonomy-repo, cli-query-graph-fixture-issue-adoption |
| cli-graph-json-ids | graph and dependency tests read created ids from JSON output | task | The graph and dependency tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-graph-spawn-migration |
| cli-label-query-json-ids | label and query tests read created ids from JSON output | task | The label and query tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 3 | — | cli-label-query-spawn-migration |
| cli-claim-worktree-spawn-migration | claim and worktree tests spawn jit through the shared process fixture | task | The claim and worktree tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-init-discovery-spawn-migration | init and discovery tests spawn jit through the shared process fixture | task | The init and discovery tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 3 | — | cli-process-fixture |
| cli-document-history-spawn-migration | document and archive tests spawn jit through the shared process fixture | task | The document and archive tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| cli-worktree-json-ids | cross-worktree tests read created ids from JSON output | task | The cross-worktree tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 1 | — | cli-claim-worktree-spawn-migration |
| cli-profile-spawn-migration | profile tests spawn jit through the shared process fixture | task | The profile tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 5 | — | cli-process-fixture |
| cli-apply-template-spawn-migration | apply, preset and template tests spawn jit through the shared process fixture | task | The apply, preset and template tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| cli-profile-json-ids | profile tests read created ids from JSON output | task | The profile tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 1 | — | cli-profile-spawn-migration |
| cli-config-spawn-migration | configuration and CLI-surface tests spawn jit through the shared process fixture | task | The configuration and CLI-surface tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-workflow-integration-spawn-migration | workflow and integration tests spawn jit through the shared process fixture | task | The workflow and integration tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-process-fixture |
| cli-serve-artifact-spawn-migration | serve and artifact tests spawn jit through the shared process fixture | task | The serve and artifact tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| cli-workflow-json-ids | workflow and integration tests read created ids from JSON output | task | The workflow and integration tests read created ids and values only from --json output. | cli-process-fixture | REQ-05, D-02 | touches 3 | — | cli-workflow-integration-spawn-migration |
| fixture-issue-commands-a-adoption | Archive, claim, dependency, document and gate command unit tests use the shared issue fixture | task | The archive, claim, dependency, document and gate command unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 5 | — | shared-fixture-issue |
| fixture-issue-commands-b-adoption | Gate-check, issue, item, executor and snapshot command unit tests use the shared issue fixture | task | The gate-check, issue, item, executor and snapshot command unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 5 | — | shared-fixture-issue |
| fixture-issue-commands-c-adoption | Template, validate and worktree command unit tests use the shared issue fixture | task | The template, validate and worktree command unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 4 | — | shared-fixture-issue |
| fixture-issue-domain-artifact-adoption | Domain artifact and projection unit tests use the shared issue fixture | task | The domain artifact and projection unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 5 | — | shared-fixture-issue |
| fixture-issue-domain-query-adoption | Domain query, item and output unit tests use the shared issue fixture | task | The domain query, item and output unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 5 | — | shared-fixture-issue |
| fixture-issue-graph-validation-adoption | Graph, validation and declaration unit tests use the shared issue fixture | task | The graph, validation and declaration unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 6 | — | shared-fixture-issue |
| fixture-issue-storage-state-adoption | Storage and repository-state unit tests use the shared issue fixture | task | The storage and repository-state unit tests call the shared issue fixture. | — | REQ-06, D-02 | touches 6 | — | shared-fixture-issue |
| library-fixture-issue-deletion | The library's own issue-construction fixture is deleted | task | domain/types.rs defines no issue-construction fixture. | — | REQ-06, D-02 | touches 1 | — | fixture-issue-commands-a-adoption, fixture-issue-commands-b-adoption, fixture-issue-commands-c-adoption, fixture-issue-domain-artifact-adoption, fixture-issue-domain-query-adoption, fixture-issue-graph-validation-adoption, fixture-issue-storage-state-adoption |
| library-batch-memory-setup-adoption | Batch-command tests use the shared memory seeding | task | The batch, bulk and event command tests obtain memory executors from the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 3 | — | memory-declaration-seeding, harness-demo-case-moves, batch-cycle-adapter-reproof-removal, cli-process-fixture |
| library-gate-memory-setup-adoption | Gate command tests use the shared memory seeding | task | Gate, gate-check and gate CLI command tests build memory executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 3 | — | memory-declaration-seeding, harness-demo-case-moves, batch-cycle-adapter-reproof-removal, cli-process-fixture |
| library-issue-memory-setup-adoption | Issue, invariant and executor tests use the shared memory seeding | task | Issue, invariant and executor-core tests build memory executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 3 | — | memory-declaration-seeding, harness-demo-case-moves, batch-cycle-adapter-reproof-removal, library-command-git-fixture-adoption |
| library-archive-file-setup-adoption | Archive command tests use the shared file-backed constructors | task | Archive command tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 1 | — | file-backed-setup-constructors, memory-declaration-seeding |
| library-gate-claim-file-setup-adoption | Gate, claim and hook tests use the shared file-backed constructors | task | Gate-check, claim and hook tests build file-backed repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 3 | — | file-backed-setup-constructors, library-gate-memory-setup-adoption |
| library-init-profile-file-setup-adoption | Init, profile and validation tests use the shared file-backed constructors | task | Init, profile, gate-run, validation and main tests build file-backed repositories through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 5 | — | file-backed-setup-constructors, memory-declaration-seeding |
| library-command-git-fixture-adoption | Command unit tests initialise git through the shared helper | task | Command unit-test modules initialise git repositories through the shared helper. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| library-storage-git-fixture-adoption | Storage and gate-execution tests initialise git through the shared helper | task | Storage, gate-execution and profile-package tests initialise git repositories through the shared helper. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| exit-code-reassertion-gate-evaluate | Gate evaluation tests stop re-asserting projected exit codes | task | The gate evaluation tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-binding-completeness, cli-gate-evaluate-spawn-migration |
| exit-code-reassertion-gate-status | Gate status and definition tests stop re-asserting projected exit codes | task | The gate status and definition tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 5 | — | exit-code-binding-completeness, cli-gate-definition-json-ids, cli-gate-status-json-ids |
| exit-code-reassertion-issue-create | Issue-creation tests stop re-asserting projected exit codes | task | The issue-creation, search, rollup and export tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 5 | — | exit-code-envelope-case-moves, exit-code-issue-prefix-case-moves, cli-issue-error-output-json-ids |
| exit-code-reassertion-issue-show | Issue show and failure-envelope tests stop re-asserting projected exit codes | task | Issue show, status and top-level failure tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-envelope-case-moves, exit-code-issue-prefix-case-moves, cli-issue-error-output-spawn-migration |
| exit-code-reassertion-issue-errors | Error-output tests stop re-asserting projected exit codes | task | Error-output tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-envelope-case-moves, exit-code-issue-prefix-case-moves, cli-issue-error-output-json-ids |
| exit-code-reassertion-profile | Profile tests stop re-asserting projected exit codes | task | Profile tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 3 | — | exit-code-claim-delete-case-moves, exit-code-gate-preset-case-move |
| exit-code-reassertion-checkout | Claim, lock and discovery tests stop re-asserting projected exit codes | task | Claim, lock, discovery and worktree tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-claim-delete-case-moves, exit-code-gate-preset-case-move |
| exit-code-reassertion-workflow | Configuration and workflow tests stop re-asserting projected exit codes | task | Configuration, serve and workflow tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 6 | — | exit-code-claim-delete-case-moves, exit-code-gate-preset-case-move |

```mermaid
flowchart LR
    N0["cli-process-fixture: Shared fixture for spawning jit in CLI tests"]
    N1["env-reading-tests-guarded: Tests read shared process environment only under the environment guard"]
    N2["exit-code-binding-completeness: Exit-code projection suite proves a binding behind each row"]
    N3["exit-code-reproof-deletion: Exit-code re-proof suites are deleted"]
    N4["query-available-domain-order: Available-work filtering and priority order become a pure domain query"]
    N5["auto-transition-domain-decision: Auto-transition readiness is decided by a pure domain function"]
    N6["harness-demo-case-moves: Unique harness demonstration cases move beside their owners"]
    N7["harness-demo-deletion: The harness demonstration suite is deleted"]
    N8["finalization-owner-gaps: Repository-state tests own each finalization case"]
    N9["finalization-reproof-deletion: Storage tests stop re-proving mutation finalization"]
    N10["issue-store-conformance-suite: Shared IssueStore conformance suite runs against both backends"]
    N11["memory-backend-path-reads: In-memory path reads serve the store's own commit-pinned state"]
    N12["backend-specific-test-pruning: Backend tests keep only backend-specific behaviour"]
    N13["profiled-fixture-input-key: Profiled-repository fixture keyed only on its inputs"]
    N14["fixture-setup-hook-removal: Fixture setup runs inside the suite with no runner hook"]
    N15["failure-corpus-in-suite: Recorded-failure corpus built in the suite under an input key"]
    N16["rule-warning-tests-down-a-layer: Create-time rule warnings proven in-process without a profiled repository"]
    N17["doctest-substep-bound: Doctest substep fits within the per-test bound"]
    N18["cargo-ci-selftest-gate: cargo-ci self-test runs as a declared gate"]
    N19["profile-suite-selftest-gate: Suite-profiler self-test runs as a declared gate"]
    N20["build-budget-selftest-gate: Build-budget checker self-test runs as a declared gate"]
    N21["dependency-policy-tool-failure: Dependency-feature policy tests fail when cargo is missing"]
    N22["build-footprint-report: Build-footprint figures before and after the epic are reported"]
    N23["suite-clock-report: Suite clock before and after the epic is reported"]
    N24["exit-code-envelope-case-moves: Blocked-transition and startup envelope cases move beside their owners"]
    N25["exit-code-claim-delete-case-moves: Claim and confirmed-delete cases move beside their owners"]
    N26["exit-code-reassertion-cli-query-graph: cli_query_graph tests stop re-asserting projected exit codes"]
    N27["exit-code-reassertion-cli-item-validate: cli_item_validate tests stop re-asserting projected exit codes"]
    N28["inline-list-output-types: Inline list bodies in main.rs become typed list envelopes"]
    N29["command-list-output-types: Command-layer list results become typed list envelopes"]
    N30["list-envelope-coverage-test: One test proves the list-envelope count for each list-emitting command"]
    N31["mcp-parity-tool-failure: MCP parity test fails when node is missing"]
    N32["server-shutdown-test-bound: Server shutdown force-close test stays under the per-test bound"]
    N33["shared-taxonomy-repo: Shared taxonomy repository type for CLI tests"]
    N34["shared-fixture-issue: Shared issue-construction fixture"]
    N35["fast-issue-fixture-issue-adoption: fast_issue uses the shared issue fixture"]
    N36["fast-docs-templates-fixture-issue-adoption: fast_docs_templates uses the shared issue fixture"]
    N37["fast-rules-fixture-issue-adoption: fast_rules uses the shared issue fixture"]
    N38["cli-query-graph-fixture-issue-adoption: cli_query_graph uses the shared issue fixture"]
    N39["store-generic-initialisation: Product initialisation runs over any repository-state store"]
    N40["shared-memory-executor: Memory executor initialised through product initialisation"]
    N41["fast-rules-memory-executor-adoption: fast_rules uses the shared memory executor"]
    N42["shipped-item-kinds-derivation: Shared test support derives shipped item kinds from the default package"]
    N43["harness-shipped-item-kinds: Harness and test taxonomy take shipped item kinds from the derivation"]
    N44["unit-test-shipped-item-kinds: Command unit tests take shipped item kinds from the default package"]
    N45["cli-item-validate-shipped-item-kinds: Decision-kind CLI tests take shipped item kinds from the default package"]
    N46["fast-rules-shipped-item-kinds: Projection-render tests take shipped item kinds from the default package"]
    N47["server-git-fixture-adoption: Server unit tests initialise git through the shared helper"]
    N48["fast-docs-templates-git-fixture-adoption: Git revision tests initialise git through the shared helper"]
    N49["file-backed-setup-constructors: Shared constructors for file-backed in-process test repositories"]
    N50["fast-rules-file-setup-adoption: fast_rules file-backed tests use the shared constructors"]
    N51["fast-docs-templates-file-setup-adoption: fast_docs_templates file-backed tests use the shared constructors"]
    N52["memory-declaration-seeding: Shared memory executor accepts declarations as data"]
    N53["fast-issue-memory-setup-adoption: fast_issue memory-backed tests use the shared seeding"]
    N54["fast-rules-memory-setup-adoption: fast_rules memory-backed tests use the shared seeding"]
    N55["fast-docs-templates-memory-setup-adoption: fast_docs_templates memory-backed tests use the shared seeding"]
    N56["exit-code-issue-prefix-case-moves: Issue-show prefix cases move beside their owner"]
    N57["exit-code-dependency-guard-case-moves: Dependency and query argument-guard cases move beside their owners"]
    N58["exit-code-gate-preset-case-move: Gate preset registry case moves beside its owner"]
    N59["cli-issue-list-envelope-reassertion-removal: cli_issue tests stop re-asserting the list-envelope count"]
    N60["cli-query-graph-list-envelope-reassertion-removal: cli_query_graph tests stop re-asserting the list-envelope count"]
    N61["cli-repo-workflow-list-envelope-reassertion-removal: cli_repo_workflow tests stop re-asserting the list-envelope count"]
    N62["query-domain-owner-gaps: Domain query tests own each property the query re-proofs assert"]
    N63["query-cli-wiring-reduction: Query CLI tests assert one wiring case per subcommand and flag"]
    N64["label-query-cli-wiring-reduction: Label and filter CLI tests assert one wiring case per subcommand and flag"]
    N65["label-query-harness-reduction: Label and strategic executor tests keep one wiring case each"]
    N66["auto-transition-wiring-reduction: Workflow CLI tests keep one auto-transition wiring case"]
    N67["auto-transition-harness-owner: Lifecycle harness tests stop re-proving the auto-transition cascade"]
    N68["batch-cycle-adapter-reproof-removal: Batch-create stops re-proving keyed cycle detection"]
    N69["cli-cycle-reproof-removal: Workflow CLI tests stop re-proving dependency cycle rejection"]
    N70["docs-check-selftest-gate: Documentation checker self-test runs as a declared gate"]
    N71["shipped-policy-regions-selftest-gate: Shipped-policy region generator self-test runs as a declared gate"]
    N72["workflow-contract-selftest-gate: Workflow-contract harness self-test runs as a declared gate"]
    N73["session-cost-benchmark-selftest-gate: Session-cost benchmark self-test runs as a declared gate"]
    N74["standards-scan-test-gate: Standards-scan test suite runs as a declared gate"]
    N75["standards-fix-test-gate: Standards-fix test suite runs as a declared gate"]
    N76["manifest-tool-test-gate: Breakdown-manifest tool test suite runs as a declared gate"]
    N77["lock-tests-deterministic-precondition: Read-only lock tests establish their precondition on any host"]
    N78["library-item-validate-memory-setup-adoption: Item and validate unit tests use the shared memory seeding"]
    N79["server-memory-setup-adoption: Server tests use the shared memory seeding"]
    N80["server-file-setup-adoption: Server tests use the shared file-backed constructors"]
    N81["cli-item-validate-in-process-setup-adoption: cli_item_validate in-process tests use the shared constructors"]
    N82["cli-query-graph-in-process-setup-adoption: cli_query_graph in-process tests use the shared constructors"]
    N83["cli-repo-workflow-in-process-setup-adoption: cli_repo_workflow in-process tests use the shared constructors"]
    N84["exit-code-envelope-ownership: Exit codes and list envelopes each have one owning test"]
    N85["query-graph-ownership: Query, transition and cycle semantics are proven at the domain and graph layers"]
    N86["storage-contracts: Storage behaviour is proven once per contract"]
    N87["shared-test-fixtures: Tests run on shared fixtures and guards"]
    N88["suite-cost: The default suite runs within its time and footprint budgets"]
    N89["cli-gate-evaluate-spawn-migration: gate evaluation tests spawn jit through the shared process fixture"]
    N90["cli-gate-status-spawn-migration: gate status and key tests spawn jit through the shared process fixture"]
    N91["cli-gate-definition-spawn-migration: gate definition tests spawn jit through the shared process fixture"]
    N92["cli-gate-definition-json-ids: gate definition and evaluation tests read created ids from JSON output"]
    N93["cli-gate-status-json-ids: gate status and key tests read created ids from JSON output"]
    N94["cli-issue-create-spawn-migration: issue create and batch tests spawn jit through the shared process fixture"]
    N95["cli-issue-show-spawn-migration: issue show and status tests spawn jit through the shared process fixture"]
    N96["cli-issue-update-spawn-migration: issue search and update tests spawn jit through the shared process fixture"]
    N97["cli-issue-show-json-ids: issue show and rollup tests read created ids from JSON output"]
    N98["cli-issue-update-json-ids: issue mutation tests read created ids from JSON output"]
    N99["cli-issue-failure-corpus-spawn-migration: failure-envelope contract tests spawn jit through the shared process fixture"]
    N100["cli-issue-error-output-spawn-migration: error-output tests spawn jit through the shared process fixture"]
    N101["cli-issue-output-mode-spawn-migration: output-mode and export tests spawn jit through the shared process fixture"]
    N102["cli-issue-error-output-json-ids: error-output and export tests read created ids from JSON output"]
    N103["cli-item-kind-spawn-migration: item-kind tests spawn jit through the shared process fixture"]
    N104["cli-validate-spawn-migration: validate tests spawn jit through the shared process fixture"]
    N105["cli-registry-render-spawn-migration: registry and render tests spawn jit through the shared process fixture"]
    N106["cli-validate-json-ids: validate tests read created ids from JSON output"]
    N107["cli-graph-spawn-migration: graph and dependency tests spawn jit through the shared process fixture"]
    N108["cli-label-query-spawn-migration: label and query tests spawn jit through the shared process fixture"]
    N109["cli-links-search-spawn-migration: link, scope and search tests spawn jit through the shared process fixture"]
    N110["cli-graph-json-ids: graph and dependency tests read created ids from JSON output"]
    N111["cli-label-query-json-ids: label and query tests read created ids from JSON output"]
    N112["cli-claim-worktree-spawn-migration: claim and worktree tests spawn jit through the shared process fixture"]
    N113["cli-init-discovery-spawn-migration: init and discovery tests spawn jit through the shared process fixture"]
    N114["cli-document-history-spawn-migration: document and archive tests spawn jit through the shared process fixture"]
    N115["cli-worktree-json-ids: cross-worktree tests read created ids from JSON output"]
    N116["cli-profile-spawn-migration: profile tests spawn jit through the shared process fixture"]
    N117["cli-apply-template-spawn-migration: apply, preset and template tests spawn jit through the shared process fixture"]
    N118["cli-profile-json-ids: profile tests read created ids from JSON output"]
    N119["cli-config-spawn-migration: configuration and CLI-surface tests spawn jit through the shared process fixture"]
    N120["cli-workflow-integration-spawn-migration: workflow and integration tests spawn jit through the shared process fixture"]
    N121["cli-serve-artifact-spawn-migration: serve and artifact tests spawn jit through the shared process fixture"]
    N122["cli-workflow-json-ids: workflow and integration tests read created ids from JSON output"]
    N123["fixture-issue-commands-a-adoption: Archive, claim, dependency, document and gate command unit tests use the shared issue fixture"]
    N124["fixture-issue-commands-b-adoption: Gate-check, issue, item, executor and snapshot command unit tests use the shared issue fixture"]
    N125["fixture-issue-commands-c-adoption: Template, validate and worktree command unit tests use the shared issue fixture"]
    N126["fixture-issue-domain-artifact-adoption: Domain artifact and projection unit tests use the shared issue fixture"]
    N127["fixture-issue-domain-query-adoption: Domain query, item and output unit tests use the shared issue fixture"]
    N128["fixture-issue-graph-validation-adoption: Graph, validation and declaration unit tests use the shared issue fixture"]
    N129["fixture-issue-storage-state-adoption: Storage and repository-state unit tests use the shared issue fixture"]
    N130["library-fixture-issue-deletion: The library's own issue-construction fixture is deleted"]
    N131["library-batch-memory-setup-adoption: Batch-command tests use the shared memory seeding"]
    N132["library-gate-memory-setup-adoption: Gate command tests use the shared memory seeding"]
    N133["library-issue-memory-setup-adoption: Issue, invariant and executor tests use the shared memory seeding"]
    N134["library-archive-file-setup-adoption: Archive command tests use the shared file-backed constructors"]
    N135["library-gate-claim-file-setup-adoption: Gate, claim and hook tests use the shared file-backed constructors"]
    N136["library-init-profile-file-setup-adoption: Init, profile and validation tests use the shared file-backed constructors"]
    N137["library-command-git-fixture-adoption: Command unit tests initialise git through the shared helper"]
    N138["library-storage-git-fixture-adoption: Storage and gate-execution tests initialise git through the shared helper"]
    N139["exit-code-reassertion-gate-evaluate: Gate evaluation tests stop re-asserting projected exit codes"]
    N140["exit-code-reassertion-gate-status: Gate status and definition tests stop re-asserting projected exit codes"]
    N141["exit-code-reassertion-issue-create: Issue-creation tests stop re-asserting projected exit codes"]
    N142["exit-code-reassertion-issue-show: Issue show and failure-envelope tests stop re-asserting projected exit codes"]
    N143["exit-code-reassertion-issue-errors: Error-output tests stop re-asserting projected exit codes"]
    N144["exit-code-reassertion-profile: Profile tests stop re-asserting projected exit codes"]
    N145["exit-code-reassertion-checkout: Claim, lock and discovery tests stop re-asserting projected exit codes"]
    N146["exit-code-reassertion-workflow: Configuration and workflow tests stop re-asserting projected exit codes"]
    N14 --> N1
    N99 --> N2
    N24 --> N3
    N25 --> N3
    N56 --> N3
    N57 --> N3
    N58 --> N3
    N4 --> N5
    N43 --> N6
    N40 --> N6
    N44 --> N6
    N6 --> N7
    N8 --> N9
    N9 --> N10
    N10 --> N11
    N11 --> N12
    N13 --> N14
    N14 --> N15
    N99 --> N15
    N84 --> N22
    N85 --> N22
    N86 --> N22
    N87 --> N22
    N15 --> N22
    N17 --> N22
    N18 --> N22
    N19 --> N22
    N20 --> N22
    N21 --> N22
    N31 --> N22
    N70 --> N22
    N71 --> N22
    N72 --> N22
    N73 --> N22
    N74 --> N22
    N75 --> N22
    N76 --> N22
    N77 --> N22
    N22 --> N23
    N2 --> N24
    N2 --> N25
    N112 --> N25
    N122 --> N25
    N64 --> N26
    N2 --> N27
    N105 --> N27
    N106 --> N27
    N28 --> N29
    N0 --> N30
    N29 --> N30
    N106 --> N31
    N34 --> N35
    N34 --> N36
    N34 --> N37
    N34 --> N38
    N39 --> N40
    N40 --> N41
    N42 --> N43
    N42 --> N44
    N42 --> N45
    N103 --> N45
    N42 --> N46
    N54 --> N46
    N0 --> N47
    N0 --> N48
    N49 --> N50
    N16 --> N50
    N49 --> N51
    N40 --> N52
    N52 --> N53
    N52 --> N54
    N41 --> N54
    N52 --> N55
    N2 --> N56
    N97 --> N56
    N2 --> N57
    N110 --> N57
    N111 --> N57
    N2 --> N58
    N117 --> N58
    N30 --> N59
    N96 --> N59
    N97 --> N59
    N30 --> N60
    N109 --> N60
    N30 --> N61
    N121 --> N61
    N118 --> N61
    N112 --> N61
    N4 --> N62
    N62 --> N63
    N0 --> N63
    N33 --> N63
    N62 --> N64
    N57 --> N64
    N62 --> N65
    N54 --> N65
    N5 --> N66
    N122 --> N66
    N5 --> N67
    N122 --> N69
    N52 --> N78
    N6 --> N78
    N52 --> N79
    N47 --> N79
    N49 --> N80
    N79 --> N80
    N32 --> N80
    N49 --> N81
    N45 --> N81
    N49 --> N82
    N52 --> N82
    N108 --> N82
    N109 --> N82
    N49 --> N83
    N116 --> N83
    N3 --> N84
    N59 --> N84
    N60 --> N84
    N61 --> N84
    N139 --> N84
    N140 --> N84
    N141 --> N84
    N142 --> N84
    N143 --> N84
    N26 --> N84
    N27 --> N84
    N144 --> N84
    N145 --> N84
    N146 --> N84
    N63 --> N85
    N66 --> N85
    N67 --> N85
    N68 --> N85
    N69 --> N85
    N7 --> N85
    N64 --> N85
    N65 --> N85
    N12 --> N86
    N92 --> N87
    N93 --> N87
    N97 --> N87
    N98 --> N87
    N99 --> N87
    N102 --> N87
    N110 --> N87
    N111 --> N87
    N113 --> N87
    N114 --> N87
    N115 --> N87
    N117 --> N87
    N118 --> N87
    N119 --> N87
    N121 --> N87
    N122 --> N87
    N137 --> N87
    N138 --> N87
    N48 --> N87
    N130 --> N87
    N35 --> N87
    N36 --> N87
    N37 --> N87
    N46 --> N87
    N50 --> N87
    N51 --> N87
    N53 --> N87
    N55 --> N87
    N1 --> N87
    N78 --> N87
    N134 --> N87
    N135 --> N87
    N136 --> N87
    N80 --> N87
    N81 --> N87
    N82 --> N87
    N83 --> N87
    N23 --> N88
    N0 --> N89
    N0 --> N90
    N0 --> N91
    N89 --> N92
    N91 --> N92
    N90 --> N93
    N0 --> N94
    N33 --> N94
    N0 --> N95
    N33 --> N95
    N0 --> N96
    N33 --> N96
    N95 --> N97
    N94 --> N98
    N96 --> N98
    N0 --> N99
    N0 --> N100
    N0 --> N101
    N100 --> N102
    N101 --> N102
    N0 --> N103
    N0 --> N104
    N0 --> N105
    N104 --> N106
    N0 --> N107
    N33 --> N107
    N38 --> N107
    N0 --> N108
    N33 --> N108
    N38 --> N108
    N0 --> N109
    N33 --> N109
    N38 --> N109
    N107 --> N110
    N108 --> N111
    N0 --> N112
    N0 --> N113
    N0 --> N114
    N112 --> N115
    N0 --> N116
    N0 --> N117
    N116 --> N118
    N0 --> N119
    N0 --> N120
    N0 --> N121
    N120 --> N122
    N34 --> N123
    N34 --> N124
    N34 --> N125
    N34 --> N126
    N34 --> N127
    N34 --> N128
    N34 --> N129
    N123 --> N130
    N124 --> N130
    N125 --> N130
    N126 --> N130
    N127 --> N130
    N128 --> N130
    N129 --> N130
    N52 --> N131
    N6 --> N131
    N68 --> N131
    N0 --> N131
    N52 --> N132
    N6 --> N132
    N68 --> N132
    N0 --> N132
    N52 --> N133
    N6 --> N133
    N68 --> N133
    N137 --> N133
    N49 --> N134
    N52 --> N134
    N49 --> N135
    N132 --> N135
    N49 --> N136
    N52 --> N136
    N0 --> N137
    N0 --> N138
    N2 --> N139
    N89 --> N139
    N2 --> N140
    N92 --> N140
    N93 --> N140
    N24 --> N141
    N56 --> N141
    N102 --> N141
    N24 --> N142
    N56 --> N142
    N100 --> N142
    N24 --> N143
    N56 --> N143
    N102 --> N143
    N25 --> N144
    N58 --> N144
    N25 --> N145
    N58 --> N145
    N25 --> N146
    N58 --> N146
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| D-01 REQ-13 scope (steward) | Chosen: the server shutdown test and the doctest substep are in scope. Early-relief terminals with no upstream edge are `profiled-fixture-input-key`, `rule-warning-tests-down-a-layer`, `server-shutdown-test-bound` and `doctest-substep-bound`. The before figures are the investigation's §A. |
| D-02 Fixtures first (steward D1) | Chosen: each fixture is a foundation, and the foundations are wave-1 roots with no edge: `cli-process-fixture`, `shared-taxonomy-repo`, `shared-fixture-issue`, `store-generic-initialisation`, `shipped-item-kinds-derivation` and `file-backed-setup-constructors`. `shared-memory-executor` and `memory-declaration-seeding` follow `store-generic-initialisation`, and adoptions follow their foundation, one per file family. Incoming edges are derived from footprint overlap: each 2195c98f retype task depends on every entry here that edits a file it retypes and reaches no external task, reduced transitively, including through the 2195c98f chain. A script checks that no such entry reaches 2195c98f, 842b0ac6 or df129d9e. Edges to add at breakdown: 0af95f14 → `command-list-output-types`, `doctest-substep-bound`, `env-reading-tests-guarded`, `fixture-issue-commands-b-adoption`, `fixture-issue-commands-c-adoption`, `fixture-issue-domain-artifact-adoption`, `fixture-issue-domain-query-adoption`, `fixture-issue-graph-validation-adoption`, `fixture-issue-storage-state-adoption`, `issue-store-conformance-suite`, `library-batch-memory-setup-adoption`, `library-command-git-fixture-adoption`, `library-gate-claim-file-setup-adoption`, `library-init-profile-file-setup-adoption`, `library-item-validate-memory-setup-adoption`, `library-storage-git-fixture-adoption`, `query-domain-owner-gaps`; 13190465 → `auto-transition-harness-owner`, `fast-issue-fixture-issue-adoption`, `fast-issue-memory-setup-adoption`; b45a2865 → `fast-rules-file-setup-adoption`, `fast-rules-fixture-issue-adoption`, `fast-rules-shipped-item-kinds`, `label-query-harness-reduction`; b73b3e5a → `fast-docs-templates-file-setup-adoption`, `fast-docs-templates-fixture-issue-adoption`, `fast-docs-templates-memory-setup-adoption`, `harness-demo-deletion`; 19d9bd7d → `cli-query-graph-in-process-setup-adoption`, `cli-registry-render-spawn-migration`, `cli-serve-artifact-spawn-migration`, `exit-code-reassertion-checkout`, `exit-code-reassertion-cli-query-graph`, `exit-code-reassertion-gate-status`; cd8adfc1 → `library-issue-memory-setup-adoption`; 5fac48b8 → `auto-transition-domain-decision`, `cli-label-query-spawn-migration`, `fast-docs-templates-fixture-issue-adoption`, `fast-issue-fixture-issue-adoption`, `fast-rules-fixture-issue-adoption`, `fast-rules-memory-executor-adoption`, `harness-demo-deletion`, `library-fixture-issue-deletion`, `library-issue-memory-setup-adoption`, `rule-warning-tests-down-a-layer`, `server-file-setup-adoption`. Edges implied by these are not listed; c797b59b needs no direct edge. 89e7a273 → `issue-store-conformance-suite` (D-03) is reached through 0af95f14. The only overlaps left as conflict-avoid are with `memory-backend-path-reads` and `backend-specific-test-pruning`, which sit behind the outgoing edge onto ec190bd5: 0af95f14 and 89e7a273. |
| D-03 Conformance suite owner (steward D2) | Chosen: `issue-store-conformance-suite` owns the suite. 89e7a273 reaches it through 0af95f14 → `issue-store-conformance-suite`, so no direct edge is added. Rejected: waiting for the in-memory fix, because the suite already passes both backends on the cases it carries. |
| D-04 df129d9e ordering (steward D4) | 650e6a56 (count structs as `ListEnvelope`) is done, so REQ-01 and REQ-04 both already follow it, and no edge is needed. Neither waits for the handler moves. `inline-list-output-types` and `command-list-output-types` type the sixteen fallback lists now, so the handler-move tasks find those lists already wrapped. |
| Outgoing edges | `memory-backend-path-reads` → ec190bd5 (ready; `memory.rs`). d125490a and 2492ab29 are done, so `harness-demo-deletion` and `query-available-domain-order` carry no external edge; their overlaps with 2195c98f files are incoming edges in the D-02 row. A live-graph check finds that none of the three external tasks reaches a 2195c98f retype task or c780514e, so no edge closes a cycle. |
| Conflict-avoid, no edge | 5687e281 (`main.rs`/`schema.rs`, also edited by `inline-list-output-types`, `command-list-output-types`, `fixture-setup-hook-removal` and `exit-code-binding-completeness`; 25 handler moves, and waiting would stall REQ-01, REQ-04 and REQ-11). 0e3b69fc (`Cargo.toml` test-support comment; backlog). bc1955a8 and b4fe7887 (`D-[0-9]+` literals; rule-compilation chain). b4929342 and 63c11d08 (`domain::types::fixture_issue` paths, which `shared-fixture-issue` removes). fe761000 (`auto` keys in `.jit/gates.toml`; new gates carry none). f9977efd and fb2dc5c3 (budget script and re-measurement; backlog). e3d06f82 and 3625b18b (event matchers in moved cases). afc2b698 (a different `scratch_build` file). |
| Stale external citations | c400730c and 2ac50581 cite `exit_code_tests.rs`, `query_tests` and `harness_demo`, and cc6ef886 cites `tests/common/`. Their owners must re-point them once the deletions land. |
| Migration scope | Chosen: migrations skip files whose properties move to one owner (`exit_code_tests.rs`, the parity suite, `list_envelope_tests.rs`, `query_tests.rs`). Owner entries delete or rewrite those files on the fixture. Rejected: migrating then deleting them (about 3,600 lines of churn). |
| `TaxonomyRepo` vs. process fixture | Chosen: one shared `TaxonomyRepo` that the process fixture can own. Rejected: folding taxonomy setup into the fixture root, which would merge a 17-file caller migration into the wave-1 foundation. |
| Doctest bound | Chosen: prune rustdoc examples that restate unit-tested properties (`@/inv/essential-code-prose`) until the substep meets the bound. Rejected: an edition-2024 bump for merged doctests, a workspace-wide migration outside this epic. |
| Server drain test | Chosen: a unit test of `run_shutdown_sequence` passes a short deadline (its existing parameter) to own the entitlement, and the process test uses the existing short-deadline seam. Rejected: a paused tokio clock, which needs the `test-util` dependency feature (`@/inv/bounded-rust-build-footprint`); shortening `GRACEFUL_DRAIN_TIMEOUT`, which changes product behaviour. |
| A test over 5 s at report time | `suite-clock-report`'s criterion fails if any test exceeds 5 s, so the report cannot pass over a regression. The execution lead adds a remediation terminal under `suite-cost` for each named test (owning the move down a layer, like the rule-warning and server cases), makes the report depend on it, and re-runs the report. The likeliest candidates are the two `profile_cli_tests` cases (3.66 s quiet). |
| Steward ruling: execution order | Waves 1–2 are on 2195c98f's critical path: the IssueId retype chain (0af95f14 onward) depends on the overlap-derived c780514e terminals in the external-edge table; the steward assigns c780514e execution with waves 1–2 first. |
| Steward ruling D-05: gate bootstrap | The ten self-test gate keys are registered in .jit/gates.toml at planning; each gate task and the container c780514e require them; the container requirement is added at breakdown with `gate add`. |
| D-06 Sizing rule (steward ruling, 2026-10-22) | Sizing rule (steward ruling, 2026-10-22): a terminal touches at most about 6 files, or one file family with one concern; leaves over 6 files name that family and concern in their sizing reason. Applied: the CLI migrations split per file family and per concern (spawn versus id parsing); the library `fixture_issue`, memory, file and git adoptions split per module family, with the production-fixture deletion as its own task after all of them; the exit-code re-assertion sweeps split per file family. Four leaves stay above 6 files, each naming its single family and concern: `profiled-fixture-input-key`, `failure-corpus-in-suite`, `doctest-substep-bound` and `command-list-output-types`. |
| Measurement ordering | Chosen: `build-footprint-report` depends on the four other stories and the remaining suite-cost terminals, so "after" covers all changes. Risk: the stories' holistic reviews then sit on the critical path to the report. |
| Same-file ordering | `exit-code-dependency-guard-case-moves` → `label-query-cli-wiring-reduction` → `exit-code-reassertion-cli-query-graph` serialises `query_json_tests.rs` and `label_query_json_tests.rs`; the wiring cut keeps the moved parent-filter guard. `fast-rules-memory-executor-adoption` → `fast-rules-memory-setup-adoption` → `label-query-harness-reduction` and `fast-rules-shipped-item-kinds` serialise the `fast_rules` executor files. The rule-warning move precedes `fast-rules-file-setup-adoption`, and `cli-query-graph-fixture-issue-adoption` precedes the `cli_query_graph` spawn migrations (`main.rs`). Query reductions depend on `query-domain-owner-gaps`, so each cut lands with its domain owner already present. |
| Shared-file contention | `test_utils.rs`, `dev/TESTING.md`, `.jit/gates.toml`, `.config/nextest.toml`, `repository_state_store_tests.rs` and `commands/issue.rs` are edited by several entries. Edges serialise same-region rewrites: query extraction before the auto-transition decision, finalization deletion before the conformance suite, hook removal before the corpus fixture, and migrations before the re-assertion and wiring cuts in their files. Workers rebase over the remaining advisories. |

## Investigation sources

- [Investigation](investigation.md) holds the claim classification, file and consumer inventories (§1, §3), primitive checks (§4), suite-clock profile (§A), upstream mapping (§B) and test layout (§C).
