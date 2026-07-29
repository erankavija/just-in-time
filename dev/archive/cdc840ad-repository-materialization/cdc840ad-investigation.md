# Investigation: transactional repository materialization and derived-state coherence

Status: historical input, archived after epic completion on 2026-07-23. Any
open-work or integration-branch language below is superseded by the final plan
and completion report.

> **Scope status (2026-07-18):** Owner decision reduced cdc840ad's scope after this investigation was written: lease credential/claim-log/claim-command redesign and worktree/data-root binding are out of scope. The section **F4: claim acquisition is an explicit reconciliation saga** overstates the surviving requirement — only claim acquire's repository-side issue/event publication moves into the shared transaction, and the lease surface is otherwise unchanged. The implementation plan (`dev/archive/cdc840ad-plan.md`, decisions D19–D20) is authoritative wherever this document proposes wider contracts. The code-level findings (current publishers, file:line citations, consumer sweep) remain valid grounding.

## Scope and method

This report investigates `cdc840ad` against the current implementation. It treats the
epic description as a hypothesis, classifies each success criterion, verifies the
promised primitives, and enumerates the live consumers that a consolidation can affect.
It records implementation-boundary and decomposition constraints needed to make the
planned cutover land coherently, but it does not author the work breakdown.

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
| Validation's `RepositoryView`/`FilesystemRepositoryView`/`OverlayRepositoryView` and profile's `RepositorySnapshot`/`SnapshotEntry`/`SnapshotFile` (`crates/jit/src/validation/repository.rs:114-292`, `crates/jit/src/profile/snapshot.rs:7-124`) | Validation, gate checks, profile planner/apply, init, and their tests import the narrow view; profile commands/planner/init and JSON capture import the rich snapshot (`crates/jit/src/commands/validate.rs:191-218`, `crates/jit/src/commands/gate_check.rs:1-20`, `crates/jit/src/commands/profile.rs:8-29`, `crates/jit/src/commands/init.rs:9-12`, `crates/jit/src/storage/json.rs:283-300`). `RepositorySnapshot` itself implements `RepositoryView`, proving the adaptation between competing models (`crates/jit/src/profile/snapshot.rs:90-124`). | Delete both old type families, exports, filesystem loaders, overlays, capture helpers, and direct tests. Crate-root `repository_state::{RepositoryLayout, VirtualPath, RepositoryEntry, RepositoryImage}` becomes the only rich image/overlay vocabulary; storage captures it through the canonical state-store session, while validation and profile consume it without a re-export bridge. |
| `CommandExecutor::refresh_default_schema_projections`, `CommandExecutor::sync_default_rule_membership`, and `RuleMembershipSync` (`crates/jit/src/commands/mod.rs:322-334`, `crates/jit/src/commands/mod.rs:1165-1265`) | Repository `config set` calls both after saving config (`crates/jit/src/commands/config.rs:253-268`); `scaffold_default_rules` calls both on re-init (`crates/jit/src/commands/mod.rs:1129-1145`). Direct tests call them in `crates/jit/tests/fast_rules/namespace_unique_writethrough_tests.rs:83-256` and `crates/jit/tests/fast_rules/type_hierarchy_schema_regen_tests.rs:159-207`. | Remove these command APIs and rewrite their behavioral tests against the shared materialization plan/application. Keeping thin wrappers would preserve callable partial-publication paths and violate D-07. |
| `IssueStore::init`, both implementations, `CommandExecutor::init`'s call-through, and `scaffold_default_rules` (`crates/jit/src/storage/mod.rs:91-100`, `crates/jit/src/storage/json.rs:813-855`, `crates/jit/src/storage/memory.rs:91-95`, `crates/jit/src/commands/mod.rs:1032-1053`, `crates/jit/src/commands/mod.rs:1105-1163`) | `main` invokes `executor.init`, `seed_project_config`, and `scaffold_default_rules` as separate mutations (`crates/jit/src/main.rs:1905-1970`). Direct `storage.init()` fixture setup is spread through command/storage unit modules, cohesive integration suites, shared harnesses, and server tests; representative central consumers are `crates/jit/src/test_utils.rs:38`, `crates/jit/src/commands/test_helpers.rs:15`, `crates/jit/tests/common/harness.rs`, `crates/server/src/lib.rs:82-100`, and `crates/server/src/routes.rs`. | Delete `IssueStore::init` from the trait and JSON/memory implementations, delete the command call-through and every forwarding test double/doc example, and migrate all tests to canonical `RepositoryStateStore` bootstrap or explicit repository-state fixtures. Fresh/partial bootstrap exists only as a state-store mutation; the no-op memory implementation is not retained as test convenience. Claim-coordinator `init` is a separate Git-backed API and is unaffected. |
| Ruleset storage publishers `write_validation_ruleset`, `write_baked_schema`, `rewrite_rules_header`, `read_rule_identities`, and `sync_namespace_unique_rules` (`crates/jit/src/storage/ruleset_store.rs:30-75`, `crates/jit/src/storage/ruleset_store.rs:158-278`) | Their only production callers are the command-local scaffold/refresh/sync functions (`crates/jit/src/commands/mod.rs:1129-1260`); the remainder of their consumers are unit tests in `storage/ruleset_store.rs` and stale intra-doc references in `validation/serialize.rs` (`crates/jit/src/validation/serialize.rs:9-10`, `crates/jit/src/validation/serialize.rs:91-105`, `crates/jit/src/validation/serialize.rs:150-153`). | Delete the independently publishing storage functions, their direct tests, and stale references. Preserve/extract authored rule parse/preservation and declaration serialization under `declarations`; move default-family/generated-fragment/schema materialization behavior from current `validation::serialize/defaults` into `repository_state`. The old validation/storage definitions do not survive as wrappers (`crates/jit/src/validation/serialize.rs:64-85`, `crates/jit/src/validation/defaults.rs:434-459`). |
| The command-local `project_render` pending-map planner and per-target write loop (`crates/jit/src/commands/project.rs:87-170`) | The CLI dispatch adapts its `ProjectRenderResult` in `main` (`crates/jit/src/main.rs:1351-1415`); render behavior is tested by `fast_rules/project_render_harness_tests.rs` and `cli_item_validate/project_render_cli_tests.rs`. `IssueStore::write_repo_file` has no other production writer (`crates/jit/src/commands/project.rs:163-169`, `crates/jit/src/storage/mod.rs:339-365`). | Keep the stable command/result contract, but derive its reports from the one shared plan and publish through the one transaction boundary. Remove the command-local pending map/write loop and, once it has no production consumer, the `write_repo_file` trait method and its implementation-specific write tests. `read_repo_file` remains independently required by item resolution (`crates/jit/src/commands/item.rs:241-272`). |
| Standalone generic target inventories `projection_targets` and `render_projections`, plus validation's separate `projected_content`/`validate_projections` expected-byte path (`crates/jit/src/validation/repository.rs:458-519`, `crates/jit/src/validation/repository.rs:1142-1195`) | Profile planning calls both target/render helpers (`crates/jit/src/profile/planner.rs:188-225`); repository validation calls its own freshness path (`crates/jit/src/validation/repository.rs:428`); unit tests call `render_projections` directly (`crates/jit/src/validation/repository.rs:1339-1390`). | One common materialization result must supply target ownership, expected final bytes, drift comparison, and command reports. Leaving these independent whole-target maps beside the new plan would retain duplicate inventories even if their render-body functions were shared. |
| Profile/init-specific final plan types `ProfileApplicationPlan`, `PlannedTarget`, `PlannedTargetAction`, `InitScaffold`, and `FreshProfilePlan`, plus `plan_profile_application_against` (`crates/jit/src/profile/planner.rs:23-88`, `crates/jit/src/profile/planner.rs:147-275`, `crates/jit/src/commands/init.rs:35-164`) | `PreparedProfileApplication` stores the plan, profile apply rebuilds an overlay and transaction actions from it, and dry-run maps it into the public result (`crates/jit/src/commands/profile.rs:23-30`, `crates/jit/src/commands/profile.rs:163-220`, `crates/jit/src/commands/profile.rs:350-375`). Fresh init copies scaffold/profile bytes into another target map (`crates/jit/src/commands/init.rs:265-288`, `crates/jit/src/commands/init.rs:419-452`). | Profile package parsing and stable public results remain, but there must be one common seed/image/delta vocabulary. Remove all profile/init final plan/target/scaffold maps; command response metadata may project from the common delta but must not copy final bytes into another authoritative inventory. Fresh neutral config/scaffold derivation becomes a root repository-state producer. |
| Profile-local projection/drift/mode inventories: `PackageProjection`, `ProjectedFile`, `ProjectedFileMode`, `project_package`, `write_projection_tree`, `compare_projection_tree`, `jit_dogfood_live_projection`/`preserve_nested_region`, and `PresetProjection`/`PresetInventory` (`crates/jit/src/profile/render.rs:10-128`, `crates/jit/src/profile/render.rs:188-268`, `crates/jit/src/profile/drift.rs:1-158`, `crates/jit/src/profile/dogfood.rs:120-185`, `crates/jit/src/profile/preset.rs:1-112`) | They are exported together from `profile/mod.rs`, consumed by planner, dogfood/preset checks, application result adaptation, command init/profile helpers, JSON snapshot capture, and direct unit tests (`crates/jit/src/profile/mod.rs:19-49`, `crates/jit/src/profile/application.rs:118-150`, `crates/jit/src/commands/init.rs:9-164`, `crates/jit/src/commands/profile.rs:5-29`, `crates/jit/src/storage/json.rs:768-797`). | Delete the full vocabulary, exports, filesystem projection/drift path, special dogfood preservation function, compatibility inventory, and direct tests. Package parsing survives only as declarations plus canonical `repository_state` seeds/claims/entries/modes; public profile response metadata derives from `RepositoryDelta`. |
| Profile `render_managed_region` and generic `splice_region` marker engines (`crates/jit/src/profile/render.rs:131-181`, `crates/jit/src/validation/projection.rs:279-328`) | The profile function is called by `project_package`, exported by `profile/mod.rs`, and directly tested (`crates/jit/src/profile/render.rs:86-128`, `crates/jit/src/profile/mod.rs:46`, `crates/jit/src/profile/render.rs:289-341`). The generic function is called by `compose_projection`, validation freshness, and its unit tests (`crates/jit/src/validation/projection.rs:330-365`, `crates/jit/src/validation/repository.rs:1142-1156`, `crates/jit/src/validation/projection.rs:471-518`). | Remove both old parser implementations and route profile package rendering and configured projection rendering through one strict byte-oriented managed-document primitive. Command/profile-specific placement and error presentation may remain policy/output concerns; retaining an old parser as a fallback is not acceptable. |
| Command-local conversion to `FileTransactionPlan` in profile and init, including `commands::profile::{transaction_path, unix_mode}` (`crates/jit/src/commands/profile.rs:184-220`, `crates/jit/src/commands/profile.rs:450-465`, `crates/jit/src/commands/init.rs:265-297`) | These are the only production `FileTransactionKernel::execute` call sites outside storage; init also reaches into profile helpers for path/mode conversion (`crates/jit/src/commands/init.rs:274-287`). | The shared application boundary must be the sole production converter from the common delta to transaction actions and the sole affected-command caller of `execute`. Remove both command-local action builders and the cross-command profile helpers. Kernel and recovery tests continue to construct raw plans because they test the storage protocol itself (`crates/jit/src/storage/file_transaction.rs:84-176`). |
| Physical `.jit`/storage-parent layout and independent recovery entrypoints | Main passes only `jit_dir`; JSON storage, validation, commands, and recovery repeatedly infer the worktree as its parent, while `FileTransactionKernel` hardcodes `.jit` freshness/internal journals/rollback/reserved paths and `RecoveryCoordinator::recover_before_services` is called independently by CLI/server (`crates/jit/src/main.rs:1829-1868`, `crates/jit/src/storage/json.rs:183-203`, `crates/jit/src/storage/file_transaction.rs:104-105`, `crates/jit/src/storage/file_transaction.rs:196-247`, `crates/jit/src/storage/recovery_coordinator.rs:59-118`). | Delete parent inference, `.jit` prefix translation/control literals, absolute/string transaction adapters, and the independent coordinator route. One `RepositoryLayout` and root-tagged `VirtualPath` reach a layout-driven kernel; recovery implementation is private to `open_mutation_session(layout)` and completes before capture for CLI, server, direct executor, checker return, JSON, and memory. |
| Repo-local direct config publication in `set_config`, `seed_project_config`, `config_store::seed_repo_config`, and the generic `save_config_document` (`crates/jit/src/commands/config.rs:253-310`, `crates/jit/src/storage/config_store.rs:31-89`) | `set_config` owns both user-global and repo-local modes; `main` calls seeding during ordinary init (`crates/jit/src/main.rs:1952-1969`); config-store unit tests call both generic writers. | Repo-local changes become seed inputs to `repository_state` and publish only through `RepositoryStateStore`. Delete `seed_repo_config` and the generic writer name/API. The user-global branch moves to an explicitly user-global-only store/capability that cannot accept a repository-local target; it is not a fallback repository materializer. |
| Storage-owned gate/rule declaration types and raw gate publishers: `storage::GateRegistry`, `domain::Gate`, `validation::rules::{RuleSet, Rule}`, `gate_store::save_gate_registry`, and `IssueStore::save_gate_registry` (`crates/jit/src/storage/mod.rs:84-89`, `crates/jit/src/storage/mod.rs:230-242`, `crates/jit/src/storage/gate_store.rs:117-133`, `crates/jit/src/validation/rules.rs:730-778`) | Gate commands save registry, issue, and audit events independently for add/remove, definition add/define/update/remove, and preset apply (`crates/jit/src/commands/gate.rs:232-355`, `crates/jit/src/commands/gate.rs:640-748`, `crates/jit/src/commands/gate.rs:950-1108`). Gate presets, profile manifests, storage loaders, validation, commands, and tests import the declarations from their current owners. | Move declaration definitions and authored parse/preservation into crate-root `declarations::{GateRegistry, GateDefinition, RuleSet, Rule}` with no aliases/re-exports. Delete raw gate save APIs; all affected gate operations produce one semantic seed containing exact registry/issue/audit changes and configured projections. Materialization-only default/schema/projection serialization moves to `repository_state`, not with validation or storage. |
| Raw typed-record mutation APIs `IssueStore::{save_issue, restore_issue_verbatim, delete_issue, append_event, save_gate_run_result, save_gate_preset}` and JSON/memory implementations (`crates/jit/src/storage/mod.rs:141-171`, `crates/jit/src/storage/mod.rs:216-280`, `crates/jit/src/storage/mod.rs:381-391`) | Every issue/graph/document/bulk/template/migration/fix/claim/gate command ultimately reaches one or more of these methods; JSON stamps/serializes/writes issue and index separately, event append has its own byte path, and gate-run/preset records have independent writers (`crates/jit/src/storage/json.rs:397-455`, `crates/jit/src/storage/json.rs:899-906`, `crates/jit/src/storage/json.rs:1023-1114`, `crates/jit/src/storage/json.rs:1168-1188`, `crates/jit/src/storage/json.rs:1335-1354`). | Delete the production raw mutators and migrate commands plus fixtures to typed semantic mutations over one session/image/delta. Read APIs and explicit aggregate fixture builders may remain. Gate-run/profile/event audit bytes are produced inside the complete mutation even though they are not authored declaration registries. |
| Archive's `stage_artifact*`, `publish_staged_artifact`, and `delete_artifact_if_identity` publication path (`crates/jit/src/storage/artifact_mutation.rs:140-200`, `crates/jit/src/storage/artifact_mutation.rs:235-342`) | `commands/archive.rs` is the only production consumer and combines destination publication, issue relinking, event append, and source deletion with compensation/reconciliation (`crates/jit/src/commands/archive.rs:398-636`, `crates/jit/src/commands/archive.rs:733-856`). | Preserve identity/no-replace policy as repository-state claims and expected preimages, but delete the alternate publication/deletion capability after the archive mutation moves through the sole store. A configured source or projection target cannot be exempt merely because archive selected it. |
| Init's direct `.gitattributes` setup (`crates/jit/src/storage/gitattributes.rs:42-79`) | `main` runs it separately from the init scaffold/transaction, treats Git absence as a no-op, downgrades setup errors to warnings, and folds successful create/modify outcomes into init output (`crates/jit/src/main.rs:1938-1947`, `crates/jit/src/main.rs:2009-2017`). | Delete the warning writer. Typed Git evidence yields `not_applicable` when no containing Git worktree exists or the selected data root is outside it. Otherwise one Git-escaped events-path line-set claim on `Worktree(".gitattributes")` lands in the init delta and reports `unchanged|created|modified`; unsafe eligible occupants/content or competing claims abort preflight. |
| Claim acquire's lease-first raw repository writes, owner-bypass release, v1 wire, partial log ops, and index reads | Acquire durably creates the `.git/jit` lease, then calls `save_issue` and `append_event`; no persisted cross-substrate phase records whether repository sync began or committed (`crates/jit/src/commands/claim.rs:72-156`). Current `Lease`/`ClaimOp`/index/log use and expose one raw UUID, while `IssueClaimed` has no coordination fence (`crates/jit/src/storage/claim_coordinator.rs:29-175`, `crates/jit/src/domain/types.rs:1205-1215`). Current release takes issue ID, resolves leases, and force-evicts without owner credential (`crates/jit/src/commands/claim.rs:204-359`). Mutators load the index, append partial ops, then patch/rewrite the index (`crates/jit/src/storage/claim_coordinator.rs:440-520`, `crates/jit/src/storage/claim_coordinator.rs:538-580`). | Cut to v2, frozen event/matrix, and client-durable authenticated handoff for every credential issuance. The log is sole authority: each fsynced op contains a complete non-secret lease snapshot, including verifier/handoff evidence; the index is replay-only. Delete partial v2 ops, index-first/index-only credential state, plaintext/recoverable server secrets, alternate credential recovery, plus `execute_claim_release_by_issue`, `ReleasedLeaseInfo`, `sanitize_actor`, `resolve_release_actor`, issue-ID release CLI/docs/tests, and aliases. |

