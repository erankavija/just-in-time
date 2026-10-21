# Investigation: c780514e test topology

Verified against main at `71ed9d4ce` (2026-10-21); the per-test timing run used `3f7ce9165`. The container's Background was written at `79c177fbd`, 421 commits earlier. Paths are relative to the repository root.

## Summary

| classification | claims |
| --- | ---: |
| already-done | 3 |
| valid-and-open | 19 |
| invalid-as-stated (drifted count, location, or scope) | 12 |
| unverifiable | 1 |

What changed since the Background:

- REQ-09 is mostly done (`6ff14f21`).
- The harness moved to `crates/jit/src/test_harness.rs`, and `crates/jit/tests/common/` no longer exists (`ef9897ab`).
- Three cross-backend `IssueStore` reader tests now exist, though no shared suite does.
- A domain `ListEnvelope` type owns `count == len` for the one response that uses it.

The Background misses six facts:

- a third nextest setup script, which runs a nested `cargo test` and is filtered by individual test names;
- 16 more tool-missing `SKIP` passes;
- two more `CANONICAL_ITEM_KINDS` copies;
- an unbounded fixture cache (1,341 entries, 747 MB);
- product init is implemented only for `JsonFileStorage`;
- the `query available` sort lives in commands, not the domain.

## 1. Claim classification

### Duplicated properties

| # | Claim | Verdict | Evidence |
| --- | --- | --- | --- |
| 1 | The projection suite has 32 tests, binds both forms, and checks completeness at `:1128`. | valid-and-open | 32 `#[test]`. `assert_projected_exit_status_in_both_forms` at `crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs:156-187`. `test_command_exit_codes_every_row_is_verified` at `:1128`. Caveat in §4. |
| 2 | `exit_code_tests.rs` has 39 tests and 1,637 lines and re-proves codes (`:36`/`:774`, `:69`/`:818`, `:1566`/`:966`). | valid-and-open | 39 tests, 1,637 lines. All cited line pairs match. 14 tests are re-proofs, and 25 own behaviour that must move: blocked-transition envelopes, claim behaviour, confirmed-delete output, id-prefix errors, argument guards, and startup envelopes. §D.1 classifies every test. Other subprocess suites also assert projected exit codes (§D.2). |
| 3 | `invocation_exit_status_parity_tests.rs` re-proves parity. | valid-and-open | 5 tests in `crates/jit/tests/cli_issue/invocation_exit_status_parity_tests.rs`. Two cases have no projection row: unresolved ids across namespaces (`:65`) and a malformed stored record (`:133`). Move them, do not drop them. |
| 4 | `domain_queries_tests.rs` owns pure filters; `query_tests.rs` has 25 subprocess tests and 1,422 lines. | valid-and-open | `crates/jit/tests/fast_issue/domain_queries_tests.rs` has 10 tests. `crates/jit/src/domain/queries.rs` has 21 more unit tests. `crates/jit/tests/cli_query_graph/query_tests.rs` has 25 tests and 1,422 lines. Label and strategic query re-proofs outside it are in §D.3. Gap: `query available` composes its filters and priority sort in `crates/jit/src/commands/query.rs:122-148`, and `crates/jit/src/commands/issue.rs:800` repeats the sort. That property has no domain owner yet. |
| 5 | `harness_demo.rs` has 526 lines and 14 tests and re-proves queries, blocking, cycles, and gates. | valid-and-open | 527 lines, 14 tests. Six cases have no other owner and must move: `:291` item list/resolve, `:314` kind/label coverage, `:377` container progress, `:442` status projection, `:462` empty rollup, and `:475` archive candidates. |
| 6 | `workflow_tests.rs:529-743` re-proves auto-transitions owned by `types.rs:2427`. | invalid-as-stated (location) | The tests are at `crates/jit/tests/cli_repo_workflow/workflow_tests.rs:529,577,646,698`. The domain predicates are tested at `crates/jit/src/domain/types.rs:2546-2703`, not `:2427`. The cascade to dependents (`commands/issue.rs:816-859`, `commands/mod.rs:2791`) is command-layer behaviour. Its harness-level owners include `commands/bulk_update.rs:2001` and `tests/fast_issue/lifecycle_timestamps_tests.rs:89`. "Proven only at the domain" needs a domain extraction or a single harness owner. |
| 7 | `batch_create.rs:686-711` repeats `graph/keyed.rs` tests over an adapter. | valid-and-open | `detect_cycle` is at `crates/jit/src/commands/batch_create.rs:640`, with four tests at `:686-707`. The owner `crates/jit/src/graph/keyed.rs:145-195` includes a proptest at `:344`. |
| 8 | `harness_demo.rs:133`, `integration_test.rs:616`, and `exit_code_tests.rs:69` re-prove cycle rejection. | valid-and-open | All three cited lines match. §D.3 classifies three more cycle tests; each owns a different property. |
| 9 | About 800 lines at `repository_state_store_tests.rs:1735-2555` re-prove finalization. | invalid-as-stated (location) | The section is now `crates/jit/src/storage/repository_state_store_tests.rs:1754-2585` ("REQ-01: typed mutation determinism"). Its own comment (`:1756-1760`) says the finalizer is a pure function of the image, which confirms the re-proof. The file has 3,810 lines. |
| 10 | 19 tests in `list_envelope_tests.rs` assert `count == len`, again at `search_tests.rs:236`, `artifact_conformance_cli_tests.rs:647`, and `profile_cli_tests.rs:1940`. | invalid-as-stated (locations, scope) | 19 tests confirmed. Re-assertions now sit at `cli_query_graph/search_tests.rs:236`, `cli_repo_workflow/artifact_conformance_cli_tests.rs:~640`, `profile_cli_tests.rs:1695` (`assert_count_envelope`, not `:1940`), `worktree_cli_tests.rs:546,1036`, `container_rollup_tests.rs:211,348`, and `cli_issue/issue_search_json_tests.rs:234`. New since `79c177fbd`: `crates/jit/src/domain/list_envelope.rs` derives `count` and owns it in unit tests (`:261`, `:305`). Only `ProfileCollection` uses it (`crates/jit/src/profile/application.rs:29`). Since 650e6a56, the output module's count structs are `ListEnvelope` instantiations. Sixteen list-emitting commands still have no typed output schema (§D.4, derived from emission sites), so a schema-driven enumeration misses them. |

### Fixtures and contracts

