# Investigation: complete profile lifecycle, composition, and upgrades

**Container:** `c639cfb5` — Complete profile lifecycle, composition, and upgrades
**Date:** 2026-07-18
**Scope:** v1.1 lifecycle delta after the completed v1.0 embedded-profile MVP

This is an investigation report, not an implementation plan. The current container is
the scope authority. The original full-profile brief remains useful design prior art,
but the v1.0 implementation and public behavior are now facts the continuation must
preserve through the canonical v1.1 design. That does not preserve source-specific
implementation paths: REQ-12 and D-07 require each canonical cutover to remove the
surviving embedded-only resolver, ordinary v1 record loader, and one-ID
command/result/current-event/public-document paths in the same change. Projection,
snapshot, marker, final-byte planning, materialization, and transaction predecessors are
assigned to the planned `cdc840ad` cutover and are expected prerequisites, not v1.1
deletion work. Because `cdc840ad` is currently Backlog, none of those absences is yet a
delivered fact.

## Executive findings

- The adoption-focused profile MVP is complete, public, and deliberately apply-only.
  The current binary exposes one embedded `jit-dogfood` package, `init --profile`, and
  `profile list/show/apply` with dry-run, JSON/schema/MCP integration, package-level
  provenance, audit, overlay validation, and recoverable publication
  (`crates/jit/src/cli.rs:39-57`, `crates/jit/src/cli.rs:2821-2854`,
  `crates/jit/src/commands/profile.rs:68-133`,
  `dev/archive/9b7b5f9c-completion-report.md:11-67`). These capabilities are
  **already-done** behavior and package data, not v1.1 work to recreate. Their
  source-specific entry points are not protected when the canonical lifecycle path
  replaces them.
