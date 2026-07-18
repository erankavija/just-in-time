# Repository materialization coherence — architecture research

**Container:** `cdc840ad` — Transactional repository materialization and derived-state coherence  
**Planning node:** `dc266cb6`  
**Date:** 2026-07-18

## Provenance convention

- **[VERIFIED]** means the claim was confirmed against repository code, project
  documentation, a resolved addressable item, or a local execution recorded here.
- **[CITED]** would mean an external source is load-bearing; no external source was
  needed for these decisions.
- **[ASSUMED]** means a proposed design or forecast; each such claim states the risk
  if the assumption is wrong.

## Resolved project constraints

- **[VERIFIED]** Repository state is repository-local rather than database-backed
  (`@/charter/D-1`, resolved with `jit item show`; `AGENTS.md`, Project Overview).
- **[VERIFIED]** Core commands must remain usable without Git
  (`@/charter/D-4`, resolved with `jit item show`; `crates/jit/src/storage/repo_lock.rs:1-13`).
- **[VERIFIED]** Item kinds declare their own source of truth
  (`@/charter/D-6`, resolved with `jit item show`), and this epic explicitly keeps
  declared configuration and registries authoritative while treating schemas,
  default membership, projections, and managed regions as materializations
  (`jit issue show cdc840ad`).
- **[VERIFIED]** The storage layer already owns durable file-set publication through
  `FileTransactionKernel`; callers provide complete bytes and a held repository
  write guard (`crates/jit/src/storage/file_transaction.rs:1-10,63-91`).
- **[VERIFIED]** The transaction kernel records original and final identities,
  stages complete bytes, rechecks the recorded original immediately before each
  publish action, and rolls back a failed publication
  (`crates/jit/src/storage/file_transaction.rs:399-489,492-593`).
- **[VERIFIED]** The current kernel records its original identity during transaction
  preparation, after command planning; `TransactionAction::WriteFile` does not carry
  the planner's expected preimage
  (`crates/jit/src/storage/transaction_action.rs:5-31,44-59`;
  `crates/jit/src/storage/file_transaction.rs:428-489`).

## Current integration failure that the design must close

- **[VERIFIED]** In a fresh temporary repository, `jit init --profile jit-dogfood`
  succeeded, the resulting `.jit/config.toml` declared `[namespaces.brackets]`, but
  `.jit/rules.toml` and `.jit/schemas/` contained no
  `namespace-unique-brackets` materialization. The command
  `jit item show @/rule/namespace-unique-brackets --json` then failed with
  `ITEM_COMMAND_FAILED` while `jit validate --json` passed with zero errors. The
  reproduction was run on 2026-07-18 with the installed current binary using:

  ```text
  jit init --profile jit-dogfood --json
  jit item show @/rule/namespace-unique-brackets --json   # exit 1
  jit validate --json                                    # exit 0
  ```

- **[VERIFIED]** This behavior follows the code paths: profile planning merges
  semantic contributions and profile files, then re-renders configured generic
  projections, but it does not call default-rule membership or default-schema
  derivation (`crates/jit/src/profile/planner.rs:155-225`). Effective rule loading
  reconciles default membership in memory from config, so validation behavior can
  be correct while persisted addressable membership is stale
  (`crates/jit/src/validation/repository.rs:578-620`;
  `crates/jit/src/validation/defaults.rs:3-17`).
- **[VERIFIED]** Ordinary `config set` writes `config.toml` first and invokes schema
  refresh and membership sync afterward, leaving those files outside one file-set
  transaction (`crates/jit/src/commands/config.rs:253-268`).
- **[VERIFIED]** `jit project render` plans all selected projection bodies first,
  composes shared targets in memory, then calls one atomic storage write per target;
  it does not publish the complete target set through `FileTransactionKernel`
  (`crates/jit/src/commands/project.rs:109-175`).
- **[VERIFIED]** Gate-definition commands mutate the authoritative gate registry
  through direct `save_gate_registry` calls, while the rules-and-gates prose is a
  configured derived projection. Define, update, remove, and preset-driven registry
  changes can therefore leave that projection outside the semantic write
  (`crates/jit/src/commands/gate.rs:673,742,966-986,1093`;
  `.jit/config.toml:219-226`).

## Question 1 — Where should the shared pure planner live?

### Why this blocks the plan

- **[VERIFIED]** The repository has two overlapping read models. Validation's
  `RepositoryView` exposes file bytes, file listings, and overlay tombstones but
  not file mode or occupied non-file kind
  (`crates/jit/src/validation/repository.rs:119-138,180-280`). Profile's
  `RepositorySnapshot` separately exposes file bytes/mode, directory, symlink, and
  unsupported entries, then adapts them back to the narrower validation view
  (`crates/jit/src/profile/snapshot.rs:7-40,66-124`).
- **[VERIFIED]** Whole-repository validation, default-rule serialization, and
  generic projection rendering are already side-effect-free after their inputs are
  loaded (`crates/jit/src/validation/repository.rs:297-464`;
  `crates/jit/src/validation/mod.rs:30-32`).
