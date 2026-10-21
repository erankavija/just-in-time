# Investigation: Give every identity and closed vocabulary one semantic type (2195c98f)

> Grounding for planning node 8aa53c43. Three scoped investigations against `main` at 9398a0b6b, after the plan bracket was applied.

## Criterion numbering

The investigations cite the epic's criteria as numbered **before** commit 2abebdd7c. That commit removed the profile-version criterion (old REQ-10) from 2195c98f, by steward decision, and moved it with the `ProfilePackageId` deletion to the profile apply-once rework 3d9a0ce3. It also narrowed REQ-09. Map an old 2195c98f number to the current one as follows:

| Old | Current | Old | Current |
|---|---|---|---|
| REQ-01 … REQ-09 | unchanged | REQ-14 | REQ-13 |
| REQ-10 | moved to 3d9a0ce3 | REQ-15 | REQ-14 |
| REQ-11 | REQ-10 | REQ-16 | REQ-15 |
| REQ-12 | REQ-11 | REQ-17 | REQ-16 |
| REQ-13 | REQ-12 | REQ-18 | REQ-17 |

A REQ number qualified by another issue's short id (for example `842b0ac6 REQ-05`) is that issue's own criterion and is not remapped.

Corrections made during planning:

- There are 9 applied-record readers, not 7, and one reserved-target matcher with two call sites.
- There are 14 production short-id truncation sites; `claim.rs:1057` is test code.
- `dependency.rs:551-572` is a tenth prefix-matching site.
- The two `claim_coordinator` prefix sites tolerate legacy short-id lease records; they are not user-prefix resolution.
- There are three production `HierarchyConfig` construction sites, not four.

## Part: Issue identity, events, leases and record paths

### Scope and baseline

Scope: REQ-01..07, REQ-12 (system actors and gate-run `by`), identity part of REQ-18. Baseline: `main` at `ef16d7c5f`. Paths are relative to the repository root; `src/` means `crates/jit/src/`. "PROD" excludes `#[cfg(test)] mod` bodies and test-only files (classifier script: `scratchpad/inv/tools/cls.py`). Counts are line-level grep counts unless stated.

Experiment evidence (scratch copy, no repo edits): `scratchpad/inv/exp/`. A naive flip of `Issue.id`/`Issue.dependencies` to a newtype produces 293 type errors in 32 lib files (`exp/errA.txt`); `main.rs`, server and all test targets are not even reached. With a full `Deref<str>`/`Borrow<str>`/`PartialEq<str|String>`/`From<String>` bridge it is still 180 errors in 29 files (`exp/errB.txt`), 144 of them `expected String, found IssueId` in the next ring of consumers. A serde flatten probe (`exp/flat/`) verified the REQ-05 envelope (see Primitive verification).

### 1. Claim classification

Totals: 22 claims (13 background bullets + 9 REQs). already-done 0; valid-and-open 21; invalid-as-stated 1 (C8, partially).

| # | Claim | Class | Current evidence |
|---|---|---|---|
| C1 | `Issue.id` and dependency list are `String` (`types.rs:414`) | valid-open | `src/domain/types.rs:414` `pub id: String`, `:427` `pub dependencies: Vec<String>`. Also `MinimalIssue.id` `:670`, `MinimalBlockedIssue.id` `:758`. No `IssueId`/`ShortId`/prefix type exists anywhere (grep for `struct IssueId|ShortId|IssueIdPrefix` is empty). |
| C2 | `GateRunResult.issue_id` and the issue id in every `Event` variant are `String` | valid-open (imprecise) | `types.rs:947`. 16 of 22 variants carry `issue_id: String` (`types.rs:1292-1616`); 6 are repository-scoped and carry none. 842b0ac6 deletes `LifecycleTimestampsBackfilled` → 16 of 21. `DependencyReduced.removed_deps: Vec<String>` is a further id list. |
| C3 | `Lease` fields are `String` (`claim_coordinator.rs:33-39`) | valid-open | `src/storage/claim_coordinator.rs:33,35,37,39`. Same strings in `ClaimOp` (`:105-139`), `ClaimCoordinator { worktree_id, agent_id }` (`:274-275`), `WorktreeIdentity.worktree_id` (`storage/worktree_identity.rs:24`). |
| C4 | `IssueStore` takes raw ids (`storage/mod.rs:166-290`) | valid-open, lines drifted | Trait `src/storage/mod.rs:99-386`: `load_issue(&str)` `:155`, `load_issue_or_not_found(&str)` `:176`, `resolve_issue_id(&str)->String` `:191`, `load_gate_run_result(run_id:&str)` `:262`, `list_gate_runs_for_issue(&str)` `:269`, `find_reusable_gate_run(gate_key:&str,..)` `:287`. |
| C5 | JSON backend joins an id containing `/` under `issues/` | valid-open | `storage/json.rs:730-731` `self.root.join(ISSUES_DIR).join(format!("{}.json", id))`; `resolve_issue_id` passes the raw user input to `load_issue` on its 32-char fast path (`json.rs:1264-1268`). Further host-path spellings `json.rs:900-905` (git `HEAD:` path), `:938-940` (main worktree). |
| C6 | memory backend panics on a non-canonical id (`memory.rs:338-340`) | valid-open | `storage/memory.rs:338-339` `VirtualPath::data(format!("issues/{id}.json")).expect(..)`, reached from `load_issue` `:829` with the raw input (fast path `:849-853`). |
| C7 | prefix matching in five functions and four inline sites | valid-open, undercounted | Functions: `commands/mod.rs:1995`, `commands/dependency.rs:458`, `commands/template.rs:911`, `storage/json.rs:1259`, `storage/memory.rs:843`. Inline: `commands/validate.rs:248/262`, `commands/gate_check.rs:722/739`, `storage/claim_coordinator.rs:306-313`, `:813-816`. Unlisted tenth: `commands/dependency.rs:551-572` (dependency-removal prefix match over `issue.dependencies`). The two `claim_coordinator` sites are not user-prefix resolution: they tolerate persisted short-id lease spellings (commit `e17bda300`, jit:a6a9b964). |
| C8 | short id derived by `chars().take(SHORT_ID_LENGTH)` at nine sites and `take(8)`/`[..8]` elsewhere (`validate.rs:1055`, `claim.rs:1057`) | **invalid-as-stated (partly)** | Nine count correct: `types.rs:551,733,795`; `errors.rs:504,557,996`; `domain/artifact_classifier.rs:1715`; `validation/repository.rs:506`; `commands/validate.rs:2628`. `claim.rs:1057` is a test (inside `mod tests` from `claim.rs:815`). Literal production sites: `visualization.rs:242`, `commands/validate.rs:1055`, `commands/graph.rs:326,588`, `main.rs:3369,3807`; `[..SHORT_ID_LENGTH]`: `commands/batch_create.rs:339`, `validation/repository.rs:498`. `validate.rs:2628` sits in `validate_transitive_reduction`, a helper of `validate_integrity_silent` that 842b0ac6 REQ-01 deletes. |
| C9 | `Event::get_issue_id` returns `""` (`types.rs:2002-2010`) | valid-open | `types.rs:1989-2013`; `""` arms `:2001,2006-2011`. Production callers: `repository_state/mutation.rs:1349`, `domain/queries.rs:77`, `commands/events.rs:30`. |
| C10 | `issues/{id}.json` spelled 65× across 19 files, two private `issue_path` helpers | valid-open, count drifted | 76 lines: 33 PROD in 15 files, 43 TEST in 13 files (`tools/ip2.txt`). Four private helpers, not two: `repository_state/mutation.rs:503`, `repository_state/archive.rs:426`, `storage/memory.rs:338`, `storage/json.rs:730`. 842b0ac6 removes the two in `commands/migrate.rs`. |
| C11 | `Event` repeats `id`/`timestamp` in 22 variants; `id()`, `assign_identity()`, `get_issue_id()` are 22-arm matches | valid-open | `types.rs:1621`, `:1653`, `:1989`. 21 after 842b0ac6. |
| C12 | `GateRunResult.by` is `Option<String>` (`types.rs:982`), re-parsed into `Assignee` | valid-open | `types.rs:982`; re-parse `commands/gate_check.rs:229-235` (`gate_state_from_run`). Producers: `gate_execution.rs:175`, `commands/gate_check.rs:56,1225` (all `AUTO_EXECUTOR`), `commands/gate.rs:906` is `SetManualGateStatus.by` (already `Assignee`). |
| C13 | system actors are literals (`claim.rs:530-700`, `mod.rs:729`) | valid-open | `commands/claim.rs:530` `system:list`, `:578` `system:check`, `:630` `system:admin`, `:700` `system:recovery`; `commands/mod.rs:729` `system:lease-check`. `:578` is inside `check_issue_lease`, deleted by 842b0ac6 REQ-19. Related: `gate_execution.rs:24` `AUTO_EXECUTOR: &str = "auto:executor"`. |
| REQ-01 | `IssueId` everywhere; run ids and gate keys typed in the trait; paths derived without validation or panics | valid-open | No id types exist. Gate keys have no grammar at all (`declarations/gates.rs:159` `pub key: String`, no validator). Run ids validated only by `gate_run_result_relative_path` (`mutation.rs:512-520`). |
| REQ-02 | prefix parsed once into `IssueIdPrefix`; resolution `IssueIdPrefix → IssueId` | valid-open | Depends on 16d63038 (Ready, no branch or commit). `MIN_ID_PREFIX_LENGTH` lives in storage (`storage/errors.rs:251`). |
| REQ-03 | only `IssueId` derives the short id | valid-open | 14 production truncation sites beside the three domain methods (C8). Web truncates too: `web/src/components/Issue/IssueDetail.tsx:295,460`, `Graph/nodes/ClusterNode.tsx:106`, `Graph/GraphView.tsx:214,715`. |
| REQ-04 | event issue accessor returns `Option<IssueId>` | valid-open | C9. |
| REQ-05 | `Event` envelope; serialized lines unchanged | valid-open, achievable | Verified (section 4). |
| REQ-06 | lease fields typed | valid-open | C3; producers of non-`wt:<hex>` worktree ids exist (section 4). |
| REQ-07 | record/schema paths from `VirtualPath` constructors, well-known files from constants | valid-open | `VirtualPath` (`repository_state/path.rs:200`) has constructors `worktree` `:330`, `data` `:337`, `from_root` (crate) `:347`, and 15 constants `:288-317`. No issue-record or schema constructor exists. |
| REQ-12 (part) | gate-run actors and system actors are `Assignee`; named constants | valid-open | C12, C13. `Assignee` (`types.rs:281`) holds two `String`s, so it cannot be a `const`. |
| REQ-18 (identity) | raw id strings only at boundaries | valid-open | ~165 production fn signatures take issue ids as `&str`/`String`/`Vec<String>` (37 files); 698 PROD `issue_id`/`full_id` word refs in 36 files. |

On-disk data check (this repository): all 1,185 issue records, every `dependencies` entry, `index.json` `all_ids`/`deleted_ids`, all 16,602 parsed events' `issue_id`/`removed_deps`/`id`, and all 4,639 gate-run `issue_id`/`run_id` are canonical lowercase hyphenated UUIDs. All gate-run `by` values are `auto:executor`. `.git/jit/claims.jsonl` (1,631 lines) holds **26 acquire records whose `issue_id` is an 8-char short id**; the 5 active leases in `claims.index.json` are canonical. Every `agent_id` there parses as an `Assignee` (`agent:*`, `copilot:cli-session`, `test:agent`).

### 2. Prior-art sweep

- `dev/active/842b0ac6-greenfield-cutover/plan.md:169` DEC-domain-path: domain types resolve only as `crate::domain::<Type>`; new types follow it. `:168` DEC-backfill-parse: the backfill tag is skipped by `parse_known_events`'s unknown-tag rule. `:191` DEC-follow-ups: archive short-id destination fallback (`artifact_classifier.rs:417-431`, `storage/artifact_planning.rs:27-75`, `commands/archive.rs:203-224`) was reported, not planned. `:197`: 16d63038 overlaps `IssueStore` edits with `remove-read-lock-sidecar-sweep`.
- `dev/active/842b0ac6-greenfield-cutover/investigation.md:131`: agent id is a validated-then-discarded `String`; typing deferred to this epic. `:546`: `flatten` is used in `visualization.rs`, `output.rs`, `claim_coordinator.rs`.
- `dev/active/b80e3c70-v1-release/handoff-4.md:43-44`: short-id prefix must not be treated as equivalent to a full container slug.
- `dev/archive/7095769d-code-smell-cleanup/active/7095769d-smell-hunt-report.md:29`: introduced `read_worktree_id` (now `storage/worktree_identity.rs:217`).
- a122b9b3 (Rejected, `resolution:ill-planned`): earlier attempt at mechanical semantic-type enforcement. No dev/ doc records an `IssueId` design.
- `storage/reference.rs:56` const-asserts `SHORT_ID_LENGTH >= MIN_ID_PREFIX_LENGTH`; `:517-527` projects both into the storage reference page; `:765-790` test `test_full_id_resolves_only_in_canonical_form` asserts today's case/hyphen behaviour, which 16d63038 flips.

