# Investigation: complete profile lifecycle, composition, and upgrades (c639cfb5)

> Grounding report for `plan.md`. Every claim below was verified by reading the
> cited file. Exhaustive inventories live here; the plan cites them.

## Executive findings

1. **The prerequisite landed.** `cdc840ad` delivered the shared materialization
   planner, the captured `RepositoryImage`, the shared mutation session, and the
   recoverable publisher, and removed the predecessor surface its plan named.
   Profile application already runs through them. Planning may now treat those
   primitives as facts rather than expected postconditions.
2. **An applied profile is write-once, and that is the adopter-visible defect.**
   Editing a profile-owned file in place is reported as drift, the owning package
   cannot be re-applied once its record exists, and the record is verified against
   the package. No sanctioned sequence returns such a repository to a valid state.
3. **Multi-package resolution partly exists; multi-package *publication* does
   not.** Dependency closures resolve, but each package applies through a separate
   call, so a multi-package selection is not one transaction.
4. **The capture mechanic is built and proven, but is not a product surface.** It
   is compiled only for this crate's tests, hardcoded to this repository's own
   package, and lacks the safety checks an adopter-facing capture needs.
5. **Archive support is closer than expected.** `tar` is already a non-dev
   dependency and package identity already has a content digest. What is absent is
   extraction, verification, and safe placement.

## Claim classification

| Criterion | Verdict | Evidence |
|---|---|---|
| REQ-01 selection | valid-and-open | Wired subcommands are exactly `list`, `show`, `apply`: `crates/jit/src/cli.rs:2926-2969`. Selection is one ID plus optional `--from`, not repeatable ordered selectors. |
| REQ-02 graph | partly-done | The manifest already declares dependencies and packages load from repository directories: `crates/jit/src/profile/manifest.rs:71-95`, `crates/jit/src/profile/package.rs:39-85`. Closures resolve at `crates/jit/src/commands/profile.rs:241-377`. Incompatibilities and compatible-JIT ranges are absent. |
| REQ-03 composition | valid-and-open | Contribution merge is value equality with no ownership sharing: `crates/jit/src/repository_state/profile_apply.rs:835-854`. Occupancy resolves at file granularity from record `target_hashes` and does not exclude the candidate package, so a package collides with its own prior record: `crates/jit/src/repository_state/profile_apply.rs:865-891`. |
| REQ-04 variables | valid-and-open | No variable declaration, input, or reference syntax exists in the manifest type: `crates/jit/src/profile/manifest.rs:71-122`. |
| REQ-05 records | valid-and-open | `AppliedProfileRecord` is a five-field package-level record: `crates/jit/src/repository_state/profile_apply.rs:304-343`. Read at `crates/jit/src/commands/profile.rs:821-844` and `crates/jit/src/commands/validate.rs:791-805`; written at `crates/jit/src/repository_state/initialize.rs:851-858` and `:940-947`. |
| REQ-06 surface | valid-and-open | Three subcommands wired: `crates/jit/src/cli.rs:2926-2969`. No `profile status`, `diff`, `capture`, `pack`, or `add`. |
| REQ-07 three-way | valid-and-open | A record whose content differs is rejected outright rather than reconciled: `crates/jit/src/repository_state/initialize.rs:566-580`. |
| REQ-08 transaction | partly-done | Application already derives one shared plan and submits it through `with_mutation_session`: `crates/jit/src/commands/profile.rs:394-438`, `:504-545`, `crates/jit/src/storage/repository_state_store.rs:502-545`. The gap is that a multi-package selection applies through separate calls: `crates/jit/src/commands/profile.rs:241-377`. |
| REQ-09 compatibility | valid-and-open | The embedded package tree and the v1 record format both exist and must survive; no migration path is present. |
| REQ-10 documentation | valid-and-open | `docs/reference/profiles.md:249-267` documents the apply-only boundary that this container removes. |
| REQ-11 evidence | valid-and-open | Existing suites cover the v1.0 surface only: `crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs`, `profile_acceptance_tests.rs`. |
| REQ-12 SSOT | valid-and-open | Constraint on every landing rather than standalone work. |
| REQ-13 capture | valid-and-open | The assembler exists but is repository-only and lacks capture safety checks. See below. |
| REQ-14 exchange | valid-and-open | No package archive contract exists. See below. |

