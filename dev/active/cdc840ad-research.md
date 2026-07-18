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

## Formal gate resolutions (F1–F3)

### F1 — bounded discovery and capture

- **[ASSUMED — recommended design]** A `RepositoryStateStore` mutation session
  performs two-phase bounded capture without releasing its guard. Phase one reads
  fixed declaration roots (`index.json`, config, rules, gates, templates,
  invariants, profile provenance, exact `events.jsonl` bytes, and the command/profile typed
  seed) plus exact command-selected issue and gate-run records. Parsing produces a
  sorted `CaptureSpec` of normalized exact paths and explicitly complete listings:
  `.jit/issues`, referenced gate-run/profile/schema directories, configured
  registry/item sources, projection sources and targets outside `.jit`, profile
  asset/region targets and parents, and every proposed target preimage. Phase two
  executes a no-follow work queue to a fixpoint. A newly parsed declaration may
  enqueue only an exact path or complete listing beneath a declared safe path; a
  visited set plus path/count/byte/depth budgets returns a typed closure-limit
  error. There is no recursive repository-root, `target`, `node_modules`, Git
  metadata, or unrelated-tree discovery fallback.
- **[ASSUMED — recommended design]** A complete `.jit/issues` listing enqueues
  every ordinary issue entry required by repository validation. Parsing those
  issues then enqueues every unpinned `Issue.documents[].path`, each document's
  validation-read local assets and parent entry preimages, and every plan-document
  path derived for a planning node from configured template/documentation roots,
  including a derived plan not yet linked in `Issue.documents`. These working-tree
  documents/assets/plan paths receive exact identities and listing revalidation.
  A missing entry is still captured absence; a producer asking for one omitted
  from the spec fails with `UndiscoveredRepositoryPath`. Current document inventory
  and asset rescan must consume this closed image rather than opening their own
  filesystem traversal (`crates/jit/src/domain/artifact_inventory.rs:161-265`;
  `crates/jit/src/commands/document.rs:689-723`).
- **[ASSUMED — recommended design]** Pinned document and validation-read asset
  requests never enqueue or borrow the working-tree entry with the same path. The
  capture boundary resolves them into typed `PinnedDocumentEvidence`. Available
  evidence records requested revision/path, canonical commit OID, object/tree
  identity, blob OID, existence, exact bytes, size, and SHA-256. Unavailable
  evidence records the requested identity plus a stable Git-unavailable,
  not-found, or read-failed diagnostic. Evidence requests may add pinned asset
  requests during the same bounded fixpoint; request/result identity and content
  hash enter `CaptureSpec` and the plan hash. Pure `repository_state` code performs
  no Git I/O and never falls back to working-tree bytes. Git remains optional: an
  unavailable object surfaces the existing typed pinned-read diagnostic only where
  that evidence is required (`crates/jit/src/domain/artifact_inventory.rs:29-67,205-265`;
  `crates/jit/src/storage/git_revision.rs:111-163`).
- **[ASSUMED — recommended design]** Capture every exact entry and complete listing
  with no-follow traversal. `RepositoryImage` records exact file bytes/mode,
  directory identity, symlink payload, unsupported kind, or absence; a complete
  listing additionally records a stable fingerprint over its sorted child
  name/kind/mode identities. This lets validation prove both “the named source is
  unchanged” and “no source appeared or disappeared in a consumed directory.”
  External configured sources and targets are repository-relative capabilities,
  never unconstrained host paths.
- **[ASSUMED — recommended design]** A pure producer requesting a path outside the
  closed `CaptureSpec` fails with `UndiscoveredRepositoryPath`; it cannot perform
  I/O or silently interpret that path as absent. The plan hash covers normalized
  `CaptureSpec`, every entry identity, every complete-listing fingerprint, the
  typed seed, and the exact `RepositoryDelta`. Immediately before durable journal
  preparation, `RepositoryStateStore::apply` revalidates the whole captured read
  set; delta actions recheck target preimages again before publication. Any source,
  target, or listing mismatch is a typed retryable capture conflict. If a
  declaration edit changes the closure, discard the image and repeat both phases
  under the same session until one unchanged bounded fixpoint is planned. Fresh
  init starts from an absent `.jit` declaration image plus exact existing external
  profile/projection targets and parent listings.

### F2 — canonical typed records become canonical bytes once