### 3. Consumer sweep

#### Identity values

| Item | PROD | TEST | Files |
|---|---|---|---|
| `issue_id` (word) | 698 | 1,120 | 92 |
| `.id` field access | 477 | 731 | 106 |
| `dependencies` (word) | 285 | 499 | 95 |
| `run_id` | 77 | 85 | 23 |
| `gate_key` | 288 | 113 | 31 |
| `gates_required` / `gates_status` | 73 / 46 | 124 / 96 | 39 / 30 |
| `lease_id` / `agent_id` / `worktree_id` | 110 / 66 / 52 | 99 / 49 / 71 | 12 / 20 / 16 |

PROD `issue_id|full_id` per file: main.rs 88; commands/document.rs 74; commands/mod.rs 69; commands/dependency.rs 65; domain/types.rs 60; commands/gate.rs 59; commands/gate_check.rs 47; errors.rs 45; commands/validate.rs 41; storage/claim_coordinator.rs 35; repository_state/mutation.rs 29; commands/claim.rs 26; output.rs 25; domain/event_catalog.rs 25; commands/issue.rs 25; commands/graph.rs 17; validation/graph.rs 16; domain/type_taxonomy.rs 14; storage/reference.rs 6; graph/hierarchy.rs 6; validation/report.rs 5; gate_execution.rs 5; domain/queries.rs 5; search.rs 4; storage/warnings.rs 3; commands/item.rs 3; cli.rs 3; storage/{memory,json}.rs 2 each; commands/{labels,events}.rs 2 each; validation/repository.rs, storage/mod.rs, schema.rs, commands/template.rs, commands/batch_create.rs 1 each.

Production fn signatures with raw issue-id params (multi-line aware): 165 in 37 files — commands/document.rs 17, domain/types.rs 15, commands/gate_check.rs 13, commands/gate.rs 12, commands/dependency.rs 10, errors.rs 9, storage/claim_coordinator.rs 9, commands/mod.rs 9, commands/template.rs 9, graph/mod.rs 5, commands/validate.rs 5, commands/claim.rs 5, commands/graph.rs 4, commands/issue.rs 4, storage/memory.rs 3, domain/artifact_classifier.rs 3, validation/graph.rs 3, repository_state/mutation.rs 3, and 1-2 in 19 more.

Other issue-id carriers: `RepositoryIndex.all_ids/deleted_ids: Vec<String>` (`repository_state/index.rs:13-15`; consumers in 29 files, heaviest storage/json.rs 40, repository_state/mutation.rs 26, index.rs 18, commands/validate.rs 11); `artifact_plan` `issue: String` (`domain/artifact_plan.rs:290,352`), `PlanTarget::Container { id }`, `artifact_classifier.rs:223`; `validation/report.rs:25,120`, `validation/graph.rs:265`; `domain/store_divergence.rs:76`; output DTOs (`output.rs:1421,1554,1563,1625,1689,1749,1836,1857,2363,2390,2627,2781,2831`); struct fields named `issue_id/from/to/ids` in `cli.rs` 7, commands/dependency.rs 8, commands/mod.rs 14, domain/types.rs 18, type_taxonomy.rs 4, errors.rs 6, storage/warnings.rs 3, mutation.rs 2.

Test literal non-UUID ids (`.id = "dep1"`, `id: "task-1".to_string()`, `dependencies = vec!["…"]`): ~170 lines in 40 files; heaviest domain/queries.rs 27, storage/claim_coordinator.rs 16, output.rs 15, graph/hierarchy.rs 9, repository_state/mutation.rs 8, main.rs 7, commands/gate_check.rs 7, validation/graph.rs 6, storage/repository_state_store_tests.rs 6, storage/gate_runs.rs 6, commands/worktree.rs 6. Rustdoc examples with non-UUID ids: `domain/queries.rs:114-116,416-430`, `graph/hierarchy.rs:110-116`, `storage/claim_coordinator.rs:63-70` (doctest).

#### IssueStore

Implementors: `InMemoryStorage` (`storage/memory.rs:820`), `JsonFileStorage` (`storage/json.rs:1045`), test wrappers `PublicationProbeStore<S>` (`tests/fast_docs_templates/template_apply_atomicity_tests.rs:125`) and `FailingMutationStorage` (`tests/fast_rules/local_rule_enforcement_tests.rs:840`). Call sites (`.<method>(`): `load_issue` 325 (commands/issue.rs 53, commands/gate_check.rs 23, commands/gate/document/archive 13 each, storage/json.rs 11, main.rs 11, commands/claim.rs 8, commands/mod.rs 7, commands/validate.rs 6, commands/dependency.rs 6, commands/breakdown.rs 6, commands/labels.rs 4; tests ~180 in 25 files); `load_issue_or_not_found` 3; `resolve_issue_id` 83 (tests/fast_issue/short_hash_tests.rs 15, commands/claim.rs 11, commands/issue.rs 9, commands/document.rs 9, main.rs 8, commands/gate.rs 6, storage/reference.rs 5, commands/graph.rs 5, commands/gate_check.rs 4, commands/validate.rs 3, breakdown.rs 2, 1 each in mod/item/archive and 3 tests); `load_gate_run_result` 17; `list_gate_runs_for_issue` 36 (commands/gate_check.rs 20); `find_reusable_gate_run` 3. 842b0ac6 deletes `read_issues` (`mod.rs:222`) and `commands/breakdown.rs`.

#### Event

- Variant patterns `Event::<Variant>`: 260 lines, 167 PROD — domain/types.rs 93, domain/event_catalog.rs 44, repository_state/mutation.rs 17, domain/queries.rs 5, domain/event_log.rs 2, and 1 each in storage/mod.rs, repository_state/archive.rs, domain/artifact_discovery.rs, commands/{mod,migrate,archive}.rs. TEST 93 in 28 files (mutation.rs 15, repository_state_store_tests.rs 11, commands/profile.rs 8, commands/issue.rs 8, local_rule_enforcement_tests.rs 7, lifecycle_timestamps_tests.rs 6, …).
- `Event::draft_*` (22 constructors, `types.rs:1689-1980`): 49 PROD calls — commands/mod.rs 18, commands/template.rs 8, commands/gate.rs 5, commands/dependency.rs 4, repository_state/mutation.rs 3, commands/document.rs 3, repository_state/archive.rs 2, commands/issue.rs 2, commands/gate_check.rs 2, repository_state/initialize.rs 1, commands/migrate.rs 1.
- `assign_identity`: 1 caller (`repository_state/mutation.rs:1217`). `get_issue_id`: 3 PROD + 18 TEST. `Event::id()`: no production caller outside tests.
- Writers: canonical `serialize_event` (`repository_state/mutation.rs:571-601`, key-sorting `canonicalize_json`) is the only log writer. Raw `serde_json::to_string(&event)` prints at `main.rs:5800,5821` (`jit events` output). Readers: `domain/event_log.rs:42` (`parse_known_events`, `Value` then `from_value`), plus the f8a6b298-owned second/third parsers `storage/json.rs:1355-1391`, `storage/memory.rs:480-492`.
- Finalizer string tags: `tag: "issue_created".to_string()` (`mutation.rs:779` and batch path) beside `event.get_type()` — EventTag scope of another investigator.

#### Leases and actors

`Lease`: claim_coordinator.rs 55/41/10 PROD refs (lease/agent/worktree), commands/claim.rs, main.rs 24 lease_id, errors.rs, commands/validate.rs, commands/worktree.rs, cli.rs; tests in claim_coordinator.rs, claim_coordinator_proptests.rs, commands/claim.rs, tests/cli_repo_workflow/claim_integration_tests.rs, tests/cli_item_validate/validation_lease_tests.rs, tests/cli_repo_workflow/worktree_{identity,cli}_tests.rs. `ClaimCoordinator::new(paths, locker, worktree_id: String, agent_id: String)` (`claim_coordinator.rs:371-376`), 7 production call sites (`commands/claim.rs:245,534,580,634,702` + acquire/release paths, `commands/mod.rs:732`). `resolve_agent_id -> Result<String>` (`agent_config.rs:88`) validates through `Assignee` and discards it (`:162`). Lease ids: `Uuid::new_v4().to_string()` at `claim_coordinator.rs:486,619,781`.

#### Short id and paths

- Short-id truncation sites: C8. `errors::short_id` helper (`errors.rs:995`) callers `errors.rs:949,954`, `output.rs:32,106,134,1276,1283`. `.short_id()` method calls: 72 PROD, 112 TEST. Short-id recognisers (parse, not truncation): `domain/artifact_conformance.rs:237-241`. Placeholder short id: `commands/batch_create.rs:409` `"0".repeat(SHORT_ID_LENGTH)`.
- `issues/{..}.json` PROD by file: commands/mod.rs 6, commands/gate_check.rs 6, commands/document.rs 3, commands/template.rs 3, commands/gate.rs 2, commands/migrate.rs 2, commands/snapshot.rs 2, commands/validate.rs 2, and 1 each in batch_create, claim, repository_state/{archive,materialize,mutation}, storage/memory, validation/repository. Listing-name spellings `format!("{id}.json")`: `commands/gate.rs:368`, `commands/template.rs:655`, `validation/repository.rs:399`. Host-path spellings: `storage/json.rs:731,904,940`; `ISSUES_DIR`/`GATE_RUNS_DIR` consts `json.rs:34,37` (16 uses), `storage/memory.rs:361,461`.
- Gate-run paths: `gate-runs/{run_id}` at `repository_state/mutation.rs:519,535`, `commands/mod.rs:1985`.
- Schema paths: `repository_state/initialize.rs:341,392,597`, `repository_state/mod.rs:1126`, `repository_state/profile_apply.rs:1881`, `repository_state/materialize.rs:290,538,592,639`, `repository_state/rule_serialize.rs:261` (TOML reference text), `declarations/rules.rs:1525` (`starts_with("schemas/")` parse check).
- Well-known `.jit` file literals outside `path.rs`: 83 PROD lines in 26 files (`tools/wellknown.txt`): profile_apply.rs 17, materialize.rs 17, initialize.rs 7, commands/validate.rs 7, commands/init.rs 7, projection_render.rs 3, and 1-2 in server/watcher.rs, templates.rs, storage/{user_config_store,discovery,json,ruleset_store,gate_store}.rs, config.rs, declarations/invariants.rs, main.rs, repository_state/mod.rs, validation/repository.rs, commands/{gate,mod,config,profile,claim,snapshot,project}.rs, profile/template_region.rs. Many are host-filesystem joins (`jit_root.join("config.toml")`), not `VirtualPath` sites.
- Control-plane files with no `VirtualPath` home: `claims.jsonl`/`claims.index.json` (`claim_coordinator.rs:203,885,936,1269,1376-1384`), `worktree.json` (`worktree_identity.rs:107,123,218`), `server.pid.json`/`server.log` (`commands/serve.rs:70,557`), `server/src/watcher.rs:89`.

#### Boundaries

- Server: ids arrive as `Path<String>` at `crates/server/src/routes.rs:97,597,627,791,836,893,910` and pass to executor methods that resolve internally; graph DTOs `routes.rs:173-205` build `id`/`from` strings and implement no store. Web types keep `id: string`; mcp-server reads no id fields.
- Published schema: `jit --schema` emits `schema_to_value::<Issue>()` (`schema.rs:564,570`); `Issue.id` must stay `{"type":"string"}` (`#[schemars(with = "String")]`, as `InputsDigest`/`ProfileId` do).
- `GraphNode` (`graph/mod.rs:45-51`, `fn id(&self)->&str`, `fn dependencies(&self)->&[String]`) implementors: `Issue` (`types.rs:814`), `EdgeView` (`graph/hierarchy.rs:493`), `ProspectiveNode` (`commands/template_expand.rs:457`, non-UUID placeholder ids `__apply_placeholder__:{role}` `:435`), `TestNode` (`graph/mod.rs:585`). `HierarchyNode` (`graph/hierarchy.rs:158-168`): `Issue` `:170`, `TestNode` `:768`, `PropNode` `:1106`, `FixtureNode` (`tests/fast_issue/hierarchy_vectors_test.rs:41`, reading `test-vectors/hierarchy_resolution.json` ids such as `r-t1`).

### 4. Primitive verification