- **[VERIFIED]** Profile planning already depends on those validation primitives;
  moving the common planner into profile would make ordinary init, config, project
  render, and validation repair depend on a package-specific subsystem
  (`crates/jit/src/profile/planner.rs:1-9`).
- **[VERIFIED]** Putting semantic derivation in storage would violate the existing
  boundary that storage owns mechanics while callers own serialization
  (`crates/jit/src/storage/file_transaction.rs:1-5`) and the repository's stated
  separation of concerns (`AGENTS.md`, Key Design Principles).

### Options considered

| Option | Evidence and trade-off | Provenance |
|---|---|---|
| Put the planner in `profile` and let other commands reuse it | Profile already has exact target planning, target actions, hashes, and overlay validation (`crates/jit/src/profile/planner.rs:23-89,132-275`). It also owns package-only vocabulary and explicitly excludes repository application lifecycle in its module contract (`crates/jit/src/profile/mod.rs:1-5`), so making it the repository-wide owner would invert that contract. | **[VERIFIED] rejected** |
| Put derivation into `storage` beside `FileTransactionKernel` | Publication would be close to planning, but storage would need config, rule, projection, profile, and validation semantics. The kernel currently accepts only deterministic storage actions and owns no serialization (`crates/jit/src/storage/file_transaction.rs:1-5,33-43`). | **[VERIFIED] rejected** |
| Put all orchestration in `commands` | Commands are already the application orchestration boundary (`crates/jit/src/commands/mod.rs:1-29`), but command-local helpers are the duplication this epic is consolidating: project render, config refresh, init, and profile each assemble different subsets. A command module would also be awkward for `validate_repository` to call without a dependency inversion. | **[VERIFIED] rejected** |
| Add the planner beside validation's current view | This minimizes file movement, but repository entry identity, semantic mutation planning, managed target ownership, and transaction input are broader than validation. It would preserve the split with profile's richer snapshot and make a validation namespace the owner of gate/config/profile mutation state. | **[VERIFIED evidence, ASSUMED consequence] rejected** |
| Create one final crate-root `repository_state` subsystem | A neutral subsystem can own the canonical entry image, overlays, exact delta, target ownership/managed-document composition, derivation, and comparison. Validation consumes its image and comparison output; profile and commands contribute neutral seeds; storage consumes its exact delta. | **[ASSUMED] recommended** |

### Recommendation

- **[ASSUMED — recommended design]** Add the final crate-root `repository_state`
  subsystem now. It owns `RepositoryPath`, `RepositoryEntry`, `RepositoryImage`,
  `RepositorySeed`, `MaterializationIntent`, `TargetClaim`, `RepositoryDelta`, the
  canonical overlay, managed-document topology, materialization derivation, and
  drift comparison. Validation owns semantic/rule evaluation over the resulting
  image; commands own use-case orchestration; profile owns package parsing; storage
  owns transactional publication. The risk is a broader cutover, but retaining a
  transitional owner would preserve competing abstractions inside the v1.0
  correctness boundary.
- **[ASSUMED — recommended design]** Define exactly one entry API on
  `RepositoryImage` whose result distinguishes
  absence, regular file (exact bytes, normalized mode, identity), directory,
  symlink, and unsupported occupant. Listing returns the same entry vocabulary in
  deterministic path order. Filesystem reads use no-follow/capability-confined
  traversal and fail closed on a symlink in any component; overlays carry exact
  file modes, directory claims, and deletion tombstones. The risk is more explicit
  handling at callers, which is the required cost of preventing byte-only
  validation and mode-aware profile planning from seeing different repositories.
- **[ASSUMED — recommended design]** Remove the profile-only
  `RepositorySnapshot` entry hierarchy and validation-only repository view in the
  same cutover after all consumers use the crate-root types. Do not retain a
  second path under the old modules. The risk is test migration volume; retaining
  both would leave two safety and identity contracts to drift.
- **[ASSUMED — recommended design]** Make producer ownership acyclic and explicit.
  Move the declarative `GateRegistry`/`GateDefinition` and `RuleSet`/`Rule` models
  from `storage` and `validation` into one neutral, pure domain/config declarations
  module. Move materialization-only default derivation and serialization, generated
  schema rendering, configured project-projection rendering, and managed-document
  composition into `repository_state`. Validation imports the neutral declarations
  plus `repository_state` for derive/compare, and storage imports the declarations
  for persistence. `repository_state` never imports `validation`, `storage`, or
  `profile`; validation-specific rule evaluation remains in `validation`. Delete
  the former definitions and exports in the same cutover, with no re-exports or
  compatibility aliases. This preserves `derive -> compare -> validate`; the risk
  is a broad source-file move, but leaving materialization producers behind a
  validation or storage facade would preserve the dependency cycle.
