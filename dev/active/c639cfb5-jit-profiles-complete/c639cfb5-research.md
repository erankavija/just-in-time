# Research: profile lifecycle architecture choices

**Container:** `c639cfb5` — Complete profile lifecycle, composition, and upgrades  
**Planning node:** `33f76b11`  
**Date:** 2026-07-18  
**Scope:** architecture decisions for the v1.1 lifecycle delta

This document resolves architecture questions that block synthesis. It is research, not
an implementation plan. `VERIFIED` claims are confirmed against current repository code,
documents, or tracker state; `ASSUMED` claims identify a proposed contract and its residual
risk. No external source is load-bearing, so there are no `CITED` claims.

- **VERIFIED:** Container `REQ-12` and `D-07` require every lifecycle extension to cut over
  to one long-term resolver, composition engine, ownership model, record, public surface,
  and publication path. V1 survival must be a durable one-way migration, and each cutover
  removes its superseded production path in the same change
  (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json`).

## 1. Minimal ownership model

### Question

What is the smallest per-file/per-key ownership model that supports identical shared
contributions, reconfiguration, and three-way upgrade without making applied records an
alternative configuration authority?

### Evidence

- **VERIFIED:** The v1 record stores only profile ID, version, origin, package hash, and
  per-target hashes; it has neither a wire-version field nor per-contribution ownership
  (`crates/jit/src/profile/application.rs:7-21`).
- **VERIFIED:** The current package model already assigns stable semantic identities to map
  entries, set members, keyed arrays, and projections, and groups assets and regions by
  repository target (`crates/jit/src/profile/package.rs:421-521`).
- **VERIFIED:** A target hash frames all authored operations for a whole target. It cannot
  distinguish one semantic key from its neighbors and therefore cannot support safe
  per-key replacement or removal (`crates/jit/src/profile/package.rs:453-521`).
- **VERIFIED:** Current merge behavior compares semantic values structurally and treats an
  equal definition as an idempotent no-op while rejecting a different definition
  (`crates/jit/src/profile/planner.rs:501-683`).
- **VERIFIED:** Project decision `@/charter/D-6` makes declared repositories and registries
  authoritative. The current epic repeats that applied records are evidence, not an
  effective-configuration source (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json`).

### Options considered

#### A. One central ownership index

- **ASSUMED:** A central map from contribution identity to an owner set makes shared-owner
  lookup direct, but it duplicates profile-to-contribution membership also needed in each
  profile's provenance record. The risk is split-brain state when the two directions are
  not changed together.
- **VERIFIED:** The transaction kernel can update several records recoverably, but that
  removes publication tearing rather than the logical duplication itself
  (`crates/jit/src/storage/file_transaction.rs:84-176`).

#### B. Whole-file ownership

- **VERIFIED:** Several semantic identities share `.jit/config.toml`, `.jit/gates.toml`,
  `.jit/rules.toml`, and `.jit/templates.toml`; whole-file ownership would claim unrelated
  adopter-authored values (`crates/jit/src/profile/manifest.rs:56-135`).
- **VERIFIED:** Generic projections can also share one managed prose target, so rendered
  target ownership is not equivalent to ownership of the projection declarations that
  produce it (`crates/jit/src/validation/projection.rs:330-365`).

#### C. One claim set per applied profile, with owners derived by joining records

- **ASSUMED:** Each versioned per-profile record can store one sorted claim for every
  authored semantic unit. An aggregate load of all records derives the owner set for a
  contribution identity, avoiding a second central index.
- **ASSUMED:** The sole current applied-record wire has explicit `record_version: 2`.
  Ordinary list/show/validate/diff/apply/upgrade readers accept only version 2; the
  five-field v1 shape exists only at the isolated one-way migration boundary.
- **ASSUMED:** A minimal claim has `identity`, `base_fingerprint`, and a canonical
  `retain_if_unowned` bit. The identity is a tagged tuple rather than an opaque path:
  semantic registry target plus adapter kind plus key; file target plus mode; or managed
  target plus region ID. The fingerprint is a domain-separated digest of the canonical
  semantic value or exact file/region bytes and mode.
- **ASSUMED:** Claims describe what a profile contributed at its recorded base. They do not
  contain an effective value loader, do not synthesize missing registry content, and do
  not override current repository bytes. The risk if this boundary is violated is the
  shadow-configuration database identified by `RISK-01` on the epic.
- **ASSUMED:** `retain_if_unowned` is the long-term representation of a repository baseline
  owner, not a migration-only flag. A profile adopting an already-identical user-authored
  value sets it, so the value remains if the last profile claim later disappears. V1
  migration uses the same rule because prior exclusive creation cannot be proven.
- **ASSUMED:** Derived outputs such as a registry-driven rendered document are not separate
  profile claims. The profile claims the authoritative projection declaration; the shared
  materializer derives the output. Static assets and static managed regions remain direct
  claims because their package bytes are the authored input.

### Recommendation

- **RECOMMENDATION — VERIFIED + ASSUMED:** Choose option C. Retain one versioned JSON record
  per profile and store only that profile's sorted claim set. Compute identical shared
  ownership as the sorted set of records containing the same identity and base
  fingerprint. This follows the existing per-profile path while avoiding a duplicated
  central owner set (`crates/jit/src/commands/profile.rs:248-300`).