- **UUID generation.** Issue, gate-run and event ids all come from `IdAuthority::uuid_at` (`repository_state/mutation.rs:149-162`): SHA-256 over seed and index, stamped v4/variant, `Uuid::from_bytes(..).to_string()`. Exposed as `MutationContext::identifier_at` (`:292`) and `allocate` (`:318`). `Issue::draft` sets `id: String::new()` (`types.rs:521-526`); the finalizer overwrites it (`mutation.rs:762-766`, batch `:802-807`). `GateRunResult` drafts set `run_id: String::new()` (`gate_execution.rs:160`, `commands/gate_check.rs:41,1204`), assigned at `mutation.rs:1074-1076`. Events draft with `id: String::new()`/`UNIX_EPOCH`. These empty-string ids are sentinels a validated `IssueId`/`GateRunId` cannot represent. Callers already pre-derive ids before capture (`commands/mod.rs:2291` `context.identifier_at(0)`; `batch_create.rs:307`).
- **Crates.** `uuid = "1.10"` with `v4, serde` is a workspace dep (`Cargo.toml:45`; lock `uuid 1.18.1`). `Uuid`'s serde is human-readable hyphenated lowercase on write, but its parse accepts simple, braced, URN and uppercase forms, so strict on-disk decoding needs a custom check (`to_string() == input`). `semver 1.0.28` is a direct jit dep (`crates/jit/Cargo.toml:54`, not workspace). `schemars 0.8.22`, `serde 1.0.228`, `serde_json 1.0.145`. No `strum`, `serde_with`, `derive_more` or `nutype` in `Cargo.lock`.
- **Transparent serde.** Two in-crate newtype conventions exist: `InputsDigest` (`domain/repository_inputs.rs:345-403`) and `ProfileId` (`profile/manifest.rs:23-49`) use `#[serde(transparent)]` Serialize, `#[schemars(with = "String")]`, `TryFrom<&str|String>` with a `String` error and a hand Deserialize through `TryFrom`; `Assignee` (`types.rs:281-397`) uses `FromStr` with a `thiserror` enum and hand Serialize/Deserialize/JsonSchema. `#[serde(try_from = "String", into = "String")]` would replace the hand impls; no type uses it yet. All on-disk ids are canonical, so a strict canonical decoder keeps every record in this repository readable and unchanged.
- **REQ-05 envelope, verified.** Probe `exp/flat/src/main.rs` with the workspace lockfile: `struct Event { id, timestamp, #[serde(flatten)] body: EventBody }` with `#[serde(tag = "type", rename_all = "snake_case")] enum EventBody`. Results: `from_str` and `from_value` both decode a real `events.jsonl` line (the `parse_known_events` path uses `from_value`); a repo-scoped variant decodes; re-serialization through key sorting is byte-identical to the on-disk line. Bytes are unchanged because `serialize_event` sorts keys (`mutation.rs:576-601`). Raw `to_string` order changes from `type,id,issue_id,timestamp,…` to `id,timestamp,type,…`, which affects only `jit events` stdout (`main.rs:5800,5821`). `schema_for!(Event)` still has a top-level `oneOf` whose variants pin `properties.type`, so `event_catalog.rs:697-733` keeps working; the envelope's `id`/`timestamp` sit in top-level `properties`. Prior art in this crate: `ClaimLogEntry { #[serde(flatten)] operation: ClaimOp }` with `ClaimOp` tagged `op`, whose `Acquire` flattens `Lease` (`claim_coordinator.rs:103-153`). Constraint: flatten forbids `deny_unknown_fields` on `Event` (it has none today).
- **Draft events.** With the envelope, `draft_*` can return `EventBody` and only the finalizer builds `Event { id, timestamp, body }`. That deletes `assign_identity` and the empty-id/epoch sentinel (`mutation.rs:495-500` `sentinel_time`). `MutationIntent::RecordEvent { event }` and `PendingEvent.event` carry the body.
- **REQ-04.** `issue_id()` over the body stays the one exhaustive match (16 `Some`, 6 `None` arms). `event_identities` (`mutation.rs:1348-1361`) sorts by `primary` string; `None` sorts before `Some`, matching today's `""` first.
- **VirtualPath constructor.** An infallible `VirtualPath::issue_record(&IssueId)` is possible only inside the `identity` module (`path.rs:191-319`), which alone can build a `RootRelativePath::Descendant` without `checked`. Outside it, every constructor returns `Result` (`from_parts` `:211`). The id guarantees one safe component, so the record path cannot enter the reserved `tmp` namespace. Host paths derive through `RepositoryLayout::resolve(&VirtualPath)` (`path.rs:530`), which would replace `json.rs:731,904,940`.
- **Const `Assignee`.** Not possible today. `VirtualPath` shows the in-crate way: `Cow<'static, str>` fields built by a `const fn` (`path.rs:249-254`). Alternatives are `LazyLock<Assignee>` or a `SystemActor` enum whose `assignee()` builds the value.
- **Graph genericity.** RPITIT `fn dependencies(&self) -> impl Iterator<Item = &str>` needs Rust ≥1.75; MSRV is 1.97 (`Cargo.toml:7`). `ProspectiveNode` placeholders and the hierarchy test vectors need non-`IssueId` node ids, so the graph traits must stay generic. Use either `&str` views (via `IssueId::as_str`) or an associated `type Id`.

### 5. Architecture fit

- Reuse: `Assignee` (`domain/types.rs:281`) for `Lease.agent_id`, `GateRunResult.by`, `ClaimCoordinator.agent_id`, and `resolve_agent_id`'s return; `InputsDigest`/`ProfileId` as the newtype template (converge the three conventions on one per `@/inv/convention-convergence`); `VirtualPath` + `RepositoryLayout::resolve` for every record path; `IdAuthority` for generation (return `Uuid` and wrap per kind).
- Layer: `IssueId`, `IssueIdPrefix` (with `MIN_ID_PREFIX_LENGTH` moved from `storage/errors.rs:251`), `GateRunId`, `LeaseId`, `WorktreeId`, a gate-key type, and system actor constants belong in `domain` (a new `domain/identity.rs`, imported as `crate::domain::IssueId` per 842b0ac6 DEC-domain-path). `Lease`/`ClaimOp` stay in storage and hold domain types. `VirtualPath::issue_record`/`schema` stay in `repository_state/path.rs` and import `crate::domain::IssueId` (inward). Stale text to sweep when doing this: `domain/types.rs:9-12` says `repository_state::path` "imports nothing from this crate".
- Prefix resolution: one pure domain function over candidate ids (`resolve(prefix, ids) -> Result<IssueId, …>`). Storage and capture callers feed it ids. This is the function 16d63038 creates.
- Delete when lease ids are typed canonical: `ClaimsMutationGuard::resolved_active_leases`'s resolver closure (`claim_coordinator.rs:301-325`) and `is_issue_identity_candidate` (`:813-817`); equality on `IssueId` replaces both.

### 6. Architectural-invariant check

- Domain purity: the new types are pure (string or `Uuid` parsing, no I/O). `uuid` in domain is already present (`types.rs:21`, cfg(test)). Generation stays in `repository_state` (`IdAuthority`).
- Domain-agnostic: issue ids, lease ids and worktree ids are engine mechanism. Gate keys are repository-declared, so a `GateKey` grammar is mechanism only if it is the engine's own token rule (as `ProfileId` uses lowercase-kebab). This repository's gate keys are all lowercase-kebab (20 distinct in issue records). The system actor names (`system:list`, …) are engine-owned audit identities, not domain vocabulary.
- Dependency direction: moving `MIN_ID_PREFIX_LENGTH` and the resolver out of storage into domain points dependencies inward. `VirtualPath::issue_record` importing domain is inward.
- canonical-cutover: delete the persisted short-id lease tolerance (C7 note) instead of carrying it into typed code. That requires a one-time rewrite or truncation of this checkout's `.git/jit/claims.jsonl` (26 records), which `ClaimCoordinator` replays on every lease-enforced mutation (`claim_coordinator.rs:432,701`) until f8a6b298 ends replay.

### 7. Sequencing

Assumes 842b0ac6 and 16d63038 land first. 842b0ac6 shrinks this work: 21 event variants, no `migrate.rs`, no `check_issue_lease`/`system:check`, no `validate_integrity_silent` (`validate.rs:2628` gone), no `read_issues`, one `crate::domain::` path. 16d63038 leaves one resolver, so REQ-02 retypes one function.

The `Issue.id` field flip cannot be split by layer with the compiler green: the experiment shows it cascades through every value that carries an id (293 lib errors before main/server/tests). The workable split is **alias staging**. `crate::domain::IssueId` is first a `pub type IssueId = String;` alias while the validated newtype lives, tested, under `domain::identity`. Layer steps retype fields and signatures to the alias using only operations valid on both (`as_str()`, `Display`, `Clone`, `Eq`/`Ord`/`Hash`), plus a temporary `IssueIdStaging` trait implemented for `String` that provides `short_id()`. The flip step replaces the alias with the newtype and deletes the trait. Alias and trait are a temporary bridge, and step S9 deletes them in this epic. Weakness: staging steps are not compiler-checked. Each is reviewed by grep criteria (no `issue_id: String`/`&str` left in its module family), and S9 catches any misses.

| Step | Outcome | Depends | Files (est.) |
|---|---|---|---|
| S1 | System actors and gate-run `by` (REQ-12 part): const-capable `Assignee` (Cow fields) or `SystemActor`; named constants for list/admin/recovery/lease-check/auto-executor; `GateRunResult.by: Option<Assignee>`; delete the re-parse at `gate_check.rs:229-235` | — | ~10 (types.rs, gate_execution.rs, gate_check.rs, claim.rs, commands/mod.rs, output.rs, storage/reference.rs, gate_runs.rs + 2 test files) |
| S2 | Event envelope (REQ-05) + `issue_id() -> Option<&IssueId-alias>` (REQ-04): `Event { id, timestamp, body: EventBody }`, drafts return `EventBody`, `assign_identity` and `sentinel_time` deleted | — | ~35, mostly a mechanical `Event::X` → `EventBody::X` rename: domain/types.rs, event_catalog.rs, event_log.rs, queries.rs, mutation.rs, archive.rs, storage/{mod,json,memory}.rs, commands ×8, main.rs + ~20 test files. Split if needed: S2a production, S2b tests (S2a cannot be green alone, so one worker or one branch) |
| S3 | Identity types: `domain::identity::{IssueId, IssueIdPrefix, GateRunId, LeaseId, WorktreeId}` with parse/serde/schemars, proptest round-trips; `pub type IssueId = String` staging alias + `IssueIdStaging` trait | — | 3-4 |
| S4 | Graph traits id-agnostic: `dependencies()` returns an iterator of `&str` (or an associated `Id`); adapt `EdgeView`, `ProspectiveNode`, test nodes, server graph DTO | S3 | ~7 (graph/mod.rs, graph/hierarchy.rs, types.rs, template_expand.rs, routes.rs, 2 test files) |
| S5 | Test id fixtures: one shared fixture yielding canonical UUID ids, replacing literal non-UUID issue ids (graph-only tests keep theirs). Split S5a src unit tests / S5b `tests/` | S3 | ~20 + ~20 |
| S6 | Alias staging, domain + storage: `Issue`/`MinimalIssue`/`MinimalBlockedIssue`, `GateRunResult.issue_id`, `EventBody` issue fields, artifact-plan ids, `RepositoryIndex`, `IssueStore` (6 methods) + 4 impls, `Lease.issue_id`; short-id callers to `short_id()` | S2,S3 | ~20 |
| S7 | Alias staging, `repository_state` + validation + errors/output (REQ-03 sites here) | S6 | ~15 |
| S8 | Alias staging, commands (S8a: dependency, gate, gate_check, template, batch_create, bulk_update; S8b: issue, document, claim, validate, graph, archive, labels, events, mod) + main.rs/cli.rs/server routes | S6 | ~10 + ~12 |
| S9 | Flip: newtype replaces alias, staging trait deleted; record paths through `VirtualPath::issue_record` (REQ-07 issue part, ~19 files incl. host paths in json.rs); CLI/server parse to `IssueIdPrefix`; resolver retyped (REQ-02); `allocate`/`identifier_at` typed; issue draft identity per decision D3; memory `expect` and lease short-id tolerance deleted; `claims.jsonl` rewrite | S4-S8 | ~30, the largest step and not splittable once staging is complete |
| S10 | Gate-run ids in trait and records: `GateRunId` in `GateRunResult.run_id`, `GateVerdictOrigin::Reused`, `gate_run_result_relative_path` infallible, output DTOs, server route; run-id draft sentinel per D3 | S9 | ~12 |
| S11 | Gate keys typed (REQ-01 "gate keys in the trait" plus REQ-18). S11a: type + trait + `GateRunResult` + events + registry; S11b: `Issue.gates_required/gates_status` + declarations + commands | S9, D4 | ~15 + ~20 |
| S12 | Lease typing (REQ-06): `LeaseId`, `WorktreeId`, `Assignee` in `Lease`/`ClaimOp`/`ClaimCoordinator`/`WorktreeIdentity`; `resolve_agent_id -> Assignee` (seam 842b0ac6 REQ-05) | S9, D5 | ~15 |
| S13 | Schema paths and well-known constants (REQ-07 rest): `VirtualPath::schema(..)`; replace `.jit/<file>` literals with constants (S13a repository_state, S13b commands/storage/server). Independent of IssueId; profile_apply sites overlap the profile investigator's REQ-08 | — | ~10 + ~16 |
| S14 | REQ-18 identity sweep and grep-based owning check | all | ~3 |