The retained behavior must also move to its final SSOT owner. Crate-root `declarations`
retains authored gate/rule parsing, preservation, and declaration serialization.
Crate-root `repository_state` owns the single canonical `RepositoryLayout`,
`VirtualPath::{Worktree, Data}`, entry/image/overlay, default-family and
generated-fragment derivation, default schema rendering, configured projection rendering,
managed composition, and derive/compare functions. Validation
retains rule execution/reporting and whole-repository policy checks, importing the root
declarations and state results; it retains no repository loader or materialization
renderer. Storage retains read-only declaration loaders and a layout-driven
`FileTransactionKernel` plus private recovery mechanics, and adds only the canonical
recovered-session/capture/apply capability. Profile
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
- Production command code contains no direct `save_issue`, `restore_issue_verbatim`,
  `delete_issue`, `append_event`, `save_gate_run_result`, or `save_gate_preset` call and
  no archive artifact publish/delete capability. Commands construct semantic intents;
  fixture-only aggregate builders are named as fixtures and cannot publish JSON paths.
- Lifecycle migration and document asset rescan contain no direct issue/event writer;
  custom preset creation contains no `save_gate_preset`; and archive contains no
  `stage_artifact`, `publish_staged_artifact`, `delete_artifact_if_identity`, compensation,
  or reconciliation publication path. Each is covered by the same sole-store conformance
  and failure-boundary tests as ordinary issue mutations.
- `jit init` has no direct `.gitattributes` writer or warning-only setup path. Typed
  eligibility reports exact `not_applicable` for no-Git/outside-worktree cases; only an eligible
  worktree-relative events path produces a `Worktree(".gitattributes")` line-set claim,
  and that claim lands in the same delta as all other init state. Graph export to stdout
  is non-mutating. Graph/snapshot destination canonicalization checks the selected data
  root first, then worktree, then external; every Data/Worktree destination is an
  explicit repository-state export intent with captured preimage, complete parent
  listings, and collision checks and never uses the direct external publisher.
- `rg` finds no storage/kernel/command path that derives the worktree from
  `data_root.parent()`, strips a virtual `.jit` prefix, or hardcodes `.jit` for selected
  data-root existence, internal locks/journals, rollback cleanup, or target conversion.
  All mutation/recovery callers supply one `RepositoryLayout`, journal actions store a
  `Worktree`/`Data` root tag plus relative path, and layout mismatch fails closed.
- Layout construction rejects equal/aliased roots, worktree-beneath-data topology,
  symlinked components, lexical escape, and root-identity change. When data is nested in worktree, tree-wide scans and tests
  prove Data precedence for every physical→virtual conversion and reject typed Worktree
  aliases beneath data before capture, collision analysis, hashing, journal creation,
  export classification, and recovery.
- No mutation-capable `CommandExecutor` call can capture before recovered-session open.
  CLI/server retained startup sessions are reusable optimizations only; direct calls and
  post-checker persistence exercise the same recovery-before-first-read contract.
- Claim control-plane records make one one-way cut to `record_version: 2` and carry
  canonical issue identity, `LeaseState`, coordination identity, monotonic generation,
  and an internal attempt-owner fence. The migration marks v1 live records
  `legacy_unverified` because neither the old log/index nor historical `IssueClaimed`
  can prove repository synchronization. New repository audit uses only
  `issue_claim_lease_changed`; bearer lease secrets never enter the index, log, event,
  status/list output, error, warning, or diagnostic. Status/list expose only
  `lk_<public-key>.REDACTED`. Conditional finalize/delete, crash
  recovery, acquire-internal fenced takeover with bearer rotation, and delayed-owner tests replace every
  lease-first/raw-`save_issue`/`append_event` gap under the sole coordinator → bootstrap
  → data-root-publication → repository → events order.