- REQ-12 and D-07 are the highest-priority architectural constraint. Lifecycle work must
  land as one package resolver, one composition/ownership model, and one publication
  path; a cutover that leaves the old embedded-only planner or five-field record in the
  ordinary runtime is incomplete. V1 support is an isolated, durable, versioned,
  one-way migration into the canonical record—not a serde union, adapter, fallback, or
  dual command path (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json:4`).
- The requested lifecycle delta is real. The manifest has no dependency,
  incompatibility, or variable vocabulary; its root rejects unknown fields
  (`crates/jit/src/profile/manifest.rs:12-30`). The package loader accepts only a
  recursively compiled `include_dir::Dir` (`crates/jit/src/profile/package.rs:33-67`).
  Initialization accepts one optional profile ID and application accepts one ID
  (`crates/jit/src/cli.rs:41-50`, `crates/jit/src/cli.rs:2841-2853`). The public schema
  has an exact-surface test that excludes every deferred lifecycle field and command
  (`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:517-639`).
- The current package and planner are strong extension seams, not a multi-profile
  engine. They already validate package bounds, semantic versions, declarations,
  canonical paths, content inventory, deterministic hashes, semantic equality, and
  conflict-on-difference (`crates/jit/src/profile/package.rs:216-311`,
  `crates/jit/src/profile/package.rs:328-417`,
  `crates/jit/src/profile/package.rs:453-526`,
  `crates/jit/src/profile/planner.rs:501-683`). However, planning accepts exactly one
  `EmbeddedProfilePackage`, and its installed record has only package-level hashes
  (`crates/jit/src/profile/planner.rs:132-150`,
  `crates/jit/src/profile/application.rs:7-21`). Shared ownership, base identities,
  migration, resolution, and three-way decisions do not exist.
- Manifest evolution must preserve the immutable v1 package rather than reauthor it as
  v2. The existing dogfood tree remains `manifest-version = 1`; its bytes and current
  package/target hashes are compatibility evidence. A version-discriminated v1/v2 wire
  decoder should normalize both into the one canonical package model, while the v1 hash
  path continues to serialize the v1 wire shape exactly as today
  (`profiles/jit-dogfood/manifest.toml:1-10`,
  `crates/jit/src/profile/package.rs:453-526`).
- The recoverable transaction kernel is real and reusable, but “atomic” must retain
  its documented recoverable meaning. It preflights before target mutation, writes a
  durable journal, identity-checks publication, rolls ordinary failures back, and
  retains typed recovery state when it cannot prove rollback or cleanup
  (`crates/jit/src/storage/file_transaction.rs:84-176`,
  `crates/jit/src/storage/file_transaction.rs:399-425`,
  `crates/jit/src/storage/file_transaction.rs:492-593`). It does not make a host
  filesystem perform a single instantaneous multi-file rename
  (`docs/reference/profiles.md:87-116`).
- `cdc840ad` is a load-bearing Backlog prerequisite, not a delivered baseline
  (`.jit/issues/cdc840ad-9332-4936-9381-772318967f0f.json:2-9`). Its plan expects one
  canonical repository image, materializer, strict managed-document engine, exact
  delta/storage capability, mutation session, and recoverable publisher, plus removal of
  profile projection/final-byte/snapshot/marker planners and command-local profile/init
  transaction builders (`dev/active/c639cfb5-plan.md:31-34`,
  `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md:63-88`). V1.1 implementation waits until `cdc840ad` is Done,
  re-investigates the actual delivered tree and capability names, rebases its surviving
  deletion inventory, and only then asserts the cdc-owned predecessors are absent.
- The highest-risk design area is v1 record migration. A v1 record has no format
  discriminator, compatible-JIT range, resolved inputs, per-key base identity, or
  ownership graph (`crates/jit/src/profile/application.rs:7-21`). Migration can safely
  infer ownership only when the recorded package identity still resolves and current
  repository contributions can be matched to that package. Ambiguous or drifted bases
  must fail closed; package-level target hashes alone do not prove per-key ownership.
  Successful migration must rewrite durable canonical state before normal planning or
  publication proceeds.
- Variables are explicitly non-secret, persistable inputs. Current v1 rejects the
  reserved `{{jit:` interpolation namespace everywhere
  (`crates/jit/src/profile/planner.rs:18-21`,
  `crates/jit/src/profile/planner.rs:278-309`). V2 adds one declared reference model and
  persists resolved values for deterministic reconfiguration. It must expose no secret
  declaration, `--secret` input, secret interpolation, redaction/resupply type, or audit
  channel; audit records variable names and source kinds but never resolved values.
- Canonical applied state needs full closure, not merely richer per-profile rows. After
  migration or mutation, every applied profile, resolved non-secret input, package
  identity, compatibility range, owned semantic/file identity, base fingerprint, and
  shared owner relationship must be represented consistently in the closed versioned
  record set. No orphan ownership row, missing owner back-reference, partial canonical
  record, or events-derived current state is acceptable.
- The planned lifecycle implementation is one vertical green cutover after `cdc840ad`
  reaches Done and its actual postconditions are re-verified. The strict
  decoder and canonical model, resolver/graph/composer, variables, applied records and
  migration, ownership/upgrade planner, runtime/init routing, CLI/schema/MCP/current event,
  current docs, and exact contract tests land only together. No decoder/model foundation
  may land separately as dead or alternate code; only evidence expansion follows the
  cutover (`dev/active/c639cfb5-plan.md:26-35`,
  `dev/active/c639cfb5-plan.md:40-61`).
- General removal remains correctly excluded. The old brief already distinguished
  upgrade-time removal of unchanged solely-owned content from a general removal
  command (`dev/active/9b7b5f9c-jit-profiles-planning-brief.md:67-71`), and the current
  container makes the same decision. No v1.1 surface should be named or modeled as a
  general uninstall operation.

## Addressable project context

The load-bearing addresses were resolved with `jit item show` before investigation:

- `@/charter/D-1` keeps all profile state repository-local and git-versioned; no
  package database or external installation ledger is permitted
  (`dev/vision/9db27a3a-charter.md:40-48`).
- `@/charter/D-4` requires core profile operations to work without Git
  (`dev/vision/9db27a3a-charter.md:76-85`; `AGENTS.md:162-163`).
- `@/charter/D-6` gives each addressable kind one declared source of truth. Applied
  records may describe provenance and ownership but may not supersede the repository
  registries they describe (`dev/vision/9db27a3a-charter.md:102-116`).
- `@/charter/D-8` explicitly completed an embedded offline MVP and deferred local
  packages, composition, variables, reconfiguration, diff, upgrade, removal, and
  shared ownership to this continuation (`dev/vision/9db27a3a-charter.md:138-150`).
- `@/inv/atomic-writes`, `@/inv/domain-agnostic`, and
  `@/inv/single-source-prose` remain binding. The projected statements are at
  `AGENTS.md:190-201`.

The container-local REQ-12 and D-07 further resolve a possible ambiguity in “preserve the
v1 MVP”: preserve its capabilities, package bytes, repository outcomes, and supported
upgrade path, but do not preserve implementation entry points that compete with the
canonical v1.1 lifecycle (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json:4`).
REQ-04, REQ-06, REQ-11, and D-06 likewise resolve the earlier variable ambiguity: values
are explicitly non-secret and durable; the supported surface proves the absence of secret
input/interpolation/audit machinery rather than implementing a redacted subset.

## Claim classification

| Input claim | Classification | Evidence and consequence |
|---|---|---|
| 1. The v1.0 MVP capabilities are done and excluded from reimplementation scope. | **already-done** | Completion evidence covers the embedded package, planner, transaction, provenance/audit, public CLI/schema/MCP, docs, and acceptance journey (`dev/archive/9b7b5f9c-completion-report.md:11-67`, `dev/archive/9b7b5f9c-completion-report.md:138-171`). Current code and the exact public-schema test agree (`crates/jit/src/commands/profile.rs:68-133`, `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:517-639`). Preserve capability and data compatibility through the canonical v1.1 replacement; do not preserve competing implementation paths. |
| 2. The current CLI accepts one embedded profile and has no local/multiple selection, dependencies, incompatibilities, variables, validate/diff/upgrade, or removal. | **valid-and-open** | `init` has one `Option<String>` and `profile apply` one positional ID (`crates/jit/src/cli.rs:41-50`, `crates/jit/src/cli.rs:2841-2853`). The profile family contains only list/show/apply (`crates/jit/src/cli.rs:2821-2854`); MCP tests explicitly reject deferred lifecycle tools (`mcp-server/test-integration.js:198-243`). The manifest's closed field set also excludes all lifecycle vocabulary (`crates/jit/src/profile/manifest.rs:12-30`). |
| 3. The delivered v1 package semantics provide validated merging and path safety. | **already-done semantic input; cdc-owned removals are expected future prerequisites** | Closed wire types and contribution variants are established (`crates/jit/src/profile/manifest.rs:12-82`). Preserve equality/conflict semantics, but `ProfileApplicationPlan`, `PlannedTarget*`, `plan_profile_application_against`, snapshot, projection, and final-byte removal belongs to the Backlog `cdc840ad` plan, not current fact or v1.1 credit (`dev/active/c639cfb5-plan.md:31-34`). |
| 4. Applied records are currently package-granular and audit is a single profile-applied contract. | **already-done current-state fact; lifecycle extension is open** | The record has exactly ID/version/embedded origin/package hash/target hashes (`crates/jit/src/profile/application.rs:7-21`), and acceptance freezes that shape (`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:583-595`). The event stores the same package-level identity (`crates/jit/src/domain/types.rs:1468-1490`) and its closed event catalog/parser are direct consumers (`crates/jit/src/domain/event_catalog.rs:111-169`, `crates/jit/src/domain/event_log.rs:35-89`). A canonical versioned record plus an isolated, transactional, one-way v1 migrator are necessary; an ordinary reader that accepts both shapes is forbidden. |
| 5. V1.1 must wait for and consume `cdc840ad`'s planned repository image/materializer/session/publisher, while owning none of its predecessor deletions. | **valid-and-open prerequisite; currently Backlog** | `cdc840ad` plans canonical image/marker/delta/storage and direct mutation cutovers, including removal of profile snapshot/projection/drift/final-byte planning and command-local publishers (`dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md:63-88`). After it is Done, v1.1 must inspect the actual tree, rebase against what really landed, consume the actual capabilities, and assert rather than claim cdc-owned removals. |
| 6. Manifest, record, result, event, and CLI contract changes have a repository-wide consumer blast radius. | **valid-and-open** | Direct consumers span runtime modules, generated schema, exact-surface tests, fixtures, package-derived gate presets, MCP curation/tests, CI, and canonical/adopter docs. The complete sweep is enumerated below. |
| 7. Earlier profile studies and session artifacts contain useful facts but may contain stale assumptions. | **valid-and-open** | The pre-MVP investigation says no product profile surface existed (`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-investigation.md:16-26`), which is now historical only. Its architecture and deferred-scope cautions remain useful (`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-investigation.md:499-548`). The core handoff records the later projection-manifest integration (`dev/archive/6eb585bc-handoff-5.md:17-21`). Current code and the completion report outrank pre-implementation descriptions. |
| 8. Registry SSOT, domain-agnostic pure logic, Git optionality, and public JSON/schema contracts constrain the design. | **valid-and-open** | Layer boundaries require pure domain logic, storage-owned persistence, command orchestration, and CLI/output-only presentation (`AGENTS.md:135-146`). JSON and list-envelope conventions are mandatory (`AGENTS.md:155-163`). The dogfood preset compatibility surface already derives from package data rather than owning another inventory (`crates/jit/src/profile/preset.rs:41-74`, `crates/jit/src/gate_presets/builtin.rs:17-52`). |
| 9. REQ-12/D-07 require one canonical lifecycle design, same-change removal of predecessors that survive the eventual cdc landing, and durable v1 migration with no compatibility branches. | **valid-and-open; architecture-defining** | The provisional v1.1 inventory is the embedded-only package/resolver, ordinary v1 record loader/matcher, source-specific one-ID init/profile command and dispatch, current `profile_applied` constructor/appender, surviving v1 result/provenance family, and current public/docs consumers. The final inventory is rebased only after `cdc840ad` is Done; cdc-owned paths are then absence assertions, never v1.1 re-deletion credit (`dev/active/c639cfb5-plan.md:31-35`). |
| 10. REQ-04/REQ-06/REQ-11 and D-06 define variables as explicitly non-secret and persistable, with no secret channel. | **valid-and-open; supersedes the earlier sensitive-value finding** | V1 has no variable vocabulary and rejects the reserved reference namespace (`crates/jit/src/profile/manifest.rs:12-30`, `crates/jit/src/profile/planner.rs:278-309`). V2 must persist every resolved variable value for deterministic reconfiguration, expose precedence and reference rendering, audit names/source kinds without values, and test that no secret declaration, input, interpolation, resupply, or special redaction surface exists (`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json:4`). |
| 11. Manifest v1 bytes and hashes remain stable while v2 decodes into the canonical model. | **valid-and-open; compatibility-defining** | The dogfood manifest is explicitly v1 (`profiles/jit-dogfood/manifest.toml:1-5`), package identity hashes canonical manifest serialization plus exact declared source bytes, and tests already pin a v1 fixture hash (`crates/jit/src/profile/package.rs:453-526`, `crates/jit/src/profile/package.rs:657-669`). Introduce version-specific wire decoding followed immediately by one normalized package; do not rewrite the dogfood tree or run v1/v2 planners. |
| 12. Canonical applied records require full closure, and decoder/model/runtime/public/docs work is one vertical cutover after `cdc840ad` reaches Done. | **valid-and-open; breakdown constraint** | The v1 record is closed but minimal (`crates/jit/src/profile/application.rs:7-21`). After re-investigating the actual cdc landing, the final decoder/model/resolver/records are introduced and used only in the same task that switches runtime/init, schema/MCP/event/docs, exact tests, and truly surviving-path deletion; no separately green foundation exists (`dev/active/c639cfb5-plan.md:26-35`, `dev/active/c639cfb5-plan.md:48-62`). |
| 13. Template, canonical-record, and current-event wire details are fixed. | **chosen planning decisions; not current code facts** | Manifest v2 asset/region `template` is boolean/default false; placeholders follow the sole bounded semantic-string rules; canonical records are schema v2; and `profile_lifecycle_changed` uses the exact operation/action/migration/no-event rules recorded below. These shapes land only in the vertical cutover and must not be cited as released v1 behavior. |

## Current system by contract

The code citations below establish the released v1 package, record, command, and public
behavior. Deletion ownership will be evaluated against the eventual `cdc840ad` postcondition,
not against every pre-cdc symbol visible in those citations. Because `cdc840ad` remains
Backlog, its plan supplies expected postconditions only. Once it is Done, v1.1 must
re-investigate the actual tree before deciding which cited predecessor still exists and is
therefore v1.1 cleanup work.

### Package model and discovery

`ProfileManifest` is a strict v1 wire type with metadata, semantic contributions,
ordinary assets, and append-placed regions. `#[serde(deny_unknown_fields)]` is used on
the manifest and its nested declarations, so future lifecycle fields cannot be added
accidentally (`crates/jit/src/profile/manifest.rs:12-30`,
`crates/jit/src/profile/manifest.rs:32-44`,
`crates/jit/src/profile/manifest.rs:178-211`). The runtime JSON Schema is generated
from that same type (`crates/jit/src/profile/manifest.rs:213-216`,
`crates/jit/src/schema.rs:731-738`).

V2 should not mutate `ProfileManifest` in place and then deserialize v1 through defaults.
The manifest discriminator already exists at `profile.manifest-version`, and the validator
currently accepts only version 1 (`crates/jit/src/profile/manifest.rs:32-44`,
`crates/jit/src/profile/package.rs:233-242`). The canonical design is closed `V1` and `V2`
wire types selected by that discriminator, each strictly validated, followed by immediate
normalization into one source-neutral package model consumed by resolution, composition,
ownership, and publication. This is durable package-format support, not two lifecycle
engines.

V1 hashing must remain its own frozen compatibility rule. Today the package hash frames
canonical serialization of the parsed v1 manifest and exact bytes of every declared source;
target hashes frame ordered operations plus exact source bytes
(`crates/jit/src/profile/package.rs:453-526`). Therefore, do not reserialize the normalized
v2-capable model when hashing v1, add lifecycle defaults to the v1 wire shape, reorder the
dogfood manifest, or reauthor it as v2. Pin the current dogfood package hash and complete
target-hash map before refactoring, alongside the already-pinned synthetic v1 fixture hash
(`crates/jit/src/profile/package.rs:657-669`). V2 may define its own normalized hashing
contract, but v1 input must produce byte-for-byte source identity and the same hashes.

The only public package constructor consumes an `include_dir::Dir`; its byte map borrows
embedded static slices (`crates/jit/src/profile/package.rs:33-67`). The only production
resolver loads one statically embedded directory and matches one ID
(`crates/jit/src/profile/dogfood.rs:11-45`,
`crates/jit/src/commands/profile.rs:330-347`). Therefore, “one package model” cannot mean
merely adding a second local-package planner beside `EmbeddedProfilePackage`. The clean
extension is a source-neutral validated package image, with embedded and explicit-local
source readers inside one resolver producing the same canonical immutable domain image.

Package bounds, stable IDs, semantic versions, compatible JIT requirements, content
declarations, unique sources/targets, and canonical paths are validated before planning
(`crates/jit/src/profile/package.rs:216-311`,
`crates/jit/src/profile/package.rs:391-417`). Local loading adds filesystem-specific work
that embedded loading does not need: no-follow traversal, symlink rejection, bounded reads,
manifest-at-root enforcement, and an immutable captured image before pure parsing. That
I/O belongs in storage/command boundaries, not in manifest or composition functions
(`AGENTS.md:139-146`).

The public source grammar is now frozen: every selecting command accepts one repeatable
`--profile SOURCE`, where `SOURCE` is exactly `id:ID` or `path:DIR`. One ordered
`ProfileSelector` collection carries the interleaved occurrence stream through CLI,
generated schema, MCP, command requests, results, and tests. Separate ID/path vectors,
untagged values, index reconstruction, implicit search, and remote discovery are rejected
(`dev/active/c639cfb5-plan.md:26-27`, `dev/active/c639cfb5-plan.md:103-105`).

### Semantic merge and multi-profile gap

The one-package planner loads TOML through `toml_edit`, preserves original bytes on a
semantic no-op, and appends missing values while retaining existing structure
(`crates/jit/src/profile/planner.rs:415-472`). It compares map, keyed-array, and projection
identities structurally and returns `ContributionConflict` on difference
(`crates/jit/src/profile/planner.rs:501-552`,
`crates/jit/src/profile/planner.rs:597-659`,
`crates/jit/src/profile/planner.rs:670-683`). Set-string contributions are idempotent
(`crates/jit/src/profile/planner.rs:566-595`).

Those semantics are reusable, but sequentially applying several packages is not a valid
composition implementation. It would publish intermediate states, record only the last
package, and make conflict attribution and shared ownership depend on application order.
Composition should first resolve one graph, index all contributions by semantic identity,
deduplicate structurally identical definitions while retaining every owner, and reject
differing definitions before repository materialization. User selection order may remain
stable in request/result presentation and dependency tie-breaking where semantics are
equal; it must never select a winner for differing definitions.

The generic registry-projection engine already composes several configured regions aimed
at one target by threading pending bytes (`crates/jit/src/validation/projection.rs:330-365`)
and tests one final target containing both results
(`crates/jit/src/validation/repository.rs:1339-1409`). That is evidence for the strict
managed-document/materialization engine planned by Backlog `cdc840ad`. V1.1 waits for its
actual landing, verifies the resulting engine, then consumes it without adding or claiming
deletion of a profile-specific shared-target loop.

### Explicitly non-secret, persistable variables

V1 has no variable fields and rejects the reserved interpolation namespace in all asset,
region, and contribution data (`crates/jit/src/profile/planner.rs:18-21`,
`crates/jit/src/profile/planner.rs:278-309`). The v1.1 manifest therefore needs an explicit
versioned variable declaration rather than interpreting arbitrary shell/template syntax.

The requested precedence is observable and testable: manifest default, values file,
manifest-declared environment variable, then repeatable command-line `--set`. Every
declared variable is explicitly non-secret, and its resolved value is persisted in the
canonical applied record so reconfiguration is deterministic. The same declared reference
syntax applies to supported semantic values and generated text; no second interpolation
engine is permitted.

The exact shapes are frozen by the current plan. Manifest v2 permits only strict
`[[variable]]` rows with required string `name` and optional string `default` and `env`;
unknown fields and duplicate names fail, and `name`/`env` use
`[A-Z][A-Z0-9_]*`. There is at most one `--values-file PATH`, whose TOML is exactly a
`[variables]` string map, plus repeatable `--set NAME=VALUE`; `--set` splits at the first
`=`, empty strings are values, undeclared names and duplicate values within one tier fail,
and the source-kind enum is `default|values_file|environment|set`
(`dev/active/c639cfb5-research.md:391-428`,
`dev/active/c639cfb5-plan.md:26-29`).

`{{jit:var:NAME}}` is the only template form. It is a single non-recursive pass over string
semantic leaves and UTF-8 asset/region bodies whose declarations explicitly opt into
templating. V1 content, identifiers, versions/ranges, dependency fields, contribution
identities, paths, marker IDs, modes, executable flags, binary assets, and unmarked bodies
cannot contain references; loops, conditionals, indirection, nested languages, malformed
or unknown references, and non-string values fail closed. Package identity covers the
unresolved package bytes; plan and ownership-claim identities cover the sorted resolved
value map and exact final semantic/materialized bytes
(`dev/active/c639cfb5-plan.md:27-29`).

This is not a partial secret feature. There is no secret flag in the manifest, `--secret`
or hidden prompt/input channel, secret reference form, ephemeral wrapper, redaction/resupply
lifecycle, or secret-specific audit representation. A manifest may name an environment
variable as a source for an explicitly non-secret profile value. Separately, package
content may store an environment-variable *name* as ordinary configuration text; the
profile engine must not infer that string as an instruction to dereference a runtime
secret. Audit events record variable names and source kinds, never resolved values, as the
minimal audit contract in REQ-06—not as a secret-value escape hatch.

### Frozen v1.1 wire decisions (planning choices, not current code facts)

The following shapes are fixed for the vertical-cutover plan. Current v1 code does not yet
implement them, and cdc's Backlog status does not change their public meaning:

- Manifest v2 asset and region declarations each carry `template: bool`, defaulting to
  `false`. Only `template = true` opts a UTF-8 asset/region body into placeholder parsing;
  unmarked bodies remain literal, and binary content cannot be templated.
- `{{jit:var:NAME}}` is the only semantic/text placeholder. Every `{{jit:` occurrence in
  an allowed string semantic leaf or opted-in body must parse as exactly one or more valid
  declared placeholders. Rendering is one non-recursive pass and inserts resolved strings
  verbatim. Placeholders remain forbidden in IDs, versions/ranges, dependency or
  incompatibility fields, contribution identities, paths, marker IDs, placement, modes,
  executable flags, non-string semantic values, and unmarked bodies.
- The canonical applied-profile record is record v2 (`record_version: 2`). Ordinary record
  loading accepts only v2; the five-field v1 shape remains private migration input only.
- The sole current audit tag is `profile_lifecycle_changed`. Its
  `requested_operation` is exactly `apply|upgrade`; initialization records `apply`.
  Each profile entry carries exactly one action from
  `installed|unchanged|reconfigured|upgraded`.
- An all-unchanged operation emits no lifecycle event. A changed aggregate event may still
  identify an included profile as `unchanged`. Dry-run emits and persists no lifecycle
  event.
- `record_migrations` is sorted by `profile_id`; each entry is exactly
  `{profile_id, from_version: 1, to_version: 2}`. Variable audit data remains sorted name/source-kind pairs
  with no resolved values or rendered bytes.

These decisions refine the broader strict-template, record-migration, and one-current-event
choices in the reviewed plan (`dev/active/c639cfb5-plan.md:26-35`). Decoder/model, record,
event, output/schema/MCP, exact tests, and canonical docs must expose them in the same
vertical cutover.

### Applied records, ownership, and migration

The v1 record is intentionally minimal and has no wire-version field
(`crates/jit/src/profile/application.rs:7-21`). Application treats any non-exact record as
a conflict, rather than migrating it (`crates/jit/src/commands/profile.rs:391-416`). Fresh
init and existing-repository apply both create the same record
(`crates/jit/src/commands/init.rs:337-404`,
`crates/jit/src/commands/profile.rs:248-300`).

A durable schema migration is categorically different from indefinite compatibility
branching. The ordinary v1.1 record reader should accept only the canonical record's
explicit `record_version`, rejecting unknown fields and non-canonical versions. Before
normal profile service construction, a migration boundary may detect the unversioned
five-field v1 image and invoke the sole legacy decoder. That decoder has no serializer and
does not return a legacy/canonical union to composition, ownership, or publication code.
It proves a canonical replacement, transactionally persists it, and only then permits
ordinary planning to begin. If mutation is not authorized, read-only commands report a
typed migration-required result rather than interpreting v1 records in memory.

The existing repository has a visible one-time migration command pattern
(`crates/jit/src/commands/migrate.rs:1-81`), but its per-issue loop is not sufficiently
atomic for cross-record profile ownership. The profile migration must use the recoverable
file transaction instead. Similarly, the repository index accepts older shapes through
serde defaults (`crates/jit/src/storage/json.rs:123-140`); REQ-12 explicitly rules out
copying that fallback pattern into ordinary profile record loading. Migration should:

1. accept only the released five-field record for embedded `jit-dogfood` and match its ID,
   version, embedded origin, package hash, and complete target-hash map to the unchanged
   byte/hash-pinned embedded package in the running binary;
2. reject `path:DIR`, local, archived-copy, inferred-base, network, or user-supplied evidence
   even when bytes or IDs appear equivalent;
3. reconstruct that authenticated package's semantic/file contribution identities;
4. compare each current repository contribution with the authenticated definition and
   create baseline-retained claims (`retain_if_unowned = true`) only for exact matches,
   because v1 cannot prove exclusive creation;
5. publish every canonical record/ownership row and migration audit marker in the same
   recoverable transaction as the requested lifecycle operation, then be idempotent on
   retry; or
6. return an actionable ambiguity/drift conflict without writes when exact evidence is
   unavailable.

This embedded-only evidence restriction is deliberate. The still-shipped pinned v1 package
is the only historical base that can be authenticated without adding a second inventory.
If it is unavailable or any hash/current unit differs, migration fails closed and names the
required ID, version, package hash, and target hashes
(`dev/active/c639cfb5-research.md:123-219`,
`dev/active/c639cfb5-plan.md:29-31`).

Package target hashes describe authored contribution frames, not the repository's
post-merge per-key base state (`crates/jit/src/profile/package.rs:453-521`). They cannot by
themselves justify deleting or overwriting one current semantic entry. New ownership rows
need stable contribution identity, owner set, and base fingerprint at file or semantic-key
granularity. Those rows remain evidence: current behavior still derives from repository
registries, and a missing ownership record must not make declared configuration disappear.

The canonical applied-record set must be closed after every migration and mutation. Its
chosen wire is record v2 (`record_version: 2`), and its records must account for each
applied profile's
origin, package version and compatibility, package identity, complete resolved non-secret
input map, every owned file or semantic identity, the corresponding base fingerprint, and
the complete owner set for shared identities. Closure requires that every owner names an
existing canonical profile record, every contribution the engine claims is represented
exactly once at its ownership granularity, and shared owner sets agree wherever represented.
No optional legacy fields, central duplicate ownership index, orphan sidecar, event replay,
or partially migrated row may be needed to reconstruct the canonical evidence. Canonical
per-profile claims and cross-record validation must prove the closure invariant
(`dev/active/c639cfb5-plan.md:29-31`, `dev/active/c639cfb5-plan.md:106-110`).

Every operation evaluates selected candidates alongside every surviving unselected owner,
dependency, dependent, and incompatibility from that canonical closure. Equal resolved
fingerprints retain all owners; differing selected or surviving bases conflict. Upgrade
must not invalidate an unselected dependent's version range or introduce an incompatibility
with a survivor (`dev/active/c639cfb5-plan.md:29-30`).

Three-way upgrade decisions can then compare recorded base fingerprint, current semantic
value, and candidate value. Only `current == base` is untouched; `current != base` plus a
candidate change is a conflict. Removal is allowed only when the item is unchanged and the
upgraded profile is its sole owner. Shared entries lose one owner but remain. This
operation is upgrade replacement, not a general removal command.

Old `ProfileApplied` events remain readable append-only history; event decoding is not a
competing mutation path. V1.1 current mutations emit only `profile_lifecycle_changed`;
ordinary `profile_applied` construction/appending is deleted while the historical tag,
parser, and torn-tail handling remain decodable for audit continuity
(`dev/active/c639cfb5-plan.md:34-35`).

### Audit and public results

The only current audit event is `ProfileApplied`, carrying package-level identity and a
torn-tail marker (`crates/jit/src/domain/types.rs:1468-1490`). Event parsing has a special
rule that accepts a torn predecessor only when the immediately following event is that
variant with `isolated_torn_tail = true`
(`crates/jit/src/domain/event_log.rs:35-89`). The fixed
`profile_lifecycle_changed` cutover must update the closed `Event`, `EventTag::ALL`, catalog
description/sample, tag mapping, schema, parser exception, generated event reference, and
tests together
(`crates/jit/src/domain/event_catalog.rs:117-170`,
`crates/jit/src/domain/event_catalog.rs:249-252`,
`crates/jit/src/domain/event_catalog.rs:403-415`,
`crates/jit/src/domain/event_catalog.rs:440-450`). Audit payloads should use profile IDs,
versions, operation kind, variable names and source kinds, and change/conflict counts or
identities; they must never copy resolved variable values. The planned exact event wire is
the frozen contract above: `requested_operation = apply|upgrade` (`init` maps to `apply`),
per-profile `installed|unchanged|reconfigured|upgraded`, sorted v1→v2
`record_migrations`, no all-unchanged event, and no dry-run event.

Current result types expose package-level list/show/apply/plan shapes
(`crates/jit/src/profile/application.rs:32-174`), and CLI dispatch formats human output
directly in `main.rs` (`crates/jit/src/main.rs:2073-2185`). The vertical cutover removes
that one-ID result family in favor of count-wrapped deterministic collection results over
repeated ordered `id:ID|path:DIR`; dry-run and mutation may remain an honest operation union,
not a single-versus-multiple compatibility union (`dev/active/c639cfb5-research.md:519-549`).

## Primitive verification

| Property likely asserted by the plan | Verdict | Verified behavior |
|---|---|---|
| Package parsing is source-neutral today. | **contradicted** | `EmbeddedProfilePackage` borrows `include_dir` bytes, and `from_files` is private (`crates/jit/src/profile/package.rs:33-67`). Parsing/validation semantics are reusable, but one canonical source-neutral package image produced by the resolver is open work; a separate local-package adapter exposed to planning would violate REQ-12. |
| Adding v2 fields directly to the v1 manifest type preserves v1 identity. | **contradicted** | V1 rejects unknown fields, validates one exact manifest version, and hashes canonical serialization of that wire object (`crates/jit/src/profile/manifest.rs:12-44`, `crates/jit/src/profile/package.rs:233-242`, `crates/jit/src/profile/package.rs:507-516`). Version-specific strict wire types must normalize into one model, while v1 hashing remains frozen. |
| Package path validation rejects lexical escapes. | **confirmed** | Empty, absolute, traversal, backslash, drive/colon, control-character, and noncanonical paths fail (`crates/jit/src/profile/package.rs:391-417`). |
| Package validation alone makes local directory loading symlink-safe. | **contradicted** | Manifest paths are checked lexically; the existing embedded loader does no live directory walk (`crates/jit/src/profile/package.rs:191-214`). Local loading needs capability-confined, no-follow capture before pure validation. |
| The released one-package planner established useful overlay semantics. | **confirmed historical input; removal is assigned to Backlog cdc** | The v1 planner uses an immutable view and exact target validation (`crates/jit/src/profile/planner.rs:132-150`, `crates/jit/src/profile/planner.rs:227-275`). The cdc plan expects `ProfileApplicationPlan`, `PlannedTarget*`, and `plan_profile_application_against` to be absent. V1.1 waits for Done, verifies actual absence, and receives no deletion credit for paths cdc really removed (`dev/active/c639cfb5-plan.md:31-34`). |
| Re-running the old one-package planner implements multi-profile composition. | **contradicted and structurally forbidden** | It accepts one package and record path (`crates/jit/src/profile/planner.rs:132-150`). The cdc plan is expected to remove that route; after cdc Done, v1.1 rechecks the tree and must neither recreate nor wrap it. |
| Current semantic merge is idempotent and rejects differing same-key definitions. | **confirmed** | Equal map/keyed/projection values no-op, existing set strings no-op, and differing definitions return `ContributionConflict` (`crates/jit/src/profile/planner.rs:501-683`). |
| Released profile application proves coupled record/event publication. | **confirmed current guarantee; replacement is assigned to Backlog cdc** | V1 holds repository/event locks and puts targets, record, and event in one file transaction (`crates/jit/src/commands/profile.rs:137-220`). The cdc plan is expected to replace command-local builders with one mutation session. V1.1 waits for that actual capability, then feeds it rather than reproducing or claiming cdc's deletion (`dev/active/c639cfb5-plan.md:31-34`). |
| The transaction kernel provides instantaneous multi-file filesystem atomicity. | **contradicted** | It publishes identity-checked actions sequentially after a durable prepared journal; failures roll back or retain recovery state (`crates/jit/src/storage/file_transaction.rs:107-176`, `crates/jit/src/storage/file_transaction.rs:492-593`). The public contract correctly calls this recoverable publication (`docs/reference/profiles.md:95-111`). |
| The transaction kernel supports a complete multi-profile file delta without a new journal protocol. | **confirmed** | `FileTransactionPlan` already accepts a deterministic action vector (`crates/jit/src/storage/file_transaction.rs:34-41`), normalizes paths/directories and rejects duplicate targets (`crates/jit/src/storage/file_transaction.rs:856-920`). v1.1 should feed it one aggregate materialization delta. |
| Recovery is available even when repository caches are invalid. | **confirmed** | The coordinator reads only transaction protocol state, not config/rules/schema (`crates/jit/src/storage/recovery_coordinator.rs:1-41`), and its tests exercise invalid caches (`crates/jit/src/storage/recovery_coordinator.rs:251-264`). |
| Existing record hashes are enough for three-way per-key upgrade. | **contradicted** | The record has only package/target hashes (`crates/jit/src/profile/application.rs:7-21`); target hashes frame authored operations by whole target (`crates/jit/src/profile/package.rs:453-521`). Per-key/file base fingerprints and migration proof are open. |
| Independent per-profile ownership rows are sufficient without a closure check. | **contradicted** | Shared ownership spans profiles, while the current record has no ownership vocabulary at all (`crates/jit/src/profile/application.rs:7-21`). Canonical validation must reject missing profile records, orphan owner references, absent contribution rows, duplicate ownership identities, inconsistent shared owner sets, and partial migrations before publication. |
| A serde union/default-based reader for old and new records counts as canonical migration. | **contradicted** | The current applied record is an exact unknown-field-denying five-field type (`crates/jit/src/profile/application.rs:7-21`), while older repository indexes are accepted through defaults (`crates/jit/src/storage/json.rs:123-140`). REQ-12 requires an isolated one-way decoder that durably rewrites canonical versioned records, not that index-style fallback in normal lifecycle reads. |
| The current profile projection drift helper is a lifecycle ownership engine. | **contradicted; planned cdc removal** | It compares a dedicated projection tree rather than semantic ownership (`crates/jit/src/profile/drift.rs:42-51`). The cdc plan assigns removal of `profile::drift` and duplicate projection vocabulary. After cdc Done, v1.1 verifies what is absent and implements drift over canonical claims without restoring it (`dev/active/c639cfb5-plan.md:31-34`). |
| Managed-target composition/publication is v1.1 work. | **contradicted; expected `cdc840ad` prerequisite** | Backlog cdc plans one strict managed-document engine, canonical image/materializer, exact delta/storage capability, and publisher. V1.1 cannot start its implementation until those postconditions are delivered and inspected (`dev/active/c639cfb5-plan.md:31-36`). |
| Current profile operations are Git-independent. | **confirmed** | Fresh and existing application are tested outside Git (`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:232-299`), and worktree identity is optional after repository publication (`crates/jit/src/commands/mod.rs:1032-1070`). Keep local package and upgrade paths inside the same boundary. |

## Planned post-cdc prerequisite and provisional v1.1 cutover inventories

### Cdc-planned removals: verify after Done, then assert absence

The cdc plan assigns removal of `PackageProjection`, `ProjectedFile*`,
`profile/render.rs`, `RepositorySnapshot*` and snapshot capture, `profile::drift`,
`PresetProjection` and duplicate projection inventory, `ProfileApplicationPlan`,
`PlannedTarget*`, `plan_profile_application_against`, the `InitScaffold` final-target
inventory, and command-local profile/init transaction builders. It also owns the canonical
repository image, strict managed-document engine, exact delta/storage capability,
materializer, mutation session, and sole recoverable publisher. These are expected
postconditions, not present facts: `cdc840ad` is Backlog. V1.1 implementation must wait for
Done, re-investigate the actual delivered tree and capability contracts, subtract paths
cdc truly removed from its own inventory, assert those names/routes remain absent, and
reject recreation; none of cdc's actual removals becomes v1.1 deletion credit
(`dev/active/c639cfb5-plan.md:31-34`, `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md:63-88`).

### Provisional surviving predecessors: rebase after cdc Done, then delete vertically

| Surviving predecessor | Canonical replacement and same-change proof |
|---|---|
| Public `EmbeddedProfilePackage`, its re-export, and the embedded-only package lookup/resolver | One strict v1/v2 decoder immediately returns the source-neutral package model, and one resolver accepts ordered `id:ID|path:DIR` selectors. Remove the embedded-only type/export/lookup rather than wrapping it (`dev/active/c639cfb5-plan.md:26-34`). |
| Embedded-only list/show/plan/apply/validate methods plus `validate_profile_id` | One aggregate lifecycle service backs every verb. Remove source-specific methods and direct callers in `commands/profile.rs`, profiled init, `main.rs`, server/direct callers, and tests in the same cutover (`dev/active/c639cfb5-plan.md:33-34`). |
| Single-ID init/profile request, dispatch, output, schema, and MCP shape | One repeated ordered `--profile id:ID|path:DIR` occurrence and collection result contract travels unchanged through CLI, schema, MCP, requests, results, tests, and current docs. Separate ID/path channels and single-versus-many result unions do not survive (`dev/active/c639cfb5-plan.md:26`, `dev/active/c639cfb5-research.md:490-551`). |
| Five-field `AppliedProfileRecord` as the ordinary current type, its loader/matcher/constructor, and the surviving v1 provenance/result family | Ordinary reads accept only the closed canonical records. Keep the five-field shape solely as the private embedded-pinned migration input; delete `ProfileApplicationStatus`, `ProfileApplicationWarning`, `ProfileApplyResult`, `ProfileListResult`, `ProfileShowResult`, `ProfileTargetAction`, `ProfileTargetChange`, `ProfilePlanStatus`, and `ProfilePlanResult` as surviving public predecessors (`dev/active/c639cfb5-plan.md:31-34`). |
| Current `Event::new_profile_applied` constructor and profile event-image appender/emitter | Current mutations emit only `profile_lifecycle_changed` through the cdc audit-seed/materialization path, with sorted variable name/source-kind pairs and no values. Remove current construction/emission; retain only historical `ProfileApplied` tag/parser and torn-tail decoding (`dev/active/c639cfb5-plan.md:34-35`). |
| Current embedded-only/apply-only/minimal-record CLI, JSON, schema, MCP, exact-test, and adopter-doc contracts | Runtime/init, outputs, generated surfaces, current event, exact tests, navigation, and canonical docs switch in the same green task. Preserve archive/presentation history; no later public/docs cleanup owns contract changes (`dev/active/c639cfb5-plan.md:33-35`, `dev/active/c639cfb5-plan.md:48-62`). |

The unchanged `profiles/jit-dogfood` v1 package and package-derived workflow inventory are
preserved, not deleted or copied. Strict v1 package decoding is a permanent wire contract;
only its embedded-only runtime wrapper is superseded.

Historical `ProfileApplied` events are not a superseded resolver or publisher. Append-only
audit requires their decoder to remain. The cutover criterion is that ordinary mutation
emits only the chosen canonical lifecycle event contract; historical decoding cannot be
used to justify a second current-state path.

## Repository-wide acceptance checks for the canonical cutover

1. Freeze the existing dogfood manifest and every declared source byte, package hash, and
   target hash. Strict v1 decoding through the normalized package model reproduces those
   hashes exactly; a v2 fixture exercises new fields through the same resolver/composer.
2. Check in a v1 repository fixture with the exact five-field applied record. Migration
   authenticates only the unchanged embedded pinned dogfood ID/version/origin/package and
   target hashes, writes baseline-retained canonical claims, and rejects path/local/archive/
   inferred evidence. Retry is a no-op; ambiguity or drift fails before any write.
3. Validate full record closure after migration, application, reconfiguration, and
   upgrade: all applied profiles and resolved non-secret inputs are present; every owned
   contribution has one base and complete existing owner set; no orphan reference,
   duplicate identity, inconsistent shared owner set, or partial canonical record remains.
4. After the migration boundary, list/show/validate/diff/reapply/upgrade and initialization
   accept only canonical records. A read-only invocation against unmigrated state returns
   migration-required; no command silently creates an in-memory compatibility model.
5. Block v1.1 implementation while `cdc840ad` is not Done. After Done, re-investigate its
   actual tree/capabilities, rebase the provisional inventory, assert cdc-owned
   projection/final-byte/snapshot/marker/materialization/transaction predecessors it truly
   removed remain absent, and delete only the embedded resolver, ordinary v1 loader,
   one-ID command/result/current-emitter/public paths that actually survive.
6. Exercise one resolver contract with interleaved repeated `--profile id:ID` and
   `--profile path:DIR` selectors and prove both sources produce the same canonical package
   type, validation, hashes, results, and errors while occurrence order is preserved only
   for request/result presentation.
7. Prove multiple selections and the complete surviving applied closure yield one aggregate
   semantic/ownership input to the cdc materializer and sole publisher. Differing
   definitions fail before repository/record writes; equal definitions retain all owners
   independent of selection order.
8. Exercise the frozen variable grammar exactly: strict manifest-v2 `[[variable]]`
   `name/default/env`, one optional TOML `[variables]` string-map values file, repeatable
   `--set NAME=VALUE`, precedence default → values file → environment → set, and sole
   `{{jit:var:NAME}}` single-pass rendering in allowed string/opted-in text positions.
   Asset/region `template` is boolean and defaults false. Persist values/source kinds,
   audit sorted names/source kinds without values, and reject every alternate placeholder,
   declaration, input shape, templating position, or secret surface.
9. Make the sole implementation cutover depend on `cdc840ad` reaching Done. Decoder/model,
   resolver/graph/composer, variables, canonical records/migration, ownership/upgrade,
   runtime/init, surviving-path deletion, CLI/schema/MCP/current event, exact tests, and
   current docs land together. No earlier foundation or later contract cleanup is green.
10. Assert generated CLI schema/MCP/current docs contain repeated `id:ID|path:DIR`, the
    canonical record-v2/result/event shapes, and frozen variable inputs. The event schema
    fixes requested `apply|upgrade`, per-profile
    `installed|unchanged|reconfigured|upgraded`, sorted
    `{profile_id, from_version: 1, to_version: 2}`
    migrations, init-as-apply, and no all-unchanged/dry-run event, while excluding the
    ordinary five-field reader, one-ID result family, current `profile_applied` emitter,
    temporary migration commands, and origin-specific lifecycle variants.
11. Assert `profiles/jit-dogfood` remains the sole authored dogfood package and preset/live
    projection behavior continues to derive from it; no duplicate manifest/assets/regions
    or workflow inventory is introduced.
12. Search current public documentation for stale “one embedded only,” “apply only,” and
    “minimal record” claims after cutover. Keep historical plan/archive/presentation claims
    unchanged and label them through their existing historical context.
13. Run local-package traversal/symlink/path-security tests and the existing Git-free,
    Windows, macOS, and Linux acceptance paths through the same resolver and lifecycle
    service, followed by profile unit/harness/CLI suites, transaction failure injection,
    generated schema/MCP tests, `cargo clippy --workspace --all-targets`, formatting, and
    docs mechanical/projection checks.

## Complete consumer sweep

This sweep covers direct consumers of the manifest/package, applied-record/ownership,
profile result/CLI/schema, and audit contracts. Generic uses of “profile” for Cargo build
profiles or arbitrary issue labels are intentionally excluded. Pre-cdc paths are retained
here only as historical blast-radius evidence; the ownership labels below determine whether
v1.1 migrates a surviving consumer or merely asserts a cdc-owned predecessor stays absent.

### Runtime and library consumers

- Surviving profile package/record/domain consumers to migrate in v1.1:
  `crates/jit/src/profile/mod.rs:1-49`,
  `crates/jit/src/profile/manifest.rs:6-216`,
  `crates/jit/src/profile/package.rs:1-93`,
  `crates/jit/src/profile/application.rs:1-194`,
  `crates/jit/src/profile/dogfood.rs:1-155`,
  and the surviving package-derived preset consumers in
  `crates/jit/src/profile/preset.rs:1-122`.
- Cdc-planned profile predecessors that v1.1 must recheck after cdc Done and, when absent,
  assert rather than migrate/delete:
  `profile/planner.rs` public final-byte plan vocabulary, `profile/render.rs`,
  `profile/snapshot.rs`, `profile/drift.rs`, duplicate preset projection inventory, and
  their exports/callers (`dev/active/c639cfb5-plan.md:31-34`).
- Command orchestration and exports: `crates/jit/src/commands/profile.rs:1-347`,
  `crates/jit/src/commands/init.rs:1-32`,
  `crates/jit/src/commands/init.rs:337-430`,
  `crates/jit/src/commands/mod.rs:40-85`, and
  `crates/jit/src/lib.rs:20-29`.
- CLI, presentation, errors, and generated schema:
  `crates/jit/src/cli.rs:39-57`, `crates/jit/src/cli.rs:2821-2903`,
  `crates/jit/src/main.rs:65-86`, `crates/jit/src/main.rs:738-765`,
  `crates/jit/src/main.rs:2073-2185`,
  `crates/jit/src/output.rs:677-694`,
  `crates/jit/src/schema.rs:475-495`, and
  `crates/jit/src/schema.rs:731-738`.
- Applied-state and audit vocabulary: `crates/jit/src/domain/types.rs:1175-1181`,
  `crates/jit/src/domain/types.rs:1468-1490`,
  `crates/jit/src/domain/types.rs:1764-1783`,
  `crates/jit/src/domain/event_log.rs:35-89`, and
  `crates/jit/src/domain/event_catalog.rs:111-170`,
  `crates/jit/src/domain/event_catalog.rs:249-252`,
  `crates/jit/src/domain/event_catalog.rs:403-415`,
  `crates/jit/src/domain/event_catalog.rs:440-450`.
- Cdc-planned storage/materialization seams that v1.1 may consume only after Done and
  verification, without replacing them:
  `crates/jit/src/storage/file_transaction.rs:34-176`,
  `crates/jit/src/storage/recovery_coordinator.rs:1-41`,
  `crates/jit/src/validation/projection.rs:330-384`, and
  `crates/jit/src/validation/repository.rs:1339-1409`; the former JSON profile snapshot
  capture at `crates/jit/src/storage/json.rs:261-300` belongs to the cdc absence inventory.
- Package-derived preset compatibility:
  `crates/jit/src/gate_presets/builtin.rs:17-52`,
  `crates/jit/src/gate_presets/planning.rs:23-95`, and
  `crates/jit/src/gate_presets/reference.rs:252-261`. These are SSOT-sensitive:
  `jit-dogfood` must remain one authored package inventory.
- Dependency/build integration: `crates/jit/Cargo.toml:40-45` and the production
  package root `profiles/jit-dogfood/manifest.toml:1-150` (continuing through its
  assets/regions inventory). Local packages should use the same wire model without
  moving or duplicating this authored tree.

### Tests and fixtures

- Unit tests are colocated in every profile module above; package fixtures are
  `crates/jit/tests/fixtures/profile-packages/synthetic-valid/manifest.toml:1-77`,
  `crates/jit/tests/fixtures/profile-packages/planner-asset-only/manifest.toml:1-9`,
  and
  `crates/jit/tests/fixtures/profile-packages/planner-invalid-interpolation/manifest.toml:1-9`.
  The synthetic fixture also owns nested assets, executable files, and managed-region
  bytes under the same fixture directories.
- CLI behavior: `crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs:24-190`.
- Cross-platform/Git-free/profile-to-plan acceptance and exact schema shape:
  `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:232-299`,
  `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:301-513`, and
  `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:516-639`.
- Generated CLI schema inventory:
  `crates/jit/tests/cli_repo_workflow/integration_schema.rs:59-89` and the runtime
  manifest-schema freshness test `crates/jit/src/schema.rs:1312-1323`.
- Cross-platform CI invokes the acceptance suite at
  `.github/workflows/ci.yml:181-209`.

### MCP consumers

- Tool curation owns current descriptions at `mcp-server/curated-tools.json:53-55`.
- Generated tool inventory, deferred-command exclusions, exact arguments, and end-to-end
  calls are asserted at `mcp-server/test-integration.js:198-243` and
  `mcp-server/test-integration.js:332-374`.
- The MCP server loads the generated command schema generically through
  `mcp-server/lib/schema-loader.js:1-32`; new profile commands should remain generated from
  `jit --schema`, with curation only for the intentionally exposed tool set.

### Public documentation and navigation consumers

- Canonical contract: `docs/reference/profiles.md:1-59`,
  `docs/reference/profiles.md:78-142`. This page should remain the single adopter-facing
  owner for lifecycle semantics.
- CLI and grammar: `docs/reference/cli-commands.md:475-600` and
  `docs/reference/cli-command-grammar.md:80-88`.
- Storage and events: `docs/reference/storage-format.md:26-51` and
  `docs/reference/events.md:21-47`.
- Package-derived workflow references:
  `docs/reference/gate-presets.md:17-25`,
  `docs/reference/configuration.md:6-10`, and
  `docs/reference/jit-content-standards.md:7-13`.
- Entry points and tutorials: `README.md:68-96`, `README.md:231-236`,
  `INSTALL.md:73-113`, `INSTALL.md:306-315`,
  `docs/index.md:47-69`,
  `docs/tutorials/quickstart.md:83-93`,
  `docs/examples/README.md:6-10`,
  `docs/concepts/planning-bracket.md:11-15`,
  `docs/how-to/adopt-planning-bracket.md:12-16`, and
  `docs/how-to/deployment.md:95-106`.
- Repository guidance names the shipped/local dogfood boundary at `AGENTS.md:88-106`.
  Any new volatile package fields or command inventory should be linked to the canonical
  profile reference or projected, not copied into this guidance.

### Historical/planning consumers that must not be rewritten as current API docs

- Original complete-lifecycle contract:
  `dev/active/9b7b5f9c-jit-profiles-planning-brief.md:16-38` and
  `dev/active/9b7b5f9c-jit-profiles-planning-brief.md:57-102`.
- MVP scope decision: `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-mvp-scope-brief.md:7-30`.
- Pre-MVP investigation and implementation plan:
  `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-investigation.md:1-22`,
  `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-plan.md:22-40`, and
  `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-plan.md:181-199`.
- Publication/embedding research:
  `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-research.md:1-15` and
  `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-research.md:183-218`.
- Delivered-state authority:
  `dev/archive/9b7b5f9c-completion-report.md:11-67` and
  `dev/archive/9b7b5f9c-completion-report.md:138-171`.
- Cross-epic projection integration record:
  `dev/archive/6eb585bc-handoff-5.md:17-21` and
  `dev/archive/6eb585bc-handoff-5.md:65-70`.
- The profile presentation is a historical consumer of v1 exact output and storage
  (`dev/archive/9b7b5f9c-jit-profiles/dev/presentations/9b7b5f9c/talk.html:55-91`,
  `dev/archive/9b7b5f9c-jit-profiles/dev/presentations/9b7b5f9c/talk.html:161-169`,
  `dev/archive/9b7b5f9c-jit-profiles/dev/presentations/9b7b5f9c/talk.html:236-267`). Preserve it as v1 evidence; do not
  silently update its claims to v1.1.

## Prior-art sweep and stale assumptions

The original complete-lifecycle brief already contains the essential product decisions:
separate embedded ID and explicit local paths, dependency/variable resolution, semantic
composition without order winners, per-profile ownership, three-way upgrade, and removal
deferral (`dev/active/9b7b5f9c-jit-profiles-planning-brief.md:16-38`,
`dev/active/9b7b5f9c-jit-profiles-planning-brief.md:47-83`). It is valuable input, but its
23-criterion scope predates the bounded MVP and must not be treated as the current public
shape.

Its secret/redaction language is now specifically stale. REQ-04, REQ-06, REQ-11, and
D-06 replace it with explicitly non-secret, persistable values and an absence test for any
secret channel. Synthesis should retain precedence and reference-rendering ideas from the
brief, but must discard sensitive wrappers, redaction paths, resupply flows, and secret
audit concepts (`dev/active/9b7b5f9c-jit-profiles-planning-brief.md:20-38`,
`.jit/issues/c639cfb5-8356-4c54-b45c-860064432560.json:4`).

The MVP brief and charter explicitly moved this breadth intact to post-1.0
(`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-mvp-scope-brief.md:7-30`,
`dev/vision/9db27a3a-charter.md:138-150`). The MVP plan additionally forbade placeholder
lifecycle fields in v1 and chose minimal provenance only
(`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-plan.md:181-195`). That choice explains why migration is real work;
it is not evidence that ownership was accidentally omitted.

The old investigation's statements that no profile CLI or transaction kernel exists are
now stale (`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-investigation.md:16-36`). Its architectural boundaries and
consumer-sweep warning remain current (`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-investigation.md:499-548`).
The research note is intentionally MVP-only and says so (`dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-research.md:1-9`);
its choice of recoverable journal and compile-time embedding applies to the v1 embedded
source, not to local-package discovery.

The completion report is the strongest delivered-state prior art. It confirms the exact
apply-only boundary and assigns lifecycle work to `c639cfb5`
(`dev/archive/9b7b5f9c-completion-report.md:93-103`,
`dev/archive/9b7b5f9c-completion-report.md:138-157`). The later core handoff records an
important integration not present in the original profile plan: projection contributions
are now named `[projection.<name>]` entries and shared-target rendering is generic
(`dev/archive/6eb585bc-handoff-5.md:17-21`). Any synthesis that restores the old singleton
projection model is stale.

The Backlog `cdc840ad` plan goes further and assigns the canonical repository image,
managed-document engine, materializer, delta/session/publisher, and deletion of the profile
projection/snapshot/final-byte/command-transaction predecessors. These are expected future
postconditions until cdc reaches Done. The current `c639cfb5` plan and research freeze the
remaining selector, variable, migration, event, and vertical-cutover choices; they outrank
this report's earlier open alternatives
(`dev/active/c639cfb5-plan.md:24-36`,
`dev/active/c639cfb5-research.md:652-672`).

No dedicated `dev/sessions/` document records a later profile-lifecycle decision. Current
issue decisions, the reviewed current plan/research, cdc's planned postconditions plus its
future Done-tree re-investigation, current code evidence for surviving v1 contracts, the
profile completion report, and the canonical profile reference outrank older prose.

## Architecture fit and invariant check

### Layer ownership

- **Pure profile domain:** versioned manifest/package types, resolved package graph,
  variable declarations and precedence over supplied inputs, semantic contribution
  identities, ownership/base fingerprints, composition, conflict classification, and
  three-way decisions. These functions consume captured data and return deterministic
  models; they do not open paths, read environment variables, acquire locks, or write
  files (`AGENTS.md:139-152`).
- **Storage boundary:** capability-confined local-directory capture, no-follow traversal,
  bounded byte reads, canonical applied-record loading, isolated version-gated migration
  before canonical service loading, and—only after cdc reaches Done—canonical record
  persistence through the verified cdc repository image/mutation session. Storage must not
  restore a profile-specific snapshot or transaction builder that the actual cdc landing
  removed (`dev/active/c639cfb5-plan.md:31-36`).
- **Command layer:** explicit package source resolution, environment/values/`--set` input
  collection, aggregate plan orchestration, locked rebuild, migration policy, audit-event
  construction, and typed result/error mapping. After cdc Done and re-investigation, it
  should call the actual shared materialization capability, not directly edit targets.
- **CLI/output/schema/MCP:** repeatable selectors, local-path syntax, lifecycle subcommands,
  stable human and JSON output, error codes, dry-run unions, generated schema, and curated
  tool exposure (`AGENTS.md:141-144`, `AGENTS.md:161-163`).

### Protected invariants

- **Domain agnosticism is preserved** if profile IDs, dependency IDs, variables, semantic
  keys, type names, labels, gate keys, projection names, and paths remain manifest/input
  data. The engine may know generic package, dependency graph, contribution, owner,
  variable, conflict, and upgrade concepts. It must not branch on `jit-dogfood`, `epic`,
  or any package-authored workflow value (`AGENTS.md:199-200`).
- **Registry SSOT is preserved** if ownership records store provenance, base identities,
  and observed owners only. Effective configuration continues to load `.jit/config.toml`,
  `.jit/rules.toml`, `.jit/gates.toml`, `.jit/templates.toml`, and declared registries.
  Reconfiguration/upgrade computes desired changes against those registries; it never
  serves behavior from `.jit/profiles/*.json`
  (`dev/vision/9db27a3a-charter.md:102-116`).
- **Package identity is preserved** if strict v1 and v2 wire types normalize into one
  runtime model while v1 hashing still consumes the frozen v1 wire representation and exact
  source bytes. Normalization must not make the v2-capable runtime serialization the new v1
  hash input (`crates/jit/src/profile/package.rs:453-526`).
- **Applied evidence is closed** if every claimed contribution/base/owner and resolved
  non-secret input is reachable from a canonical applied profile, shared owner sets agree,
  and validation rejects orphan, partial, duplicate, or events-derived state before
  publication. Closure strengthens evidence without making it configuration authority.
- **Git optionality is preserved** if explicit local packages are ordinary filesystem
  inputs and profile lifecycle never calls claim/worktree/commit discovery. The existing
  public acceptance suite already establishes the expected Git-free behavior
  (`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:232-299`).
- **Recoverable publication is preserved** if one complete lifecycle input—repository
  declarations/claims, every canonical record, and current audit seed—enters the cdc
  materializer/mutation session and sole publisher. Do not call several single-profile
  operations, build a command-local delta, or invoke a second transaction boundary.
- **Single-source prose is preserved** if `docs/reference/profiles.md` owns lifecycle
  guarantees, generated CLI/event/schema references derive from code, and other entry
  points link rather than copy command/manifest inventories (`AGENTS.md:200`).

### Literal-leak audit

The existing engine has one production package literal at the boundary where the binary
binds its embedded asset tree (`crates/jit/src/profile/dogfood.rs:11-45`) and package-derived
planning-preset compatibility (`crates/jit/src/gate_presets/builtin.rs:17-52`). That is
consistent with the shipped embedded package. The generic manifest and package validator
contain no dogfood taxonomy literals. Cdc plans to remove planner/render/snapshot
vocabularies; v1.1 must verify that actual postcondition after Done. Synthetic package tests
use unrelated vocabulary
(`crates/jit/tests/fixtures/profile-packages/synthetic-valid/manifest.toml:1-77`). v1.1
should keep local resolution and composition generic and leave `jit-dogfood` selection in
the embedded source registry only.

## Constraints handed to synthesis

1. Treat Backlog `cdc840ad` as a blocking implementation prerequisite. Its plan owns
   canonical image/marker/delta/materialization/publication mechanics and predecessor
   deletions, but those are not facts until Done. Then re-investigate the actual tree,
   capabilities, and removals, rebase v1.1, and assert actual cdc-owned paths remain absent.
   No decoder/model/resolver/record implementation lands separately before or during cdc.
2. Define strict version-specific v1 and v2 manifest wire types that immediately normalize
   into one source-neutral validated package image. Embedded and explicit-local readers may
   differ inside one resolver's I/O boundary; resolution, composition, planning, ownership,
   results, and publication must not fork by origin or manifest version.
3. Carry one repeated ordered `--profile id:ID|path:DIR` selector stream, then resolve all
   selected candidates together with every surviving canonical applied owner, dependency,
   dependent, and incompatibility before materialization. Equal definitions retain every
   owner; differing definitions fail without an order-selected winner.
4. Define one versioned ownership/base-fingerprint record and an isolated one-way v1
   migrator. Its decoder accepts the five-field shape only when exact embedded pinned
   dogfood ID/version/origin/package and target hashes authenticate it; `path:DIR`, local,
   copied, inferred, or network evidence is forbidden. It writes baseline-retained claims
   into canonical record v2 (`record_version: 2`) in the requested operation's transaction.
   No ordinary dual reader may accept v1.
5. Enforce full closure over the canonical applied-record set: complete applied profiles,
   package identity/compatibility, resolved non-secret inputs, contribution identities,
   bases, and owner sets; no orphan owner, missing contribution, inconsistent shared set,
   partial migration, event-derived current state, or undeclared sidecar. Keep those records
   evidential: repository registries remain the authority for effective behavior.
6. Freeze variables as strict manifest-v2 `[[variable]] name/default/env`, at most one
   `--values-file PATH` containing exactly a TOML `[variables]` string map, repeatable
   `--set NAME=VALUE`, and sole single-pass `{{jit:var:NAME}}` references in string semantic
   leaves or UTF-8 asset/region bodies with `template = true`; the boolean defaults false.
   Persist values/source kinds; audit sorted names/source kinds only. Add no alternate or
   secret surface.
7. Separate upgrade-time removal of unchanged solely-owned items from general removal.
   Add no `remove`, `uninstall`, or equivalent command/manifest operation in v1.1.
8. Make one vertical live cutover: runtime/init routing, superseded-path deletion,
   Clap/recovery classification, result/error types, output schema, exact acceptance schema,
   MCP curation/inventory, current event construction/catalog/parser/reference, canonical
   docs, and core exact tests change together. A following evidence leaf may add only
   supported-platform/failure/concurrency/recovery/absence coverage, never contracts or
   implementation cleanup.
9. Freeze the planned current event as `profile_lifecycle_changed` with requested operation
   `apply|upgrade` (`init` is `apply`), per-profile action
   `installed|unchanged|reconfigured|upgraded`, and `record_migrations` sorted by
   `profile_id` with `from_version: 1, to_version: 2`. Emit no event for an all-unchanged
   operation or dry run; retain historical `ProfileApplied` decoding only.
10. Preserve the v1 `jit-dogfood` manifest and declared source bytes, package hash, complete
   target-hash map, and package-derived workflow/preset facts as one authored inventory.
   Compute v1 hashes from its frozen wire representation and exact source bytes during
   versioned decode, then pass only the normalized package onward; do not add defaults to
   v1, reauthor it as v2, or create a v1.1 package copy.
11. State publication guarantees precisely as validated, serialized, recoverable
    all-old/all-new convergence. Do not describe a sequential multi-file transaction as an
    instantaneous filesystem-atomic set replacement.
12. Provisionally target only expected surviving post-cdc predecessors in v1.1:
    embedded-only resolver/type, ordinary five-field loader/matcher, one-ID
    command/request/result family, current `profile_applied` constructor/appender, and
    current public/docs contracts. After cdc Done, subtract anything it actually removed.
    Treat its actual
    projection/snapshot/marker/final-byte/materializer/transaction deletions as absence
    assertions, never v1.1 work or recreated helpers.

## Residual implementation facts to verify

- Canonical record schema version 2 is fixed. Implementation must verify that ordinary
  loading accepts only that closed shape and per-profile claims satisfy complete
  applied-closure validation without a central duplicate ownership index.
- The future `cdc840ad` implementation may use names different from its plan. V1.1 remains
  blocked until cdc Done, then must re-read the delivered tree before coding, subtract
  actually absent symbols from deletion accounting, bind only to actual canonical
  capabilities, and fail structural checks if any cdc-owned predecessor is recreated.
- All public spellings are otherwise resolved: repeated ordered
  `--profile id:ID|path:DIR`; strict `[[variable]] name/default/env`; optional exact TOML
  `[variables]` values file; repeatable `--set NAME=VALUE`; sole
  `{{jit:var:NAME}}`; asset/region `template` boolean default false; record v2; and the
  exact `profile_lifecycle_changed` operation/action/migration/no-event rules above, with
  historical `ProfileApplied` decode-only (`dev/active/c639cfb5-plan.md:26-35`).