| # | Claim | Verdict | Evidence |
| --- | --- | --- | --- |
| 11 | No shared subprocess fixture exists. | valid-and-open | None in `crates/jit/src/test_utils.rs`, `test_harness.rs`, or suite roots. `cli_issue/failure_probe_fixture.rs` is suite-local. |
| 12 | `fn jit_binary` is defined in 56 files. | valid-and-open | 56 files, all under `crates/jit/tests/`. |
| 13 | 42 files use `cargo_bin!`; 68 use `env!("CARGO_BIN_EXE_jit")`. | invalid-as-stated (count) | Now 43 and 69 in tests, plus one doc mention at `test_utils.rs:225`. |
| 14 | 57 files define `setup_test_repo`/`setup_repo`. | invalid-as-stated (scope) | 56 test files plus `test_utils.rs:918`. These are not the only private setups: other names add 33 more definitions, including `setup` ×6, `setup_test_env` ×3, `setup_repository` ×3, `setup_repo_with_issue` ×3, `setup_git_repo`, and `setup_initialized_repo`. The in-process ones outside the CLI targets are listed in §D.6. |
| 15 | 112 `split_whitespace` sites scrape ids. | invalid-as-stated (count) | 114 sites in 39 test files. Not every site scrapes ids; `scratch_build/build_profile_policy_tests.rs:132` parses a script. |
| 16 | 17 files hand-roll `git init`. | invalid-as-stated (count) | 20 test files spawn `git init`, plus 8 `src` test modules. Private helpers include `run_git`, `git`, `init_git`, `setup_git_repo`, and `git_repository_with_jit` (for example `cli_repo_workflow/lock_contention_tests.rs:31`). |
| 17 | No file removes inherited `JIT_*` variables. | invalid-as-stated | Several files remove chosen variables: `steering_scenarios.rs:143-146` (and pins `JIT_AGENT_ID`), `repo_discovery_tests.rs:23-25`, `profile_acceptance_tests.rs:64-65`, `project_config_tests.rs:144`, and `failure_probe_fixture.rs:566`. The substance holds: no shared fixture scrubs all `JIT_*`. The product reads `JIT_AGENT_ID` (10 sites), `JIT_DATA_DIR`, `JIT_WORKTREE_*`, `JIT_ALLOW_DELETION`, `JIT_LOCK_TIMEOUT`, and others, so most spawns inherit them. |
| 18 | `fixture_issue` has 5 copies; `TaxonomyRepo` 2; `memory_executor` 2 with different config and index seeding. | valid-and-open | `fixture_issue`: `domain/types.rs:546`, `tests/fast_issue/main.rs:22`, `fast_docs_templates/main.rs:33`, `cli_query_graph/main.rs:68`, `fast_rules/main.rs:54` (499 call sites). `TaxonomyRepo`: `cli_issue/main.rs:50`, `cli_query_graph/main.rs:27`. `memory_executor`: `commands/test_helpers.rs:186` seeds `enforce_leases = "off"`; `fast_rules/main.rs:21` seeds an empty config and a literal `"schema_version": 2` index (`:44`). |
| 19 | Fixture code lives in `test_utils`, `test_helpers`, `test_taxonomy`, `tests/common/harness.rs`, and suite `main.rs` files. | invalid-as-stated (location) | `tests/common/` is gone. The harness is `crates/jit/src/test_harness.rs` (`fed2a4f2c`). Also `crates/jit/src/storage/test_support.rs` (`seed_repository_tree_fixture`, `:72`). |
| 20 | `CANONICAL_ITEM_KINDS` in `tests/common/harness.rs:59-100` restates the manifest. | invalid-as-stated (location, count) | Three copies: `crates/jit/src/test_harness.rs:56-100`, `crates/jit/src/commands/item.rs:570`, `crates/jit/src/commands/validate.rs:2975`. `crates/jit/src/test_taxonomy.rs:225` (`SUITE_TABLES`) is a fourth restatement. The source is `profiles/jit-default/manifest.toml`, `[[contribution]] target = "item-kinds"` (`:217-228`). |
| 21 | `D-[0-9]+` appears 14 times. | valid-and-open | 14 in `crates/`, across `test_harness.rs`, `commands/validate.rs`, `commands/item.rs`, `domain/item.rs`, `cli_item_validate/decision_kind_tests.rs`, `decision_risk_story_tests.rs`, and `fast_rules/project_render_harness_tests.rs`. |
| 22 | `IssueStore` has two implementations at `json.rs:1045` and `memory.rs:820`. | valid-and-open (lines drift) | `crates/jit/src/storage/json.rs:1042`, `crates/jit/src/storage/memory.rs:814`. |
| 23 | No shared `IssueStore` conformance suite; a cross-backend matrix exists only for `RepositoryStateStore` (`:660`). | invalid-as-stated | No shared suite exists. Three ad-hoc cross-backend reader tests now do: `repository_state_store_tests.rs:220` (`load_issue`/`list_issues`/`resolve_issue_id`), `:278` (id resolution, `16d63038`), and `:691` (event log, `842b0ac6`/`47a082ef`). The matrix header is at `:644`. `read_issues` is gone (`f9add0dd8`). The trait has 21 methods (`crates/jit/src/storage/mod.rs`). |
| 24 | `InMemoryStorage::read_path_bytes` reads real disk and ignores `at_commit` (`memory.rs:987-1022`). | valid-and-open | Now `crates/jit/src/storage/memory.rs:943-990`: `_at_commit` is unused and the read is `std::fs::read(root_path.join(path))`. |
| 25 | Env changes in `config.rs`, `serve.rs`, and `server/src/lib.rs` run without a shared guard; `EnvironmentRestore` is private. | already-done | `EnvGuard` (`crates/jit/src/test_utils.rs:85-131`) and `CurrentDirGuard` (`:50-77`) are the only mutators. Users: `config.rs`, `serve.rs`, `crates/server/src/lib.rs`, `storage/json.rs`, `commands/hooks.rs`, `commands/claim.rs`. `EnvironmentRestore` is gone. The policy test is `crates/jit/tests/scratch_build/process_state_policy_tests.rs`, and no `set_var`/`remove_var`/`set_current_dir` call remains outside `test_utils.rs`. |
| 26 | `merged_tree_gate_verification_tests.rs:47-53` prints `SKIP:` and passes when the self-test exits 2. | valid-and-open | `:48-53`. Also unlisted: `scratch_build/rust_build_budget_checker_tests.rs` has 14 `SKIP: jq not on PATH` passes (`:175`…`:521`), `scratch_build/dependency_feature_policy_tests.rs:168,185` skips without cargo, and `cli_item_validate/validate_cli_rule_tests.rs:930-935` skips without node. §D.5 covers the full sweep. |
| 27 | `cargo-ci-selftest.sh` (622 lines) runs nested cargo builds of six fixture crates inside the default suite. | valid-and-open | 622 lines. `cargo build` at `scripts/cargo-ci-selftest.sh:501`, gate runs at `:486`, scenarios at `:606-612`. It takes 3.99 s quiet (§A). |
| 28 | `build_profile_policy_tests.rs:332` runs `profile-test-suite.sh --self-test`. | valid-and-open | `crates/jit/tests/scratch_build/build_profile_policy_tests.rs:332-345`. Unlisted: `cli_issue/failure_probe_fixture.rs:1060` runs `scripts/setup-recorded-failure-corpus.sh --self-test`, and `rust_build_budget_checker_tests.rs:145` drives `rust-build-budget.sh`. No gate in `.jit/gates.toml` runs a script self-test. |
| 29 | `test_utils.rs:78-860` keys the cache on SHA-256 of the test and `jit` executables. | valid-and-open (lines drift) | Fixture code is `crates/jit/src/test_utils.rs:133-916`; key at `:588-634`. It hashes the setup executable (`current_exe`) and an optional scenario executable. Each test binary therefore gets its own key, and every rebuild invalidates every key. Unlisted: entries are never collected. `target/jit-profiled-repository-fixtures` holds 1,341 entries (714 run receipts) and 747 MB. |
| 30 | A nextest receipt path (`:336-470`) skips hashing. | valid-and-open | Now `:393-524`. The setup still hashes the `jit` binary (`:404-410`); consumers skip it. |
| 31 | A hidden `__test-fixture-setup` subcommand is compiled into the product binary (`main.rs:2357`). | valid-and-open (line drift) | `crates/jit/src/main.rs:1787-1793`, behind `cfg(feature = "test-support")`. The self dev-dependency (`crates/jit/Cargo.toml:106`) turns that feature on for every test build of the binary. |
| 32 | `.config/nextest.toml` lists test names by regex. | invalid-as-stated (scope) | Three setup scripts, not two. `default-repository-fixture` filters 5 module prefixes and 1 named test. `dogfood-repository-fixture` filters 1 module and 8 named tests. `recorded-failure-corpus` filters 3 named tests and runs `scripts/setup-recorded-failure-corpus.sh`, which runs a nested `cargo test --workspace --test cli_issue` (`:33`) inside the suite run. |
| 33 | Debug `jit` is 141 MB, hashing takes about 0.24 s, and a profiled repository builds in about 0.13 s. | unverifiable as a current measurement | `target/debug/jit` is 139,497,912 B now. The quiet-run setups took 0.111/0.136 s including a hash of that binary (§A). Hashing alone was not isolated. |
| 34 | 10 targets against a limit of 12; 1.62 GiB across 14 executables against 2 GiB. | already-done (still accurate) | The last gate run (`8e22dd96`, `0896fc29b`) reports `integration-targets=10/12 active-executables=14 bytes=1723670208/2147483648` (1.61 GiB). |
| 35 | Each suite executable is 83–136 MB. | already-done (still accurate) | `target/debug/deps`: scratch_build 82.4 MB … fast_docs_templates 134.3 MB, server_integration 131.5 MB. The `jit` lib-test executable is 182.5 MB and is not in that range. |

