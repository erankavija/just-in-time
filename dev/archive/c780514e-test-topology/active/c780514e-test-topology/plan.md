# Plan: Give each test property one owning test on shared fixtures and contracts (c780514e)

> Planning node: 7c885165. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

Fixtures land first, then the per-target migrations onto them; owners gain missing cases before re-proofs are deleted. Each entry lands green. Per-test-bound relief (REQ-11 key, REQ-13 outliers) starts in the first wave with no upstream edge.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | `exit-code-projection-ownership`: the projection suite derives its verified set from its binding cases and absorbs the two row-less parity cases. The 25 unique `the removed exit-code subprocess suite` cases move in three families (envelopes, claim and delete, arguments) before the two re-proof files are deleted. One sweep per CLI target strips projected exit-code assertions in numeric, multi-line, symbolic and helper form, and keeps error-class evidence through the envelope code. | Investigation §1 #1–#3, §D.1, §D.2 |
| REQ-02 | `domain-query-ownership`: extract the `query available` composition and order, and the auto-transition decision, into pure domain functions. Then cut `query_tests`, the label and filter CLI files, and the executor label tests to wiring cases, cut cycle re-proofs, move six `harness_demo` cases and delete the suite. | §1 #4–#8, §5, §D.3 |
| REQ-03 | Add missing finalization cases to `repository_state`, then delete the storage determinism section. | §1 #9 |
| REQ-04 | `list-envelope-coverage`: sixteen list-emitting commands found from their emission sites, not the schema, gain typed `ListEnvelope` outputs (six inline bodies plus a schema key, then nine command-layer results). One schema-driven test then replaces the 19 per-command cases with a runtime guard against untyped list output, and the per-command re-assertions go. | §1 #10, §4, §D.4 |
| REQ-05 | `cli-process-fixture` (wave 1), then one spawn migration per file family (23 families across the eight CLI target slices), plus a separate JSON-id migration per target for the files that scrape human-readable output (D-06). Git setup adoptions are split per test boundary (library, server, `fast_docs_templates`). File-backed and memory-backed setups each get a shared-constructor foundation, then adoptions per boundary across the whole workspace: the three in-process suites, the in-process helpers of three CLI suites, two library module families plus the library file-backed modules, and the server crate. The mechanical sweep in §D.6 classifies every candidate helper and names its owner. Files whose properties move elsewhere are excluded from migration and handled by their owner entries. | §1 #11–#17, §C, §D.6 |
| REQ-06 | Wave-1 foundations: shared `fixture_issue`, shared `TaxonomyRepo`, and product initialisation over any store (followed by the shared memory executor built on it). One adoption per test boundary then retires each copy: library and four suites for `fixture_issue`, two CLI suites for `TaxonomyRepo` (callers per spawn migration, then one root-removal leaf per suite), `fast_rules` for the executor. The memory-backed adoptions in the suites, the library and the server remove every private memory executor and literal index body. | §1 #18, §4, §D.6 |
| REQ-07 | `shipped-item-kinds-fixture` derivation (wave 1), then one adoption each for the harness tables, command unit tests, `cli_item_validate` and `fast_rules`. | §1 #20–#21 |
| REQ-08 | `issue-store-conformance` (absorbs the three ad-hoc tests), then the in-memory path-read fix, then backend-test pruning. | §1 #22–#24 |
| REQ-09 | The receipt env lock goes with the runner hook; no test changes process-wide state, and varied values pass explicitly (user decision 2026-10-28, after 55043636 removed `EnvGuard`/`CurrentDirGuard`). | §1 #25, §4 |
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
| env-reading-tests-guarded | Tests read no shared process environment | task | No test reads a variable another test sets; a varied value passes explicitly. | — | REQ-09 | touches 1, uncertain | — | fixture-setup-hook-removal |
| exit-code-binding-completeness | Exit-code projection suite proves a binding behind each row | task | The projection suite fails when a projected exit-code row lacks a binding case in both invocation forms. | exit-code-projection-ownership | REQ-01, D-04 | touches 1, uncertain | — | cli-issue-failure-corpus-spawn-migration |
| exit-code-reproof-deletion | Exit-code re-proof suites are deleted | task | the removed exit-code subprocess suite and the parity suite are gone; the projection suite alone binds exit codes. | exit-code-projection-ownership | REQ-01, D-04 | touches 3 | — | exit-code-envelope-case-moves, exit-code-claim-delete-case-moves, exit-code-issue-prefix-case-moves, exit-code-dependency-guard-case-moves, exit-code-gate-preset-case-move |
| query-available-domain-order | Available-work filtering and priority order become a pure domain query | task | query available's filter composition and priority order are one pure domain function with unit and property tests. | domain-query-ownership | REQ-02 | touches 3 | — | — |
| auto-transition-domain-decision | Auto-transition readiness is decided by a pure domain function | task | A pure domain function decides which dependents a terminal transition makes ready. | domain-query-ownership | REQ-02 | touches 3 | — | query-available-domain-order |
| harness-demo-case-moves | Unique harness demonstration cases move beside their owners | task | The six harness-demonstration cases with no other owner have owning tests in the library modules they exercise. | shipped-item-kinds-fixture | REQ-02 | touches 4 | — | harness-shipped-item-kinds, unit-test-shipped-item-kinds, library-item-validate-memory-setup-adoption, fixture-issue-commands-a-adoption, fixture-issue-commands-b-adoption, fixture-issue-domain-query-adoption, library-issue-memory-setup-adoption, library-archive-file-setup-adoption |
| harness-demo-deletion | The harness demonstration suite is deleted | task | harness_demo.rs is gone, and contributor guidance cites owning tests instead. | — | REQ-02 | touches 3 | — | harness-demo-case-moves, fast-docs-templates-fixture-issue-adoption |
| finalization-owner-gaps | Repository-state tests own each finalization case | task | Each finalization case the storage determinism section covers has an owning repository_state test. | — | REQ-03 | touches 1 | — | — |
| finalization-reproof-deletion | Storage tests stop re-proving mutation finalization | task | The storage determinism section is gone, and storage tests cover only backend-added behaviour. | — | REQ-03 | touches 1 | — | finalization-owner-gaps, fixture-issue-storage-state-adoption, library-storage-git-fixture-adoption |
| issue-store-conformance-suite | Shared IssueStore conformance suite runs against both backends | task | One reader and writer conformance suite runs unchanged against JsonFileStorage and InMemoryStorage. | — | REQ-08, D-03 | creates 1, touches 2 | — | finalization-reproof-deletion |
| memory-backend-path-reads | In-memory path reads serve the store's own commit-pinned state | task | InMemoryStorage path reads, decoding and commit-pinned reads pass the shared conformance suite. | issue-store-conformance | REQ-08 | touches 3, uncertain | — | issue-store-conformance-suite |
| backend-specific-test-pruning | Backend tests keep only backend-specific behaviour | task | JSON and in-memory storage tests assert only behaviour their own backend has. | issue-store-conformance | REQ-08 | touches 3 | — | memory-backend-path-reads |
| profiled-fixture-build-lock | Profiled-repository fixture lock waits are bounded | task | Profiled-fixture consumers make each entry once and never wait on the lock past the per-test bound. | — | REQ-11, REQ-13, D-01 | touches 1 | — | — |
| profiled-fixture-input-key | Profiled-repository fixture keyed only on its inputs | task | The profiled-repository fixture key derives from package bytes and crate source, and stale entries are collected. | — | REQ-11, REQ-13, D-01 | touches 8 | — | profiled-fixture-build-lock |
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
| exit-code-envelope-case-moves | Blocked-transition and startup envelope cases move beside their owners | task | Blocked-transition and startup-envelope cases live in the error-envelope suites with their non-exit assertions. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 2 | — | exit-code-binding-completeness, cli-issue-error-output-json-ids |
| exit-code-claim-delete-case-moves | Claim and confirmed-delete cases move beside their owners | task | Claim and confirmed-delete cases live in the claim and lifecycle suites with their non-exit assertions. | exit-code-projection-ownership, cli-process-fixture | REQ-01, D-04 | touches 2 | — | exit-code-binding-completeness, cli-claim-worktree-spawn-migration, cli-workflow-json-ids |
| exit-code-reassertion-cli-query-graph | cli_query_graph tests stop re-asserting projected exit codes | task | The cli_query_graph suite asserts only properties it adds, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 5 | — | label-query-cli-wiring-reduction, cli-query-graph-in-process-setup-adoption |
| exit-code-reassertion-cli-item-validate | cli_item_validate tests stop re-asserting projected exit codes | task | The cli_item_validate suite asserts only properties it adds, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 5 | — | exit-code-binding-completeness, cli-registry-render-spawn-migration, cli-validate-json-ids |
| inline-list-output-types | Inline list bodies in main.rs become typed list envelopes | task | Six inline list bodies emit typed list envelopes, and graph rdeps declares its schema. | list-envelope-coverage | REQ-04, D-04 | touches 5 | — | — |
| command-list-output-types | Command-layer list results become typed list envelopes | task | Command-layer list results for nine commands are list envelopes declared as their output schemas. | list-envelope-coverage | REQ-04, D-04 | touches 7 | — | inline-list-output-types |
| list-envelope-coverage-test | One test proves the list-envelope count for each list-emitting command | task | A single schema-driven test checks count against collection length for each list-emitting command. | list-envelope-coverage, cli-process-fixture | REQ-04, D-04 | touches 2 | — | command-list-output-types, cli-issue-show-spawn-migration |
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
| cli-repo-workflow-list-envelope-reassertion-removal | cli_repo_workflow tests stop re-asserting the list-envelope count | task | No cli_repo_workflow test re-asserts count against collection length. | list-envelope-coverage | REQ-04 | touches 3 | — | list-envelope-coverage-test, cli-profile-json-ids, cli-claim-worktree-spawn-migration |
| query-domain-owner-gaps | Domain query tests own each property the query re-proofs assert | task | domain/queries.rs owns each query property the CLI and executor re-proofs assert. | domain-query-ownership | REQ-02 | touches 1 | — | query-available-domain-order |
| query-cli-wiring-reduction | Query CLI tests assert one wiring case per subcommand and flag | task | query_tests.rs keeps one wiring case per query subcommand or flag on the shared CLI process fixture. | domain-query-ownership, cli-process-fixture | REQ-02, REQ-05 | touches 2 | — | query-domain-owner-gaps, cli-process-fixture, shared-taxonomy-repo |
| label-query-cli-wiring-reduction | Label and filter CLI tests assert one wiring case per subcommand and flag | task | Label and filter CLI tests keep one wiring case per subcommand or flag, and the domain owns label semantics. | domain-query-ownership | REQ-02 | touches 4 | — | query-domain-owner-gaps, exit-code-dependency-guard-case-moves |
| label-query-harness-reduction | Label and strategic executor tests keep one wiring case each | task | Executor label and strategic tests keep one wiring case per entry point, and the domain owns the semantics. | domain-query-ownership | REQ-02 | touches 2 | — | query-domain-owner-gaps, fast-rules-memory-setup-adoption |
| auto-transition-wiring-reduction | Workflow CLI tests keep one auto-transition wiring case | task | One CLI case shows completing a dependency readies its dependent; the domain owns the rules. | domain-query-ownership | REQ-02 | touches 1 | — | auto-transition-domain-decision, cli-workflow-json-ids |
| auto-transition-harness-owner | Lifecycle harness tests stop re-proving the auto-transition cascade | task | Lifecycle harness tests assert timestamps only, and one harness test owns the cascade wiring. | domain-query-ownership | REQ-02 | touches 1 | — | auto-transition-domain-decision |
| batch-cycle-adapter-reproof-removal | Batch-create stops re-proving keyed cycle detection | task | Batch-create keeps at most one adapter wiring test, and graph tests own keyed cycle detection. | domain-query-ownership | REQ-02 | touches 1 | — | library-batch-memory-setup-adoption |
| cli-cycle-reproof-removal | Workflow CLI tests stop re-proving dependency cycle rejection | task | integration_test.rs holds no dependency cycle re-proof. | domain-query-ownership | REQ-02 | touches 1 | — | cli-workflow-json-ids |
| docs-check-selftest-gate | Documentation checker self-test runs as a declared gate | task | The documentation checker self-test runs as its own gate over the checkers. | script-self-test-gates | REQ-10 | touches 2, uncertain | — | — |
| shipped-policy-regions-selftest-gate | Shipped-policy region generator self-test runs as a declared gate | task | The shipped-policy region generator self-test runs as its own gate over the generator. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| workflow-contract-selftest-gate | Workflow-contract harness self-test runs as a declared gate | task | The workflow-contract self-test runs as its own gate over the harness and its vectors. | script-self-test-gates | REQ-10 | touches 2, uncertain | — | — |
| session-cost-benchmark-selftest-gate | Session-cost benchmark self-test runs as a declared gate | task | The session-cost benchmark self-test runs as its own gate over the benchmark script. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| standards-scan-test-gate | Standards-scan test suite runs as a declared gate | task | The standards-scan test suite runs as its own gate over the scanner. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| standards-fix-test-gate | Standards-fix test suite runs as a declared gate | task | The standards-fix test suite runs as its own gate over the fixer. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| manifest-tool-test-gate | Breakdown-manifest tool test suite runs as a declared gate | task | The breakdown-manifest tool test suite runs as its own gate over the tool and its fixtures. | script-self-test-gates | REQ-10 | touches 1, uncertain | — | — |
| lock-tests-deterministic-precondition | Read-only lock tests establish their precondition on any host | task | The four read-only lock tests run their assertions on any host and never pass by skipping. | — | — | touches 1 | — | — |
| library-item-validate-memory-setup-adoption | Item and validate unit tests use the shared memory seeding | task | Item and validate unit tests build memory executors through the shared seeding, with no literal index. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | memory-declaration-seeding |
| server-memory-setup-adoption | Server tests use the shared memory seeding | task | Server tests build memory stores through the shared seeding, with no literal index. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | memory-declaration-seeding, server-git-fixture-adoption |
| server-file-setup-adoption | Server tests use the shared file-backed constructors | task | Server tests build file-backed repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 3 | — | file-backed-setup-constructors, server-memory-setup-adoption, server-shutdown-test-bound |
| cli-item-validate-in-process-setup-adoption | cli_item_validate in-process tests use the shared constructors | task | The cli_item_validate in-process tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, REQ-06, D-02 | touches 3 | — | file-backed-setup-constructors, cli-item-validate-shipped-item-kinds |
| cli-query-graph-in-process-setup-adoption | cli_query_graph in-process tests use the shared constructors | task | The cli_query_graph in-process tests build repositories and executors through the shared constructors. | file-setup-constructors, memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 2 | — | file-backed-setup-constructors, memory-declaration-seeding, cli-label-query-spawn-migration, cli-links-search-spawn-migration |
| cli-repo-workflow-in-process-setup-adoption | cli_repo_workflow in-process tests use the shared constructors | task | The cli_repo_workflow in-process tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, REQ-06, D-02 | touches 2 | — | file-backed-setup-constructors, cli-profile-spawn-migration |
| exit-code-envelope-ownership | Exit codes and list envelopes each have one owning test | story | command exit codes and the list-envelope count each have one owning test | — | REQ-01, REQ-04 | — | — | exit-code-reproof-deletion, cli-issue-list-envelope-reassertion-removal, cli-query-graph-list-envelope-reassertion-removal, cli-repo-workflow-list-envelope-reassertion-removal, exit-code-reassertion-gate-evaluate, exit-code-reassertion-gate-status, exit-code-reassertion-issue-create, exit-code-reassertion-issue-show, exit-code-reassertion-issue-errors, exit-code-reassertion-cli-query-graph, exit-code-reassertion-cli-item-validate, exit-code-reassertion-profile, exit-code-reassertion-checkout, exit-code-reassertion-workflow |
| query-graph-ownership | Query, transition and cycle semantics are proven at the domain and graph layers | story | query filters, auto-transitions and cycle detection are owned by domain and graph tests, with CLI and harness tests reduced to wiring | — | REQ-02 | — | — | query-cli-wiring-reduction, auto-transition-wiring-reduction, auto-transition-harness-owner, batch-cycle-adapter-reproof-removal, cli-cycle-reproof-removal, harness-demo-deletion, label-query-cli-wiring-reduction, label-query-harness-reduction |
| storage-contracts | Storage behaviour is proven once per contract | story | finalization is proven in repository state and every IssueStore backend runs one conformance suite | — | REQ-03, REQ-08 | — | — | backend-specific-test-pruning |
| shared-test-fixtures | Tests run on shared fixtures | story | CLI and in-process tests use one definition of each fixture and never change process-wide state | — | REQ-05, REQ-06, REQ-07, REQ-09, D-02 | — | — | cli-apply-template-spawn-migration, cli-config-spawn-migration, cli-gate-definition-json-ids, cli-gate-status-json-ids, cli-graph-json-ids, cli-issue-error-output-json-ids, cli-issue-failure-corpus-spawn-migration, cli-issue-show-json-ids, cli-issue-taxonomy-root-removal, cli-issue-update-json-ids, cli-item-validate-in-process-setup-adoption, cli-label-query-json-ids, cli-profile-json-ids, cli-query-graph-in-process-setup-adoption, cli-query-graph-taxonomy-root-removal, cli-registry-render-spawn-migration, cli-repo-workflow-in-process-setup-adoption, cli-validate-json-ids, cli-workflow-json-ids, cli-worktree-json-ids, env-reading-tests-guarded, fast-docs-templates-file-setup-adoption, fast-docs-templates-fixture-issue-adoption, fast-docs-templates-git-fixture-adoption, fast-docs-templates-memory-setup-adoption, fast-issue-fixture-issue-adoption, fast-issue-memory-setup-adoption, fast-rules-file-setup-adoption, fast-rules-fixture-issue-adoption, fast-rules-shipped-item-kinds, harness-shipped-item-kinds, library-archive-file-setup-adoption, library-batch-memory-setup-adoption, library-fixture-issue-deletion, library-gate-claim-file-setup-adoption, library-init-profile-file-setup-adoption, library-issue-memory-setup-adoption, library-item-validate-memory-setup-adoption, library-storage-git-fixture-adoption, library-store-validation-file-setup-adoption, server-file-setup-adoption, unit-test-shipped-item-kinds |
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
| cli-claim-worktree-spawn-migration | claim and worktree tests spawn jit through the shared process fixture | task | The claim and worktree tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 6 | — | cli-document-history-spawn-migration, cli-init-discovery-spawn-migration |
| cli-init-discovery-spawn-migration | init and discovery tests spawn jit through the shared process fixture | task | The init and discovery tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 3 | — | cli-process-fixture |
| cli-document-history-spawn-migration | document and archive tests spawn jit through the shared process fixture | task | The document and archive tests spawn jit only through the shared CLI process fixture. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-serve-artifact-spawn-migration |
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
| library-batch-memory-setup-adoption | Batch-command tests use the shared memory seeding | task | The batch, bulk and event command tests obtain memory executors from the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 3 | — | memory-declaration-seeding, cli-process-fixture |
| library-gate-memory-setup-adoption | Gate command tests use the shared memory seeding | task | Gate, gate-check and gate CLI command tests build memory executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 3 | — | memory-declaration-seeding, cli-process-fixture |
| library-issue-memory-setup-adoption | Issue, invariant and executor tests use the shared memory seeding | task | Issue, invariant and executor-core tests build memory executors through the shared seeding. | memory-declaration-seeding | REQ-05, REQ-06, D-02 | touches 3 | — | memory-declaration-seeding, library-command-git-fixture-adoption |
| library-archive-file-setup-adoption | Archive command tests use the shared file-backed constructors | task | Archive command tests build repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 1 | — | file-backed-setup-constructors, memory-declaration-seeding |
| library-gate-claim-file-setup-adoption | Gate, claim and hook tests use the shared file-backed constructors | task | Gate-check, claim and hook tests build file-backed repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 3 | — | file-backed-setup-constructors, library-gate-memory-setup-adoption |
| library-init-profile-file-setup-adoption | Init and profile command tests use the shared file-backed constructors | task | Init and profile command tests build file-backed repositories and executors through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 2 | — | file-backed-setup-constructors, memory-declaration-seeding |
| library-store-validation-file-setup-adoption | Gate-run, repository-validation and CLI tests use the shared file-backed constructors | task | Gate-run storage, repository-validation and main tests build file-backed repositories through the shared constructors. | file-setup-constructors | REQ-05, D-02 | touches 3 | — | file-backed-setup-constructors, memory-declaration-seeding |
| library-command-git-fixture-adoption | Command unit tests initialise git through the shared helper | task | Command unit-test modules initialise git repositories through the shared helper. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| library-storage-git-fixture-adoption | Storage and gate-execution tests initialise git through the shared helper | task | Storage, gate-execution and profile-package tests initialise git repositories through the shared helper. | cli-process-fixture | REQ-05, D-02 | touches 4 | — | cli-process-fixture |
| exit-code-reassertion-gate-evaluate | Gate evaluation tests stop re-asserting projected exit codes | task | The gate evaluation tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-binding-completeness, cli-gate-evaluate-spawn-migration |
| exit-code-reassertion-gate-status | Gate status and definition tests stop re-asserting projected exit codes | task | The gate status and definition tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 5 | — | exit-code-binding-completeness, cli-gate-definition-json-ids, cli-gate-status-json-ids |
| exit-code-reassertion-issue-create | Issue-creation tests stop re-asserting projected exit codes | task | The issue-creation, search, rollup and export tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 5 | — | exit-code-envelope-case-moves, exit-code-issue-prefix-case-moves, cli-issue-create-spawn-migration, cli-issue-update-spawn-migration |
| exit-code-reassertion-issue-show | Issue show and failure-envelope tests stop re-asserting projected exit codes | task | Issue show, status and top-level failure tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-envelope-case-moves, exit-code-issue-prefix-case-moves |
| exit-code-reassertion-issue-errors | Error-output tests stop re-asserting projected exit codes | task | Error-output tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-envelope-case-moves, exit-code-issue-prefix-case-moves |
| exit-code-reassertion-profile | Profile tests stop re-asserting projected exit codes | task | Profile tests assert only properties they add, with no projected exit-code re-assertion. | exit-code-projection-ownership | REQ-01 | touches 3 | — | exit-code-claim-delete-case-moves, exit-code-gate-preset-case-move, cli-repo-workflow-in-process-setup-adoption, cli-profile-json-ids |
| exit-code-reassertion-checkout | Claim, lock and discovery tests stop re-asserting projected exit codes | task | Claim, lock, discovery and worktree tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 4 | — | exit-code-claim-delete-case-moves, exit-code-gate-preset-case-move |
| exit-code-reassertion-workflow | Configuration and workflow tests stop re-asserting projected exit codes | task | Configuration, serve and workflow tests assert only properties they add. | exit-code-projection-ownership | REQ-01 | touches 6 | — | exit-code-claim-delete-case-moves, exit-code-gate-preset-case-move, cli-config-spawn-migration |
| cli-issue-taxonomy-root-removal | The cli_issue suite root drops its taxonomy repository copy | task | The cli_issue suite root defines no taxonomy repository copy. | — | REQ-06, D-02 | touches 1 | — | cli-issue-create-spawn-migration, cli-issue-show-spawn-migration, cli-issue-update-spawn-migration |
| cli-query-graph-taxonomy-root-removal | The cli_query_graph suite root drops its taxonomy repository copy | task | The cli_query_graph suite root defines no taxonomy repository copy. | — | REQ-06, D-02 | touches 1 | — | cli-graph-spawn-migration, cli-label-query-spawn-migration, cli-links-search-spawn-migration, query-cli-wiring-reduction |