- **RECOMMENDATION — ASSUMED:** Define three-way decisions over canonical fingerprints:
  `current == recorded_base` is untouched; `current != recorded_base` is drift; equal
  candidate fingerprints from several owners share the identity; different candidates
  conflict. A removed claim may delete content only when current equals base, no other
  record claims it, and `retain_if_unowned` is false.
- **RECOMMENDATION — ASSUMED:** Make claim iteration and aggregate owner derivation
  deterministic with canonical identities and sorted maps/sets. Keep human-readable
  target/key fields alongside hashes for actionable diagnostics, but never store a second
  copy of the current effective configuration.

### Rejected alternatives and trade-offs

- **REJECTED — ASSUMED:** Reject a central owner index because faster lookup does not
  justify duplicated membership or an additional repair protocol at the expected profile
  count.
- **REJECTED — VERIFIED:** Reject whole-file ownership because current registry adapters
  deliberately merge independently keyed values in shared files
  (`crates/jit/src/profile/planner.rs:415-683`).
- **REJECTED — ASSUMED:** Reject storing full base values in records by default. Canonical
  fingerprints are sufficient for drift and update decisions, while an exact historical
  package supplies richer diagnostics when available. The trade-off is a less detailed
  base-side diff when the historical package is unavailable.

## 2. Fail-closed migration from v1 records

### Question

How can the five-field v1 `AppliedProfileRecord` migrate when the exact historical package
content may or may not still resolve?

### Evidence

- **VERIFIED:** V1 deserialization expects the exact five-field type and current apply
  rejects any non-exact record as an installed-record conflict
  (`crates/jit/src/commands/profile.rs:391-416`).
- **VERIFIED:** The current production resolver exposes only the package embedded in the
  running binary and matches by ID; it is not a historical package archive
  (`crates/jit/src/commands/profile.rs:330-347`).
- **VERIFIED:** Package hashes cover the canonical manifest and all declared source bytes,
  while target hashes cover authored operations grouped by target
  (`crates/jit/src/profile/package.rs:453-526`). Exact agreement authenticates the pinned
  embedded v1 base, but target hashes alone cannot reconstruct its per-key values.
- **VERIFIED:** A v1 application may have encountered an already-identical semantic entry
  and treated it as a no-op before writing package-level provenance. The old record cannot
  prove that such an entry was exclusively created by the profile
  (`crates/jit/src/profile/planner.rs:501-683`).
- **VERIFIED:** The shipped manifest discriminator is `manifest-version = 1`, the strict v1
  wire type has no lifecycle fields, and the embedded `jit-dogfood` package is loaded from
  one authored directory (`crates/jit/src/profile/manifest.rs:6-30`,
  `crates/jit/src/profile/dogfood.rs:11-45`, `profiles/jit-dogfood/manifest.toml`).
- **VERIFIED:** The existing v1 package hash is pre-resolution: it hashes a canonicalized v1
  manifest plus authored payload paths and bytes. No variable input exists in that domain
  (`crates/jit/src/profile/package.rs:453-526`).

### Options considered

#### A. Infer bases from the current repository

- **ASSUMED:** Treating current values as historical bases would erase the distinction
  between untouched content and user drift. A later upgrade could then overwrite or
  remove user intent while reporting it as profile-owned.

#### B. Bundle a permanent copy of every historical embedded package

- **ASSUMED:** A historical archive makes automatic migration easy, but it duplicates
  package inventory and grows without a lifecycle policy. It conflicts with the explicit
  requirement to keep `jit-dogfood` as one authored inventory.

#### C. Verify an exact package image, migrate only provable claims, otherwise stop

- **ASSUMED:** The repository migrator can recognize the five-field input and authenticate
  it only against the byte/hash-pinned `jit-dogfood` package in the running binary's
  embedded catalog. Exact ID, version, embedded origin, package hash, and target-hash
  agreement are all required before using it as the historical base.
- **ASSUMED:** Each reconstructed unit can then be canonicalized and compared with the
  current repository. An all-exact repository can produce one canonical migration plan.
  Any absent, differing, malformed, or ambiguously identified unit blocks publication; no
  partial or runtime-only migration state is accepted.
- **ASSUMED:** Claims reconstructed from v1 use canonical `retain_if_unowned = true`
  semantics because the old record cannot prove exclusive creation even when current
  content equals the authenticated base. This is the same baseline-ownership rule used
  whenever a new profile adopts an already-identical repository value, not a compatibility
  mode.
- **ASSUMED:** Install origin and evidence source remain separate canonical concepts, but
  shipped v1 records have one valid combination: install origin is `embedded`, and migration
  evidence source is the exact pinned embedded `jit-dogfood` image. A path package is never
  accepted as evidence for those records even if its bytes produce a matching hash.

#### D. Evolve the v1 manifest in place

- **ASSUMED:** Adding lifecycle fields under `manifest-version = 1` would change the
  released wire contract and risk changing the decoded/hash image of the shipped package.
  Keeping separate v1 and v2 runtime package models would instead violate the canonical
  path requirement.

### Recommendation

- **RECOMMENDATION — VERIFIED + ASSUMED:** Choose option C as a one-way repository-format
  migration, not a dual-format lifecycle reader. Before lifecycle services open records,
  a dedicated migration boundary recognizes the five-field input, authenticates and
  reconstructs it, and publishes the canonical `record_version: 2` record recoverably.
  All `list`, `show`, `validate`, `diff`, apply, and upgrade code then reads only version 2.