- Live-code/schema scans find no current `schema_version` discriminator for claim
  records, v1 `ClaimOp::{Acquire,Renew,Heartbeat,Release,AutoEvict,ForceEvict}` decoder,
  newly emitted `issue_claimed`, plaintext bearer field, or complete bearer formatting.
  Exactly one isolated v1 current-record migration reader plus the permanent decode-only
  v1 log-history boundary remain. Tests assert the exact `LeaseState`
  and log-operation enumerations, exact `issue_claim_lease_changed` fields, constant
  redaction across every human/JSON/debug/error/status/list/worktree surface, and fresh
  generation/key/secret on acquire, acquire-internal grace takeover, renew, and legacy
  heartbeat/renew upgrade. Normal heartbeat and release do not rotate or reveal a secret;
  release requires the full bearer; force-evict accepts only `lease_key`; recovery and
  reconciliation handle no secrets. The release-by-issue owner bypass and all aliases are absent.
- An absent disjoint selected data root never receives an internal lock, journal, or
  partially published child. Tests and structural scans prove layout opens the existing
  parent capability, all contenders derive one publication-lock key from parent identity
  plus leaf, all `Data(...)` writes target a verified sibling stage, and only a verified
  synchronized no-replace directory rename publishes the root. Recovery/rollback cleanup
  is conditional on the recorded parent/leaf/object identity.
- Every layout has a permanent two-sided binding: worktree
  `.jit-bootstrap/binding.json` and data-root `RepositoryBindingV1` agree on random
  binding ID and no-follow worktree-root identity. Race losers reuse only an exact
  same-worktree/layout match; other ownership is `DataRootOwnedByDifferentWorktree`.
  Nested legacy roots bind once by containment; disjoint legacy roots fail ordinary open
  and require the explicit double-confirmation migration. No auto-adoption or unbound
  compatibility path survives.
- Credential issuance is crash-safe without server plaintext: acquire/renew/legacy
  upgrade uses caller-durable authenticated handoff, deterministic HKDF replay, and a
  pending record; the first new-bearer use appends `handoff_confirmed`. Only same-handoff
  replay accepts an old rotating credential. CLI creates an atomic `0600` caller-owned
  handoff file unless sensitive input is supplied; schema/MCP expose the same write-only
  protocol and no alternate recovery command.
- `claims.jsonl` is the sole v2 authority. Every fsynced op carries a complete non-secret
  `LeaseV2` replay snapshot, then the index is atomically regenerated. Crash recovery
  replays the log; migrated legacy aliases, credential verifiers, handoff evidence,
  rotations, states, and fences never exist only in the index.
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

## Retained consolidation findings

### Bounded discovery must produce a complete read set

The proposed store does not need to copy the whole working tree into memory, but the
current selected-path capture is not a complete alternative. `capture_profile_snapshot`
captures only caller-supplied paths and their existing ancestors
(`crates/jit/src/storage/json.rs:277-300`,
`crates/jit/src/storage/json.rs:731-765`). Profile apply supplies package targets plus
its record directory, record, and event log (`crates/jit/src/commands/profile.rs:248-260`),
and fresh init supplies the same bounded set (`crates/jit/src/commands/init.rs:337-354`).
Both then validate through a separate live filesystem view
(`crates/jit/src/commands/profile.rs:163-176`,
`crates/jit/src/commands/profile.rs:278-292`). The snapshot therefore neither records
every byte validation read nor proves those later reads belong to the same image.

The current `RepositoryView` access pattern defines the minimum discoverer contract:

| Read-set class | Complete bounded inventory | Why it is a dependency |
|---|---|---|
| Fixed repository authorities | Logical adopter paths `.jit/config.toml`, `.jit/templates.toml`, `.jit/invariants.toml`, `.jit/rules.toml`, `.jit/gates.toml`, `.jit/index.json`, and `.jit/events.jsonl` currently select declarations and audit history (`crates/jit/src/validation/repository.rs:547-638`, `crates/jit/src/validation/repository.rs:655-715`). | In the final vocabulary these are `Data("config.toml")`, `Data("templates.toml")`, and so on, resolved through `RepositoryLayout`, never by assuming a physical `.jit` prefix. Absence and non-file occupants are part of the image, not equivalent to empty bytes. |
| Rule-selected schema inputs | Every logical `.jit/{reference}` named by an authored schema assertion (`crates/jit/src/validation/repository.rs:578-617`) | Rule loading performs these dynamic reads after parsing `rules.toml`; each becomes a typed `Data(reference)` read. Capturing only the rules file permits a schema race. |
| Complete live issue set | Every logical `.jit/issues/{id}.json` named by `index.json` **and** recursive enumeration of the selected data root's `issues` directory (`crates/jit/src/validation/repository.rs:678-707`) | The `Data("issues")` listing is itself a read dependency: validation rejects unindexed extra JSON and missing indexed JSON. A digest of named issue files alone is incomplete. |
| Issue-selected validation inputs | Every issue-linked **unpinned** document path; every repository-local resolved asset carried by that document; any additional local asset path discovered by rescanning the captured document bytes; and every template/issue-derived completed-plan path (`crates/jit/src/domain/types.rs:791-811`, `crates/jit/src/document/assets.rs:14-29`, `crates/jit/src/commands/document.rs:32-55`, `crates/jit/src/validation/repository.rs:749-805`, `crates/jit/src/validation/repository.rs:990-1013`) | Parsing issues expands the closure. The document and its local assets are exact image entries, while the asset scan is a pure consumer of captured document bytes and can enqueue newly discovered local paths to the fixpoint. Completed planning-node documents are derived from the configured template plus issue graph, so they cannot be omitted merely because no command argument named them. |
| Configured item sources | Every project-scoped markdown `source` and registry-first `source.toml` declared under `[item_kinds]` (`crates/jit/src/validation/repository.rs:1017-1068`, `crates/jit/src/domain/item.rs:694-719`) | These external repository paths are authored item SSOTs. They may live outside `.jit`, so a `.jit`-only capture is incomplete. |
| Configured projection inputs and targets | Every item source selected by each configured projection, each configured target, and target ancestors/entry kinds (`crates/jit/src/validation/project_render.rs:93-150`, `crates/jit/src/validation/repository.rs:475-519`, `crates/jit/src/validation/repository.rs:1142-1195`) | Separate-file rendering reads sources and compares targets; region rendering additionally preserves current target bytes. Shared targets must be captured once with mode and occupant identity so composition and expected preimages refer to one entry. |
| Intent-selected mutation/audit paths | Profile package targets, the profile metadata directory and installed record, events, gate-run result paths, custom gate-preset targets, archive source/destination paths, the init `.gitattributes` line-set target, and repository-local graph/snapshot export destinations selected by the semantic intent (`crates/jit/src/commands/profile.rs:248-277`, `crates/jit/src/storage/json.rs:382-395`, `crates/jit/src/storage/json.rs:1335-1354`, `crates/jit/src/storage/artifact_mutation.rs:235-342`, `crates/jit/src/storage/gitattributes.rs:42-79`, `crates/jit/src/main.rs:4905-4931`, `crates/jit/src/commands/snapshot.rs:443-490`) | These are not all selected by whole-repository validation, but their absence/current identity, complete parent listings, and parent occupant types are required to compute safe create/write/delete actions. |

Pinned document evidence has a separate boundary. A `DocumentReference.commit` may be a
symbolic revision such as `HEAD`; the current validator resolves the revision and tree
entry directly through git (`crates/jit/src/validation/repository.rs:752-775`). Phase one
must resolve it once to an immutable commit object ID plus a typed present/missing tree
entry identity, including pinned local-asset evidence where the command validates those
assets. That `PinnedGitEvidence` is included in the seed/plan hash and supplied to pure
validation, but it is neither a working-tree `RepositoryEntry` nor a delta expected
preimage, and capture never recursively scans `.git`. An unpinned document never falls
back to this evidence: its document, stored local assets, rescan-discovered assets, and
derived plan path belong to the guarded repository image. A pinned reference outside a
Git repository yields typed unavailable evidence rather than silently switching to the
working tree.

Discovery is consequently a bounded fixpoint, not a caller-maintained path list: capture
the fixed `Data` authorities; parse config/index/issues/rules; add every discovered
typed schema, issue-directory listing, item source, projection target, unpinned linked
document/local asset, derived plan document, and intent target; then capture the added
entries and ancestors under the same opaque store session. `RepositoryLayout` and the
root class of every `VirtualPath` belong in the resulting sorted read-set identity,
`RepositoryImage`, plan hash, and delta expected preimages. Immutable embedded
profile-package bytes and
`PinnedGitEvidence` are explicit typed inputs/evidence. Machine-local locks, transaction journals,
server PID/log files, and the Git claim control plane are deliberately outside the
semantic image (`crates/jit/src/storage/repo_lock.rs:40-44`,
`crates/jit/src/storage/claim_coordinator.rs:312-321`).

### Typed-to-byte production and lock ownership are part of the SSOT

The current split is not only several file writers; several timestamps, UUIDs, audit
records, and final byte images are created at different layers and at different points
relative to serialization. A single state store cannot be correct if commands still
construct authoritative bytes or storage silently changes typed state after planning.