Decisions referenced above:
- D1 `IssueId` representation. Option (a): canonical-string newtype (InputsDigest pattern), with free `as_str()` and `short_id() -> &str`. Option (b): `Uuid` inner, which is `Copy` and needs no `as_str` borrow. Recommend (a), with strict canonical decode.
- D2 Staging strategy. Option (a): alias staging (above). Option (b): one big-bang flip spread over several non-green commits. Recommend (a).
- D3 Draft identity for issues and gate runs, whose empty-string ids are sentinels. Option (a): drafts carry the pre-derived `identifier_at(i)` id. Option (b): separate draft types without an id. Option (c): a nil-UUID placeholder, which is a protocol sentinel. Recommend (a).
- D4 Gate-key grammar. Lowercase-kebab like `ProfileId`, or non-empty only. Alternatively scope gate keys out of this epic beyond the trait.
- D5 `WorktreeId` producers. Two fallbacks emit ids that are not `wt:<8 hex>`: `commands/worktree.rs:282` (`wt:{branch}`) and `commands/mod.rs:731` (worktree root path). Either remove the fallbacks or widen the grammar.
- D6 Web short-id truncation (5 sites). Either serve `short_id` from the server or declare the web a rendering boundary outside REQ-03.
- D7 Event naming. REQ-05 read literally makes `Event` the envelope, which renames ~260 `Event::X` patterns. Naming the envelope `EventRecord` and keeping `Event` as the body would leave most patterns untouched but departs from the criterion's wording.
- D8 Control-plane files (`claims.*`, `worktree.json`, `server.pid.json`) are not repository content. Either scope them out of REQ-07 or give them a separate constant set.
- D9 Data rewrite of this checkout's `.git/jit/claims.jsonl` (26 short-id records) at S9.

Parallelism: S1, S2, S3, S13 are mutually independent. S6-S8 can run in parallel after S3 (alias = String). S10-S12 are parallel after S9.

Seams with in-flight work:
- 16d63038: overlaps exactly REQ-02's ten sites; must land before S8/S9, or this epic must absorb it, which contradicts D-b.
- f8a6b298 (depends on this epic): restructures claims and gate-run storage. S12 should type fields and constructors only and not refactor lease sequences, which f8a6b298 rewrites. The envelope needs no change to `parse_known_events`.
- aa423d02 (Ready): edits `CommandExecutor::claim_next` (`commands/issue.rs:737`), a text-conflict seam with S8b only.
- 7d1bfe89: query lexer; no identity seam.
- d23c58c0: code merged to main (`7691e0e09`, ancestor of main); no seam.
- 842b0ac6: S2 must follow its deletion of `LifecycleTimestampsBackfilled`; S12 follows its `resolve_agent_id` unification (its REQ-05).

## Part: Profile identity, reserved targets and action owners

### Scope

Profile identity, applied-profile records, versions, reserved application targets, repository action owners. Criteria: REQ-08, REQ-09, REQ-10, REQ-11 (repository action owner), REQ-18 (profile part). Verified against the working tree at `ef16d7c5f`. All paths are under `crates/jit/src/` unless stated otherwise.

### Upstream state: 3d9a0ce3, e643ef66 and charter D-8

- **Charter D-8** (`dev/vision/9db27a3a-charter.md:145-185`, revised 2026-10-15) already decides the surface. Its Consequences paragraph (`:178-183`) deletes "applied-profile records, claims and fingerprints, profile validation and its checks inside `jit validate`, three-way publication, conflict remedies, capture's reconciliation, reconfigure, and upgrade". It keeps the following: one-time apply; variable resolution; composition in declared dependency order, where the dependent package wins; refusal when two unrelated packages declare one key differently; `jit profile diff`; pack and add; and an event recording what was applied (`:149-162`).
- **3d9a0ce3** (epic, `backlog`, assignee `agent:planning-lead-3d9a0ce3`). REQ-04, "Profile packages apply once, with no ownership records to keep in sync (`@/charter/D-8`)", is the criterion that deletes `profiles/{id}.json` and its readers. REQ-09 ("one package-graph error type, one conflict-origin type, ...") also touches this scope: `ProfileGraphError`, plus `ContributionConflictOwner`/`ProfileConflictOccupant`, which both hold `ProfilePackageId`.
- **Is it planned? No.** A plan bracket exists: `b5ba1b61` (planning) and `a3437bab` (breakdown), both `backlog`. `dev/active/3d9a0ce3-lead-friction/` does not exist. The children filed so far cover gates, documents, emoji, rejection and checker faults (`3754095f`, `66986f88`, `72d89161`, `8673dc5e`, `da12ad66`, `f37d228e`, `fd9bfab8`) plus the study `e643ef66`. No child implements REQ-04, REQ-06, REQ-07 or REQ-09 through REQ-13.
- **e643ef66** (study, `ready`, not started). No design note exists. Its REQ-03 files the implementation issues only "after the maintainer's ruling". The premise that "the design is decided by study e643ef66" cannot be verified: the study has produced nothing. The decision that governs this scope is the text of D-8 itself.
- **This repository holds no applied-profile record.** `.jit/profiles/` is absent, which matches `dev/active/842b0ac6-greenfield-cutover/investigation.md:662`.

#### Profile surface after D-8 (derived from the charter plus code)

| Construct | Survives D-8? | Basis |
|---|---|---|
| `ProfileId` (`profile/manifest.rs:26`) | yes | Manifest identity, graph keys (`profile/graph.rs:36-40`), the event payload `ProfileLifecycleProfile.id` (`domain/types.rs:1183`), `ProfileSelector::Id` (`commands/profile.rs:122`) |
| `ProfilePackageId` (`repository_state/profile_apply.rs:608`) | no as a twin; its role survives | ~95% of its references are ownership claims, ownership maps and record readers. The conflict origin it carries in `ContributionConflictOwner::Package` (`profile_apply.rs:245`) and `ProfileConflictOccupant::Package` (`:590`) survives as D-8's "refuses two unrelated packages", and 3d9a0ce3 REQ-09 merges those two types into one |
| `AppliedProfileRecord` (`profile_apply.rs:1395`), the `profiles/{id}.json` path, every reader, `VirtualPath::PROFILES` (`repository_state/path.rs:300`) | no | D-8 Consequences |
| Package version, `compatible-jit`, dependency and incompatibility requirements (`profile/manifest.rs:136-178`) | yes | D-8 composes "in declared dependency order", so graph resolution (`profile/graph.rs:50-160`) survives |
| Upgrade version check (`commands/profile.rs:2275-2305`), record version fields (`profile_apply.rs:1402,1404,1514`), `ProfileAgreement` (`profile/agreement.rs:245`) | no | upgrade and record are deleted |
| `ProfileSelector::Id` resolution "through this repository's applied records" (`commands/profile.rs:121`, `:1685-1700`, `:1765-1772`) | **undetermined** | Once records are gone, an `id:` selector has no resolution route. D-8 does not say what replaces it, so 3d9a0ce3 must decide |

### 1. Claim classification

Summary: 9 claims or REQs are valid-and-open, 2 are invalid-as-stated, none is already done. The 3d9a0ce3 rework removes 2 claims (C2, C3) entirely and shrinks C1, C4 and C5. 842b0ac6 removes only the V1 manifest wire structs (`profile/wire.rs:71-184`), which carry two of the `String` version fields.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| C1 | `ProfilePackageId` is an unvalidated twin of `ProfileId`, constructed at 37 sites, mostly by stringifying a validated `ProfileId` | **valid-and-open** | Definition: `profile_apply.rs:603-626`. It derives `Deserialize` with no validation and has `new(impl Into<String>)`. 71 references in 8 files (`profile_apply.rs` 49, `commands/profile.rs` 7, `profile/apply_claims.rs` 3, `main.rs` 3, `repository_state/mod.rs` 3, `materialize.rs` 4, `initialize.rs` 1, `profile/difference.rs` 1). 38 `ProfilePackageId::new` calls, of which **4 are production**: `profile/apply_claims.rs:109,128` and `profile_apply.rs:2209,2652`, each `ProfilePackageId::new(x.id.to_string())` over a validated `ProfileId`. The other 34 are tests (`profile_apply.rs` ≥3547: 20; `commands/profile.rs` ≥3038: 6; `main.rs:9305,9331`; `materialize.rs:1015,1740,1742`; `initialize.rs:1288`). **3d9a0ce3 deletes nearly all of it** |
| C2 | The `profiles/{id}.json` path is spelled at 8 production sites | **valid-and-open (count is 9)**; deleted by 3d9a0ce3 REQ-04 | `profile/package_capture.rs:510`, `commands/init.rs:404`, `commands/validate.rs:406`, `profile_apply.rs:1499,2044,2081,2122,2628`, `commands/profile.rs:2467`. Test sites: `initialize.rs:1294`, `materialize.rs:1021`, `profile_apply.rs:3602,3886,3924,4687`, plus `.jit/profiles/...` joins in `commands/profile.rs` and `commands/init.rs` tests |
| C3 | Seven readers parse that record, each validating differently | **invalid-as-stated (there are 9)**; all deleted by 3d9a0ce3 REQ-04 | `profile_apply.rs:2053,2090,2131` (three `recorded_*` helpers, 2039-2140), `profile_apply.rs:2603`, `initialize.rs:645,1096`, `commands/profile.rs:2994` (`read_applied_record`, which checks `record.id.as_str() == id` at `:2996`), `profile/package_capture.rs:528`, `commands/validate.rs:887` (`captured_applied_record`, 878-890). The claim's sites are correct but incomplete |
| C4 | Versions are `String`. Validation parses and discards them, and graph resolution re-parses them behind four `ProfileGraphError::Invalid*` variants that a validated package cannot produce | **valid-and-open** | Fields: `profile/manifest.rs:136` (`version`), `:139` (`compatible_jit`), `:168` (dependency `version`), `:177` (incompatibility `version`). Parse-and-discard: `profile/package.rs:634-661`. Re-parse: `profile/graph.rs:65,72,98,105,137,145`. Variants: `graph.rs:221` `InvalidPackageVersion`, `:228` `InvalidEngineRequirement`, `:249` `InvalidDependencyRequirement`, `:269` `InvalidIncompatibilityRequirement`. A fifth parse site exists at `commands/profile.rs:2283,2290` (upgrade), which D-8 deletes. Wire structs: `profile/wire.rs:207-228` (V2) and `:178-184` (V1, deleted by 842b0ac6) |
| C5 | Reserved application targets are matched as raw `.jit/...` prefixes before classification at `commands/profile.rs:3018` and `init.rs:400` | **invalid-as-stated in part** | **One** matcher exists: `commands/profile.rs:3017-3035` (`reject_reserved_application_targets`). It has two **call sites**: `commands/profile.rs:2054` and `commands/init.rs:400`. "Raw prefix before classification" is correct: the matcher runs on `package.hashes().targets` keys (`BTreeMap<String, PackageHash>`, `profile/package.rs:36`), and `classify_repository_relative` runs afterwards (`commands/profile.rs:2062-2069`). The list holds `.jit/events.jsonl`, `.jit/profiles[/…]`, `.jit/tmp[/…]` and `.git[/…]`. **No test covers `ReservedApplicationTarget`**: grep finds only the definition and the two calls |
| C6 | Repository action owners are strings (`image.rs:1096`), and behaviour branches on `matches!(action.owner(), "default-rule-membership" \| "default-schema")` (`initialize.rs:560`) | **valid-and-open** | `image.rs:1096` is `TargetClaim.owner`. The `RepositoryAction` variant fields are `image.rs:1229,1235,1245,1252,1257`, and `owner()` is at `:1351`. The branch is at `initialize.rs:560`, inside `coupled_default_repair_profile` (`:554-567`), whose caller is `:515` |
| REQ-08 | One `VirtualPath` predicate, applied after classification | **valid-and-open** | No `VirtualPath` predicate exists. The precedent shape is `is_bootstrap_lock(&VirtualPath)` (`repository_state/path.rs:28-31`) over the private `enters` (`:34-37`) |
| REQ-09 | One profile identity type, validated at construction, in the domain layer, with no unvalidated twin and no string comparison of profile ids | **valid-and-open** | `ProfileId` is validated (`manifest.rs:35-55`, deserialisation at `:115-122`) but lives in `profile/`, not in `domain/`. `domain/types.rs:1183` names `crate::profile::ProfileId`, an outward dependency from domain. String comparisons: `profile_apply.rs:2521,2711` and `commands/profile.rs:1696,2996` (all record code, deleted by D-8), plus `commands/profile.rs:1767,1778` (`dependency: &str`, `:1760`; dependency resolution, which survives in some form) |
| REQ-10 | Versions and requirements are semver types parsed once at decode, with no unparseable-version graph variant | **valid-and-open** | See C4 and Primitive verification |
| REQ-11 (owner) | Repository action owner is one enum | **valid-and-open** | 14 distinct production spellings; see the consumer sweep |
| REQ-18 (profile) | Raw profile id and version strings only at boundaries | **valid-and-open** | Interior strings that survive D-8: `profile/application.rs:43,121,139,274,462,495,527` (`id: String` in result types) and `:45,123,141,276,464,497,529` (`version: String`); `ProfileGraphError` `package`/`dependency`/`other`/`id: String` (`graph.rs:218-284`); `ProfileDependencyError` (`commands/profile.rs:262-300`); `ProfileSelector::id(impl AsRef<str>)`, which rebuilds the string and re-parses it (`commands/profile.rs:190-193`); `resolve_dependency_package_from(dependency: &str)` (`:1760`); `packaged_manifest_path(id: &str)` (`profile/contribution_drift.rs:253`) |