## Corrections to the prior report

The previous investigation was written before `cdc840ad` completed and is
superseded by this document, which replaces it. Its load-bearing claims were
re-verified against the current tree; these were contradicted:

- It treats `cdc840ad` as Backlog and its planner, session, and materializer as
  future work. They are present: `crates/jit/src/repository_state/mod.rs:660-780`,
  `crates/jit/src/storage/repository_state_store.rs:186-207`.
- It locates `AppliedProfileRecord` in `profile/application.rs`. The definition
  moved to `crates/jit/src/repository_state/profile_apply.rs:304-343`;
  `crates/jit/src/profile/application.rs:1-47` now holds result types and imports
  the record.
- It says multi-profile composition does not exist. Dependency closures resolve
  and composed results return today: `crates/jit/src/commands/profile.rs:241-377`,
  `crates/jit/src/profile/application.rs:49-80`.
- It says the manifest has no dependencies and loading is embedded-only. Both are
  wrong now: `crates/jit/src/profile/manifest.rs:71-95`,
  `crates/jit/src/profile/package.rs:39-85`.
- It expects `profile_lifecycle_changed` to be the emitted event. Current
  mutations still construct and append `ProfileApplied`:
  `crates/jit/src/repository_state/initialize.rs:868-878`,
  `crates/jit/src/repository_state/mutation.rs:1170-1193`.
- It expects the `InitScaffold` final-target inventory to disappear. Exact
  `InitScaffold` is absent, but shared `InitializationScaffold::delta_paths`
  survives: `crates/jit/src/repository_state/initialize.rs:184-225`, `:370-390`.

Confirmed absent, as its plan expected: `PackageProjection`, `ProjectedFile`,
`profile/render.rs`, `RepositorySnapshot` and profile snapshot capture,
`profile::drift`, `PresetProjection`, `ProfileApplicationPlan`, `PlannedTarget*`,
`plan_profile_application_against`, and command-local profile/init transaction
builders. Replacements are `MaterializationPlan`,
`MaterializationRequest::ApplyProfile`, and `ProfileTargetMaterialization`:
`crates/jit/src/repository_state/mod.rs:555-717`,
`crates/jit/src/repository_state/profile_apply.rs:346-355`. Structural absence of
`profile/render.rs` is already enforced:
`crates/jit/tests/provenance_contract/repository_state_cutover_tests.rs:68-93`.

## The write-once trap (motivating defect)

Four mutually exclusive walls leave a drifted repository with no exit:

1. Editing the live file is reported as stale by `validate_materializations`,
   which compares each expected materialization action against the captured image
   and requires identical bytes and mode:
   `crates/jit/src/validation/repository.rs:836-855`,
   `crates/jit/src/repository_state/mod.rs:1089-1128`.
2. Editing the package to match makes the applied record disagree with the
   package it resolves to: `crates/jit/src/commands/validate.rs:884-923` compares
   the stored record against the record recomputed from the package, including
   `package_hash` and every target hash: `crates/jit/src/commands/profile.rs:788-800`,
   `crates/jit/src/repository_state/profile_apply.rs:304-318`.
3. Re-applying the edited package fails twice over. `profile_record_changed`
   returns `InstalledRecordConflict` whenever an existing record differs from the
   new one: `crates/jit/src/repository_state/initialize.rs:566-580`. Before that,
   `equal_or_conflict` rejects any contribution whose live registry value differs
   from the candidate's: `crates/jit/src/repository_state/profile_apply.rs:835-854`.
4. Editing the record is rejected by wall 2.

Because occupancy resolves at file granularity and does not exclude the candidate
package, the conflict a re-apply reports names the candidate as its own occupant:
`crates/jit/src/repository_state/profile_apply.rs:865-891`. Repository content in
no package is also unprotected — nothing in the current record model marks a
target as repository-owned and retained.

## Capture: what exists (REQ-13)

- `assemble_package_tree(package_source, repository_root, destination)` parses
  `manifest.toml`, draws live assets from `repository_root/asset.target`, draws
  package-authored files from `package_source`, stages, validates the staged
  directory as a `ProfilePackage`, publishes it, reads it back, and returns the
  published package: `crates/jit/src/profile/package_assembly.rs:167-183`, `:255-307`.