| Typed-to-byte producer today | Current lock/publication behavior | Required cutover ownership |
|---|---|---|
| `Issue::new` generates a UUID and samples `Utc::now` for `created_at`/`updated_at`; create-time auto-Ready samples again for `first_ready_at` and constructs `IssueCreated` with another UUID/time (`crates/jit/src/domain/types.rs:403-509`, `crates/jit/src/commands/issue.rs:114-169`). Batch/template creation inherits the same constructor. | Identity and three logically coupled creation timestamps exist before the repository session; a rejected create burns an ID/time and JSON/memory cannot inject deterministic authorities. | Replace persisted-record construction with identity/time-free issue intents. A session-injected `IdAuthority` allocates issue identities in stable command-local order only for a semantically accepted non-noop create; one `MutationTimestamp` supplies `created_at`, `updated_at`, initial `first_ready_at` when born Ready, and the creation event time. Test-only issue builders may take explicit IDs/times but cannot be production publishers. |
| `IssueStore::save_issue` stamps `updated_at` with `Utc::now`, then JSON pretty-serializes the issue; a new issue is written before `index.json` (`crates/jit/src/storage/mod.rs:141-171`, `crates/jit/src/storage/json.rs:397-438`, `crates/jit/src/storage/json.rs:899-906`). Memory independently repeats the stamp (`crates/jit/src/storage/memory.rs:71-109`). | Timestamp creation occurs before `persist_issue` acquires the repository lock; JSON then takes repository → index → issue locks and performs separate issue/index replacements. Delete removes the issue before rewriting the index (`crates/jit/src/storage/json.rs:1023-1047`). | A semantic issue mutation carries intent, not pre-serialized bytes. The canonical producer stamps time/IDs once inside the state-store session, derives issue plus index plus audit bytes, and applies one delta. `save_issue`, `restore_issue_verbatim`, and `delete_issue` disappear as production mutation surfaces; JSON and memory consume the same finalized typed result. |
| Every `Event::new_*` constructor generates a UUID and current timestamp, and issue creation has an equivalent command-local literal constructor (`crates/jit/src/domain/types.rs:1493-1782`, `crates/jit/src/commands/issue.rs:160-169`). Ordinary `append_event` compact-serializes under repository → events locks and inserts a newline before any non-newline tail (`crates/jit/src/storage/json.rs:1084-1114`). | Issue/gate/config/archive commands call append after other writes. Multiple events in one command are multiple independently locked appends, and constructor call order currently determines IDs/times rather than a documented audit order. | Commands submit identity/time-free event intents. After semantic no-op detection, the finalizer sorts them by `(mutation phase, event tag, primary identity, secondary identity, command-local ordinal)`, allocates event IDs through `IdAuthority`, applies the one `MutationTimestamp`, and builds exactly one next `events.jsonl` image. All `Event::new_*` production allocation and append sequences are removed. |
| First-occurrence lifecycle fields are sampled independently in issue create/transition/assign/claim/bulk paths (`crates/jit/src/domain/types.rs:438-467`, `crates/jit/src/domain/types.rs:608-628`, `crates/jit/src/commands/mod.rs:753-762`, `crates/jit/src/commands/issue.rs:889-904`, `crates/jit/src/commands/bulk_update.rs:353-390`). The one-time migration derives historical values from existing event timestamps, then `save_issue` overwrites `updated_at` with the current storage clock and appends a separately timed migration event (`crates/jit/src/domain/queries.rs:11-99`, `crates/jit/src/commands/migrate.rs:19-80`). | One operation can currently carry different `updated_at`, lifecycle, and audit times. Migration correctly preserves first historical event times, but its changed-issue publication and audit are split. | For ordinary non-noop mutations, the same `MutationTimestamp` sets every changed issue's `updated_at` and any newly established `first_ready_at`/`claimed_at`/`done_at`. Existing first-occurrence values never move. Lifecycle migration retains historical timestamps derived from captured event history; only each rewritten issue's `updated_at` and the one `LifecycleTimestampsBackfilled` event use the new mutation instant, all in one delta. |
| `GateState.updated_at` is sampled independently for add/pass/fail; checker recording instead copies `GateRunResult.started_at`. External and built-in runners generate a run UUID plus actual start/completion/duration evidence (`crates/jit/src/commands/gate.rs:269-299`, `crates/jit/src/commands/gate.rs:478-493`, `crates/jit/src/commands/gate.rs:610-624`, `crates/jit/src/commands/gate_check.rs:308-331`, `crates/jit/src/gate_execution.rs:100-185`, `crates/jit/src/commands/gate_check.rs:369-529`). | Manual gate issue/event times can differ; automated gate state currently means checker start rather than repository mutation time. The runner's timing fields are observational evidence and must not be rewritten to pretend publication occurred then. | `GateState.updated_at`, changed issue `updated_at`, and gate pass/fail event time use `MutationTimestamp`. `GateRunResult.started_at`, `completed_at`, and `duration_ms` remain typed checker-observation evidence produced by a checker clock outside the mutation session. The runner returns an identity-free result draft; `IdAuthority` allocates the persisted `run_id` only when the post-checker mutation finalizes. |
| Profile uses a second append helper, `append_profile_event_image`, and the command separately determines `ProfileApplied.isolated_torn_tail`: it is false for an empty/newline-terminated prefix and otherwise reflects whether the final unterminated line fails JSON parsing (`crates/jit/src/profile/application.rs:176-194`, `crates/jit/src/commands/profile.rs:265-277`, `crates/jit/src/commands/profile.rs:380-389`). | Profile holds repository and events locks, computes the full next image, then places it in its command-local transaction (`crates/jit/src/commands/profile.rs:142-175`, `crates/jit/src/commands/profile.rs:208-220`). | The canonical audit finalizer preserves all prefix bytes, inserts exactly one separator newline iff a nonempty prefix lacks one, and alone computes `isolated_torn_tail`: true only for a malformed final non-newline JSON line, false for empty, newline-terminated, or valid unterminated input. Delete both the append helper and command-local detector; profile supplies provenance fields, not event-log bytes. |
| `expected_record` assembles `AppliedProfileRecord`; `to_bytes` pretty-serializes with a trailing newline (`crates/jit/src/commands/profile.rs:330-338`, `crates/jit/src/profile/application.rs:7-29`). | Record bytes are added beside planned targets only after profile planning, so the profile plan is not the complete final repository (`crates/jit/src/commands/profile.rs:160-175`, `crates/jit/src/commands/profile.rs:184-212`). | Profile provenance/audit-record assembly is a root repository-state producer in the same derivation as profile targets and events. The public result may expose hashes/status, but commands retain no authoritative record bytes. |
| Gate checking assembles a typed `GateRunResult`, saves its pretty JSON, then independently saves the issue gate state and appends a pass/fail event (`crates/jit/src/commands/gate_check.rs:282-331`, `crates/jit/src/storage/json.rs:1168-1188`). | The external checker deliberately runs outside the retained recovery session; recovery serialization is reacquired before persistence (`crates/jit/src/storage/mod.rs:124-139`, `crates/jit/src/storage/json.rs:862-883`). The three writes are still distinct, and the run-result path has only the repository lock. | Checker execution remains outside the mutation session. Its typed result becomes input to a newly acquired/recovered session that re-reads the issue and atomically derives run-result, issue, index-if-needed, event, and affected projection bytes. `save_gate_run_result` is no longer a standalone publisher. |
| Gate registry, config, rules/default schemas, generic projections, custom gate presets, and fresh-init scaffolds each have local serializers/writers (`crates/jit/src/storage/gate_store.rs:117-133`, `crates/jit/src/storage/config_store.rs:43-70`, `crates/jit/src/storage/ruleset_store.rs:30-75`, `crates/jit/src/commands/project.rs:109-170`, `crates/jit/src/storage/json.rs:1335-1354`, `crates/jit/src/commands/init.rs:43-85`). | Most take the repository lock only in the outer storage method; repo config/rules helpers can publish directly and project render locks once per target. Custom preset creation is its own pretty-JSON replacement. | Authored declaration preservation stays in `declarations`; every repository-local byte renderer and target claim lives in `repository_state`; `RepositoryStateStore` is the only publisher. The user-global config writer remains a separate, explicitly non-repository capability. Custom preset creation is a semantic repository mutation and must join the cutover. |
| Archive stages/publishes/deletes repository documents through a separate verified-artifact capability, relinks issues, and appends a durable audit event (`crates/jit/src/storage/artifact_mutation.rs:1-6`, `crates/jit/src/storage/artifact_mutation.rs:235-342`, `crates/jit/src/commands/archive.rs:398-407`, `crates/jit/src/commands/archive.rs:497-636`). | One repository guard spans the sequence, but destination publications, issue rewrites, event append, and source deletions remain separate and use compensating/reconciliation logic (`crates/jit/src/commands/archive.rs:610-636`, `crates/jit/src/commands/archive.rs:733-856`). | Archive sources/destinations can contain linked documents or configured item/projection paths and therefore affect declared/item state. Preserve its identity/no-replace policy as intent validation, but route the complete publication/relink/audit/deletion delta through the sole state store. The old artifact mutation publication surface cannot remain an alternate repository writer. |

#### Final F2 identity, clock, timestamp, and audit decision

`RepositoryStateStore` receives injected `IdAuthority` and `MutationClock`
capabilities. Commands submit identity-free and persistence-time-free drafts/intents;
they do not reserve persisted UUIDs, sample mutation time, or construct final audit
bytes. One `MutationContext` is created after the recovered mutation session opens and
is reused unchanged through capture-closure expansion and retryable capture conflicts.
Under that session/context, finalization is binding and ordered as follows:

1. Capture the closed image/evidence, derive the proposed semantic state, and determine
   whether the operation is a semantic no-op. A no-op samples no mutation clock and
   allocates no persisted identity.
2. For an accepted non-noop, sample `MutationClock` exactly once to obtain the
   `MutationTimestamp`.
3. Allocate persisted identities in a documented deterministic order: new issue IDs in
   stable canonical request order, so their expected-absent `Data("issues/<id>.json")`
   paths enter `CaptureSpec`; gate-run and provenance-record IDs in `(target, key)` order;
   then event IDs after sorting event intents by `(mutation phase, event tag, primary
   identity, secondary identity, command-local ordinal)`. The context seed and every
   allocation enter the semantic plan hash.
4. Closure expansion, expected-preimage conflict recovery, and backend execution reuse
   the same context, IDs, and mutation instant; they never resample. A fully abandoned
   command invocation does not publish those allocations. Fixed authorities in tests
   make ordering and JSON/memory images reproducible.

The one `MutationTimestamp` supplies `Issue.created_at` for new issues, every changed
issue's `updated_at`, an initial or newly established `first_ready_at`/`claimed_at`/
`done_at`, `GateState.updated_at`, and every event timestamp in that mutation. Existing
first-occurrence fields never move. Lifecycle migration is the deliberate exception for
historical facts: its backfilled lifecycle fields preserve the first matching timestamps
derived from captured event history; only the rewritten issues' `updated_at` values and
the migration event receive the current `MutationTimestamp`. Checker `started_at`,
`completed_at`, and `duration_ms` remain external observation evidence, while the
persisted `run_id`, issue/gate update, and audit IDs/times are finalized by the store.

Audit finalization preserves the exact existing `events.jsonl` prefix. It inserts one
separator newline iff that prefix is nonempty and lacks a trailing newline, then appends
the sorted canonical records. For `ProfileApplied`, the finalizer—not the command—sets
`isolated_torn_tail` true iff the captured final non-newline line is malformed JSON; a
valid unterminated record still receives the separator but sets the flag false.
Transaction/journal IDs remain private storage recovery identities and are neither
allocated by semantic `IdAuthority` nor included as semantic plan content.

The transaction journal serializer is different: it encodes machine-local recovery
control, not project semantic state (`crates/jit/src/storage/file_transaction.rs:596-603`).
It remains storage/kernel-owned, but only `RepositoryStateStore::apply` may create a
production journal for these repository targets. The existing lock order remains
bootstrap → repository → finer legacy locks (`crates/jit/src/storage/repo_lock.rs:27-31`);
after direct cutover the opaque state-store session is the only public repository
serialization capability and finer locks are private implementation details rather than
callable partial-write boundaries.

### The landing must be one indivisible vertical cutover

The decomposition cannot land a marker parser first and temporarily adapt the profile
and generic renderers to it. That would preserve both old repository images, both target
inventories, and both publication boundaries while adding a third shared seam. The
independently landable unit is one vertical marker + planner + store + all-consumer
cutover. Implementation may be assembled on dedicated `integration/cdc840ad` as five
cumulative, ordered final-code packages: declarations/image/capture/managed documents;
layout-aware store/kernel/recovery; typed mutation/audit/fenced claims v2;
materializers/drift/repair; then all-consumer migration and predecessor deletion. Every
package is based on and tested with all prior packages, exposes only final interfaces,
and none is merged or released independently. The only main-branch landing is
the complete green result. It introduces the final strict managed-document engine, rich
image/bounded discoverer/delta, injected identity/time finalizer, and JSON+memory
`RepositoryStateStore`; migrates **every** JIT-originated repository mutator through
capture → derive → final-image validate → apply; and deletes every predecessor marker,
planner, image, byte producer, writer, export, fixture, and direct test. There are no
main-branch command waves and no marker-first adapter, compatibility re-export, fallback
parser, or dual-publisher phase. The current marker consumers and old owners are
inseparable evidence for this constraint
(`crates/jit/src/profile/render.rs:86-181`,
`crates/jit/src/validation/projection.rs:279-365`,
`crates/jit/src/profile/snapshot.rs:90-124`).