## 2. Prior-art sweep

- `dev/archive/4b7c06d0-test-suite-performance/active/plan.md` (Archived): origin of the `suite-clock`, `nextest-reporter-evidence`, and `suite-timing-evidence` contracts.
- `dev/benchmarks/rust-build-budgets/README.md` (`0708d692`): derives the three limits. Warm clocks were 24,171/22,437/21,841 ms. The first run after clearing the fixture cache was 29,726 ms. That 5.5–8 s difference is the cold-key cost REQ-11 addresses.
- `dev/benchmarks/applied-profile-baselines-ae17798d/` (`ae17798d`, Done): introduced the nextest setup recipes and their named-test filters.
- `dev/benchmarks/ephemeral-nextest-receipts-4ee95d17/` (Rejected, `no_change`): measured setup scripts summing to about 10.6 s at the time. Today a warm cache takes 0.54 s for all three.
- `dev/benchmarks/nextest-concurrency-934985ca/` (`no_change`; 48 workers caused shared-lock failures) and `dev/benchmarks/test-profile-123efe74/` (opt-level decision).
- `dev/benchmarks/suite-profile.json` plus `scripts/profile-test-suite.sh` produce per-test timings (libtest-json-plus).
- `dev/TESTING.md:86-166,562,615-619,716,747` documents `harness_demo`, `with_item_kinds`, and `query_tests` as worked examples. It goes stale with REQ-02 and REQ-07.
- `dev/active/df129d9e-rust-code-quality/plan.md:66` covers `ListEnvelope` scope, and `dev/active/df129d9e-rust-code-quality/investigation.md:733` covers the `__test-fixture-setup` hook.

## 3. Consumer sweep

Live consumers only: crates, scripts, `.config`, `.jit/gates.toml`, `dev/TESTING.md`, and docs. `dev/archive/**`, benchmark `raw/` logs, and test inventories are historical and need no edit.

