# f368ce3d investigation: engine vocabulary and test switches

> **Diátaxis Type:** Reference

Fact base for planning epic f368ce3d, verified against HEAD `6cb2dee0d` (2026-11-06).
The container's Background cites lines at `79c177fbd`; every reference below is current.
"Production" means a `src/` file outside a `#[cfg(test)]` module, a `*_tests.rs` file,
`test_*` support modules, and `cfg(any(test, feature = "test-support"))` items.

## 1. Claim classification

Counts: 35 claims (22 Background bullets, one implied-scope claim, 12 criteria):
**already-done 8**, **valid-and-open 26**, **invalid-as-stated 1**.

### Background claims

| # | Claim | Class | Evidence at HEAD |
|---|---|---|---|
| C1 | `issue reject --reason` writes `resolution:<reason>` | valid-and-open, moved | `labels.rs:108-125` `resolution_label` (`format!("resolution:{reason}")` at `:115`), called by `commands/issue.rs:1069` `reject_issue`; label appended by `CapturedFieldUpdate::rejection` (`commands/mod.rs:1172-1180`). |
| C2 | `resolution` is declared by the default profile | valid-and-open | `profiles/jit-default/manifest.toml:161-169` (identity at `:164`). Neutral `jit init` writes only `[version]` and `[project]` (`repository_state/initialize.rs:58-60`, used at `commands/init.rs:271-276`), so a fresh repository declares no namespace. |
| C3 | `extract_hard_criteria` collects literal `[hard]` lines | valid-and-open, moved | Now `templates.rs:821-830` (filter `starts_with("[hard]")` at `:827`), feeding `InterpolationContext` (`templates.rs:716-791`, token at `:781`). |
| C4 | Markers are declared per item kind | valid-and-open | Field `ItemKindConfig::markers`, `config.rs:460-463`; resolved accessor `ItemKind::markers`, `domain/item.rs:859-862`. |
| C5 | `LinkValidator::is_permanent_path` hardcodes `docs/`, `README.md`, `CONTRIBUTING.md`, `LICENSE`, `dev/architecture/` | valid-and-open | `document/link_validator.rs:199-211`, used at `:150-152` (comment at `:151` repeats the list). Declared registry: `DocumentationConfig::permanent_paths`, `config.rs:321-323`. |
| C6 | `suggest_label_fix` always suggests `component:<label>` | valid-and-open (wording) | `labels.rs:128-136` suggests `type:<label>` **and** `component:<label>`, only for a colon-less label, never consulting the registry. The gap holds; "always" overstates the trigger. |
| C7 | `query_strategic` falls back to levels 1-2 | valid-and-open, moved | `commands/query.rs:139-160`; fallback `:145-152` (`level <= 2` at `:150`). |
| C8 | Web picks level 2 as cluster level | valid-and-open | `web/src/utils/subgraphClustering.ts:60-66` (`uniqueLevels.includes(2) ? 2 : …` at `:66`). |
| C9 | `web/src/types/hierarchy.ts` repeats `level <= 2` | already-done | File deleted by `71343aef8` (jit:dc6fac0c). `web/src/types/` holds only `models.ts`, `subgraphCluster.ts`. |
| C10 | Snapshot README lists `docs/`, `dev/` | valid-and-open, moved | `commands/snapshot.rs:396-447` `generate_readme`; `docs/` at `:428`, `dev/` at `:429`. |
| C11 | Production reads `JIT_TEST_MODE` at three sites | already-done | No reader in `crates/`, `web/`, `scripts/`, `docs/`, `.github/`, `.jit/gates.toml`, `dev/TESTING.md`. Removed by `c03850c61` (claims-index check, `test_mode.rs`) and `b27d75188` (guard test-mode branch). Only historical `.jit/` records mention it. |
| C12 | Guard runs on gate define, update, remove | valid-and-open | Three call sites: `commands/gate.rs:1161` (`define_gate`), `:1258` (`update_gate`), `:1294` (`remove_gate_definition`). The Background's fourth line no longer exists. |
| C13 | Guard compares `HEAD` with literal `origin/main` | valid-and-open | `commands/worktree.rs:59-77` `require_common_history_with_main`; literal at `:65`, message `:69-74`. |
| C14 | Guard spawns git in the process working directory | already-done | Runs `GitCheckout::compare_with_base` (`storage/git.rs:395-430`) on `require_worktree_paths()?.git()`; `Git::run` passes `-C <worktree_root>` (`storage/git.rs:141`). CLI supplies paths from the selected data root (`commands/bootstrap.rs:77-123`). Done by fb5e8e8d / 7984d807. |
| C15 | Guard rustdoc claims a `cfg(test)` skip | already-done | Rustdoc `worktree.rs:59-60` describes the code; no `cfg(test)` wording remains. |
| C16 | Public doctest sets `JIT_TEST_MODE` | already-done | `update_gate` doctest `gate.rs:1213-1248` uses an in-memory executor whose default `WorktreePaths::non_git` (`commands/mod.rs:2561-2579`) skips the guard; no env var. |
| C17 | `validate --branch-drift` hardcodes `origin/main` | valid-and-open | `commands/validate.rs:1195-1234` (literal at `:1213`, messages `:1215`, `:1218`, `:1225-1228`). Process-cwd part already done (uses `write_checkout()`). |
| C18 | Help text names `origin/main` | valid-and-open, moved | `cli.rs:257-262` (`ValidateArgs::branch_drift`). |
| C19 | Drift message names `origin/main` | valid-and-open, moved | `main.rs:7549` gone; now `output/validate.rs:152` ("Branch is up-to-date with origin/main"). |
| C20 | Shipped pre-commit compares against `origin/main` or `main` | valid-and-open, moved | `scripts/hooks/pre-commit:24-37`; embedded by `commands/hooks.rs:7` `include_str!`. |
| C21 | Config accepts `[global_operations] require_main_history`, `allowed_branches` | already-done | Removed by `e30502eb5` (jit:af9a5509); no match in `crates/`, `docs/`, `profiles/`, `.jit/config.toml`. |
| C22 | `type` namespace, per-level default icons and storage paths are mechanism vocabulary | already-done (holds, no work) | `labels.rs:166` `TYPE_NAMESPACE`; `type_icons.rs:23,50-72` (applies only to declared types); data-root paths `repository_state/path.rs:13-26`. |
| C23 | (implied by REQ-01) `mcp-server/` holds production source | **invalid-as-stated** | Directory absent (`ls mcp-server` fails); retired per `@/charter/D-23`. |