## Formal gate F1–F4 findings

### F1: arbitrary `JIT_DATA_DIR` is not represented end to end

`JIT_DATA_DIR` is currently only a physical storage-root override, not a complete
repository layout. The CLI resolves it as `current_dir.join(custom_dir)` and passes only
that path to `JsonFileStorage` (`crates/jit/src/main.rs:1829-1852`). Storage constructs
its bootstrap lock beside that root, while validation, document commands, and recovery
infer the working-tree root as `data_root.parent()`
(`crates/jit/src/storage/json.rs:183-203`,
`crates/jit/src/validation/repository.rs:146-171`,
`crates/jit/src/commands/document.rs:476-527`,
`crates/jit/src/storage/recovery_coordinator.rs:59-118`). That inference happens to fit
the default `<worktree>/.jit`; it maps a sibling or absolute external data directory's
parent as the working tree and therefore cannot identify both roots correctly.

The transaction kernel independently hardcodes `.jit` as the data-root occupant. It
tests freshness at the physical `.jit` occupant, locates internal journals through the
literal string ".jit/tmp/transactions", opens
recovery controls through that spelling, removes `.jit` after fresh rollback, and
reserves that same literal during normalization
(`crates/jit/src/storage/file_transaction.rs:31-58`,
`crates/jit/src/storage/file_transaction.rs:104-105`,
`crates/jit/src/storage/file_transaction.rs:196-247`,
`crates/jit/src/storage/file_transaction.rs:355-395`,
`crates/jit/src/storage/file_transaction.rs:849-871`). Thus the present kernel and
recovery coordinator do not actually make arbitrary `JIT_DATA_DIR` equivalent to the
default layout even though higher layers virtually strip `.jit` and redirect it to the
selected root.

The same split creates aliases when a custom data root is nested in the worktree.
`FilesystemRepositoryView::resolve` maps paths beginning with the literal ".jit/" prefix through `jit_root` and
every other spelling through `root`, while profile's `transaction_path` repeats that
prefix switch (`crates/jit/src/validation/repository.rs:165-175`,
`crates/jit/src/commands/profile.rs:450-465`). One physical occupant can therefore be
named once through the data mapping and once through its worktree-relative location.
Current transaction/journal actions persist only an unqualified string path, so hashing,
duplicate detection, recovery, and final-identity checks cannot retain which root was
intended (`crates/jit/src/storage/transaction_action.rs:5-31`,
`crates/jit/src/storage/transaction_action.rs:61-84`). Graph and snapshot exporters also
accept physical destinations independently of that virtual mapping
(`crates/jit/src/main.rs:4905-4931`,
`crates/jit/src/commands/snapshot.rs:443-490`). Alias resolution must therefore cover all
mapping, capture, claim, journal, and export consumers rather than only the new path type.

The binding layout decision is one immutable `RepositoryLayout { worktree_root,
data_root }`, constructed at command/server repository resolution and passed through
storage, capture, planning, kernel, journal, and recovery. `VirtualPath` has exactly two
root-qualified forms: `Worktree(rel)` for authored documents, projections, exports, and
`.gitattributes`; `Data(rel)` for index/config/rules/gates/issues/events/provenance/runs.
Their typed `RootRelativePath::{Root, Descendant}` represents the selected root itself
without an empty-string sentinel, or a normalized safe nonempty descendant; root class
is never inferred from `.jit` string syntax. Logical adopter-facing `.jit/events.jsonl` means
`Data("events.jsonl")` under default, relative custom, in-worktree custom, sibling, and
absolute external data roots. Journal actions persist root class plus relative path and
a layout identity, never an absolute physical path; a mismatched layout fails recovery.
External fresh-root controls remain at `Worktree(".jit-bootstrap")`, while internal
controls are `Data("tmp/transactions")`. All parent inference, `.jit` prefix stripping,
kernel literals, and command-local `transaction_path` adapters are deleted.

Canonicalization is global, not exporter-specific. `RepositoryLayout` first validates
the canonical/no-follow physical roots and rejects unsafe overlap: equal/aliased roots,
a worktree nested beneath its data root, symlinked components, lexical escape, or root
identity change during capability acquisition. A data root nested beneath its worktree remains supported, but any
physical path under that selected data root canonicalizes to `Data(...)` **before**
testing worktree containment. Every seed, configured source/target, document/asset,
claim, capture/listing key, delta action, journal action, recovery lookup, report path,
and graph/snapshot destination passes through that one Data-precedence canonicalizer.
Physical-path inputs beneath `data_root` become `Data(...)`; an already typed
`Worktree(...)` that resolves there is rejected as a noncanonical alias before hashing.
Unsafe root topology returns typed `OverlappingRepositoryRoots`; an already-typed alias
returns `DataRootAlias`. Two virtual spellings can never survive as separate
claims/preimages/actions for one occupant. Export classification is
therefore Data first, then Worktree, then external, and journals round-trip the same
canonical root tag rather than reclassifying on recovery.

An absent disjoint data root requires a distinct publication protocol: there is no root
capability in which to create an internal lock or journal, and creating its children one
by one would expose a partial repository. `RepositoryLayout` must therefore open the
existing parent no-follow, reject symlinked or unsupported ancestry, and identify the
prospective root by the parent capability's stable identity plus its single leaf name.
Before testing root existence it acquires the permanent parent-sibling
`DataRootPublicationLock` derived only from that identity/name pair, so two worktrees or
layouts aimed at the same physical absent root serialize on the same guard. The external
durable journal stays under `Worktree(".jit-bootstrap/transactions")`.

Fresh publication builds every `Data(...)` action through a verified sibling staging
directory capability, synchronizes and verifies the complete staged tree, then uses an
atomic no-replace directory rename at the parent and fsyncs that parent. It opens the
published root no-follow and rechecks identity before treating it as `Data(Root)`.
Another occupant is a typed root-publication conflict; rollback/recovery may remove or
rename a staged or published tree only when recorded parent, leaf, and object identities
still match. Thus observers see absence or the complete selected root, never a partly
initialized root. Existing roots continue to use internal
`Data("tmp/transactions")` journals. This parent-capability protocol is required for
relative and absolute disjoint roots alike; using the worktree bootstrap lock alone
would not serialize a second repository targeting the same external root.

Formal gate `2bd35931` adds the permanent ownership proof needed to decide whether the
root found after that race belongs to this repository. Every worktree has
`Worktree(".jit-bootstrap/binding.json")` containing a random
`worktree_binding_id` plus the no-follow filesystem object identity of the worktree root.
The selected data-root `index.json` carries `RepositoryBindingV1` with the same binding
ID and root identity. An absent disjoint-root stage includes that binding before the
stage is verified or published. Under the shared parent/leaf publication lock, a loser
may use the winner's root only when both binding and complete canonical layout match—the
same worktree retry case. Any other occupied root fails
`DataRootOwnedByDifferentWorktree`; it is never adopted from matching content, path,
repository metadata, or timing.

Binding migration is intentionally asymmetric. A legacy unbound root nested strictly
beneath the worktree may be bound once under the recovered locks because no-follow
containment proves its ownership. A legacy unbound disjoint root fails closed during
ordinary open; there is no automatic adoption, compatibility reader, or best-effort
inference. The permanent explicit escape hatch is
`jit migrate bind-data-root --confirm-worktree --confirm-data-root`, which
opens and verifies both roots no-follow, takes bootstrap → data-root-publication →
repository/events locks, records both confirmations, and publishes the two-sided binding
recoverably. The same explicit protocol handles a legitimate root recreation/rebind;
normal init/open cannot impersonate it.

### F2: recovery must be part of opening a mutation session

CLI mutation dispatch currently recovers before constructing `CommandExecutor` and
retains that `RecoverySession` through dispatch; the HTTP server separately recovers
before validating services (`crates/jit/src/main.rs:1824-1868`,
`crates/server/src/lib.rs:66-79`). In contrast, `CommandExecutor::new` only builds config
caches and performs no recovery, so direct in-process callers can read state that a
prepared or committed journal still owns (`crates/jit/src/commands/mod.rs:371-385`). The
checker path demonstrates a partial bridge by dropping a retained startup session for
the subprocess and calling `RecoveryCoordinator` again before verdict persistence
(`crates/jit/src/storage/json.rs:862-899`). Startup orchestration is useful evidence, but
it cannot remain the correctness boundary.

The binding decision is that `RepositoryStateStore::open_mutation_session(layout)` is
the only mutation entry and returns an already-recovered opaque session. It acquires the
worktree bootstrap guard and then the layout's shared `DataRootPublicationLock` before
checking selected-root existence, recovers/verifies/cleans all external prepared and
committed residue, then, when `data_root` exists, acquires the selected data-root
repository and events guards in bootstrap → data-root-publication → repository →
events order and recovers/verifies/cleans all internal residue **before the first capture
read**. For an absent disjoint root the publication guard remains held through staging,
no-replace publication, rollback/recovery, parent sync, final open, and committed cleanup;
no lock is created beneath that absent root. A matching CLI/server startup session
is consumed or reused reentrantly; it never authorizes skipping recovery. Direct
`CommandExecutor` mutations and post-checker persistence use the identical entrypoint.
Any recovery or committed-residue verification error prevents capture and returns a
typed recovery failure. The old standalone coordinator may survive only as private
session-opening mechanics; it is not a caller-selected alternative boundary.
`InMemoryStorage` models prepared/committed residue and injected recovery failure behind
the same open-session-before-capture contract rather than treating recovery as a
filesystem-only concern.

### F3: eligible Git `.gitattributes` behavior must be exact

The current helper uses `git rev-parse --show-toplevel`, silently returns `NotGitRepo`,
considers the marker comment alone sufficient for idempotence, writes the hardcoded
`.jit/events.jsonl merge=union` rule, and lets main downgrade setup errors to a warning
after other init writes have already occurred (`crates/jit/src/storage/gitattributes.rs:25-79`,
`crates/jit/src/main.rs:1938-1947`). The binding policy replaces both ambiguity and the
warning-only split writer:

- Before capture, boundary code resolves typed Git evidence. No containing Git worktree
  or a selected data root outside that same worktree yields exact status
  `not_applicable`, captures no `.gitattributes` target, and init succeeds.
- When eligible, the actual worktree-relative physical path for
  `Data("events.jsonl")` is Git-escaped and determines the merge-rule line. Init
  contributes one semantic line-set claim for `Worktree(".gitattributes")`; capture
  includes the exact file and parent listing. Composition preserves every unrelated byte,
  ensures the canonical JIT marker/rule exactly once, and reports exactly `unchanged`,
  `created`, or `modified` plus matching created/modified path lists.
- The eligible claim lands in the same init delta. Once included, a preimage conflict or
  publication failure fails/rolls back the complete init transaction. A symlink,
  directory, unsupported occupant, unsafe malformed content, or competing semantic claim
  is a typed preflight error before journaling, never a warning/skip. The standalone setup
  API, CLI warning merge, and outcome-driven direct writer disappear.

### F4: claim acquisition is an explicit reconciliation saga

