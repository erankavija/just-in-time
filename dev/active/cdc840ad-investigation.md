# Investigation: transactional repository materialization and derived-state coherence

## Scope and method

This report investigates `cdc840ad` against the current implementation. It treats the
epic description as a hypothesis, classifies each success criterion, verifies the
promised primitives, and enumerates the live consumers that a consolidation can affect.
It does not prescribe the work breakdown.

The investigation also exercised a fresh temporary repository with
`jit init --profile jit-dogfood`, `jit validate`, and `jit item show`. The observed
repository validated successfully even though the profile-added `brackets` namespace
had no persisted `namespace-unique-brackets` rule or corresponding default schema.
That behavior follows directly from the load-time derivation described below.

## Executive finding

The consolidation is justified. The repository already has most of the required
primitives, but it does not yet have one operation that derives every coupled target
from one final view and hands the complete delta to the recoverable transaction kernel.
Consequently:

- profile planning merges declared state and generic projections but omits default-rule
  membership and default-schema materializations induced by the merged configuration
  (`crates/jit/src/profile/planner.rs:155-225`);
- `config set` commits `config.toml` before independently refreshing schemas and rule
  membership (`crates/jit/src/commands/config.rs:253-268`);
- `jit project render` validates all outputs first, then publishes targets one at a time
  (`crates/jit/src/commands/project.rs:109-170`); and
- the profile and generic projection paths implement different marker validation
  semantics (`crates/jit/src/profile/render.rs:131-181`,
  `crates/jit/src/validation/projection.rs:279-328`).

Shared-target projection composition, overlay validation, recoverable multi-file
publication, and mutation-startup recovery already exist
(`crates/jit/src/validation/repository.rs:475-519`,
`crates/jit/src/storage/file_transaction.rs:84-176`,
`crates/jit/src/storage/recovery_coordinator.rs:59-102`). Their algorithms are useful,
but their current ownership is not the final architecture: validation's byte-only
`RepositoryView` and profile's richer `RepositorySnapshot` are competing repository
images (`crates/jit/src/validation/repository.rs:114-138`,
`crates/jit/src/profile/snapshot.rs:7-40`). One crate-root `repository_state` subsystem
must replace both and own image/overlay, derivation/comparison, target claims, and managed
rendering; storage captures/applies that neutral state and validation consumes it.

## Claim classification

| Criterion | Classification | Evidence and qualification |
|---|---|---|
| REQ-01: one side-effect-free, domain-agnostic materialization planner | **valid-and-open** | Validation's view can expose a coherent byte overlay, but it cannot represent file mode, directory, symlink payload, or unsupported occupants (`crates/jit/src/validation/repository.rs:114-138`, `crates/jit/src/validation/repository.rs:227-292`). Profile separately models those richer entries and adapts them back to the narrow view (`crates/jit/src/profile/snapshot.rs:7-40`, `crates/jit/src/profile/snapshot.rs:90-124`). Derivation also remains split across profile, validation, config, and project commands (`crates/jit/src/profile/planner.rs:155-225`, `crates/jit/src/commands/mod.rs:1181-1265`, `crates/jit/src/commands/project.rs:87-170`). Both old images and their owners must be replaced by one crate-root `repository_state` image/delta subsystem; no current construct meets that contract. |
| REQ-02: profile init/apply include all declared and derived targets in one validated transaction | **valid-and-open** | The profile planner includes semantic contributions, package assets/static regions, and configured generic projections, then validates the overlay (`crates/jit/src/profile/planner.rs:155-225`, `crates/jit/src/profile/planner.rs:252-264`). It does not derive default schemas or default-rule membership after the profile changes configuration. Fresh init first creates neutral defaults from the pre-profile config (`crates/jit/src/commands/init.rs:43-85`) and then overlays the profile plan (`crates/jit/src/commands/init.rs:337-417`). |
| REQ-03: ordinary init/config publish the complete state through one recoverable transaction | **valid-and-open** | Fresh init already uses the transaction machinery, but config mutation writes the authority first and performs two independent repair operations afterward (`crates/jit/src/commands/config.rs:253-268`). Schema refresh loops over per-file atomic writes and membership sync writes `rules.toml` separately (`crates/jit/src/commands/mod.rs:1181-1200`, `crates/jit/src/commands/mod.rs:1231-1265`). |
| REQ-04: transactional all-target project rendering with deterministic shared targets | **valid-and-open**, with a major **already-done** subproperty | Deterministic shared-target composition is implemented with a target-keyed pending map and has direct coverage (`crates/jit/src/validation/repository.rs:475-519`, `crates/jit/src/validation/repository.rs:1339-1390`). The command also renders all selected outputs before writing, but publishes each distinct target separately (`crates/jit/src/commands/project.rs:109-170`). A later publication failure can therefore leave earlier targets installed. |
| REQ-05: common strict marker parser/splicer with separate policies | **valid-and-open** after clarification | The profile parser rejects partial, reversed, and duplicate matching markers; the generic splicer finds only the first begin/end pair and does not reject duplicate pairs (`crates/jit/src/profile/render.rs:131-181`, `crates/jit/src/validation/projection.rs:279-328`). The corrected criterion permits well-formed nesting of distinct identities and rejects duplicate same-identity pairs, partial/reversed pairs, crossing regions, and otherwise ambiguous topology. That is necessary because the dogfood profile intentionally owns an outer `dogfood-guidance` region containing the configured `invariants` region (`profiles/jit-dogfood/assets/live/regions/agents-jit-guidance.md:1-5`, `profiles/jit-dogfood/manifest.toml:575-579`, `AGENTS.md:187-202`). Placement remains separate: profile regions append when absent, while configured region projections require an existing target and marker pair (`crates/jit/src/profile/render.rs:153-173`, `crates/jit/src/validation/projection.rs:330-365`). The superseded blanket “overlapping” wording would have been invalid-as-stated if interpreted to reject all nesting; the current criterion resolves that ambiguity. |
| REQ-06: validate diagnoses derived drift and `--fix` repairs it transactionally | **valid-and-open**, with generic projection diagnosis **already-done** | Whole-repository validation already recomputes and checks configured projections (`crates/jit/src/validation/repository.rs:1160-1195`). It does not diagnose missing/corrupt default schemas or missing default membership: default schema read failures are deliberately replaced by a placeholder and the assertion is derived from config (`crates/jit/src/validation/rules.rs:1109-1133`), while membership is reconciled in memory (`crates/jit/src/validation/repository.rs:578-620`). Current fix mode repairs hierarchy, transitive reduction, and transitions only (`crates/jit/src/commands/validate.rs:37-85`) and its issue writes are independent (`crates/jit/src/commands/validate.rs:174-185`). |
| REQ-07: new registry-first invariant projected into guidance | **valid-and-open** | The invariant registry is the declared authority and the `invariants` projection targets `AGENTS.md` (`.jit/config.toml:132-142`, `.jit/config.toml:210-217`). No `derived-state-coherence` row exists in `.jit/invariants.toml` (`.jit/invariants.toml:12-71`). The new row must be authored there and projected; `AGENTS.md` is derived, not the editing surface. `single-source-prose` already has a distinct, narrower statement (`.jit/invariants.toml:61-65`). |
| REQ-08: integration, failure, recovery, concurrency, Git-free, and result-contract coverage | **valid-and-open** | Existing tests cover profile transactions, transaction failure injection, generic render preflight, and shared targets, but there is no complete planner or derived-state repair to exercise yet. Project rendering's current failure test proves only render-time failures occur before publication (`crates/jit/tests/fast_rules/project_render_harness_tests.rs:295-339`); the kernel exposes deterministic failure points for publication/recovery tests (`crates/jit/src/storage/transaction_recovery.rs:29-55`). Human and JSON output consumers are inventoried below. |
| REQ-09: direct cutover with no compatibility or duplicate paths | **valid-and-open** | The current system has two repository images (`crates/jit/src/validation/repository.rs:114-292`, `crates/jit/src/profile/snapshot.rs:7-124`), parallel final-target planners (`crates/jit/src/profile/planner.rs:63-88`, `crates/jit/src/commands/project.rs:87-170`, `crates/jit/src/validation/repository.rs:475-519`), two marker engines, and command-local transaction assembly in profile/init. `IssueStore::init` is also a second bootstrap contract used throughout production and tests (`crates/jit/src/storage/mod.rs:91-100`, `crates/jit/src/storage/json.rs:813-855`, `crates/jit/src/storage/memory.rs:91-95`). Gate mutations expose another coupled-state bypass by saving registry/issue/event bytes separately (`crates/jit/src/commands/gate.rs:232-355`, `crates/jit/src/commands/gate.rs:640-748`, `crates/jit/src/commands/gate.rs:950-1108`). The constraint is feasible only if old definitions, exports, test doubles, fixtures, and direct mutation calls disappear in the same cutovers. |

