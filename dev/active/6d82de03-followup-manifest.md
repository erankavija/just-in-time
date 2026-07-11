# Follow-up filing manifest — `6d82de03`

## Filing boundary

This manifest is the deliverable for `6d82de03`. It records future, separately
owned work only: it makes no engine, product, configuration, documentation, or
projection change itself. Each listed leaf issue depends on `6d82de03` and is a
direct dependency of the separately owned `004d10b7` follow-up epic. That epic
is a direct dependency of the v1.0 milestone (`9db27a3a`). This keeps the filing
operation downstream of the audit without making the completed audit epic wait
for the subsequently owned work.

The type hierarchy in `.jit/config.toml:24-37` permits `task` and `bug` at
level 4; its namespaces include `type`, `epic`, `milestone`, and `component`.
Every proposed issue below therefore carries the required labels
`epic:documentation-contract-followups` and `milestone:v1.0`, along with one
`type:*` and one source-supported `component:*` label.

## Summary

| # | Proposed title | Type / priority | Component | Basis |
|---|---|---|---|---|
| 1 | Complete the generated CLI schema contract | bug / high | cli | Mandatory schema completeness |
| 2 | Correct `jit issue create --description` help text | bug / normal | cli | Mandatory CLI polish |
| 3 | Project command-specific exit-code mappings | task / normal | cli | Mandatory missing projection |
| 4 | Project the event-tag catalog and issue association semantics | task / normal | core | Mandatory missing projection |
| 5 | Project authoritative storage-layout specifics | task / normal | storage | Mandatory missing projection |
| 6 | Remove the stale `.jit/claims.jsonl` merge-driver entry | bug / normal | storage | Authorized optional defect |
| 7 | Resolve the inert `validation.strictness` contract | bug / high | validation | Authorized optional defect |
| 8 | Declare and enforce the supported Rust MSRV | task / high | core | Authorized optional packaging gap |
| 9 | Add a projector for built-in gate presets | task / normal | gates | Authorized optional projection gap |
| 10 | Project runtime coordination and recovery defaults | task / normal | core | Authorized optional projection gap |

The component-image `:latest` release-policy question is explicitly excluded
and has no proposed issue.

## 1. Complete the generated CLI schema contract

- **Recommended filing:** `type:bug`, priority `high`
- **Labels:** `type:bug`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:cli`
- **Verified source citations:**
  - `crates/jit/src/schema.rs:103-141` builds the generated schema, while
    `:459-469` hard-codes a six-value `State` list without `rejected`.
  - `crates/jit/src/domain/types.rs:19-42,75-101` defines the real seven-state
    lifecycle and its exhaustive `State::all()` list, including `Rejected`.
  - `crates/jit/src/schema.rs:108-118,235-249,286-315` omits built-in
    help/version entries and serializes only the primary long flag name.
  - `crates/jit/src/cli.rs:20-36` defines the global `--quiet` and `--schema`
    options; Clap supplies `--help` and `--version`. Visible aliases include
    `--add-label` at `:978-987` and `--title` at `:1822-1828`.
- **Durable design/research doc:** No. This is a bounded schema/Clap
  synchronization defect; executable tests are the durable contract.
- **Independent ownership:** The schema generator and its tests can be changed
  without choosing a projection format or altering the runtime behavior of any
  command.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

The generated `jit --schema` contract is incomplete. Its `types.State.enum`
omits the real `rejected` lifecycle state, and its command/global-flag inventory
does not expose all spellings that Clap accepts: visible aliases and the global
help/schema/version surface are not represented. This makes the schema an
unsafe sole oracle for clients and future documentation checks.

Source: `crates/jit/src/schema.rs:103-141,235-249,286-315,459-469`;
`crates/jit/src/domain/types.rs:19-42,75-101`; and
`crates/jit/src/cli.rs:20-36,978-987,1822-1828`.

## Success Criteria

- [hard] REQ-01: `jit --schema` represents every real `State` value, including
  `rejected`, from one authoritative enumeration rather than a divergent
  hand-maintained subset.
- [hard] REQ-02: The generated schema exposes each accepted visible alias and
  all global CLI flags, including the help/schema/version surface, in a
  documented machine-readable representation.
- [hard] REQ-03: Focused tests compare the generated state and flag inventories
  to the Clap/domain sources and fail when a future state, primary flag, alias,
  or global flag is omitted.

## 2. Correct `jit issue create --description` help text

- **Recommended filing:** `type:bug`, priority `normal`
- **Labels:** `type:bug`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:cli`
- **Verified source citations:** `crates/jit/src/cli.rs:612-669` defines
  `IssueCommands::Create`; unlike the neighboring title, type, gate, label,
  and content-format arguments, its `description` argument at `:640-641` has
  no help text. The content-format explanation at `:664-669` describes parsing
  of the description body rather than the `--description` argument itself.