### 2. Prior-art sweep

- `dev/vision/9db27a3a-charter.md:145-185`: D-8, which decides the profile surface.
- `dev/active/842b0ac6-greenfield-cutover/investigation.md:628-687`: deleting the V1 manifest wire. Records that the dogfood manifest edit changes package hashes but not target hashes (`:661`), that this repository has no applied record (`:662`), and that the 3d9a0ce3 REQ-04/06/07/09-13 children are unfiled (`:684`). `:921` records the convention that a type added to `domain/types.rs` is reachable only as `domain::X`, and a new submodule only by its module path.
- `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md:31`: the origin of the "producer/ownership identity" on each action. `:57` lets schema deletion rely on "recorded producer ownership". That is a design intent; the current code does not read owners for it (see section 3).
- `dev/archive/c639cfb5-jit-profiles-complete/active/{plan.md,investigation.md,handoff-4.md,handoff-6.md}` and `dev/archive/e204e63d-derived-package-sources/active/*`: history of the ownership lifecycle that D-8 rejects. None of it records an identity-typing decision.
- `docs/` has no hit for `ProfilePackageId`, reserved application targets or action owners.

### 3. Consumer sweep

#### `ProfileId` (moves to domain, REQ-09)

68 references in 13 files, all in `crates/jit/src`: `profile/manifest.rs` 10, `profile/graph.rs` 10, `commands/profile.rs` 10 (production lines 8, 122, 180, 1643, 1679, 1711-1713, 1761, 2216), `profile_apply.rs` 8 (`:4412,4453,4476` are tests), `repository_state/mod.rs` 7 (`:582,617,637,653,682,716,752`), `profile/difference.rs` 6 (`:28,47,59,70,89,109`), `profile/wire.rs` 6, `profile/package.rs` 4, `initialize.rs` 2, `profile/package_archive.rs` 2 (`:63,159`), `profile/package_capture.rs` 1, `profile/mod.rs` 1 (re-export), `domain/types.rs` 1 (`:1183`). It appears 0 times in `crates/server`, `crates/jit/tests`, `mcp-server`, `web` and `scripts`, where only the wire string travels.

`is_lowercase_kebab` (`profile/manifest.rs:397`) is used by `ProfileId` (`:39`) and `RegionId` (`:74`). It must move with `ProfileId`, or be exposed from domain, to avoid a copy.

#### `ProfilePackageId` (deleted, REQ-09)

Files and counts are in C1. Production references outside `profile_apply.rs`: `profile/apply_claims.rs:8,109,128` (claims, deleted by D-8), `repository_state/mod.rs:194,625,645` (`owners: BTreeSet<ProfilePackageId>`, ownership, deleted), `profile/difference.rs:225` (`owner_names`). Production references in `profile_apply.rs` (<3547): 214, 236, 245, 252, 289, 312-374, 590, 608-626, 1013, 1643-1817, 1915-2016, 2041-2246, 2404-2537, 2635-2711, all ownership or record code except the conflict-owner enums at `:241-252` and `:586-600`.

#### Version fields (REQ-10)

- Model and wire: `profile/manifest.rs:136,139,168,177`; `profile/wire.rs:207-228` (V2) and `:178-184` (V1, deleted by 842b0ac6).
- Parse sites: `profile/package.rs:634-661`; `profile/graph.rs:65-152`; `commands/profile.rs:2283,2290` (D-8 deletes).
- `ProfilePackageError` variants that become unreachable once serde parses at decode: `profile/package.rs:306` `InvalidVersion`, `:314` `InvalidCompatibility`, `:322` `InvalidDependencyRequirement`, `:332` `InvalidIncompatibilityRequirement`. They become serde decode errors instead. Unlike the graph variants, REQ-10 does not forbid these, but they duplicate the decode error.
- `.version` and `compatible_jit` readers in production: `commands/profile.rs` (815, 896, 978, 1151, 1266, 1983, 2024, 2711-2713, 2899, 2973-2974 survive; 2258-2301, 2744 and 2932-2933 are record or upgrade code), `commands/init.rs:413-414`, `commands/validate.rs:1009`, `profile/package_archive.rs:161,346,708-709`, `initialize.rs:155,652,1100`, `profile_apply.rs:1402,1478,1514,1627-1628` (record or input), `profile/application.rs` (7 result-type `version: String` fields).
- Tests that assert `Invalid*`: none in `crates/jit/tests`, `crates/server`, `docs` or `mcp-server`. The four graph variants are constructed only at `graph.rs:66-146`.

#### Reserved targets (REQ-08)

`commands/profile.rs:401` (`ProfileApplyError::ReservedApplicationTarget`), `:2054` (call), `:3017-3035` (matcher), `commands/init.rs:400` (call). Zero tests.

#### Repository action owners (REQ-11)

Production spellings (struct-literal sites; the `RepositoryAction::*` constructor fns at `image.rs:1267-1335` are `pub(crate)` and called only from tests):

| Spelling | Declared at | Used at |
|---|---|---|
| `repository-state-mutation` | `mutation.rs:491` const `OWNER` | `mutation.rs:751,964,988,1011,1037,1066,1083,1088,1238,1292,1338` |
| `repository-init` | `initialize.rs:47` `SCAFFOLD_OWNER` | `initialize.rs:359-396` (scaffold table), `:1168`, `:1239` (`file.owner`) |
| `profile-application` | `initialize.rs:49` `PROFILE_OWNER` | `initialize.rs:940,947,979` |
| `gitattributes-merge-driver` | `initialize.rs:51` | `initialize.rs:813` |
| `archive-execution` | `archive.rs:17` | `archive.rs:84,130,259,391` |
| `package-capture` | `package_tree.rs:24` | `package_tree.rs:292,318` |
| `repository-export` | `export.rs:11` | `export.rs:182,192,202,210,219` |
| `data-root-ignore` | `materialize.rs:383` | `materialize.rs:450` |
| `configured-projection` | literal | `materialize.rs:371` |
| `default-rule-membership` | literal | `materialize.rs:520`; matched at `initialize.rs:560` |
| `default-schema` | literal | `materialize.rs:548,599`; matched at `initialize.rs:560` |
| `authored-config` | literal | `repository_state/mod.rs:903` |
| `profile-repair` | literal | `repository_state/mod.rs:1056` (profile repair; D-8 deletes profile validation) |
| copied | n/a | `initialize.rs:757,767` (`owner.clone()` when rebuilding a `SetMode`) |

Readers of `owner()`: `image.rs:1437` (`validate_owner`, `:1491-1495`, `DeltaError::InvalidOwner` `:1580`) and `initialize.rs:560`. **No redo record, wire output or docs carry the owner**: `storage/redo_record.rs` and `redo_apply.rs` have no `owner`; grep for `"owner"` in tests, server, mcp-server, web and docs finds only unrelated hits. `RepositoryAction` derives `Serialize` (`image.rs:1223`), but no production serialisation of an action was found.

Test-only owner literals (they must become variants or a test helper): `image.rs:2009-2272` (~14), `repository_state/mod.rs:1279-1396` (8), `overlay.rs:224`, and `storage/repository_state_store_tests.rs` (35 constructor calls).

Adjacent but separate vocabularies, outside REQ-11 as written:
- `TargetClaim.owner` (`image.rs:1096`): `format!("profile-asset:{}")` and `format!("profile-region:{}")` in `profile/apply_claims.rs:76,101`. These are profile claims, which D-8 deletes.
- `ManagedDocumentClaim` `owner: String` (`managed_document.rs:21,28`), fed by `profile_apply.rs:1831`. Region owners take part in delimiter conflict detection (`managed_document.rs:133-147`), so they are data identities, not a closed vocabulary.

### 4. Primitive verification

- **semver**: `crates/jit/Cargo.toml:54` `semver = "1.0"`, locked at 1.0.28 (`Cargo.lock:2386`). Not a workspace dependency. **Only the `default` feature is enabled** (`cargo tree -e features -i semver`), so `Version` and `VersionReq` have no serde impls today. "Parsed once at decode" needs `features = ["serde"]`, which the semver 1.0.28 manifest declares as `serde = ["dep:serde"]`, or a `#[serde(deserialize_with)]` adapter.
- **schemars**: the workspace pins `0.8` with `chrono` (`Cargo.toml:42`), locked at 0.8.22. Its `semver` feature implements `JsonSchema` for `semver::Version` only (`schemars-0.8.22/src/json_schema_impls/semver.rs:4`), not for `VersionReq`. `#[schemars(with = "String")]`, already used for `ProfileId` (`manifest.rs:24`), keeps the published schema unchanged for both types without adding a feature.
- **Ordering**: `semver::Version` derives `Ord` (semver `lib.rs:157`), and `VersionReq` does not (`:308`: `Default, Clone, Eq, PartialEq, Hash`). The incompatibility sort in `graph.rs:127-133` compares `version` strings, so it needs a key once the field is a `VersionReq` (for example `to_string()`), or the sort must be dropped.
- **Serialised shape unchanged**: `ProfilePackageModel` is `Serialize` and `JsonSchema` (`manifest.rs:129`), and tests check instances against its schema (`profile/package.rs:1248-1324`). With semver's serde feature, `Version` and `VersionReq` serialise as strings, so the shape is unchanged. The package identity hash frames the manifest bytes (`identity_manifest`, `wire.rs:155-157`), not the model, so hashes do not move. One caveat: `VersionReq`'s `Display` normalises (for example `"1.2"` renders as `^1.2`), so a model→JSON round trip may differ from the authored spelling. Check the wire test at `wire.rs:347`.
- **EngineVersion** (`graph.rs:13-31`) already wraps `semver::Version` and can be reused for the `compatible-jit` match.
- **Validated at construction**: `ProfileId` already validates in `TryFrom` and `Deserialize` (`manifest.rs:35-55,115-122`) and has `Display` and `AsRef<str>` (`:103-113`). REQ-09 is a relocation plus deletion of the twin. No new validation is needed.
- **VirtualPath predicate**: `.jit/tmp` already fails classification. `VirtualPath::data` → `checked()` rejects `RECORD_SCRATCH_DIR` (`path.rs:233-242`), and `classify_repository_relative` returns `ReservedControlPath`, so the `.jit/tmp` arm of the raw matcher is redundant. The remaining arms map to `VirtualPath::EVENTS` (`path.rs:294`), `VirtualPath::PROFILES` (`:300`, deleted by D-8), and a worktree `.git` component (no constant; `enters(relative, ".git")` with root `Worktree`). "Applied after classification" means `reject_reserved_application_targets` takes the `Vec<VirtualPath>` that `commands/profile.rs:2062-2069` already builds, and `init.rs:400` must classify first (`init.rs:398-407` has a `layout`).
- **strum / serde_with**: not checked. The owner enum needs neither: a plain enum with an `as_str` or `Display` for messages is enough, because owners have no wire form.

### 5. Architecture fit

- `ProfileId` belongs in `domain/`, either as a new `domain/profile_id.rs` submodule or in `domain/types.rs`, under the 842b0ac6 path convention (`investigation.md:921`). Precedent: `ProfileOrigin` already lives in domain and is re-exported by `profile/mod.rs:41`. Moving it fixes the outward `domain → profile` dependency at `domain/types.rs:1183`. `ProfileVariableName` and `VariableSource` at `domain/types.rs:1173,1175` show the same pattern but are outside this scope; flag them to the inv-identity or inv-vocab owner.
- Version types stay in `profile/` (the package model), or move to domain with `ProfileId` if the domain event payload (`ProfileLifecycleProfile`) ever carries a version. Today it does not (`domain/types.rs:1179-1188`).
- The reserved-target predicate belongs on `VirtualPath` in `repository_state/path.rs`, next to `is_bootstrap_lock`. The error variant stays in `commands/profile.rs:401`.
- The action-owner enum belongs in `repository_state/image.rs`. Its variants name the repository_state producers, which are engine mechanism rather than domain vocabulary, so `@/inv/domain-agnostic` is satisfied.

### 6. Architectural-invariant check

- Domain purity: `ProfileId` and `is_lowercase_kebab` are pure string validation and fit in domain. Moving them removes a domain→profile edge without adding I/O.
- Domain-agnostic: the owner variants are engine producer names, not repository vocabulary. Profile ids are repository-declared data and fit too.
- Canonical cutover: deleting `ProfilePackageId` needs no compat reader. It is `serde(transparent)` over the same string as `ProfileId`, so every serialised shape (`ProfileConflictOccupant`, `ContributionConflictOwner`, owners) stays the same. `ProfileId` deserialisation now rejects non-kebab strings where `ProfilePackageId` accepted them. Only record code deserialises those, and D-8 deletes it.