- **[ASSUMED — recommended design]** Define a neutral seed/delta API, conceptually:

  ```text
  RepositorySeed       = declared/profile/audit candidate changes
  MaterializationIntent = SemanticMutation | RenderProjections | RepairDerived
  RepositoryDelta      = exact sorted target changes + read-set/plan hash
  ```

  The semantic-mutation intent must always run every coupled producer. There is no
  callback/provider registry and no caller-selected producer flag or arbitrary
  producer list; constrained intents encode complete operations. The risk of a
  freely selectable producer surface is recreating partial command-specific paths
  under a shared type name.
- **[ASSUMED — recommended design]** Profile remains responsible for package
  parsing and package validation, then emits only neutral `RepositorySeed`,
  `TargetClaim`, `RepositoryEntry`, mode, and `RepositoryDelta` vocabulary into the
  shared path. Delete profile-local `PackageProjection`, `ProjectedFile`,
  `ProjectedFileMode`, `project_package`, and their exports; delete the profile
  snapshot/capture helpers as the canonical image replaces them. Generic
  projections, default membership, default schemas, and final overlay validation
  are owned by `repository_state`. The risk if profile retains any final-byte map
  or coupled default derivation is recurrence of the verified fresh-init failure.
- **[ASSUMED — recommended design]** Commands remain responsible for choosing the
  intent, appending command-owned provenance/audit changes, adapting the internal
  plan to their response type, and invoking storage. Storage remains responsible
  only for capability-confined publication and recovery. This preserves the
  repository's existing direction of dependencies.
- **[ASSUMED — recommended design]** Treat every mutation of an authoritative
  registry as `SemanticMutation`. The complete gate cutover covers definition
  add/define/update/remove, issue gate add/remove, and preset application. Each
  operation contributes every applicable `gates.toml` byte, issue attachment/state
  byte, audit event, provenance record, and configured gate projection to the same
  `RepositorySeed` and transaction. Gate-run results remain audit records, not
  gate-registry semantic changes. The risk of leaving one raw gate-registry or
  issue/event save path public is a bypass that recreates split publication.

### Required same-change disposition

- **[VERIFIED inventory, ASSUMED disposition]** Delete the profile-local final-byte
  vocabulary `PackageProjection`, `ProjectedFile`, `ProjectedFileMode`, and
  `project_package`, plus their exports and direct tests
  (`crates/jit/src/profile/render.rs:7-128`;
  `crates/jit/src/profile/mod.rs:45-48`). Package parsing and dedicated-tree drift
  must use only the canonical neutral seed, claim, entry, mode, and delta types.
  Delete `RepositorySnapshot`, `SnapshotEntry`, `SnapshotFile`, their exports, and
  the JSON `capture_profile_snapshot` helper rather than adapting them to
  `RepositoryImage` (`crates/jit/src/profile/snapshot.rs:7-124`;
  `crates/jit/src/storage/json.rs:283-300`).
- **[VERIFIED inventory, ASSUMED disposition]** Delete repository-local
  `seed_repo_config`; its pure byte derivation belongs in `repository_state` and
  its publication belongs in `RepositoryStateStore`
  (`crates/jit/src/storage/config_store.rs:57-89`). Split and rename the remaining
  truly user-global config writer so its type/module boundary cannot accept a
  repository-local target; do not retain generic `save_config_document` as a
  repository publication escape hatch.
- **[VERIFIED inventory, ASSUMED disposition]** Remove `IssueStore::init` from the
  production trait and every forwarding implementation
  (`crates/jit/src/storage/mod.rs:92-100`). Repository bootstrap is a
  `RepositoryStateStore` mutation: absent `.jit` uses the bootstrap-only session,
  while an existing root uses bootstrap-to-repository serialization. Tests use the
  canonical in-memory bootstrap path or explicit fixture builders; they do not keep
  `IssueStore::init` as a test convenience. Claim-coordinator initialization is a
  separate Git-backed concern and is not renamed into repository bootstrap.
- **[ASSUMED — recommended design]** Delete former declaration definitions,
  materialization producers, repository writers, final-byte maps, exports, and
  test doubles in the same change. Do not re-export old names from their former
  modules, add aliases, retain wrapper writers, or leave a provider/callback
  registry. A live-tree structural scan is part of the acceptance evidence.

## Question 2 — How should several projections share one target?

### Current evidence

- **[VERIFIED]** Generic projections currently use a `BTreeMap` keyed by target and
  apply projections in config-map order, so distinct regions in one target compose
  deterministically rather than last-writer-wins
  (`crates/jit/src/validation/repository.rs:484-530`;
  `crates/jit/src/validation/repository.rs:1339-1409`).
- **[VERIFIED]** Generic region splicing uses `str::find`; it detects a missing begin,
  missing end, or end-before-begin, but it does not reject duplicate begin/end
  markers or validate overlaps among several regions
  (`crates/jit/src/validation/projection.rs:279-328`).
- **[VERIFIED]** Profile managed regions use a second byte-oriented implementation
  that can append absent markers and rejects duplicate matching markers
  (`crates/jit/src/profile/render.rs:131-181`).