- **[VERIFIED]** Current issue persistence stamps `updated_at` inside
  `IssueStore::save_issue`, writes the issue, and separately updates the index
  (`crates/jit/src/storage/json.rs:400-430,899-904`). Current event append takes a
  second path that holds repository then event locks and isolates a non-newline
  tail before appending (`crates/jit/src/storage/json.rs:1084-1112`), while profile
  init/apply construct a competing complete event-log image through
  `append_profile_event_image` (`crates/jit/src/profile/application.rs:180-198`;
  `crates/jit/src/commands/init.rs:396`;
  `crates/jit/src/commands/profile.rs:277`). Gate-run results are yet another
  repository-owned direct writer (`crates/jit/src/storage/json.rs:1168-1188`).
- **[ASSUMED — recommended design]** `repository_state` is the single typed-to-byte
  finalizer for repository-owned mutations. A `RepositorySeed` carries neutral
  typed issue upsert/delete intent, registry declarations, event records,
  gate-run/audit records, and profile provenance; commands submit semantic values
  with neither serialized bytes, IDs, nor mutation timestamps. Exactly once after
  canonical session acquisition, create a `MutationContext` containing injected
  `IdAuthority` and `MutationClock`; reuse that unchanged context when capture
  expansion or conflict recovery rebuilds the operation. Production samples its
  UUID/random source once into the context and deterministically derives operation
  identifiers; memory/tests inject a deterministic seed. Allocation order is
  frozen: new issue IDs first in canonical request order so expected-absent issue
  paths enter `CaptureSpec` before closure capture; transaction-visible gate-run,
  profile, and other record IDs next in `(target, key)` order; event IDs last after
  canonical event ordering. Every allocated ID and the context seed identity enter
  the semantic plan hash; neither a retry nor a backend may resample them.
  Machine-local journal IDs remain store-internal. After the bounded non-noop
  closure is complete, `MutationClock` yields one `MutationTimestamp`. A no-op
  allocates no persistent IDs or mutation time. Domain/event constructors perform
  no `Uuid::new_v4` or `Utc::now` calls. The finalizer applies the timestamp to every
  changed issue and committed audit event field whose contract denotes the mutation
  instant. No-ops neither sample a publication timestamp nor bump an issue or emit
  an event; capture expansion and conflict recovery reuse the same context and
  timestamp. Pure
  declaration serializers retain their format authority in
  `declarations`; `repository_state` alone assigns their repository paths, composes
  them with issue/index/provenance/audit changes, and emits final claims/delta.
  Storage does not stamp timestamps or serialize semantic records, and commands do
  not construct complete repository file images. One timestamp value is applied to
  every issue upsert that the semantic operation says changed; verbatim restore is
  an explicit typed policy, not a second writer.
- **[ASSUMED — recommended design]** Lifecycle timestamps obey one rule set. Issue
  creation receives its allocated ID and sets `created_at == updated_at` to the
  operation `MutationTimestamp`; creation initially entering Ready also sets
  `first_ready_at` to that value. A semantic update sets `updated_at` once. The
  first transition into Ready, first claim/assignment, and first transition into
  Done set still-absent `first_ready_at`, `claimed_at`, and `done_at` respectively
  to that same value and never overwrite them on later cycles. Archive/revive
  changes `archived_from` under the same issue timestamp without inventing a second
  lifecycle time. Every changed `GateState.updated_at` uses the operation timestamp;
  every committed audit event uses it; an unchanged gate status does not bump the
  issue or gate state. Storage journal timestamps remain machine-local recovery
  metadata outside semantic plans/deltas.
- **[ASSUMED — recommended design]** Checker execution is typed external evidence,
  not permission for command-local persistence. The checker boundary records its
  start/completion observations and monotonic duration as evidence; the checker
  semantic outcome itself carries no self-generated ID or mutation timestamp.
  `IdAuthority` allocates the gate-run ID, the stored run preserves
  those observed checker times, and the later repository publication uses its one
  `MutationTimestamp` for the issue `updated_at`, `GateState.updated_at`, and audit
  event. External checker execution remains outside `RepositoryStateStore`, but its
  gate-run/issue/event bytes publish only in the resulting delta.
- **[ASSUMED — recommended design]** Lifecycle migration derives missing
  `first_ready_at`, `claimed_at`, and `done_at` only from each earliest matching
  historical event, preserves every existing stamp, and leaves an unrecoverable
  field `None`. The migration's actual issue rewrites use one new
  `MutationTimestamp` for `updated_at`; the single
  `LifecycleTimestampsBackfilled` event uses that timestamp and an allocated event
  ID. All issue rewrites and the event publish in one delta. A rerun with nothing
  to fill allocates no ID/time and emits no event
  (`crates/jit/src/commands/migrate.rs:1-78`).