The epic's decisions D-01 through D-07 remain compatible with the implementation. The
current REQ-05 now supplies the marker-topology clarification needed by D-04. D-06 is
correctly scoped: the v1 profile
manifest declares contributions, assets, and regions but no removal lifecycle
(`crates/jit/src/profile/manifest.rs:12-29`).

## REQ-09 direct-cutover inventory

REQ-09 is not satisfied by making the existing helpers call a new planner. Several of
those helpers are themselves alternative planners, target inventories, or publication
boundaries. The following are the exact superseded constructs and their consumers.

| Superseded construct | Current consumers | Clean-cutover implication |
|---|---|---|
| Validation's `RepositoryView`/`FilesystemRepositoryView`/`OverlayRepositoryView` and profile's `RepositorySnapshot`/`SnapshotEntry`/`SnapshotFile` (`crates/jit/src/validation/repository.rs:114-292`, `crates/jit/src/profile/snapshot.rs:7-124`) | Validation, gate checks, profile planner/apply, init, and their tests import the narrow view; profile commands/planner/init and JSON capture import the rich snapshot (`crates/jit/src/commands/validate.rs:191-218`, `crates/jit/src/commands/gate_check.rs:1-20`, `crates/jit/src/commands/profile.rs:8-29`, `crates/jit/src/commands/init.rs:9-12`, `crates/jit/src/storage/json.rs:283-300`). `RepositorySnapshot` itself implements `RepositoryView`, proving the adaptation between competing models (`crates/jit/src/profile/snapshot.rs:90-124`). | Delete both old type families, exports, filesystem loaders, overlays, capture helpers, and direct tests. Crate-root `repository_state::{RepositoryPath, RepositoryEntry, RepositoryImage}` becomes the only rich image/overlay vocabulary; storage captures it through the canonical state-store session, while validation and profile consume it without a re-export bridge. |
| `CommandExecutor::refresh_default_schema_projections`, `CommandExecutor::sync_default_rule_membership`, and `RuleMembershipSync` (`crates/jit/src/commands/mod.rs:322-334`, `crates/jit/src/commands/mod.rs:1165-1265`) | Repository `config set` calls both after saving config (`crates/jit/src/commands/config.rs:253-268`); `scaffold_default_rules` calls both on re-init (`crates/jit/src/commands/mod.rs:1129-1145`). Direct tests call them in `crates/jit/tests/fast_rules/namespace_unique_writethrough_tests.rs:83-256` and `crates/jit/tests/fast_rules/type_hierarchy_schema_regen_tests.rs:159-207`. | Remove these command APIs and rewrite their behavioral tests against the shared materialization plan/application. Keeping thin wrappers would preserve callable partial-publication paths and violate D-07. |
| `IssueStore::init`, both implementations, `CommandExecutor::init`'s call-through, and `scaffold_default_rules` (`crates/jit/src/storage/mod.rs:91-100`, `crates/jit/src/storage/json.rs:813-855`, `crates/jit/src/storage/memory.rs:91-95`, `crates/jit/src/commands/mod.rs:1032-1053`, `crates/jit/src/commands/mod.rs:1105-1163`) | `main` invokes `executor.init`, `seed_project_config`, and `scaffold_default_rules` as separate mutations (`crates/jit/src/main.rs:1905-1970`). Direct `storage.init()` fixture setup is spread through command/storage unit modules, cohesive integration suites, shared harnesses, and server tests; representative central consumers are `crates/jit/src/test_utils.rs:38`, `crates/jit/src/commands/test_helpers.rs:15`, `crates/jit/tests/common/harness.rs`, `crates/server/src/lib.rs:82-100`, and `crates/server/src/routes.rs`. | Delete `IssueStore::init` from the trait and JSON/memory implementations, delete the command call-through and every forwarding test double/doc example, and migrate all tests to canonical `RepositoryStateStore` bootstrap or explicit repository-state fixtures. Fresh/partial bootstrap exists only as a state-store mutation; the no-op memory implementation is not retained as test convenience. Claim-coordinator `init` is a separate Git-backed API and is unaffected. |
| Ruleset storage publishers `write_validation_ruleset`, `write_baked_schema`, `rewrite_rules_header`, `read_rule_identities`, and `sync_namespace_unique_rules` (`crates/jit/src/storage/ruleset_store.rs:30-75`, `crates/jit/src/storage/ruleset_store.rs:158-278`) | Their only production callers are the command-local scaffold/refresh/sync functions (`crates/jit/src/commands/mod.rs:1129-1260`); the remainder of their consumers are unit tests in `storage/ruleset_store.rs` and stale intra-doc references in `validation/serialize.rs` (`crates/jit/src/validation/serialize.rs:9-10`, `crates/jit/src/validation/serialize.rs:91-105`, `crates/jit/src/validation/serialize.rs:150-153`). | Delete the independently publishing storage functions, their direct tests, and stale references. Preserve/extract authored rule parse/preservation and declaration serialization under `declarations`; move default-family/generated-fragment/schema materialization behavior from current `validation::serialize/defaults` into `repository_state`. The old validation/storage definitions do not survive as wrappers (`crates/jit/src/validation/serialize.rs:64-85`, `crates/jit/src/validation/defaults.rs:434-459`). |
| The command-local `project_render` pending-map planner and per-target write loop (`crates/jit/src/commands/project.rs:87-170`) | The CLI dispatch adapts its `ProjectRenderResult` in `main` (`crates/jit/src/main.rs:1351-1415`); render behavior is tested by `fast_rules/project_render_harness_tests.rs` and `cli_item_validate/project_render_cli_tests.rs`. `IssueStore::write_repo_file` has no other production writer (`crates/jit/src/commands/project.rs:163-169`, `crates/jit/src/storage/mod.rs:339-365`). | Keep the stable command/result contract, but derive its reports from the one shared plan and publish through the one transaction boundary. Remove the command-local pending map/write loop and, once it has no production consumer, the `write_repo_file` trait method and its implementation-specific write tests. `read_repo_file` remains independently required by item resolution (`crates/jit/src/commands/item.rs:241-272`). |
| Standalone generic target inventories `projection_targets` and `render_projections`, plus validation's separate `projected_content`/`validate_projections` expected-byte path (`crates/jit/src/validation/repository.rs:458-519`, `crates/jit/src/validation/repository.rs:1142-1195`) | Profile planning calls both target/render helpers (`crates/jit/src/profile/planner.rs:188-225`); repository validation calls its own freshness path (`crates/jit/src/validation/repository.rs:428`); unit tests call `render_projections` directly (`crates/jit/src/validation/repository.rs:1339-1390`). | One common materialization result must supply target ownership, expected final bytes, drift comparison, and command reports. Leaving these independent whole-target maps beside the new plan would retain duplicate inventories even if their render-body functions were shared. |
| Profile/init-specific final plan types `ProfileApplicationPlan`, `PlannedTarget`, `PlannedTargetAction`, `InitScaffold`, and `FreshProfilePlan`, plus `plan_profile_application_against` (`crates/jit/src/profile/planner.rs:23-88`, `crates/jit/src/profile/planner.rs:147-275`, `crates/jit/src/commands/init.rs:35-164`) | `PreparedProfileApplication` stores the plan, profile apply rebuilds an overlay and transaction actions from it, and dry-run maps it into the public result (`crates/jit/src/commands/profile.rs:23-30`, `crates/jit/src/commands/profile.rs:163-220`, `crates/jit/src/commands/profile.rs:350-375`). Fresh init copies scaffold/profile bytes into another target map (`crates/jit/src/commands/init.rs:265-288`, `crates/jit/src/commands/init.rs:419-452`). | Profile package parsing and stable public results remain, but there must be one common seed/image/delta vocabulary. Remove all profile/init final plan/target/scaffold maps; command response metadata may project from the common delta but must not copy final bytes into another authoritative inventory. Fresh neutral config/scaffold derivation becomes a root repository-state producer. |
| Profile-local projection/drift/mode inventories: `PackageProjection`, `ProjectedFile`, `ProjectedFileMode`, `project_package`, `write_projection_tree`, `compare_projection_tree`, `jit_dogfood_live_projection`/`preserve_nested_region`, and `PresetProjection`/`PresetInventory` (`crates/jit/src/profile/render.rs:10-128`, `crates/jit/src/profile/render.rs:188-268`, `crates/jit/src/profile/drift.rs:1-158`, `crates/jit/src/profile/dogfood.rs:120-185`, `crates/jit/src/profile/preset.rs:1-112`) | They are exported together from `profile/mod.rs`, consumed by planner, dogfood/preset checks, application result adaptation, command init/profile helpers, JSON snapshot capture, and direct unit tests (`crates/jit/src/profile/mod.rs:19-49`, `crates/jit/src/profile/application.rs:118-150`, `crates/jit/src/commands/init.rs:9-164`, `crates/jit/src/commands/profile.rs:5-29`, `crates/jit/src/storage/json.rs:768-797`). | Delete the full vocabulary, exports, filesystem projection/drift path, special dogfood preservation function, compatibility inventory, and direct tests. Package parsing survives only as declarations plus canonical `repository_state` seeds/claims/entries/modes; public profile response metadata derives from `RepositoryDelta`. |
| Profile `render_managed_region` and generic `splice_region` marker engines (`crates/jit/src/profile/render.rs:131-181`, `crates/jit/src/validation/projection.rs:279-328`) | The profile function is called by `project_package`, exported by `profile/mod.rs`, and directly tested (`crates/jit/src/profile/render.rs:86-128`, `crates/jit/src/profile/mod.rs:46`, `crates/jit/src/profile/render.rs:289-341`). The generic function is called by `compose_projection`, validation freshness, and its unit tests (`crates/jit/src/validation/projection.rs:330-365`, `crates/jit/src/validation/repository.rs:1142-1156`, `crates/jit/src/validation/projection.rs:471-518`). | Remove both old parser implementations and route profile package rendering and configured projection rendering through one strict byte-oriented managed-document primitive. Command/profile-specific placement and error presentation may remain policy/output concerns; retaining an old parser as a fallback is not acceptable. |
| Command-local conversion to `FileTransactionPlan` in profile and init, including `commands::profile::{transaction_path, unix_mode}` (`crates/jit/src/commands/profile.rs:184-220`, `crates/jit/src/commands/profile.rs:450-465`, `crates/jit/src/commands/init.rs:265-297`) | These are the only production `FileTransactionKernel::execute` call sites outside storage; init also reaches into profile helpers for path/mode conversion (`crates/jit/src/commands/init.rs:274-287`). | The shared application boundary must be the sole production converter from the common delta to transaction actions and the sole affected-command caller of `execute`. Remove both command-local action builders and the cross-command profile helpers. Kernel and recovery tests continue to construct raw plans because they test the storage protocol itself (`crates/jit/src/storage/file_transaction.rs:84-176`). |
| Repo-local direct config publication in `set_config`, `seed_project_config`, `config_store::seed_repo_config`, and the generic `save_config_document` (`crates/jit/src/commands/config.rs:253-310`, `crates/jit/src/storage/config_store.rs:31-89`) | `set_config` owns both user-global and repo-local modes; `main` calls seeding during ordinary init (`crates/jit/src/main.rs:1952-1969`); config-store unit tests call both generic writers. | Repo-local changes become seed inputs to `repository_state` and publish only through `RepositoryStateStore`. Delete `seed_repo_config` and the generic writer name/API. The user-global branch moves to an explicitly user-global-only store/capability that cannot accept a repository-local target; it is not a fallback repository materializer. |
| Storage-owned gate/rule declaration types and raw gate publishers: `storage::GateRegistry`, `domain::Gate`, `validation::rules::{RuleSet, Rule}`, `gate_store::save_gate_registry`, and `IssueStore::save_gate_registry` (`crates/jit/src/storage/mod.rs:84-89`, `crates/jit/src/storage/mod.rs:230-242`, `crates/jit/src/storage/gate_store.rs:117-133`, `crates/jit/src/validation/rules.rs:730-778`) | Gate commands save registry, issue, and audit events independently for add/remove, definition add/define/update/remove, and preset apply (`crates/jit/src/commands/gate.rs:232-355`, `crates/jit/src/commands/gate.rs:640-748`, `crates/jit/src/commands/gate.rs:950-1108`). Gate presets, profile manifests, storage loaders, validation, commands, and tests import the declarations from their current owners. | Move declaration definitions and authored parse/preservation into crate-root `declarations::{GateRegistry, GateDefinition, RuleSet, Rule}` with no aliases/re-exports. Delete raw gate save APIs; all affected gate operations produce one semantic seed containing exact registry/issue/audit changes and configured projections. Materialization-only default/schema/projection serialization moves to `repository_state`, not with validation or storage. |