### 7. Sequencing

**Recommended edge:** the ProfilePackageId and version steps (P3, P4) depend on the 3d9a0ce3 task that implements its REQ-04 (ownership-record deletion). That task is not yet filed; 3d9a0ce3's planning node `b5ba1b61` must create it. A joint plan is unnecessary. P1, P2 and P5 below are independent of D-8 and can land now. Done first, P3 and P4 would retype about 70 references (C1) and the record and upgrade version code (`commands/profile.rs:2258-2305`) that D-8 deletes. They would also collide with 3d9a0ce3 REQ-09's rework of `ProfileGraphError` and the conflict-owner enums in the same files.

| Step | Outcome | Depends on | Files (est.) |
|---|---|---|---|
| P1 | Move `ProfileId` and `is_lowercase_kebab` into domain; `RegionId` imports the predicate; update every `ProfileId` import path, including `domain/types.rs:1183` | 842b0ac6's re-export removal, so the path convention is settled; otherwise none | ~14 |
| P2 | One `VirtualPath::is_reserved_application_target` predicate (EVENTS, PROFILES, worktree `.git`), applied to the classified targets at `commands/profile.rs:2054-2069` and `init.rs:398-407`; drop the redundant `.jit/tmp` arm; add the first test for `ReservedApplicationTarget` | none. 3d9a0ce3 REQ-04 later drops the PROFILES arm with `VirtualPath::PROFILES` | 3-4 |
| P5 | `RepositoryActionOwner` enum: retype `RepositoryAction` owner fields (`image.rs:1229-1257`), delete `validate_owner` and `DeltaError::InvalidOwner`, replace the `initialize.rs:560` string match with a variant match, convert the 13 production spellings and the test literals | none. The `ProfileApplication`/`ProfileRepair` variants are later deleted by 3d9a0ce3 REQ-04 if their producers go | ~12 (8 production + `image.rs`/`mod.rs`/`overlay.rs` tests + `storage/repository_state_store_tests.rs`). Splitting is not needed |
| P3 | Delete `ProfilePackageId`; the surviving conflict-origin type (3d9a0ce3 REQ-09) holds `ProfileId`; type the result-type ids in `profile/application.rs` and the remaining `&str` dependency or selector ids | P1; 3d9a0ce3 REQ-04 and REQ-09 | ~6 after D-8 (estimate; depends on what D-8 leaves) |
| P4 | Semver `Version`/`VersionReq` in `ProfilePackageModel`, the dependency and incompatibility structs, and the V2 wire structs (serde at decode); enable semver `serde`; delete the four `ProfileGraphError::Invalid*` variants and the `package.rs:634-661` re-validation; type the result-type versions; fix the `VersionReq` sort key | 842b0ac6 REQ-06 (V1 wire deletion); 3d9a0ce3 REQ-04 (upgrade and record code); serialise with 3d9a0ce3 REQ-09 (one graph error type) on `graph.rs` | ~9 (`Cargo.toml`, `manifest.rs`, `wire.rs`, `package.rs`, `graph.rs`, `application.rs`, `commands/profile.rs`, `commands/init.rs`, `package_archive.rs`) |

Fold P3 into the 3d9a0ce3 REQ-09 conflict-origin unification: the merged type can hold `ProfileId` from birth instead of being retyped afterwards. No step needs a temporary bridge.

## Part: Closed vocabularies, wire tables and declarations

### Scope and baseline

Scope: REQ-11 (every vocabulary except repository action owner), REQ-12 (query assignee filter only), REQ-13, REQ-14, REQ-15, REQ-16, REQ-17, and the vocabulary part of REQ-18.

Verified against the working tree at `main` 1ed0a049c. Main moved during the investigation (6ff14f21 merged). That merge touches `config.rs` and `validate.rs` but none of the constructs below. Paths are relative to `crates/jit/src/` unless they start with `crates/`, `docs/`, `web/` or `mcp-server/`.

Premise checks for D-a (842b0ac6 lands first). None of this has landed yet:
- `State::from_str` still accepts `open` and `inprogress` (`domain/types.rs:162-164`).
- `KindScope::parse` and `parse_for` are at `domain/item.rs:398-418`.
- `HierarchyError` and `ConfigError::DuplicateType` are at `domain/type_taxonomy.rs:124-128`.
- `MinimalIssue::state_symbol` is at `domain/types.rs:746`.
- `GateRunStatus::Skipped` is at `domain/types.rs:1111`.
- `Event::LifecycleTimestampsBackfilled` exists.
- The raw rule tables are still untyped (`declarations/rules.rs:923-927`).

**The dependency graph does not encode D-a.** 2195c98f depends only on its breakdown 31b4f30e, and 842b0ac6 only on 6792cc41. There is no edge from 2195c98f to 842b0ac6.

### 1. Claim classification

Tally: 19 background claims in scope, all checked: 0 already-done, 18 valid-and-open, 1 invalid-as-stated (C21). All 8 REQs in scope are valid-and-open.

| # | Claim | Class | Current evidence |
|---|---|---|---|
| C1 | The state token is spelled four times | valid-and-open, **undercounted** | Eight spellings exist:<br>• `State::as_str` `domain/types.rs:73-83`<br>• `errors::state_name` `errors.rs:975-985`<br>• `rules.rs` `state_token` `:338-349`<br>• `VALID_STATE_TOKENS` `:354-362`<br>• the inverse parser `token_to_state` `rules.rs:370-382`<br>• `State::from_str` `types.rs:156-170`<br>• `query_engine/error.rs:12-13` `STATE_VALUES`<br>• the mermaid class names `visualization.rs:190-196`, which spell `inprogress`<br>842b0ac6 REQ-09 shrinks `from_str` and REQ-12 owns the mermaid table. |
| C2 | `errors::gate_status_name` copies `GateStatus::as_str` | valid-and-open | `errors.rs:987-993`; `GateStatus::as_str` `types.rs:257-263`. |
| C3 | `strictness` and `content_format` are strings, re-parsed on every access | valid-and-open | Fields at `config.rs:267,274`. `deserialize_strictness` (`config.rs:233-243`) parses, then keeps the raw `String`. `ValidationConfig::strictness()` (`:302`) and `content_format()` (`:283`) parse again. `Strictness` has no serde, plus a hand-written `token` (`validation/strictness.rs:68-74`) and a case-insensitive `FromStr` (`:92-106`). `ContentFormat::from_str` also accepts `md` and any case (`types.rs:224-237`). |
| C4 | `Transform::kind` is a string, re-parsed | valid-and-open | `templates.rs:384-389`. Parsed at load (`templates.rs:603`) and again at apply (`commands/template_expand.rs:242-249`). `TransformKind` has hand-written token constants: `MOVE_UPSTREAM_TO_ROLE` and `SUPPORTED` (`templates.rs:167-182`). |
| C5 | Built-in finding severity and disposition are literals at four `gate_check.rs` sites | valid-and-open | The four sites:<br>• `commands/gate_check.rs:622-629` (rule report)<br>• `:662-664` (repository integrity)<br>• `:1123-1125` (label target)<br>• `:1148-1150` (review placeholder)<br>The engine branches on a string at `:1188` (`== Some("blocking")`). The built-in `origin` values (`issue-impact`, `pre-existing`) are literals at the same sites. |
| C6 | `GateRunResult.by` is `Option<String>` and is re-parsed into `Assignee` | valid-and-open | Field at `domain/types.rs:982`; re-parsed at `commands/gate_check.rs:231-236`. The production writers emit only `AUTO_EXECUTOR` (`gate_execution.rs:24,175,1513`). The view copy is `output.rs:2502`. |
| C7 | The query language holds `Assignee(String)` and hand-copied state and priority lists | valid-and-open | `query_engine/parser.rs:39,142`; `query_engine/error.rs:12-17`. The comment at `error.rs:8-11` documents the alias behaviour that 842b0ac6 removes. |
| C8 | `mode_token` and `style_token` restate serde renames | valid-and-open | Functions at `commands/project.rs:55-69`, used at `:211-212`. Their targets are `String` fields at `:33,35`. The enums at `config.rs:1103-1145` already carry the serde renames. |
| C9 | `KindScopeConfig` mirrors `item::KindScope` | valid-and-open | The enum is at `config.rs:811` (`:785` is the doc comment of its error type). It has a hand-written `FromStr`, `Deserialize` and `Serialize` with its own token match (`config.rs:822-856`). `KindScope` is at `domain/item.rs:383`; the conversion is at `item.rs:712-713`. |
| C10 | `Projection` stringifies state and priority through `serde_json::Value` | valid-and-open | Fields at `domain/projection.rs:64,67`; helpers `serde_plain_state` and `serde_plain_priority` at `:164-181`. |
| C13 | `EventTag` hand-writes 22 `as_str` strings and a 22-entry `ALL` | valid-and-open | Enum `domain/event_catalog.rs:77-123` (`rename_all = "snake_case"`); `ALL` `:130-153`; `as_str` `:157-182`. `EventScope::as_str` `:63-69` is a further twin. 842b0ac6 REQ-08 takes it to 21 tags. |
| C14 | `EvidenceCode`, `BlockerCode` and `WarningCode` follow the same pattern | valid-and-open | `domain/artifact_plan.rs:369-406`, `:408-466`, `:486-543`. Each pairs a kebab-case serde rename with a hand-written `ALL` and a `const fn as_str`. |
| C15 | `PlanError` hand-writes `Display` and `Error` | valid-and-open | `domain/artifact_plan.rs:1113-1187`. A sibling, `InventoryError` (`domain/artifact_inventory.rs:136`), is also hand-written; REQ-14 does not name it. |
| C17 | `Priority` has no `Ord`, so its rank match is copied | valid-and-open | `commands/query.rs:141-146`; `commands/issue.rs:751-756`. There are no other rank matches in `crates/`, `web/src` or `mcp-server`. |
| C18 | The three graph assertions hold raw tables | valid-and-open | Variants at `declarations/rules.rs:577-590`; the raw fields are at `:923-927`. |
| C19 | Evaluation re-parses keys and compiles regexes on every call | valid-and-open | Label-coverage keys are read at `validation/graph.rs:855-946` and its regex compiled at `:869`. `:1608` (`criteria-to-check`) and `:1781` (`criteria-label-match`) compile regexes from **typed** `String` fields. The comment at `:1606` says the pattern was "already validated at load", but `declarations/rules.rs` compiles no regex. |
| C20 | A malformed rule surfaces only during evaluation | valid-and-open, with exceptions | `child-state` and the `kind` sugar are already checked at load (`rules.rs:1403-1410`, `:1226-1283`). Every other key fails at evaluation. `dependency-shape` `target` is converted to a `Selector` at evaluation (`graph.rs:1380-1393`), so an invalid state token in it never meets `StatePredicate::validate`. |
| C21 | `HierarchyConfig` is built at four production sites with three error policies | **invalid-as-stated** | `commands/template_expand.rs:703` sits inside `#[cfg(test)] mod test_declarations` (`:678-679`) and builds from literals, not from `LabelNamespaces`. There are **three** production sites with three policies:<br>• `config_manager.rs:307` wraps the error in `InvalidArgumentError`.<br>• `commands/validate.rs:1039` propagates a raw `ConfigError` through `?`.<br>• `repository_state/default_rules.rs:520-525` silently falls back to `HierarchyConfig::empty()`.<br>No load path validates the hierarchy, so a level-0 type is accepted by one command family and rejected by the other. |
| C22 | "Strategic" has a hardcoded `level <= 2` | valid; **f368ce3d owns it** | `commands/query.rs:57-77` (the filter is at `:68`). Confirmed: f368ce3d REQ-07 deletes the fallback. The site reads `declared_type_hierarchy()` and builds no `HierarchyConfig`, so REQ-17 is unaffected. |

### 2. Prior-art sweep

- `dev/active/842b0ac6-greenfield-cutover/investigation.md`:
  - REQ-04 §4 at `:600-606` recommends typed `Raw*` structs with `deny_unknown_fields` that keep the table payload.
  - Open decision 4 at `:883-885` defers full payload typing "to a semantic-types refactor", which is this epic's REQ-16.
  - REQ-09 at `:689-729` recommends exact-match state parsing. Its open decision 2 (`:877-879`) leaves `Priority::from_str` case folding to this epic.
- `dev/archive/a2546471-json-error-contract/active/investigation-guard-docs.md:470-500` records the `ErrorCode` conversion from a struct of constants to an enum with a hand-written `ALL` and `as_str`. That produced today's `output.rs:770-830` pattern: a hand-written table plus a schemars conformance test. No study evaluated strum or a macro.
- `dev/active/b80e3c70-v1-release/handoff-4.md:49`: a122b9b3, which tried to enforce semantic types mechanically, was Rejected as `ill-planned` for an overbroad enforcement promise. This epic should not promise a grep-style enforcement check.
- No other `dev/` or `docs/` study covers these vocabularies, `HierarchyConfig` construction, or the typing of graph-assertion tables.