- **[ASSUMED — recommended design]** Canonical audit composition consumes the exact
  captured `events.jsonl` preimage and typed events. It preserves every prefix byte,
  including a malformed/torn final record, and adds exactly one separator newline
  only when a non-empty prefix lacks one. New events sort by the stable key
  `(mutation phase, event tag, primary identity, secondary identity,
  command-local ordinal)`, serialize once in canonical form, and each receives one
  trailing newline. Exact preimage/final bytes and the listing identity enter the
  same plan hash and delta. Delete `append_profile_event_image`, its
  export/tests/calls, and command-local
  issue/index/registry/provenance/gate-run/event byte producers.
- **[ASSUMED — recommended design]** Torn-tail classification belongs exclusively
  to canonical audit composition. It distinguishes empty/newline-terminated,
  complete-but-unterminated JSON, and malformed unterminated final bytes while
  preserving the prefix exactly and inserting only the required separator.
  `ProfileApplied.isolated_torn_tail` is set by this finalizer—never profile command
  code—and is true exactly when that profile event immediately follows a preserved
  malformed unterminated tail. A complete JSON record missing only its newline is
  separated but reports `false`. Delete `has_malformed_unterminated_event_tail`
  and every command-owned assignment of this field
  (`crates/jit/src/commands/profile.rs:268-281`;
  `crates/jit/src/domain/types.rs:1472-1489`).
- **[ASSUMED — recommended design]** For an existing root, one JSON mutation
  session holds locks in the fixed order bootstrap → repository → events for its
  complete capture/plan/revalidate/publish lifetime; no append or repair path may
  acquire them in reverse. For an absent `.jit`, bootstrap is both repository and
  event-append serialization authority and the session neither creates nor
  acquires an events lock inside the absent root. Gate-run and provenance targets
  publish under the same session and exact-delta preconditions, without an
  independent semantic writer.
- **[ASSUMED — recommended design]** `RepositoryStateStore` is the sole publication
  capability for repository-owned issue/index, gate/rule/config registry,
  `events.jsonl`, gate-run/audit artifact, and profile-provenance mutations, as well
  as their derived targets. Remove the corresponding mutation methods from
  `IssueStore`; it may remain a read/query capability. The only exclusions are
  machine-local coordination/recovery files owned internally by the store,
  Git-backed claim leases under `.git/jit`, explicitly user-global config, and
  unrelated external-process artifacts. Those are different authorities and do
  not publish repository-owned project state.
- **[ASSUMED — recommended design]** Fresh/re-init treats `.gitattributes` as one
  semantic line-set `TargetClaim` in the init `RepositoryDelta`, not a post-init
  storage helper. The claim deterministically ensures the JIT comment/rule set
  (including `.jit/events.jsonl merge=union`) exactly once, preserves every
  unrelated line and byte, and rejects a conflicting claim for the same attribute
  pattern. It requires no Git process or repository detection, so Git-optional init
  has the same repository image. Exact
  `.gitattributes` bytes and its parent listing are captured/revalidated under the
  bootstrap-only or existing-root session. Delete
  `storage::gitattributes::setup_gitattributes` and the `main.rs` after-init writer
  (`crates/jit/src/storage/gitattributes.rs:42-80`;
  `crates/jit/src/main.rs:1938-1947`).
- **[ASSUMED — recommended design]** Graph and snapshot exports classify their
  destination before publication. An output path within the repository root is a
  constrained `RepositoryStateStore` export intent: graph's file or snapshot's complete
  file/tree claims, target ancestors, and preimages join `CaptureSpec` and publish
  once through `RepositoryStateStore`. Stdout is an ephemeral presentation sink and
  receives pure rendered bytes without a repository delta. An explicitly selected
  path outside the repository uses a separate external-export sink with staging and
  no authority to change repository-owned state. Shell redirection remains stdout
  from JIT's perspective. Delete graph's direct atomic path writer and snapshot's
  in-repository rename/tar writers; retain external sink mechanics only for proven
  external destinations (`crates/jit/src/main.rs:4923-4932`;
  `crates/jit/src/commands/snapshot.rs:528-599`).