- **Durable design/research doc:** No. This is an isolated CLI help correction.
- **Independent ownership:** It changes one argument's user-facing definition
  and test coverage without affecting schema completeness or documentation
  projections.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

`jit issue create --description` has no argument-specific help text. The
adjacent content-format documentation explains how the body is parsed, not what
the description option accepts. Give the creation flag a direct, accurate help
description and lock the behavior down with CLI-help coverage.

Source: `crates/jit/src/cli.rs:612-669`.

## Success Criteria

- [hard] REQ-01: `jit issue create --help` describes `-d`/`--description` as
  the initial issue description/body rather than as a content-format setting.
- [hard] REQ-02: The help text remains consistent with the option's actual
  default and behavior in the create command.
- [hard] REQ-03: A focused regression test verifies the argument-specific help
  wording or the equivalent Clap metadata.

## 3. Project command-specific exit-code mappings

- **Recommended filing:** `type:task`, priority `normal`
- **Labels:** `type:task`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:cli`
- **Verified source citations:** `crates/jit/src/schema.rs:520-556` supplies
  only a global exit-code taxonomy. `crates/jit/src/main.rs:33-247` maps typed
  failures to codes at runtime, and command dispatch uses direct
  `std::process::exit` paths (for example `crates/jit/src/output_macros.rs:112-121`).
  The audit plan identifies the per-command mapping as an unprojected surface:
  `dev/active/2d109173-plan.md:384-403`.
- **Durable design/research doc:** No. The issue should decide and implement a
  compact, source-derived schema/reference representation in the code and test
  it; no separate research artifact is needed.
- **Independent ownership:** Exit-code projection can be designed and shipped
  without changing event, storage, preset, or configuration projections.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

`jit --schema` documents the global exit-code taxonomy but not which command
outcomes can produce each code. The actual behavior is distributed through
typed error classification and command-specific exits, forcing documentation
to cite implementation details instead of consuming a maintained projection.
Add a source-derived per-command exit-code mapping to an appropriate
machine-readable and/or generated reference surface; do not duplicate it by
hand.

Source: `crates/jit/src/schema.rs:520-556`; `crates/jit/src/main.rs:33-247`;
`crates/jit/src/output_macros.rs:112-121`; and
`dev/active/2d109173-plan.md:384-403`.

## Success Criteria

- [hard] REQ-01: A discoverable, generated surface identifies the exit codes
  that each command (or explicitly documented command family) can return,
  including command-specific exceptions to the global taxonomy.
- [hard] REQ-02: The projection derives from or is mechanically verified against
  the runtime classification/dispatch sources, so a mapping change cannot leave
  it stale.
- [hard] REQ-03: Documentation consuming the new surface does not hand-copy
  per-command exit-code tables.

## 4. Project the event-tag catalog and issue association semantics

- **Recommended filing:** `type:task`, priority `normal`
- **Labels:** `type:task`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:core`
- **Verified source citations:**
  - `crates/jit/src/domain/types.rs:1305-1581` declares the serialized event
    variants and makes clear that `DocumentArchived`, gate-definition events,
    and lifecycle-timestamp backfill omit `issue_id`.
  - `crates/jit/src/domain/types.rs:1981-2031` is the exact tag catalog and
    maps the no-issue variants to an empty association.
  - `crates/jit/src/storage/json.rs:735-775` serializes and reads the event
    log as JSON Lines.
  - The audit's Group-C definition records this as an unprojected fact:
    `dev/active/2d109173-plan.md:384-403`.
- **Durable design/research doc:** No. The event enum is the source; the future
  issue needs a generated projection plus tests.
- **Independent ownership:** The event contract has a distinct source enum and
  consumer audience, so it can be projected without redesigning storage layout
  or command exit behavior.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

The event stream has a finite, source-defined tag vocabulary, but neither the
tag catalog nor the important fact that some repository/registry events carry
no `issue_id` is maintained as a projection. Provide a generated,
machine-consumable and documentation-friendly event-contract surface derived
from the `Event` enum.

Source: `crates/jit/src/domain/types.rs:1305-1581,1981-2031` and
`crates/jit/src/storage/json.rs:735-775`.

