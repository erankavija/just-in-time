# Investigation: profile surface of 3d9a0ce3 (REQ-04, REQ-06..REQ-15, study e643ef66)

Read-only investigation at `main` 2abebdd7c (2026-10-20). Scope: REQ-04 (`@/charter/D-8`), REQ-06..REQ-13, and REQ-14/REQ-15, which moved here from 2195c98f. Line numbers are at that commit. "Prod" means a non-`#[cfg(test)]` caller.

Primary files:

| File | Lines | Production part | Test part |
| --- | ---: | --- | --- |
| `crates/jit/src/commands/profile.rs` | 8699 | 1-3038 | `mod tests` 3039-8699, 117 tests |
| `crates/jit/src/repository_state/profile_apply.rs` | 5149 | 1-3546 | 3547-5149, 35 tests |
| `crates/jit/src/repository_state/initialize.rs` | 1512 | 1-1250 | |
| `crates/jit/src/commands/init.rs` | 1560 | | |
| `crates/jit/src/profile/*.rs` | 18 files | | |

---

## 1. Claim classification

Classes:
- **done**: already done.
- **open**: valid and open.
- **invalid**: invalid as stated.
- **mooted**: mooted by D-8, meaning the code is deleted with the ownership machinery.
- **D-8+**: still valid, but its final shape can only be judged after the D-8 deletion lands.

### 1.1 Epic Background, profile paragraphs

| # | Claim | Class | Evidence |
| --- | --- | --- | --- |
| C1 | Plan, apply, diff, reconfigure and upgrade repeat one selection-preparation skeleton (`commands/profile.rs:1163-1840`) | open | Separate skeletons exist in `plan_profiles_with_inputs` 1168, `plan_profiles_from_sources` 1201, `apply_profile_with_inputs` 1285, `apply_profile_from_sources` 1342, `diff_profiles_from_sources` 1388, `apply_profile_package_with_inputs` 1818, and `init.rs` 127-145 and 157-172. The reconfigure and upgrade parts are mooted. |
| C2 | Folded into `lifecycle_selection_from_sources` at 1540 for only some callers | open, then mooted | Defined at 1540. Its only callers are reconfigure and upgrade (1432, 1457, 1482, 1507). Its body is record-driven (`read_applied_profile_record`, `RecordedValueAuthority`), so the whole function is deleted with D-8. |
| C3 | `*_from_sources` resolves selectors and graph, then `*_with_inputs` resolves both again (1206/1173, 1347/1290) | open | 1206-1210 then 1173-1177. 1347-1351 then 1289-1293. |
| C4 | `init.rs:133-167` repeats the double resolution | open, understated | `init.rs:133-137` and 166-167. `main.rs:2552` also calls `validate_profile_selection` (`profile.rs:1792`), which resolves selectors and graph a third time. Init therefore resolves 3 times. |
| C5 | One selection captures the repository three times | open | `prepare_profile_selection` (`profile.rs:2040`) calls `capture_proposed_base` at 2097, 2125 and 2148. Each `resolve_profile_graph` also opens a session in `resolve_applied_profile_packages` 1679-1705, and every `id:` selector opens another in `read_applied_profile_record` 2191. Those extra sessions are mooted. |
| C6 | Rebuilds its inputs three times | open | `profile_selection_inputs` at 2102, 2129 and 2152, defined at 2943. |
| C7 | Composes every package 2 + 2N times | open, understated | Full-context compositions per selection: `profile_contribution_overrides` at 2104 and 2155 (2); `profile_capture_closure` per input at 2109 and 2158 (2N); `derive_materialization` at 2131 and 2170, each composing once per package inside `derive_profile_applications` (`initialize.rs:1028-1031`, 2N more). The total is at least 2 + 4N. |
| C8 | Registry TOML re-parsed per rendered contribution | open | `render_composed_contribution` (`profile_apply.rs:3066`) calls `semantic_document` (3278: `to_string` then `toml_edit::de::from_str`) per contribution. `contribution_in_registry` (2912) re-parses through `MergeDocument::load` and `semantic_document` per identity, reached per candidate from `repository_definition` (2878). |
| C9 | Variable-input validation runs twice per command | open | `load_profile_variable_inputs` validates at `profile.rs:500` (`validate_against_names`). `validate_variable_inputs` repeats it at 1177, 1293, 1403, 1824 and `init.rs:169`. |
| C10 | `validate_model_references` runs in the package constructor (`package.rs:623`) and again at `variables.rs:311,337` | open; the :337 call is mooted | :623 is in `validate_manifest_declarations`. :311 is in `resolve_package` and survives. :337 is in `resolve_package_from_record`, which is deleted with records. |
| C11 | Executor keeps dead or test-only public plan/apply entry points | open | See 1.3 (REQ-13). |
| C12 | Five private canonical-JSON functions | open, partly mooted | `package.rs:980` (`canonical_bytes`, with helpers `canonical_json` 985 and `canonicalize_value` 991). `wire.rs:240` (`canonicalize`, a `Value` transform used only by v1 `decode_manifest` at :79; 842b0ac6 REQ-06 deletes it). `variables.rs:512` (`canonical_json_bytes` and `canonicalize_json` 520, used only by `ResolvedProfileContent::target_hashes` 466). `profile_apply.rs:1367` (`canonical_json_bytes` and `canonicalize_json` 1371, used only by `fingerprint_semantic_contribution` 1285, so mooted). `mutation.rs:576` and `:588` (`canonical_json` and `canonicalize_json`, which serialize issues, gate runs, events and presets; survives). |
| C13 | With `preserve_order` off, each one is an identity transform | open, needs precision | The recursive `canonicalize*` helpers are identity on a `serde_json::Value`, because `Map` is a `BTreeMap` (section 4). The wrapping functions are not identity relative to `serde_json::to_vec(&T)`. They go through `to_value`, which turns struct field declaration order into sorted key order. The surviving function must keep the `to_value` step, or sort keys explicitly. |
| C14 | `ProfileApplyResult` (`application.rs:41`) duplicates `ProfilePlanEntry` (`:272`), bridged by a trait | open | Trait `SelectionObservation` at `application.rs:80`, impls at :93 and :105. The twin status enums `ProfileApplicationStatus` (:11) and `ProfilePlanStatus` (:256) are not named in the epic. The `profile_apply` schema is a hand-written `oneOf` over both (`schema.rs:514-530`). |
| C15 | `ContributionConflictOwner` (`profile_apply.rs:241`) and `ProfileConflictOccupant` (`:586`) are one enum with two wire spellings | open, D-8+ | Both are `{Repository, Package(ProfilePackageId)}`. One derives `Serialize` with default PascalCase tags and no `JsonSchema`; the other uses `rename_all = "kebab-case"` with `JsonSchema`. After D-8 the repository is never a conflict party, so the origin type may reduce to package ids alone (decision N4). |
| C16 | `ProfileDependencyError` (`commands/profile.rs:262`) mirrors `ProfileGraphError` (`profile/graph.rs:215`) through a field-by-field translator | open | The translator is `profile_graph_error` at `profile.rs:2317`. Its fallback arm `other => anyhow!(other)` (:2372) sends `SelectedPackageMissing`, `MissingDependency` and the four `Invalid*` variants to the generic error code instead of `ProfileError`, because only `ProfileDependencyError` is downcast at `main.rs:144` and `:1276`. That is a classification inconsistency. |
| C17 | `ProfilePackage` and `CapturedPackageTree` (`package_capture.rs:133`) are parallel validated-package types | open | Both are built through `ProfilePackage::validate_content` (`compose_captured_tree`, `package_capture.rs:697`). `CapturedPackageTree` survives D-8 as the type `pack` reads (`capture_package_tree`, used by `pack_profile_package_inner`, `profile.rs:883`) and the type `add` extracts (`read_package_archive`, `package_archive.rs:391`). |
| C18 | `ProfilePackageSource` (`package.rs:45`) has a single variant | open | `Directory(PathBuf)` only. Destructured irrefutably at `profile.rs:1762` and :2310. The same pattern appears in `ProfileOrigin` (`domain/types.rs:1130`, irrefutable at `main.rs:1176`), and after D-8 in `ProfileSelector` (`profile.rs:120`), whose `Id` variant loses its only resolution route. |
| C19 | Hand-written wire and schema mirrors `ProfileDivergenceWire`, `ProfileDivergenceSchema`, `ProfileTargetConflictWire` | first two mooted; third open, D-8+ | `ProfileDivergenceWire` and `ProfileDivergenceSchema` are at `agreement.rs:127` and `:203`; `agreement.rs` is deleted with `profile validate`. `ProfileTargetConflictWire` (`profile_apply.rs:810`) exists only to inject the `remedy` field, which D-8 deletes. A fourth mirror the epic does not name, `ProfileTargetSubjectWire` (`profile_apply.rs:911`), survives. |
| C20 | Registry target paths are spelled four times | open | `Contribution::registry_path` (`profile_apply.rs:65-72`), `ContributionRegistry::path` (149-157), `KeyedArrayTarget::registry_path` (513-520), and the `KeyedArrayTarget`-to-`ContributionRegistry` match in `Contribution::semantic_identity` (106-115). `VirtualPath::{CONFIG,GATES,RULES,TEMPLATES,INVARIANTS}` (`repository_state/path.rs:288-297`) already declare the paths, so they are a fifth source. `initialize.rs:332-336` spells them again outside the profile model. |
| C21 | Registry merge renderers re-implement the presence lookup `contribution_in_registry` owns | open, D-8+ | Each renderer has its own presence test: `merge_scalar` 3101, `merge_map_entry` 3120 (through `semantic_map_entry`), `merge_set_string` 3177, `merge_keyed_array` 3209 (its own scan, which also enforces identity uniqueness), `merge_projection` 3256. Every renderer returns early when the entry exists (insert-only). D-8 needs replace semantics (decision N3), so the renderers are rewritten anyway. |