### Criteria

| REQ | Class | Gap at HEAD |
|---|---|---|
| REQ-01 | valid-and-open | Background sites C1, C3, C5-C8, C10, C13, C17-C20 plus sweep findings S1-S6 (§3.1). `mcp-server/` scope is vacuous (C23). |
| REQ-02 | valid-and-open | No declaration names a reason namespace; `reject_issue` writes `resolution:` unconditionally, bypassing rule enforcement (`issue.rs:1060-1063`; test `issue.rs:4203` `test_reject_issue_appends_resolution_label_unchecked`). |
| REQ-04 | valid-and-open | `templates.rs:821-830` ignores declared markers and sections; see decision D3. |
| REQ-05 | valid-and-open | `link_validator.rs:202-211` literal list; see decision D4. |
| REQ-06 | valid-and-open | `labels.rs:130-136`; no registry reaches `validate_label`. |
| REQ-07 | valid-and-open | `query.rs:143-153`. The domain function already returns empty for no types (`domain/queries.rs:307-323`, guard `:310-312`). |
| REQ-08 | valid-and-open | `subgraphClustering.ts:66`; served `strategic_types` is fetched but discarded (`GraphView.tsx:587-592`). |
| REQ-09 | valid-and-open | README text is a static template with two manifest counts (`snapshot.rs:396-447`). |
| REQ-10 | **already-done** | See C11, C14, C16. The guard runs only when the caller supplies git `WorktreePaths` (`commands/mod.rs:2549-2579`); the claims-index check is deleted. Test owner: `gate.rs:1530-1590` `test_gate_registry_guard_maps_supplied_checkout_comparisons` sets paths via `with_worktree_paths`. |
| REQ-11 | valid-and-open | Invocation-repository part done for guard and drift; base literal remains in all three comparisons; no declaration exists. Hook runs git in its own repository (git hooks run in the checkout) but falls back to `main`. |
| REQ-12 | valid-and-open | `cfg(test)` part done; `origin/main` claims remain at `worktree.rs:59-60,71-73`, `validate.rs:1195-1198`, `cli.rs:257`, `output/validate.rs:152`, `pre-commit:2,29-33`. Inaccurate hook descriptions: `cli.rs:2600,2605` and `commands/hooks.rs:9,16` say the hook "validates branch drift"; it gates `.jit/config.toml`/`.jit/gates.toml` commits. Comments `gate.rs:1160,1257,1293` say "common history with main". |
| REQ-13 | valid-and-open | No test exercises these sites over declarations differing from this repository's. |