## Success Criteria

- [hard] REQ-01: The projection lists every currently emitted event tag and
  identifies its source-defined association scope (issue, registry, or
  repository), including whether an `issue_id` field is present.
- [hard] REQ-02: The no-issue event set includes `document_archived`, the three
  gate-definition tags, and `lifecycle_timestamps_backfilled`, and no
  issue-scoped event is mislabeled.
- [hard] REQ-03: Automated coverage makes additions or changes to the `Event`
  enum fail until the projected catalog is updated or regenerated.

## 5. Project authoritative storage-layout specifics

- **Recommended filing:** `type:task`, priority `normal`
- **Labels:** `type:task`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:storage`
- **Verified source citations:**
  - `crates/jit/src/domain/types.rs:16-17,547-579` establishes UUID issue IDs
    and eight-character human short IDs.
  - `crates/jit/src/storage/json.rs:24-39,82-89,152-179,634-670,735-775,778-812`
    defines the `.jit` files, JSONL event persistence, and the canonical
    `gate-runs/<run-id>/result.json` layout.
  - `dev/active/2d109173-plan.md:31,384-403` records ID scheme, event shape,
    and gate-runs layout as cite-source-only facts awaiting follow-up.
- **Durable design/research doc:** No. The storage implementation is the source
  of truth; a generated reference and conformance tests are sufficient.
- **Independent ownership:** This is a storage contract projection with no
  required changes to event vocabulary, command behavior, or coordination
  defaults.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

Adopter documentation currently has to cite storage source for the issue-ID
scheme, event-file shape, and gate-run result layout because no maintained
projection exposes them. Add a source-derived storage-format reference that
covers these facts without turning machine-local state into a versioned data
contract.

Source: `crates/jit/src/domain/types.rs:16-17,547-579` and
`crates/jit/src/storage/json.rs:24-39,82-89,152-179,634-670,735-775,778-812`.

## Success Criteria

- [hard] REQ-01: A generated storage reference specifies full UUID issue IDs,
  the eight-character human short-ID convention, and the accepted prefix
  behavior only where the source supports it.
- [hard] REQ-02: The same reference specifies that events are JSON Lines in
  `.jit/events.jsonl` and describes their serialized shape by linking to the
  authoritative event contract rather than copying a second tag catalog.
- [hard] REQ-03: The canonical gate-run result location is projected as
  `.jit/gate-runs/<run-id>/result.json`, with source-backed structured result
  fields and no hand-maintained alternative layout.
- [hard] REQ-04: Projection freshness or equivalent conformance tests prevent
  the published layout from drifting from storage code.

## 6. Remove the stale `.jit/claims.jsonl` merge-driver entry

- **Recommended filing:** `type:bug`, priority `normal`
- **Labels:** `type:bug`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:storage`
- **Verified source citations:**
  - `crates/jit/src/storage/gitattributes.rs:1-8,57-62` writes a union merge
    driver for `.jit/claims.jsonl`.
  - `crates/jit/src/storage/worktree_paths.rs:8-22,93-103` places shared control
    plane state under `<common-dir>/jit`, distinct from local `.jit` data.
  - `crates/jit/src/storage/claim_coordinator.rs:701-724` appends claims to
    `self.paths.shared_jit.join("claims.jsonl")`.
  - The audit notes record that this mismatch is a code defect, not a docs claim:
    `dev/active/2b9a80fb-audit-notes.md:17-18`.
- **Durable design/research doc:** No. Source ownership is unambiguous; tests
  for generated `.gitattributes` are sufficient.
- **Independent ownership:** Removing or correcting the obsolete merge-driver
  output is isolated from claim protocol behavior and all projection work.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

`jit init` writes a `.gitattributes` union-merge entry for
`.jit/claims.jsonl`, but claim coordination writes its append-only log in the
shared git control plane (`.git/jit/claims.jsonl`), not under the versioned
local `.jit` data plane. Remove the dead entry or replace it only if a real,
supported merge target is intentionally introduced; do not document a path the
product does not create.

Source: `crates/jit/src/storage/gitattributes.rs:1-8,57-62`,
`crates/jit/src/storage/worktree_paths.rs:8-22,93-103`, and
`crates/jit/src/storage/claim_coordinator.rs:701-724`.

## Success Criteria

- [hard] REQ-01: A newly initialized git repository does not receive a merge
  driver entry for `.jit/claims.jsonl` unless that path is an actual supported
  persisted product file.