`jit claim acquire` resolves and loads the issue, then durably acquires the Git-backed
lease; only afterward does it reload/mutate the issue, call `save_issue`, and append the
`IssueClaimed` event (`crates/jit/src/commands/claim.rs:72-156`). A failure in either
repository write leaves a live exclusive lease with missing or split repository-visible
assignment/audit. The lease control plane cannot join the repository file transaction,
so merely migrating the last two calls to `RepositoryStateStore` would still leave this
crash window.

The current wire cannot encode that recovery state. `Lease` is a flat nine-field value
whose raw UUID `lease_id` is serialized beside issue/agent/worktree/branch, TTL/times,
and derived `stale`; it has no phase, coordination, generation, or verifier
(`crates/jit/src/storage/claim_coordinator.rs:29-53`). `ClaimOp` only flattens that value
for `Acquire` and addresses `Renew`, `Heartbeat`, `Release`, `AutoEvict`, and
`ForceEvict` by the raw ID (`crates/jit/src/storage/claim_coordinator.rs:102-139`). Both
log and index call their discriminator `schema_version`, and rebuild simply replays those
operations into a raw-ID-keyed active map (`crates/jit/src/storage/claim_coordinator.rs:141-175`,
`crates/jit/src/storage/claim_coordinator.rs:936-1018`). Repository history is weaker:
`IssueClaimed` has exactly ID, issue, timestamp, and assignee, so it cannot establish
which lease attempt produced the assignment (`crates/jit/src/domain/types.rs:1205-1215`).
The CLI currently returns and then reprints/serializes the same raw `lease_id`, including
inside status/list lease objects (`crates/jit/src/main.rs:7287-7299`,
`crates/jit/src/main.rs:7320-7425`). Its input matrix is also unsafe/inconsistent: acquire
correctly takes an issue ID and no credential, but release takes an issue ID, looks up all
leases, and force-evicts them through an owner bypass
(`crates/jit/src/cli.rs:2614-2659`, `crates/jit/src/commands/claim.rs:204-359`); renew and
heartbeat take the raw lease ID, while force-evict also names its admin argument
`lease_id` (`crates/jit/src/main.rs:7366-7425`, `crates/jit/src/main.rs:7607-7628`). These
are migration inputs and deletion evidence, not fields or aliases to extend optionally.

The binding decision makes one one-way migration to the sole current
`record_version: 2` wire for both `ClaimIndex` and `Lease`. `LeaseState` is exactly
`pending_repository_sync | active | legacy_unverified | released | expired |
reconciliation_required`; claim lifecycle operations are exactly `migrated_legacy |
pending | activated | released | expired | reconciliation_required`, and the credential
protocol adds only `handoff_confirmed`. An isolated v1
index migration decodes current records once, appends/fsyncs authoritative complete
`migrated_legacy` snapshots, and regenerates the v2 index; the
append-only log retains only a permanent decode-only v1-history reader. Ordinary
operation reads/writes only v2, with no mixed current reader, serde default, dual write,
fallback, or optional-v1 fields. An expired v1 lease migrates terminal; a live one becomes
`legacy_unverified`, not `active`: the old `Lease`,
`ClaimOp`, and four-field `IssueClaimed` contain no shared coordination evidence, so
declaring repository synchronization would invent a fact. Reconciliation must inspect
captured issue/history under the global guards. The first heartbeat, renew, release, or
reconciliation must match both legacy assignee and historical `IssueClaimed`; mismatch is
typed `LegacyClaimReconciliationConflict`. A match may emit one idempotent legacy-sync
event using the corresponding `acquire|release` operation and `migration: v1_to_v2`;
release may close the matching legacy lease directly, while heartbeat/renew upgrades it
and returns a rotated v2 handle. Recovery/reconciliation may classify and converge the
non-secret fence but never issue or rotate a credential. The consumed v1 alias cannot be reused.
Historical `issue_claimed` remains decode-only and contributes to the first `claimed_at`
fold, but no new operation emits it.

The public owner credential remains named `lease_id` and becomes an opaque bearer handle with exact shape
`lk_<public-key>.<base64url-256-bit-secret>`: a 128-bit lowercase-hex public lookup key
and unpadded base64url 256-bit secret, frozen by regex
`^lk_[0-9a-f]{32}\.[A-Za-z0-9_-]{43}$`. The authoritative v2 log snapshot stores only
`lease_key`, a fresh 128-bit salt, and constant-time-checked
`SHA-256("jit-claim-v2" || salt || secret)`; the index merely projects those fields by
replay.
Owner-authorized calls propagate that exact opaque input to the coordinator, which
authenticates it and then works only with the stored public key/verifier, coordination
ID, generation, and internal `attempt_owner`; callers never parse or reconstruct it. The
secret is never stored in plaintext and neither the complete bearer nor a reversible
form appears in `claims.jsonl`, `claims.index.json`, repository events, status/list or
worktree views, errors, warnings, tracing, snapshots, fixtures, or debug formatting.
Only a successful issuance response or its authenticated same-handoff replay may reveal
the full value; all other human/JSON output is redacted. Status/list use
`lk_<public-key>.REDACTED`, while persisted/event surfaces use `lease_key`. Rotation
invalidates the old verifier before the new attempt can finalize, and a delayed old owner
receives `FencedClaimAttempt`; admin expiry/eviction addresses the public key and does
not disclose or require the bearer. Secret wrappers implement neither general
serialization nor `Display`/revealing `Debug`; CLI/schema/MCP mark full-handle inputs and
one-time outputs sensitive/write-only and distinguish them from the redacted-output
pattern.

The authoritative command matrix is exact:

| Command | Accepted identity/credential | Rotation and output |
|---|---|---|
| `acquire(issue_id)` | Issue ID and durable client `handoff_id`/256-bit `handoff_secret`; no lease credential. | HKDF-derives a fresh generation/key/secret and returns the full bearer once. Retrying the same authenticated handoff returns the identical bearer. A grace takeover is internal to acquire and returns only through it. |
| `renew(full lease_id)` | Current full owner bearer plus a new durable client handoff. | Rotates generation/key/secret and returns the replacement once; afterward the old bearer authenticates only replay of that same handoff. |
| `heartbeat(full lease_id)` | Current full owner bearer. A v1 legacy upgrade also supplies a durable client handoff. | Normal v2 heartbeat neither rotates nor returns/echoes a secret. Only the authenticated v1 upgrade derives and returns its replacement once. |
| `release(full lease_id)` | Current full owner bearer; issue ID is not accepted. | No rotation and no bearer in output. Delete `execute_claim_release_by_issue`, `ReleasedLeaseInfo`, `sanitize_actor`, `resolve_release_actor`, the issue-ID/owner-bypass CLI shape, and every alias/test/doc that preserves that bypass. |
| `force-evict(lease_key, reason)` | Admin public `lease_key` only; never a bearer. | No rotation or replacement credential; output may identify only the public key. |
| `status` / `list` | Filters only. | Redacted `lease_id` only; never usable as a credential. |
| recovery / reconciliation | Internal non-secret fencing identities only. | Never accepts, rotates, returns, logs, or reconstructs a secret. |

Formal gate F2 makes the handoff a permanent crash-safe client protocol, not a temporary CLI workaround.
Every acquire, renew, or legacy upgrade that creates a credential begins with a
client-generated `handoff_id` and 256-bit `handoff_secret`, supplied through a sensitive
stdin/file/API field and retained by the caller before mutation starts. The coordinator
generates a nonce and HKDF-derives the lease key and secret from the handoff secret plus
that nonce, a fixed domain separator, coordination ID, and generation. It persists no
plaintext secret: the non-secret lease snapshot contains only the normal salted lease
verifier plus `handoff_id`, nonce, a separately salted handoff verifier, and
`credential_handoff_pending`. A retry authenticating the same handoff deterministically
derives and returns the same bearer; acquire retry is authenticated by the handoff
secret even though acquire has no prior lease credential. During rotation the old bearer
is accepted only to replay that exact pending handoff, never for heartbeat, release,
another renewal, or a different handoff.

The first successful use of the new bearer appends `handoff_confirmed`; replay then
projects away the old credential verifier and handoff verifier from current state. No
server recovery path can reveal or reconstruct a bearer without the caller's handoff
secret, and there is no separate recover-credential command. By default the CLI
atomically creates a caller-owned mode-`0600` handoff file before entering coordinator
mutation; sensitive stdin/file/API fields allow a caller to provide equivalent durable
input. Schema and MCP distinguish this write-only input from `lease_id`, never serialize
it into ordinary output, and preserve the same authenticated replay. The caller may
delete the file after confirmation; until then it is the caller's durable credential
handoff, not server state.

Formal gate F3 makes `claims.jsonl` the sole current claim-control authority and
`claims.index.json` a disposable replay projection. Every v2 operation—including
`migrated_legacy`, lifecycle operations, rotations with
`credential_handoff_pending`, and `handoff_confirmed`—contains the complete non-secret
`LeaseV2` snapshot needed to rebuild state: `lease_key`, salts/verifiers, handoff ID,
nonce and pending/confirmed disposition, `LeaseState`, coordination ID, generation,
canonical issue/agent/worktree identities, TTL/times, and internal attempt owner where
applicable. Mutation appends and fsyncs the authoritative operation first, then rewrites
the derived index atomically. A crash between them is recovered only by log replay; no
command trusts newer/different index content.

`migrated_legacy` carries `legacy_unverified` plus a salted hash verifier for the legacy
alias, never an index-only raw alias. Every rotation appends a new complete snapshot.
The index may cache the replay result but owns no verifier, handoff evidence, state, or
transition absent from the log. Current replay has one v2 snapshot reducer plus the
permanent decode-only v1 history boundary; partial v2 ops, index-first mutation,
index-only credential data, and reconstruction by patching an index record are deleted.

The cross-substrate transition remains lease-first but is now a fenced durable saga.
The global claim-operation order is coordinator → bootstrap → data-root-publication →
repository → events. Acquire holds coordinator while it records canonical issue
identity and a `pending_repository_sync` lease containing operation `acquire`, stable
`coordination_id`, generation, and internal attempt owner; it then opens the recovered
repository session and submits one issue-plus-event mutation. The sole new event is
`issue_claim_lease_changed` with exactly `{id, timestamp, issue_id, assignee, operation:
acquire|release, lease_key, coordination_id, generation, migration: null|v1_to_v2}`. It
carries no secret or internal owner token. A current
acquire event also participates in the `claimed_at` fold; release preserves that
first-claim history.

Still under the same coordinator guard, completion conditionally appends `activated`
only when operation, coordination ID, generation, and internal owner remain unchanged;
definite pre-journal failure conditionally removes that exact pending attempt. Release
mirrors acquire and appends `released` after its repository issue/event transition, or
restores active on a proven pre-journal failure. Uncertain publication or conditional
finalization appends/retains `reconciliation_required`, never false atomic success.
After a crash a reconciler takes coordinator first, opens/recovers repository state, and
compares the exact event fence. A match finalizes without a duplicate; ambiguous evidence
remains reconciliation-required. It never receives or issues a bearer. Proven absence
may become eligible after grace, but only a later `acquire(issue_id)` performs the
internal takeover, increments generation, replaces the internal owner, rotates key and
secret, and returns that new handle through acquire. Finite expiry and TTL-zero stale
takeover also pass through this fence. Control-plane records stay outside
`RepositoryStateStore`, but every repository-visible correction uses the nested recovered
session; no repository-first path may later acquire coordinator.

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