- **RECOMMENDATION — ASSUMED:** The cutover removes the old runtime record path,
  exact-record matcher, and one-package record constructor after redirecting every caller
  through the canonical migration and ownership model. The five-field decoder remains
  only as an input schema of the durable repository migrator; it is never an in-memory
  fallback or alternate lifecycle representation.
- **RECOMMENDATION — ASSUMED:** When the running binary does not contain the exact pinned
  embedded base, return an actionable typed migration error naming the required ID,
  version, package hash, and target hashes. There is no path selector, network lookup,
  nearby-directory search, or user-supplied evidence override for v1 migration.
- **RECOMMENDATION — ASSUMED:** Keep v1 migration all-or-nothing and conservative: exact
  base plus exact current units produce canonical baseline-retained claims; current drift,
  unavailable pinned embedded content, or claim ambiguity blocks reconfiguration/upgrade.
  Repository drift must be resolved independently; no supplied package can replace the
  required embedded evidence.
- **RECOMMENDATION — VERIFIED + ASSUMED:** Introduce new dependency, incompatibility,
  variable, and reference syntax only as manifest v2. Use one version-dispatching decoder
  that immediately normalizes v1 and v2 wire images into one source-neutral semantic
  package model; downstream validation, hashing results, resolution, composition,
  ownership, and publication never branch on a v1/v2 runtime model.
- **RECOMMENDATION — ASSUMED:** Keep v1 wire decoding as a permanent input capability
  because the unchanged shipped dogfood package remains v1. This is one branch at the
  decoder boundary into the canonical semantic model, not a legacy runtime/planner path.
- **RECOMMENDATION — ASSUMED:** Keep package identity strictly pre-resolution. Freeze and
  fixture-pin the released v1 `jit-dogfood` directory bytes and its current v1 hash
  algorithm/result. Define the v2 hash domain over the captured unresolved raw manifest and
  payload bytes, including reference/template source but excluding values-file,
  environment, and `--set` resolutions. Plan and claim identities cover resolved public
  values and final semantic/file bytes.
- **RECOMMENDATION — VERIFIED + ASSUMED:** Use the still-shipped exact embedded v1
  `jit-dogfood` image as evidence when its package and target hashes match a v1 record. Do
  not copy it into a historical-package directory or reauthor it as v2. If exact embedded
  evidence is unavailable, fail closed.
- **RECOMMENDATION — ASSUMED:** A successful migration is metadata on the requested
  lifecycle event, not its own operation or event. The event carries sorted
  `record_migrations` entries exactly shaped as
  `{profile_id, from_version: 1, to_version: 2}`.

### Rejected alternatives and trade-offs

- **REJECTED — ASSUMED:** Reject current-state inference because it converts unknown history
  into false certainty.
- **REJECTED — ASSUMED:** Reject a duplicated embedded historical inventory because it
  creates a second package source of truth and an unbounded retention promise.
- **REJECTED — ASSUMED:** Reject partial migration because mixed old/canonical claims
  make later shared-owner and removal decisions dependent on which units happened to be
  clean during migration. The trade-off of all-or-nothing migration is that one drifted
  unit blocks the mutation, but that is the required fail-closed behavior.
- **REJECTED — ASSUMED:** Reject dual-format readers, lazy per-command adapters, and fallback
  to package-level v1 behavior. Those designs make every lifecycle operation carry two
  ownership semantics indefinitely and violate `REQ-12`/`D-07`.
- **REJECTED — ASSUMED:** Reject adding lifecycle fields to manifest v1 or maintaining two
  semantic package types. V2 is the only new wire syntax, while both wire versions decode
  once into the canonical model.
- **REJECTED — ASSUMED:** Reject changing the shipped v1 dogfood bytes/hash or carrying a
  duplicate historical inventory. Fixture-pinned exact evidence preserves migration
  without creating another package source of truth.
- **REJECTED — ASSUMED:** Reject local-directory evidence for a shipped v1 record. Origin
  authenticity is part of the recorded claim, so byte/hash similarity from `path:DIR`
  cannot substitute for the pinned embedded package.

## 3. Deterministic dependency and contribution resolution

### Question

How should dependencies, incompatibilities, and multi-profile contributions resolve
deterministically without giving selection order semantic conflict precedence?

### Evidence

- **VERIFIED:** The v1 manifest is closed and has no dependency or incompatibility fields
  (`crates/jit/src/profile/manifest.rs:12-30`).
- **VERIFIED:** Package validation already rejects duplicate identities within one package,
  unsafe paths, invalid semantic versions, and invalid JIT compatibility requirements
  (`crates/jit/src/profile/package.rs:216-417`).
- **VERIFIED:** Current profile commands resolve exactly one embedded ID, and local package
  discovery does not exist (`crates/jit/src/commands/profile.rs:330-347`).