- [hard] REQ-02: Any remaining JIT merge-driver entries correspond to files
  written in their documented storage plane.
- [hard] REQ-03: Focused initialization tests cover the resulting
  `.gitattributes` content and the real claim-log location.

## 7. Resolve the inert `validation.strictness` contract

- **Recommended filing:** `type:bug`, priority `high`
- **Labels:** `type:bug`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:validation`
- **Verified source citations:**
  - `.jit/config.toml:42-47` still declares `validation.strictness = "loose"`.
  - `crates/jit/src/config.rs:284-305` explicitly calls `strictness` an inert
    forward-compatibility key with no validation behavior.
  - `docs/reference/configuration.md:75-91` and
    `docs/reference/example-config.toml:84-99` must currently warn readers that
    the setting is inert.
  - `dev/active/36d5451e-audit-notes.md:75-83,126-134` records the owner-approved
    choice: remove the key or give it real behavior.
- **Durable design/research doc:** No. The future owner should make and record a
  bounded source-level decision in the issue/implementation; no broad research
  artifact is required.
- **Independent ownership:** This config contract can be removed or implemented
  independently of schema, storage, MSRV, preset, and runtime-default work.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

`[validation].strictness` is present in the generated/local configuration but
does not affect validation. Resolve the false configuration contract: either
remove/migrate the inert key and its examples, or define, implement, and test
precise behavior for `strict`, `loose`, and `permissive`. Do not leave a
selectable setting whose values are ignored.

Source: `.jit/config.toml:42-47`; `crates/jit/src/config.rs:284-305`; and
`docs/reference/configuration.md:75-91`.

## Success Criteria

- [hard] REQ-01: The project selects and implements one coherent outcome:
  remove/migrate `validation.strictness`, or give every documented value
  observable, tested validation behavior.
- [hard] REQ-02: Repository templates, config parsing/validation, examples,
  and reference documentation agree with the selected outcome.
- [hard] REQ-03: Regression tests prove that no accepted strictness setting is
  silently inert.

## 8. Declare and enforce the supported Rust MSRV

- **Recommended filing:** `type:task`, priority `high`
- **Labels:** `type:task`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:core`
- **Verified source citations:**
  - `Cargo.toml:1-25` declares workspace metadata and dependencies but no
    `rust-version`; `crates/jit/Cargo.toml:1-59` inherits edition/license but
    also has no `rust-version`.
  - `.github/workflows/ci.yml:43-46` tests the moving
    `dtolnay/rust-toolchain@stable` channel rather than a minimum release.
  - `dev/active/36d5451e-audit-notes.md:66-79,126-134` records the resulting
    unsourced documentation floor and the required follow-up choice.
- **Durable design/research doc:** No. Selecting a supported compiler floor and
  encoding it in Cargo/CI is a bounded release-maintenance task.
- **Independent ownership:** Toolchain support is a packaging/CI commitment,
  independent of behavior, storage, and documentation projection changes.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

The workspace has no declared or tested minimum Rust version: Cargo manifests
omit `rust-version` and CI follows `stable`. Decide the supported MSRV, make it
authoritative in package metadata and CI, and update installation guidance to
cite that source. If the project intentionally supports only the moving stable
channel, state and enforce that policy explicitly instead of implying an MSRV.

Source: `Cargo.toml:1-25`, `crates/jit/Cargo.toml:1-59`, and
`.github/workflows/ci.yml:43-46`.

## Success Criteria

- [hard] REQ-01: The project has one explicit, authoritative Rust support
  policy (a numeric MSRV or an explicitly enforced stable-only policy).
- [hard] REQ-02: Cargo metadata and CI enforce the selected policy; CI does not
  merely test a newer moving toolchain when the project claims an MSRV.
- [hard] REQ-03: Installation/development documentation derives its Rust
  requirement from the declared policy and contains no unsupported version
  floor.

## 9. Add a projector for built-in gate presets