The retained behavior must also move to its final SSOT owner. Crate-root `declarations`
retains authored gate/rule parsing, preservation, and declaration serialization.
Crate-root `repository_state` owns the single canonical path/entry/image/overlay,
default-family and generated-fragment derivation, default schema rendering, configured
projection rendering, managed composition, and derive/compare functions. Validation
retains rule execution/reporting and whole-repository policy checks, importing the root
declarations and state results; it retains no repository loader or materialization
renderer. Storage retains read-only declaration loaders and `FileTransactionKernel`
plus recovery, and adds only the canonical session/capture/apply capability. Profile
retains immutable manifest/source parsing and public provenance/response envelopes, not
its image, mode, projection, drift, preset inventory, or final-byte types. This replaces,
rather than wraps or re-exports, the current validation/profile implementations.

### Repository-wide acceptance checks for the cutover

A completed cutover needs both behavioral tests and structural absence checks. The
following checks are grounded in the current consumer set:

- `rg` over `crates/jit/src`, `crates/jit/tests`, `docs`, `profiles`, `scripts`, and
  `mcp-server` finds no references to
  `refresh_default_schema_projections`, `sync_default_rule_membership`,
  `RuleMembershipSync`, `write_baked_schema`, `sync_namespace_unique_rules`,
  `read_rule_identities`, `scaffold_default_rules`, `seed_repo_config`, the generic
  `save_config_document`, `save_gate_registry`, or `IssueStore::init`. Historical `dev/`
  records may retain those names as prior art.