- **[VERIFIED]** `project_package` renders every profile region against the original
  `existing` map and then collects by target into a `BTreeMap`; two profile regions
  targeting one file therefore do not thread their changes through one another and
  a later entry replaces the earlier map entry
  (`crates/jit/src/profile/render.rs:86-128`).
- **[VERIFIED]** Profile/static placement and generic projection placement are not
  identical contracts: a profile region may append markers to a missing or
  unmarked target, while generic `region` mode requires an existing marked target
  (`crates/jit/src/profile/render.rs:131-162`;
  `crates/jit/src/validation/projection.rs:330-365`).
- **[VERIFIED]** Well-formed nesting is an intentional live contract, not malformed
  topology: the profile-owned `jit:dogfood-guidance` region encloses the configured
  `jit:invariants` projection in `AGENTS.md`
  (`profiles/jit-dogfood/assets/live/regions/agents-jit-guidance.md:1-5`;
  `profiles/jit-dogfood/manifest.toml:575-579`; `AGENTS.md:187-211`;
  `dev/active/cdc840ad-investigation.md:48`).

### Options considered

| Option | Evidence and trade-off | Provenance |
|---|---|---|
| Keep sequential string splices and call them in a fixed global order | This preserves the current generic happy path and is deterministic for distinct valid regions, but duplicate markers, crossing intervals, ambiguous nesting, and whole-file-versus-region claims remain implicit. Profile byte content would also need lossy or rejecting UTF-8 conversion. | **[VERIFIED] rejected** |
| Make profile regions use generic projection placement exactly | This removes one function, but it would remove the profile manifest's append placement and change its package installation contract. | **[VERIFIED] rejected** |
| Parse each target into an explicit managed-region containment tree, validate all claims, then apply policy-specific edits | One parser can serve bytes, custom marker pairs, duplicate detection, valid containment, crossing rejection, and ownership checks. Placement remains a caller-supplied policy rather than parser behavior. | **[ASSUMED] recommended; risk is a larger primitive than either current helper** |

### Recommendation

- **[ASSUMED — recommended design]** Put the one pure byte-oriented
  managed-document engine under `repository_state`, with explicit inputs:
  target bytes, region identity, begin/end bytes, replacement bytes, and placement
  policy (`RequireExisting` or `AppendIfAbsent`). The risk is error-mapping churn;
  retain command/profile-specific error wrappers while sharing the parser error
  details.
- **[ASSUMED — recommended design]** For each target, collect all claims before
  rendering. Reject partial pairs, duplicate same-identity pairs, reversed pairs,
  crossing intervals, duplicate region ownership, ambiguous containment, and
  incompatible exclusive whole-file writers before producing final bytes. Permit
  properly nested distinct identities. The risk if identical duplicate claims are
  accepted is hidden co-ownership; reject them until an explicit ownership model
  exists.
- **[ASSUMED — recommended design]** Model each target as a containment tree rather
  than incidental lexical order: apply at most one base/whole-file provider first,
  then apply an outer region before any claimed descendant so the descendant's
  final content is not overwritten by the outer replacement. Reparse or assemble
  structurally after an outer update, and fail if it removes or duplicates a child
  marker claimed by another producer. Apply disjoint siblings in stable producer
  order (or descending source offset after sorting) so their offsets cannot drift.
  The risk is a profile package that implicitly relied on manifest ordering;
  fixtures should expose such a dependency and force it to become an explicit
  producer identity or declared order.
- **[ASSUMED — recommended design]** Keep static/profile versus dynamic/registry
  placement policy as metadata on each tree node, not as a blanket ordering rule.
  Containment imposes the required outer-before-inner edge; producer identity is a
  deterministic tie-breaker for unrelated siblings. The risk of always forcing
  every static edit before every dynamic edit is a contradictory order when a
  dynamic outer region legitimately contains a static inner region.
- **[ASSUMED — recommended design]** Permit a profile asset to establish a target
  that later receives regions, because the dogfood profile already treats package
  assets as projection bases (`crates/jit/src/profile/planner.rs:188-209`). Reject
  two independent whole-file renderers for one target, or a whole-file renderer
  scheduled after region edits, because either makes ownership/order ambiguous.
- **[ASSUMED — recommended design]** Preserve non-managed bytes exactly and keep
  policy outside the parser: profile/static callers choose append semantics;
  registry/dynamic callers choose require-existing semantics. This directly
  implements container decision D-04 without conflating ownership.

## Question 3 — How should persisted derived-state drift be detected?

### Current evidence

- **[VERIFIED]** `validate_repository` already re-renders configured generic
  projections from the supplied view and reports a stale target; it does not trust
  a stored copy as the source of expected content
  (`crates/jit/src/validation/repository.rs:1160-1193`).
- **[VERIFIED]** Effective default rules deliberately derive assertions and
  membership from config in memory; baked default schemas are external-consumer
  projections, not validation authority
  (`crates/jit/src/validation/defaults.rs:3-17`;
  `crates/jit/src/validation/serialize.rs:157-167`;
  `dev/active/af4c901a-derive-default-rules-at-load.md:25-63`).