- It republishes manifest, asset sources, and region sources only; a source the
  manifest stopped declaring does not survive:
  `crates/jit/src/profile/package_assembly.rs:206-234`, `:747-784`. A live-source
  edit changes the resulting package hash: `:659-695`. This is exactly the refresh
  semantics REQ-13 asks for.
- It is repository-only. `PACKAGE_SOURCE_PATH` is hardcoded to
  `profiles/jit-dogfood` and `PACKAGE_ASSEMBLY_ENTRY_POINT` to
  `./scripts/assemble-package.sh`: `crates/jit/src/profile/package_assembly.rs:40-46`.
  The example always passes that path and the checkout root:
  `crates/jit/examples/assemble-package.rs:23-38`, `:58-62`.
- It is not in an adopter binary. The module is gated
  `#[cfg(any(test, feature = "test-support"))]`: `crates/jit/src/profile/mod.rs:12-18`;
  `test-support` is off by default and enabled through a dev self-dependency:
  `crates/jit/Cargo.toml:82-98`.
- The live-source selector is the `assets/live/` prefix:
  `crates/jit/src/profile/package_assembly.rs:185-192`,
  `crates/jit/src/profile/repository_package.rs:1-5`. A real declaration:
  `profiles/jit-dogfood/manifest.toml:531-537`.
- Manifest path safety exists through `validate_relative_path` /
  `is_safe_relative_path`: `crates/jit/src/profile/package.rs:557-573`, `:758-766`.
- **Symlink refusal on capture input is absent.** Capture uses ordinary `fs::read`
  for both package and live sources, which follows links:
  `crates/jit/src/profile/package_assembly.rs:211-216`, `:237-253`. The staged
  output is link-free only because it is recreated with `fs::write`: `:332-354`.
  Undeclared executable repository content is never read, but is not scanned or
  refused.
- Publication stages beside the destination, validates before publishing, retires
  an old destination, then calls `publish_staged_directory_noreplace`:
  `crates/jit/src/profile/package_assembly.rs:255-307`. That primitive really does
  invoke `renameat2(..., RENAME_NOREPLACE)` and rejects unsupported platforms:
  `crates/jit/src/storage/atomic_write.rs:45-73`. **It is atomic but not
  recoverable**: there is no directory fsync or journal, and a failure after
  retirement drops the retired tree with the temporary workspace:
  `crates/jit/src/profile/package_assembly.rs:292-300`.

## Exchange: what exists (REQ-14)

- `tar = "0.4"` is already a **non-dev** dependency of `crates/jit`:
  `crates/jit/Cargo.toml:48-53`, locked at `tar 0.4.45`: `Cargo.lock:2591-2600`.
  `flate2` is dev-only (`crates/jit/Cargo.toml:103-113`, `Cargo.lock:694-702`) but
  also arrives transitively through non-dev `ureq`'s gzip feature:
  `crates/jit/Cargo.toml:62-65`, `Cargo.lock:1304-1332`.
- Archive **creation** exists in Rust only for snapshots:
  `crates/jit/src/commands/snapshot.rs:628-676`. Archive **extraction** is absent
  from `crates/` entirely.
- Release archives are produced by CI shell, not Rust: the workflow runs the
  assembler, copies package trees, then runs `tar -czf` and `tar -tzf`:
  `.github/workflows/release-artifacts.yml:122-153`.
- Package identity already is a content digest: SHA-256 over the domain string
  `jit-profile-package-v1`, the canonical manifest bytes, then each non-manifest
  file's path and contents in sorted order:
  `crates/jit/src/profile/package.rs:18-19`, `:788-854`. An archive can carry this
  rather than inventing a second digest.
- No archive digest verifier exists. The only digest comparison is profile
  provenance: `crates/jit/src/commands/profile.rs:789-800`,
  `crates/jit/src/commands/validate.rs:915-923`.
- Worktree confinement already exists: `package_origin` requires
  `classify_and_canonicalize` to succeed and to classify the directory as
  `RepositoryRootClass::Worktree`, which excludes the separate data root:
  `crates/jit/src/commands/profile.rs:760-785`,
  `crates/jit/src/repository_state/path.rs:487-499`.