- The same live-tree scan finds no old
  `profile::render::render_managed_region` or
  `validation::projection::splice_region` implementation/call path, no
  `ProfileApplicationPlan`, `FreshProfilePlan.changed_files`, `RepositoryView`,
  `OverlayRepositoryView`, `RepositorySnapshot`, `SnapshotEntry`, `SnapshotFile`,
  `InitScaffold`, `PackageProjection`, `ProjectedFile`, `ProjectedFileMode`,
  `PresetProjection`, or `PresetInventory`. There is exactly one strict managed-document
  parser/splicer and one
  rich repository image under crate-root `repository_state`, and every profile,
  projection, validation, JSON, and memory test targets those canonical types.
- `validation` contains no repository filesystem/overlay loader, default/schema
  materialization producer, configured projection renderer, target composer, or marker
  parser; it imports crate-root `repository_state` derive/compare/final-image results.
  Former modules/exports are deleted rather than forwarding or re-exporting old names.
- `GateRegistry`/`GateDefinition` and `RuleSet`/`Rule` each have exactly one definition
  under crate-root `declarations`; storage/domain/validation expose no alias or re-export.
  Gate definition add/define/update/remove, issue gate add/remove, and preset apply contain
  no raw registry/issue/event save sequence and use one state-store semantic mutation.
- No test, doctest, shared harness, forwarding test double, or server fixture calls the
  removed repository `init` trait method. JSON fixtures bootstrap through the canonical
  repository-state store, and in-memory fixtures seed the aggregate state through the
  canonical builder. Claim-coordinator `.init()` calls are excluded because they are a
  distinct Git-backed API.
- `commands/init.rs`, `commands/profile.rs`, `commands/project.rs`,
  `commands/config.rs`, and `commands/validate.rs` contain no `FileTransactionPlan`
  construction and no direct `FileTransactionKernel::execute` call. One shared
  application boundary owns delta-to-action conversion; storage protocol tests remain
  exempt.
- `commands/project.rs` contains no target-keyed pending-byte map and no
  `write_repo_file` call. The write half of `IssueStore::write_repo_file` is removed if
  the repository-wide search confirms no remaining production consumer; the read half
  remains for item/source loading.