- Layout/root inference consumers that must move together:
  `crates/jit/src/main.rs`, `crates/server/src/main.rs`,
  `crates/server/src/lib.rs`, `crates/jit/src/storage/json.rs`,
  `crates/jit/src/storage/discovery.rs`,
  `crates/jit/src/validation/repository.rs`, and command paths that currently use
  `storage.root().parent()` for working-tree documents/exports. They construct or consume
  one `RepositoryLayout`; no layer reconstructs one root from the other. Init/open and
  the new explicit `migrate bind-data-root` also own the permanent worktree/data binding,
  nested one-way binding, disjoint fail-closed behavior, and recreate/rebind protocol.
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
  `crates/server/src/lib.rs:18-69`), `CommandExecutor` direct mutation entrypoints, and
  `IssueStore::run_external_process`/checker persistence. Startup coordinator calls are
  folded into `open_mutation_session`; they do not remain an alternative prerequisite.
- Cross-substrate claim-saga consumers:
  `crates/jit/src/commands/claim.rs`, `crates/jit/src/storage/claim_coordinator.rs`,
  claim status/heartbeat/renew/release/eviction and worktree views, claim schemas/log
  rebuild/migration, CLI output/error mapping, domain event decoding/claimed-at folds,
  credential-handoff input/file handling, schema/MCP sensitivity metadata, and claim
  validation. All must understand exact v2 states/operations and use
  coordination ID plus generation/internal owner for reconciliation. Owner operations
  follow the exact matrix: acquire issue/no credential; renew full bearer with rotation;
  heartbeat full bearer without normal rotation/output; release full bearer without
  rotation/output; force-evict public key; status/list redacted; recovery/reconciliation
  secret-free. Persistence/events use only public `lease_key`, and debug/error/validation
  paths expose no secret. All reads reduce authoritative complete-snapshot log entries;
  the index is never a credential/state authority. Release-by-issue, partial v2 ops,
  index-only fields, alternate credential recovery, CLI/schema/MCP/docs/tests, and
  owner-bypass helpers are deletion consumers, not compatibility surfaces.
- Tests:
  unit tests in the transaction and init/profile modules and
  `crates/jit/tests/cli_issue/recover_command_tests.rs`, plus server recovery, claim
  coordinator/property, claim CLI, custom-data-root, and direct-executor suites.
- Documentation:
  `docs/reference/profiles.md`, `docs/reference/cli-commands.md`,
  `docs/reference/storage-format.md`, and `docs/concepts/guarantees.md`.

### Complete repository-mutator cutover inventory

The following classification covers production code that can alter a live repository
path or a typed record from which addressable/derived state is computed. “State-store
owned” means the command supplies a typed semantic intent and receives result metadata;
it does not retain a raw typed-record save, byte serializer, path writer, or compensating
publisher.

| Mutator family | Current production surfaces | Sole `RepositoryStateStore` disposition |
|---|---|---|
| Issue creation/update/state/assignment/release/deletion and event side effects | `commands/issue.rs` writes issues/events and deletion also rewrites dependents (`crates/jit/src/commands/issue.rs:68-170`, `crates/jit/src/commands/issue.rs:427-755`, `crates/jit/src/commands/issue.rs:782-1053`). | **State-store owned.** Issue descriptions and labels are issue-scoped item declarations, and `index.json`/events are coupled state. Every operation becomes one semantic mutation; direct save/delete/append calls are removed. |
| Dependency, label, document-reference, bulk, batch/template, and validation-fix writes | `commands/dependency.rs:133-263`, `commands/dependency.rs:554-740`, `commands/labels.rs:20-46`, `commands/document.rs:7-176`, `commands/bulk_update.rs:162-390`, `commands/batch_create.rs:275-355`, `commands/template.rs:208-588`, and `commands/validate.rs:37-185`. | **State-store owned.** These all change issue item text/links, graph-visible records, timestamps, or audit. Batch/template rollback helpers and `restore_issue_verbatim` are replaced by prevalidated aggregate deltas, not retained as compensating escape hatches. |
| Document asset rescan | Rescan reads the current linked document, replaces the stored asset inventory, and calls `save_issue` without appending an event (`crates/jit/src/commands/document.rs:672-759`). | **State-store owned.** Rescanned asset discovery consumes captured document bytes. A changed inventory is one typed mutation containing the issue update, `MutationTimestamp`, canonical audit event, and all affected item/validation/projection effects; the current unaudited direct save disappears. |
| Lifecycle timestamp migration | Migration derives first historical lifecycle occurrences from event history, then independently saves changed issues and appends one migration event (`crates/jit/src/domain/queries.rs:11-99`, `crates/jit/src/commands/migrate.rs:19-80`). | **State-store owned.** Historical lifecycle values retain their captured event timestamps. The current mutation instant applies only to rewritten `updated_at` values and the one migration audit record, and all bytes land in one delta. |
| Claim acquire/release repository synchronization | Claim acquisition first mutates the Git-backed lease control plane, then separately saves issue/event (`crates/jit/src/commands/claim.rs:72-156`). Current release is an issue-ID owner bypass; current coordinator reads index state and appends partial ops before patching it (`crates/jit/src/commands/claim.rs:159-415`, `crates/jit/src/storage/claim_coordinator.rs:440-580`). | **Split substrate, fenced v2 saga, durable client handoff, and log authority.** Credential-creating acquire/renew/legacy upgrade receives durable handoff input; HKDF/authenticated replay returns the same bearer after response loss, old credentials only replay their pending handoff, and first new-bearer use confirms it. The exact command matrix and frozen repository event remain. Every coordinator mutation fsyncs one complete non-secret `LeaseV2` snapshot before atomically regenerating the index; replay is recovery. No plaintext/recoverable server secret, partial op, index-only verifier/state, credential-recovery command, or release bypass survives. |
| Gate assignment/status/definition/preset application and checker recording | `commands/gate.rs` changes issue gates, gate declarations, and events (`crates/jit/src/commands/gate.rs:211-355`, `crates/jit/src/commands/gate.rs:370-624`, `crates/jit/src/commands/gate.rs:634-990`, `crates/jit/src/commands/gate.rs:1020-1108`); `commands/gate_check.rs` writes run/issue/event (`crates/jit/src/commands/gate_check.rs:282-331`). | **State-store owned.** Gate declarations are registry-first item SSOT, issue statuses affect lifecycle, and run/event records are the audit of the same verdict. One post-checker mutation owns all affected bytes and projections. |
| Custom gate-preset creation | `create_gate_preset` serializes a new `.jit/config/gate-presets/<name>.json` through `IssueStore::save_gate_preset` (`crates/jit/src/commands/gate.rs:1135-1190`, `crates/jit/src/storage/json.rs:1335-1354`). | **State-store owned.** A custom preset is a repository-authored future mutation input. Its typed definition and target claim join the image/delta; the raw trait writer is deleted. |
| Repository config mutation, default rule/schema refresh, gate/rule declaration publication, project render, profile apply, and init | `commands/config.rs:177-310`, `commands/mod.rs:1105-1265`, `commands/project.rs:87-170`, `commands/profile.rs:122-245`, `commands/init.rs:173-452`, plus the storage writers inventoried above. | **State-store owned.** These are the core declared→derived materialization paths. The cutover deletes every independent publisher/action builder, including ordinary init's separate `IssueStore::init` path. User-global config is the explicit exception below. |
| Eligible Git `.gitattributes` setup performed by init | `main` calls the helper separately from repository initialization, warns on helper errors, and reports create/modify outcomes (`crates/jit/src/main.rs:1938-1947`, `crates/jit/src/main.rs:2009-2017`); the helper detects Git and directly writes the versioned root file (`crates/jit/src/storage/gitattributes.rs:42-79`). | **Conditionally state-store owned.** No-Git/outside-worktree is exact `not_applicable`. Otherwise the Git-escaped relative selected-data events path produces a same-delta `Worktree(".gitattributes")` line-set claim and exact `unchanged|created|modified`; unsafe eligible targets abort before journaling. The direct helper and CLI warning merge are deleted. |
| Dependency-aware document/container archive | Archive publishes destination documents/markers, relinks issue documents, appends audit, and deletes sources (`crates/jit/src/commands/archive.rs:398-636`, `crates/jit/src/commands/archive.rs:733-856`). | **State-store owned.** Any archived path may be a linked document or config-selected item/projection source/target. Identity/no-replace checks survive as policy and expected preimages; alternate artifact publication/deletion and reconciliation writers do not. |
| User-directed graph and snapshot exports | Graph export writes an arbitrary requested output path or stdout (`crates/jit/src/main.rs:4905-4931`); snapshot export assembles and publishes an arbitrary directory/archive, often beneath the current directory (`crates/jit/src/commands/snapshot.rs:443-490`, `crates/jit/src/commands/snapshot.rs:553-638`). | **Classified canonically by destination.** Graph stdout is non-mutating. Physical output under selected data is `Data` even when data nests in worktree; otherwise output under worktree is `Worktree`; only outside both is external. Every virtual output file/archive/tree is one state-store export intent with exact preimages, complete parent listings, canonical collision/alias checks, and no direct publisher. |
| Manual editor/build-tool changes to versioned repository files | External tools can always edit config, registries, issues, sources, or targets; git-versioned plain files are a product requirement (`@/charter/D-1`). | **Not capturable as a command cutover.** The store does not monopolize the filesystem. It owns all JIT-originated publication and uses complete under-lock preimages; external edits before session capture become input, and edits racing after capture fail expected-preimage checks. Validation reports their derived drift. |
| User-global config, Git hooks/claim control plane, caller-owned credential-handoff files, worktree identity, server PID/logs, locks/temp files, recovery journals, and snapshot/export destinations outside the repository | User-global config branches at `commands/config.rs:253-275`; hooks and claims write Git control paths; worktree/server/lock/recovery files are machine-local (`crates/jit/src/storage/worktree_identity.rs:243-251`, `crates/jit/src/commands/serve.rs:83-101`, `crates/jit/src/storage/repo_lock.rs:40-44`). | **Excluded from semantic state.** Each keeps its substrate-specific capability. CLI handoff files are atomic mode-`0600` caller secrets created before coordinator mutation and deleted only after confirmation, never repository/server truth. The API must make these exclusions structural, so none can accept an arbitrary live repository target. |

