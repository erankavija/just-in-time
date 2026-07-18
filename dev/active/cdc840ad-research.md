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

## Accumulated architecture resolutions

### Bounded discovery and capture

- **[ASSUMED — recommended design]** A `RepositoryStateStore` mutation session
  performs two-phase bounded capture without releasing its guard. Phase one reads
  fixed `Data(...)` declaration roots (`Data(Descendant("index.json"))`, config,
  rules, gates, templates, invariants, profile provenance, exact
  `Data(Descendant("events.jsonl"))` bytes, and the command/profile typed seed)
  plus exact command-selected issue and gate-run records. Parsing produces a sorted
  `CaptureSpec` of validated `VirtualPath` values and explicitly complete
  listings: `Data(Descendant("issues"))`, referenced gate-run/profile/schema
  directories, configured registry/item sources, projection sources and targets
  under either declared root, profile asset/region targets and parents, and every
  proposed target preimage. Phase two executes a no-follow work queue to a fixpoint. A newly
  parsed declaration may enqueue only an exact path or complete listing beneath a
  declared safe path; a visited set plus path/count/byte/depth budgets returns a
  typed closure-limit error. There is no recursive worktree-root or data-root,
  `target`, `node_modules`, Git metadata, or unrelated-tree discovery fallback.
- **[ASSUMED — recommended design]** A complete
  `Data(Descendant("issues"))` listing enqueues
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
  External configured sources and targets are layout-confined capabilities, never
  unconstrained host paths. Capture indexes entries by the layout's canonical
  physical identity as well as `VirtualPath`; physical-boundary input is first
  canonicalized with data-root precedence, while an API requiring an already
  canonical virtual path rejects `DataRootAlias`. Thus two requested spellings can
  never reach the image, listing fingerprint, plan hash, delta, or journal as
  separate identities.
- **[ASSUMED — recommended design]** A pure producer requesting a path outside the
  closed `CaptureSpec` fails with `UndiscoveredRepositoryPath`; it cannot perform
  I/O or silently interpret that path as absent. The plan hash covers normalized
  `RepositoryLayout` identity, `CaptureSpec`, every typed path/entry identity,
  every complete-listing fingerprint, the typed seed, pinned evidence, and the
  exact `RepositoryDelta`. Immediately before durable journal preparation,
  `RepositoryStateStore::apply` revalidates the whole captured read set; delta
  actions recheck target preimages again before publication. Any source, target,
  or listing mismatch is a typed retryable capture conflict. If a declaration edit
  changes the closure, discard the image and repeat both phases under the same
  session until one unchanged bounded fixpoint is planned. Fresh init starts from
  an absent selected data-root image plus exact existing `Worktree(...)` targets
  and parent listings.

### Canonical typed records become canonical bytes once

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
- **[ASSUMED — recommended design]** One JSON mutation session holds locks in the
  fixed order bootstrap → data-root-publication → repository → events for its
  complete recovery/capture/plan/revalidate/publish lifetime; no append or repair
  path may acquire them in reverse. Claim mutations prepend and retain the
  coordinator lock, making their complete order coordinator → bootstrap →
  data-root-publication → repository → events.
  For an absent selected data root, the worktree bootstrap guard is both repository
  and event-append serialization authority and the session neither creates nor
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
- **[ASSUMED — recommended design]** Fresh/re-init acquires typed Git evidence
  before capture and contributes a `Worktree(Descendant(".gitattributes"))` line-set
  `TargetClaim` only when Git identifies a containing worktree and the selected
  data root lies inside that same worktree. The canonical line is the Git-escaped,
  worktree-relative selected-data-root path followed by
  `/events.jsonl merge=union`; the default therefore remains
  `.jit/events.jsonl merge=union`. Eligible composition preserves every unrelated
  line and byte and reports exactly `unchanged`, `created`, or `modified`, with the
  corresponding created/modified path lists. When Git cannot identify such an
  eligible same-worktree root, status is exactly `not_applicable`, no
  `.gitattributes` target is captured, and init succeeds. An eligible target that
  is a symlink, directory, or unsupported kind, malformed content that cannot be
  safely preserved, or a competing semantic claim is a typed preflight error that
  aborts the whole init before journaling. This preserves Git-optional core use
  without pretending an outside-worktree data root has a Git attribute. Delete
  `storage::gitattributes::setup_gitattributes` and the `main.rs` after-init writer
  (`crates/jit/src/storage/gitattributes.rs:42-80`;
  `crates/jit/src/main.rs:1938-1947`).