### 3. Consumer sweep

Counts are references per file. `src/` means `crates/jit/src`, `tests/` means `crates/jit/tests`.

**State token copies**
- `errors::state_name`:
  - `errors.rs:757,767,810,868,951,968`
  - `output.rs:1257,1258,1278,1304,1307` (JSON error payloads)
- `errors::gate_status_name`: `errors.rs:959`; `output.rs:1294`.
- `rules.rs` `state_token`: `:285` (a selector explanation message) and test `:1807`.
- `VALID_STATE_TOKENS`: `:487,495` (error lists); tests `:1770,1805,1834`.
- `token_to_state`: `:441,491`; tests `:1806-1814`.
- `State::as_str`: `output.rs:2077,2192`; `main.rs:3477`; `schema.rs:765`.
- `State::all`: `queries.rs:372`, `validation/engine.rs:1011`, `schema.rs:763`, `tests/fast_docs_templates/harness_demo.rs:419,468`.
- `GateStatus::as_str`: `output.rs:2065`.
- Parse sites via `FromStr`:
  - State: `main.rs:2013,3174,3585,3707`; `query_engine/parser.rs:188`.
  - Priority: `main.rs:2016,3009,3175,3584,3712,6442,6488,6576,6621`; `parser.rs:197`; `commands/batch_create.rs:353,468`.
- Further `Priority` token copies:
  - `commands/graph.rs:64-71` `priority_str`, used at `:372` for `BatchIssueDef.priority: Option<String>`.
  - `schema.rs:780`, a hand-written `["low","normal","high","critical"]`. The State list beside it is derived (`:763`).
- Human renders use `Debug` instead of the wire token: `main.rs:2068,2069,2091` (`format!("{:?}").to_lowercase()` yields `inprogress`), `:4872,6474,6559,6607,6652`; `commands/bulk_update.rs:490`.
- TypeScript boundary copies: `web/src/types/models.ts:1-2`; `mcp-server/index.js:41` (prose).

**Strictness and content format**
- `Strictness`:
  - Defined in `validation/strictness.rs` (44 refs).
  - `validation/local.rs` (15 refs).
  - `config.rs:239,302-303`; `commands/template.rs:1167-1168`; `commands/config.rs:223`; `profile/package.rs:814-816`; `repository_state/profile_apply.rs:4789` (test).
- `ValidationConfig.strictness` raw-string asserts: `config.rs:2585,2597,2734,2782,3276`.
- `ValidationConfig::content_format()`: `validation/repository.rs:573,780`; `commands/template.rs:1163`.
- `ContentFormat::from_str`: `config.rs:287`; `main.rs:3011,3611`.
- Raw-string asserts: `config.rs:2735,2783`.

**Item-kind scope**
- `KindScopeConfig`:
  - `config.rs` (31 refs, 15 of them its own definition at `:785-856`).
  - `domain/item.rs` (30 refs, including doctests at `:619-629,1095-1111,1247-1259,1565-1576` and tests at `:1623-2933`).
  - `validation/graph.rs:2187,4292,4306` (tests).
  - `tests/fast_docs_templates/harness_demo.rs:328`.
- `KindScope::ISSUE_TOKEN`: `item.rs:233,252,255,258,912`. It spells the reserved `@/issue/` address segment, not the config scope token.
- `KindScope::PROJECT_TOKEN`: only `parse_for`, which 842b0ac6 deletes.

**Projection mode and style**: `mode_token` and `style_token` are used only at `commands/project.rs:211-212`.

**Transform kind**
- `TransformKind`: `templates.rs` (15 refs); `commands/template_expand.rs:33,242,247,258`.
- `transforms[..].kind`: `templates.rs:734,992`; `template_expand.rs:973`; `tests/fast_docs_templates/template_apply_tests.rs:1396`.
- `UnknownTransformKind`: `templates.rs:139,604,972,981`.
- Test `tests/fast_docs_templates/templates_loader_tests.rs:131`.
- 17 TOML fixtures and docs spell `move-upstream-to-role`; they are unaffected.

**Built-in gate findings**
- `GateFinding {` constructions: `commands/gate_check.rs` (4, listed under C5), `storage/gate_runs.rs:86` (test), `storage/reference.rs:346` (generated reference sample), `output.rs:3240` (test), `main.rs:1582` (test), `domain/gate_findings.rs:284` (test).
- The only engine reader of a disposition is `gate_check.rs:1188`.
- Test asserts on the strings: `gate_check.rs:2025,2786,2826-2834,2914`.

**Gate-run actor field**
- Writers: `gate_execution.rs:175,1513`; `commands/gate_check.rs:56,1225`.
- Tests: `storage/gate_runs.rs:82,254,312`; `storage/reference.rs:340,383`; `storage/repository_state_store_tests.rs:2197,2404`; `gate_check.rs:1793`; `main.rs:1576,8989`; `commands/mod.rs:3610,3938`.
- Reader: `gate_check.rs:233`; view at `output.rs:2502,2560`.

**Query language**
- `QueryCondition::Assignee`: `parser.rs:39,142`; `evaluator.rs:56-59,186,189`.
- `STATE_VALUES` and `PRIORITY_VALUES`: `parser.rs:5,190,199,270,284`.
- Docs: `docs/reference/cli-commands.md:1221,1227`. The example `NOT assignee:*` at `:1227` is wrong today, because `*` is matched literally and so matches nothing.

**Projection fields**: `domain/projection.rs:157-158,264-265,348-349,409-410` (schema contract test). `.jit/schemas/*.json` consume the serialized value, which does not change.

**Event tags**
- `EventTag::ALL`:
  - `domain/event_catalog.rs` (10 refs, including the conformance tests `:614-676,802`).
  - `domain/event_log.rs:66,148`.
  - `storage/reference.rs:815`.
- `as_str`, `tag()` and `get_type()`:
  - Production: `domain/types.rs:2024`; `repository_state/mutation.rs:1103,1161,1274` (`PendingEvent.tag: String` at `:422`, sorted as a string at `:433`); `commands/events.rs:25` (`--event-type` compared as a string); `storage/reference.rs` (4 refs).
  - Tests: `commands/dependency.rs` (5), `commands/issue.rs` (14), `tests/` (16 across 6 files), `tests/scratch_build/repository_state_cutover_tests.rs` (2).
- The generated reference `docs/reference/events.md` is checked by a golden test.

**Artifact-plan codes**
- `EvidenceCode`: `domain/artifact_classifier.rs` 22, `artifact_plan.rs` 7, `commands/archive.rs` 6, `output.rs` 2, `artifact_discovery.rs` 1, `tests/fast_docs_templates/artifact_plan_model_tests.rs` 2, `tests/fast_docs_templates/artifact_inventory_tests.rs` 2.
- `BlockerCode`: classifier 32, `archive.rs` 18, plan model tests 8, `artifact_plan.rs` 7, `artifact_discovery.rs` 4, discovery tests 3, inventory tests 2, `output.rs` 2, `artifact_inventory.rs` 2.
- `WarningCode`: discovery tests 7, classifier 7, plan model tests 6, `artifact_discovery.rs` 6, `artifact_plan.rs` 4, `archive.rs` 4, inventory tests 2, `storage/artifact_planning.rs` 2, `repository_state/archive.rs` 2, `output.rs` 2, `artifact_inventory.rs` 2.
- `as_str` consumers:
  - Output: `output.rs:1041,1148`; `main.rs:6337,6369`; `commands/archive.rs:861`.
  - Sort keys: `artifact_inventory.rs:261-262`, `repository_state/archive.rs:337-338`, `artifact_plan.rs:1224,1228`.
  - Wire pin (the canonical external-contract suite): `artifact_plan_model_tests.rs:343,355,373`.
- `PlanError`: `artifact_plan.rs` 18, `artifact_inventory.rs` 4 (`From` at `:138`), `artifact_discovery.rs` 4 (`#[from]` at `:207`), classifier 4, `repository_state/archive.rs` 3 (`#[from]` at `:547`), plan model tests 1.

**Priority rank**: `commands/query.rs:141-146`; `commands/issue.rs:751-756`.

**Graph-assertion tables**
- `Assertion::{LabelCoverage, LabelReference, DependencyShape}` consumers:
  - `validation/graph.rs:163,507,606,615,618` and tests.
  - `declarations/rules.rs:717-719,751` (`is_repo_wide_at_transition` reads `scope`), `:1410-1416`, test `:2838`.
  - `validation/desugar.rs:60-62,627-635`.
  - `repository_state/rule_serialize.rs:267-275`.
  - `commands/graph.rs:404-409` (reads `satisfies-namespace` and `container-from-label`).
  - `gate_presets/planning.rs:66-197`, which 842b0ac6 REQ-20 deletes.
- Raw key-string reads outside `graph.rs`: `rules.rs:1194,1272-1281,1407`; `commands/graph.rs:405`.
- Tests that need a malformed table to reach evaluation (each becomes a load-failure test):
  - `graph.rs`: `test_label_coverage_malformed_config_is_config_error` `:2421`, `test_child_type_exclude_must_be_array_of_strings` `:2590`, `test_label_reference_missing_key_is_config_error` `:3042`, `test_dependency_shape_missing_target_is_config_error` `:3146`.
  - `tests/fast_issue/graph_rule_validation_tests.rs:185-209`.
  - `tests/cli_item_validate/validate_cli_rule_tests.rs:463,694-745`.
  - `tests/fast_issue/transition_graph_enforcement_tests.rs:550`.
  - The tolerance tests `rules.rs:2869,2894` (also listed by 842b0ac6 REQ-04).
- Assertion TOML appears in 40 files: 12 test files, 9 docs pages and examples, `profiles/jit-dogfood/manifest.toml:86,100`, `.jit/rules.toml`, `.jit/config.toml`, and `src/` (graph.rs 50, rules.rs 31).
- `default_ruleset` contains none of the three kinds. The table arms of `rule_serialize.rs` are therefore reached only by tests.

**`HierarchyConfig` construction**
- `config_manager::get_hierarchy_config` call sites:
  - `commands/graph.rs:230,310,398,490,578`
  - `commands/document.rs:155,214`
  - `commands/archive.rs:159,180`
  - `commands/validate.rs:1473`
  - `crates/server/src/routes.rs:178`
- `repository_state::hierarchy_config` call sites:
  - `commands/validate.rs:1445,1955,2291`
  - `commands/gate_check.rs:495,576`
  - `commands/template.rs:498`
  - `commands/mod.rs:1117`
  - `commands/archive.rs:515`
  - `validation/repository.rs:590`
- Direct `HierarchyConfig::new` calls:
  - Production: `validate.rs:1039`.
  - Tests and fixtures: `test_taxonomy.rs:62` and 11 more under `src/` plus 10 under `tests/`.

### 4. Primitive verification

- **Deriving the wire spelling once.** `strum`, `serde_with`, `serde_plain` and `derive_more` are not in `Cargo.lock`. `semver 1.0.28` is a direct dependency (`crates/jit/Cargo.toml`).
  - The crate already has a precedent that emits several items from one declaration list: `declare_well_known_paths!` (`repository_state/path.rs:259-310`) emits constants plus a complete `ALL_KNOWN` inventory. `output_macros.rs` shows the crate-wide `#[macro_export]` form.
  - A `macro_rules!` that takes `Variant = "wire"` entries can emit:
    - the enum with a per-variant `#[serde(rename = "wire")]`, which schemars 0.8 honours, so `JsonSchema` derives are unchanged;
    - `const fn as_str`, a complete `ALL`, an exact-match `FromStr`, and `Display`.
  - This states each spelling once and adds no dependency.
  - strum would need its own `serialize_all`, a second statement of the rule, plus two proc-macro crates (`@/inv/bounded-rust-build-footprint`).
  - **Recommend the macro.**
  - The existing schemars conformance tests then prove nothing the generated code can break, and are removed:
    - `event_catalog.rs` `test_all_lists_every_tag_variant` at `:620`
    - the `ErrorCode` twin in `output.rs`
  - One owning test proves the macro once: each `as_str` equals its serde encoding, `FromStr` inverts it, and `ALL` matches the derived schema.
- **"Serialized shape unchanged".**
  - Per-variant renames reproduce `rename_all` output exactly.
  - Typed `ProjectionRenderReport.mode` and `style` serialize identically.
  - A typed `Projection.state` and `priority` serialize identically, but the **schemars schema changes** from `{"type":"string"}` to an enum `$ref`. `projection.rs:409-410` pins `"type":"string"`, so the contract test must change. User `.jit/schemas` validate values and are unaffected.
- **`Priority: Ord`.**
  - Declaration order is Low < Normal < High < Critical.
  - Both sorts want Critical first, so they become `sort_by_key(|i| Reverse(i.priority))`. A stable sort keeps tie order, as `issue.rs:747-748` requires.
  - No ordered collection of `Priority` exists today.