- **[VERIFIED]** `.jit/rules.toml` is mixed-authority: custom rules and default-rule
  policy fields are authored state, while default assertions and default-family
  membership derive from config
  (`crates/jit/src/validation/defaults.rs:71-83`;
  `dev/active/af4c901a-derive-default-rules-at-load.md:27-43`).
- **[VERIFIED]** Current `validate_with_fix` runs several independent repair loops
  and returns only a count plus text messages; after applying fixes it reruns
  validation (`crates/jit/src/commands/validate.rs:26-85`).

### Options considered

| Option | Evidence and trade-off | Provenance |
|---|---|---|
| Validate only effective behavior | This is the current default-rule behavior and prevents stale files from becoming authoritative, but the verified profile-init repository passes validation while an addressable rule is absent. | **[VERIFIED] rejected** |
| Load persisted derived files as validation authority | This would detect disagreement by changing behavior, but it reverses the chosen SSOT decision and recreates the stale-schema failure documented by `af4c901a`. | **[VERIFIED] rejected** |
| Invoke the full mutation planner recursively from `validate_repository` | It would reuse code, but if the planner validates its final overlay by calling `validate_repository`, the dependency becomes recursive and obscures which stage owns failure. | **[ASSUMED] rejected; risk is recursion or special-case flags** |
| Split derivation, comparison, and final validation | Pure derivation computes expected managed fragments/targets; comparison emits drift; mutation planning overlays the expected delta and then invokes ordinary whole-repository validation. | **[ASSUMED] recommended** |

### Recommendation

- **[ASSUMED — recommended design]** Make pure derivation independent of validation:

  ```text
  derive_materializations(image, seed, intent) -> expected delta
  compare_materializations(image, expected delta) -> structured drift findings
  validate_repository(final overlay) -> semantic/structural validation
  ```

  `validate_repository` may call `repository_state` derivation and comparison
  functions on its supplied image because neither calls validation. For a mutation,
  command orchestration derives and compares within the mutation session, builds
  the final `repository_state` overlay, and then calls `validate_repository`;
  `repository_state` never calls back into validation. On the final overlay,
  recomputation should yield
  no drift. The risk is accidental re-entry if a producer calls validation; enforce
  the acyclic API in module boundaries and tests.
- **[ASSUMED — recommended design]** Recompute expected bytes from declared sources
  on every drift check. Persisted targets may be preservation inputs—for example,
  unmanaged prose around a region or custom/default-policy portions of
  `rules.toml`—but never decide generated membership, assertions, or rendered body.
  The risk if “input for preservation” is confused with authority is masking a
  stale managed fragment; comparison must be scoped to explicit owned fragments or
  generated targets.
- **[ASSUMED — recommended design]** Emit deterministic error-severity findings
  under a stable built-in diagnostic identity such as `derived-state-coherence`,
  with target, producer/region identity, drift kind (`missing`, `stale`, or
  `unexpected-owned`), and repairability. The risk is conflating an invariant ID
  with a declared `.jit/rules.toml` rule; keep the diagnostic documented as a
  built-in validation pass unless a real addressable rule is also declared.
- **[ASSUMED — recommended design]** `jit validate --fix` should request
  `RepairDerived`, publish the complete repair delta in one transaction, and rerun
  whole-repository validation. It must preserve unrelated prose, custom rules,
  editable default-policy fields, comments where the existing pure transform can
  preserve them, and non-owned schema files. The risk of deleting an unexpected
  file by filename convention alone is data loss; only delete a target whose
  ownership is explicit in the generator contract or recorded provenance.
- **[ASSUMED — recommended design]** Add the registry-first
  `derived-state-coherence` invariant as a separate project invariant and project
  it through the existing `invariants` projection. Do not broaden
  `single-source-prose`; that existing invariant explicitly governs prose copies
  (`.jit/invariants.toml`). Whether the new invariant is marked `enforced` must
  match a resolvable rule/gate binding, because enforcement-drift validation rejects
  dangling `enforced-by` references
  (`crates/jit/src/validation/drift.rs:121-202`).

## Question 4 — What belongs outside versus inside the mutation lock?

### Current evidence

- **[VERIFIED]** The repository-wide write lock serializes JIT mutation sequences
  without Git and is intended to cover the read-validate-write-rollback window
  (`crates/jit/src/storage/repo_lock.rs:1-31`).
- **[VERIFIED]** Profile apply currently acquires the repository lock before
  rebuilding all plan inputs and explicitly trusts nothing computed before that
  boundary (`crates/jit/src/commands/profile.rs:122-148`).
- **[VERIFIED]** Profile dry-run planning uses the same planner without publishing,
  and the public apply result reports the plan hash rebuilt under the write lock
  (`crates/jit/src/commands/profile.rs:103-107,350-377`;
  `crates/jit/src/profile/application.rs:55-70`).
- **[VERIFIED]** The kernel's publication-time identity check protects the original
  observed during transaction preparation, but the current action API cannot prove
  that this original matches the preimage used by the command planner
  (`crates/jit/src/storage/file_transaction.rs:428-503`;
  `crates/jit/src/storage/transaction_action.rs:5-31`).