| Removal or migration | Consumers |
| --- | --- |
| `exit_code_tests.rs` | `crates/jit/tests/cli_issue/main.rs` (`mod`). |
| `invocation_exit_status_parity_tests.rs` | `crates/jit/tests/cli_issue/main.rs`. |
| `query_tests.rs` | `crates/jit/tests/cli_query_graph/main.rs`; `crates/jit/tests/fast_rules/main.rs` (mention); `dev/TESTING.md:619`. |
| `harness_demo.rs` | `crates/jit/tests/fast_docs_templates/main.rs`; `dev/TESTING.md:86,161,166,562,615,716,747`. |
| `TestHarness` | 15 files: `test_harness.rs`; `fast_docs_templates/{batch_export,document_event_log,harness_demo,template_apply}_tests.rs`; `fast_issue/{archived_semantics,bulk_operations,dep_add_atomic,dependency_met,lifecycle_timestamps,readiness_coherence,short_hash,state_transition_fix,transitive_reduction_validation}_tests.rs` and `test_no_coordinator.rs`; `dev/architecture/core-system-design.md`. |
| `with_item_kinds` / `CANONICAL_ITEM_KINDS` | `crates/jit/src/test_harness.rs`, `crates/jit/src/commands/item.rs:570-1228`, `crates/jit/src/commands/validate.rs:2975-3115`, `fast_docs_templates/harness_demo.rs`, `dev/TESTING.md:90,104,128,134`. |
| Local binary locators | `fn jit_binary` in 56 files; `cargo_bin!` in 43; `CARGO_BIN_EXE_jit` in 69. |
| Local repo setups | 56 test files plus 33 differently named setups (§1 #14). |
| Id scraping | 114 `split_whitespace` sites in 39 files. |
| `fixture_issue` | `domain/types.rs:546` (`pub(crate)`) plus 4 suite copies; about 499 calls across `src/commands/*`, `src/domain/*`, `src/graph/*`, and the suites. |
| `memory_executor` | `commands/test_helpers.rs:186`; its users `commands/{batch_create,bulk_update,dependency,gate_check,gate,issue,mod,validate}.rs` and `test_harness.rs`; `fast_rules/main.rs:21` plus `fast_rules/{example_rulesets,label_query,label_strategic,validate_rule_runner,validation}_tests.rs` and `fast_issue/graph_rule_validation_tests.rs`. |
| `TaxonomyRepo` | `cli_issue/main.rs` plus `batch_create_tests.rs`, `gate_field_contract_test.rs`, `issue_create_positional_type_tests.rs`; `cli_query_graph/main.rs` plus 11 files. |
| `__test-fixture-setup` and receipts | `crates/jit/src/main.rs:1787`; `crates/jit/src/test_utils.rs:160-161,393-524,1675-1830` (receipt tests); `.config/nextest.toml:5,17`; `crates/jit/tests/fast_docs_templates/template_apply_tests.rs` and `crates/jit/tests/scratch_build/build_profile_policy_tests.rs` (nextest config reads); `dev/TESTING.md`. |
| Nextest name lists and `setup-recorded-failure-corpus.sh` | `.config/nextest.toml:9-37`; `scripts/setup-recorded-failure-corpus.sh`; `crates/jit/tests/cli_issue/failure_probe_fixture.rs:1060-1075` (self-test plus a nextest config parse); `scripts/cargo-ci.sh` (pinned version); `dev/TESTING.md`. |
| Nested cargo builds and script self-tests | `scratch_build/merged_tree_gate_verification_tests.rs` → `scripts/cargo-ci-selftest.sh`; `scratch_build/build_profile_policy_tests.rs:332` → `scripts/profile-test-suite.sh`; `cli_issue/failure_probe_fixture.rs:1060` → `setup-recorded-failure-corpus.sh`; `scratch_build/rust_build_budget_checker_tests.rs` → `scripts/rust-build-budget.sh`; `CHANGELOG.md:42,76`; `dev/TESTING.md:195,384`. `.jit/gates.toml` declares no script-self-test gate. |

## 4. Primitive verification

- **Exit-code projection suite.** Both forms: yes; `assert_projected_exit_status_in_both_forms` asserts the plain status, the JSON status, and their equality (`command_exit_code_projection_tests.rs:156-187`). Completeness: partial. `test_command_exit_codes_every_row_is_verified` (`:1128-1185`) compares the projection to a hand-maintained `verified` set. It does not check that each listed row has a binding test, so a row can be listed with no test behind it.
- **Profile package bytes as a cache key.** Available on disk. `capture_profile_package_closure` (`test_utils.rs:561-586`) already captures the dependency closure with path, bytes, and mode, and `profiled_repository_fixture_key` hashes it (`:621-632`). Packages total 47,743 B under `profiles/`. No crate-source digest exists: `crates/jit/build.rs` emits only `JIT_BUILD_PROFILE`/`JIT_BUILD_TARGET`, and its comment says it reads nothing ambient. `crates/jit/src` is 7.4 MB in 188 `.rs` files.
- **Product init seeding a memory executor.** Not available directly. `initialize_fresh_repository` and `run_initialization` exist only on `impl CommandExecutor<JsonFileStorage>` (`crates/jit/src/commands/init.rs:73-215`), although they go through the generic `with_mutation_session`. The current route is to initialize on disk, then `InMemoryStorage::seed_repository_tree_fixture` (`storage/test_support.rs:72`, used at `test_utils.rs:258-268`). A pure starting point for the minimal config exists: `repository_state::initialize::structural_minimum_config` (`crates/jit/src/repository_state/initialize.rs:64`).
- **`EnvGuard` / `CurrentDirGuard`.** Both serialise on separate static mutexes, recover after poisoning, and restore in `Drop`. `EnvGuard` restores in reverse order (`test_utils.rs:120-131`). Limits: guards do not nest (`:83`), and code that reads the environment without holding the guard is not serialised. `PROFILED_REPOSITORY_FIXTURE_ENV_LOCK` (`:42`, `cfg(test)`) is a second, fixture-specific lock taken next to `EnvGuard` (`:234`, `:1675-1819`).
- **`ListEnvelope`.** `crates/jit/src/domain/list_envelope.rs` derives `count` on serialize and rejects a mismatch on deserialize (`:305`), so the type owns the property by construction. One response uses it.
- **Test-support gating.** `test_harness`, `test_utils`, and `test_taxonomy` are compiled only under `cfg(any(test, feature = "test-support"))` (`lib.rs:35`, `test_utils.rs:5`, `test_taxonomy.rs:25`). `test_mode` is unconditional.

## 5. Architecture fit

- Reuse:
  - `EnvGuard` and `CurrentDirGuard` for REQ-09.
  - `ListEnvelope` for REQ-04: make every list response an instantiation, then one schema- or registry-driven test covers every list-emitting command.
  - `capture_profile_package_closure` for the REQ-11 key.
  - `seed_repository_tree_fixture` or a generalised `run_initialization<S: RepositoryStateStore>` for REQ-06.
  - `CommandSchema::generate().command_exit_codes` for REQ-01.
  - The `repository_state_store_tests.rs` cross-backend pattern (`:220`, `:278`, `:691`) as the start of the REQ-08 suite.
- Layering:
  - Query composition and priority ordering (`commands/query.rs:122-148`, `commands/issue.rs:800`) belong in `domain/queries.rs` before CLI tests can shrink to wiring, per REQ-02.
  - The auto-transition cascade is command-layer behaviour. Give it one harness owner.
- The shared subprocess fixture (REQ-05) belongs in the test-support surface (`crate::test_utils` or a sibling module under `cfg(feature = "test-support")`), following `ef9897ab`'s single compilation of shared harness code.

## 6. Architectural-invariant check

Each invariant was resolved with `jit item show`.

- `@/inv/orthogonal-tests`: violated by claims 2–10. REQ-09's half ("passes alone, in any order, in parallel") is enforced for env and cwd by `process_state_policy_tests.rs`. The receipt env lock and fixture-cache lock contention (§A) remain order- and load-sensitive.
- `@/inv/shared-test-contracts`: violated by #11–#24. No `IssueStore` suite exists, and `memory_executor`, `fixture_issue`, and `TaxonomyRepo` each have parallel copies.
- `@/inv/no-ignored-tests`: `cargo-ci` reports 0 ignored, but 18 tool-missing skip paths pass silently (#26, §D.5). These are held-out tests in substance.
- `@/inv/bounded-rust-build-footprint`: passes at 10/12 targets and 1.61/2 GiB. The fixture cache's 747 MB in `target/` is unbudgeted. A new integration target for a shared fixture is not needed; the test-support surface already compiles once.
- `@/inv/domain-agnostic`: the `CANONICAL_ITEM_KINDS` copies and `D-[0-9]+` literals are test-side copies of package vocabulary. REQ-07's derivation from `profiles/jit-default` must stay in test support and out of engine code.

## A. Suite-clock profile

Passing `cargo-ci` runs on main, 2026-10-21, from `.jit/gate-runs/*/result.json`:

- No rebuild before the run (`suite-build: built in ~0.5 s`, 15 runs): suite clock 20,731–22,946 ms, median about 21.1 s.
- After a 25–47 s rebuild (19 runs): 21,174–29,597 ms, mostly 24.4–29.6 s.
- Latest warm run (`8e22dd96`, `0896fc29b`): `test` 16,657 ms + `doctest` 4,436 ms = 21,111 ms.

The records carry no per-test lines, so one focused run was made under the flock wrapper at `3f7ce9165`. The 1-minute load average was 8.8 before and 10.2 after, on 24 cores. That run took 17,012 ms for 4,659 tests, with summed test time of 284.2 s. Setups took 0.111, 0.136, and 0.294 s (warm cache).

The failed-run stdout from the last 7 days shows `TIMEOUT 20 s` repeatedly for the four `jit::fast_rules cli_warnings_integration_tests::*` tests and the two `jit test_utils::tests::test_profiled_*` tests. They finish last (`4660/4665`). In the quiet warm run each took 0.22–0.24 s. Their cost is the cold fixture key after a rebuild, made worse by host load.

| REQ | Tests (quiet warm) | Summed time | Wall relief estimate |
| --- | --- | ---: | --- |
| REQ-01 | `exit_code_tests` ×39 plus parity ×5 | 3.25 s | about 0.15 s (CPU ÷ 24) |
| REQ-02 | `query_tests` ×25, `harness_demo` ×14, workflow auto ×4, cycle ×5 | 5.53 s | about 0.2 s |
| REQ-03 | finalization ×9 | 0.09 s | 0 |
| REQ-04 | `list_envelope_tests` ×19 | 3.38 s | about 0.1 s, if replaced by one in-process or schema-driven test |
| REQ-10 | `merged_tree…selftest` 3.99 s; budget checker ×14 0.91 s; `profile-test-suite` self-test 0.18 s; corpus setup self-test inside `failure_probe_fixture` | 5.1 s | Removes a 4 s long pole. Wall gain is capped by `server_integration graceful_shutdown…force_closes` (5.32 s, outside every REQ), which ends the run. |
| REQ-11 | 87 fixture-consumer tests (9.3 s), 3 setup scripts (0.54 s warm) | — | Warm: about 0.5 s. After a rebuild: the main lever. Each distinct setup executable gets a new key on every rebuild, and the README measured 5.5–8 s cold-key cost. Keying on package bytes plus crate source rebuilds each profile once per source change, not once per test binary. |
| REQ-05 | 1,182 CLI subprocess tests, 183.8 s summed | — | Not quantified. Reusing an initialized repository instead of spawning `jit init` per test cuts CPU, not the long-pole tail. |

The relief above is not enough on its own. The tail is the 5.32 s server test, `merged_tree…` at 3.99 s, and two `profile_cli_tests` at 3.66 s, all starting late. The doctest substep (4.4 s, 61 doctests; edition 2021 compiles each separately) is 21% of the clock and falls under no REQ. The remaining warm headroom is about 9 s against the 30,000 ms budget, and post-rebuild runs reach 29.6 s.

## B. Upstream mapping

Upstream state:

- **2195c98f** (planning lead assigned): 6 top-level children and 107 non-done transitive tasks, all `backlog`. Nothing in it is executing.
- **842b0ac6** (execution lead): 22 tasks done, 11 `ready`, 9 `backlog`. Its only direct dependency is `63c11d08`.
- **df129d9e** shares several tasks with 842b0ac6. Its handler-move tasks are all `backlog`.

`c780514e` itself depends only on its breakdown node `3d7d8ab2`. The 2195c98f edge was lifted in `0886d4a64`, "for re-homing".

| c780514e area | Upstream task (state, unmet deps) | Relation | What an edge into it delays |
| --- | --- | --- | --- |
| REQ-01 delete `exit_code_tests`, parity | `19d9bd7d` CLI suites carry typed ids (backlog; 0/1 → `0af95f14` → `3e353abd` 1/2) | Do c780514e first. Deleting re-proofs shrinks 19d9bd7d's surface. | The whole IssueId chain, more than 5 hops of backlog. |
| REQ-01 projection suite | `5687e281` reduce `main.rs` (backlog; 0/25) | Avoid conflict. Its REQ-07 rewrites `schema.rs` exit-code rows, so the projection test is edited there too. | 25 handler-move tasks; do not wait. |
| REQ-02 delete `harness_demo`, move unique cases | `d125490a` test gates via `define_gate` (**ready**, 842b0ac6 lead) edits `harness_demo.rs` and `test_harness.rs:209-220`; `fe761000` removes `auto` (1/2 → d125490a) | Let 842b0ac6 land first, or have it drop its `harness_demo` edits. | Little: d125490a is ready. |
| REQ-02 query owners | `2492ab29` remove lifecycle backfill (**ready**) deletes `derive_lifecycle_timestamps` tests in `domain/queries.rs` and `integration_test.rs:431`; `0af95f14` rewrites `domain/queries.rs` test ids (27 lines) | 2492ab29 first. Avoid conflicting with 0af95f14. | 2492ab29: none. 0af95f14: blocked by `3e353abd`. |
| REQ-03 finalization tests | `641bd14e` mutations carry typed ids, `89e7a273` storage trait IssueId (0/3), `0af95f14` (`repository_state_store_tests.rs` ids, 6 lines) | c780514e first. Deleting about 830 lines shrinks the retype. | — |
| REQ-04 list envelope | `650e6a56` count structs as `ListEnvelope` (**ready**, df129d9e lead, cargo-ci passed 15:59). Its REQ-02 requires `list_envelope_tests.rs` to pass "without assertion changes". Handler-move tasks wrap the remaining lists later. | Consume 650e6a56 first: edge REQ-04 onto it. | Short: 650e6a56 is ready and gated. |
| REQ-05 subprocess fixture, id scraping | `19d9bd7d` (CLI suites; ids stay strings at the process boundary, REQ-02) | Avoid conflict. Both edit most `cli_*` files. Ordering is a decision: fixture first lets 19d9bd7d retype one fixture. | IssueId chain. |
| REQ-06 `fixture_issue`, `TaxonomyRepo`, `memory_executor` | `0af95f14` adds a canonical-id fixture to `test_utils`. `13190465`/`b45a2865`/`b73b3e5a` retype the `fast_issue`/`fast_rules`/`fast_docs_templates` suites, which hold the `fixture_issue` copies. `c797b59b` retypes `test_harness.rs` (0/4). `b4929342` (13/26) and `63c11d08` remove `domain::types::` paths, including the 333 in `fixture_issue`. `d125490a` changes the harness gate helper. | Heaviest overlap. Either consume them first or land REQ-06 first and let them retype one definition. | 0af95f14 and the suite tasks are behind `3e353abd`; b4929342 needs 13 more deps. |
| REQ-07 item kinds | `bc1955a8` item kinds hold `IdPattern` (0/1 → `b48d7b2a`); `b4fe7887` item-kind scope enum | Avoid conflict on `D-[0-9]+` literals in `domain/item.rs` and `commands/item.rs`. | Rule-compilation chain. |
| REQ-08 `IssueStore` conformance | `89e7a273` storage trait takes `IssueId` (0/3); `ec190bd5` removes cfg twins in storage (**ready**; `with_mutation_interruptions` at `json.rs:455,475`) | Consume ec190bd5. Write the suite before or after 89e7a273 (decision). | 89e7a273 is behind 3 retype tasks. |
| REQ-09 remainder | `5e846743`, `af9a5509`, `34361448`, `459bb995`, `fbb661f2` (config tests under `EnvGuard`) | Unrelated. | — |
| REQ-10 self-tests to gates | `afc2b698` remove source-scan tests (**ready**) edits `scratch_build/repository_state_cutover_tests.rs`, in the same target | Unrelated (different file). | — |
| REQ-11 fixture key, hidden subcommand, nextest lists | `5687e281` (`main.rs` `run()` hosts the hook at `:1787`); `0e3b69fc` compile renderers under test-support (2/4) rewrites the `test-support` comment in `crates/jit/Cargo.toml` | Avoid conflict with both. Neither needs to land first. | — |
| Event-matching tests in moved cases | `e3d06f82` and `3625b18b` (body view; 0/1 each) | Avoid conflict. | — |

More overlaps, verified with `jit issue show`:

- `89e7a273` REQ-04 reads: "The shared storage conformance suite passes against both backends". That is the suite c780514e REQ-08 creates, so exactly one of the two must own it.
- `fb2dc5c3` (df129d9e, backlog, behind `a08c4b38`) has two overlaps:
  - Its REQ-02 (shared harness compiled once in test-support) is already met by `ef9897ab`.
  - Its REQ-04 re-measures the build budgets, which overlaps c780514e REQ-12.
- `f9977efd` (backlog, behind `5687e281`) adds a `main.rs` line ceiling to `scripts/rust-build-budget.sh`.
- `af9a5509` (ready, claimed) edits the projection-suite lever at `command_exit_code_projection_tests.rs:510`.
- About 26 df129d9e handler-move tasks restate the exit-code rows in `generate_command_exit_codes` and turn list payloads into `ListEnvelope`. Consolidate REQ-01 and REQ-04 either before or after that wave, never partway through it.
- Some task text cites stale paths:
  - `cc6ef886` (df129d9e, backlog) puts its lease helper in `crates/jit/tests/common/`, which no longer exists.
  - `c400730c` and `2ac50581` cite `exit_code_tests.rs`, `query_tests`, and `harness_demo` as existing assertions.

  Deleting those files leaves the citations dangling.
- `81e7cdc3` (ready) adds a shared reference fixture to `crates/jit/src/test_utils.rs`. Consume it first.

Recommendation for the dispatcher: give REQ-04 an edge onto `650e6a56` and the harness_demo move an edge onto `d125490a`; both are ready. Do not put a blanket edge onto 2195c98f, which would hold all 12 REQs behind 107 backlog tasks. See Needs decision for the REQ-05/06/08 ordering.

## C. Current test layout

- **Integration targets.** 10 targets: `crates/jit/tests/{cli_gate,cli_issue,cli_item_validate,cli_query_graph,cli_repo_workflow,fast_docs_templates,fast_issue,fast_rules,scratch_build}` and `crates/server/tests/server_integration`. Files per target, excluding `main.rs`: cli_gate 15, cli_issue 36 (one `.toml`), cli_item_validate 15, cli_query_graph 16, cli_repo_workflow 38, fast_docs_templates 19, fast_issue 16, fast_rules 15, scratch_build 6, server_integration 2. Nextest sees 14 binaries; the extras are the `jit` and `jit-server` lib and bin unit tests.
- **Shared test support.**
  - `crates/jit/src/test_utils.rs` (1,912 lines): guards, fixture cache, `setup_test_repo`, package helpers.
  - `crates/jit/src/test_harness.rs` (326): `TestHarness`, `seed_memory_*`.
  - `crates/jit/src/test_taxonomy.rs` (296).
  - `crates/jit/src/commands/test_helpers.rs` (215): `memory_executor`, interruption helpers.
  - `crates/jit/src/storage/test_support.rs`.
  - `crates/jit/src/test_mode.rs`.
  - Suite roots `crates/jit/tests/*/main.rs` hold local fixtures (`fixture_issue`, `TaxonomyRepo`, `memory_executor`).
- **Build budget** (`scripts/rust-build-budget.sh:41-43`, `dev/benchmarks/rust-build-budgets/README.md`): targets 10/12; active test executables 14, 1,723,670,208/2,147,483,648 B; suite-duration limit 30 s. The latest `suite-build` line warmed 17 executables totalling 2,051,867,776 B in 73 ms.
- **Script self-test gates.** None. `.jit/gates.toml` declares `cargo-ci` (`./scripts/cargo-ci.sh`), `cargo-ci-features`, `clippy`, `docs-mechanical`, `fmt`, `jit-validate`, `mcp-ci`, `npm-ci`, `repo-validate`, `secret-detection`, `tests`, `dependency-audit`, and the AI-review gates. All four script self-tests run inside the default Rust suite.

## D. Re-proof and fixture sweeps

### D.1 `exit_code_tests.rs` case inventory

| Lines | Tests | Assertions beyond the exit code | Class and destination |
| --- | --- | --- | --- |
| `:36`, `:51`, `:69`, `:147`, `:169`, `:296` | success, not found, cycle, invalid argument, I/O error, help and version | none, or a generic stderr fragment | re-proof (projection rows; graph owns cycles) |
| `:112` | gate not in registry | "not found in registry" and JSON code | re-proof (`fast_issue/bulk_operations_tests.rs:70`, `storage/errors.rs`) |
| `:187` | already exists | "already exists" | re-proof (claim, profile and snapshot suites assert it) |
| `:227` | JSON error format | `ISSUE_NOT_FOUND` envelope | re-proof (failure-envelope contract suite) |
| `:247` | validation command | none | re-proof |
| `:1040` | invalid label pattern | "Invalid label pattern" | re-proof (`label_query_json_tests.rs`, `fast_rules/label_query_tests.rs`) |
| `:1060` | invalid document scope | "Invalid scope" | re-proof (`cli_query_graph/check_links_tests.rs:384`) |
| `:1255` | batch description guard | guard message | re-proof (`issue_update_description_ops_tests.rs`) |
| `:1566` | unconfirmed delete | `JIT_ALLOW_DELETION=1` remediation, empty stdout | re-proof (`error_json_tests.rs`, projection suite) |
| `:312`, `:405`, `:493`, `:559`, `:605` | transition blocked by gates or dependencies, human and JSON; missing dependency | `BLOCKED` details (`requested_state`, `actual_state`, blockers), remediation commands, human remediation text | unique → `cli_issue/error_json_tests.rs` |
| `:1488`, `:1512`, `:1528` | startup repository-not-found (JSON and plain), format too new | `REPOSITORY_NOT_FOUND` and `REPOSITORY_FORMAT_TOO_NEW` envelopes, empty plain stdout | unique → `cli_issue/top_level_failure_envelope_tests.rs` |
| `:652`, `:702`, `:765`, `:820`, `:897`, `:920`, `:954`, `:999` | claim blocked by dependencies or precheck gate, claim-next blocked and skipping non-ready issues, `--assign` hints (human and JSON), assign-only claim, ready-issue claim | blocked details, skip behaviour, assign hint text, resulting state and assignee | unique → `cli_repo_workflow/claim_integration_tests.rs` |
| `:1590`, `:1616` | confirmed delete, text and JSON | "Deleted issue:" text and JSON body (strings appear only in `main.rs`) | unique → `cli_repo_workflow/integration_test.rs` |
| `:1189`, `:1220` | ambiguous and too-short prefix on `issue show` | "Ambiguous ID", `AMBIGUOUS_ID`, "at least 4 characters" (no other test owns these) | unique → `cli_issue/issue_status_projection_tests.rs` (already owns `INVALID_ID_PREFIX`) |
| `:1372`, `:1447` | too-short and ambiguous prefix in both `dep add` positions | position-specific error codes | unique → `cli_query_graph/dep_add_redundancy_cli_tests.rs` |
| `:1315` | `dep rm` symmetric argument validation | symmetric error for both positions | unique → `cli_query_graph/dep_add_redundancy_cli_tests.rs` |
| `:1280` | parent filter before subcommand | "were given before" guard and `INVALID_ARGUMENT` envelope (string only in `main.rs`) | unique → `cli_query_graph/query_json_tests.rs` |
| `:1079` | gate preset create naming a removed registry gate | "Gate not found in registry: mygate" | unique → `cli_repo_workflow/gate_preset_tests.rs` |

### D.2 Projected exit codes asserted outside the projection suite

Sweep over `crates/jit/tests/cli_*`, excluding `exit_code_tests.rs`, the parity suite and the projection suite. It covers four forms:

- numeric `.code(N)`;
- `status.code()` compared with `Some(…)`, single-line or multi-line, where the expected value is a literal, `ExitCode::X`, or a derived `code.exit_code().code()`;
- symbolic `ExitCode::X.code()` and `ExitCode::X as i32`;
- helpers that take an expected exit code (`profile_acceptance_tests.rs:105` `failed_json_with_path`).

The projection's wildcard rows (`*` with 0, 1, 2, 3, 5, 10 and 11) bind every command, so each such assertion re-proves a row unless its command and code pair has no row. Several tests use the code as their only evidence of the error class on a path, for example `repo_discovery_tests.rs:355` ("NotFound (3), not GenericError (1)") and the lock timeouts in `lock_contention_tests.rs:66` and `profile_concurrency_tests.rs:119`. These need another class witness, such as the `--json` envelope's `error.code`.

| Target | Files (assertion sites) |
| --- | --- |
| `cli_gate` | `gate_evaluate_exit_code_test.rs` (13 sites in 13 tests, whose added property is the JSON verdict), `gate_evaluate_all_test.rs` (9), `gate_cli_integration_test.rs` (7), `gate_status_all_strict_test.rs` (5), `gate_evaluate_rerun_test.rs` (5), `gate_preset_apply_json_failure_test.rs` (2), and one each in `gate_findings_test.rs`, `bracket_coverage_gate_run_test.rs` and `gate_status_all_lean_response_test.rs` |
| `cli_issue` | `batch_create_tests.rs` (10), `top_level_failure_envelope_tests.rs` (6), `issue_status_projection_tests.rs` (4), `issue_show_projection_test.rs` (4), `issue_create_description_file_tests.rs` (3), and one each in `error_json_tests.rs`, `issue_search_label_filter_tests.rs` (`ExitCode::InvalidArgument`), `stored_record_classification_tests.rs`, `search_exit_status_tests.rs`, `snapshot_export_tests.rs`, `verb_hint_tests.rs`, `serve_failure_envelope_tests.rs` and `container_rollup_tests.rs` |
| `cli_query_graph` | `check_links_tests.rs` (7), `dep_add_redundancy_cli_tests.rs` (7), `scope_validation_tests.rs` (4), `graph_json_tests.rs` (2), `label_query_json_tests.rs` (2, derived from the error classification) |
| `cli_item_validate` | `validate_cli_rule_tests.rs` (6), `invariant_check_cli_tests.rs` (4), `derived_state_repair_tests.rs` (3), `project_render_cli_tests.rs` (1), `validation_lease_tests.rs` (1) |
| `cli_repo_workflow` | `profile_cli_tests.rs` (9), `claim_integration_tests.rs` (5), `config_get_tests.rs` (4), `project_config_tests.rs` (4), `serve_cli_tests.rs` (2), and one each in `first_guess_residuals_test.rs`, `format_compat_cli_tests.rs`, `integration_test.rs`, `lock_contention_tests.rs` (`ExitCode::LockTimeout`), `profile_concurrency_tests.rs`, `repo_discovery_tests.rs` and `worktree_cli_tests.rs`; `profile_acceptance_tests.rs` asserts through its `failed_json_with_path` helper |

Out of scope: three recorded-corpus contracts compare statuses against the registered error classification. They are `failure_probe_fixture.rs` (`:1101`, `:1251`), `failure_envelope_contract_tests.rs:95` and `failure_probe_coverage_tests.rs`. Matches under `scratch_build` and `fast_docs_templates/ai_review_verdict_tests.rs` assert shell-script exit codes, not `jit`'s.

### D.3 Query and cycle re-proofs outside `query_tests.rs`

The domain owns `query_by_label`, `query_by_labels` and `query_strategic` (`domain/queries.rs:278,318,336`).

| File | Tests | Class |
| --- | ---: | --- |
| `cli_query_graph/label_filter_repeatable_query_tests.rs` | 17 | re-proof: AND semantics and single-label regression for each of eight list subcommands; the wiring is one repeatable `--label` case per subcommand |
| `cli_query_graph/label_query_json_tests.rs` | 7 | re-proof of pattern semantics (exact, wildcard, no match, case); the invalid-pattern error is the wiring case |
| `cli_query_graph/query_json_tests.rs` | 5 | wiring of `--state`, `--priority`, `--assignee`, `ready` and `blocked` JSON output; it re-proves filter semantics only where it asserts membership beyond one matching issue |
| `cli_query_graph/label_hierarchy_e2e_test.rs` | 4 | mixed: label validation and hierarchy warnings are owned here; query semantics inside the workflows are re-proofs |
| `fast_rules/label_query_tests.rs` | 12 | re-proof of `query_by_label` through the executor; the `search_issues_with_filters` cases (3) own the search filter composition |
| `fast_rules/label_strategic_tests.rs` | 7 | re-proof of `query_strategic` through the executor |
| `fast_rules/validation_tests.rs:61` | 1 | owner: repository validation reporting a stored cycle (not dependency-operation rejection) |
| `cli_issue/batch_create_tests.rs:196` | 1 | owner: batch atomicity (zero created) and cycle keys named in the message; only its exit code re-proves a row |
| `fast_issue/bulk_operations_tests.rs:221` | 1 | owner: atomic rejection of a multi-edge dependency batch (`DependencyBatchRejectedError`) |

### D.4 List-emitting command outputs

Derived from emission sites, not from the schema. The sweep covered every struct under `crates/jit/src` with a numeric `count` field and every inline `json!` body with a `"count"` key. `schema.rs:730` falls back to `(None, "CommandResponse")` for unregistered commands. The generated schema has 29 commands whose output carries `count`.

| Emission | Command | Schema today |
| --- | --- | --- |
| inline `json!` in `main.rs` (`:4547`, `:4984`, `:5005`, `:5900`, `:7391`, `:7476`) | `gate preset list`, `events tail`, `events query`, `label values`, `claim status`, `claim list` | fallback |
| `DocumentListResult`, `AssetListResult` (`commands/mod.rs:1863`, `:1878`) | `doc list`, `doc assets list` | fallback |
| `ItemListResult` (`commands/item.rs:20`) | `item list`, `item search` | fallback |
| `InvariantCheckResult` (`commands/invariant.rs:24`) | `invariant check` | fallback |
| `ProjectRenderResult` (`commands/project.rs:47`) | `project render` | fallback |
| `ArchiveCandidates`, `ArtifactPlan` (`domain/artifact_plan.rs:30`, `:962`; `count` is the artifact or candidate length) | `archive candidates`, `archive document`, `archive container` | fallback |
| `GraphDownstreamResponse`, registered under the key `graph_downstream` (`schema.rs:668`) | `graph rdeps` | fallback, because of the key mismatch |
| `ListEnvelope` instantiations in `output.rs` | the 29 schema commands | typed |

The following carry a `count` that is not a list envelope:

- `ProjectionRenderReport.count` (`commands/project.rs:27`) counts rendered items, not `kinds`.
- `StateCount` (`output.rs:1909`) is a tally.
- `IssuesInfo` and `DocumentsInfo` (`snapshot.rs:53`, `:64`) are snapshot manifest data written to the export, not command stdout.
- `GraphData` (`crates/server/src/routes.rs:117`) is the HTTP API, not a CLI command.

`list_envelope_tests.rs` covers every fallback command except `claim status`, `project render` and the three archive commands.

### D.5 Tool-missing skips

| Site | Tool | Count |
| --- | --- | ---: |
| `scratch_build/merged_tree_gate_verification_tests.rs:48-53` | git, cargo or jq (script exit 2) | 1 |
| `scratch_build/rust_build_budget_checker_tests.rs:175`…`:521` | jq | 14 |
| `scratch_build/dependency_feature_policy_tests.rs:168,185` | cargo | 2 |
| `cli_item_validate/validate_cli_rule_tests.rs:930-935` | node | 1 |

`storage/lock.rs:652` (`skip_unless_write_is_denied`) skips when the process can write a read-only file, as root can. That is an environment precondition, not a missing tool.

### D.6 In-process private setups outside the CLI targets

File-backed setups. These are functions that create a temporary repository, write configuration, run `jit init` or open a `JsonFileStorage` executor:

- `fast_rules/default_rules_registry_derivation_tests.rs`: `executor` (`:18`) and `setup_initialized_repo` (`:28`).
- `fast_rules/type_hierarchy_schema_regen_tests.rs`: `executor` (`:10`) and `setup_initialized_repo` (`:16`), the same as the previous file.
- `fast_rules/effective_rules_tests.rs`: `repo` (`:16`) and `executor` (`:36`).
- `fast_rules/undeclared_taxonomy_tests.rs:20`: `scaffold_declaring`, a third init-from-config copy.
- `fast_docs_templates/planning_preset_tests.rs:21`: `file_backed_executor`.
- `fast_docs_templates/templates_loader_tests.rs:16`: `setup`, which writes declaration files into a temporary directory.
- `fast_docs_templates/git_revision_tests.rs:15-20`: hand-rolls `git init` with a fixed author identity.
- `fast_rules/cli_warnings_integration_tests.rs:24`: a profiled setup, which the rule-warning move removes.

Memory-backed setups. These are functions that build an `InMemoryStorage` or a memory executor and seed declarations, instead of taking them from the shared executor:

- `store_with_rules` in `fast_issue/graph_rule_validation_tests.rs:15` and `fast_rules/validate_rule_runner_tests.rs:21`.
- `executor_with_rules` in `fast_issue/transition_graph_enforcement_tests.rs:24` and `fast_rules/local_rule_enforcement_tests.rs:20`.
- `executor` in `fast_issue/strictness_enforcement_tests.rs:42` and `fast_issue/strictness_transition_tests.rs:41`.
- `executor_for` in `fast_issue/graph_rule_validation_tests.rs:34`.
- `make_storage` in `fast_issue/short_hash_tests.rs:389`.
- `executor_with_templates` in `fast_docs_templates/template_binding_tests.rs:106`.
- `memory_storage` in `fast_docs_templates/template_apply_atomicity_tests.rs:380`.
- `storage_with` and `render_executor` in `fast_rules/project_render_harness_tests.rs:77,90`.
- `strategic_executor` in `fast_rules/label_strategic_tests.rs:13`.
- `executor_with_sdd_example` and `research_coverage_executor` in `fast_rules/example_rulesets_tests.rs:612,1524`.

Helpers that act on a `TestHarness` or executor they are given (for example `configure`, `create_epic`, `seed_epic`) are scenario steps, not setups.