- **Sort order of string-sorted vocabularies.**
  - `PendingEvent` sorts by its tag string (`mutation.rs:433`), and so does the event identity order.
  - The artifact codes sort by `as_str` (`artifact_plan.rs:1224`).
  - Keep sorting by `as_str()` and do not switch to the derived `Ord`: declaration order differs from alphabetical, and changing it would reorder published records.
- **Compiled regex at load.**
  - `regex::Regex` has no `PartialEq`, but `Rule` and `Assertion` derive `PartialEq` (`rules.rs:768`).
  - Precedent: `ItemKind` stores `id_pattern_src: String` beside `id_pattern: Regex` and derives no `PartialEq` (`domain/item.rs:643-655`).
  - A small `IdPattern { source, regex }` newtype that compares by source and serializes as source is needed. One type should serve label-coverage, criteria-to-check and criteria-label-match, and also `ItemKind`.
  - The default pattern is spelled twice: `graph.rs:72` and `rules.rs:1455`.
- **Typed configs and serialization.**
  - `rule_serialize.rs` renders the tables with `render_config_table` (`:366-396`), in BTreeMap key order. `toml` 0.8 does not enable `preserve_order`.
  - Typed configs need a renderer that emits the same keys. Byte order matters only to tests, because `default_ruleset` emits none of these kinds.
- **Errors that name the rule.** `RuleConfigError::InvalidAssertion { rule, message }` exists and is reachable from `RawAssert::into_assertion(rule, …)` (`rules.rs:1286`).
- **Case folding.** These parsers lowercase their input today:
  - `State`, `Priority` and `ContentFormat` `FromStr` (`types.rs:160,192,224`)
  - `Strictness` (`strictness.rs:96`)
  - `KindScopeConfig` (`config.rs:826`)
  - `token_to_state` (`rules.rs:371`)
  
  Serde derives are exact. `ContentFormat` also accepts `md`. Docs list only the canonical tokens (`docs/reference/configuration.md:239,241`; `docs/reference/cli-commands.md:1114,1165`). `deserialize_strictness` keeps the author's spelling for `config get` (`config.rs:232`).
- **Built-in finding vocabulary.** `GateFinding.severity`, `disposition` and `origin` are checker-defined open strings on the wire (`domain/gate_findings.rs:46-66`), because exec checkers emit arbitrary labels such as `medium`. A closed enum on the wire field would reject external checker output.
- **Query assignee.** Typing the filter value as `Assignee` makes `assignee:foo` (no colon) and `assignee:*` parse errors. Today both silently match nothing.

### 5. Architecture fit

Primitives to reuse:
- `Assignee` (`domain/types.rs:271-400`, with `FromStr`, serde and `PartialEq<str>`).
- `RuleConfigError::InvalidAssertion`.
- The `RawGateRecency`, `RawCriteriaToCheck` and `RawTypeHierarchy` lowering pattern (`rules.rs:940-1089`).
- `thiserror` (101 derives in the crate).
- The `declare_well_known_paths!` pattern.

Where each new type belongs:
- **The vocabulary macro:** a crate-level pure module, used by `domain`, `config`, `declarations`, `templates`, `validation` and `output`. It performs no I/O.
- **`KindScope`:** keep it in `domain/item.rs`, with serde, and point `ItemKindConfig.scope` (`config.rs:539`) at it.
  - This removes one domain-to-config import (`domain/item.rs:42-44` imports `crate::config::KindScopeConfig`). The other config types imported there stay; that wider inversion is out of scope.
  - The `@/issue/` address segment needs its own address-grammar constant beside `PROJECT_SCOPE_SENTINEL`; it should stop borrowing `KindScope::ISSUE_TOKEN`.
- **`Strictness`:** stays in `validation/strictness.rs` and gains serde; `ValidationConfig.strictness` becomes `Option<Strictness>`.
- **`ContentFormat`:** stays in the domain.
- **`TransformKind`:** stays in `templates.rs` with serde. This deletes `UnknownTransformKind`, the defensive apply-time check at `template_expand.rs:242-249`, and two tests (`template_expand.rs:973`; `tests/fast_docs_templates/template_apply_tests.rs:1396`).
- **Built-in finding enums:** belong in `domain/gate_findings.rs`.
- **Typed assertion configs:** belong in `declarations/rules.rs`. Evaluation in `validation/graph.rs` stays pure.
- **`HierarchyConfig::from_declarations(&LabelNamespaces)`:** belongs in `domain/type_taxonomy.rs`, which is pure.

### 6. Architectural-invariant check

- **Domain purity.** Every proposed type is I/O-free. The macro is compile-time only.
- **Domain-agnostic engine.** The vocabularies here are engine mechanism:
  - lifecycle, gate status, strictness, format, scope, mode and style, transform kind, codes, tags;
  - the built-in finding severity and disposition, which the engine itself emits.
  
  Assertion keys and their defaults (`satisfies`, `success_criteria`, the id pattern) are defaults of parameters that a declared rule binds, so they count as mechanism vocabulary. No new domain literal is introduced.
- **Dependency direction.** Merging `KindScopeConfig` into the domain removes one `domain -> config` import. `domain/item.rs:42-44` still imports the other config types; that inversion remains and is not this epic's.

### 7. Sequencing

Each step compiles and passes the test suite and clippy on its own, and none needs a bridge. The order assumes the relevant 842b0ac6 tasks have landed (REQ-04, REQ-08, REQ-09, REQ-12, REQ-16, REQ-20).

| Step | Outcome | Est. files |
|---|---|---|
| V1 | Vocabulary macro and its owning test. Apply it to `EventTag` (21 tags after 842b0ac6) and `EventScope`. Delete the hand `as_str` and `ALL`, the schemars `ALL` test, and the stale module prose (`event_catalog.rs:9-25`). Expose `FromStr` for f8a6b298 and `commands/events.rs:25`. | 3-4 |
| V2 | Artifact-plan codes through the macro. `PlanError` derives `thiserror`, and so optionally does `InventoryError`. | 2-3 |
| V3 | `State` and `GateStatus` through the macro. Delete `errors::state_name`, `gate_status_name`, `rules.rs` `state_token`, `VALID_STATE_TOKENS` and `token_to_state`. Derive `STATE_VALUES` (REQ-11, part of REQ-13). | ~7: `types.rs`, `errors.rs`, `output.rs`, `rules.rs`, `query_engine/error.rs`, `parser.rs`, `schema.rs` |
| V4 | `Priority` through the macro with `Ord`. Delete both rank matches, `priority_str`, the `schema.rs:780` list and `PRIORITY_VALUES` (REQ-15, rest of REQ-13). | ~6 |
| V5 | `Strictness` gains serde. `ValidationConfig.strictness` and `content_format` become typed. Delete `deserialize_strictness`, `token` and `from_config_value`. | ~8 |
| V6 | `KindScope` absorbs `KindScopeConfig`. Add the address-segment constant. | ~5; ~80 mechanical references |
| V7 | `TransformKind` gains serde. `ProjectionRenderReport` mode and style become typed. | ~5 |
| V8 | Built-in finding severity, disposition and origin become enums, used at the five engine sites. | 3-5 |
| V9 | The query assignee filter becomes `Assignee`. Fix the doc example. Type the `Projection` state and priority and update the schema contract test (rest of REQ-13). | ~5 |
| V10a | Introduce `IdPattern`. Compile `criteria-to-check` and `criteria-label-match` patterns at load. Make the `type-hierarchy` kind and `label-uniqueness` scope serde enums. Delete the `rule_serialize.rs:291` token match. | ~4 |
| V10b | Typed `label-reference` (with a reference-scope enum) and `dependency-shape` (with a mode enum and a load-parsed, validated `Selector`). Convert their tests to load failures. | ~6 |
| V10c | Typed `label-coverage`: `kind` sugar, `child-state` as `State`, `ChildLink` with serde, `IdPattern`. Remove the `Table` payload, `require_str` and `optional_str`. Convert its tests to load failures. | ~7 |
| V11 | `HierarchyConfig::from_declarations` as the one constructor, with one policy (propagate `ConfigError`). Delete `repository_state::hierarchy_config` and the wrapping in `get_hierarchy_config`. Production callers propagate. | ~13 |
| V12 | Sweep the remaining `Debug` renders and the other serde-plus-`as_str` twins (see Needs decision 7). | ~6 |

Notes on the steps:
- **V3 and V4 cannot merge.** Together they touch about 11 files, which is over one worker. The split is clean: each enum's consumers are disjoint apart from `query_engine/error.rs` and `schema.rs`.
- **Graph-assertion typing splits three ways** (V10a, V10b, V10c), by assertion kind. Each kind's variant shape changes in one step, so no compatibility bridge is needed.
- **V10 is ordered before 16c53cc2.** That epic depends on 2195c98f, and V10 rewrites only the configuration preamble of the evaluation functions in `graph.rs`. The algorithms 16c53cc2 owns stay untouched: `children_of`, `linked_issues`, `depends_on_target`, `resolve_container`.
- **V11** can split into a domain constructor plus the `default_rules` callers, then the `config_manager` callers and the server.

Seams with other issues:
- **aa423d02** (claim-next filter, Ready): it rewrites `commands/issue.rs:737-762`, the same lines as V4. Land aa423d02 first, or have V4 touch only the sort key.
- **7d1bfe89** (lexer, InProgress in `.agents/worktrees/agent-7d1bfe89`): it changes only `query_engine/lexer.rs` and one CLI test, so it does not overlap V3, V4 or V9. `FilterField` (`lexer.rs:18-41`) and the hand-written field list in `error.rs:39` are a further closed vocabulary.
- **d23c58c0**: its fix is already merged to `main` (7691e0e09, `type_taxonomy.rs` `validate_orphans`), with gates still pending. No conflict with V11.
- **f368ce3d**: owns `commands/query.rs:68` (confirmed). Its REQ-03 benefits from the typed `TypeHierarchyKind` after V10a.
- **df129d9e** REQ-16: its CLI filter argument group consumes the generated `FromStr` from V3 and V4 at the `main.rs` parse sites.
- **f8a6b298** REQ-10: owns `event_log.rs:66`, which selects events by tag string; it consumes V1's typed parse.

### Needs decision

1. **Case folding and the `md` alias.** Options: (a) exact serde tokens everywhere (CLI, config, rules, query); (b) keep case-insensitive `FromStr`. Recommend (a). It gives one parser, matches the docs, and is consistent with 842b0ac6 open decision 2. Tests that change: `types.rs:2915-2923,2951-2960`, `rules.rs:1795,1810`, `strictness.rs:197-200`, `config.rs:3151-3156`.
2. **Built-in finding vocabulary.** Options: (a) closed enums for what the engine emits, with the wire fields left as checker-defined `String` and built-in findings built through a typed constructor; (b) an open enum with an `Other(String)` fallback on the wire. Recommend (a). REQ-11 names "built-in", and exec checkers emit other labels.
3. **The query `assignee:` value becomes a strict `Assignee`,** so a value without a colon, or `*`, is a parse error. Recommend yes, and fix `docs/reference/cli-commands.md:1227` to use `unassigned`.
4. **Where `HierarchyConfig` validity is enforced.** Options: (a) one fallible constructor that every caller propagates (REQ-17 read literally); (b) also validate at declaration load so a bad level fails `config.toml` loading. Recommend (a), with (b) as an optional follow-up.
5. **Missing graph edge.** D-a (842b0ac6 first) is not in the graph. Add 2195c98f → 842b0ac6, or edges from the V3, V6 and V10 tasks to 842b0ac6 REQ-09, REQ-16 and REQ-04.
6. **`Projection` schema contract.** Typing `state` and `priority` changes the schemars type from `string` to an enum. Accept it and rewrite the pin at `projection.rs:409-410`. Recommend accepting.
7. **Other serde-plus-hand-token twins outside REQ-11's list.** These restate a spelling but are not named by any criterion:
   - `Severity::token` and `RuleScope::token` (`rules.rs:180-215`)
   - `GateStage` and `GateMode` `as_str` and `FromStr` (`declarations/gates.rs:31-90`)
   - `ErrorCode` `ALL`, `as_str` and `FromStr` (`output.rs:770-870,1081`)
   - `DivergentRecord` and `DivergenceClass` (`domain/store_divergence.rs:32,59`)
   - `InvariantKind` (`repository_state/projection.rs:131-133`)
   - `ScalarTarget::semantic_name` (`repository_state/profile_apply.rs:449-455`)
   - `TypeHierarchyKind` (`rules.rs:1420`, `rule_serialize.rs:291`)
   - `FilterField` (`query_engine/lexer.rs:32-41`)
   
   Options: fold them into V12 under REQ-18, or leave them out of scope. Recommend folding in the ones the macro converts mechanically (`Severity`, `RuleScope`, `GateStage`, `GateMode`, `ErrorCode`, `DivergentRecord`, `DivergenceClass`). They are the same convention (`@/inv/convention-convergence`).