### 1.2 Audit rows and the out-folds "dropped" list

- **P12** (perf 12). Valid as itemized in C3-C8. The "opens a full session per record" part is mooted: record reads are deleted.
- **A12** (abstraction 12). Valid. The record path and the seven record readers are mooted by D-8; the identity and version parts are covered in 1.4.
- **S10** (simplification 10). Valid. The skeleton is repeated 6 or more times (C1). `ProfileDependencyError` (C16), the double validation (C9), and the dead entry points (1.3) are confirmed.
- **S11** (simplification 11). Valid with the precision from C13.
- **S12** (simplification 12). The per-target hashing pair is `package.rs:910-971` and `variables.rs:466-505`. The audit says `target_hashes` exists only for a config/rules check; that is confirmed: `ProfileApplicationInput::target_hashes` (`profile_apply.rs:1520`) is read only for key presence (`profile_apply.rs:1560-1561`, `initialize.rs:956-957`).

The four "dropped" items in out-folds.json:

| Item | Verdict |
| --- | --- |
| (1) A12 record path and readers | Correctly mooted. `applied_record_path` (`profile.rs:2466`), `read_applied_record` (2986), `applied_profile_records` (`profile_apply.rs:2592`), `initialize.rs:637`/`1087`, and `commands/validate.rs:835-890` all go. |
| (2) P12 per-record sessions | Correctly mooted. |
| (3) S12 target hashing | Correct for the value hashes in `variables.rs` and the record fingerprints. `ProfilePackageHashes::targets` (`package.rs:910-971`) still has surviving consumers: its keys give the target set (`profile.rs:2052`) and `profile show` prints the values (`application.rs:149`). See N6. |
| (4) S12 divergence triple | Correctly mooted. Its survivor is C19's `ProfileTargetConflictWire` and the new `ProfileTargetSubjectWire`. |

The fold note says "if capture is deleted, REQ-09's single package type is satisfied outright". That is **invalid**. `CapturedPackageTree` and `capture_package_tree` serve `pack` and `add`, which survive (C17). Only the `capture` subcommand and capture's contribution refresh are deleted.

### 1.3 Criterion-level status (REQ-13 detail)

Executor entry points in `commands/profile.rs` and `commands/init.rs`, by production reachability from `main.rs` dispatch (`main.rs:2552-2981`):

- **Dead** (no caller at all):
  - `plan_profile` (1221) → `plan_profile_with_inputs` (1226) → `plan_profile_package_with_inputs` (1236).
  - `initialize_profiled_repository_from_sources` (`init.rs:89`).
- **Test-only:**
  - `plan_profiles` (1163).
  - `apply_profile` (1274): unit tests and two integration files.
  - `apply_profile_package` (1807) → `apply_profile_package_with_inputs` (1818) → `resolve_profile_closure` (1622). There are 76 unit-test call sites plus `fast_rules/derived_state_repair_tests.rs`.
  - `initialize_profiled_repository` (`init.rs:79`).
  - `prepare_profile` (1995, `#[cfg(test)]`).
  - `initialize_fresh_repository` (`init.rs:105`) is called by `crates/server/src/routes.rs:1115`, so it is **not** test-only.
- **Production but redundant layers:**
  - `plan_profiles_with_inputs` and `apply_profile_with_inputs` are reached only through their `*_from_sources` wrappers (C3).
  - `validate_profile_selection` (1792) is reached only from init (C4).
- **Mooted by D-8:**
  - `list_recorded_profiles` (614), `validate_recorded_profiles` (654).
  - `capture_profile_package` and `plan_profile_capture` (744, 754).
  - `reconfigure_profiles_from_sources`, `plan_reconfigure_profiles_from_sources` (1427, 1452).
  - `upgrade_profiles_from_sources`, `plan_upgrade_profiles_from_sources` (1477, 1502).