- **VERIFIED:** The container excludes remote discovery and implicit local search, while
  requiring local IDs not to shadow embedded IDs
  (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json`).

### Options considered

#### A. Sequentially apply selected profiles (historical planning baseline)

- **HISTORICAL VERIFIED:** At planning time, application published and audited one package
  immediately, so a loop exposed intermediate states and made the first package occupy keys
  before the next was considered (`crates/jit/src/commands/profile.rs:137-245`).
- **HISTORICAL ASSUMPTION:** Under sequential application, order becomes an accidental conflict policy
  and shared ownership cannot be recorded as one aggregate decision.

#### B. Add a version solver over discovered packages

- **ASSUMED:** A general solver is unnecessary when there is no registry and local packages
  are explicitly addressed. It would imply candidate discovery and backtracking semantics
  that the v1.1 scope deliberately excludes.

#### C. Resolve the complete candidate-and-applied closure, then compose by identity

- **ASSUMED:** Normalize repeated tagged selectors into a candidate map keyed by profile ID.
  Embedded dependencies may come from the embedded catalog; local dependencies must be
  explicitly selected. Reject different packages with one ID and reject any local package
  using an embedded ID, even when bytes happen to match.
- **ASSUMED:** A mutation closure is larger than its selected candidates. It includes every
  canonical applied record, every surviving unselected owner of a touched identity, every
  selected replacement, and every dependency and incompatibility claim reachable from
  either selected or surviving applied profiles. Planning against only selected packages
  is incomplete even when their internal graph is valid.
- **ASSUMED:** Canonical records normalize dependency evidence as dependency ID, authored
  requirement, resolved version, and resolved package hash, and normalize incompatibility
  declarations as profile ID plus authored version condition. These fields govern profile
  lifecycle consistency only; current repository registries still govern JIT behavior.
- **ASSUMED:** Validate every selected candidate and every surviving applied claim as one
  closed graph. Detect cycles and unsatisfied/replaced dependencies before planning, check
  incompatibilities symmetrically across selected and surviving profiles, then use a stable
  topological order with profile ID as tie-breaker only for deterministic traversal.
- **ASSUMED:** Index contributions by canonical semantic identity. Structurally equal
  definitions collapse to one desired value with a sorted owner set; different definitions
  produce one conflict listing every contributor in sorted order. Selection order never
  chooses a definition.
- **ASSUMED:** Mixed ownership is resolved over the whole closure. A selected owner may drop
  a claim while a surviving owner retains it, which removes only the selected claim. A
  selected owner may change a shared value only when every surviving owner either selects
  the identical new value or relinquishes the claim; an unselected owner retaining the old
  base makes the change a conflict. When all owners select the same new value, one update
  advances every claim base together.

### Recommendation

- **RECOMMENDATION — VERIFIED + ASSUMED:** Choose option C. Resolve the complete package graph
  and variable declarations before invoking repository materialization, then union it with
  all applied records and surviving owners before any lifecycle decision. Keep root
  selector order only as request/result metadata; use canonical ID order for otherwise-equal
  traversal; treat semantic equality, not order, as the only deduplication rule.
- **RECOMMENDATION — ASSUMED:** Do not implement version choice. There is exactly one
  candidate package per ID in a request. A dependency requirement either accepts that
  candidate, resolves the named embedded package, or fails with a sorted explanation.
- **RECOMMENDATION — ASSUMED:** Make incompatibility checking symmetric at evaluation time:
  a selected pair fails when either package declares the other incompatible. This prevents
  outcome differences caused by which package was loaded first.
- **RECOMMENDATION — ASSUMED:** Persist normalized dependency and incompatibility claims in
  the canonical record transaction with ownership claims. Reconfiguration and upgrade
  replace those claims only after the entire proposed post-mutation closure validates.
- **RECOMMENDATION — ASSUMED:** Test mixed selected/unselected owners explicitly: selected
  owner removal with a survivor, subset update conflict, unanimous owner update, baseline
  retention after the last profile claim, drifted shared content, replacement of a profile
  required by an unselected dependent, and new incompatibility with an unselected profile.

### Rejected alternatives and trade-offs

- **HISTORICAL REJECTION — VERIFIED + ASSUMED:** Reject sequential apply because the planning-time
  command was a publication boundary and could not represent aggregate ownership
  (`crates/jit/src/commands/profile.rs:137-245`).
- **REJECTED — ASSUMED:** Reject dependency-order conflict precedence. Topological order may
  schedule computation but cannot select among different definitions.
- **REJECTED — ASSUMED:** Reject selected-only validation because it can overwrite a shared
  value still claimed by an unselected profile or invalidate an unselected dependent after
  upgrading its dependency.
- **REJECTED — ASSUMED:** Reject a registry-style version solver until remote or multi-version
  discovery is in scope. The trade-off is that users must explicitly provide the single
  local dependency candidate.

## 4. Persistable variables and reference rendering

### Question

How should explicitly non-secret variables resolve and render deterministically across
supported semantic values and generated text without creating a second template language
or an implied secret channel?

### Evidence

- **VERIFIED:** V1 has no variable declaration and rejects the reserved `{{jit:` token in
  contributions, assets, and regions (`crates/jit/src/profile/planner.rs:18-21`,
  `crates/jit/src/profile/planner.rs:278-309`).
- **VERIFIED:** Owner-updated `REQ-04`, `REQ-06`, `REQ-11`, and `D-06` make variables
  explicitly non-secret and persistable, require one reference/template model, and forbid
  any secret declaration, input, indirection, persistence, or audit channel
  (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json`).
- **VERIFIED:** Current plan identities hash planned final target bytes
  (`crates/jit/src/profile/planner.rs:266-275`,
  `crates/jit/src/profile/planner.rs:841-864`), while current package identity hashes the
  unresolved manifest and authored payload rather than a later input resolution
  (`crates/jit/src/profile/package.rs:453-526`).

### Options considered

#### A. Separate semantic-reference and text-template syntaxes

- **ASSUMED:** Separate syntaxes would require two escaping rules, two validation passes,
  and two user mental models. Their supported value types would drift as new contribution
  adapters are added.

#### B. Treat every string as an implicit template

- **ASSUMED:** Implicit interpolation makes literal reserved-looking text ambiguous and
  can silently change existing package meaning. It also gives non-string semantic values
  no typed reference representation.

#### C. One frozen public variable and reference grammar

- **ASSUMED:** Manifest v2 declares only `[[variable]]` entries with required string
  `name` and optional string `default` and `env` fields. Unknown fields and duplicate names
  fail. `name` and `env` use the portable case-sensitive grammar `[A-Z][A-Z0-9_]*`; both
  source fields may be absent when the caller must supply the value.
- **ASSUMED:** The only values-file surface is one optional `--values-file PATH` whose TOML
  contains exactly a `[variables]` string map. The only direct override is repeatable
  `--set NAME=VALUE`. Resolution order is manifest `default`, values-file `[variables]`,
  manifest-declared `env`, then `--set`; undeclared names and duplicate values at one
  precedence tier fail deterministically.
- **ASSUMED:** All resolved values are UTF-8 strings. `--set` splits on the first `=`, and
  empty strings are valid from every source; a present empty environment value is not the
  same as an absent variable. Non-UTF-8 environment values fail with a name-only diagnostic.
- **ASSUMED:** The only reference spelling is `{{jit:var:NAME}}`. It is recognized in
  string leaves of supported semantic contribution values. Manifest-v2 asset and region
  declarations each add `template: bool`, canonically authored as `template = true|false`
  and defaulting to false when omitted; only a true declaration permits placeholders in
  its UTF-8 body. References are forbidden in package/profile IDs, versions, dependency and
  incompatibility fields, contribution identities, paths, executable modes, projection
  names/targets, and managed-region marker or placement fields.
- **ASSUMED:** Rendering is a single non-recursive pass. A source string may contain several
  references and ordinary surrounding text; resolved values are inserted verbatim and are
  never rescanned. Every `{{jit:` sequence must be one valid declared variable reference;
  malformed/unknown tokens fail. V1 bodies and v2 asset/region bodies with `template = false`
  reject placeholders; `template = true` requires UTF-8. Untemplated binary bodies remain
  valid when they contain no placeholder.
- **ASSUMED:** The package hash covers the unresolved captured package: its variable
  declarations, reference nodes/template source, and authored payload bytes. It never
  varies with a values file, environment, or `--set`.
- **ASSUMED:** The aggregate plan identity includes the sorted resolved public-value map
  and exact final bytes. Each ownership claim fingerprints the resolved semantic value or
  final file/region bytes it established, so reconfiguration has a stable base.
- **ASSUMED:** Canonical records persist resolved values plus their source kinds. Audit
  events record variable names and the exact source-kind enum
  `default|values_file|environment|set` only; they do not duplicate values already owned
  by the record. Profiles expose no `secret`, `sensitive`, credential, prompt,
  keyring, encrypted-value, or secret-indirection vocabulary.

### Recommendation

- **RECOMMENDATION — VERIFIED + ASSUMED:** Choose option C and freeze these public shapes:
  manifest v2 `[[variable]] name/default/env`, optional `--values-file PATH` with TOML
  `[variables]`, repeatable `--set NAME=VALUE`, `{{jit:var:NAME}}`, and v2 asset/region
  `template: bool` serialized as `template = true|false` with default false. All commands,
  generated schema, MCP tools, examples, and canonical docs use exactly these spellings.
- **RECOMMENDATION — ASSUMED:** Decode every allowed `{{jit:var:NAME}}` occurrence to one
  internal reference node, resolve all public inputs once before composition, and render
  supported semantic strings plus generated UTF-8 text through the same single-pass
  function. Graph/identity/target-shaping fields remain fully known before resolution.
- **RECOMMENDATION — ASSUMED:** Test exact grammar and precedence, inline/multiple and
  non-recursive substitution, malformed/unknown/forbidden-position references, UTF-8 and
  binary payload boundaries, absent/false/true asset and region template flags, missing
  and extra values, equal-precedence duplicates,
  deterministic reconfiguration, package
  hash stability across input sets, plan/claim identity changes across resolved values,
  event omission of values, and complete absence of a secret-input schema or CLI surface.

### Rejected alternatives and trade-offs

- **REJECTED — ASSUMED:** Reject two interpolation languages because they create competing
  semantics and future adapter-specific exceptions.
- **REJECTED — ASSUMED:** Reject implicit all-string interpolation because literal package
  bytes must remain unambiguous and v1 packages must retain their exact meaning.
- **REJECTED — VERIFIED + ASSUMED:** Reject every sensitive-value or resupply design. The
  updated owner contract intentionally makes resolved variables public, persistable inputs;
  adding a secret type, redaction wrapper, indirection, prompt, or later resupply path would
  recreate a forbidden product surface.

## 5. Command and internal surface factoring

### Question

How should local packages, repeated selection, validate/diff/reconfiguration/upgrade, and
public contracts be factored while reusing the v1 package model, planner, transaction
kernel, generated schema/MCP path, and Git-free behavior?

### Evidence

- **VERIFIED:** Current `init` accepts one optional embedded ID, and `profile` has only
  `list`, `show`, and one-ID `apply`; only non-dry-run apply is classified as a recovery
  mutation (`crates/jit/src/cli.rs:39-57`, `crates/jit/src/cli.rs:2821-2903`).
- **VERIFIED:** `EmbeddedProfilePackage` directly borrows `include_dir` bytes and its
  source-map constructor is private, so local loading cannot reuse the runtime type without
  a source-neutral package image refactor (`crates/jit/src/profile/package.rs:33-67`).
- **VERIFIED:** The existing planner is pure over a package and immutable snapshot, while
  command orchestration rebuilds under locks and feeds one `FileTransactionPlan`
  (`crates/jit/src/profile/planner.rs:132-275`,
  `crates/jit/src/commands/profile.rs:137-245`).
- **VERIFIED:** Public output schema is generated from result types and already models
  dry-run versus mutation as a `oneOf`; the MCP server consumes generated command schema
  rather than owning a parallel protocol (`crates/jit/src/schema.rs:475-495`,
  `mcp-server/lib/schema-loader.js:1-32`).
- **VERIFIED:** Profile acceptance currently exercises application without Git, and
  project decision `@/charter/D-4` keeps core commands Git-optional
  (`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:232-299`).
- **VERIFIED:** The v1 audit surface has one `ProfileApplied` event carrying package-level
  identity; the event enum/catalog/parser are closed consumers that must change together
  (`crates/jit/src/domain/types.rs:1468-1490`,
  `crates/jit/src/domain/event_catalog.rs:111-170`,
  `crates/jit/src/domain/event_log.rs:35-89`).

### Options considered

#### A. Add separate embedded and local command/planner stacks

- **ASSUMED:** Separate stacks would duplicate manifest validation, hashing, semantic
  composition, result shapes, and transaction behavior, making origin affect semantics.

#### B. Make every lifecycle verb independently materialize and publish

- **ASSUMED:** Independent apply/reconfigure/upgrade pipelines would repeat snapshot,
  ownership, audit, validation, and recovery logic and make their guarantees drift.

#### C. One source-neutral package pipeline and one aggregate lifecycle operation model

- **ASSUMED:** Embedded lookup and capability-confined local-directory capture are I/O
  loaders that produce one immutable validated package image. Manifest parsing, hashing,
  dependency resolution, variables, composition, and results operate on that image without
  branching on origin.
- **ASSUMED:** One repeatable selector occurrence uses the fixed grammar
  `--profile id:ID` or `--profile path:DIR`. A single occurrence stream
  preserves exact embedded/local interleaving in argv and results while keeping source
  resolution unambiguous. Separate flag arrays cannot reliably express their cross-kind
  interleaving as one public contract.
- **ASSUMED:** `profile apply` remains the canonical initial/reapply operation. Applying an
  already-recorded version with changed persistable public inputs is reconfiguration,
  reported by per-profile action rather than a third requested operation. Profile
  `upgrade` alone enables
  version-replacement and upgrade-time removal semantics. `validate` and `diff` are
  read-only views over the same aggregate planner; `list` and `show` inspect catalogs,
  explicit packages, and applied records.
- **ASSUMED:** One internal `LifecycleRequest` preserves tagged selector occurrences,
  operation kind, public inputs, and dry-run. The resolver expands it to the complete
  selected-plus-applied closure before producing one aggregate change/conflict model. Only
  mutation execution adds canonical record and audit images and invokes the transaction
  kernel.
- **ASSUMED:** The sole current mutation event is `profile_lifecycle_changed`.
  `requested_operation` is exactly `apply|upgrade`; init delegates to apply. Its sorted
  per-profile `action` is exactly `installed|unchanged|reconfigured|upgraded`, and sorted
  `record_migrations` entries are exactly
  `{profile_id, from_version: 1, to_version: 2}`. Migration is metadata on the requested
  event, never an operation or separate event.
- **ASSUMED:** A dry-run appends no event. A real mutation whose complete aggregate is
  unchanged appends no event. A changed aggregate emits one event and may include
  `unchanged` actions for selected profiles that were unchanged beside other changes.

### Recommendation

- **RECOMMENDATION — VERIFIED + ASSUMED:** Choose option C and keep four boundaries:
  source loaders capture packages; pure domain code resolves and composes them; after
  `cdc840ad` reaches Done and its actual postconditions are verified, the cutover uses the
  resulting canonical materializer to derive the complete repository delta; command/storage
  code publishes that delta plus records and events through the sole recoverable path.
- **RECOMMENDATION — ASSUMED:** Cut over to one repeatable tagged-selector request and one
  count-wrapped deterministic result envelope for both one and many profiles. Update human
  output, JSON Schema, MCP curation, tests, and docs in the same change, then remove the
  single-package request/result and dispatch path. Dry-run versus mutation may remain an
  honest schema union because those are distinct operations, not compatibility behavior.
- **RECOMMENDATION — ASSUMED:** Use one tagged selector field in CLI, generated schema, MCP,
  command-executor requests, events, and tests, accepting exactly `id:ID|path:DIR`.
  Preserve occurrence order for presentation only; resolver and composition semantics
  remain order-independent.
- **RECOMMENDATION — VERIFIED + ASSUMED:** Update Clap recovery classification, result/error
  schemas, exact CLI acceptance, MCP curation, event catalog/parser, and canonical profile
  reference as one public-contract slice. Keep local loaders on ordinary filesystem
  capabilities and never call claim/worktree APIs, preserving Git-free behavior.
- **RECOMMENDATION — ASSUMED:** Freeze one current audit/result vocabulary: requested event
  operation `apply|upgrade`; profile action
  `installed|unchanged|reconfigured|upgraded`; sorted version-1-to-2 record migrations;
  init-as-apply; and no event for dry-run or an entirely unchanged aggregate. Historical
  `ProfileApplied` remains append-only decode history, with no current constructor/emitter.

### Rejected alternatives and trade-offs

- **REJECTED — ASSUMED:** Reject separate local and embedded engines because package origin
  is discovery provenance, not semantic policy.
- **REJECTED — ASSUMED:** Reject a standalone `reconfigure` command for v1.1. Existing
  `apply` already owns idempotent reapplication, and changed persisted inputs are the same
  package-planning operation. The exact per-profile actions distinguish installed,
  unchanged, and reconfigured results without inventing a requested operation.
- **REJECTED — ASSUMED:** Reject an invocation-sensitive single-versus-multiple JSON union.
  It would preserve an additional compatibility path indefinitely. One collection envelope
  is the canonical lifecycle contract required by `REQ-12`/`D-07`.
- **REJECTED — ASSUMED:** Reject leaving the embedded-only planner or result types behind as
  wrappers. Once embedded and local loaders feed the source-neutral image, all production
  callers move to the aggregate resolver/composer and superseded functions and types are
  removed in that same cutover.
- **REJECTED — ASSUMED:** Reject separate source-specific selector flag collections. Even
  if a parser can expose occurrence indices internally, two public selector channels invite divergent
  parsing/schema behavior and obscure the required embedded/local interleaving.

## 6. Dependency boundary on `cdc840ad`

### Question

Where should the implementation dependency on `cdc840ad` appear so planning can complete
but no source-neutral foundation lands beside the live embedded-only runtime?

### Evidence

- **VERIFIED:** `cdc840ad` is Backlog with an unmet breakdown dependency as of 2026-07-18.
  Its issue and plan assign it the side-effect-free final-view materializer, profile/config
  integration, shared-target composition, unified marker mechanics, and recoverable
  multi-target publication boundary; those are expected postconditions, not facts about
  the repository tree today
  (`.jit/issues/cdc840ad-9332-4936-9381-772318967f0f.json`,
  `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md`).
- **VERIFIED:** `33f76b11` currently depends only on the completed v1 profile MVP, so profile
  lifecycle planning can complete before `cdc840ad` implementation
  (`jit graph deps 33f76b11 --json`, observed 2026-07-18).
- **VERIFIED:** Package parsing, dependency resolution, variable resolution, contribution
  identity, ownership aggregation, and three-way classification belong in pure domain
  layers, while storage and commands own publication (`AGENTS.md:135-152`). Layer
  separation does not require those engines to land as independently callable production
  paths.
- **VERIFIED:** Every lifecycle mutation criterion requires `cdc840ad` to reach Done before
  the cutover and then requires publication through the actual canonical materializer and
  transaction path found in the resulting tree
  (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json`).

### Options considered

#### A. Make the planning node or entire continuation epic depend on `cdc840ad`

- **ASSUMED:** Making planning node `33f76b11` depend on implementation would serialize
  research, synthesis, and breakdown behind code they do not consume. The implementation
  dependency can start at the single live cutover without weakening it.

#### B. Leave only a prose note and add no graph edge

- **ASSUMED:** A prose-only dependency permits an execution lead to schedule a second
  profile-specific materializer or publication path before the shared foundation exists.

#### C. Put the edge on the first live source-neutral cutover

- **ASSUMED:** The load-bearing boundary occurs when any production caller stops using the
  embedded-only package/planner path and starts using the source-neutral resolver. That
  cutover must wait for `cdc840ad` to reach Done because a live resolver without the canonical
  materializer/publication path would create the forbidden temporary architecture.
- **ASSUMED:** The cutover is one vertical end-to-end node: existing embedded v1 and new
  local/v2 inputs enter the source-neutral decoder/resolver; composition and complete
  applied-state closure produce one desired view; after the prerequisite reaches Done the
  cutover verifies and consumes its actual repository image, materializer, managed-region
  engine, delta/audit seed, mutation session, and sole publisher; runtime dispatch,
  human/JSON output, generated schema/MCP, acceptance coverage, and the canonical profile
  reference switch together; superseded live code is removed.
- **ASSUMED:** The source-neutral decoder/model, resolver/composer, canonical record and
  migration, shared materializer integration, live v1 behavior, and first public local/v2
  surface land only as this one vertical implementation cutover. They are not separate
  breakdown leaves or mergeable engines beside live v1. Later lifecycle operations extend
  the already-live canonical path.

### Recommendation

- **RECOMMENDATION — VERIFIED + ASSUMED:** Choose option C. During breakdown, add a direct
  DAG edge from the first live source-neutral profile-runtime cutover node to `cdc840ad`.
  Execution waits for `cdc840ad` to reach Done, then inspects the resulting code and tests
  before implementation. Do not defer the edge until a later mutation-integration leaf,
  and do not add it to `33f76b11` or the breakdown node.
- **RECOMMENDATION — ASSUMED:** Name the boundary by behavior, for example “Cut over the
  live profile runtime to source-neutral lifecycle resolution and materialization.” Its
  acceptance must span runtime, publication, migration, human/JSON output, generated
  schema/MCP, Git-free end-to-end tests, and canonical docs in one vertical change. It has
  no alternate profile-specific projection, marker, config-sync, transaction, or public
  compatibility path.
- **RECOMMENDATION — ASSUMED:** Do not decompose decoder, semantic model, resolver,
  composition, canonical record/migration, or publication integration into independently
  landing implementation issues. They may be internally organized within the cutover, but
  its acceptance and merge boundary is one live end-to-end issue depending on `cdc840ad`.
- **RECOMMENDATION — ASSUMED:** Treat every name and removal listed in
  `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md` only as an expected prerequisite postcondition. After the
  issue reaches Done, verify the actual canonical types, behaviors, absence set, and public
  contracts; rebase the v1.1 blast radius and deletion list to what remains. Do not claim
  prerequisite removals as v1.1 work or reintroduce plan-era names that changed.

### Rejected alternatives and trade-offs

- **REJECTED — ASSUMED:** Reject a planning-level dependency because planning is evidence
  and contract work, not a consumer of the implementation foundation.
- **REJECTED — ASSUMED:** Reject placing `cdc840ad` on the planning or breakdown bracket;
  evidence and decomposition can complete first. Also reject allowing foundational
  decoder/model/resolver/record implementation to land before the dependent live cutover.
- **REJECTED — ASSUMED:** Reject placing the dependency only on a later publication leaf.
  A source-neutral resolver made live before `cdc840ad` would require a temporary
  materializer or retain the superseded embedded-only path, violating `REQ-12`/`D-07`.
- **REJECTED — ASSUMED:** Reject treating the prerequisite plan's architecture or deletion
  inventory as already present. Backlog plans describe intended postconditions; only the
  tree and tests after Done can establish what v1.1 may consume.
- **REJECTED — ASSUMED:** Reject separate runtime, public-contract, and documentation
  cutovers. Their intermediate states would document or expose a surface that is not yet
  the canonical end-to-end path.
- **REJECTED — ASSUMED:** Reject horizontal implementation leaves for decoder/model,
  resolver/composition, or record migration. Even without public CLI entry points, landed
  parallel engines would be temporary production architecture beside live v1.
- **REJECTED — ASSUMED:** Reject prose-only coordination because the dependency DAG, not
  markdown, is the scheduling source of truth.

## Consolidated decision handoff

1. **VERIFIED + ASSUMED:** Use per-profile versioned claim sets; derive shared owners by
   joining records, compare canonical base/current/candidate fingerprints, and never read
   records as effective configuration.
2. **VERIFIED + ASSUMED:** Migrate shipped v1 records only from the exact byte/hash-pinned
   embedded dogfood package and exact current units through a one-way repository migration;
   write canonical baseline-retained claims, remove the old runtime path in the cutover,
   and fail closed on missing embedded evidence, drift, or ambiguity.
3. **VERIFIED + ASSUMED:** Decode frozen v1 and new v2 wire images once into one semantic
   model; fixture-pin shipped v1 dogfood bytes/hash, keep package identity unresolved, and
   put resolved public values/final bytes in plan and claim identities.
4. **VERIFIED + ASSUMED:** Resolve one explicit candidate per profile ID, then validate the
   complete closure of selected candidates, every applied record, surviving mixed owner,
   dependency, and incompatibility before composing or publishing.
5. **VERIFIED + ASSUMED:** Freeze persistable variables as manifest v2
   `[[variable]] name/default/env`, `--values-file` TOML `[variables]`, repeatable
   `--set NAME=VALUE`, `{{jit:var:NAME}}`, and asset/region `template: bool` default false;
   persist resolved values, record only names/source kinds in events, and expose no secret
   channel.
6. **VERIFIED + ASSUMED:** Generalize to one source-neutral package image and one aggregate
   lifecycle operation model; preserve `apply` for reconfiguration, reserve `upgrade` for
   replacement/removal semantics, cut public output to one collection envelope, and remove
   superseded package/planner/result paths in the same changes that redirect their callers.
7. **VERIFIED + ASSUMED:** Use one repeated `id:ID|path:DIR` selector occurrence to preserve
   embedded/path interleaving without giving order semantic precedence.
8. **VERIFIED + ASSUMED:** Put the `cdc840ad` edge on the first live source-neutral cutover
   and make that node the single merge boundary for decoder/model/resolver/record,
   runtime/public/schema/MCP/test/docs transition. It waits for the Backlog prerequisite to
   reach Done, validates the resulting postconditions, and treats plan-era removal lists as
   expected prerequisite outcomes only.
9. **VERIFIED + ASSUMED:** Use only current record version 2 and one changed-mutation event:
   requested operation `apply|upgrade`, sorted profile actions
   `installed|unchanged|reconfigured|upgraded`, and sorted
   `{profile_id, from_version: 1, to_version: 2}` migration metadata; init is apply, while
   dry-run and an entirely unchanged aggregate emit no event.