- **[ASSUMED — recommended design]** Gate preset creation is a typed declaration
  mutation of `.jit/config/gate-presets/<name>.json`: capture the complete preset
  directory and target preimage, serialize through the canonical declarations
  format, publish it with any audit/derived changes through `RepositoryStateStore`,
  and delete `IssueStore::save_gate_preset` plus its JSON/memory implementations
  (`crates/jit/src/commands/gate.rs:1170-1190`;
  `crates/jit/src/storage/json.rs:1335-1356`). Preset apply consumes the same
  declaration and mutation boundary.
- **[ASSUMED — recommended design]** Document asset `--rescan` consumes the
  unpinned document/asset closure, produces an id/time-free issue-document metadata
  intent, and publishes the changed issue plus its typed update event in one delta;
  a failed scan or identical asset set is a warning/no-op with no ID/time. Archive
  execution likewise captures every unpinned source/destination, issue reference,
  asset, and event preimage, then expresses relocations as recoverable create/write/
  delete actions plus issue upserts and one typed archive event in the same delta.
  Pinned historical artifacts remain non-relocatable. Delete rescan's direct
  `save_issue` and archive's stage/relink/compensating repository publishers; a
  legacy partial archive is reconciled by a fresh canonical intent over captured
  state, not by reviving the parallel writer
  (`crates/jit/src/commands/document.rs:689-723`;
  `crates/jit/src/commands/archive.rs:370-611,771-854`).

### F3 — one vertical cutover, then enforcement

- **[ASSUMED — recommended design]** After the canonical declarations/image and
  exact recoverable delta foundations exist, land one vertical cutover containing
  the `repository_state` managed-document engine and fixed producers, the complete
  planner, both `RepositoryStateStore` implementations, every affected command and
  profile consumer, typed-record serialization, and deletion of every superseded
  marker/planner/profile/publisher/`IssueStore` mutation path. No adapter, wrapper,
  alias, dual write, fallback, or command-by-command migration is a permissible
  merge boundary.
- **[ASSUMED — recommended design]** Only derived-state enforcement and evidence
  follow that cutover: the `derived-state-coherence` finding/repair/invariant,
  conformance/failure/concurrency tests, structural absence scans, and stable public
  contract evidence. Later “cleanup” work must not be used to remove a competing
  engine or publisher that the vertical cutover should have deleted.

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
  from `storage` and `validation` into the neutral, pure crate-root `declarations`
  module. Move materialization-only default derivation and serialization, generated
  schema rendering, configured project-projection rendering, and managed-document
  composition into `repository_state`. Validation imports `declarations`
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
- **[VERIFIED inventory, ASSUMED disposition]** Remove repository-owned mutation
  methods from `IssueStore`, including issue/index save/delete/restore, registry
  save, gate-preset save, event append, and gate-run result save, after their consumers submit one
  neutral `RepositorySeed` through `RepositoryStateStore`. Delete
  `append_profile_event_image`, `AppliedProfileRecord::to_bytes`, and every
  command/profile complete-byte producer for events or provenance
  (`crates/jit/src/profile/application.rs:23-28,180-198`). Retain only read/query
  methods on `IssueStore`; machine-local recovery, Git claim leases, and
  user-global config remain explicitly outside this repository-owned mutation
  boundary.
- **[VERIFIED inventory, ASSUMED disposition]** Delete ID/time-producing
  `Issue::new`, `Issue::new_with_labels`, and production `Event::new_*` seed paths,
  plus command-local `Utc::now`/`Uuid::new_v4` construction for repository-owned
  records. Commands submit id/time-free intents through one `MutationContext`;
  deterministic fixture builders replace compatibility constructors in tests.
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
  preserve command/profile error context while sharing the canonical parser error
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
| Split derivation, comparison, and final validation | Pure derivation computes expected managed fragments/targets; comparison emits drift; mutation planning overlays the expected delta and then invokes ordinary semantic/structural validation over the closed captured image. | **[ASSUMED] recommended** |

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
  semantic/structural validation over the closed captured image. It must preserve
  unrelated prose, custom rules, editable default-policy fields, comments where the
  existing pure transform can preserve them, and non-owned schema files. The risk
  of deleting an unexpected file by filename convention alone is data loss; only
  delete a target whose ownership is explicit in the generator contract or recorded
  provenance.
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
  mutation session, execute the fixed-root plus `CaptureSpec` closure, finalize
  typed timestamps/records, rebuild the complete `RepositoryDelta`, construct the
  exact final overlay, validate the closed image, revalidate the read set, and apply
  that delta once. Report the session-built plan hash, not a preview hash. The risk
  is session duration; the affected inputs are bounded local paths and pure
  transforms, so correctness should take precedence for v1.0.