- **Note:** 76 unit tests drive `apply_profile_package`. Deleting it means rewriting those tests onto the surviving entry point (`apply_profile_from_sources` or its successor). That is the largest test-migration cost in REQ-13.

### 1.4 Moved criteria REQ-14 and REQ-15

**REQ-14 (`ProfilePackageId`).** Valid and open. `ProfilePackageId` is defined at `profile_apply.rs:608` and appears 71 times in production and test sources:

| File | Count |
| --- | ---: |
| `profile_apply.rs` | 49 |
| `commands/profile.rs` | 7 |
| `materialize.rs` | 4 |
| `apply_claims.rs` | 3 |
| `repository_state/mod.rs` | 3 |
| `main.rs` | 3 |
| `difference.rs` | 1 |
| `initialize.rs` | 1 |

Constructor (`ProfilePackageId::new`) sites:

- **Production:**
  - `profile_apply.rs:2209` (`profile_target_ownership`, record-driven): deleted by D-8.
  - `profile_apply.rs:2652` (`recorded_semantic_claims`): deleted by D-8.
  - `apply_claims.rs:109` and `:128` (building `ProfileContributionClaim` from a resolved package): survives.
- **Test:**
  - `profile_apply.rs` 3723-4659: 20 sites.
  - `materialize.rs:1015`, `1740`, `1742`.
  - `initialize.rs:1288`.
  - `commands/profile.rs:5650-5949`: 6 sites.
  - `main.rs:9305`, `9331`.

Field and type uses:
- **Survive** (as composition inputs and conflict parties): `ProfileContributionClaim.package_id` (215), `ComposedContribution.owners` (237), `ContributionConflictOwner::Package` and `ProfileConflictOccupant::Package` (or their merged successor), `ProfileClaims.package_id`.
- **Deleted by D-8:** the record-ownership maps (`ProfileTargetOwnership` 2197, `ProfileTargetDecision.owners` 1737, `ProfileContributionDecision.owners` 1767, `RecordedSemanticClaim` 2634).

After D-8, roughly a third of the production occurrences remain. Do this after 2195c98f delivers the domain `ProfileId`.

**REQ-15 (semver types).** Valid and open.
- Versions are `String`: `ProfilePackageModel.version` and `.compatible_jit` (`manifest.rs:136`, `:139`), `ProfileDependencyRequirement.version` (:168), `ProfileIncompatibility.version` (:176).
- They are parsed and discarded in `validate_manifest_declarations` (`package.rs:634-660`), then re-parsed in `ResolvedProfileGraph::resolve` (`graph.rs:65`, `72`, `98`, `105`, `137`, `145`).
- The four unreachable variants are `ProfileGraphError::InvalidPackageVersion` (`graph.rs:220`), `InvalidEngineRequirement` (227), `InvalidDependencyRequirement` (247), and `InvalidIncompatibilityRequirement` (266).
- More parse sites: `commands/profile.rs:2283` and `2290` (`ensure_upgrade_version_is_newer_when_replaced`, deleted with upgrade). `ProfileApplicationInput.version` and `.compatible_jit` are `String` (`profile_apply.rs:1513-1517`). Result types carry `version: String` (`application.rs:45`, `:276`, and others).
- **Dependencies.** `semver = "1.0"` is already a direct dependency (`crates/jit/Cargo.toml:54`, lock 1.0.28), with only the `default` and `std` features. Serializing `semver::Version` and `VersionReq` in the model, which derives `Serialize` and `JsonSchema` (`manifest.rs:130`), needs `semver`'s `serde` feature and schemars' `semver` feature. schemars 0.8.22 declares an optional `semver` dependency (`~/.cargo/registry/.../schemars-0.8.22/Cargo.toml:365`). Both are currently off (`cargo tree -e features`).
- **Wire dependency.** Decoding happens in `profile/wire.rs` (`ManifestV2`), which 842b0ac6 REQ-06 rewrites.

### 1.5 e643ef66 failures, the code that produces each, and the D-8 deletion that removes each

| Failure | Producing code | Removed by |
| --- | --- | --- |
| 1. Editing a profile-owned gate fails validation, and the suggested remedy (capture into the package) worsens the state | The `profile-ownership` rule in `validation/repository.rs:668` (`collect_profile_ownership_findings`, rule name at :698), via `claimed_target_divergences` (`agreement.rs:335`). The remedy text is `ProfileResolution::CaptureRepositoryContent` (`profile_apply.rs:757-761`), attached by `ProfileTargetConflict::remedy` Diverged (680-683) and `ProfileDivergence::remedy` (`agreement.rs:83`). | "profile validation and its checks inside `jit validate`" and "conflict remedies" |
| 2a. Capture rewrites the contribution as a single-line inline table | `write_contribution_values` (`package_capture.rs:667-690`) inserts `Item::Value(value)` from `contributed_toml_value` (`profile_apply.rs:3478`), so an `[[contribution]]` value becomes `value = { ... }` inline. Reached from `refresh_contributions` (439) and `republished_manifest` (594). | "capture's reconciliation" |
| 2b. Capture leaves the applied record naming the old package hash, which then fails integrity | Capture republishes only the package tree, never `.jit/profiles/<id>.json`. `package_identity_divergence` (`profile.rs:2729`) and `ChangedPackageIdentity` (`agreement.rs`) then report it; `validate --fix` refuses at `commands/validate.rs:1003-1010`. | "applied-profile records" and "capture's reconciliation" |
| 3. Upgrade refuses with "defined differently by package sim-research", naming the same package twice | In `contribution_disposition` (`profile_apply.rs:2401-2459`), the "holding" branch: the registry holds a value whose fingerprint matches the package's own record claim, and the candidate moved. Owners = definers (candidate `sim-research`) ∪ recorded claimants (`sim-research`), deduplicated through a `BTreeSet`, so one package is rendered by `ProfileTargetConflict::message` (718-727). | "claims and fingerprints", "three-way publication", "upgrade" |
| 4. No registry marker shows which entries a profile owns | Ownership lives only in `AppliedProfileRecord.claims` (`profile_apply.rs:1395`, `.jit/profiles/<id>.json`). | "applied-profile records". After the deletion, no ownership concept exists to mark. |

Each failure is removed by a named D-8 Consequence. **Premise holds.**

---

## 2. Prior-art sweep

- **Charter D-8** (`dev/vision/9db27a3a-charter.md:145-186`, revised 2026-10-15 in commit 2917cf319). Index line at :31, mirrored at `AGENTS.md:110` and `CLAUDE.md:110`.
- **c639cfb5** (archived) built the machinery D-8 deletes.
  - REQ-05: records. REQ-07: three-way reconfigure/upgrade. REQ-13: capture. REQ-14: pack/add.
  - `dev/archive/c639cfb5-jit-profiles-complete/active/plan.md:39-128` is the contract list (selector, graph, ownership composition, applied record, three-way decision, capture tree, package archive); :217-232 is the decision table.
  - `c639cfb5-research.md` §5 (:468-590) argues for one source-neutral pipeline and one tagged selector. That is the prior design REQ-06 restores.