- **[VERIFIED]** The durable transaction kernel is a concrete capability-directory
  service used by JSON-backed init/profile command implementations, while generic
  commands such as project render operate over `IssueStore` and write targets one
  at a time (`crates/jit/src/storage/file_transaction.rs:63-91`;
  `crates/jit/src/commands/init.rs:166-211`;
  `crates/jit/src/commands/profile.rs:68-141`;
  `crates/jit/src/commands/project.rs:74-175`).
- **[VERIFIED]** `InMemoryStorage` has an in-process repository lock, separate
  issue/gate/event maps, and a string-only repository-file map, but no implementation
  of the file-set transaction plan (`crates/jit/src/storage/memory.rs:15-55,91-105,190-203`).
- **[VERIFIED]** Fresh init manually selects a repository-sibling bootstrap guard
  when `.jit` is absent and an inner repository guard when it exists; the transaction
  kernel separately selects external versus internal journal control from root
  existence (`crates/jit/src/commands/init.rs:226-245`;
  `crates/jit/src/storage/file_transaction.rs:84-107,383-396`).
- **[VERIFIED]** `TransactionAction` can create directories, write files, and set
  mode, but cannot delete a file recoverably
  (`crates/jit/src/storage/transaction_action.rs:5-31`).

### Options considered

| Option | Evidence and trade-off | Provenance |
|---|---|---|
| Compute once outside the lock and publish that plan | Minimizes lock time but permits a cooperating JIT writer to change semantic inputs between plan and lock. | **[VERIFIED] rejected by the lock contract** |
| Do all work, including immutable package parsing and unrelated rendering, under the lock | Safest conceptually and matches current profile apply, but unnecessarily lengthens lock hold time as package/profile capabilities grow. | **[ASSUMED] viable but not preferred** |
| Precompute immutable work, then rebuild all repository-dependent derivation and validation under the lock | Preserves correctness while moving only work that cannot observe repository state outside. | **[ASSUMED] recommended** |
| Reuse an optimistic pre-lock plan if a complete read-set digest matches under lock | Can reduce lock time, but a correct read-set for config sources, target preservation bytes, modes, directories, and linked documents is itself a new correctness surface. | **[ASSUMED] defer until profiling proves need** |
| Let each command downcast or branch on JSON versus in-memory storage | This can reach the concrete kernel quickly, but generic command behavior and tests no longer exercise one application contract. Backend-specific branches would spread root/lock/recovery policy through commands. | **[VERIFIED evidence, ASSUMED consequence] rejected** |
| Add `RepositoryStateStore`, implemented by both stores | Commands use the same opaque mutation-session, canonical image, exact delta, and expected-preimage contract. JSON supplies durable journal recovery; in-memory supplies atomic copy-on-write publication under its process lock. | **[ASSUMED] recommended** |

### Recommendation

- **[ASSUMED — recommended design]** Before mutation-session acquisition, parse and
  validate immutable embedded/package inputs, compile static templates, and
  optionally produce a user-facing preview. Do not perform recovery, repository
  reads, network, or Git work in this phase. The risk is accidentally reading
  repository-dependent package collision state; keep the pre-session API incapable
  of accepting a `RepositoryImage`.
- **[ASSUMED — recommended design]** Within the opaque `RepositoryStateStore`
  mutation session, capture/re-read every repository-dependent input, rebuild the
  complete `RepositoryDelta`, construct the exact final overlay including
  provenance/audit records, run whole-repository validation, and apply that delta
  once through the same session. Report the session-built plan hash, not a preview
  hash. The risk is session duration; the affected inputs are local files and pure
  transforms, so correctness should take precedence for v1.0.
- **[ASSUMED — recommended design]** Add the separate `RepositoryStateStore`
  capability beside `IssueStore`, and require it for every affected generic
  command. It exposes one associated opaque mutation-session type and only session
  acquisition, `RepositoryImage` capture through that session, and application of
  one finalized `RepositoryDelta` through that session. It exposes no per-file or
  command-semantic writer. `JsonFileStorage` implements it through
  `FileTransactionKernel`; `InMemoryStorage` implements identical
  preimage/conflict/action semantics with a copy-on-write state image swapped under
  its process-local lock. No implementation forwards to old
  `save_gate_registry`, `write_repo_file`, or config/ruleset writers. The risk is
  enlarging the backend contract, but it makes in-process tests evidence for the
  same semantic transaction rather than a separate write loop.
- **[ASSUMED — recommended design]** Both storage implementations capture the same
  crate-root `RepositoryImage`. The in-memory backend must not keep a gate-registry map and
  a disagreeing `.jit/gates.toml` shadow as independent truths: its canonical
  serializers/accessors and transaction clone must make typed gate reads and
  repository-entry reads observe the same prospective state before the atomic
  swap. Apply the same rule to every semantic record included in a transaction.
  The risk of retaining independent typed/file maps is a passing in-memory
  transaction test that cannot reproduce JSON-backed projection drift.