```mermaid
flowchart LR
    N0["cli-process-fixture: Shared fixture for spawning jit in CLI tests"]
    N1["env-reading-tests-guarded: Tests read no shared process environment"]
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
    N13["profiled-fixture-build-lock: Profiled-repository fixture lock waits are bounded"]
    N14["profiled-fixture-input-key: Profiled-repository fixture keyed only on its inputs"]
    N15["fixture-setup-hook-removal: Fixture setup runs inside the suite with no runner hook"]
    N16["failure-corpus-in-suite: Recorded-failure corpus built in the suite under an input key"]
    N17["rule-warning-tests-down-a-layer: Create-time rule warnings proven in-process without a profiled repository"]
    N18["doctest-substep-bound: Doctest substep fits within the per-test bound"]
    N19["cargo-ci-selftest-gate: cargo-ci self-test runs as a declared gate"]
    N20["profile-suite-selftest-gate: Suite-profiler self-test runs as a declared gate"]
    N21["build-budget-selftest-gate: Build-budget checker self-test runs as a declared gate"]
    N22["dependency-policy-tool-failure: Dependency-feature policy tests fail when cargo is missing"]
    N23["build-footprint-report: Build-footprint figures before and after the epic are reported"]
    N24["suite-clock-report: Suite clock before and after the epic is reported"]
    N25["exit-code-envelope-case-moves: Blocked-transition and startup envelope cases move beside their owners"]
    N26["exit-code-claim-delete-case-moves: Claim and confirmed-delete cases move beside their owners"]
    N27["exit-code-reassertion-cli-query-graph: cli_query_graph tests stop re-asserting projected exit codes"]
    N28["exit-code-reassertion-cli-item-validate: cli_item_validate tests stop re-asserting projected exit codes"]
    N29["inline-list-output-types: Inline list bodies in main.rs become typed list envelopes"]
    N30["command-list-output-types: Command-layer list results become typed list envelopes"]
    N31["list-envelope-coverage-test: One test proves the list-envelope count for each list-emitting command"]
    N32["mcp-parity-tool-failure: MCP parity test fails when node is missing"]
    N33["server-shutdown-test-bound: Server shutdown force-close test stays under the per-test bound"]
    N34["shared-taxonomy-repo: Shared taxonomy repository type for CLI tests"]
    N35["shared-fixture-issue: Shared issue-construction fixture"]
    N36["fast-issue-fixture-issue-adoption: fast_issue uses the shared issue fixture"]
    N37["fast-docs-templates-fixture-issue-adoption: fast_docs_templates uses the shared issue fixture"]
    N38["fast-rules-fixture-issue-adoption: fast_rules uses the shared issue fixture"]
    N39["cli-query-graph-fixture-issue-adoption: cli_query_graph uses the shared issue fixture"]
    N40["store-generic-initialisation: Product initialisation runs over any repository-state store"]
    N41["shared-memory-executor: Memory executor initialised through product initialisation"]
    N42["fast-rules-memory-executor-adoption: fast_rules uses the shared memory executor"]
    N43["shipped-item-kinds-derivation: Shared test support derives shipped item kinds from the default package"]
    N44["harness-shipped-item-kinds: Harness and test taxonomy take shipped item kinds from the derivation"]
    N45["unit-test-shipped-item-kinds: Command unit tests take shipped item kinds from the default package"]
    N46["cli-item-validate-shipped-item-kinds: Decision-kind CLI tests take shipped item kinds from the default package"]
    N47["fast-rules-shipped-item-kinds: Projection-render tests take shipped item kinds from the default package"]
    N48["server-git-fixture-adoption: Server unit tests initialise git through the shared helper"]
    N49["fast-docs-templates-git-fixture-adoption: Git revision tests initialise git through the shared helper"]
    N50["file-backed-setup-constructors: Shared constructors for file-backed in-process test repositories"]
    N51["fast-rules-file-setup-adoption: fast_rules file-backed tests use the shared constructors"]
    N52["fast-docs-templates-file-setup-adoption: fast_docs_templates file-backed tests use the shared constructors"]
    N53["memory-declaration-seeding: Shared memory executor accepts declarations as data"]
    N54["fast-issue-memory-setup-adoption: fast_issue memory-backed tests use the shared seeding"]
    N55["fast-rules-memory-setup-adoption: fast_rules memory-backed tests use the shared seeding"]
    N56["fast-docs-templates-memory-setup-adoption: fast_docs_templates memory-backed tests use the shared seeding"]
    N57["exit-code-issue-prefix-case-moves: Issue-show prefix cases move beside their owner"]
    N58["exit-code-dependency-guard-case-moves: Dependency and query argument-guard cases move beside their owners"]
    N59["exit-code-gate-preset-case-move: Gate preset registry case moves beside its owner"]
    N60["cli-issue-list-envelope-reassertion-removal: cli_issue tests stop re-asserting the list-envelope count"]
    N61["cli-query-graph-list-envelope-reassertion-removal: cli_query_graph tests stop re-asserting the list-envelope count"]
    N62["cli-repo-workflow-list-envelope-reassertion-removal: cli_repo_workflow tests stop re-asserting the list-envelope count"]
    N63["query-domain-owner-gaps: Domain query tests own each property the query re-proofs assert"]
    N64["query-cli-wiring-reduction: Query CLI tests assert one wiring case per subcommand and flag"]
    N65["label-query-cli-wiring-reduction: Label and filter CLI tests assert one wiring case per subcommand and flag"]
    N66["label-query-harness-reduction: Label and strategic executor tests keep one wiring case each"]
    N67["auto-transition-wiring-reduction: Workflow CLI tests keep one auto-transition wiring case"]
    N68["auto-transition-harness-owner: Lifecycle harness tests stop re-proving the auto-transition cascade"]
    N69["batch-cycle-adapter-reproof-removal: Batch-create stops re-proving keyed cycle detection"]
    N70["cli-cycle-reproof-removal: Workflow CLI tests stop re-proving dependency cycle rejection"]
    N71["docs-check-selftest-gate: Documentation checker self-test runs as a declared gate"]
    N72["shipped-policy-regions-selftest-gate: Shipped-policy region generator self-test runs as a declared gate"]
    N73["workflow-contract-selftest-gate: Workflow-contract harness self-test runs as a declared gate"]
    N74["session-cost-benchmark-selftest-gate: Session-cost benchmark self-test runs as a declared gate"]
    N75["standards-scan-test-gate: Standards-scan test suite runs as a declared gate"]
    N76["standards-fix-test-gate: Standards-fix test suite runs as a declared gate"]
    N77["manifest-tool-test-gate: Breakdown-manifest tool test suite runs as a declared gate"]
    N78["lock-tests-deterministic-precondition: Read-only lock tests establish their precondition on any host"]
    N79["library-item-validate-memory-setup-adoption: Item and validate unit tests use the shared memory seeding"]
    N80["server-memory-setup-adoption: Server tests use the shared memory seeding"]
    N81["server-file-setup-adoption: Server tests use the shared file-backed constructors"]
    N82["cli-item-validate-in-process-setup-adoption: cli_item_validate in-process tests use the shared constructors"]
    N83["cli-query-graph-in-process-setup-adoption: cli_query_graph in-process tests use the shared constructors"]
    N84["cli-repo-workflow-in-process-setup-adoption: cli_repo_workflow in-process tests use the shared constructors"]
    N85["exit-code-envelope-ownership: Exit codes and list envelopes each have one owning test"]
    N86["query-graph-ownership: Query, transition and cycle semantics are proven at the domain and graph layers"]
    N87["storage-contracts: Storage behaviour is proven once per contract"]
    N88["shared-test-fixtures: Tests run on shared fixtures"]
    N89["suite-cost: The default suite runs within its time and footprint budgets"]
    N90["cli-gate-evaluate-spawn-migration: gate evaluation tests spawn jit through the shared process fixture"]
    N91["cli-gate-status-spawn-migration: gate status and key tests spawn jit through the shared process fixture"]
    N92["cli-gate-definition-spawn-migration: gate definition tests spawn jit through the shared process fixture"]
    N93["cli-gate-definition-json-ids: gate definition and evaluation tests read created ids from JSON output"]
    N94["cli-gate-status-json-ids: gate status and key tests read created ids from JSON output"]
    N95["cli-issue-create-spawn-migration: issue create and batch tests spawn jit through the shared process fixture"]
    N96["cli-issue-show-spawn-migration: issue show and status tests spawn jit through the shared process fixture"]
    N97["cli-issue-update-spawn-migration: issue search and update tests spawn jit through the shared process fixture"]
    N98["cli-issue-show-json-ids: issue show and rollup tests read created ids from JSON output"]
    N99["cli-issue-update-json-ids: issue mutation tests read created ids from JSON output"]
    N100["cli-issue-failure-corpus-spawn-migration: failure-envelope contract tests spawn jit through the shared process fixture"]
    N101["cli-issue-error-output-spawn-migration: error-output tests spawn jit through the shared process fixture"]
    N102["cli-issue-output-mode-spawn-migration: output-mode and export tests spawn jit through the shared process fixture"]
    N103["cli-issue-error-output-json-ids: error-output and export tests read created ids from JSON output"]
    N104["cli-item-kind-spawn-migration: item-kind tests spawn jit through the shared process fixture"]
    N105["cli-validate-spawn-migration: validate tests spawn jit through the shared process fixture"]
    N106["cli-registry-render-spawn-migration: registry and render tests spawn jit through the shared process fixture"]
    N107["cli-validate-json-ids: validate tests read created ids from JSON output"]
    N108["cli-graph-spawn-migration: graph and dependency tests spawn jit through the shared process fixture"]
    N109["cli-label-query-spawn-migration: label and query tests spawn jit through the shared process fixture"]
    N110["cli-links-search-spawn-migration: link, scope and search tests spawn jit through the shared process fixture"]
    N111["cli-graph-json-ids: graph and dependency tests read created ids from JSON output"]
    N112["cli-label-query-json-ids: label and query tests read created ids from JSON output"]
    N113["cli-claim-worktree-spawn-migration: claim and worktree tests spawn jit through the shared process fixture"]
    N114["cli-init-discovery-spawn-migration: init and discovery tests spawn jit through the shared process fixture"]
    N115["cli-document-history-spawn-migration: document and archive tests spawn jit through the shared process fixture"]
    N116["cli-worktree-json-ids: cross-worktree tests read created ids from JSON output"]
    N117["cli-profile-spawn-migration: profile tests spawn jit through the shared process fixture"]
    N118["cli-apply-template-spawn-migration: apply, preset and template tests spawn jit through the shared process fixture"]
    N119["cli-profile-json-ids: profile tests read created ids from JSON output"]
    N120["cli-config-spawn-migration: configuration and CLI-surface tests spawn jit through the shared process fixture"]
    N121["cli-workflow-integration-spawn-migration: workflow and integration tests spawn jit through the shared process fixture"]
    N122["cli-serve-artifact-spawn-migration: serve and artifact tests spawn jit through the shared process fixture"]
    N123["cli-workflow-json-ids: workflow and integration tests read created ids from JSON output"]
    N124["fixture-issue-commands-a-adoption: Archive, claim, dependency, document and gate command unit tests use the shared issue fixture"]
    N125["fixture-issue-commands-b-adoption: Gate-check, issue, item, executor and snapshot command unit tests use the shared issue fixture"]
    N126["fixture-issue-commands-c-adoption: Template, validate and worktree command unit tests use the shared issue fixture"]
    N127["fixture-issue-domain-artifact-adoption: Domain artifact and projection unit tests use the shared issue fixture"]
    N128["fixture-issue-domain-query-adoption: Domain query, item and output unit tests use the shared issue fixture"]
    N129["fixture-issue-graph-validation-adoption: Graph, validation and declaration unit tests use the shared issue fixture"]
    N130["fixture-issue-storage-state-adoption: Storage and repository-state unit tests use the shared issue fixture"]
    N131["library-fixture-issue-deletion: The library's own issue-construction fixture is deleted"]
    N132["library-batch-memory-setup-adoption: Batch-command tests use the shared memory seeding"]
    N133["library-gate-memory-setup-adoption: Gate command tests use the shared memory seeding"]
    N134["library-issue-memory-setup-adoption: Issue, invariant and executor tests use the shared memory seeding"]
    N135["library-archive-file-setup-adoption: Archive command tests use the shared file-backed constructors"]
    N136["library-gate-claim-file-setup-adoption: Gate, claim and hook tests use the shared file-backed constructors"]
    N137["library-init-profile-file-setup-adoption: Init and profile command tests use the shared file-backed constructors"]
    N138["library-store-validation-file-setup-adoption: Gate-run, repository-validation and CLI tests use the shared file-backed constructors"]
    N139["library-command-git-fixture-adoption: Command unit tests initialise git through the shared helper"]
    N140["library-storage-git-fixture-adoption: Storage and gate-execution tests initialise git through the shared helper"]
    N141["exit-code-reassertion-gate-evaluate: Gate evaluation tests stop re-asserting projected exit codes"]
    N142["exit-code-reassertion-gate-status: Gate status and definition tests stop re-asserting projected exit codes"]
    N143["exit-code-reassertion-issue-create: Issue-creation tests stop re-asserting projected exit codes"]
    N144["exit-code-reassertion-issue-show: Issue show and failure-envelope tests stop re-asserting projected exit codes"]
    N145["exit-code-reassertion-issue-errors: Error-output tests stop re-asserting projected exit codes"]
    N146["exit-code-reassertion-profile: Profile tests stop re-asserting projected exit codes"]
    N147["exit-code-reassertion-checkout: Claim, lock and discovery tests stop re-asserting projected exit codes"]
    N148["exit-code-reassertion-workflow: Configuration and workflow tests stop re-asserting projected exit codes"]
    N149["cli-issue-taxonomy-root-removal: The cli_issue suite root drops its taxonomy repository copy"]
    N150["cli-query-graph-taxonomy-root-removal: The cli_query_graph suite root drops its taxonomy repository copy"]
    N15 --> N1
    N100 --> N2
    N25 --> N3
    N26 --> N3
    N57 --> N3
    N58 --> N3
    N59 --> N3
    N4 --> N5
    N44 --> N6
    N45 --> N6
    N79 --> N6
    N124 --> N6
    N125 --> N6
    N128 --> N6
    N134 --> N6
    N135 --> N6
    N6 --> N7
    N37 --> N7
    N8 --> N9
    N130 --> N9
    N140 --> N9
    N9 --> N10
    N10 --> N11
    N11 --> N12
    N13 --> N14
    N14 --> N15
    N15 --> N16
    N100 --> N16
    N85 --> N23
    N86 --> N23
    N87 --> N23
    N88 --> N23
    N16 --> N23
    N18 --> N23
    N19 --> N23
    N20 --> N23
    N21 --> N23
    N22 --> N23
    N32 --> N23
    N71 --> N23
    N72 --> N23
    N73 --> N23
    N74 --> N23
    N75 --> N23
    N76 --> N23
    N77 --> N23
    N78 --> N23
    N23 --> N24
    N2 --> N25
    N103 --> N25
    N2 --> N26
    N113 --> N26
    N123 --> N26
    N65 --> N27
    N83 --> N27
    N2 --> N28
    N106 --> N28
    N107 --> N28
    N29 --> N30
    N30 --> N31
    N96 --> N31
    N107 --> N32
    N35 --> N36
    N35 --> N37
    N35 --> N38
    N35 --> N39
    N40 --> N41
    N41 --> N42
    N43 --> N44
    N43 --> N45
    N43 --> N46
    N104 --> N46
    N43 --> N47
    N55 --> N47
    N0 --> N48
    N0 --> N49
    N50 --> N51
    N17 --> N51
    N50 --> N52
    N41 --> N53
    N53 --> N54
    N53 --> N55
    N42 --> N55
    N53 --> N56
    N2 --> N57
    N98 --> N57
    N2 --> N58
    N111 --> N58
    N112 --> N58
    N2 --> N59
    N118 --> N59
    N31 --> N60
    N97 --> N60
    N98 --> N60
    N31 --> N61
    N110 --> N61
    N31 --> N62
    N119 --> N62
    N113 --> N62
    N4 --> N63
    N63 --> N64
    N0 --> N64
    N34 --> N64
    N63 --> N65
    N58 --> N65
    N63 --> N66
    N55 --> N66
    N5 --> N67
    N123 --> N67
    N5 --> N68
    N132 --> N69
    N123 --> N70
    N53 --> N79
    N53 --> N80
    N48 --> N80
    N50 --> N81
    N80 --> N81
    N33 --> N81
    N50 --> N82
    N46 --> N82
    N50 --> N83
    N53 --> N83
    N109 --> N83
    N110 --> N83
    N50 --> N84
    N117 --> N84
    N3 --> N85
    N60 --> N85
    N61 --> N85
    N62 --> N85
    N141 --> N85
    N142 --> N85
    N143 --> N85
    N144 --> N85
    N145 --> N85
    N27 --> N85
    N28 --> N85
    N146 --> N85
    N147 --> N85
    N148 --> N85
    N64 --> N86
    N67 --> N86
    N68 --> N86
    N69 --> N86
    N70 --> N86
    N7 --> N86
    N65 --> N86
    N66 --> N86
    N12 --> N87
    N118 --> N88
    N120 --> N88
    N93 --> N88
    N94 --> N88
    N111 --> N88
    N103 --> N88
    N100 --> N88
    N98 --> N88
    N149 --> N88
    N99 --> N88
    N82 --> N88
    N112 --> N88
    N119 --> N88
    N83 --> N88
    N150 --> N88
    N106 --> N88
    N84 --> N88
    N107 --> N88
    N123 --> N88
    N116 --> N88
    N1 --> N88
    N52 --> N88
    N37 --> N88
    N49 --> N88
    N56 --> N88
    N36 --> N88
    N54 --> N88
    N51 --> N88
    N38 --> N88
    N47 --> N88
    N44 --> N88
    N135 --> N88
    N132 --> N88
    N131 --> N88
    N136 --> N88
    N137 --> N88
    N134 --> N88
    N79 --> N88
    N140 --> N88
    N138 --> N88
    N81 --> N88
    N45 --> N88
    N24 --> N89
    N0 --> N90
    N0 --> N91
    N0 --> N92
    N90 --> N93
    N92 --> N93
    N91 --> N94
    N0 --> N95
    N34 --> N95
    N0 --> N96
    N34 --> N96
    N0 --> N97
    N34 --> N97
    N96 --> N98
    N95 --> N99
    N97 --> N99
    N0 --> N100
    N0 --> N101
    N0 --> N102
    N101 --> N103
    N102 --> N103
    N0 --> N104
    N0 --> N105
    N0 --> N106
    N105 --> N107
    N0 --> N108
    N34 --> N108
    N39 --> N108
    N0 --> N109
    N34 --> N109
    N39 --> N109
    N0 --> N110
    N34 --> N110
    N39 --> N110
    N108 --> N111
    N109 --> N112
    N115 --> N113
    N114 --> N113
    N0 --> N114
    N122 --> N115
    N113 --> N116
    N0 --> N117
    N0 --> N118
    N117 --> N119
    N0 --> N120
    N0 --> N121
    N0 --> N122
    N121 --> N123
    N35 --> N124
    N35 --> N125
    N35 --> N126
    N35 --> N127
    N35 --> N128
    N35 --> N129
    N35 --> N130
    N124 --> N131
    N125 --> N131
    N126 --> N131
    N127 --> N131
    N128 --> N131
    N129 --> N131
    N130 --> N131
    N53 --> N132
    N0 --> N132
    N53 --> N133
    N0 --> N133
    N53 --> N134
    N139 --> N134
    N50 --> N135
    N53 --> N135
    N50 --> N136
    N133 --> N136
    N50 --> N137
    N53 --> N137
    N50 --> N138
    N53 --> N138
    N0 --> N139
    N0 --> N140
    N2 --> N141
    N90 --> N141
    N2 --> N142
    N93 --> N142
    N94 --> N142
    N25 --> N143
    N57 --> N143
    N95 --> N143
    N97 --> N143
    N25 --> N144
    N57 --> N144
    N25 --> N145
    N57 --> N145
    N26 --> N146
    N59 --> N146
    N84 --> N146
    N119 --> N146
    N26 --> N147
    N59 --> N147
    N26 --> N148
    N59 --> N148
    N120 --> N148
    N95 --> N149
    N96 --> N149
    N97 --> N149
    N108 --> N150
    N109 --> N150
    N110 --> N150
    N64 --> N150
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| D-01 REQ-13 scope (steward) | Chosen: the server shutdown test and the doctest substep are in scope. Early-relief terminals with no upstream edge are `profiled-fixture-build-lock` (the six-test timeout fix; `profiled-fixture-input-key` follows it), `rule-warning-tests-down-a-layer`, `server-shutdown-test-bound` and `doctest-substep-bound`. The before figures are the investigation's §A. |
| D-02 Fixtures first (steward D1) | Chosen: each fixture is a foundation, and the foundations are wave-1 roots with no edge: `cli-process-fixture`, `shared-taxonomy-repo`, `shared-fixture-issue`, `store-generic-initialisation`, `shipped-item-kinds-derivation` and `file-backed-setup-constructors`. `shared-memory-executor` and `memory-declaration-seeding` follow `store-generic-initialisation`, and adoptions follow their foundation, one per file family. Incoming edges are derived from footprint overlap: each 2195c98f retype task depends on every entry here that edits a file it retypes and reaches no external task, reduced transitively, including through the 2195c98f chain. A script checks that no such entry reaches 2195c98f, 842b0ac6 or df129d9e. Edges to add at breakdown: 0af95f14 → `command-list-output-types`, `doctest-substep-bound`, `env-reading-tests-guarded`, `fixture-issue-commands-c-adoption`, `fixture-issue-domain-artifact-adoption`, `fixture-issue-graph-validation-adoption`, `harness-demo-case-moves`, `issue-store-conformance-suite`, `library-batch-memory-setup-adoption`, `library-gate-claim-file-setup-adoption`, `library-store-validation-file-setup-adoption`, `query-domain-owner-gaps`; 13190465 → `auto-transition-harness-owner`, `fast-issue-fixture-issue-adoption`, `fast-issue-memory-setup-adoption`; b45a2865 → `fast-rules-file-setup-adoption`, `fast-rules-fixture-issue-adoption`, `fast-rules-shipped-item-kinds`, `label-query-harness-reduction`; b73b3e5a → `fast-docs-templates-file-setup-adoption`, `fast-docs-templates-memory-setup-adoption`, `harness-demo-deletion`; 19d9bd7d → `cli-query-graph-taxonomy-root-removal`, `cli-registry-render-spawn-migration`, `exit-code-reassertion-checkout`, `exit-code-reassertion-cli-query-graph`, `exit-code-reassertion-gate-status`; 5fac48b8 → `auto-transition-domain-decision`, `batch-cycle-adapter-reproof-removal`, `cli-query-graph-taxonomy-root-removal`, `fast-issue-fixture-issue-adoption`, `fast-rules-fixture-issue-adoption`, `fast-rules-memory-executor-adoption`, `harness-demo-deletion`, `library-fixture-issue-deletion`, `rule-warning-tests-down-a-layer`, `server-file-setup-adoption`. Edges implied by these are not listed; c797b59b and cd8adfc1 need no direct edge. 89e7a273 → `issue-store-conformance-suite` (D-03) is reached through 0af95f14. The only overlaps left as conflict-avoid are with `memory-backend-path-reads` and `backend-specific-test-pruning`, which sit behind the outgoing edge onto ec190bd5: 0af95f14 and 89e7a273. |
| D-03 Conformance suite owner (steward D2) | Chosen: `issue-store-conformance-suite` owns the suite. 89e7a273 reaches it through 0af95f14 → `issue-store-conformance-suite`, so no direct edge is added. Rejected: waiting for the in-memory fix, because the suite already passes both backends on the cases it carries. |
| D-04 df129d9e ordering (steward D4) | 650e6a56 (count structs as `ListEnvelope`) is done, so REQ-01 and REQ-04 both already follow it, and no edge is needed. Neither waits for the handler moves. `inline-list-output-types` and `command-list-output-types` type the sixteen fallback lists now, so the handler-move tasks find those lists already wrapped. |
| Outgoing edges | `memory-backend-path-reads` → ec190bd5 (ready; `memory.rs`). d125490a and 2492ab29 are done, so `harness-demo-deletion` and `query-available-domain-order` carry no external edge; their overlaps with 2195c98f files are incoming edges in the D-02 row. A live-graph check finds that none of the three external tasks reaches a 2195c98f retype task or c780514e, so no edge closes a cycle. |
| Conflict-avoid, no edge | 5687e281 (`main.rs`/`schema.rs`, also edited by `inline-list-output-types`, `command-list-output-types`, `fixture-setup-hook-removal` and `exit-code-binding-completeness`; 25 handler moves, and waiting would stall REQ-01, REQ-04 and REQ-11). 0e3b69fc (`Cargo.toml` test-support comment; backlog). bc1955a8 and b4fe7887 (`D-[0-9]+` literals; rule-compilation chain). b4929342 and 63c11d08 (`domain::types::fixture_issue` paths, which `shared-fixture-issue` removes). fe761000 (`auto` keys in `.jit/gates.toml`; new gates carry none). f9977efd and fb2dc5c3 (budget script and re-measurement; backlog). e3d06f82 and 3625b18b (event matchers in moved cases). afc2b698 (a different `scratch_build` file). |
| Stale external citations | c400730c and 2ac50581 cite `the removed exit-code subprocess suite`, `query_tests` and `harness_demo`, and cc6ef886 cites `tests/common/`. Their owners must re-point them once the deletions land. |
| Migration scope | Chosen: migrations skip files whose properties move to one owner (`the removed exit-code subprocess suite`, the parity suite, `list_envelope_tests.rs`, `query_tests.rs`). Owner entries delete or rewrite those files on the fixture. Rejected: migrating then deleting them (about 3,600 lines of churn). |
| `TaxonomyRepo` vs. process fixture | Chosen: one shared `TaxonomyRepo` that the process fixture can own. Rejected: folding taxonomy setup into the fixture root, which would merge a 17-file caller migration into the wave-1 foundation. |
| Doctest bound | Chosen: prune rustdoc examples that restate unit-tested properties (`@/inv/essential-code-prose`) until the substep meets the bound. Rejected: an edition-2024 bump for merged doctests, a workspace-wide migration outside this epic. |
| Server drain test | Chosen: a unit test of `run_shutdown_sequence` passes a short deadline (its existing parameter) to own the entitlement, and the process test uses the existing short-deadline seam. Rejected: a paused tokio clock, which needs the `test-util` dependency feature (`@/inv/bounded-rust-build-footprint`); shortening `GRACEFUL_DRAIN_TIMEOUT`, which changes product behaviour. |
| A test over 5 s at report time | `suite-clock-report`'s criterion fails if any test exceeds 5 s, so the report cannot pass over a regression. The execution lead adds a remediation terminal under `suite-cost` for each named test (owning the move down a layer, like the rule-warning and server cases), makes the report depend on it, and re-runs the report. The likeliest candidates are the two `profile_cli_tests` cases (3.66 s quiet). |
| Steward ruling: execution order | Waves 1–2 are on 2195c98f's critical path: the IssueId retype chain (0af95f14 onward) depends on the overlap-derived c780514e terminals in the external-edge table; the steward assigns c780514e execution with waves 1–2 first. |
| Steward ruling D-05: gate bootstrap | The ten self-test gate keys are registered in .jit/gates.toml at planning; each gate task and the container c780514e require them; the container requirement is added at breakdown with `gate add`. |
| D-06 Sizing rule (steward ruling, 2026-10-22) | Sizing rule (steward ruling, 2026-10-22): a terminal touches at most about 6 files, or one file family with one concern; leaves over 6 files name that family and concern in their sizing reason. Applied: the CLI migrations split per file family and per concern (spawn versus id parsing); the library `fixture_issue`, memory, file and git adoptions split per module family, with the production-fixture deletion as its own task after all of them; the exit-code re-assertion sweeps split per file family. The fixture's build lock and its cache key are separate leaves, and the init/profile setups are separate from the storage, validation and CLI ones. Each suite-root `TaxonomyRepo` copy is removed by its own leaf after every caller has moved, and a cross-file setup helper retires only after the sibling leaves that call it. Four leaves stay above 6 files, each naming its single family and concern: `profiled-fixture-input-key` (cache key), `failure-corpus-in-suite`, `doctest-substep-bound` and `command-list-output-types`. |
| Measurement ordering | Chosen: `build-footprint-report` depends on the four other stories and the remaining suite-cost terminals, so "after" covers all changes. Risk: the stories' holistic reviews then sit on the critical path to the report. |
| Same-file ordering | Every cut task (exit-code re-assertions and case moves, wiring reductions, re-proof removals and deletions, list-envelope re-assertion removals, the binding-completeness and coverage tests) depends on every migration or adoption that shares a file with it, and no migration or adoption depends on a cut; a generator pass derives these edges from the footprints and a check confirms no shared-file pair is unordered. `exit-code-dependency-guard-case-moves` → `label-query-cli-wiring-reduction` → `exit-code-reassertion-cli-query-graph` serialises `query_json_tests.rs` and `label_query_json_tests.rs`; the wiring cut keeps the moved parent-filter guard. `fast-rules-memory-executor-adoption` → `fast-rules-memory-setup-adoption` → `label-query-harness-reduction` and `fast-rules-shipped-item-kinds` serialise the `fast_rules` executor files. The rule-warning move precedes `fast-rules-file-setup-adoption`, and `cli-query-graph-fixture-issue-adoption` precedes the `cli_query_graph` spawn migrations (`main.rs`). Query reductions depend on `query-domain-owner-gaps`, so each cut lands with its domain owner already present. |
| Shared-file contention | `test_utils.rs`, `dev/TESTING.md`, `.jit/gates.toml`, `.config/nextest.toml`, `repository_state_store_tests.rs` and `commands/issue.rs` are edited by several entries. Edges serialise same-region rewrites: query extraction before the auto-transition decision, finalization deletion before the conformance suite, hook removal before the corpus fixture, and migrations before the re-assertion and wiring cuts in their files. Workers rebase over the remaining advisories. |

## Investigation sources

- [Investigation](investigation.md) holds the claim classification, file and consumer inventories (§1, §3), primitive checks (§4), suite-clock profile (§A), upstream mapping (§B) and test layout (§C).

The approved `inline-list-output-types` footprint includes `gate_presets.rs` and `storage/claim_coordinator.rs` for `JsonSchema` derives and required imports on the canonical `PresetInfo` and `Lease` types. This avoids duplicate item representations; runtime behavior, serde behavior, criteria, and gates remain unchanged. Further changes to fields, dependencies, features, or schema semantics require a separate assessment.