- **[ASSUMED — recommended design]** Graph and snapshot exports classify their
  destination through the globally validated physical `RepositoryLayout` before
  publication. A destination beneath the selected data root is `Data(...)` even
  when that root is nested inside the worktree; otherwise a destination beneath
  the worktree is `Worktree(...)`; any other destination is external. A virtual
  destination is canonicalized to that exclusive classification, and an
  already-canonical API rejects an alias rather than hashing or publishing it
  twice. Worktree/data destinations become constrained
  `RepositoryStateStore` export intents: graph's file or snapshot's complete
  file/tree claims, target ancestors, and preimages join `CaptureSpec` and publish
  once. Stdout is an ephemeral presentation sink and receives pure rendered bytes
  without a repository delta. An explicitly external path uses a separate staged
  export sink with no authority to change repository-owned state. Shell redirection
  remains stdout from JIT's perspective. Delete graph's direct atomic path writer
  and snapshot's in-repository rename/tar writers; retain external sink mechanics
  only for proven external destinations (`crates/jit/src/main.rs:4923-4932`;
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

### Bounded staging packages, one vertical product cutover

- **[ASSUMED — recommended design]** Implement the broad architecture on one
  dedicated worktree and `integration/cdc840ad` branch as five ordered, cumulative
  work packages: (1) declarations/image/capture/managed documents; (2) layout-aware
  store/kernel/recovery; (3) typed mutation/audit/fenced claims v2; (4)
  materializers/drift/repair; and (5) every consumer migration plus predecessor
  deletion. Each package commits final-form code only to the staging branch, runs
  its targeted tests plus all prior cumulative tests, and exposes only final
  interfaces required by the next package.
- **[ASSUMED — recommended design]** No work-package commit merges to or releases
  from main independently. Main retains the old architecture until a final
  integration package rebases the complete stack, resolves it only in the final
  architecture, runs the full conformance/recovery/security/docs/schema and
  structural-absence gates, and lands one reviewed transition. Staging commits are
  implementation scaffolding in version-control history, not supported product
  states or partial solutions.
- **[ASSUMED — recommended design]** No package may introduce an adapter, feature
  flag, compatibility re-export, dual path/write, fallback, selectable engine,
  temporary public API, or deferred cleanup. Old definitions disappear on the
  staging branch as soon as their final consumers move; the fifth package completes
  the live-tree absence inventory before integration. Independent derived-state and
  release assurance follows only after that clean product landing and uses the sole
  engine rather than removing predecessors.

### Explicit layout, recovered sessions, and claim reconciliation

- **[VERIFIED]** `JsonFileStorage` currently retains only its selected storage root,
  derives bootstrap/repository context from that root's parent, and profile
  transaction planning strips a literal `.jit` prefix before rewriting the path
  for the selected storage directory
  (`crates/jit/src/storage/json.rs:145-205,718-736`;
  `crates/jit/src/commands/profile.rs:457-466`). Those rules conflate logical data
  identity with one physical layout and cannot faithfully represent supported
  relative, strictly nested in-worktree, sibling, or absolute disjoint
  `JIT_DATA_DIR` placement.
- **[ASSUMED — recommended design]** Construct one `RepositoryLayout` from the
  no-follow, lexically normalized worktree root and selected data root and pass it
  to every storage/session/kernel/export boundary. A data root strictly nested
  beneath the worktree or physically disjoint from it is allowed. Equal roots or a
  data root containing the worktree fail as typed
  `OverlappingRepositoryRoots`; symlinked root components, unresolved lexical
  escape, and root identity changes during capability acquisition are rejected
  rather than silently canonicalized through. These checks precede recovery and
  capture and apply when the selected data root is absent as well as present.
- **[ASSUMED — recommended design]** The layout's only semantic path type is
  `VirtualPath`, with `Worktree(RootRelativePath)` and
  `Data(RootRelativePath)` variants; the relative component explicitly represents
  either the selected root itself or a normalized non-empty descendant and rejects
  parent, prefix, control, and alternate separators. Thus the logical
  `.jit/index.json` and `.jit/events.jsonl` contracts are always data descendants
  `index.json` and `events.jsonl`, while authored documents and `.gitattributes`
  are worktree descendants, regardless of the physical data root. When the data
  root is strictly inside the worktree, physical classification always gives the
  more-specific data root precedence. The sole physical conversion,
  `RepositoryLayout::classify_and_canonicalize`, maps a submitted worktree spelling
  beneath that subtree to its exclusive `Data(...)` identity before insertion;
  APIs that require an already canonical virtual path reject the same input as
  `DataRootAlias`. Every discovery queue, `CaptureSpec`, claim/target set, delta
  duplicate check, plan hash, journal action, export/attributes classifier,
  validation lookup, and recovery operation uses the canonical value and verifies
  virtual-to-physical injectivity. No two virtual identities for one physical
  target survive. Delete
  storage-root-parent discovery, physical `.jit` translation, prefix stripping,
  `transaction_path`, and every string/absolute-path adapter between semantic
  deltas and the kernel.
- **[ASSUMED — recommended design]** Parameterize `FileTransactionKernel` with
  root-confined capabilities for both layout roots and explicit transaction control
  roots. Before the selected data root exists, external journals live under the
  worktree control root `.jit-bootstrap/transactions`; afterward internal journals
  live at `Data(Descendant("tmp/transactions"))`. Every journal action stores only
  its root class and normalized relative path. Recovery binds the journal to the complete
  `RepositoryLayout` identity, resolves each action through that layout, and rejects
  a mismatch; it never persists an absolute physical target or a synthetic
  `.jit`-prefixed surrogate.
- **[ASSUMED — recommended design]** An absent disjoint data root is published
  through an existing-parent capability, never by incrementally creating the final
  directory. Layout acquisition opens that parent no-follow, rejects symlink or
  unsupported ancestry, and derives a permanent parent-sibling
  `DataRootPublicationLock` key solely from canonical parent identity plus final leaf
  name. Every repository targeting that physical root therefore takes the same
  advisory lock before testing root existence and holds it through external
  recovery, capture/planning, publication, rollback, and committed cleanup.
- **[ASSUMED — recommended design]** Fresh publication creates and verifies a
  complete sibling staging directory beneath that parent capability. All
  `Data(...)` actions resolve against the staging capability while the durable
  journal remains a canonical worktree action beneath
  `Worktree(Descendant(".jit-bootstrap/transactions/..."))`. The kernel verifies
  and synchronizes every staged byte, mode, and directory, atomically publishes the
  complete root with a no-replace rename, fsyncs the parent, opens the final root
  no-follow, and verifies its recorded identity. A competing final occupant is a
  typed root-publication conflict. Rollback and prepared/committed recovery may
  rename or remove a stage or published root only when recorded parent, leaf, and
  object identities still match; ambiguous identity preserves evidence and requires
  recovery. Other processes can therefore observe only absence or the complete
  root, never a partially populated data directory. Existing roots continue to use
  internal `Data(Descendant("tmp/transactions/..."))` journals.
- **[VERIFIED]** Startup recovery already orders external before internal journals,
  but it is a CLI/service boundary and infers repository context from the data-root
  parent (`crates/jit/src/storage/recovery_coordinator.rs:1-120`). Direct
  `CommandExecutor`, HTTP, test, and post-checker callers therefore cannot treat it
  as the mutation correctness boundary.
- **[ASSUMED — recommended design]**
  `RepositoryStateStore::open_mutation_session(layout)` is the mandatory recovered
  boundary for every affected caller. It acquires the worktree bootstrap guard and
  then the layout's shared data-root-publication guard before any root-existence
  check, recovers all external journals, and verifies/cleans committed residue
  before re-evaluating selected data-root existence. When the data root exists, it
  then acquires data-root repository and event guards in canonical
  bootstrap → data-root-publication → repository → events order, recovers internal journals, and
  verifies/cleans their committed residue. Capture cannot begin until both stages
  succeed. An ordinary command may consume/reuse a matching retained CLI
  `RecoverySession`; a claim command must acquire its coordinator guard before any
  repository guard and therefore cannot inherit a pre-held repository session.
  Startup can schedule claim reconciliation, but the claim path opens/re-enters its
  recovered session only beneath the coordinator guard. Direct callers enter the
  same boundary. External checker subprocesses run after retained repository guards
  are released and their result publication reacquires the recovered session. Any
  unresolved recovery or residue failure is a typed error that prevents capture
  and planning.
- **[VERIFIED]** Claim acquisition currently commits a Git-backed lease under
  `.git/jit`, then independently calls issue save and event append with a newly
  sampled timestamp (`crates/jit/src/commands/claim.rs:100-156`). There is no
  durable record connecting the two control planes, so a crash can leave the lease
  and repository assignment/event disagreeing.
- **[ASSUMED — recommended design]** Claims remain explicitly Git-required and use
  one global lock order: claim coordinator → worktree bootstrap →
  data-root-publication → data-root repository → events. Acquire takes and retains the coordinator lock, creates a
  durable `pending_repository_sync` lease with desired transition `acquire`, stable
  coordination ID, monotonically increasing `attempt_generation`, and a fresh
  `attempt_owner`, then opens the recovered repository session while still holding
  the coordinator guard. The idempotent issue/event transition carries only its
  public `lease_key`, coordination ID, generation, `acquire|release` operation, and
  migration marker; `attempt_owner` remains non-public internal coordinator state.
  After the repository transaction converges and its guards are
  released, conditional finalization changes the pending lease to `active` only if
  desired transition, coordination ID, generation, and internal owner are
  unchanged; the coordinator lock is released last. A definite
  pre-journal failure conditionally deletes the same pending attempt under the
  retained guard. An uncertain/recovery-required publication or conditional-
  finalization failure retains pending state and returns typed
  `reconciliation_required`, never a false atomic-success result. A crash releases
  the OS locks but leaves that durable pending attempt for fenced reconciliation.
- **[ASSUMED — recommended design]** Startup/claim recovery and every subsequent
  claim acquire/release/status operation reconcile under the identical coordinator
  → repository-session lock order. Repository journal recovery runs before event
  evidence is interpreted. An event matching lease key, coordination ID,
  generation, expected `acquire|release` operation, and migration marker, together
  with an unchanged internal `attempt_owner`, conditionally finalizes that exact pending attempt; proven absence of its
  transaction compensates it. Retry recognizes the same non-secret event identity
  and emits no duplicate. After the configured grace, a
  stale or abandoned pending attempt may be taken over only while holding the
  coordinator lock and after recovered evidence has not established its commit:
  takeover increments `attempt_generation` and installs a new `attempt_owner`
  before any repository work. Every delayed prior event/finalizer then fails with
  `FencedClaimAttempt`; issue/event finalization and heartbeat, renew, release, and
  retry paths likewise reject a generation/owner that has lost the fence. Release
  mirrors the same pending record, event identity, ordering, conditional finalization,
  and fenced-takeover rules before durable lease removal or compensation back to
  active. Pending records retain the requested lease TTL, use the runtime-defaults
  SSOT for the default and the configured indefinite-lease stale grace when TTL is
  zero, and expose state, desired transition, coordination ID, generation,
  redacted lease key, created/expiry/last-attempt times, stale flag, and
  reconciliation warning in status/list output. Lock-order APIs and tests reject
  any path that acquires the coordinator after
  bootstrap/data-root-publication/repository/events. Non-Git claim calls retain
  `ClaimRequiresGitError`; no saga makes Git mandatory for unrelated core commands.

### Claim v2 migration and credential closure

- **[VERIFIED]** Current coordination persists a raw string `lease_id` in `Lease`,
  embeds the complete lease in `ClaimOp::Acquire`, stores active leases in a
  schema-versioned `ClaimsIndex`, and rebuilds that index by replaying
  `.git/jit/claims.jsonl` (`crates/jit/src/storage/claim_coordinator.rs:31-181,943-1008`).
  The current `IssueClaimed` event carries only issue, timestamp, and assignee, so
  it cannot prove which lease attempt produced repository state
  (`crates/jit/src/domain/types.rs:1200-1215`).
- **[VERIFIED]** The current CLI's ordinary release resolves an issue's active
  lease and force-evicts it without any owner credential; its own documentation
  calls this an owner bypass (`crates/jit/src/cli.rs:2634-2668`;
  `crates/jit/src/commands/claim.rs:205-360`). Current force-evict also accepts the
  raw lease ID rather than a public lookup key
  (`crates/jit/src/cli.rs:2743-2765`;
  `crates/jit/src/commands/claim.rs:600-642`). Both surfaces must change rather
  than be retained as compatibility aliases.
- **[ASSUMED — recommended design]** Make the live coordinator format
  `record_version: 2`, with `LeaseV2`, `ClaimOpV2`, and v2-only current index/read
  APIs. `LeaseState` is exactly `pending_repository_sync`, `active`,
  `legacy_unverified`, `released`, `expired`, or `reconciliation_required`; the v2
  log operation vocabulary is exactly `migrated_legacy`, `pending`, `activated`,
  `released`, `expired`, or `reconciliation_required`. One isolated migration
  boundary exactly decodes v1 index/lease records.
  The append-only claims log permanently retains a decode-only v1-history decoder
  so its audit sequence remains readable, but no ordinary reader/writer accepts a
  mixed or serde-defaulted v1 shape. New writes are v2 only. Index rewrite and the
  corresponding log append are recoverable, idempotent coordinator publications
  under its retained lock; migration is one-way and preserves sequence/history.
- **[ASSUMED — recommended design]** An active v1 lease migrates to
  `legacy_unverified` at generation zero without fabricating repository evidence;
  an already expired v1 lease migrates terminal. On its first heartbeat, renew,
  release, or reconciliation, coordinator-first recovered capture must match both
  the legacy assignee and historical `IssueClaimed` evidence. A match may append
  exactly one idempotent v2 `issue_claim_lease_changed` event with
  `operation: acquire` and `migration: v1_to_v2`; a
  mismatch fails `LegacyClaimReconciliationConflict`. Release may close a matching
  legacy lease directly. Heartbeat or renew upgrades it to fully fenced v2 and
  returns a rotated handle; the consumed v1 alias can never be reused. There is no
  v1 writer, dual-format current index, default-filled compatibility record, or
  invented event.
- **[ASSUMED — recommended design]** Freeze new claim audit writes on event tag
  `issue_claim_lease_changed` with exactly `{id, timestamp, issue_id, assignee,
  operation, lease_key, coordination_id, generation, migration}`. `operation` is
  exactly `acquire|release`; `migration` is exactly `null|v1_to_v2`. The event has
  no `worktree_id`, `attempt_generation`, `change`, or `lease_id`, and carries no
  bearer secret, salt/hash, or internal `attempt_owner`. Historical `IssueClaimed`
  remains decode/fold-only. Lifecycle reconstruction uses the earliest historical
  `IssueClaimed` or v2 `operation: acquire` as `claimed_at`; `operation: release`
  updates current claim/assignee state without rewriting first-claim time. Renew
  remains a coordinator-log operation and emits no alternate repository event.
- **[ASSUMED — recommended design]** Public `lease_id` remains the single opaque
  CLI/JSON credential name, but a v2 bearer handle is exactly
  `lk_<public-key>.<secret>`: a 128-bit lowercase-hex lookup key and an unpadded
  base64url 256-bit secret matching
  `^lk_[0-9a-f]{32}\.[A-Za-z0-9_-]{43}$`. The v2 index stores only the lookup key,
  a fresh 128-bit salt, and a constant-time-checked
  `SHA-256("jit-claim-v2" || salt || secret)`; plaintext never enters the index,
  claim log, repository event, or journal. A stale or consumed handle fails
  `FencedClaimAttempt` even for the same agent/worktree, and internal
  `attempt_owner` is never a public credential. A v1 ID is accepted only at the
  isolated migration boundary described below, after which its alias is consumed.
- **[ASSUMED — recommended design]** The command/credential/event matrix is fixed:

  | Operation | Authoritative credential and rotation | One-time secret output | Frozen repository event |
  |---|---|---|---|
  | `acquire(issue_id)` | No input credential. Fresh acquire creates generation/key/secret. Grace takeover is internal to this path, increments generation, and creates a new key/secret; there is no takeover command. | Full replacement `lease_id` once through the acquire response. | `operation: acquire`, `migration: null` (including takeover). |
  | `renew(full lease_id)` | Owner bearer required; successful renew increments generation and rotates key/secret. | Full replacement `lease_id` once. The prior handle is fenced. | None; rotation is coordinator state, not another repository event taxonomy. |
  | `heartbeat(full lease_id)` | Owner bearer required. Normal v2 heartbeat does not rotate. A v1 legacy heartbeat at the isolated migration boundary upgrades the lease and rotates generation/key/secret. | None normally; the legacy-upgrade response returns its replacement once. | None normally; legacy upgrade emits `operation: acquire`, `migration: v1_to_v2`. |
  | `release(full lease_id)` | Owner bearer required; no rotation. The current release-by-issue owner bypass and handler are deleted with no alias. | None. | `operation: release`, `migration: null`. |
  | `force-evict(lease_key, reason)` | Admin operation accepts only the public 128-bit lowercase-hex `lease_key`; never a bearer. No rotation. | None. | `operation: release`, `migration: null`. |
  | `status` / `list` | No bearer input or output; projected handles are redacted. | None. | None. |
  | recovery / reconciliation | Internal fencing evidence only; never accepts, reconstructs, or returns a bearer secret. | None. | Only the idempotent acquire/release event required to converge an already recorded operation. |

- **[ASSUMED — recommended design]** Reveal a complete bearer only in the
  successful acquire, renew, or legacy-upgrade response cells above. Status/list/
  index project `lease_id` as `lk_<public-key>.REDACTED` with non-secret
  coordination/generation/state data and can never be used as credentials. Errors,
  warnings, `Debug`/`Display`, tracing,
  events, journals, findings, crash reports, and MCP never echo a secret or raw
  input; they use the redacted key or a generic invalid-credential message.
  CLI/schema/MCP mark inputs and one-time outputs sensitive/write-only where
  applicable and distinguish full-handle from redacted-output patterns. Secret
  wrappers implement neither general serialization nor display; only the
  acquire/rotation response adapter may consume them.
- **[ASSUMED — recommended design]** Generated CLI/JSON/schema/MCP surfaces and
  tests freeze the matrix rather than exposing a generic credential shape:
  acquire accepts `issue_id` and returns one sensitive full handle; renew accepts a
  full handle and returns one sensitive replacement; heartbeat accepts a full
  handle and returns a replacement only for the typed legacy-upgrade result;
  release accepts a full handle and has no secret result; force-evict accepts the
  public `lease_key` plus reason; status/list use only the redacted-output pattern;
  recovery/reconciliation have no secret fields. Structural tests prove the
  release-by-issue argument, handler, schema, MCP route, and aliases are absent.

## Latest plan-review findings resolution (F1–F4)

| Finding | Resolved contract |
|---|---|
| F1 — absent disjoint-root publication | Open the existing parent capability; take the canonical parent/leaf `DataRootPublicationLock` before existence checks; stage, verify, and fsync the complete root; publish atomically no-replace; fsync/reopen/verify the final root; and rollback/recover only matching recorded identities. |
| F2 — claim-state compatibility | Current state is v2-only `LeaseV2`/`ClaimOpV2`/index. One exact one-way boundary migrates active v1 records to generation-zero `legacy_unverified`, preserves v1 log/event history as decode/fold-only, and requires coordinator-first repository reconciliation before upgrade or release. |
| F3 — fencing credential propagation | `lease_id` is the exact opaque bearer handle; only key/salt/domain-separated hash persist, frozen operations rotate it, status and diagnostics redact it, and current `issue_claim_lease_changed` carries only the frozen acquire/release, lease-key, coordination, generation, and migration fields. |
| F4 — actionable cutover | Five bounded cumulative final-form packages live only on `integration/cdc840ad`; each runs targeted plus prior tests and adds no adapter/flag/temporary API. Only the fully rebased, clean, gated stack lands on main, after consumer migration and predecessor deletion are complete. |

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
  subsystem now. It owns `RepositoryLayout`, `VirtualPath`, `RepositoryEntry`,
  `RepositoryImage`, `RepositorySeed`, `MaterializationIntent`, `TargetClaim`,
  `RepositoryDelta`, the canonical overlay, managed-document topology,
  materialization derivation, and drift comparison. Validation owns semantic/rule
  evaluation over the resulting image; commands own use-case orchestration;
  profile owns package parsing; storage owns transactional publication. The risk
  is a broader cutover, but retaining a transitional owner would preserve competing
  abstractions inside the v1.0 correctness boundary.
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
  cumulative package that moves their final consumers to crate-root types. Do not
  retain a second path under the old modules. The risk is test migration volume;
  retaining both would leave two safety and identity contracts to drift.
- **[ASSUMED — recommended design]** Make producer ownership acyclic and explicit.
  Move the declarative `GateRegistry`/`GateDefinition` and `RuleSet`/`Rule` models
  from `storage` and `validation` into the neutral, pure crate-root `declarations`
  module. Move materialization-only default derivation and serialization, generated
  schema rendering, configured project-projection rendering, and managed-document
  composition into `repository_state`. Validation imports `declarations`
  plus `repository_state` for derive/compare, and storage imports the declarations
  for persistence. `repository_state` never imports `validation`, `storage`, or
  `profile`; validation-specific rule evaluation remains in `validation`. Delete
  the former definitions and exports in the same cumulative package, with no
  re-exports or compatibility aliases. This preserves `derive -> compare -> validate`; the risk
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
  `TargetClaim`, `VirtualPath`, `RepositoryEntry`, mode, and `RepositoryDelta`
  vocabulary into the shared path. Delete profile-local `PackageProjection`, `ProjectedFile`,
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

### Required cumulative-cutover disposition

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
  `RepositoryStateStore` mutation: an absent disjoint selected data root uses
  bootstrap plus the shared parent-sibling publication guard and stages a complete
  root, while an existing selected data root uses publication-to-repository/event
  serialization. Tests use the canonical in-memory bootstrap path or explicit
  fixture builders; they do not keep
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
- **[VERIFIED inventory, ASSUMED disposition]** Delete
  `commands::profile::transaction_path`, storage-root-parent repository discovery,
  every physical `.jit` prefix translator, and any kernel/journal API that accepts
  an untyped repository-relative or absolute semantic target. Replace them directly
  with `RepositoryLayout` plus `VirtualPath`; do not leave a compatibility adapter.
  Delete standalone `.gitattributes` warning/write code after its eligible line-set
  claim and exact status/error contract are part of init's main delta. Replace
  direct claim `save_issue`/`append_event` synchronization with the durable
  `pending_repository_sync` saga; lease storage remains the only separate
  Git-required control-plane publisher.
- **[ASSUMED — recommended design]** Delete former declaration definitions,
  materialization producers, repository writers, final-byte maps, exports, and test
  doubles in the cumulative staging package that moves their final consumers. Do
  not re-export old names from their former modules, add aliases, retain wrapper
  writers, or leave a provider/callback registry. The predecessor inventory is
  already empty before final integration, and each later package reruns the
  cumulative live-tree structural scan.

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
  when the selected data root is absent and an inner repository guard when it
  exists; the transaction kernel separately selects external versus internal
  journal control from root existence (`crates/jit/src/commands/init.rs:226-245`;
  `crates/jit/src/storage/file_transaction.rs:84-107,383-396`). Current storage
  nevertheless derives repository context from the storage root's parent, and
  profile translates semantic targets by stripping a literal `.jit` prefix
  (`crates/jit/src/storage/json.rs:145-205,718-736`;
  `crates/jit/src/commands/profile.rs:457-466`).
- **[VERIFIED]** Recovery is currently coordinated at CLI/service startup rather
  than at the generic mutation capability, so a direct `CommandExecutor` caller is
  not independently guaranteed external-then-internal recovery before its first
  capture (`crates/jit/src/storage/recovery_coordinator.rs:1-120`).
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
- **[ASSUMED — recommended design]** Pass one explicit `RepositoryLayout` to
  `open_mutation_session`, capture, revalidation, the transaction kernel, and
  repository export classification. Construct it only after no-follow lexical and
  capability-identity validation accepts either a data root strictly inside the
  worktree or a disjoint data root; reject equal, data-ancestor, symlinked,
  escaping, or identity-changing roots.
  The session resolves only typed `VirtualPath::{Worktree, Data}` values through
  root-confined capabilities. `classify_and_canonicalize` gives data-root
  precedence for nested physical destinations; canonical-only APIs reject
  `DataRootAlias`, and every set proves one virtual identity per physical target.
  Journal entries retain the canonical variant plus normalized relative path and
  bind to the full layout identity. The kernel receives explicit external and
  internal transaction control roots. No session or kernel discovers a worktree by
  taking the selected data root's parent, interprets a `.jit` prefix, adapts an
  absolute semantic path, or accepts a virtual alias.
- **[ASSUMED — recommended design]** Both storage implementations capture the same
  crate-root `RepositoryImage`. The in-memory backend must not keep issue,
  gate-registry, event, gate-run, or provenance maps and disagreeing repository-byte
  shadows as independent truths: its canonical accessors and transaction clone must
  make typed reads and repository-entry reads observe the same prospective state
  before the atomic swap. Apply the same rule to every semantic record included in
  a transaction.
  The risk of retaining independent typed/file maps is a passing in-memory
  transaction test that cannot reproduce JSON-backed projection drift.
- **[ASSUMED — recommended design]** Make recovery and root-state selection the
  opening protocol of the JSON `RepositoryStateStore` session. Acquire the
  worktree-root bootstrap guard and the layout's shared parent-sibling
  `DataRootPublicationLock`, recover and verify/clean every external journal, then
  decide selected data-root presence. For an absent disjoint data root, retain both
  guards, capture an absent `Data(...)` image, stage the complete root through the
  existing-parent capability, and publish it atomically no-replace; never create an
  inner lock as a side effect of checking absence. For an existing root, acquire
  repository serialization and then events, recover and verify/clean every internal
  journal, and retain bootstrap → data-root-publication → repository → events through
  capture/rebuild/revalidation/publication with `InternalRepository`. Recovery or
  committed-residue verification failure prevents capture. A matching retained
  CLI recovery session may be consumed reentrantly, but is never the only
  correctness path; direct callers and post-checker publication use the same open
  protocol. Claim/release/reconciliation acquires the coordinator first and opens
  this session beneath it, never by inheriting a pre-held later guard.
  Commands choose neither root modes nor journal locations, and the claim
  orchestrator chooses only its required outer coordinator guard.
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
      -> globally validated RepositoryLayout + canonical physical identities
      -> claim only: retain coordinator guard; persist fenced pending attempt
      -> open_mutation_session: bootstrap + publication guard; external recovery
      -> existing root: repository/events guards + internal recovery
      -> one session MutationContext (IdAuthority + MutationClock)
      -> typed Worktree/Data roots + bounded CaptureSpec closure
      -> RepositoryImage + listing fingerprints + PinnedDocumentEvidence
      -> frozen issue/record/event ID allocation + one MutationTimestamp
      -> canonical typed-record and exact-prefix audit bytes
      -> repository_state::derive_materializations
      -> repository_state managed-document/ownership composition
      -> exact deterministic RepositoryDelta
      -> compare expected versus base/final state
      -> final repository_state overlay validation
      -> captured read-set revalidation
      -> recoverable JSON publication (staged complete absent root) or memory swap
      -> claim only: conditional attempt finalization/reconciliation; release guard
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
- **[ASSUMED — recommended design]** Build the managed-document engine, planner,
  `RepositoryStateStore`, typed finalizer, all affected consumers, and all old-path
  deletions as the five bounded cumulative packages on `integration/cdc840ad`, then
  rebase, gate, and land the complete stack as the sole vertical transition on
  main. Only independent coherence/release assurance follows. No staging package
  is shipped, and no adapter-bearing or selectable intermediate architecture exists
  on either branch.
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
| Validation and profile see different entry kinds or modes | Replace both read models with `repository_state::RepositoryImage` in the first cumulative package and keep the integration branch on final interfaces thereafter. | **[VERIFIED basis]** `crates/jit/src/validation/repository.rs:119-138`; `crates/jit/src/profile/snapshot.rs:7-40` |
| JSON and in-memory backends implement different transaction semantics | Put expected-preimage, action ordering, conflicts, and outcomes in the `RepositoryStateStore` contract; run the same conformance suite against both implementations. | **[ASSUMED]** |
| A custom `JIT_DATA_DIR` is normalized back to `.jit`, aliases a worktree spelling, or escapes through physical topology | Accept only strict data-within-worktree nesting or disjoint roots; reject `OverlappingRepositoryRoots`, symlink/escape, and identity changes; canonicalize physical input with Data precedence, reject `DataRootAlias` at canonical APIs, and prove injectivity through capture, hashing, exports, journals, recovery, and publication. | **[VERIFIED basis]** `crates/jit/src/storage/json.rs:145-205,718-736`; `crates/jit/src/commands/profile.rs:457-466` |
| A direct caller captures prepared or committed residue before recovery | Make external-then-internal recovery and residue verification mandatory inside `open_mutation_session`; retain startup recovery only as a reusable session/reporting facility. | **[VERIFIED basis]** `crates/jit/src/storage/recovery_coordinator.rs:1-120` |
| Two repositories race to initialize the same absent disjoint data root, or rollback deletes a competing occupant | Open the existing parent capability, share `DataRootPublicationLock` by canonical parent identity/leaf, stage and fsync the complete sibling root, publish atomically no-replace, fsync/reopen/verify, and mutate stage/final paths during recovery only when recorded identities match. | **[ASSUMED]** |
| Storage and commands assign different timestamps or event bytes | Sample one `MutationClock` after non-noop capture and let `repository_state` finalize all issue/event/gate-run/provenance bytes; delete storage stamping and command-local image helpers. | **[VERIFIED basis]** `crates/jit/src/storage/json.rs:899-904,1084-1112`; `crates/jit/src/profile/application.rs:180-198` |
| Capture rebuild or backend choice changes generated IDs | Reuse one session `MutationContext`; deterministically derive issue, record, then canonically ordered event IDs from one `IdAuthority` seed and hash every allocation. | **[VERIFIED basis]** current constructors call `Uuid::new_v4` throughout `crates/jit/src/domain/types.rs:1497-1651` |
| Exact event-log replacement races an ordinary append | Make `RepositoryStateStore` the sole event publisher and hold bootstrap → data-root-publication → repository → events for an existing root; absent-root bootstrap plus shared publication lock owns the staged complete event image. | **[VERIFIED basis]** `crates/jit/src/storage/json.rs:261-267,1084-1089` |
| An absent-root transaction acquires a guard that incrementally exposes the selected data root | Take bootstrap then the shared sibling publication lock before checking existence; direct every `Data(...)` action into a complete staged root and publish only with synchronized no-replace directory rename and identity verification. | **[VERIFIED basis]** `crates/jit/src/commands/init.rs:226-245`; `crates/jit/src/storage/repo_lock.rs:80-121` |
| A repair deletion cannot roll back | Implement `DeleteFile` as a journaled rename-to-verified-backup with prepared rollback and committed absence verification; never call an unjournaled remove. | **[ASSUMED]** |
| A gate command bypasses materialization | Route gate definition add/define/update/remove, issue gate add/remove, and preset apply through `SemanticMutation`; remove raw command-level registry/issue/event saves that split their coupled state. | **[VERIFIED basis]** `crates/jit/src/commands/gate.rs:232-352,653-677,966-990,1020-1103` |
| Init or export retains a quiet raw repository writer | For an eligible same-worktree data root, put `.gitattributes` line-set composition in init's delta with exact status/error semantics; otherwise report `not_applicable`. Route repository-contained graph/snapshot destinations through the explicit export intent; stdout and proven external outputs remain non-repository sinks. | **[VERIFIED basis]** `crates/jit/src/storage/gitattributes.rs:42-80`; `crates/jit/src/main.rs:4923-4932` |
| Claim lease state and repository assignment/event diverge, or a stale worker finalizes a replacement attempt | Hold coordinator across coordinator → bootstrap → data-root-publication → repository → events; keep internal owner out of the event, emit only the frozen acquire/release + lease-key/coordination/generation/migration wire, finalize conditionally, and fence takeover. | **[VERIFIED basis]** `crates/jit/src/commands/claim.rs:100-156` |
| Existing v1 claims are silently treated as proven v2 leases or audit history is rewritten | Use one exact v1 decoder and one-way v2 index migration; retain v1 log/event decode-only history, mark active records `legacy_unverified`, reconcile under coordinator-first recovery, and fail mismatches with `LegacyClaimReconciliationConflict`. | **[VERIFIED basis]** `crates/jit/src/storage/claim_coordinator.rs:31-181,943-1008`; `crates/jit/src/domain/types.rs:1200-1215` |
| A bearer secret leaks or command-specific rotation drifts | Store only key/salt/domain-separated hash; rotate only for acquire/takeover, renew, or legacy upgrade; never rotate for normal heartbeat, owner release, force-evict, status/list, or recovery; reveal replacements only in the frozen responses; redact all other surfaces; and reject stale handles. | **[VERIFIED basis]** current raw `Lease.lease_id` at `crates/jit/src/storage/claim_coordinator.rs:31-38` |
| Bounded implementation work is mistaken for mergeable partial architecture | Keep five cumulative final-form packages only on `integration/cdc840ad`; run targeted plus prior tests each time, forbid flags/adapters/temporary APIs, delete predecessors before integration, and let only the fully gated rebased stack land on main. | **[ASSUMED]** |
| Preset creation, asset rescan, migration, or archive keeps a less-visible publisher | Include each in the typed-mutation inventory and delete `save_gate_preset`, rescan `save_issue`, per-issue migration saves, and archive staging/relink/event writers in the vertical cutover. | **[VERIFIED basis]** `crates/jit/src/commands/gate.rs:1170-1190`; `crates/jit/src/commands/document.rs:689-723`; `crates/jit/src/commands/migrate.rs:1-78`; `crates/jit/src/commands/archive.rs:370-854` |
| Generic result unification breaks automation | Keep current top-level envelopes and generated schemas; share only internal delta and additive nested change records. | **[VERIFIED basis]** existing result types cited in Question 5 |
| The v1 planner becomes a profile lifecycle engine | Accept profile-produced seed changes but import no profile types; profile removal remains out of scope per container D-06. | **[VERIFIED basis]** `jit issue show cdc840ad` |