## 2. Prior art

| Source | Fact recorded |
|---|---|
| 09974ad1 (done) | Added `GitCheckout::compare_with_base(base)`; Notes defer "which base branch, and whether they run" to this epic. |
| fb5e8e8d (done) | Guard became executor method over `paths.git()` with base `origin/main`; on `BaseUnresolved`/`NoMergeBase` it allows. Deliberately `git()` not `write_checkout()`: "A relocated code checkout does not lend history to an external non-Git data store." |
| 7984d807 (done) | Guards act on the served repository; removed test-mode branch. |
| 2f563673 (done), commit `c03850c61` | Deleted claims-index replay, `JIT_TEST_MODE`, `test_mode.rs`. |
| af9a5509 (done), commit `e30502eb5` | Removed `[global_operations]`, `[locks]`, `[events]`, `worktree.mode`. |
| dc6fac0c, commit `71343aef8` | Removed orphaned `web/src/types/hierarchy.ts`. |
| d91ea408 (done) | Web consumes served `type`, `parent`, `children`, `cluster`, `rank`; no client parses `type:`. |
| 0cce1a44 (done) | `query strategic` checks the `type:` label, not membership labels. |
| 5f71a48d, da12ad66 (done) | `--reason` that cannot form a label is refused before write (`labels.rs:114-125`). |
| `dev/active/f4d9502a-git-cli-module/plan.md:56`, `investigation.md:43,86,214` | Base branch and whether guard/drift run are this epic's decision. |
| `graph/hierarchy.rs:94-100` (module rustdoc) | Records the "deliberate divergence": web clusters at level 2, core `cluster` is the strategic root. A REQ-08 consumer. |
| 030df4b1 (backlog) | Moves read paths (including `get_namespaces`, server config reads) onto one captured-declaration helper. |

No study or session document records a decision on a reason-namespace role or a base-branch key.

## 3. Consumer sweep

### 3.1 REQ-01 sweep beyond the Background

Production sources searched: `crates/*/src` (test modules excluded), `web/src` (tests excluded), `scripts/hooks/`, the hook embedded by `commands/hooks.rs:7` (the only hook `jit hooks install` writes).

**Instances or unresolved (need a ruling):**

| Id | Site | Construct | Assessment |
|---|---|---|---|
| S1 | `templates.rs:682` `PLAN_DOC_LABEL = "plan"`; read `:708`, consumers `validation/repository.rs:650`, `commands/validate.rs:1146` | const | Document-label vocabulary the engine assumes; no declaration names it. Instance under a literal REQ-01 reading. |
| S2 | `commands/init.rs:307` `.unwrap_or_else(\|\| "main".to_string())` | string literal | Fabricated branch label recorded in worktree identity when `HEAD` has no branch. A branch name no repository declares. |
| S3 | `profile/registry_region.rs:25` `REGION_PACKAGE_ID = "jit-dogfood"` | pub const in an ungated `pub mod` (`profile/mod.rs:28`) | Names this repository's profile. Consumers are test-support `generated_artifacts.rs:269-270` and `examples/package-drift.rs:17`; no production command reads it. |
| S4 | `cli.rs:1115,2188,2200-2201,2352,2377-2378` | rustdoc help text | Domain example labels (`resolution:`, `epic:auth`, `milestone:v1.0`). Illustrative prose, not behavior; `cli.rs:1115` changes with REQ-02. |
| S5 | `commands/snapshot.rs:19-23` `find_remote("origin")` via `git2` | string literal | Remote name, not branch; also a `@/charter/D-21` deviation. Owned by in-progress c140921c (moves to `storage/git.rs:357-367` `origin_url`, which also names `origin`). |
| S6 | `crates/server/src/main.rs:164`, `first_read.rs:21,34-40` `JIT_SERVER_REPORT_FIRST_READS` | env read | Production read of a test-only switch (set only by `server/tests/server_integration/graceful_shutdown_tests.rs:261`). Same pattern REQ-10 removes, under another name. |
| S7 | `scripts/hooks/pre-commit:16-22,24` | shell | Hook hardcodes `.jit/(config\|gates).toml` (ignores a relocated data root) and runs the branch check only unless `worktree.enforce_leases = off`, coupling the base check to lease mode. |