- A source review confirms that only the common plan type owns complete final bytes and
  modes. Init/profile command context and human/JSON report types may carry paths,
  classifications, hashes, and metadata, but no second `BTreeMap`/list of final bytes or
  separately assembled transaction-action inventory.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`,
  `cargo test`, and `cargo doc --workspace --no-deps` pass. The documentation build is
  material because current module docs link directly to superseded storage and marker
  functions (`crates/jit/src/validation/serialize.rs:9-10`,
  `crates/jit/src/validation/rules_gates_projection.rs:9-18`).
- The focused suites `cargo test --test fast_rules`,
  `cargo test --test cli_repo_workflow`, `cargo test --test cli_item_validate`, and
  `cargo test --test cli_issue` pass with rewritten tests exercising only public/shared
  paths. `cd mcp-server && npm test` protects generated command/schema curation.
- `scripts/docs-check-projections.sh`, `scripts/docs-check-selftest.sh`,
  `scripts/docs-check-citations.sh <changed-path>...`, and `jit validate` pass after
  projecting the new invariant. This proves both persisted derived freshness and the
  registry-first SSOT route rather than merely compiling the new planner.

## Confirmed coherence defect

### Why the profiled repository can validate while persisted state is stale

The fresh-init sequence is:

1. Generate `config.toml`, `rules.toml`, and default schema files from the neutral
   hierarchy template (`crates/jit/src/commands/init.rs:43-85`).
2. Merge profile contributions and package files, then render configured generic
   projections (`crates/jit/src/profile/planner.rs:155-225`).
3. Validate the final overlay (`crates/jit/src/commands/init.rs:401-417`).

There is no derivation pass between steps 2 and 3 for default membership or default
schemas. Validation does not expose the omission because persisted default schemas are
explicitly rebuildable projections, so a missing or malformed one is tolerated and its
assertion is replaced at load (`crates/jit/src/validation/rules.rs:1109-1133`). Likewise,
`load_rules` reconciles the parsed rule set with the current namespace configuration in
memory (`crates/jit/src/validation/repository.rs:578-620`). Tests intentionally assert
that missing and corrupt default schema projections do not change effective validation
(`crates/jit/tests/fast_rules/default_rules_registry_derivation_tests.rs:227-289`).

That is correct for behavioral SSOT, but insufficient for persisted addressability.
The project `rule` item kind is registry-first over `.jit/rules.toml`
(`.jit/config.toml:144-158`), and `jit item show` resolves project items from their
configured sources (`crates/jit/src/commands/item.rs:326-340`). A rule synthesized only
in the effective in-memory set therefore cannot be resolved as `@/rule/<name>`.

### Ordinary configuration has the same split at a different boundary

`config set` validates the post-edit document before any write, which is good
validated-first behavior for that single file (`crates/jit/src/commands/config.rs:223-251`).
It then saves `config.toml` and only afterward calls schema refresh and membership sync
(`crates/jit/src/commands/config.rs:253-268`). The refresh writes schemas in a loop, each
atomically, and the membership sync subsequently writes `rules.toml`
(`crates/jit/src/commands/mod.rs:1181-1200`, `crates/jit/src/commands/mod.rs:1251-1265`).
The storage API promises per-file atomicity, not set-level publication
(`crates/jit/src/storage/ruleset_store.rs:30-52`,
`crates/jit/src/storage/ruleset_store.rs:54-75`,
`crates/jit/src/storage/ruleset_store.rs:241-278`). A failure after saving config can
therefore leave a partially refreshed repository.

## Primitive verification

| Property likely asserted by planning | Verification |
|---|---|
| A complete proposed repository can be inspected without publishing it | **Contradicted as a single canonical primitive; partly confirmed for bytes.** Validation's `RepositoryView`/overlay supplies file bytes, listings, and tombstones (`crates/jit/src/validation/repository.rs:114-138`, `crates/jit/src/validation/repository.rs:227-292`), but it loses file mode and non-file occupant identity. Profile separately captures file mode, directory, symlink payload, and unsupported entries (`crates/jit/src/profile/snapshot.rs:7-40`). The final rich `RepositoryImage`/overlay must replace both under crate-root `repository_state`; validation cannot remain the image owner. |
| Existing profile planning is side-effect-free | **Confirmed for current target planning, but the plan/image vocabulary is superseded.** `plan_profile_application_against` builds maps and overlay views and returns a plan before publication (`crates/jit/src/profile/planner.rs:147-275`). It still bridges profile's rich snapshot into validation's narrow view and owns a profile-only final-byte map, so it is evidence for pure planning rather than a retained API (`crates/jit/src/profile/snapshot.rs:90-124`, `crates/jit/src/profile/planner.rs:63-88`). |
| Generic shared-target composition is deterministic | **Confirmed behavior, superseded owner.** Projections are traversed in configuration-map order and threaded through one target-keyed pending map (`crates/jit/src/validation/repository.rs:498-519`). The root `repository_state` claim engine should retain deterministic composition while replacing both the validation helper and command-side map; validation must consume its result rather than own another renderer. |
| `write_repo_file` makes the whole render atomic | **Contradicted.** The trait and implementation guarantee temp-file/rename atomicity for one target (`crates/jit/src/storage/mod.rs:339-365`, `crates/jit/src/storage/json.rs:1269-1306`). `project_render` invokes it once per pending target (`crates/jit/src/commands/project.rs:160-170`). The CLI help's “writes every ... atomically” wording is therefore broader than the implementation (`crates/jit/src/cli.rs:587-592`); the reference documentation correctly says writes are individually atomic (`docs/reference/cli-commands.md:3123-3128`). |
| `FileTransactionKernel` provides all-old/all-new failure behavior | **Confirmed as recoverable convergence, not instantaneous multi-path filesystem atomicity.** It preflights before mutation, rolls back ordinary publication errors, and retains a durable recovery journal if rollback cannot be proven (`crates/jit/src/storage/file_transaction.rs:84-176`). Actions publish sequentially and rollback in reverse (`crates/jit/src/storage/file_transaction.rs:492-593`). Recovery sends prepared journals to all-old and verifies committed journals before cleanup (`crates/jit/src/storage/file_transaction.rs:305-351`). Planning and documentation should use “recoverable transaction” or “converges to all-old/all-new,” not claim a filesystem-wide atomic rename. |
| The transaction kernel rejects unsafe or conflicting plans before mutation | **Confirmed.** Plan normalization validates relative paths and rejects duplicate targets (`crates/jit/src/storage/file_transaction.rs:856-866`); `execute` normalizes before creating/publishing transaction state (`crates/jit/src/storage/file_transaction.rs:84-107`). |
| The transaction kernel can transactionally remove an explicitly owned stale materialization | **Contradicted.** `TransactionAction` currently supports create-directory, write-file, and set-mode only (`crates/jit/src/storage/transaction_action.rs:5-31`), and the journal has the same omission (`crates/jit/src/storage/transaction_action.rs:61-84`). Exact repair needs a recoverable `DeleteFile` carrying the expected regular-file preimage, with journaled backup, rollback, and prepared/committed recovery; it must never delete a directory or an ambiguously owned path. |
| The transaction kernel itself proves that the planner read the same preimage it publishes over | **Contradicted.** `TransactionAction::WriteFile` carries final bytes but no planner-expected preimage (`crates/jit/src/storage/transaction_action.rs:5-31`). The kernel records the live original identity later during journal preparation (`crates/jit/src/storage/file_transaction.rs:399-489`). The final `RepositoryDelta`/transaction action must carry the normalized expected preimage captured under the canonical state-store session and the kernel must reject mismatch; replanning plus the current action type is not the final contract. |
| Interruption boundaries are testable | **Confirmed.** The injector enumerates deterministic publication, synchronization, rollback, and cleanup failure points (`crates/jit/src/storage/transaction_recovery.rs:29-55`). |
| Mutating CLI commands recover before loading repository services | **Confirmed.** Mutation classification is exhaustive at the top-level command enum (`crates/jit/src/cli.rs:2856-2893`), project render is classified as mutating (`crates/jit/src/cli.rs:3047-3053`), and recovery runs before executor construction while its session remains alive through dispatch (`crates/jit/src/main.rs:1824-1868`). The recovery session holds bootstrap and repository locks (`crates/jit/src/storage/recovery_coordinator.rs:59-102`). Direct library command calls still need an explicit serialized application boundary. |
| Current static-region parsing is already suitable as the shared strict parser | **Partly confirmed.** It detects matching-marker absence, reversal, and duplication and byte-preserves foreign regions (`crates/jit/src/profile/render.rs:131-181`, `crates/jit/src/profile/render.rs:308-341`). It is byte-oriented and append-policy-specific; a shared topology parser must additionally reason about distinct-region crossing while allowing required nesting. |
| Current generic splicing is strict | **Contradicted.** It uses first-match `find` for begin/end and does not inspect additional matching markers (`crates/jit/src/validation/projection.rs:288-328`). |
| Current `validate --fix` is transactional | **Contradicted.** It dispatches independent fix routines and saves each repaired issue separately (`crates/jit/src/commands/validate.rs:45-80`, `crates/jit/src/commands/validate.rs:174-185`). This does not prevent adding one transactional materialization repair, but the epic should not imply that all legacy fix categories thereby become one transaction unless it deliberately expands scope. |
| `IssueStore::init` is a canonical repository bootstrap | **Contradicted.** The trait exposes an idempotent storage initializer (`crates/jit/src/storage/mod.rs:91-100`); JSON writes directories/index/gates/events independently (`crates/jit/src/storage/json.rs:813-855`), while memory implements it as a no-op (`crates/jit/src/storage/memory.rs:91-95`). It cannot prove the same transactional repository-state contract across backends and must be deleted, including fixture calls. |
| JSON and memory currently expose equivalent atomic state application | **Contradicted.** In-memory state is split across separate mutexes for issues, gate registry, events, runs, and repository files (`crates/jit/src/storage/memory.rs:21-54`). The canonical `RepositoryStateStore` needs native JSON capture/journaled apply and native memory aggregate clone/apply/validate/swap implementations with a shared conformance suite; a memory wrapper around old setters would preserve different semantics. |
| Gate/rule declarations have a neutral SSOT owner | **Contradicted.** `GateRegistry` is defined in storage and contains `domain::Gate` (`crates/jit/src/storage/mod.rs:84-89`), while `RuleSet`/`Rule` are validation-owned (`crates/jit/src/validation/rules.rs:730-778`). Their final declaration semantics belong together in crate-root `declarations`; storage loads/persists them and validation evaluates them, but neither consumer remains the definition owner. |

## Complete live consumer sweep

The following inventory covers tracked implementation, tests, fixtures, scripts,
configuration, and adopter-facing documentation. Historical studies are separated in
the prior-art section so they are not mistaken for runtime consumers.

### Repository views, final-state validation, and profile planning

- Core view and validation implementation:
  `crates/jit/src/validation/repository.rs`.
- Profile snapshot and plan layers:
  `crates/jit/src/profile/snapshot.rs`, `crates/jit/src/profile/planner.rs`,
  `crates/jit/src/profile/application.rs`, `crates/jit/src/profile/mod.rs`.
- Storage capture adapter:
  `crates/jit/src/storage/json.rs:283-311` and
  `crates/jit/src/storage/json.rs:731-813`.
- Command consumers:
  `crates/jit/src/commands/profile.rs`, `crates/jit/src/commands/init.rs`,
  `crates/jit/src/commands/validate.rs`, `crates/jit/src/commands/gate_check.rs`.
- CLI dispatch/result consumers:
  `crates/jit/src/main.rs`, `crates/jit/src/schema.rs`, `crates/jit/src/cli.rs`.
- Tests:
  unit tests in the view/planner/command modules above,
  `crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs`,
  `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs`, and
  `crates/jit/tests/cli_repo_workflow/integration_schema.rs`.

The two production profile-planner call sites are profile apply and fresh profiled init
(`crates/jit/src/commands/profile.rs:292`, `crates/jit/src/commands/init.rs:376`). Every
listed consumer must move to crate-root `repository_state`; both current view/snapshot
definitions, their exports, and JSON's profile-specific capture adapter are deleted.

### Default rule membership and schema materialization

- Derivation and serialization:
  `crates/jit/src/validation/defaults.rs`,
  `crates/jit/src/validation/rules.rs`,
  `crates/jit/src/validation/serialize.rs`,
  `crates/jit/src/validation/repository.rs`, `crates/jit/src/validation/mod.rs`.
- Mutation and storage:
  `crates/jit/src/commands/config.rs`, `crates/jit/src/commands/mod.rs`,
  `crates/jit/src/commands/init.rs`, `crates/jit/src/storage/ruleset_store.rs`, and the
  neutral shipped configuration comments in `crates/jit/src/hierarchy_templates.rs`.
- Registry-first item consumers:
  `.jit/config.toml`, `crates/jit/src/domain/item.rs`,
  `crates/jit/src/commands/item.rs`.
- Tests:
  `crates/jit/tests/fast_rules/default_rules_registry_derivation_tests.rs`,
  `crates/jit/tests/fast_rules/effective_rules_tests.rs`,
  `crates/jit/tests/fast_rules/namespace_unique_writethrough_tests.rs`,
  `crates/jit/tests/fast_rules/type_hierarchy_schema_regen_tests.rs`,
  `crates/jit/tests/cli_item_validate/item_cli_tests.rs`, and unit tests in the
  implementation modules. Default-rule enforcement is also exercised indirectly by
  `crates/jit/tests/cli_query_graph/label_constraints_tests.rs` and command unit tests
  in `crates/jit/src/commands/batch_create.rs` and
  `crates/jit/src/commands/bulk_update.rs`.
- Documentation/examples:
  `docs/reference/configuration.md`, `docs/reference/example-config.toml`,
  `docs/reference/labels.md`, `docs/reference/rules-and-gates.md`, and
  `docs/how-to/adopt-planning-bracket.md`.

Authored rule declarations/parsing move to crate-root `declarations`; default-family,
generated-fragment, and default-schema materialization implementations move to
`repository_state`. Validation keeps evaluation/reporting only, so these current
validation files are consumers/move sources rather than retained materialization owners.

### Generic projections and project-render contracts

- Configuration and render implementation:
  `crates/jit/src/config.rs`, `crates/jit/src/validation/project_render.rs`,
  `crates/jit/src/validation/projection.rs`,
  `crates/jit/src/validation/rules_gates_projection.rs`,
  `crates/jit/src/validation/repository.rs`,
  `crates/jit/src/commands/project.rs`, `crates/jit/src/storage/mod.rs`,
  `crates/jit/src/storage/json.rs`.
- CLI and output contracts:
  `crates/jit/src/cli.rs`, `crates/jit/src/main.rs`,
  `crates/jit/src/commands/project.rs`, `crates/jit/src/schema.rs`.
  `ProjectRenderResult` is currently emitted directly by the CLI; unlike profile apply,
  it has no explicit command-result schema entry. MCP command discovery sees the CLI
  schema, but the curated tool list intentionally excludes `jit_project_render`
  (`mcp-server/curated-tools.json:109`).
- Tests and fixtures:
  `crates/jit/tests/fast_rules/project_render_harness_tests.rs`,
  `crates/jit/tests/cli_item_validate/project_render_cli_tests.rs`,
  `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs`,
  `crates/jit/tests/fixtures/profile-packages/synthetic-valid/manifest.toml`, and unit
  tests in `validation/project_render.rs`, `validation/repository.rs`,
  `validation/projection.rs`, and `validation/rules_gates_projection.rs`.
- Script consumers of `--json` (`.projections[].target`):
  `scripts/docs-check-projections.sh:12-21`,
  `scripts/docs-check-projections.sh:40-60`,
  `scripts/docs-check-selftest.sh:132-137`.
- Documentation/config consumers:
  `.jit/config.toml`, `docs/reference/configuration.md`,
  `docs/reference/cli-commands.md`, `docs/reference/example-config.toml`,
  `docs/reference/gate-presets.md`, `docs/reference/storage-format.md`,
  `docs/concepts/guarantees.md`, `docs/reference/rules-and-gates.md`, `README.md`,
  `CHANGELOG.md`, `crates/jit/src/gate_presets/reference.rs`, and `AGENTS.md`.

Current command output is the list envelope declared by `ProjectRenderResult`
(`crates/jit/src/commands/project.rs:26-54`) and documented at
`docs/reference/cli-commands.md:3126-3129`; it should remain stable when publication is
replaced.

Projection body rendering, target composition, and drift comparison move out of
`validation` into `repository_state`; the command and validation keep their public/policy
roles while importing the same derived claims/final image.

### Static profile regions and profile result contracts

- Manifest/package/render/planner implementation:
  `crates/jit/src/profile/manifest.rs`, `crates/jit/src/profile/package.rs`,
  `crates/jit/src/profile/render.rs`, `crates/jit/src/profile/planner.rs`,
  `crates/jit/src/profile/drift.rs`, `crates/jit/src/profile/preset.rs`,
  `crates/jit/src/profile/application.rs`, `crates/jit/src/profile/dogfood.rs`,
  `crates/jit/src/profile/snapshot.rs`, `crates/jit/src/profile/mod.rs`.
- Application/init/CLI/schema consumers:
  `crates/jit/src/commands/profile.rs`, `crates/jit/src/commands/init.rs`,
  `crates/jit/src/main.rs`, `crates/jit/src/cli.rs`, `crates/jit/src/schema.rs`.
- Embedded and fixture packages:
  `profiles/jit-dogfood/manifest.toml`,
  `profiles/jit-dogfood/assets/live/regions/agents-jit-guidance.md`,
  `crates/jit/tests/fixtures/profile-packages/synthetic-valid/manifest.toml`, and the
  fixture region at
  `crates/jit/tests/fixtures/profile-packages/synthetic-valid/regions/agents.md`.
- Tests:
  unit tests in all profile modules,
  `crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs`,
  `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs`,
  `crates/jit/tests/cli_repo_workflow/integration_schema.rs`, and
  `mcp-server/test-integration.js`.
- External contracts/docs:
  `mcp-server/curated-tools.json`, `docs/reference/profiles.md`,
  `docs/reference/cli-commands.md`, and
  `profiles/jit-dogfood/assets/live/.jit/reference/content-standards.md`.

Profile apply's JSON schema is explicitly published
(`crates/jit/src/schema.rs:475-495`) and acceptance-tested
(`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:598-636`). A shared
planner must preserve that result shape even if its target inventory grows. Manifest and
source parsing plus public provenance/results survive; the profile projection/mode,
snapshot, drift, preset compatibility, and final-plan inventories listed above do not.

### Transaction, recovery, and concurrency boundary

- Kernel and protocol:
  `crates/jit/src/storage/file_transaction.rs`,
  `crates/jit/src/storage/transaction_action.rs`,
  `crates/jit/src/storage/transaction_journal.rs`,
  `crates/jit/src/storage/transaction_recovery.rs`,
  `crates/jit/src/storage/transaction_staging.rs`,
  `crates/jit/src/storage/recovery_coordinator.rs`,
  `crates/jit/src/storage/repo_lock.rs`, `crates/jit/src/storage/mod.rs`.
- Production publication consumers:
  currently `crates/jit/src/commands/init.rs` and
  `crates/jit/src/commands/profile.rs`; the final sole production consumer is the JSON
  `RepositoryStateStore::apply` implementation.
- Recovery classification/startup consumers:
  `crates/jit/src/cli.rs`, `crates/jit/src/main.rs`,
  `crates/server/src/lib.rs` (server recovery is invoked at
  `crates/server/src/lib.rs:18-69`).
- Tests:
  unit tests in the transaction and init/profile modules and
  `crates/jit/tests/cli_issue/recover_command_tests.rs`.
- Documentation:
  `docs/reference/profiles.md`, `docs/reference/cli-commands.md`,
  `docs/reference/storage-format.md`, and `docs/concepts/guarantees.md`.

### Repository bootstrap and fixture consumers

- Trait/implementations/call-through:
  `crates/jit/src/storage/mod.rs`, `crates/jit/src/storage/json.rs`,
  `crates/jit/src/storage/memory.rs`, `crates/jit/src/commands/mod.rs`.
- Shared fixture setup:
  `crates/jit/src/test_utils.rs`, `crates/jit/src/commands/test_helpers.rs`,
  `crates/jit/tests/common/harness.rs`.
- Unit-test consumers:
  command modules `archive`, `batch_create`, `gate`, `gate_check`, `graph`, `init`,
  `invariant`, `issue`, `item`, `profile`, `query`, `snapshot`, and `validate`; storage
  modules `artifact_planning`, `gate_runs`, `json`, `memory`, `reference`, and `mod`.
- Cohesive suites:
  `crates/jit/tests/cli_item_validate`, `crates/jit/tests/cli_query_graph`,
  `crates/jit/tests/fast_docs_templates`, `crates/jit/tests/fast_issue`, and
  `crates/jit/tests/fast_rules`.
- Server fixtures:
  `crates/server/src/lib.rs`, `crates/server/src/routes.rs`.

Every repository-bootstrap call in these paths migrates to the canonical JSON bootstrap
fixture or in-memory aggregate-state fixture. This list excludes claim-coordinator
`init`, which initializes Git-backed lease state rather than a JIT repository.

### Declaration ownership and gate mutations

- Current declaration owners/loaders:
  `crates/jit/src/storage/mod.rs`, `crates/jit/src/domain/types.rs`,
  `crates/jit/src/validation/rules.rs`, `crates/jit/src/storage/gate_store.rs`, and the
  rules loaders/serializers under `validation`.
- Gate mutation consumers:
  `crates/jit/src/commands/gate.rs`, `crates/jit/src/gate_presets`, profile package and
  dogfood modules, CLI/schema/MCP surfaces, and gate/issue/preset tests.
- Direct split writes:
  issue gate add/remove at `crates/jit/src/commands/gate.rs:232-355`, definition
  add/define/update/remove at `crates/jit/src/commands/gate.rs:640-990`, and preset apply
  at `crates/jit/src/commands/gate.rs:1020-1108`.

All declaration consumers migrate to crate-root `declarations`. All listed gate
mutations become complete `SemanticMutation` seeds and lose raw registry/issue/event
publication access; gate-run audit results are not semantic registry mutations.

### Validation/fix and invariant projection

- Validation and fix surfaces:
  `crates/jit/src/commands/validate.rs`,
  `crates/jit/src/validation/repository.rs`, `crates/jit/src/main.rs`,
  `crates/jit/src/cli.rs`, `docs/reference/cli-commands.md`,
  `docs/reference/worktree-validate.md`, and validation suites under
  `crates/jit/tests/fast_rules`, `crates/jit/tests/cli_item_validate`, and
  `crates/jit/tests/cli_repo_workflow`.
- Invariant authority/addressability/projection:
  `.jit/invariants.toml`, `.jit/config.toml`, `crates/jit/src/validation/invariants.rs`,
  `crates/jit/src/commands/invariant.rs`, `crates/jit/src/commands/item.rs`,
  `crates/jit/src/validation/projection.rs`, `AGENTS.md`,
  `docs/reference/rules-and-gates.md`.

Current `validate --fix --json` returns `valid`, `fixes_applied`, `dry_run`, and
`message` (`crates/jit/src/main.rs:6680-6712`). Any derived-state repair must preserve
or deliberately version that contract; the existing documentation lists the current
fix categories at `docs/reference/cli-commands.md:2911-2937`.

## Prior art and decisions already recorded

- `dev/active/af4c901a-derive-default-rules-at-load.md:10-69` established configuration
  as authority, load-time derivation for effective behavior, and persisted schema files
  as write-through projections. It rejected making regenerated files authoritative.
- `dev/active/d74a9ed1-write-through-namespace-unique-membership.md:10-79` recorded the
  exact registry-first addressability gap and added the current narrow membership sync.
- `dev/active/450db193-generic-projection-design.md:6-82` established the generic
  projection model and two-phase render-before-write behavior.
- `dev/active/9b7b5f9c-plan.md:53-65` established final-state views, while
  `dev/active/9b7b5f9c-plan.md:159-174` accurately framed transaction guarantees as
  recoverable all-old/all-new convergence.
- `dev/active/9b7b5f9c-mvp-scope-brief.md:7-30` bounded the profile MVP and deferred the
  broader lifecycle, consistent with D-06.
- `dev/active/cdc840ad-research.md:34-70` independently records the reproduced
  profile/default-materialization failure and current split write boundaries;
  `dev/active/cdc840ad-research.md:72-217` resolves the final crate-root state,
  declaration ownership, profile inventory, and init deletion boundaries, and
  `dev/active/cdc840ad-research.md:219-300` evaluates managed-region topology.
- `dev/active/cdc840ad-plan.md:21-37` is the final implementation-vocabulary decision:
  crate-root `declarations` and `repository_state`, one `RepositoryStateStore`, deletion
  of both old images and every old publisher/init/profile inventory, and no re-export
  bridge. Its research-to-final mapping at `dev/active/cdc840ad-plan.md:37` explicitly
  supersedes the earlier possible `validation::materialization` placement.
- `dev/archive/6eb585bc-completion-report.md:7-15` records completion of core
  maintenance; its cross-epic findings at
  `dev/archive/6eb585bc-completion-report.md:39-54` and
  `dev/archive/6eb585bc-completion-report.md:71-76` already identify projection/profile
  integration as a collision surface. This epic consumes that completed foundation; it
  does not reopen the core-maintenance container.

## Architecture fit and invariant check

### Required layer placement

The repository's boundary rule keeps domain/graph logic pure and I/O-free, puts all
persistence in storage, and lets commands orchestrate the two (`AGENTS.md:137-146`).
The final ownership that preserves those boundaries is:

- crate-root `declarations` owns neutral authored `GateRegistry`/`GateDefinition` and
  `RuleSet`/`Rule` declaration semantics, parsing, and preservation. Storage and
  validation are consumers, not competing definition owners;
- crate-root `repository_state` owns `RepositoryPath`, rich `RepositoryEntry`,
  `RepositoryImage`, overlay, seed, intent, target claim, exact delta, managed-document
  engine, all materialization producers, and pure derive/compare functions. It depends
  on declarations/config, never validation, storage, commands, or profile;
- validation imports declarations plus repository-state images and derive/compare
  results, evaluates rules and whole-repository policy, and validates final images. It
  owns no filesystem capture, alternate image/overlay, default/schema/projection
  materializer, target composer, or marker engine;
- storage implements `RepositoryStateStore`: acquire the canonical opaque mutation
  session, capture one `RepositoryImage`, and apply one finalized `RepositoryDelta`.
  JSON uses the file-transaction kernel; memory uses an aggregate clone/apply/validate/
  swap boundary. Storage owns no command semantic or materialization renderer; and
- commands and profile parse use cases/packages into declarations and neutral seeds,
  rebuild under the state-store session, ask root derivation/comparison for the exact
  result, validate its final image, and apply once. Public response adapters carry
  metadata, never a second final-byte inventory.

The one-way dependency is therefore declarations/config → `repository_state` →
validation, with storage touching repository state only at capture/publication and
commands orchestrating the boundary. This explicitly supersedes earlier placement beside
`validation::repository`: the current filesystem view performs I/O and is byte-only
(`crates/jit/src/validation/repository.rs:114-225`), while profile's separate image owns
richer entry identity (`crates/jit/src/profile/snapshot.rs:7-40`). Keeping either owner or
re-exporting it would preserve the SSOT defect.

### Domain-agnostic invariant

The shared mechanism must dispatch from declared namespaces, item-kind sources,
projection tables, and profile manifest contributions. It must not embed `brackets`,
`jit-dogfood`, invariant IDs, gate names, or adopter workflow types in its general
engine. The protected invariant says those concepts come from repository configuration
(`.jit/invariants.toml:56-59`, projected at `AGENTS.md:199`). Existing generic projection
rendering already derives its target/kinds/mode from configuration
(`crates/jit/src/validation/repository.rs:475-515`), and the profile manifest maps each
contribution to a declared registry path (`crates/jit/src/profile/manifest.rs:46-93`).
Those config-driven behaviors move into root `repository_state`; their current validation
and profile module seams are not preserved. Producer composition is a closed direct call
graph selected by constrained intent/config, not a provider list, callback registry, or
caller-selected producer flags that could recreate partial derivation.

### SSOT and derived-state invariants

The proposed direction preserves D-06's per-kind source authority and the existing
`single-source-prose` invariant: expected bytes are always recomputed from declared
configuration/registries, never adopted from a stale derived file. The new
`derived-state-coherence` invariant should be registry-first in `.jit/invariants.toml`
and rendered with the existing projection. Unless a concrete gate or rule mechanically
checks the full property, it should be declared `kind = "advisory"`; the registry
distinguishes advisory from mechanically enforced entries (`.jit/invariants.toml:5-10`).
The install-only dogfood asset is an empty adopter-owned invariant registry
(`profiles/jit-dogfood/assets/install/.jit/invariants.toml:1-4`, declared at
`profiles/jit-dogfood/manifest.toml:318-321`); the JIT project's architecture-specific
invariant must not be copied into that shipped adopter template.

### Lock and recovery implications

The epic risk about lock duration is real. A speculative preview can be computed outside
the session, but application acquires the `RepositoryStateStore` session, captures the
canonical image, rebuilds the delta with exact expected preimages, validates the final
image, and applies once before releasing it. Profile apply already replans under its
repository guard (`crates/jit/src/commands/profile.rs:122-147`), and CLI startup holds the
recovery session through dispatch (`crates/jit/src/main.rs:1852-1868`), but neither is a
substitute for the new backend contract. Session acquisition takes the repository-sibling
bootstrap guard first: an absent `.jit` retains bootstrap only and uses external control;
an existing root then acquires repository serialization and uses internal control. The
recovery coordinator already demonstrates that conditional order
(`crates/jit/src/storage/recovery_coordinator.rs:59-102`). Fresh init must not create or
acquire a lock inside the absent `.jit` root.

## Test gaps that the planned work must close

Existing coverage proves the component properties, but not the consolidated contract.
The missing evidence is:

- a profiled fresh init and existing-repository apply whose semantic contributions alter
  default membership/schema bytes, with persisted item addressability checked;
- a configuration mutation failure at every transaction publication/recovery boundary,
  proving convergence and exact custom-content preservation;
- one conformance suite over JSON and memory `RepositoryStateStore` images/deltas,
  including absence, bytes/mode, directory, symlink payload, unsupported kind, expected
  preimage conflicts, create/write/mode/delete, failure, and no-swap/rollback behavior;
- a multi-target project render publication failure after one target has been installed;
- duplicate same-ID and crossing marker rejection, plus successful distinct-ID nesting;
- validation errors for missing/stale derived schema, membership, projection, and region
  targets, followed by idempotent transactional repair;
- concurrent mutation serialization through direct in-process APIs as well as CLI;
- gate definition add/define/update/remove, issue gate add/remove, and preset application
  proving registry, issue, audit, and configured projection bytes land in one delta;
- fixture migration proving no production/test/doc consumer retains `IssueStore::init`,
  the validation view, profile snapshot/projection/drift types, old declaration owners,
  raw gate/config/rules/project writers, or compatibility re-exports;
- operation without a git repository, consistent with the core-command exception stated
  at `AGENTS.md:162-163`; and
- unchanged human/JSON envelopes for profile apply, project render, init, config, and
  validate fix.

This investigator modified no code or `.jit/` state.