- **[ASSUMED — recommended design]** Add the separate `RepositoryStateStore`
  capability beside read/query-only `IssueStore`, and require it for every command
  that publishes repository-owned issue/index, registry, event, gate-run/audit,
  provenance, or materialized state. It exposes one associated opaque
  mutation-session type and only session acquisition, bounded `RepositoryImage`
  capture through a `CaptureSpec`, and application of one finalized
  `RepositoryDelta` through that session. It exposes no per-file or
  command-semantic writer. `JsonFileStorage` implements it through
  `FileTransactionKernel`; `InMemoryStorage` implements identical
  preimage/conflict/action semantics with a copy-on-write state image swapped under
  its process-local lock. No implementation forwards to old
  `save_issue`, `append_event`, `save_gate_registry`, `save_gate_run_result`,
  `write_repo_file`, or config/ruleset writers. The risk is
  enlarging the backend contract, but it makes in-process tests evidence for the
  same semantic transaction rather than a separate write loop.
- **[ASSUMED — recommended design]** Both storage implementations capture the same
  crate-root `RepositoryImage`. The in-memory backend must not keep issue,
  gate-registry, event, gate-run, or provenance maps and disagreeing repository-byte
  shadows as independent truths: its canonical accessors and transaction clone must
  make typed reads and repository-entry reads observe the same prospective state
  before the atomic swap. Apply the same rule to every semantic record included in
  a transaction.
  The risk of retaining independent typed/file maps is a passing in-memory
  transaction test that cannot reproduce JSON-backed projection drift.
- **[ASSUMED — recommended design]** Make root-state selection an internal protocol
  of the JSON `RepositoryStateStore` session. Acquire the repository-sibling
  bootstrap guard first and decide root presence while it is held. For absent
  `.jit`, retain only bootstrap as repository and event authority, capture an
  absent image, and publish with `ExternalBootstrap`; never create or acquire
  `.jit/.repo-write.lock` or `.jit/.events.lock`. For an
  existing or partial `.jit`, acquire repository serialization after bootstrap,
  then the events lock, and retain bootstrap → repository → events through
  capture/rebuild/revalidation/publication with `InternalRepository`. Commands choose
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
  inside the kernel. Higher-level conflict recovery reuses the same
  `MutationContext`, IDs, and `MutationTimestamp`; neither backend resamples them.
  Bound recovery attempts so an active external editor cannot starve the command.
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
  typed command/profile/issue/registry/audit seed changes
      -> one session MutationContext (IdAuthority + MutationClock)
      -> fixed roots + bounded CaptureSpec document/asset/plan closure
      -> RepositoryImage + listing fingerprints + PinnedDocumentEvidence
      -> frozen issue/record/event ID allocation + one MutationTimestamp
      -> canonical typed-record and exact-prefix audit bytes
      -> repository_state::derive_materializations
      -> repository_state managed-document/ownership composition
      -> exact deterministic RepositoryDelta
      -> compare expected versus base/final state
      -> final repository_state overlay validation
      -> captured read-set revalidation
      -> one RepositoryStateStore mutation session
      -> recoverable JSON publication or atomic in-memory publication
      -> command-specific public result projection
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
  publication boundary.
  Retain distinct producer policies (profile/static versus registry/dynamic) and
  distinct public command results. The risk of consolidating policy and reporting
  as well as mechanics is a single oversized abstraction that obscures ownership.