- **Recommended filing:** `type:task`, priority `normal`
- **Labels:** `type:task`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:gates`
- **Verified source citations:**
  - `crates/jit/src/gate_presets/builtin.rs:1-6,43-117,313-348,351-372`
    constructs and lists the binary-bundled preset definitions.
  - `crates/jit/src/gate_presets/manager.rs:27-44,111-137` merges built-ins
    with custom presets and exposes runtime listing metadata.
  - Existing projections are narrowly scoped to invariant and rule/gate
    registries: `crates/jit/src/commands/invariant.rs:1-23` and
    `crates/jit/src/commands/reference.rs:1-19`; neither projects built-in
    presets. The audit plan records the general no-new-engine boundary that
    deferred this gap: `dev/active/2d109173-plan.md:31,384-403`.
- **Durable design/research doc:** No. A configuration-driven target and
  deterministic renderer, accompanied by freshness tests, is sufficient.
- **Independent ownership:** Preset projection reads a discrete built-in catalog
  and does not require changes to the gate registry or planning bracket.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

Built-in gate presets are defined in Rust and can change without a maintained
documentation surface. Add a deterministic projector/reference for the
built-in catalog—names, descriptions, gate contents, stages, modes, and
relevant checker metadata—so user-facing text does not hand-copy preset counts
or contents. Keep repository-local gate registries and shipped built-ins
clearly distinct.

Source: `crates/jit/src/gate_presets/builtin.rs:1-6,43-117,313-372` and
`crates/jit/src/gate_presets/manager.rs:27-44,111-137`.

## Success Criteria

- [hard] REQ-01: A deterministic, discoverable projection renders every
  built-in preset from the binary's source definitions, including its gates'
  keys, stages, modes, and descriptions.
- [hard] REQ-02: The rendered surface distinguishes shipped built-in presets
  from project-local/custom preset configuration.
- [hard] REQ-03: Automated freshness coverage fails when a built-in preset or
  gate definition changes without the projection being regenerated.

## 10. Project runtime coordination and recovery defaults

- **Recommended filing:** `type:task`, priority `normal`
- **Labels:** `type:task`, `epic:documentation-contract-followups`, `milestone:v1.0`, `component:core`
- **Verified source citations:**
  - `crates/jit/src/agent_config.rs:43-65` sets the default heartbeat interval
    to 30 seconds.
  - `crates/jit/src/storage/json.rs:107-123` and
    `crates/jit/src/storage/lock.rs:118-155` set the default lock timeout to
    five seconds and poll every ten milliseconds.
  - `crates/jit/src/commands/claim.rs:830-848` cleans orphaned temporary files
    at a one-hour threshold during recovery.
  - `crates/jit/src/cli.rs:2551-2572` sets `jit claim acquire --ttl` to 600
    seconds; `docs/reference/configuration.md:261-280` already documents some
    coordination defaults, so the future projection must consolidate rather
    than duplicate them.
  - `dev/active/2d109173-progress.json` records this owner-approved candidate
    as `unprojected-config-defaults` in `engine_followup_slate`.
- **Durable design/research doc:** No. The defaults are code-owned operational
  facts; a generated reference/structured output with freshness coverage is
  the required durable artifact.
- **Independent ownership:** This introduces a projection of existing runtime
  behavior without changing the defaults or coordination protocol itself.
- **Sequencing:** None among this manifest's issues.

### Standalone issue description

Several operational defaults are source-defined but lack one maintained,
complete projection: heartbeat cadence, lock acquisition timeout/polling,
orphaned-temp cleanup age, and default claim TTL. Create a source-derived
runtime-default reference or machine-readable surface, reconcile the existing
claim documentation with it, and make freshness mechanically checkable. This
task projects current behavior; it does not change the default values or
container release policy.

Source: `crates/jit/src/agent_config.rs:43-65`,
`crates/jit/src/storage/json.rs:107-123`,
`crates/jit/src/storage/lock.rs:118-155`,
`crates/jit/src/commands/claim.rs:830-848,2551-2572`, and
`docs/reference/configuration.md:261-280`.

## Success Criteria

- [hard] REQ-01: One discoverable, source-derived surface presents the default
  heartbeat interval, lock timeout and polling behavior, temporary-file cleanup
  threshold, and claim TTL with their units and operational scope.
- [hard] REQ-02: Existing coordination documentation either consumes or is
  mechanically checked against that surface, so it cannot diverge from source.
- [hard] REQ-03: Focused freshness/conformance tests fail if any listed runtime
  default changes without the projection being updated or regenerated.

## Filing notes

- All ten leaf issues are deliberately independent: they have no inter-leaf
  dependency. Each should depend only on `6d82de03` for this filing graph.
- No issue needs a durable linked design or research document; each has a
  bounded, source-owned implementation and testable criteria. Durable output
  belongs in code, generated/reference documentation, and automated tests.
- The `CLAUDE.md` to `AGENTS.md` invariant-projection correction is already
  represented by the current `.jit/config.toml` target and is not a Group-C
  follow-up in this approved filing scope.