- **9b7b5f9c** (archived): MVP with the minimal `.jit/profiles/` record (REQ-05).
- **42c8c9c1** (done): ownership claims, aggregate publication, three-way reconfigure/upgrade. Children 7156f64b and 9a81ab12.
- **f2389b18** (done): `jit profile validate`, sharing logic with `jit validate`. D-8 deletes it.
- **5a3ecba0** (done): capture refreshes claimed contributions; unclaimed ones are reported as `unowned`. This is the origin of failure 2a. Owner rulings of 2026-08-13 are recorded.
- **Done issues usable as a deletion map:**
  - Records: 9a9a4cfc, 6c47b99a, 8371bd46 (validate `--fix` repair from the record), 44ec7192 (three-way decision).
  - Capture and diff: 6a479c58, acf49914, 96268a98 (diff).
  - Remedies: dd26c204, 7b820ccc, 03678c53.
  - Selection and graph: eac6ec13, 474a90a8, 7835fdc0, 5d12a79f, 1c0eb82e.
- **`dev/archive/4a559332-gf2-adopter-reliability/plan.md:98-126`**: the `profile-remedy-data` contract, deleted.
- **`dev/archive/e204e63d-derived-package-sources/`**: why `jit-dogfood` packages live assets drawn from repository files. These survive through `pack`.
- **`dev/architecture/repository-state-materialization.md:158-175`** describes `ApplyProfileSelection` / `derive_profile_applications`. **:197-214** describes repair and `AmbiguousOwnershipError` over two installed profiles; stale after D-8.
- **`dev/active/842b0ac6-greenfield-cutover/investigation.md:625-688`** (REQ-06 v1→v2):
  - It deletes `wire.rs:240`.
  - It changes both shipped package hashes.
  - This repository has no `.jit/profiles/`.
  - It couples to that epic's REQ-10 on `profiles/jit-dogfood/manifest.toml`.
- **`dev/active/df129d9e-rust-code-quality/`** is empty. Its epic touches `main.rs::profile_json_error` (one of ten JSON-error emitters, its REQ-11).
- **8376299c** has no profile content.
- **Older canonical-JSON origin:** `dev/archive/cdc840ad-repository-materialization/cdc840ad-handoff-6.md:24`, where the owner approved "local canonical JSON serialization". This is the likely origin of the `mutation.rs` copy.

---

## 3. Consumer sweep

### 3.1 Inside `crates/jit/src`: module classification after D-8

| File | Verdict | What changes |
| --- | --- | --- |
| `profile/agreement.rs` (485) | deleted | `profile validate` and the `jit validate` divergence classes, remedies, `ProfileDivergenceWire`/`Schema` |
| `profile/three_way.rs` (244) | deleted | three-way decision |
| `profile/apply_claims.rs` (225) | shrinks | `build_profile_repair_claims` and the record-claim building go. The construction of `ProfileContributionClaim` and the overlap validation (`validate_target_overlaps` 141) survive as composition input. |
| `profile/application.rs` (548) | shrinks | `ProfileSummary` (list), `ProfileShowEntry.applied`, `ProfileCaptureEntry` go. Plan and apply twins are merged by REQ-07. |
| `profile/difference.rs` (227) | survives, reshaped | `ensure_publishable_targets` and `profile_plan_entry` survive. Owners, retain and remove go. |
| `profile/variables.rs` (1484) | shrinks | `resolve_package_from_record` 324, `resolve_variables_from_record` 231, `RecordedValueAuthority` 206, `justifies_recorded_value` 402, `target_hashes` 466-505, and the canonical copy 512 go. |
| `profile/package_capture.rs` (1633) | shrinks | Capture-reconciliation pieces go: `refresh_contributions` 439, `published_identities` 505, `read_registry` 558, `republished_manifest` 594, `write_contribution_values` 667, `CapturedContribution*` 101-130, and record reads. The tree reader for `pack` survives. |
| `profile/package.rs` (2274) | survives | REQ-09 `ProfilePackageSource`, REQ-08 canonical copy, REQ-15 version parsing |
| `profile/package_archive.rs` (1337) | survives | pack and add |
| `profile/graph.rs` (414) | survives | Applied packages are no longer graph candidates. REQ-09/REQ-15 error variants. |
| `profile/manifest.rs` (587), `wire.rs` (459), `nofollow.rs` (113) | survive | wire is shrunk by 842b0ac6 REQ-06 |
| `profile/contribution_drift.rs` (988), `drift_report.rs` (253), `repository_package.rs` (1928, all `#[cfg(test)]`), `template_region.rs` (239) | survive, pending decision N5 | These are test-support checks that bind this checkout's `jit-dogfood` manifest to its own `.jit/` registries. They are not adopter records. |
| `repository_state/profile_apply.rs` (5149) | shrinks heavily | Deleted: records (1057-1510), fingerprints (1069-1365), `ProfileRemedy`/`ProfileResolution` (733-806), `remedy_suggestions` 993, `Diverged`, record-ownership composition (2197-2256, 2466-2760), `claimed_target_state` 2760-2877, asset retention and departure (1990-2196). Survives: contributions, composition, render, presence lookup. |
| `repository_state/initialize.rs` (1512) | shrinks | `profile_record_changed` 637, `serialize_profile_record` 1141, record/status classification in `profile_lifecycle_profile` 1081, `InstalledRecordConflict` 152, `.jit/profiles` directory creation 1057-1063 |
| `repository_state/materialize.rs`, `mod.rs` | shrink | record exports, `ApplyProfileSelection` keeps its shape |
| `repository_state/path.rs:305` | shrinks | `VirtualPath::PROFILES` goes |
| `domain/types.rs:1143-1166` | shrinks | `ProfileLifecycleOperation::{Reconfigure,Upgrade}`, `ProfileLifecycleStatus::{Reconfigured,Upgraded}`. `Installed` is defined as "no prior record existed", so it must be re-derived. |
| `domain/event_catalog.rs:421-430` | follows | sample event |
| `commands/profile.rs` (8699) | shrinks heavily | Deleted functions: list 614, validate 654, capture 744-854, reconfigure/upgrade 1427-1621, `resolve_applied_profile_packages` 1679, `read_applied_profile_record` 2191, `ensure_reconfiguration_package_identity` 2252, `ensure_upgrade_version_is_newer_when_replaced` 2275, and everything in 2377-2600 and 2631-2812 (`applied_record_*`, `capture_applied_records*`, `claim_capture_budget`, `recorded_*`, `package_identity_divergence`, `recorded_profile_agreement`), plus `read_applied_record` 2986. About 56 of 117 unit tests name records, claims, capture, reconfigure, upgrade or divergence. |
| `commands/init.rs` | shrinks | `record_path` at 404, `resolve_profile_graph` callers |
| `commands/validate.rs` | shrinks | `--fix` profile repair 650-1030, `with_profile_ownership_findings` 745, record readers 835-970, record list at 406. 842b0ac6 REQ-01 also edits this file (`validate_integrity_silent` 1835-2640), in a disjoint region. |
| `validation/repository.rs:639-700` | shrinks | `collect_profile_ownership_findings`, `PROFILE_OWNERSHIP_RULE` |
| `cli.rs:2966-3330` | shrinks | `List`, `Validate`, `Reconfigure`, `Capture`, `Upgrade` variants go. The `Show` `id` argument goes. Doc comments of `Diff` (3016-3041) name "recorded claims it would retain or remove". |
| `main.rs` | shrinks | dispatch 2623-2981, `render_profile_agreement` 1192-1250 (with ✓/✗ emoji, overlapping REQ-05), `NotFoundError` suggestion naming `jit profile list` (1263), `ProfileDependencyError` suggestion "apply that package first so this repository records where its bytes are" (1280-1283) |
| `schema.rs:478-530` | shrinks | `profile_list`, `profile_validate`, `profile_capture`, `profile_reconfigure`, `profile_upgrade` arms; the hand-written `oneOf` |
| `output.rs:966-970` | shrinks | `ProfileConflict` description links `profiles.md#the-remedy-contract` |