**Mechanism vocabulary (exempt under `@/inv/domain-agnostic`):**

- Binding defaults: `templates.rs:199,203,207` (`planning`, `breakdown`, `container`); `declarations/rules.rs:696` (`success_criteria`), `:710` (`satisfies`), `:40` `DEFAULT_CRITERION_ID_PATTERN`; `repository_state/rule_serialize.rs:334,363`.
- Built-in rule names over the registry: `repository_state/default_rules.rs:90` (`label-format`), `:108` (`namespace-registry`).
- `labels.rs:166` `TYPE_NAMESPACE`; `type_icons.rs`.
- Test-support documentation paths: `REFERENCE_PATH` consts in `test_support` modules (`storage/reference.rs:52-58`, `domain/event_catalog.rs:22-30`, `runtime_defaults.rs:59`, `schema.rs:688`, `gate_presets/reference.rs:13`) and `generated_artifacts.rs:180-181`, gated by `lib.rs:21-22`.

**Web non-literal residue (REQ-08 sweep):** comments with level literals at `subgraphClustering.ts:37,63-65,78,85` and `types/subgraphCluster.ts:5,35`; level arithmetic `clusteredGraphLayout.ts:59-70` (`primaryContainerLevel + 1`) assumes contiguous levels. Label-example comments `graphFilter.ts:13,88,102`, `ClusterNode.tsx:16`.

### 3.2 Reason label (REQ-02)

- Code: `labels.rs:108-125`; `commands/issue.rs:1060-1076` (rustdoc `:1060` names `resolution:`); `commands/mod.rs:1172-1180`; help `cli.rs:1115`.
- Unit tests: `labels.rs:267-275`; `commands/issue.rs:4009` (`test_reject_issue_with_reason_records_state_events_and_resolution_label`, asserts `:4037`), `:4075`, `:4110`, `:4176-4189`, `:4203`.
- CLI tests: `tests/cli_repo_workflow/workflow_tests.rs:331-348,357,409,462`; `lease_enforcement_tests.rs:46`; `tests/cli_issue/list_flag_tests.rs:71` (label literal only).
- Declarations: `.jit/config.toml:121-124`; `profiles/jit-default/manifest.toml:161-169`.
- Docs: `docs/reference/cli-commands.md:1470-1510` (`:1483` states the label); `docs/concepts/core-model.md:679,809,846-864,934`; `docs/reference/labels.md:162,722`.
- Agent skills: `.agents/skills/jit-execution-lead/SKILL.md:225`.

### 3.3 Hard-criteria token (REQ-04)

- Code: `templates.rs:716-791` (`InterpolationContext`), `:821-830`; callers `templates.rs:805`, `commands/template.rs:903,977`, `commands/template_expand.rs:199,332`.
- Tests: `templates.rs:920-958` (`test_extract_hard_criteria_pulls_only_hard_items`, `_empty_when_none`, interpolation test); `tests/fast_docs_templates/template_apply_tests.rs:60` (fixture template).
- Shipped consumers: none. No template in `.jit/templates.toml`, `profiles/`, or `docs/examples/sdd/templates.toml` uses `{container.hard_criteria}`; no document in `docs/` names it.

### 3.4 Permanent link paths (REQ-05)

- Code: `document/link_validator.rs:150-152,199-211`; sole constructor `commands/document.rs:1224` (`doc check-links`).
- Tests: `link_validator.rs:245-482` (fixtures use `docs/...` paths; none asserts the root-relative permanent branch); CLI `tests/cli_issue/command_exit_code_projection_tests.rs:176-193`, `tests/cli_repo_workflow/derived_documents_cli_tests.rs:85-119`.
- Docs: `docs/reference/configuration.md:53,80,111,195-198` describe `permanent_paths`; no doc describes link permanence.

### 3.5 Label suggestion (REQ-06)