- **[ASSUMED — recommended design]** Make root-state selection an internal protocol
  of the JSON `RepositoryStateStore` session. Acquire the repository-sibling
  bootstrap guard first and decide root presence while it is held. For absent
  `.jit`, retain only bootstrap, capture an absent image, and publish with
  `ExternalBootstrap`; never create or acquire `.jit/.repo-write.lock`. For an
  existing or partial `.jit`, acquire repository serialization after bootstrap,
  then capture/rebuild and publish with `InternalRepository`. Commands choose
  neither guards, root modes, nor journal locations. The risk of acquiring the
  inner repository lock for an absent root is creating `.jit` before absence
  validation and changing the very preimage being protected.
- **[ASSUMED — recommended design]** Carry each planned target's expected preimage
  (absent/directory/file hash+size+mode, with symlinks/unsupported entries rejected)
  into the storage transaction action and
  require the kernel to compare it during preparation before making a journal
  durable. Keep the kernel's existing second identity check immediately before
  publication. The risk is a storage API change; without it, a manual writer that
  ignores the JIT lock can change a file after planning but before kernel preparation
  and have that newer file become the rollback preimage while still being overwritten.
- **[ASSUMED — recommended design]** If an expected preimage mismatch occurs, fail
  before publication with a typed retryable conflict rather than silently rebuilding
  inside the kernel. Commands may reacquire/replan once at a higher level, but an
  unbounded retry loop could starve under an active external editor.
- **[ASSUMED — recommended design]** Add `DeleteFile` to the exact delta and durable
  journal protocol. It requires an expected regular-file identity, stages/records a
  verified backup, renames the target aside and synchronizes the parent, and treats
  absence as the committed final identity. Prepared recovery restores the verified
  backup; committed recovery verifies absence before cleanup; rollback proceeds in
  reverse action order. Reject absent, directory, symlink, or identity-mismatched
  targets before publication, and omit an already-absent target as a planner no-op.
  The risk of implementing deletion as an unjournaled remove is that a later action
  failure cannot converge to the complete old state.

## Question 5 — Should every command expose one delta/result vocabulary?

### Current evidence

- **[VERIFIED]** Profile dry-run already exposes stable target actions and a plan
  hash, while profile apply exposes profile identity, applied/unchanged status,
  transaction ID, and cleanup warnings
  (`crates/jit/src/profile/application.rs:32-174`).
- **[VERIFIED]** `jit project render` exposes projection names, target, mode, style,
  kinds, and rendered row count (`crates/jit/src/commands/project.rs:26-54`).
- **[VERIFIED]** `config set` exposes key, value, file, and scope
  (`crates/jit/src/commands/config.rs:28-43`), while fresh init returns project
  identity, optional profile result, and warnings
  (`crates/jit/src/commands/init.rs:24-40`).
- **[VERIFIED]** Existing `validate --fix` output is conceptually different again:
  fix count plus repair messages, followed by revalidation
  (`crates/jit/src/commands/validate.rs:34-85`).
- **[VERIFIED]** These result types are already adapted into command-specific human,
  JSON, and schema surfaces; for example profile apply is included in the generated
  command schema (`crates/jit/src/schema.rs:487-494`).

### Options considered

| Option | Evidence and trade-off | Provenance |
|---|---|---|
| Replace every public result with one generic transaction envelope | Maximizes surface uniformity but discards or awkwardly nests command semantics and creates broad pre-v1 contract churn. | **[VERIFIED] rejected** |
| Keep completely separate internal and external target vocabularies | Avoids API changes, but recreates action/hash/mode/conflict logic and makes cross-command transactional tests harder to state. | **[ASSUMED] rejected** |
| Unify the internal delta and optionally reuse a nested public file-change record, while preserving command envelopes | Shares correctness mechanics without erasing command intent or breaking stable human output. | **[ASSUMED] recommended** |

### Recommendation

- **[ASSUMED — recommended design]** Use one internal exact-delta vocabulary across
  init, config, profile, project render, and validation repair. Each target should
  carry path, producer/owner identity, action (`create`, `update`, `delete`,
  `set-mode`, `unchanged`), expected preimage, exact final bytes/mode where
  applicable, and after identity. The aggregate should carry a schema/domain
  version, deterministic plan hash, and sorted targets. The risk is overexposing
  byte-heavy internal state; keep bytes out of serialized public results.
- **[ASSUMED — recommended design]** Preserve command-specific top-level result and
  human-output contracts. Profile still reports profile/version/status; project
  render still reports projection counts; config still reports the selected key;
  init still reports repository identity; validation repair still reports findings
  repaired. The risk is superficially different outputs, but those differences
  represent real user intent rather than competing transaction semantics.
- **[ASSUMED — recommended design]** Where target-level reporting is useful, add or
  reuse a small nested public `RepositoryTargetChange` projection of the internal
  delta (`path`, action, executable/mode intent, producer), and expose it additively
  through generated schemas. Do not expose transaction journals, rollback state, or
  full bytes; those are storage recovery details, not semantic command results.