### 3.2 Outside `crates/jit/src`

**Tests (16 files).**

- `crates/jit/tests/cli_repo_workflow/`:
  - `profile_acceptance_tests.rs`: `profile validate` tests 1185-1426; schema pin 792-1017; record presence at 269, 387, 534, 1104; `id:` at 288; diff owner vocabulary 1499-1750.
  - `profile_cli_tests.rs`: list/show, reconfigure/upgrade, capture, record and claim assertions (see the sweep list in this section's source report). About 25 tests are deleted or rewritten.
  - `profile_edit_refresh_cli_tests.rs`: the whole file is deleted.
  - `profile_no_git_lifecycle_tests.rs:46`: rewritten for apply/diff/pack/add only.
  - `profile_publication_tests.rs`: the `RECORD_PATH` and ownership snapshot at 25 and 36-81.
  - `profile_concurrency_tests.rs:163`.
  - `integration_schema.rs`: 60-117, 145-186.
  - `help_cross_reference_tests.rs:115`.
  - `repo_discovery_tests.rs`: 109, 121, 150, 189.
- `cli_item_validate/derived_state_repair_tests.rs`: 110, 226, 327, 378.
- `fast_rules/derived_state_repair_tests.rs`: 216, 365, 445-825 (10 tests).
- `cli_issue/failure_lever_registry.toml`: 674-762 (6 arms).
- `cli_issue/failure_probe_fixture.rs`: `MalformedProfileRecord` 626-630, 781, 818, 864.
- `scratch_build/repository_state_cutover_tests.rs`: 159-166, 418, 1283-1344, 1382-1394 (also claimed by 842b0ac6 REQ-13), 1513-1534, 1685, 1759.
- `common/harness.rs:28-31` (doc comment).
- `fast_rules/config_loading_tests.rs:115` (wording only).
- Fixtures under `tests/fixtures/profile-packages/` hold no deleted items.

**Other code and configuration.**

- `crates/server`: none.
- `web/`: none.
- `mcp-server/`:
  - `curated-tools.json:56-63`: list, show, validate, capture, reconfigure, upgrade. The surviving diff (:59) and apply (:60) entries say "claims" and "structured remedy".
  - `test-integration.js:404-409`, `597-725`.
  - `test-unit.js:465-478` (schema-derived).
  - `README.md:91-94`.
- `scripts/docs-check-package-content.sh:141-151` runs `profile capture` and `profile show`. It is wired as `scripts/docs-mechanical.sh:103` (docs-mechanical gate, M8) and in `docs-check-selftest.sh:43,84,100,420`.
- `.github/workflows/release-artifacts.yml`:
  - :122-140 builds the release package with `profile capture`.
  - :269 runs `profile list --json` (asserted at 280-294).
  - :270 runs `profile apply --profile id:jit-dogfood --dry-run`.
- `.github/workflows/ci.yml:287-315`: the `profile-adoption` job runs `profile_acceptance_tests::`. Also `.github/workflow-contract.yml:67`.
- `.config/nextest.toml:22` names tests in `commands::validate::tests` that are deleted.
- `.agents/skills`: only `id:` selectors (`jit-manage/SKILL.md:82`, `jit-migrate/SKILL.md:26`, `jit-project-lead/SKILL.md:200`).
- `profiles/`: none. The `[[live-source]]` roots (`jit-dogfood/manifest.toml:524-548`) serve `pack`.
- `.jit/`: no `profiles/` directory, no `profile_lifecycle` event in `events.jsonl`, and no gate calls `profile validate`. The deletion drops nothing from this repository.

**Docs.**

- `docs/reference/profiles.md`, about 55 lines:
  - :7-10 intro.
  - :24-26 the record.
  - :179-182.
  - :223-277 list, show, validate, capture, `id:`.
  - :328-397.
  - :398-450 "The remedy contract".
  - :457-462.
  - :491-514 `--fix` provenance repair.
  - :524-570 publication and records.
- `docs/reference/cli-commands.md`, about 80 lines:
  - :546-555 init `id:`.
  - :631-648 preamble.
  - :656-737 list, show, validate.
  - :751-766 diff claims and remedy.
  - :791-830 apply `id:`.
  - :832-967 reconfigure, upgrade, capture.
  - :977, 1041, 1044 pack/add references to capture.
  - :3463, 3526 validate provenance.
- Other reference pages:
  - `storage-format.md:27`, `87-89`.
  - `worktree-validate.md:319-336`, `392-395`.
  - `error-codes.md:34`.
  - `cli-command-grammar.md:86-89`.
  - `jit-content-standards.md:12-13`.
  - `concepts/guarantees.md:160`, `661-662`.
  - `docs/index.md:53`.
  - `events.md:21`, `48` (the operation set shrinks).
- `dev/`:
  - `dev/index.md:112-138` ("Capturing the Workflow Package").
  - `dev/TESTING.md:455-512` (timing table names deleted tests).
  - `dev/architecture/repository-state-materialization.md:197-214`.
- `CHANGELOG.md` [Unreleased]: 11-12, 114-150, 248-252, 291, 317-341, 354, 429-436, 653-663. These need a removal entry.

---

## 4. Primitive verification

**`serde_json` `preserve_order` is off.**
- `Cargo.toml:41` declares `serde_json = "1.0"`, resolved to 1.0.145.
- The `Cargo.lock` entry lists dependencies `itoa`, `memchr`, `ryu`, `serde`, `serde_core`. There is no `indexmap`, which `preserve_order` would add.
- `cargo tree -e features -i serde_json --offline` shows only the features `default`, `std` and `raw_value`.

**Identity transform.** With `preserve_order` off, `serde_json::Map<String, Value>` is a `BTreeMap`. The `canonicalize*` helpers re-collect each object through a `BTreeMap` (`package.rs:991`, `wire.rs:240`, `variables.rs:520`, `profile_apply.rs:1371`) or a sorted `Vec` (`mutation.rs:588`). Either way they rebuild an already-sorted map: identity.

The order-bearing step is `serde_json::to_value(&T)`, which builds that sorted map. `mutation.rs` byte output is pinned by the on-disk format: issues, gate runs and events are sorted-key JSON. The one survivor must therefore be `to_value` followed by `to_vec`/`to_vec_pretty`, not a direct `to_vec(&T)`.

**Making it fail when `preserve_order` is enabled.** serde_json exposes no `cfg` for the feature. Two options:

- **A unit test (recommended).** Build `serde_json::Map` by inserting `"b"` then `"a"`, and assert that the first key iterated is `"a"`. Or serialize a struct with fields declared `b, a` through the canonical function and assert the bytes are `{"a":…,"b":…}`. Under `preserve_order` the map keeps insertion order and the test fails.
- **A compile-time alternative.** A `const` assertion is impossible, because `Map`'s backing type is private. A build-script check of `cargo metadata` features is heavier than the test.

**schemars and derive (REQ-12).**
- schemars 0.8 is in the workspace with features `chrono` and `derive` (`Cargo.toml:42`).
- Every `profile/application.rs` result type derives `Serialize` and `JsonSchema`.
- Hand-written impls in scope:
  - `impl Serialize for ProfileDivergence` and `impl JsonSchema for ProfileDivergence` (`agreement.rs:157`, `:233`): deleted.
  - `impl Serialize for ProfileTargetConflict` (`profile_apply.rs:827`): exists only to add the computed `remedy`; derivable once remedy goes.
  - `impl Serialize for ProfileTargetSubject` (`profile_apply.rs:916`): exists only to render `VirtualPath` as `repository_relative()`; derivable with `#[serde(serialize_with)]` or a path newtype.
- `ProfileTargetConflictsError` (`profile_apply.rs:969`) and `ProfileTargetConflictEntry` (936) derive `Serialize` but not `JsonSchema`. They reach JSON as error `details` (`main.rs:1290`) without a published schema.
- The `profile_apply`/`reconfigure`/`upgrade` output schema is a hand-written `json!({"oneOf": …})` (`schema.rs:514-530`).

**Git primitives (for D-8's refuse-on-uncommitted).** No production code detects uncommitted files.
- `GitRevisionResolver::changed_worktree_paths` (`storage/git_revision.rs:537`) is test-only. 842b0ac6 REQ-18 deletes it, and f4d9502a (D-21) replaces the whole git access layer.

**semver and schemars features:** see 1.4.

---

## 5. Architecture fit

**One selection path (REQ-06).** It belongs in the commands layer, split in two:

- **A pure domain function** that takes `(selected packages, candidate dependency packages, engine)` and returns the resolved graph. This already exists: `ResolvedProfileGraph::resolve` (`profile/graph.rs:50`), which is I/O-free.
- **One command-layer `ProfileSelection`.** It is built once per invocation from `(&[ProfileSelector], &ProfileVariableOptions)`. It resolves selectors (filesystem), loads dependencies beside the declaring package (filesystem), resolves the graph once, loads and validates variable inputs once, and resolves package content once. It holds `selected`, `packages` and `resolved`, and the contribution context derived from them.

Plan, apply, diff and init all take this value. Init's preflight (`main.rs:2552`) becomes building the selection before the session opens, which removes the third resolution. `apply_profile_selection` / `prepare_profile_selection` then take the selection instead of four parallel slices with a length check (`profile.rs:2050`).

After D-8, applied packages leave the graph. `resolve_profile_graph_with_applied` (1640) and the record fallback in `resolve_dependency_package_from` (1757-1790) are removed. Graph inputs become the selection's own closure, which also shrinks the I/O.

**Plan as a read-only apply (REQ-07, D-20).**
- Today both routes call `prepare_profile_selection`, but only apply runs final validation (`profile.rs:1891-1900`, `FinalValidationFindings`). A `--dry-run` therefore does not fail where the apply would.
- The fit: one `run_profile_selection(selection, mode)` returns one result type, where mode is publish, rehearse or report. The rehearsal returns the plan instead of `SessionStep::Apply`.
- `ProfileConflictResponse::{Refuse,Report}` (`profile.rs:104`) is already the diff/rehearsal switch.

**Canonical JSON (REQ-08).** The survivor belongs in the storage or repository-state serialization layer (`mutation.rs`), exported `pub(crate)`. Profile package hashing in `profile/package.rs` would import it from `repository_state`. That direction is already allowed: `profile` already imports `repository_state` types (`Contribution`, `VirtualPath`).

**Registry declaration (REQ-11).** `ContributionRegistry` (`profile_apply.rs:139`) should be the one declaration, with:
- `path()` derived from `VirtualPath::{CONFIG,GATES,...}`;
- `KeyedArrayTarget::registry()` returning a `ContributionRegistry`;
- `Contribution::registry()` replacing `registry_path` and the match in `semantic_identity`.

`initialize.rs:332-336` can reuse the `VirtualPath` constants.

**Domain-agnostic** (`@/inv/domain-agnostic`). The registry vocabulary (`config`, `gates`, `rules`, `templates`, `invariants`) is engine vocabulary for its own registries, so it is mechanism. No new instance content is introduced. The "dependent wins" ordering (N2) derives from manifest-declared dependencies, so it is mechanism too.

**Layering** (`@/inv/architecture-dependency-direction`).
- `ProfileDependencyError` is a command-layer type that mirrors a pure-layer error. Merging into `ProfileGraphError` (pure), plus one command-layer `UnresolvableDependency` (loading needs I/O causes), keeps direction intact.
- `main.rs` would then downcast `ProfileGraphError`. Today unmapped variants fall to the generic code (C16).

---

## 6. Invariant check

- **canonical-cutover.** D-8 states that records are dropped with no migration. This repository holds none. Deleted CLI subcommands, schema arms, MCP curated tools and docs go in the same change as the code. No alias for `id:` remains (decision N1).
- **convention-convergence.** These parallel variants are defects to fold:
  - the five canonical copies;
  - two conflict-origin enums;
  - two status enums;
  - two result types plus a bridge trait;
  - two package types;
  - two dependency error types;
  - five presence lookups;
  - four registry path tables.
  `pack` reading through `capture_package_tree` while `apply` reads through `ProfilePackage::from_directory` is a further pair. REQ-09 covers it.
- **semantic-types.** Covered by REQ-14 and REQ-15. Also:
  - `ProfileApplyResult.id: String` and `ProfilePlanEntry.id: String` (`application.rs:43`, `:274`) and `owners: Vec<String>` (201, 247) are raw strings at a serialization boundary. They are allowed, but the typed `ProfileId` serializes identically.
  - `ProfileSelector::Id` after D-8 has no meaning (N1).
- **orthogonal-tests.**
  - The 76 `apply_profile_package` unit tests in `commands/profile.rs` re-prove composition properties that `profile_apply.rs` tests own (for example conflict independence of order: `profile.rs:5697` vs `profile_apply.rs:4085`). The REQ-13 migration should drop duplicates instead of porting them.
  - The schema pins in `profile_acceptance_tests.rs:792-1017` and `integration_schema.rs:60-117` duplicate each other.
- **essential-code-prose.**
  - `profile/mod.rs:1-5` says the module holds no application lifecycle; it does. This is stale.
  - Many rustdoc blocks describe record semantics, for example `contribution_disposition` 2370-2399 and `cli.rs` Diff 3016-3041. They go with the code.
- **atomic-writes / derived-state-coherence.** The apply refusal on uncommitted files is a read check made before the session. It must not weaken the redo-record path. The refusal reads git state; the preimage check stays the concurrency guard.
- **D-4.** Without git, apply cannot know what is committed. D-8's reasoning (charter :170-172) says that there `jit profile diff` is the comparison to read before applying, so apply proceeds. This needs a test in `profile_no_git_lifecycle_tests.rs`.

---

## 7. Cross-epic overlap

| Epic | Shared code | Conflict | Ordering |
| --- | --- | --- | --- |
| 2195c98f REQ-09 (domain `ProfileId`) | `ProfileId` (`manifest.rs:26`) moves to the domain; consumed by this epic's REQ-14 | Our REQ-14 replaces `ProfilePackageId` with it | 2195c98f REQ-09 first, then our D-8 deletion (removes about two thirds of the sites), then our REQ-14 |
| 2195c98f record path and readers (8 sites, 7 readers) | `profile.rs:2466`, `:2986`, `profile_apply.rs:2039-2122`, `initialize.rs:636,1080` | Mooted by our D-8 deletion | Our D-8 deletion before any 2195c98f work there. Tell 2195c98f to drop those items. |
| 2195c98f REQ-08 (reserved target predicate) | `commands/profile.rs:3018`, `init.rs:400` | Same functions our selection rewrite touches | 2195c98f REQ-08 after our REQ-06, or include it in REQ-06 |
| 842b0ac6 REQ-06 (manifest v2) | `profile/wire.rs` (`canonicalize` :240, `decode_v1`), `package.rs` hash oracle :1225-1243 and pinned hash :1465, both shipped manifests, fixtures | Our REQ-08 counts `wire.rs:240`; our REQ-15 rewrites wire version types | 842b0ac6 REQ-06 first (small). Then REQ-15 builds on `ManifestV2`. |
| 842b0ac6 REQ-10 (gate `auto`) | `profiles/jit-dogfood/manifest.toml` | Package hash churn | Not ours; serialized within 842b0ac6 |
| 842b0ac6 REQ-01 | `commands/validate.rs` 1835-2640 | Our D-8 deletes 650-1030 in the same file | Disjoint hunks; either order, but not in parallel worktrees without a rebase |
| 842b0ac6 REQ-13 | `repository_state_cutover_tests.rs:1382` (profile record authority guard) | Our D-8 deletes the record half of that test | Coordinate: whoever lands second edits that test |
| 842b0ac6 REQ-18 | `git_revision.rs::changed_worktree_paths` (test-only, deleted there) | The D-8 refusal needs a dirty-path query | Do not resurrect it. Add the query to f4d9502a's module. |
| f4d9502a (D-21 git CLI module) REQ-02, REQ-11 | All git access; module exposes only production-called operations | The D-8 dirty-file refusal is a new git operation | Land the refusal after f4d9502a's module exists, or add the operation to `git_revision.rs` and let f4d9502a port it (N7) |
| df129d9e REQ-11 | `main.rs::profile_json_error` (one of ten JSON-error emitters) | We delete its record-specific suggestion texts and the `ProfileDependencyError` downcast | Our D-8 slice first (it shrinks the function), then df129d9e |
| f368ce3d (domain-agnostic) | No profile code | None | |
| 8376299c | None | None | |
| 3d9a0ce3 REQ-05 (emoji) | `main.rs:1240-1248` `render_profile_agreement` ✓/✗ | Deleted by our D-8 slice | D-8 first, or REQ-05 skips it |

---

## 8. Decomposition sketch (input for the plan)

Hot files and how to avoid parallel conflicts:
- `commands/profile.rs` (8.7k lines; 5.6k are tests) and `repository_state/profile_apply.rs` (5.1k): never edit either in two parallel slices. Every slice below touches at least one of them, so the profile track is **serial**.
- `main.rs`, `cli.rs`, `schema.rs` and docs are also shared with other epics. Merge them often.

| # | Slice | Footprint | Test boundary | After |
| --- | --- | --- | --- | --- |
| P0 | (external) 842b0ac6 REQ-06 manifest v2 | `wire.rs`, `package.rs` tests, profiles, fixtures | its own | — |
| P1 | **Delete record-dependent commands**: `list`, `validate`, `capture` (command plus contribution refresh), `reconfigure`, `upgrade`. Remove the `id:` selector (N1) and the `show` `id` argument and `applied` field. Remove CLI variants, dispatch, schema arms, MCP curated entries and tests, related docs sections, the release workflow (replace capture with `pack`, or a plain copy plus `pack`), and `docs-check-package-content.sh`. | `cli.rs`, `main.rs`, `schema.rs`, `commands/profile.rs`, `package_capture.rs`, `variables.rs` (record resolution), `agreement.rs` (deleted), `mcp-server/*`, `.github/workflows/release-artifacts.yml`, `scripts/docs-check-package-content.sh`, `docs/reference/{profiles,cli-commands,cli-command-grammar,jit-content-standards}.md`, skills, `dev/index.md` | Schema shows only apply, diff, show, pack, add. The release smoke and docs gate pass. | P0 |
| P2 | **Delete records, claims, fingerprints, three-way, remedies.** Remove `AppliedProfileRecord` and claims, `ProfileBaseFingerprint`, `three_way.rs`, `ProfileRemedy`/`ProfileResolution`, `Diverged`, the record-ownership composition, `.jit/profiles/` writes, `VirtualPath::PROFILES`, `InstalledRecordConflict`. Re-derive the event `ProfileLifecycleStatus`. Shrink `ProfileLifecycleOperation`. | `profile_apply.rs`, `initialize.rs`, `materialize.rs`, `repository_state/{mod,path}.rs`, `domain/types.rs`, `event_catalog.rs`, `apply_claims.rs`, `difference.rs`, `commands/{profile,init}.rs`, `main.rs` suggestions, `output.rs:966-970`, `docs/reference/{profiles,storage-format,events,error-codes}.md`, `guarantees.md` | No `.jit/profiles` path is written. Repeat apply reports unchanged when content is equal. The event names the profile, version and variables (N8). | P1 |
| P3 | **Delete validation integration**: the `profile-ownership` rule, `validate --fix` profile repair and its readers | `validation/repository.rs:639-700`, `commands/validate.rs:406,549,650-1030`, `fast_rules/derived_state_repair_tests.rs`, `cli_item_validate/derived_state_repair_tests.rs`, `docs/reference/worktree-validate.md`, `.config/nextest.toml`, `dev/architecture/repository-state-materialization.md:197-214`, `dev/TESTING.md` | `jit validate` emits no profile rule. `--fix` reads no package. | P2 (can merge with P2 if one worker) |
| P4 | **New D-8 behavior**: (a) replace semantics in the registry renderers, where the package value overwrites an existing entry key (N3); (b) the dependent package wins a key both declare; unrelated packages that differ are refused (N2); (c) apply refuses while the files it writes carry uncommitted changes, with git only, no-op without git (N7) | `profile_apply.rs` composition and render, `commands/profile.rs`, git module | Re-apply of a newer version over a committed edit overwrites it. A dirty target refuses with exit 4. No-git apply proceeds. Dependent-wins and unrelated-conflict property tests (proptest over order independence). | P2; (c) after f4d9502a or coordinated |
| P5 | REQ-06 + REQ-10 + REQ-13: one `ProfileSelection` built once. Delete dead and test-only entry points. Validate inputs once. `validate_model_references` once (drop `variables.rs:311`). Migrate the 76 `apply_profile_package` tests to the surviving entry, dropping duplicates. | `commands/profile.rs`, `commands/init.rs`, `main.rs:2552`, `variables.rs` | A counting test or harness probe: selectors and graph resolved once per invocation, inputs built once, one capture where possible | P2 (P4 independent of P5 conceptually, but same files, so serial) |
| P6 | REQ-07 + REQ-12: one result type and one status enum for plan, apply and diff. Delete `SelectionObservation`, the `oneOf` schema, `ProfileTargetConflictWire`, `ProfileTargetSubjectWire`. Rehearsal runs final validation. | `application.rs`, `difference.rs`, `profile_apply.rs`, `schema.rs`, `main.rs` printers, `profile_acceptance_tests.rs` schema pin | A dry run fails exactly where apply fails, including final validation. The published schema is derived. | P5 |
| P7 | REQ-09: merge `ProfileDependencyError` into `ProfileGraphError` and fix the `main.rs` downcast. One conflict-origin type. Remove the single-variant `ProfilePackageSource` (and `ProfileSelector`, `ProfileOrigin` if N1 leaves them single-variant). One validated package type (`CapturedPackageTree` folded into `ProfilePackage`, with files kept for pack/add). | `graph.rs`, `commands/profile.rs`, `main.rs`, `package.rs`, `package_capture.rs`, `package_archive.rs`, `domain/types.rs` | Every graph refusal maps to `PROFILE_ERROR`. Pack, add and apply read through one type. | P6 |
| P8 | REQ-10 (registry) + REQ-11: one registry declaration. One parsed registry document per composition. All renderers go through `contribution_in_registry`. | `profile_apply.rs` | A parse-count probe, or a structural test that one `DocumentMut` per registry is used per composition | P4(a) |
| P9 | REQ-08: one `canonical_json` in `repository_state` (from `mutation.rs`) plus the preserve-order guard test. Remove the `package.rs:980` copy. The `variables.rs` and `profile_apply.rs` copies are already gone with P2. | `mutation.rs`, `package.rs` | A test that fails under `preserve_order` (section 4) | P0, P2 |
| P10 | REQ-14: `ProfileId` everywhere `ProfilePackageId` survives. REQ-15: semver `Version`/`VersionReq` in the model (enable `semver/serde` and `schemars/semver`), delete the four `Invalid*` variants. | `profile_apply.rs`, `apply_claims.rs`, `difference.rs`, `materialize.rs`, `manifest.rs`, `wire.rs`, `graph.rs`, `Cargo.toml` | `graph.rs` has no parse error variant; decode rejects a bad version | 2195c98f REQ-09, P0, P7 |
| P11 | Docs and CHANGELOG sweep for the surviving profile surface (`profiles.md` rewritten around apply-once, diff, pack, add) | `docs/`, `CHANGELOG.md`, `mcp-server/README.md` | docs gates | P1-P4 |

P1 to P3 can be one worker sequence, or one larger slice, since each leaves the build green only if consumers go with them. P9 is the only slice independent of the hot files, apart from `package.rs`, so it can run in parallel with P5-P7 if `package.rs` is not edited concurrently.

### Needs decision (carried to the plan)

- **N1 — `id:` selector.** It resolves only through the applied record (`profile.rs:546-559`).
  - (a) Delete it; selectors are paths only, and `ProfileSelector` becomes a plain path.
  - (b) Redefine `id:` as a named package directory under a declared packages root.
  - Recommend (a) per D-18. It touches skills, the release workflow and docs.
- **N2 — "dependent package wins a key both declare".** This is not implemented today. `compose_contribution_identity` (`profile_apply.rs:350-383`) contests every differing definition. Confirm it is in REQ-04 scope (D-8 says so) and that it is new work.
- **N3 — "writes the new rendering".** The renderers are insert-only today; an existing entry is preserved "whatever it says" (`profile_apply.rs:3101-3276`, rustdoc 2370-2399). D-8 requires the package value to replace the existing entry. Confirm the semantics:
  - (a) replace the whole entry under the identity;
  - (b) deep-merge.
  - Recommend (a).
- **N4 — conflict parties.** After D-8 the repository is never a conflict party. The conflict-origin type can become `Vec<ProfileId>` instead of a merged enum. Recommend that.
- **N5 — repository-local drift checks.** `contribution_drift.rs`, `drift_report.rs`, `repository_package.rs` and `template_region.rs` keep this checkout's `jit-dogfood` manifest in sync with `.jit/`, which is two copies kept in sync. They are not adopter records, so D-8 does not name them.
  - (a) Keep them.
  - (b) Delete them and make `jit-dogfood` the authority applied to this repository.
  - Recommend (a) for this epic, with a follow-up.
- **N6 — `profile show` target hashes.** These are the only remaining consumer of per-target hash values (`application.rs:149`). Recommend dropping the values and keeping the target set. That deletes `package.rs:910-971` value hashing and the `package.rs:980` canonical copy.
- **N7 — uncommitted-change refusal.**
  - Which files: the files the apply writes.
  - Does it apply to the first application and to `init --profile`?
  - The no-git behavior: proceed.
  - Where the git query lives: f4d9502a's module.
  - Recommend: refuse on any written path that is dirty against HEAD; skip without git; init exempt when the data root is absent.
- **N8 — event content.** D-8 says the event "records what was applied". `ProfileLifecycleProfile` (`domain/types.rs:1181`) has id, status and variables, but no version or package hash. Recommend adding the version.