- Code: `labels.rs:50-62` (`validate_label`), `:128-160`. User-visible only through `validate_label_pattern` (`labels.rs:73-90`), called by `cli.rs:142` (`IssueFilterArgs::parse`), `commands/query.rs:182,199,232,293`, `commands/search.rs:164`. `parse_label`, `type_value_of`, `matches_pattern` call `validate_label` and discard the message.
- Tests: `labels.rs:441-445` (`test_suggest_label_fix_missing_namespace` asserts `type:auth`), `:448-459`.
- Dead branch: `labels.rs:139-142` is unreachable (the `!contains(':')` case returned at `:130`).

### 3.6 Strategic fallback (REQ-07)

- Code: `commands/query.rs:139-160,227-234`; dispatch `dispatch/query.rs:29-37`.
- Tests: CLI `tests/cli_query_graph/query_tests.rs:101-114`, `label_hierarchy_e2e_test.rs:98`, `tests/cli_issue/gate_field_contract_test.rs:215-243`, `label_filter_repeatable_query_tests.rs:117,233` — all over a taxonomy that declares `strategic_types`. No test covers the fallback.
- Stale prose: `cli.rs:2146` and `docs/reference/cli-commands.md:2578` ("labels from strategic namespaces").

### 3.7 Web cluster level (REQ-08)

- Code: `subgraphClustering.ts:36-70`; `clusteredGraphLayout.ts:56-70`; `GraphView.tsx:587-592,635`; `types/models.ts:172` (`strategic_types` already typed).
- Tests: `utils/subgraphClustering.test.ts`, `utils/clusteredGraphLayout.test.ts`, `utils/clusteredGraphLayout.integration.test.ts`, `components/Graph/__tests__/GraphView.test.tsx:103,155-186`, `GraphView.layoutCache.test.tsx:147`.
- Rust prose consumer: `graph/hierarchy.rs:94-100`.

### 3.8 Snapshot README (REQ-09)

- Code: `commands/snapshot.rs:396-447`, written at `:530`; copied state `:449-480` (`config.toml`, `gates.toml` only when present, not recorded in the manifest).
- Manifest: `crates/jit/src/snapshot.rs:14-33` `SnapshotManifest` — `repo`, `scope`, `issues.files` (paths + state counts), `documents.items` (path, hash, assets), `metadata`, `verification`. The README also claims "Complete JIT configuration and gate definitions" (`commands/snapshot.rs:411`) and carries a speculative "Import (Future)" section (`:433-437`).
- Tests: `tests/cli_issue/snapshot_export_tests.rs:65` (existence only).

### 3.9 Branch comparisons (REQ-11, REQ-12)