- **[ASSUMED — recommended design]** Land the managed-document engine, planner,
  `RepositoryStateStore`, typed finalizer, all affected consumers, and all old-path
  deletions as one vertical cutover. Only coherence enforcement/repair and evidence
  may follow; there is no adapter-bearing intermediate architecture.
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
| A producer discovers an uncaptured path or a directory changes after planning | Fail with `UndiscoveredRepositoryPath` or a typed capture conflict; close a bounded no-follow `CaptureSpec`, hash complete listings, and revalidate the entire read set before journal preparation. | **[ASSUMED]** |
| Validation reads an issue-linked document/asset or derived plan outside capture | Enqueue every unpinned issue document, validation-read asset/parent, and configured derived plan path during the bounded fixpoint; pinned requests use only `PinnedDocumentEvidence`. | **[VERIFIED basis]** `crates/jit/src/domain/artifact_inventory.rs:161-265`; `crates/jit/src/commands/validate.rs:746-794` |
| A pinned read silently borrows working-tree bytes or makes Git mandatory | Hash typed commit/tree/blob evidence at the capture boundary and carry a stable unavailable diagnostic; pure derivation performs no Git I/O or fallback. | **[VERIFIED basis]** `crates/jit/src/domain/artifact_inventory.rs:29-67,205-265` |
| A manual editor races a locked JIT mutation | Add expected-preimage checks to the transaction action, in addition to existing publication checks. | **[VERIFIED basis]** `crates/jit/src/storage/transaction_action.rs:5-31`; `crates/jit/src/storage/file_transaction.rs:428-503` |
| Repair deletes user-owned files by convention | Delete only explicitly generator-owned targets; otherwise report an unrepairable or review-required finding. | **[ASSUMED]** |
| Validation and profile see different entry kinds or modes | Replace both existing read models with `repository_state::RepositoryImage` and migrate consumers in the same cutover. | **[VERIFIED basis]** `crates/jit/src/validation/repository.rs:119-138`; `crates/jit/src/profile/snapshot.rs:7-40` |
| JSON and in-memory backends implement different transaction semantics | Put expected-preimage, action ordering, conflicts, and outcomes in the `RepositoryStateStore` contract; run the same conformance suite against both implementations. | **[ASSUMED]** |
| Storage and commands assign different timestamps or event bytes | Sample one `MutationClock` after non-noop capture and let `repository_state` finalize all issue/event/gate-run/provenance bytes; delete storage stamping and command-local image helpers. | **[VERIFIED basis]** `crates/jit/src/storage/json.rs:899-904,1084-1112`; `crates/jit/src/profile/application.rs:180-198` |
| Capture rebuild or backend choice changes generated IDs | Reuse one session `MutationContext`; deterministically derive issue, record, then canonically ordered event IDs from one `IdAuthority` seed and hash every allocation. | **[VERIFIED basis]** current constructors call `Uuid::new_v4` throughout `crates/jit/src/domain/types.rs:1497-1651` |
| Exact event-log replacement races an ordinary append | Make `RepositoryStateStore` the only repository event publisher and hold bootstrap → repository → events for the full existing-root session; absent-root bootstrap is the event authority. | **[VERIFIED basis]** `crates/jit/src/storage/json.rs:261-267,1084-1089` |
| An absent-root transaction acquires a guard that creates `.jit` | Keep bootstrap-versus-existing-root selection inside JSON transaction storage and verify root state after the correct outer guard is held. | **[VERIFIED basis]** `crates/jit/src/commands/init.rs:226-245`; `crates/jit/src/storage/repo_lock.rs:80-121` |
| A repair deletion cannot roll back | Implement `DeleteFile` as a journaled rename-to-verified-backup with prepared rollback and committed absence verification; never call an unjournaled remove. | **[ASSUMED]** |
| A gate command bypasses materialization | Route gate definition add/define/update/remove, issue gate add/remove, and preset apply through `SemanticMutation`; remove raw command-level registry/issue/event saves that split their coupled state. | **[VERIFIED basis]** `crates/jit/src/commands/gate.rs:232-352,653-677,966-990,1020-1103` |
| Init or export retains a quiet raw repository writer | Put `.gitattributes` line-set composition in init's delta and route repository-contained graph/snapshot destinations through the explicit export intent; stdout and proven external outputs remain classified non-repository sinks. | **[VERIFIED basis]** `crates/jit/src/storage/gitattributes.rs:42-80`; `crates/jit/src/main.rs:4923-4932` |
| Preset creation, asset rescan, migration, or archive keeps a less-visible publisher | Include each in the typed-mutation inventory and delete `save_gate_preset`, rescan `save_issue`, per-issue migration saves, and archive staging/relink/event writers in the vertical cutover. | **[VERIFIED basis]** `crates/jit/src/commands/gate.rs:1170-1190`; `crates/jit/src/commands/document.rs:689-723`; `crates/jit/src/commands/migrate.rs:1-78`; `crates/jit/src/commands/archive.rs:370-854` |
| Generic result unification breaks automation | Keep current top-level envelopes and generated schemas; share only internal delta and additive nested change records. | **[VERIFIED basis]** existing result types cited in Question 5 |
| The v1 planner becomes a profile lifecycle engine | Accept profile-produced seed changes but import no profile types; profile removal remains out of scope per container D-06. | **[VERIFIED basis]** `jit issue show cdc840ad` |