This inventory means the final “sole store” claim is broader than deleting the five
obvious config/profile/project writers. Every JIT-originated `Worktree` target and every
`Data` target enters `RepositoryStateStore` regardless of whether the physical selected
data root is inside, beside, or outside the worktree. Only an explicitly classified
external-export target outside both layout roots or a machine/control-plane substrate is
excluded, and that capability is structurally unable to publish through a virtual
repository path. Otherwise a configured source/target can still change outside the
captured read-set and SSOT boundary.

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
publication access. A gate-run result is not an authored registry declaration, but the
post-checker `GateCheckMutation` still publishes its run record, issue status, audit
event, and any affected derived projection through the same state-store delta.

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
- `dev/archive/cdc840ad-plan.md:21-37` is the final implementation-vocabulary decision:
  crate-root `declarations` and `repository_state`, one `RepositoryStateStore`, deletion
  of both old images and every old publisher/init/profile inventory, and no re-export
  bridge. Its research-to-final mapping at `dev/archive/cdc840ad-plan.md:37` explicitly
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
- crate-root `repository_state` owns immutable `RepositoryLayout`, root-qualified
  `VirtualPath::{Worktree, Data}`, rich `RepositoryEntry`, `RepositoryImage`, overlay,
  `RepositoryBindingV1`/binding-match semantics, seed, intent, target claim, exact delta,
  managed-document engine, all materialization producers, and pure derive/compare
  functions. It depends on declarations/config, never
  validation, storage, commands, or profile;
- validation imports declarations plus repository-state images and derive/compare
  results, evaluates rules and whole-repository policy, and validates final images. It
  owns no filesystem capture, alternate image/overlay, default/schema/projection
  materializer, target composer, or marker engine;
- storage implements `RepositoryStateStore`: open the canonical already-recovered opaque
  mutation session for one layout, capture one `RepositoryImage`, and apply one finalized
  `RepositoryDelta`. JSON gives the file-transaction kernel capabilities for both typed
  roots; memory models both roots and recovery residue behind an aggregate
  clone/apply/validate/swap boundary. Storage owns no command semantic or materialization
  renderer. It supplies no-follow root identities and recoverably publishes/verifies the
  permanent two-sided binding; and
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
the session, but application opens the `RepositoryStateStore` session for its explicit
`RepositoryLayout`, completes mandatory recovery before capture, rebuilds the delta with
exact expected preimages, validates the final image, and applies once before releasing
it. Profile apply already replans under its repository guard
(`crates/jit/src/commands/profile.rs:122-147`), and CLI startup holds a recovery session
through dispatch (`crates/jit/src/main.rs:1852-1868`), but neither is a substitute for
the new backend contract. Session acquisition takes the worktree bootstrap guard first
and the layout-wide `DataRootPublicationLock` second, before checking whether the
selected root exists. An absent disjoint `data_root` uses the existing parent capability
and parent-sibling lock, keeps its durable journal at
`Worktree(".jit-bootstrap/transactions")`, and stages the complete root beside its final
leaf; it creates no guard/control child under the absent root. An existing selected root
must match the permanent worktree/data binding and canonical layout under those guards
before it is opened as repository state; an unbound disjoint root exits to the explicit
double-confirmation migration rather than continuing. A bound existing root then acquires repository/events serialization and uses
`Data("tmp/transactions")` internal control. The recovery coordinator demonstrates
external-before-internal order
(`crates/jit/src/storage/recovery_coordinator.rs:59-102`), while the kernel's current
hardcoded `.jit` locations are evidence to replace, not preserve. Fresh init must not
create or acquire a lock inside an absent selected data root, and direct executor capture
must not precede recovery. Ordinary mutations enter at bootstrap; claim acquire/release,
status reconciliation, heartbeat, renew, and eviction enter with the coordinator guard
already held and may then nest the recovered session. The state store never acquires the
coordinator, and lock APIs/runtime checks reject any coordinator acquisition while a
bootstrap/data-root-publication/repository/events guard is held.

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
- `RepositoryLayout` conformance for default `.jit`, relative/in-worktree custom,
  sibling, and absolute outside-worktree data roots: identical `Data` semantics, distinct
  `Worktree` semantics, root-tagged journal round trips, mismatch rejection, selected-root
  locks/control cleanup, and structural absence of `.jit`/parent-inference adapters.
  Equal/physical-alias roots, worktree-beneath-data, symlinked components, lexical escape,
  and root-identity change reject; supported data-beneath-worktree topology proves global Data precedence
  across configured sources/targets, documents/assets, capture/listings, deltas, journals,
  collision detection, reports, exports, and recovery, with typed Worktree aliases rejected;
- absent-disjoint-root conformance proving no capability/lock/journal is opened inside
  the absent leaf; parent identity plus leaf yields one shared publication lock; every
  `Data` action applies to a verified sibling stage; and other processes observe absence
  or one complete no-replace-published root. Inject failure at stage creation, every
  write/mode/sync, stage verification, no-replace rename, parent fsync, final no-follow
  open/identity check, rollback, prepared recovery, committed cleanup, and competing-root
  occupation. Race two distinct worktrees/layouts against the same absent disjoint root;
- repository-binding conformance proving the permanent worktree binding and data-root
  `RepositoryBindingV1` agree on random binding ID and no-follow root-object identity;
  the absent stage contains the binding; and a race loser accepts only the exact same
  binding/layout retry. Different worktree, copied binding, changed root identity, or
  layout mismatch yields `DataRootOwnedByDifferentWorktree`. Nested unbound roots bind
  once by proven containment. Disjoint unbound ordinary open always fails, while only
  `jit migrate bind-data-root --confirm-worktree --confirm-data-root` under all guards
  can bind/recreate/rebind; missing confirmation, races, symlinks, identity change,
  partial two-sided publication, auto-adoption, and compatibility-reader cases fail;
- prepared and committed external/internal residue tests proving CLI, server, direct
  `CommandExecutor`, and post-checker mutation all recover/verify before the first capture
  read, including injected recovery failure that prevents capture;
- bounded-discovery tests proving the captured read set expands through schema
  references, indexed issues plus issue-directory enumeration, working-tree linked/plan
  documents, configured item sources, projection sources/targets, and intent targets;
  changing any discovered entry after planning must cause an expected-preimage conflict,
  while unrelated build trees are not captured;
- deterministic typed-to-byte conformance across JSON and memory using fixed
  `MutationClock` and `IdAuthority` implementations: no-op allocation counts, stable
  new-issue/run/event ordering, one mutation timestamp across issue/lifecycle/gate/audit
  fields, historical lifecycle migration values, preserved checker observation times,
  profile records, gate-run records, and custom presets. Audit cases cover empty and
  newline-terminated prefixes plus valid and malformed non-newline tails, exact prefix
  preservation/separator bytes, and the resulting `isolated_torn_tail` value;
- a multi-target project render publication failure after one target has been installed;
- duplicate same-ID and crossing marker rejection, plus successful distinct-ID nesting;
- validation errors for missing/stale derived schema, membership, projection, and region
  targets, followed by idempotent transactional repair;
- concurrent mutation serialization through direct in-process APIs as well as CLI;
- gate definition add/define/update/remove, issue gate add/remove, and preset application
  proving registry, issue, audit, and configured projection bytes land in one delta;
- gate checker recording, custom preset creation, lifecycle migration, document asset
  rescan, and archive publication/relink/deletion proving each complete mutation lands in
  one delta with audit and has no alternate publisher;
- Git-aware init proving eligibility for default and worktree-contained custom data
  paths, the actual events-relative rule, unrelated-byte preservation, idempotence,
  same-delta rollback, exact four-state reporting, `not_applicable` for no-Git and
  outside-worktree roots, and preflight rejection of unsafe eligible occupants/content;
  graph stdout proving no mutation; graph/snapshot destinations under nested data proving
  `Data`, elsewhere under worktree proving `Worktree`, and outside both proving external,
  with state-store preimage, complete-parent-listing, alias collision, and recovery behavior;
- claim-v2 and saga tests at every boundary: one-way conversion of each v1 `Lease` and
  `ClaimOp` into exact `record_version: 2` records, live migration state
  `legacy_unverified`, `migrated_legacy` log evidence, no v1 ordinary read/write, and no
  inference of synchronization from historical `issue_claimed`. New events have exact
  `issue_claim_lease_changed` fields, acquire folds `claimed_at`, release preserves its
  first value, and historical events remain decode-only. Bearers match exactly
  `^lk_[0-9a-f]{32}\.[A-Za-z0-9_-]{43}$`; stored 128-bit salt/domain-separated hash
  authenticates in constant time. Matrix tests prove acquire accepts issue/no credential
  plus durable handoff and returns one fresh handle; grace takeover exists only inside acquire; renew requires
  the full bearer, rotates generation/key/secret, and returns replacement once; normal
  heartbeat requires the full bearer without rotation/secret output, except v1 legacy
  upgrade; release requires the full bearer without rotation/secret output; force-evict
  accepts only public `lease_key`; status/list redact; recovery/reconciliation are
  secret-free. The old bearer is fenced immediately after every rotation. Persisted/
  logged/evented data contains only `lease_key`; status/list/worktree use
  `lk_<public-key>.REDACTED`; only successful authority issuance or authenticated
  same-handoff replay reveals the full handle,
  and debug/error/warning/tracing paths never reveal it. Crash-safe handoff tests cover
  response loss after every append/fsync/index/repository/finalization boundary; same
  authenticated `handoff_id`/secret deterministically replays the same bearer; wrong or
  changed handoff fails; an old rotating bearer can only replay its pending handoff; and
  first new-bearer use appends `handoff_confirmed` and projects away old/handoff
  verifiers. CLI tests prove atomic pre-mutation mode-`0600` creation, supplied sensitive
  stdin/file/API equivalence, retention through uncertainty, deletion after confirmation,
  and no plaintext server/log/index/recovery/schema/MCP/output secret. Structural/CLI/schema/MCP/docs
  tests prove `execute_claim_release_by_issue`, `ReleasedLeaseInfo`, `sanitize_actor`,
  `resolve_release_actor`, issue-ID release, and every alias are absent. Saga injection covers pending
  acquire/release before repository publication, definite pre-journal compensation,
  uncertain recovery, commit-before-conditional-finalization, retry/event dedupe,
  startup and every claim-entry reconciliation, all six `LeaseState` values, all six
  lifecycle operations, and `handoff_confirmed`, TTL-zero grace/finite expiry, delayed stale-owner
  finalize/delete rejection, and the enforced coordinator → bootstrap →
  data-root-publication → repository → events order with reverse acquisition absent;
- claim-log authority tests prove every v2 op contains the complete non-secret `LeaseV2`
  replay snapshot; append+fsync precedes index rewrite; interruption at every byte/sync/
  rename boundary converges by log replay; a deleted/corrupt/stale/ahead index is rebuilt
  or rejected as projection, never selected as truth; rotations and confirmation replay
  exactly; and `migrated_legacy` carries hashed alias verifier plus
  `legacy_unverified`. Structural scans find no partial current op, index-first write,
  index-only verifier/handoff/state/fence, or raw legacy alias in a v2 snapshot;
- cumulative `integration/cdc840ad` evidence runs formatting, lint, conformance,
  structural absence, and affected integration suites after each of the five ordered
  component packages against all preceding packages. No component package is mergeable
  on its own, and the final
  main-branch change is one green landing containing all consumer migrations and
  predecessor deletions;
- fixture migration proving no production/test/doc consumer retains `IssueStore::init`,
  the validation view, profile snapshot/projection/drift types, old declaration owners,
  raw gate/config/rules/project writers, or compatibility re-exports;
- operation without a git repository, consistent with the core-command exception stated
  at `AGENTS.md:162-163`; and
- unchanged human/JSON envelopes for profile apply, project render, init, config, and
  validate fix.

This investigator modified no code or `.jit/` state.