- Code: `commands/worktree.rs:59-77`; `commands/gate.rs:1160-1161,1257-1258,1293-1294`; `commands/validate.rs:1195-1234`; `dispatch/validate.rs:124-134`; `output/validate.rs:134-152`; `cli.rs:257-262,2598-2605`; `commands/hooks.rs:7-17`; `scripts/hooks/pre-commit`; `scripts/hooks/README.md:8-11`.
- Tests: `commands/gate.rs:1530-1590`; `tests/cli_repo_workflow/worktree_cli_tests.rs:580-801` (six `test_validate_branch_drift_*` plus `test_git_guards_act_on_the_repository_they_serve_rather_than_the_working_directory`, all creating `refs/remotes/origin/main`); `tests/cli_issue/top_level_failure_envelope_tests.rs:224-242` (relies on absent `origin/main`); `tests/cli_item_validate/validate_cli_rule_tests.rs:617-626`; `tests/cli_repo_workflow/git_hook_tests.rs:26-57,95-116` (relies on the hook's local `main` fallback); `tests/fast_docs_templates/git_module_tests.rs:1444-1530` (primitive, base-agnostic).
- Docs: `docs/how-to/troubleshooting.md:305-335`; `docs/reference/worktree-validate.md:430`; `docs/reference/cli-commands.md:3285,3301-3302,3727-3732`; `docs/reference/exit-codes.md:63`.
- CI and gates: no workflow or `.jit/gates.toml` entry runs `--branch-drift` or installs hooks.

## 4. Primitive verification

| Primitive | Location | Delivers | Caveat |
|---|---|---|---|
| Item-kind markers | `domain/item.rs:859-862` `ItemKind::markers`; `:924-929` `marker_matches` (skips one checkbox token, `:948-955`) | Declared markers per kind | No kind is designated "the criteria kind"; the kind is found by its `section` (`success_criteria` in `.jit/config.toml:154-160`). |
| Item extraction | `domain/item.rs:1155-1164` `index_items`, `:1177-1205` `extract_raw_items` | Section-scoped, marker-filtered items | Drops a marked line without an id-pattern match; `extract_hard_criteria` keeps it and scans the whole description. |
| Captured item kinds | `repository_state/mod.rs:328-368` (`CapturedRepositoryDeclarations::item_kinds`, `pub(crate)`) | Resolved kinds on the template-apply path (`commands/template.rs:396,409`) | `expand_template` (`template_expand.rs:184-191`) receives only `DocumentationConfig` and `HierarchyConfig`; its signature must grow. |
| Permanent paths | `config.rs:321-323`; `ArtifactClassificationPolicy::from_documentation` `domain/artifact_classifier.rs:58-81`; `retains_source_in_place` `:155-158` | Declared, normalized paths and `contains_path` matching | Rustdoc `config.rs:292-294` scopes them to "paths inside the development root"; `docs/`, `README.md` lie outside `dev/`. |
| Strategic types | `config.rs:218`; `LabelNamespaces::strategic_types` `domain/types.rs:2950-2953`; `config_manager.rs:246-249`; server `routes.rs:1007-1025` (`GET /config/hierarchy` → `{types, strategic_types, icons}`) | Declared list, `None` when undeclared | Server already maps `None` to `[]` (`routes.rs:1018`). |
| Served hierarchy | `routes.rs:364-397` (`GET /graph` nodes carry `type`, `parent`, `children`, `cluster`, `rank`); `graph/hierarchy.rs:73-75` | `cluster` = topmost container of the parent chain | `cluster` is the milestone when one exists, not the epic the UI clusters on today. |
| Namespace registry | `config.rs:376-387` `NamespaceConfig {description, unique, examples}`, `deny_unknown_fields` | Declared namespaces | No role field exists. Profiles contribute whole entries (`repository_state/profile_apply.rs:301-318`, `MapEntryTarget::Namespaces`). |
| Base comparison | `storage/git.rs:174-186` `BaseComparison`; `:395-430` `compare_with_base(base: &str)` | Typed outcome, runs `-C worktree_root` | Base is a caller argument; no declaration feeds it. Guard uses `WorktreePaths::git()` (`storage/worktree_paths.rs:263-266`); drift uses `write_checkout()` (`:270-276`). |
| Config read by hook | `config.rs:1363-1420` `EffectiveConfig::full_snapshot` includes `worktree` and `coordination` | `jit config get worktree.<key>` already serves the hook (`pre-commit:16`) | `full_snapshot` and `commands/config.rs` belong to b697a3f7's in-flight work. |

## 5. Architecture fit

- **Pure sites.** `labels.rs` suggestion and `templates.rs` interpolation are pure; rebinding passes declarations in as arguments. `domain/queries.rs:307` `query_strategic` already takes the declared list.
- **Commands orchestrate.** `reject_issue` (`commands/issue.rs:1064`) and `query_strategic` (`commands/query.rs:139`) read config through `config_manager`; b697a3f7 REQ-10 and 030df4b1 move those reads to captured images. A new read should go through whichever path lands first.
- **Git.** The guard and drift check already reach git only through `storage/git.rs` (`@/charter/D-21`). Snapshot's `git2` use (S5) is c140921c's.
- **Web.** The served `strategic_types` reaches `GraphView.tsx:587` and is dropped; using it needs no server change. Using `cluster` needs no config read but changes cluster semantics.
- **Profiles.** A new scalar declaration (base branch) needs a `ScalarTarget` variant (`profile_apply.rs:340-366`) to be profile-declarable; a namespace role field travels with existing `MapEntryTarget::Namespaces` values.

## 6. Architectural-invariant check

| Invariant | Status at HEAD |
|---|---|
| `@/inv/domain-agnostic` | Violated by C1, C3, C5, C6, C7, C8, C10, C13, C17-C20; possibly S1-S3 (decision D5). |
| `@/inv/architecture-dependency-direction` | Guard switch is a caller input (`with_worktree_paths`); held. S6 is an ambient-environment test switch in the server binary. |
| `@/charter/D-21` | Held for guard and drift. Snapshot uses `git2` (S5, owned by c140921c). The hook calls git directly; it is a shell script outside the engine. |
| `@/charter/D-4` | Held: without a checkout the guard allows (`worktree.rs:62-64`) and drift fails with "requires a git repository" (`validate.rs:1208-1212`). |
| `@/inv/canonical-cutover` | `[global_operations]` already removed. Any new key replaces the literal without an alias. |

## 7. Coordination overlaps

| REQ | Sites | Overlapping work |
|---|---|---|
| REQ-02 | `labels.rs`, `commands/issue.rs:1060-1076`, `commands/mod.rs:1172-1180`, `config.rs:376-387`, `cli.rs:1115` | 783b33fa (`commands/issue.rs`, `commands/mod.rs`; release actor, different function); b697a3f7 field-update and readiness work in `issue.rs`/`mod.rs`; c2b3cefa (`cli.rs`). |
| REQ-04 | `templates.rs:716-830`, `template_expand.rs:184-199,332`, `commands/template.rs:903,977` | cd1da611 adds a `{container.slug}` token to `InterpolationContext` (its text cites `commands/template.rs`; the struct is in `templates.rs:716`); 0723860e (template apply prerequisites, `template_expand.rs`/`template.rs`); c2b3cefa and 1a737bb4 (`commands/template.rs`); b697a3f7 (`TemplateRegistry::load`). |
| REQ-05 | `link_validator.rs`, `commands/document.rs:1224` | b697a3f7 ("snapshot and link-check scopes, with one scope type"). |
| REQ-06 | `labels.rs`, `cli.rs:135-149`, `commands/query.rs`, `commands/search.rs` | c2b3cefa, 1a737bb4 (`cli.rs`). |
| REQ-07 | `commands/query.rs:139-160` | 030df4b1 (read paths from captured declarations); b697a3f7 REQ-10 (no disk config caches). |
| REQ-08 | `web/src/utils/*`, `GraphView.tsx`; `graph/hierarchy.rs:94-100` | 030df4b1 moves server config reads (`routes.rs:1007`). |
| REQ-09 | `commands/snapshot.rs:396-480` | c140921c (in progress, same file); 19404174 (b697a3f7 child) REQ-04 copies every declaration file, changing what the README describes. |
| REQ-11/12 | `worktree.rs:59-77`, `gate.rs:1160-1294`, `validate.rs:1195-1234`, `cli.rs:257-262,2598-2605`, `output/validate.rs:152`, `hooks.rs`, `scripts/hooks/`, `config.rs` (new key), `full_snapshot` | c2b3cefa, 9a2563fd, 1a737bb4 (`gate.rs`, `cli.rs`); b697a3f7 (`JitConfig::load`, `EffectiveConfig::load`, `commands/config.rs`). |
| REQ-01 sweep | S1 `templates.rs:682`; S2 `commands/init.rs:307`; S5 `snapshot.rs` | b697a3f7 (`templates.rs`); c140921c (S5). |

## 8. Open decisions

- **D1 Reason-namespace declaration (REQ-02).** Homes: (a) a role field on `[namespaces.<ns>]` (`NamespaceConfig`, `config.rs:379-387`), carried by existing profile namespace entries; (b) a binding table like `[type_hierarchy.label_associations]` (`config.rs:214-216`) or templates' `[roles]` (`templates.rs:209-242`); (c) delete `--reason` (`@/charter/D-18`), which contradicts REQ-02 as written.
- **D2 Base branch (REQ-11).** Homes: (a) key under `[worktree]` (`WorktreeConfig`, `config.rs:1057-1064`), readable by the hook through `jit config get` like `enforce_leases`; (b) new `[git]` table; (c) delete all three comparisons, which REQ-11 permits. The guard already allows silently when the base does not resolve (`worktree.rs:65-75`).
- **D3 `{container.hard_criteria}` (REQ-04).** The default requirement kind declares `["[hard]", "[aspirational]"]` (`profiles/jit-default/manifest.toml:212`), so "criteria carrying the declared markers" includes aspirational items under a token named `hard_criteria`. No shipped template uses the token (§3.3).
- **D4 Permanent links (REQ-05).** Declared `permanent_paths` cover paths inside the development root. Under REQ-05 as written, root-relative links to `docs/` or `README.md` in this repository turn Risky unless declared. `retains_source_in_place` (`artifact_classifier.rs:155-158`) is the existing "stays in place" predicate (outside the development root or permanent).
- **D5 Sweep findings S1-S7.** Which are in REQ-01 scope.
- **D6 REQ-01 text.** Names absent `mcp-server/`.