- **[ASSUMED — recommended design]** Map shared planner/storage errors into stable
  command-independent error codes for malformed managed regions, materialization
  conflicts, stale preimages, and recovery-required outcomes, while preserving
  command context in messages. The risk of command-specific codes for the same
  underlying conflict is inconsistent automation behavior.

## Consolidated recommendation and rejected competing ideas

- **[ASSUMED — recommended design]** The common pipeline should be:

  ```text
  command/profile/gate-registry seed changes
      -> repository_state::RepositoryImage
      -> repository_state::derive_materializations
      -> repository_state managed-document/ownership composition
      -> exact deterministic RepositoryDelta
      -> compare expected versus base/final state
      -> final repository_state overlay validation
      -> one RepositoryStateStore mutation session
      -> recoverable JSON publication or atomic in-memory publication
      -> command-specific result adapter
  ```

  The risk is that provenance/audit bytes computed after validation could escape
  the final view; require those bytes to enter the overlay before validation and
  the same transaction plan afterward.
- **[VERIFIED]** The competing ideas to consolidate are not different product
  semantics; they are separate implementations of the same final-state operation:
  profile exact-target planning, config default refresh, generic projection
  composition, validation projection drift, profile managed-region splicing, and
  the storage transaction kernel. Their present locations and contracts are cited
  in the five question sections above.
- **[ASSUMED — recommended design]** Consolidate the delta, derivation order,
  managed-document engine, drift comparator, session/rebuild protocol, and
  publication adapter.
  Retain distinct producer policies (profile/static versus registry/dynamic) and
  distinct public command results. The risk of consolidating policy and reporting
  as well as mechanics is a single oversized abstraction that obscures ownership.
- **[VERIFIED]** No external dependency is required by the recommended design: the
  repository already has deterministic maps, SHA-256 hashing, byte-exact views,
  TOML editing, capability-confined storage, locking, and transaction recovery in
  its current dependency set and implementation
  (`crates/jit/src/profile/planner.rs:10-16`;
  `crates/jit/src/storage/file_transaction.rs:7-27`).

## Planning risks to carry forward

| Risk | Safest planning response | Provenance |
|---|---|---|
| Mixed-authority `rules.toml` is rewritten as if it were wholly derived | Specify owned fragments: config owns default assertions/membership; the file owns custom rules and editable policy. Require preservation tests. | **[VERIFIED basis]** `dev/active/af4c901a-derive-default-rules-at-load.md:25-63` |
| A materialization producer calls full validation and creates recursion | Keep derivation and comparison lower-level than `validate_repository`; enforce with module API and an overlay test. | **[ASSUMED]** |
| Several writers claim one target | Group claims, build a containment tree, allow well-formed distinct-ID nesting, and reject crossing intervals, ambiguous ownership, or incompatible base writers before producing bytes. | **[ASSUMED]** |
| A manual editor races a locked JIT mutation | Add expected-preimage checks to the transaction action, in addition to existing publication checks. | **[VERIFIED basis]** `crates/jit/src/storage/transaction_action.rs:5-31`; `crates/jit/src/storage/file_transaction.rs:428-503` |
| Repair deletes user-owned files by convention | Delete only explicitly generator-owned targets; otherwise report an unrepairable or review-required finding. | **[ASSUMED]** |
| Validation and profile see different entry kinds or modes | Replace both existing read models with `repository_state::RepositoryImage` and migrate consumers in the same cutover. | **[VERIFIED basis]** `crates/jit/src/validation/repository.rs:119-138`; `crates/jit/src/profile/snapshot.rs:7-40` |
| JSON and in-memory backends implement different transaction semantics | Put expected-preimage, action ordering, conflicts, and outcomes in the `RepositoryStateStore` contract; run the same conformance suite against both implementations. | **[ASSUMED]** |
| An absent-root transaction acquires a guard that creates `.jit` | Keep bootstrap-versus-existing-root selection inside JSON transaction storage and verify root state after the correct outer guard is held. | **[VERIFIED basis]** `crates/jit/src/commands/init.rs:226-245`; `crates/jit/src/storage/repo_lock.rs:80-121` |
| A repair deletion cannot roll back | Implement `DeleteFile` as a journaled rename-to-verified-backup with prepared rollback and committed absence verification; never call an unjournaled remove. | **[ASSUMED]** |
| A gate command bypasses materialization | Route gate definition add/define/update/remove, issue gate add/remove, and preset apply through `SemanticMutation`; remove raw command-level registry/issue/event saves that split their coupled state. | **[VERIFIED basis]** `crates/jit/src/commands/gate.rs:232-352,653-677,966-990,1020-1103` |
| Generic result unification breaks automation | Keep current top-level envelopes and generated schemas; share only internal delta and additive nested change records. | **[VERIFIED basis]** existing result types cited in Question 5 |
| The v1 planner becomes a profile lifecycle engine | Accept profile-produced seed changes but import no profile types; profile removal remains out of scope per container D-06. | **[VERIFIED basis]** `jit issue show cdc840ad` |