- Precedent for a command writing outside `.jit/` but inside the worktree:
  `jit snapshot export --out` takes an output path and a `tar` format:
  `crates/jit/src/cli.rs:2642-2674`, and classifies the path through the layout
  before publishing: `crates/jit/src/commands/snapshot.rs:680-705`,
  `crates/jit/src/repository_state/export.rs:143-165`.

## Primitive verification

| Asserted property | Verdict | Evidence |
|---|---|---|
| Profile application publishes through one recoverable transaction | confirmed | `crates/jit/src/commands/profile.rs:394-438`, `:504-545`, `crates/jit/src/storage/repository_state_store.rs:502-545` |
| A multi-package selection publishes as one transaction | contradicted | Each package applies through a separate call: `crates/jit/src/commands/profile.rs:241-377` |
| Package-tree publication is atomic | confirmed | `renameat2(RENAME_NOREPLACE)`, unsupported platforms rejected: `crates/jit/src/storage/atomic_write.rs:45-73` |
| Package-tree publication is recoverable | contradicted | No fsync or journal; a post-retirement failure drops the retired tree: `crates/jit/src/profile/package_assembly.rs:292-300` |
| Capture refuses symlinked sources | contradicted | Plain `fs::read` follows links: `crates/jit/src/profile/package_assembly.rs:211-216`, `:237-253` |
| Derived-state validation and repair are transactional | confirmed | `RepairDerivedState`: `crates/jit/src/validation/repository.rs:152-200`, `crates/jit/src/commands/validate.rs:43-74` |

## Architecture fit

**Layer ownership** (per the boundaries in `AGENTS.md:27-36`):

- *Pure domain* (`profile/`): versioned manifest and package types, the resolved
  package graph, variable declarations and precedence over supplied inputs,
  semantic contribution identities, ownership and base fingerprints, composition,
  conflict classification, and three-way decisions. These consume captured data and
  return deterministic models.
- *Storage* (`storage/`, `repository_state/`): bounded no-follow capture of
  declared targets, archive reading and writing, safe extraction, atomic worktree
  placement, canonical record persistence, and the version-gated migration input.
- *Commands* (`commands/profile.rs`): package source resolution, values and
  `--set` collection, aggregate plan orchestration, migration policy, audit-event
  construction, and typed result mapping.
- *CLI, output, schema, MCP*: repeatable selectors, subcommand grammar, stable
  human and JSON output, generated schema, curated tool exposure.

**Protected invariants.** Domain agnosticism holds if profile IDs, dependency IDs,
variable names, semantic keys, and paths stay manifest or input data and the
engine never branches on `jit-dogfood` or any package-authored workflow value. The
one production package literal is the embedded asset binding at
`crates/jit/src/profile/dogfood.rs:11-45`, which is consistent with a shipped
embedded package. Registry SSOT holds if ownership records store provenance, base
identities, and observed owners only, and effective behavior keeps loading the
declared registries. Git optionality holds because packages are ordinary
filesystem inputs; the existing acceptance suite already asserts Git-free
behavior: `crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs:232-299`.

**Build-footprint budget is the binding constraint on test topology.** The
enforced budgets are 12 integration-test targets and 2 GiB of active
test-executable bytes: `scripts/rust-build-budget.sh:31-37`; the current target
count is 11 of 12: boundary behavior is asserted at
`crates/jit/tests/scratch_build/rust_build_budget_checker_tests.rs:161-197`. New
coverage must land in existing suites; this container can afford at most one new
integration-test target in total. `AGENTS.md:124-142` also requires dependency
features to stay intentional, which is why exchange reuses the `tar` dependency
already present rather than adding a container format.

## Complete consumer sweep

Direct consumers of the manifest/package, applied-record, profile result/CLI/schema,
and audit contracts. Line numbers were current when written; paths are the
authority.

**Runtime and library.** `crates/jit/src/profile/mod.rs`,
`crates/jit/src/profile/manifest.rs`, `crates/jit/src/profile/package.rs`,
`crates/jit/src/profile/application.rs`, `crates/jit/src/profile/dogfood.rs`,
`crates/jit/src/profile/preset.rs`, `crates/jit/src/profile/apply_claims.rs`,
`crates/jit/src/profile/contribution_drift.rs`,
`crates/jit/src/profile/drift_report.rs`,
`crates/jit/src/profile/template_region.rs`,
`crates/jit/src/profile/repository_package.rs`,
`crates/jit/src/profile/package_assembly.rs`,
`crates/jit/src/repository_state/profile_apply.rs`,
`crates/jit/src/repository_state/initialize.rs`.

**Commands and exports.** `crates/jit/src/commands/profile.rs`,
`crates/jit/src/commands/init.rs`, `crates/jit/src/commands/validate.rs`,
`crates/jit/src/commands/mod.rs`, `crates/jit/src/lib.rs`.

**CLI, presentation, schema.** `crates/jit/src/cli.rs`,
`crates/jit/src/main.rs`, `crates/jit/src/output.rs`, `crates/jit/src/schema.rs`.

**Applied-state and audit vocabulary.** `crates/jit/src/domain/types.rs`,
`crates/jit/src/domain/event_log.rs`, `crates/jit/src/domain/event_catalog.rs`,
`crates/jit/src/repository_state/mutation.rs`.

**Package-derived presets** (SSOT-sensitive — `jit-dogfood` must remain one
authored inventory). `crates/jit/src/gate_presets/builtin.rs`,
`crates/jit/src/gate_presets/planning.rs`,
`crates/jit/src/gate_presets/reference.rs`.

**Tests and fixtures.** Unit tests colocated in every profile module above;
`crates/jit/tests/fixtures/profile-packages/synthetic-valid/manifest.toml`,
`.../planner-asset-only/manifest.toml`,
`.../planner-invalid-interpolation/manifest.toml`;
`crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs`;
`crates/jit/tests/cli_repo_workflow/profile_acceptance_tests.rs`;
`crates/jit/tests/cli_repo_workflow/integration_schema.rs`;
`crates/jit/tests/provenance_contract/repository_state_cutover_tests.rs`;
`crates/jit/tests/scratch_build/build_profile_policy_tests.rs`;
the runtime manifest-schema freshness test in `crates/jit/src/schema.rs`.

**MCP.** `mcp-server/curated-tools.json` owns tool descriptions;
`mcp-server/test-integration.js` asserts the generated inventory, exclusions, and
end-to-end calls; `mcp-server/lib/schema-loader.js` loads the generated schema
generically, so new subcommands should stay generated from `jit --schema` with
curation only for the intentionally exposed set.

**Documentation.** `docs/reference/profiles.md` is the canonical adopter owner for
lifecycle semantics. Also `docs/reference/cli-commands.md`,
`docs/reference/cli-command-grammar.md`, `docs/reference/storage-format.md`,
`docs/reference/storage-records.md`, `docs/reference/events.md`,
`docs/reference/gate-presets.md`, `docs/reference/configuration.md`,
`docs/reference/error-codes.md`, `docs/reference/exit-codes.md`, `README.md`,
`INSTALL.md`, `docs/index.md`, `docs/tutorials/quickstart.md`,
`docs/examples/README.md`, `docs/concepts/planning-bracket.md`,
`docs/how-to/adopt-planning-bracket.md`, `docs/how-to/deployment.md`, and the
shipped/local dogfood boundary in `AGENTS.md`.

**Build and packaging.** `crates/jit/Cargo.toml`,
`crates/jit/examples/assemble-package.rs`, `scripts/assemble-package.sh`,
`scripts/rust-build-budget.sh`, `.github/workflows/release-artifacts.yml`,
`.github/workflows/ci.yml`, and the authored package tree
`profiles/jit-dogfood/manifest.toml` with its asset and region inventory.

## Prior art

- `docs/reference/profiles.md:249-267` states the v1.0 apply-only boundary and
  enumerates exactly what this container adds. It is the adopter-facing statement
  of the defect, not a discovery.
- `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md` records the
  option analysis behind the frozen wire and variable decisions.
- The write-once trap was hit in a downstream repository before it was traced
  here; the four walls above are the mechanism behind that report.

## Residual uncertainty

- Active test-executable bytes were not recomputed: the checker measures them by
  running `cargo test --no-run`, which this read-only investigation did not do
  (`scripts/rust-build-budget.sh:84-93`). The target count (11 of 12) is exact.
- Whether `flate2` must become a direct non-dev dependency for exchange depends on
  whether the archive is compressed. It is already present transitively through
  `ureq`, so the decision affects declared features rather than the dependency
  graph.
