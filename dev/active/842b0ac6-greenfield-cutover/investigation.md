# Investigation: Remove dead code and compatibility shims (842b0ac6)

Grounding for planning node 201634a8, verified against main at a41eaacf3. Three slices: A (library and storage, REQ-01/02/05/07/08/13/18/19/20/23/24), B (configuration, profile wire, lifecycle tokens, gates, archive, mermaid, REQ-03/04/06/09/10/11/12), C (domain, output, document, web, REQ-14/15/16/17/21/22/25). Each slice owns the consumer inventory for its criteria.

## 842b0ac6 investigation — group A (library/storage dead code)

Verified against `main` at `c0113c6b4` (one `.jit`-only commit past `a41eaacf3`; no source drift from the audit baseline for the cited lines unless noted).

Legend: **done** = already true at HEAD; **open** = valid and open; **invalid** = invalid as stated.

---

### REQ-01 — breakdown module, `validate_integrity_silent` + helpers, `drift::enforcement_drift`

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `commands/breakdown.rs` (547 lines) reached only from tests | open | 547 lines; `mod breakdown;` private (`commands/mod.rs:31`), re-export `pub use breakdown::{BracketBreakdownResult, BracketChild}` (`commands/mod.rs:74`). No `src/` caller of `bracket_breakdown*`. |
| opens one mutation session per child and one per edge | open | per-child `self.create_issue(` (`breakdown.rs:406`), per-edge `self.add_dependency(` (`breakdown.rs:424,433,450`). |
| `validate_integrity_silent` + three private helpers, `validate.rs:1835-2640`, ~230 lines, no caller | open | fn `validate.rs:1835-1916`; helpers `validate_document_references` `2466-2545`, `check_file_exists_in_git` `2547-2561`, `validate_transitive_reduction` `2590-2638`. Each helper's only caller is `validate_integrity_silent` (`1876`, `1903`; `2503`,`2519`). Total ≈245 lines incl. doc comments. |
| re-implements integrity checks in `validation/repository.rs` | open | `repository.rs:457-503` (documents, DAG, isolated, transitive reduction), `550` (claims index). |
| one helper opens a git2 repository against the process cwd | **invalid** | `Repository::open(layout.worktree_root())` (`validate.rs:2470-2471`) opens the *selected* worktree. The cwd dependence is the working-tree fallback `std::path::Path::new(&doc.path).exists()` (`2521`, `2532`), which resolves the repo-relative path against process cwd. Net effect same (removal), wording wrong. |
| `drift::enforcement_drift` (`drift.rs:107`) called only by its own tests | open | Only callers `drift.rs:252-376` (11 tests). Production uses `enforcement_drift_tolerant` (`commands/validate.rs:1743,1777`; `validation/repository.rs:613,618`). |

#### 2. Consumer sweep
- `bracket_breakdown`, `bracket_breakdown_with_template`, `BracketChild`, `BracketBreakdownResult`:
  - `crates/jit/tests/fast_docs_templates/bracket_breakdown_tests.rs` (863 lines, whole file; `mod bracket_breakdown_tests;` at `fast_docs_templates/main.rs:15`).
  - `crates/jit/tests/fast_docs_templates/template_binding_tests.rs:23,162-175,207-220,272-278` (two tests call `bracket_breakdown`; module doc `:13-21` names it).
  - Prose only: `tests/cli_repo_workflow/template_binding_cli_tests.rs:18-20` (module doc), `tests/cli_gate/bracket_coverage_gate_run_test.rs:16-18,151` (doc comments "the shape `bracket_breakdown` produces").
  - Doctest inside `breakdown.rs:32-56`.
  - No mention in `docs/`, `dev/` (outside archive/benchmarks), `.agents/skills`, `mcp-server`, `web`, `.jit/schemas`.
- Orphaned by deletion: `CommandExecutor::template_roles` (`commands/template.rs:103`, `pub(crate)`), whose only caller is `breakdown.rs:327`; its doc comment ("every consumer (apply, refresh, breakdown, validation)") becomes false. `find_keyed_cycle`, `declared_container_label` keep other callers.
- `validate_integrity_silent` + helpers: no caller anywhere; only `dev/archive/...audit-notes.md:123` (archive, leave).
- `enforcement_drift`: drift.rs tests; doc links `drift.rs:149` ("Semantics extend [`enforcement_drift`]").

#### 3. Primitive verification
- "Nothing else reads X": confirmed for all three. `validate_claims_index_with_paths` stays (also called from `validate_claims_index_at`, `validate.rs:3045`). `require_worktree_paths` stays (many callers).
- After deletion, `validate.rs` has no `git2` use (its two git2 lines are in the deleted helpers); git2 remains in `document.rs`, `snapshot.rs`, `storage/json.rs`.
- Property coverage that disappears: template_binding_tests' renamed-role path through breakdown. The surviving renamed-role coverage is `apply_template` + `--force` refresh in the same tests; nothing else in production consumes breakdown, so no rewrite target exists — delete the breakdown halves of those two tests.

#### 4. Architecture fit
- `enforcement_drift_tolerant` becomes the sole entry; its `_tolerant` suffix only distinguished it from the deleted twin (see Open decisions).
- 11 drift.rs tests rewrite onto `enforcement_drift_tolerant(&inv, SourceState::Loaded(&rules), SourceState::Loaded(&gates))` — same pure layer.

#### 5. Invariant check
No domain-agnostic concern; deletion only. `@/inv/essential-code-prose`: fix `template.rs:97-101`, `drift.rs:149`, the three test module docs.

#### 6. Footprint
| File | Change | ≈Lines |
|---|---|---|
| `src/commands/breakdown.rs` | delete | −547 |
| `src/commands/mod.rs` | drop `mod breakdown;`, re-export | −2 |
| `src/commands/template.rs` | delete `template_roles` | −9 |
| `tests/fast_docs_templates/bracket_breakdown_tests.rs` + `main.rs:15` | delete | −864 |
| `tests/fast_docs_templates/template_binding_tests.rs` | drop breakdown halves, `BracketChild`, `child()`, doc | −50 |
| `tests/cli_repo_workflow/template_binding_cli_tests.rs`, `tests/cli_gate/bracket_coverage_gate_run_test.rs` | prose | ~−8 |
| `src/commands/validate.rs` | delete fn + 3 helpers | −245 |
| `src/validation/drift.rs` | delete fn, rewrite 11 tests, fix doc | −15 net |
Split point: (a) breakdown, (b) validate+drift. Independent.

#### 7. Overlap
- **6ff14f21** (InProgress, worktree `agent-6ff14f21`, uncommitted): edits `validate.rs:1906` (inside `validate_integrity_silent`), `bracket_breakdown_tests.rs:656`, `template_binding_tests.rs:108,293`, `template_apply_atomicity_tests.rs:392`. Conflict resolves by deletion; land 6ff14f21 first or tell its worker the hunks are going away.
- **f4d9502a** Background cites `validate.rs:2467`, `2549` (libgit2 doc-reference check) — REQ-01 deletes them; f4d9502a must sequence after or drop that bullet.

---

### REQ-02 — linked-worktree evidence

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| capture requested only by tests (`CaptureSpec::discover_linked_worktree`, `image.rs:719`) | open | callers `storage/repository_state_store_tests.rs:160`, `image.rs:1746` (tests). |
| every JSON capture calls `configured_worktree_paths()` (`repository_state_store.rs:597`) | open | `capture_over` `:597`, threaded to `capture_capability_image` (`:1108`) → `capture_linked_evidence` (`:1583`, used `:1634-1635`). That is the param's only use. |
| which can spawn git | open | `JsonFileStorage::configured_worktree_paths` (`storage/json.rs:560`) → `WorktreePaths::detect_for_data_root` → `Command::new("git")` (`worktree_paths.rs:64`); cached per storage after first call. |

#### 2. Consumer sweep (full chain)
- Capture-spec entry: `CaptureSpec.linked_worktree: BTreeSet<VirtualPath>` (`image.rs:630,645`), `discover_linked_worktree` (`:719-722`), `linked_worktree()` (`:752`), spec path/validation chains (`:771,779,783-...`).
- Image field: `RepositoryImage.linked_worktree` (`image.rs:816`), `close(..., linked_worktree)` param (`:827`, checks `:891-904`), `linked_worktree_evidence()` (`:972`), `into_answers` (`:984,994`), `CapturedAnswers.linked_worktree` (`:805`), `captured_byte_count` param (`:1065,1083`).
- Evidence types: `LinkedWorktreeSourceClass` (`image.rs:503-515`), `LinkedWorktreeEvidence` (`:517-584`, `Serialize`).
- Errors: `CaptureError::{IncompleteLinkedEvidence, UnexpectedLinkedEvidence, LinkedEvidenceRequestMismatch, LinkedEvidenceRequiresDataPath, InvalidLinkedEvidence}` (`image.rs:1541-1550`).
- Re-exports: `repository_state/mod.rs:60-61`.
- Gate-evidence field: `GateEvaluationEvidence.linked` (`commands/gate_check.rs:91-94`), filled `:412-417,425`. Struct is `PartialEq` only (compared for reuse); not serialized.
- Overlay: `repository_state/overlay.rs:62` (and its `close` at `:93`).
- Store: `repository_state_store.rs:10` imports, `:597-601` param, `:1105-1229` (`capture_capability_image` linked block `:1218-1228`), memory capture linked block `:1283-1307`, `capture_linked_evidence` `:1580-1659`.
- `RepositoryImage::close(` call sites passing a linked map (each loses one argument): `repository_state_store.rs:1229,1309`; `repository_state/{initialize.rs:1328, overlay.rs:93,198, materialize.rs:727,748, export.rs:334,472, mutation.rs:1417,1767, profile_apply.rs:3677,4722,4881,4945, mod.rs:1265,1328,1428,1441, archive.rs:638}`; `document/reference.rs:359`. 21 sites.
- Tests: `repository_state_store_tests.rs:140-166` (linked→main-worktree evidence test), `:2791` (test harness calls `configured_worktree_paths` to feed `capture_capability_image`), `image.rs:1713-1750,2253-2258`, `tests/fast_docs_templates/template_apply_atomicity_tests.rs:768`.
- Docs: `dev/architecture/repository-state-materialization.md:89` ("and pinned or linked-worktree evidence").
- No consumer in `docs/`, `web/`, `mcp-server/`, `crates/server`, `.jit/schemas`, `jit --schema`.

#### 3. Primitive verification
- `configured_worktree_paths` itself stays: other callers `storage/json.rs:870,935` (primary data root for HEAD fallback). Only the capture path stops calling it.
- `GitRevisionResolver::read_pinned_path` loses the `repository_state_store.rs:1619` caller but stays (`commands/archive.rs:99`).
- Dependency check: **66986f88** (artifact-dir documents, REQ-04 "linked once the branch merges") and **3d9a0ce3 REQ-03** derive by location after merge, from the local tree; neither reads capture-time fallback evidence from HEAD/main worktree. No dependency.

#### 4. Architecture fit
Pure removal across domain (`repository_state/image.rs`) and storage. `capture_capability_image` drops its `worktree_paths` param.

#### 5. Invariant check
Removes a git spawn from the capture path (D-21 surface shrinks). No purity issue.

#### 6. Footprint
`image.rs` ≈−170, `repository_state_store.rs` ≈−130, `gate_check.rs` ≈−15, `overlay.rs` −2, `mod.rs` −2, 21 `close(` sites −1 each (mostly `BTreeMap::new()` args), tests ≈−60, dev doc 1 line. Total ≈−400. One worker; mechanical but wide (touches files REQ-24 also edits: `repository_state/mod.rs`).

#### 7. Overlap
No in-flight worktree touches these files. Same files as REQ-24 (`repository_state/mod.rs`), REQ-18 (git reader consumer). Sequence REQ-02 before or with REQ-24 in one worker to avoid `mod.rs` conflicts.

---

### REQ-05 — one agent configuration type; tests exercise the production resolver

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `config::AgentConfig` and `AgentIdentity` (`config.rs:1437`) used only by tests | open | defs `config.rs:1434-1481`; uses only `config.rs:3637-3726` tests (6 tests). |
| tests exercise a `cfg(test)` copy of `resolve_agent_id` (`agent_config.rs:122`) | open | `#[cfg(test)] fn resolve_agent_id_with_env` `agent_config.rs:121-155`, body duplicates `resolve_agent_id` `:82-112`. Tests `:288-353`. |

#### 2. Consumer sweep
- Production type: `agent_config::AgentConfig`/`AgentSection` (`agent_config.rs:23-41`, `load()` `:43-74`). `resolve_agent_id` callers: `commands/claim.rs:105,241,300,435,483`, `commands/template.rs:301`, `commands/mod.rs:728,753,3269`.
- `AgentSection.default_ttl_secs` (`agent_config.rs:39-40`): never read in production; `docs/reference/configuration.md:621-623` documents it as "parsed metadata only".
- `config::AgentIdentity::default_ttl_secs()` (`config.rs:1458`): test-only.
- Docs: `docs/reference/configuration.md:26,592-623`, `docs/how-to/multi-agent-coordination.md:262`, `docs/how-to/troubleshooting.md:330,465,485`, `docs/tutorials/parallel-work-worktrees.md:80`, `docs/reference/claim.md:149`. None name the Rust types.

#### 3. Primitive verification
- The two types disagree: `agent_config::AgentSection` requires `created_at`, `description` (`String`); `config::AgentIdentity` makes them `Option`. Production behaviour = required.
- The `cfg(test)` twin calls the real `AgentConfig::load()` (`agent_config.rs:140`), which reads `dirs::config_dir()/jit/agent.toml`. `test_resolve_agent_id_no_config_error` (`:342`) passes only on hosts without that file (this host has none) — an `@/inv/orthogonal-tests` process-state violation.

#### 4. Architecture fit
Recommended shape: one private non-`cfg` function `resolve_agent_id_from(cli, env: impl Fn(&str)->Result<String,VarError>, load: impl FnOnce()->Result<Option<AgentConfig>>)`; `resolve_agent_id(cli)` passes `env::var` and `AgentConfig::load`; tests pass stubs for both. Delete `config::AgentConfig`, `AgentIdentity` and their tests.

#### 5. Invariant check
`@/inv/semantic-types`: agent id is a `String` validated by `validate_agent_id` via `Assignee` parse; converting to a typed identity is outside REQ-05.

#### 6. Footprint
`config.rs` ≈−50 src, −95 tests; `agent_config.rs` −35 twin, ~+10 seam, tests edited. ≈−170. Small.

#### 7. Overlap
`config.rs` is a hotspot: REQ-03/REQ-04 (other slices), **d92f54aa** (InProgress, no worktree found — cannot diff; file-level overlap `config.rs`), **bedf0962** (Ready; `config.rs`/`config_manager.rs`), **6ff14f21** (uncommitted `config.rs` edits). REQ-04's `deny_unknown_fields` on agent.toml applies to the surviving type — do REQ-05 first or in the same task as REQ-04's agent part.

---

### REQ-07 — read-lock sidecar sweep and `read_issues`

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `list_issues` sweeps once per process (`json.rs:1317`) | open | `json.rs:1304-1324`, guard `sidecars_swept` (`:395-397,442`), helper `storage/lock.rs:440-492`. |
| the sweep misses short-id sidecars | open | helper requires `Uuid::parse_str(stem)` (`lock.rs:480-484`). |
| 19 `.lock` files remain | open | 19 files, all short-id/prefix names (`003f.lock`, `003f9f83.lock`, …), mtimes 2025-12-22…2026-02-01. Untracked (`git ls-files .jit/issues \| grep lock` → 0), ignored by `.gitignore:5` `.jit/**/*.lock`. |
| `read_issues` exists only to keep the sweep out of one caller | open | trait default `storage/mod.rs:193-224`; override `json.rs:1326-1333`; sole production caller `commands/document.rs:215` (doc conformance). |

#### 2. Consumer sweep
- Src: `json.rs:395-397,442,1304-1333`; `lock.rs:440-492`; `storage/mod.rs:193-224` (trait doc on `list_issues` also references `read_issues`); `commands/document.rs:215`.
- Tests: `json.rs:2305` (`secondary_storage.read_issues()` → `list_issues`), `json.rs:2411-2437`, `2439-2470+`; `lock.rs:1015-1080` (3 tests); `tests/cli_repo_workflow/artifact_conformance_cli_tests.rs:415-427,464-467` (plants a sidecar to prove conformance leaves it — meaningless after removal).
- Docs: `docs/reference/storage-format.md:77-82`. Prose: `dev/studies/perf/session-cost-27ffbd2d.md` (study record, leave).
- No `InMemoryStorage` override of `read_issues`.

#### 3. Primitive verification
- "Listing issues writes nothing": after removal `list_issues` still calls `locker.lock_shared(".index.lock")`, which opens create-if-absent (`lock.rs:255`, `open_or_create`). See Open decisions.
- Removing the 19 files is a primary-checkout working-tree action (untracked, ignored); no commit carries it, and a worker in a linked worktree cannot do it.

#### 4. Architecture fit
Delete trait method, override, sweep, `AtomicBool`, helper; `document.rs:215` → `list_issues()`. Test owning "list writes nothing": convert `json.rs:2380-2409` (lock count independent of N) or add one asserting the tree is byte-identical after `list_issues` (except the retained `.index.lock`).

#### 5. Invariant check
None.

#### 6. Footprint
`json.rs` ≈−75, `lock.rs` ≈−95, `storage/mod.rs` −25, `document.rs` 1, `artifact_conformance_cli_tests.rs` −15, `storage-format.md` −6. ≈−215. Plus lead-run `rm .jit/issues/*.lock` in the main checkout.

#### 7. Overlap
`json.rs` also edited by REQ-24 (`with_mutation_interruptions`) and by **16d63038** (`resolve_issue_id` in the same `IssueStore` impl block). `storage/mod.rs` trait shared with 16d63038.

---

### REQ-08 — lifecycle-timestamp backfill

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| ships `migrate` command, `commands/migrate.rs`, `Event::LifecycleTimestampsBackfilled`, `MutationIntent::RepairIssueLifecycle`, `derive_lifecycle_timestamps` | open | `cli.rs:435-440,2310-2330,3265,3325,3765-3769,3870`; `main.rs:23,7873-7895`; `commands/migrate.rs` (148); `commands/mod.rs:48,87`; `domain/types.rs:1553-1568,1642,1674,1943-1949,2010`; `domain/event_catalog.rs:118-119,151,178,206,257-259,416,478,790`; `repository_state/mutation.rs:372-380,941-953,2400-2430`; `domain/queries.rs:10-100` (+ tests `514-572`). |
| cost O(issues × events) | open | `derive_lifecycle_timestamps` folds the whole event slice per issue (`migrate.rs:98`). |

#### 2. Consumer sweep
- `LifecycleTimestamps` struct (`queries.rs:18`) — only used by `derive_lifecycle_timestamps`; delete with it.
- Tests: `tests/fast_issue/lifecycle_timestamps_tests.rs:220-479` (11 backfill tests incl. index-rejection ones that exercise migrate's private loader only; delete). `:1-69` write-point tests stay; module doc `:1-7` names backfill.
- MCP: `mcp-server/curated-tools.json:73` (`jit_migrate_lifecycle-timestamps` exclude entry). `mcp-server/test-unit.js:506-508` fails on a stale curated name, so this line must go in the same change.
- Generated docs: `docs/reference/events.md:20,47` (generated from `event_catalog`, `REFERENCE_PATH` `event_catalog.rs:38`) — regenerate.
- Hand docs: `docs/reference/cli-commands.md:3904-~3925`; `docs/reference/cli-command-grammar.md:27` (noun list names `migrate`); `docs/reference/storage-format.md:233-236,254-256,427`; `docs/concepts/guarantees.md:199-200`.
- `CHANGELOG.md:944-945` — historical release note.
- No `web/` or `crates/server` consumer.

#### 3. Primitive verification — event-log parsing of unknown types
- `domain::parse_known_events` (`domain/event_log.rs:42-76`) deserializes a record only when its `type` is in `EventTag::ALL` (`:66`); any other well-formed tagged record is skipped. Doc `:35`: "retaining retired/unknown records".
- Production readers all go through it: `storage/json.rs:160,1352`, `repository_state/mutation.rs:684`, `repository_state/archive.rs:465`. `json.rs:1381` (`read_artifact_archive_events`) filters by tag string before typed parse.
- Existing owning test: `storage/json.rs:1831` `test_read_events_skips_structurally_valid_retired_tags_but_keeps_strict_records`.
- Exception: `InMemoryStorage::load_events` (`storage/memory.rs:479-492`) uses strict `serde_json::from_str::<Event>` per line and errors on unknown tags. Test-only backend, but diverges from the JSON backend (`@/inv/shared-test-contracts`).
- This repository: `.jit/events.jsonl` holds **1** backfill record (line 5417, `issues_updated: 435`) of 16 530 lines.

**Recommended design:** rely on the existing generic tolerance. Delete the variant and its tag; the record at line 5417 becomes an unknown tag that `parse_known_events` skips. No new test is needed: `json.rs:1831` owns the property. Verify by running `jit events query --json` and `jit validate` on this repository after the change. Rejected options: rewriting this repo's log (breaks the append-only log, `@/inv/event-log`/`@/inv/atomic-writes`, and erases audit history); a parse-only variant (`@/inv/canonical-cutover`).

#### 4. Architecture fit
Deletion only. `parse_repository_index` keeps other callers.

#### 5. Invariant check
Removing a repository-scoped event tag changes the `jit --schema` `events` array and the generated `events.md` — both derived, regenerate.

#### 6. Footprint
Src ≈−330 (`migrate.rs` −148, `queries.rs` −120, `types.rs` −25, `event_catalog.rs` −15, `mutation.rs` −55, `cli.rs` −30, `main.rs` −25, `mod.rs` −2); tests −260; docs −30; MCP −1. ≈−650. Natural split: none needed; one worker.

#### 7. Overlap
`cli.rs`/`main.rs` are **df129d9e**'s main targets (and **2d10f1dd** uncommitted `main.rs`, `commands/mod.rs` edits). `curated-tools.json` is also touched by **0f8efc37**. `memory.rs` strict event parse is in REQ-24's file. Sequence REQ-08 early (small, self-contained hunks in `main.rs`).

---

### REQ-13 — source-scan absence tests

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| absence tests at `repository_state_cutover_tests.rs:1283,1412,1430` | open | `1283` `test_cutover_has_no_known_predecessor_or_compatibility_module` (paths, aliases, packages, wrapper fns); `1412` `..._no_retired_selector_bypass`; `1430` `..._no_retired_profile_applied_event_contract`. |

#### 2. Consumer sweep (all retired-name assertions found)
- `tests/scratch_build/repository_state_cutover_tests.rs`:
  - test `1283-1352` (whole test). Helper used only there: `contains_cargo_manifest` `562-569`.
  - test `1412-1427`, test `1430-1438`.
  - helpers used only by those: `assert_retired_lifecycle_entry_point_absent` `169-185`, `assert_retired_profile_applied_contract_absent` `186-224`, `generated_event_tags` `245-251`.
  - self-test blocks in `test_profile_lifecycle_structural_guards_reject_their_forbidden_shapes`: `1555-1590` (retired bypass + retired event contract cases).
  - Keep: `production_sources` (also used `1459`), `definition_owners` (`1369,1794`), and the authority/ownership/capability tests (`1084`, `1122`, `1355`, `1382`, `1446`, `1468`, `1759`, `1792`, `1819`, `1861`) — they assert current ownership or forbidden capabilities, not retired names.
- Elsewhere: `tests/cli_issue/command_alias_tests.rs:99` `test_retired_document_archive_has_no_alias_or_stub` asserts a retired CLI verb is absent and that no migration hint names it. Not a file-path/function-name/event-tag scan, but the same pattern (see Open decisions).
- No other `include_str!`/`read_to_string(src)` scan asserts a retired name (`output.rs:3715`, `failure_lever_registry.rs:178`, `failure_probe_fixture.rs:25` check current structure).

#### 3–5. Verification / fit / invariants
Deletion-only; `@/inv/orthogonal-tests` favours removal. No new absence tests may be added by REQ-01/02/07/08 deletions.

#### 6. Footprint
≈−150 lines in one file (+ optionally −20 in `command_alias_tests.rs`).

#### 7. Overlap
Profile tests touch **3d9a0ce3**'s (profile ownership) and REQ-06's (other slice) subject matter; `1382` checks `profile/wire.rs` decoder authority, which REQ-06 edits — keep and coordinate.

---

### REQ-18 — git revision test-only queries; `Clone` error

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `changed_paths_between`, `changed_worktree_paths`, `inspect_pinned_target` called only by tests | open | defs `git_revision.rs:525,537,214`; callers only `tests/fast_docs_templates/git_revision_tests.rs:128-137,256,426,448,451`. No internal call (`inspect_pinned_targets` has its own batched path). |
| 50-line `clone_git_revision_error` because the error is not `Clone` | open | `git_revision.rs:837-888` (52) + `clone_io_error` `890-895`; used `:340,363,592`. |

#### 2. Consumer sweep
- Deleted with the queries: `parse_changed_paths` (`:897-911`), `GitRevisionError::ChangedPathsFailed` (`:45-47`), `parse_pinned_tree_entry` (`:732-751`, only `inspect_pinned_target` uses it).
- Tests: `git_revision_tests.rs:118-140` (single-target classification), `:240-280` (batch parity vs single), `:405-431` (git-unavailable parity), `:434-457` (changed paths, delete). 457-line file.
- `GitRevisionError` external matchers: only the test file (`:96,100,158,191-200,429`); `storage/json.rs:500` (type in signature).

#### 3. Primitive verification — what blocks `Clone`
- `GitUnavailable { source: std::io::Error }` (`:27-31`) — `io::Error` is not `Clone`.
- `InvalidPath(#[from] PathReadError)` (`:51`) — `PathReadError::Other(anyhow::Error)` (`path_errors.rs:31`) is not `Clone`.
- The only producer of `InvalidPath` is `validate_repo_relative_path` (`path_errors.rs:57-74`), which yields only `PathReadError::InvalidPath(String)`.

**Simplest `Clone`:** `GitUnavailable { program, source: Arc<io::Error> }` (`Arc<io::Error>` is `Error + Clone`, works with `#[source]`) and `InvalidPath(String)` with `map_err` at the two remaining call sites (`:178`, `:304`). Then `#[derive(Clone)]` and delete both clone helpers. Making `PathReadError` `Clone` instead would ripple into `crates/server/src/routes.rs:351-354`, `main.rs:373-375` and every `PathReadError::Other(...)` constructor (≈12 sites).

#### 4. Architecture fit
Tests that used `inspect_pinned_target` as an oracle must assert classification through `inspect_pinned_targets` directly (expected kinds per request); git-unavailable parity reduces to "every request reports `GitUnavailable`".

#### 5. Invariant check
No new git surface; shrinks the module f4d9502a builds on.

#### 6. Footprint
`git_revision.rs` ≈−180 (queries −110, clones −60, enum/variant −5, +5); `git_revision_tests.rs` ≈−60 net. ≈−240.

#### 7. Overlap
**f4d9502a** Background lists exactly these items (`git_revision.rs:525-560`, `214`, `837`) as its own; the 842b0ac6 container notes say REQ-18 owns deletions + `Clone`. Do REQ-18 first; f4d9502a should drop those bullets. REQ-02 removes the `read_pinned_path` caller at `repository_state_store.rs:1619`; no conflict (different file).

---

### REQ-19 — claim commands' unused type parameter; uncalled lease check

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `check_issue_lease` (`claim.rs:559`) no caller | open | def `claim.rs:542-605` (with doc); zero references in tree outside the def (archive docs mention similar names only). |
| three claim commands declare unused `<S: IssueStore>` | open | `execute_claim_renew<S>` `:421`, `execute_claim_status<S>` `:469`, `execute_claim_force_evict<S>` `:618`; none takes an `S` argument. (`execute_claim_release_by_issue<S>` `:334` uses `storage: &S` — keep.) |
| turbofish at `main.rs:8278,8353,8519` | open | exactly those three; no other caller (tests use private `_test` helpers). |

#### 2. Consumer sweep
Only `main.rs:8275-8278`, `8350-8353`, `8516-8519`. No `crates/server`, tests, docs, skills consumer.

#### 3. Primitive verification
The unit tests `claim.rs:1067-1090` (`execute_claim_status_test`), `1244-1260` (`execute_claim_renew_test`), `1630-1645` (`execute_claim_force_evict_test`) do not call the production functions; they rebuild `ClaimCoordinator` calls themselves. Cross-slice (c780514e, test re-proofs).

#### 6. Footprint
`claim.rs` −65, `main.rs` 3 lines. Trivial; bundle with REQ-20 or REQ-23.

#### 7. Overlap
`main.rs` (df129d9e, 2d10f1dd uncommitted). **d7d0d0db** (InProgress) edits `storage/claim_coordinator.rs`, not `claim.rs`.

---

### REQ-20 — one typed not-required error; preset rule constructor; test-only checker entry; passthrough

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `preview_coverage_rule` (`gate_presets/planning.rs`) reached only by tests | open | def `planning.rs:81`; callers its own tests `:159-249` and doctest `:44-57`; re-export `gate_presets.rs:15,19`. The whole 252-line module is that fn + tests. `sdd_bracket_tests.rs:109`, `research_bracket_tests.rs:117` only share the name (they check example rules). |
| `gate_execution::execute_gate_checker` reached only by tests | open | def `gate_execution.rs:26-39`; 10 test call sites in `gate_execution.rs` (`683,711,738,775,1166,1191,1219,1247,1357` …). Production uses `_with_context` (`gate_check.rs:1342`). |
| `reusable_verdict` one-line passthrough | open | `gate_check.rs:1043-1049` → `self.storage.find_reusable_gate_run`; one caller `:928`. |
| not-required reported three ways (`gate.rs:822,1039`, `gate_check.rs:206`) | open | typed `GateNotRequiredError` (`gate.rs:37-43`) raised at `gate.rs:823` (`pass_gate`) and `commands/mod.rs:1613` (manual evaluate); `anyhow!` at `gate.rs:1039-1043` (`fail_gate`); `bail!` at `gate_check.rs:204-209` (`required_automated_gate`, callers `:776,896,959`). |

#### 2. Consumer sweep — who matches
- Typed matching: `main.rs:79` (`error_to_error_code` → `InvalidArgument`), `main.rs:719-730` (JSON envelope details `issue_id`, `key`), `main.rs:9063` (classifier table test).
- Message-text matching: none. Tests use exit codes (`tests/cli_gate/gate_evaluate_exit_code_test.rs:205,237`). No server, MCP or web consumer.
- `required_automated_gate` reachability: `check_gate_with` is reached after the typed check in `pass_gate` (`gate.rs:868`) and in the all-gates loop (`gate_check.rs:1599`) over `gates_required`, so its `bail!` fires only for a concurrent change between captures (`:959`). `fail_gate` (`jit gate fail`, public, `main.rs:5312`) is the one user-visible change: its not-required failure becomes exit 2 / `InvalidArgument` instead of the `GateError` fallback.

#### 3. Primitive verification
`GateNotRequiredError` survives. Its message "Gate '{k}' is not required for issue {id}" replaces "…for this issue" in `fail_gate`.

#### 4. Architecture fit
Commands layer only; classification already lives in `main.rs`.

#### 6. Footprint
`gate_presets/planning.rs` −252, `gate_presets.rs` −2, `gate_execution.rs` −14 + 10 test edits, `gate_check.rs` −10, `gate.rs` ~±6. Add one owning test that `gate fail` on a non-required gate exits 2 (or extend `gate_evaluate_exit_code_test.rs`). ≈−280.

#### 7. Overlap
`main.rs` classifier is **df129d9e** REQ-11/12 territory; REQ-20 needs no `main.rs` edit. **6ff14f21** edits a doc comment in `gate.rs:1276` (no conflict).

---

### REQ-23 — FileLocker poll loops; non-blocking variants and lock-path accessor test-only

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| one poll loop repeated three times (`lock.rs:213-346`) | open | `lock_exclusive` `213-241`, `lock_shared` `254-281`, `lock_exclusive_with_metadata` `347-395` (third loop extends past 346). |
| test-only `try_*` under `allow(dead_code)` | open | `try_lock_exclusive` `292-302`, `try_lock_shared` `313-325`. |

#### 2. Consumer sweep
- `FileLocker::try_lock_exclusive`: `lock.rs` tests (`532-749`), `storage/claim_coordinator.rs:1589,2698,2700` (all `#[cfg(test)]`). `try_lock_shared`: `lock.rs:734,749`. No integration test (`crates/jit/tests`) or server use; integration tests use `lock_exclusive` only (`lock_contention_tests.rs:93,125`, `command_exit_code_projection_tests.rs:262`, `failure_probe_fixture.rs:329`). `lock_cleanup.rs:98` calls fs4's `File::try_lock_exclusive`, not `FileLocker`.
- Lock-path accessor: `LockGuard::path()` `lock.rs:145-148` (`allow(dead_code)`) and field `path` `:113` (`allow(dead_code)`); readers only `lock.rs:512,807,827` tests. `Drop` does not read `path`.

#### 3. Primitive verification
`#[cfg(test)]` suffices for `try_*` (all callers are in-crate unit tests). The `path` field exists only for the accessor.

#### 4. Architecture fit
One private `poll(&self, file, path, mode, try_fn) -> Result<File>` loop; `lock_exclusive_with_metadata` writes metadata after it returns.

#### 6. Footprint
`lock.rs` ≈−45 net. Small; bundle with REQ-19/REQ-24.

#### 7. Overlap
REQ-07 deletes `remove_orphaned_issue_read_sidecars` from the same file. **d7d0d0db** edits `claim_coordinator.rs` (contains test callers `1589,2698`).

---

### REQ-24 — storage / repository_state cfg twins and dead_code

#### 1. Claim classification
| Claim | Class | Evidence |
|---|---|---|
| `with_mutation_interruptions` cfg twin (`json.rs:455,475`) | open | `#[cfg(feature = "test-support")] pub fn` `453-462`; `#[cfg(not(feature))] #[allow(dead_code)] pub(crate) fn` `473-482`. `crates/jit/Cargo.toml:103` dev-dep enables `test-support` for every `cfg(test)` build, so the twin is never needed; all src callers are `cfg(test)`, one integration caller (`tests/cli_repo_workflow/profile_publication_tests.rs:113`). |
| `InMemoryStorage` stale `allow(dead_code)` (`memory.rs:34,68`) | open | on `pub struct` and `pub fn new`; both are `pub` from a `pub mod` re-exported (`storage/mod.rs:92`), so no lint fires without them. |
| `neutral_files` no caller | open | `repository_state/initialize.rs:325-344`; zero references. |
| `default_rule_membership_diff` is `allow(dead_code)` | open | `default_rules.rs:404-421`; also `DefaultRuleMembershipDiff::is_empty` `:336-344`; only own tests + doctest. Production uses `default_rule_membership_diff_from_identities` (`:434`). |
| `mod.rs:41-99` holds cfg re-export twins | open, **understated** | twins at `mod.rs:34-51` (membership diff), `80-99` (capture closure) **and** `214-230` (`serialize_ruleset`/`SchemaFile`/`SerializedRuleSet`). |

#### 2. Consumer sweep — remaining `allow(dead_code)` in `storage/` and `repository_state/`
| Site | Item visibility | Note |
|---|---|---|
| `storage/json.rs:474` | `pub(crate)` twin | delete with twin |
| `storage/memory.rs:34,68` | `pub` | remove attrs |
| `storage/memory.rs:121` (`with_mutation_interruptions`), `155` (`with_mutation_interruption_view`), `171` (`inject_repository_state_apply_conflicts`) | `pub(crate)`, `cfg(any(test, feature))`/`cfg(feature)` | dead only in feature-on, non-test builds (`crates/server` test build) |
| `storage/lock.rs:113,145,292,313` | field / `pub` | REQ-23 |
| `repository_state/index.rs:100` (`upsert_active`), `118` (`mark_deleted`) | `pub(crate)` | callers `storage/test_support.rs:193`, `commands/test_helpers.rs:121`, `commands/dependency.rs:1243` |
| `repository_state/default_rules.rs:341,410` | `pub` | delete fns |
- `cfg` re-export consumers: `validate_capture_closure` — `commands/validate.rs` (internal) and `tests/fast_rules/derived_state_repair_tests.rs:3,147`; `serialize_ruleset`/`SchemaFile`/`SerializedRuleSet` — internal + integration tests (not enumerated individually; making them unconditionally `pub` satisfies both).

#### 3. Primitive verification
Unconditional `pub use` removes every twin without changing resolution: `pub` items in a `pub mod` raise no unused lint. Deleting `default_rule_membership_diff` removes the only reason for its seam; `DefaultRuleMembershipDiff` stays `pub(crate)`-reachable through `_from_identities`.

#### 4. Architecture fit
No layer change. `memory.rs:479-492` strict event parsing (see REQ-08) is in this file; converging it on `parse_known_events` is a `@/inv/convention-convergence` fix, though outside REQ-24's literal text.

#### 6. Footprint
`json.rs` −13, `memory.rs` −2 attrs (more if `pub(crate)` attrs are addressed), `initialize.rs` −20, `default_rules.rs` −45 incl. tests/doctest, `repository_state/mod.rs` −40. ≈−120.

#### 7. Overlap
`repository_state/mod.rs:60-61` also edited by REQ-02; `json.rs` by REQ-07; `memory.rs` by 16d63038 (`resolve_issue_id`). Group REQ-02+REQ-24, or sequence REQ-02 → REQ-24.

---

### Cross-slice couplings
- **REQ-01 ↔ f4d9502a**: REQ-01 deletes the libgit2 document-reference check (`validate.rs:2466-2561`) that f4d9502a's Background lists. **REQ-18 ↔ f4d9502a**: identical items. f4d9502a should follow 842b0ac6 and drop those bullets.
- **REQ-05 ↔ REQ-04** (other slice): REQ-04's `deny_unknown_fields`/no `[behavior]` tolerance applies to the agent type REQ-05 keeps. Shared file `agent_config.rs`; `config.rs` is shared with REQ-03/REQ-04, d92f54aa, bedf0962, 6ff14f21.
- **REQ-08 ↔ df129d9e / 0f8efc37**: `cli.rs`/`main.rs` arms and `mcp-server/curated-tools.json:73` (the MCP unit test fails on a stale curated name, `test-unit.js:508`).
- **REQ-08 ↔ REQ-24**: `InMemoryStorage::load_events` rejects unknown tags (`memory.rs:488`), unlike the JSON backend.
- **REQ-13 ↔ REQ-06 / 3d9a0ce3**: keep authority test `cutover_tests.rs:1382` (checks `profile/wire.rs` decoder ownership, which REQ-06 rewrites).
- **REQ-13 ↔ df129d9e REQ-20**: `command_alias_tests.rs:99` (retired `doc archive` verb absent) sits next to the hidden-stub work.
- **REQ-19 ↔ c780514e**: claim unit tests use private `_test` re-implementations instead of the production commands (`claim.rs:1067,1244,1630`).
- **REQ-20 ↔ df129d9e**: `GateNotRequiredError` classification lives in `main.rs:79,719,9063`; unchanged by REQ-20.
- **REQ-07 ↔ 16d63038**: both edit the `IssueStore` trait (`storage/mod.rs`) and its JSON/memory impls.
- **6ff14f21** (InProgress, not on the dispatcher's list): uncommitted edits inside `validate_integrity_silent` and in the breakdown/template test files REQ-01 deletes.

### Open decisions
1. **REQ-07 "Listing issues writes nothing" vs `.index.lock`.** `list_issues` opens `.index.lock` create-if-absent (`lock.rs:255`). Options: (a) read it as "no repository content" (lock files are gitignored runtime state, `storage-format.md:70-75`); (b) make shared reads open existing lock files only. Recommend (a), and state it in the owning test (the tree is unchanged apart from the retained lock set). Confirm with the reviewer, since criteria are read literally.
2. **REQ-07 removing this repository's 19 `.lock` files.** They are untracked and ignored, so no commit removes them. Recommend the lead runs `rm .jit/issues/*.lock` in the primary checkout at integration and records it as evidence.
3. **REQ-23 lock-path accessor.** Options: `#[cfg(test)]` on `LockGuard::path()` and the field (the field then needs a cfg-gated initializer in three constructors); or delete accessor and field and drop the three `guard.path()` assertions. The literal criterion says the accessor "exist[s] only in test builds". Recommend `cfg(test)` to match the text.
4. **REQ-01 rename `enforcement_drift_tolerant` → `enforcement_drift`** once the twin is gone (4 call sites + doctest). Recommend rename; the suffix only described the difference from the deleted function.
5. **REQ-05 surviving agent-config shape.** Keep `agent_config::AgentConfig` (production). Whether to also drop the inert `default_ttl_secs` field (docs call it "parsed metadata only") is a D-18 choice that REQ-04's `deny_unknown_fields` makes user-visible. Recommend dropping it in the REQ-04 task, not REQ-05.
6. **REQ-24 scope for `pub(crate)` `allow(dead_code)`** (`memory.rs:121,155,171`, `index.rs:100,118`). The criterion names public items only. Recommend leaving them (they are configuration-dependent, not dead) unless the reviewer reads "storage contains no allow(dead_code)" broadly.
7. **REQ-13 `command_alias_tests.rs:99`.** It is outside the literal categories (file paths, function names, event tags). Recommend deleting it under REQ-13 for consistency, or leaving it to df129d9e REQ-20.

## 842b0ac6 investigation, group B (REQ-03, 04, 06, 09, 10, 11, 12)

Verified at HEAD `a41eaacf3` (main). cd2ed136 is already merged (`f641f2f29`), so `visualization.rs` line numbers below are post-merge.

### REQ-03: inert configuration keys; each default stated once

#### 1. Claim classification
- "`worktree.mode`/`JIT_WORKTREE_MODE`, `[global_operations]`, `[locks]`, `[events]` are parsed and merged, but only the `config show` printer reads them": **valid-and-open, partly invalid as stated**. Three readers exist, not one:
  - `config show`: `main.rs:6804-6827` (JSON) and `6834-6897` (human).
  - `config get`: `EffectiveConfig::full_snapshot`, `config.rs:1913-2004`, with sections at `1958-2001`.
  - `config validate`: checks `JIT_WORKTREE_MODE` at `main.rs:7006-7010`.
  - No runtime command reads them.
- "Each default is written twice: per-layer `config.rs:1335-1428` vs `Merged*` `2009-2172`": **valid-and-open**. More copies exist (below). The per-layer accessors have no production caller. Their only callers are tests (`config.rs:3508-3587`). Production goes through the `Merged*` accessors (`commands/claim.rs:112,143-144`, main.rs printers, `full_snapshot`).
- "`docs/reference/configuration.md:582` documents them as 'Parsed compatibility fields'": **valid-and-open**. The heading is at :582 and the text at :584-590.

#### 2. Consumer sweep
Declarations and merge (`crates/jit/src/config.rs`):
- `JitConfig` fields: `worktree` `:54`, `global_operations` `:58`, `locks` `:60`, `events` `:62`. The config-absent literal is at `:1510-1513`.
- `WorktreeModeError` `:1188-1196`; `WorktreeConfig.mode` `:1217`; `WorktreeMode` with FromStr/Deserialize `:1226-1262`; `WorktreeConfig::worktree_mode` `:1308-1310`.
- `GlobalOperationsConfig` `:1373-1391`, `LocksConfig` `:1394-1410`, `EventsConfig` `:1413-1429`.
- `EffectiveConfig::worktree_mode` `:1749-1781` (env var + three layers), `global_operations()/locks()/events()` `:1848-1882`.
- `full_snapshot` worktree.mode `:1958-1962` plus sections `:1977-2001`, with rustdoc `:1884-1912`.
- `Merged{GlobalOperations,Locks,Events}Config` `:2087-2172`.

Printers and checkers:
- `main.rs:6804-6827` and `6834-6897` (config show).
- `main.rs:7006-7010` (validate env check).
- Dead code at `main.rs:7016-7018` ("Try to build effective config", `let _ = loader.with_repo_config`).

Other code:
- `commands/config.rs:488` (section list in `test_get_config_covers_whole_surface_on_fresh_repo`).
- `edit_config_document` type heuristics `commands/config.rs:187-200`. The bool branch (`enable_`/`require_`/`auto_`) matches only removed keys. The numeric `_secs`/`_pct` branch matches only keys d92f54aa removes.
- `config_manager.rs:19` (rustdoc names `JIT_WORKTREE_MODE`).

Tests:
- `config.rs` has 19 tests to delete or rewrite. Seven parse or resolve `worktree.mode` as their primary property:
  - `test_worktree_mode_{auto,on,off,invalid,default_to_auto}`
  - `test_worktree_mode_case_sensitivity_divergence_is_gone`
  - `test_effective_config_worktree_mode`

  Seven test removed sections or env overrides:
  - `test_env_override_worktree_mode`, `test_env_overrides_config_file`, `test_env_invalid_value_returns_error`
  - `test_{global_operations,locks,events}_config{,_defaults}`

  Five mix removed keys with surviving ones:
  - `test_worktree_mode_and_enforcement_mode_from_str_case_insensitive`
  - `test_config_loader_defaults_only`
  - `test_full_parallel_work_config`
  - plus two more found by `awk` over `config.rs:2174-`
- `crates/jit/tests/cli_repo_workflow/config_get_tests.rs:372` (`test_get_worktree_mode_still_works_unchanged`).
- `crates/jit/tests/cli_repo_workflow/project_config_tests.rs:144,178,196,215,229`. The `:196` assertion counts 3 findings, including one from JIT_WORKTREE_MODE.
- `crates/jit/tests/cli_issue/failure_lever_registry.toml:129-136`. The `config validate` exit-4 lever is `env:JIT_WORKTREE_MODE=definitely-not-a-mode`.
- `crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs:510-522` (same lever).
- `crates/jit/tests/cli_repo_workflow/steering_scenarios.rs:26,138,146,607`.

Docs and other files:
- `docs/reference/configuration.md:29`, `:582-590`, `:651` ("worktree/coordination/lock/event settings"), `:667-670`.
- `docs/reference/cli-commands.md:3970-3979` and `:4003-4005` (section list in the error example).
- `docs/how-to/multi-agent-coordination.md:257` (`worktree.mode`; the same sentence is d92f54aa's).
- `docs/how-to/troubleshooting.md:424` (stale mention).
- `profiles/jit-default/manifest.toml:440-466` (adopter comment block: `mode`, `[global_operations]`, `[locks]`, `[events]`).
- `dev/eval/steering/README.md:58` and `dev/eval/steering/run_eval.py:69`.
- `CHANGELOG.md:1027` is history; leave it.

No occurrences in web/, mcp-server/, crates/server, `.agents/skills`, `docs/reference/example-config.toml`, or this repo's `.jit/config.toml` (which declares only `[worktree] write_policy`).

#### 3. Primitive verification
After REQ-03 and d92f54aa, the remaining defaults and every place each is stated:
- `enforce_leases`:
  - `Strict` at `config.rs:1317`, `config.rs:1820` and `config.rs:1966`.
  - `main.rs:6807` and `main.rs:6849`.
  - `Off` when there is no section, at `config_manager.rs:111`. This is the divergent value, bedf0962's defect.
  - Docs: `configuration.md:482`.
- `max_indefinite_leases_per_agent = 2`:
  - Code: `config.rs:1364` and `config.rs:2065`.
  - Prose: `configuration.md:560`, `profiles/jit-default/manifest.toml:453`.
- `max_indefinite_leases_per_repo = 10`:
  - Code: `config.rs:1368` and `config.rs:2084`.
  - Prose: `configuration.md:564`, `manifest.toml:454`.
- `write_policy`: already one constant (`LinkedCheckoutWriteStance::DEFAULT`), referenced at `config.rs:1330` and `config_manager.rs:146`. Compliant.

`config show` (main.rs) and `full_snapshot` (config.rs) are two printers of the same merged view. Both restate every default through their own `unwrap_or`.

#### 4. Architecture fit
Proposed single-statement shape:
- Delete the per-layer default accessors (`CoordinationConfig::*()`, `WorktreeConfig::enforcement_mode()`) and keep raw `Option` fields per layer.
- Give `EffectiveConfig` one generic layered lookup (`repo.or(user).or(system)` over a field projection). The `Merged*` structs become thin or disappear.
- State each default once, as a named const beside its key. `runtime_defaults.rs` already projects named defaults into `docs/reference/runtime-defaults.md`. Putting the two lease limits there would also remove the prose copies in `configuration.md` and `manifest.toml`, per `@/inv/single-source-prose`.
- Render `config show` from `full_snapshot`, so the printer stops restating defaults.

Layering is respected: config IO stays in `config.rs`, and main.rs only renders.

#### 5. Invariant check
None. All keys are engine mechanism, not domain vocabulary.

#### 6. Footprint
Natural splits:
- **(a) Delete inert keys.**
  - Files: `config.rs` (about -450 lines including tests), `main.rs` printers and validate, `config_manager.rs:19`.
  - Tests: the 19 `config.rs` tests, `config_get_tests.rs`, `project_config_tests.rs`, `steering_scenarios.rs`, the lever and exit-code test pair (needs a replacement lever; see Open decisions), `commands/config.rs` section list and heuristics.
  - Docs, the manifest comment block, and `dev/eval/steering`.
  - Size: about 800 deleted lines, one worker.
- **(b) Single-statement defaults.**
  - Files: layered lookup in `config.rs`, `config show` rendered from `full_snapshot`, consts moved to `runtime_defaults.rs`, then regenerate `docs/reference/runtime-defaults.md` (`scripts/generate-runtime-defaults-reference.sh`).
  - Size: about 200 lines net removed.
  - Depends on (a), d92f54aa and bedf0962.

#### 7. Overlap and sequencing
- **d92f54aa** (InProgress, no worktree, no commits yet).
  - Its region: `CoordinationConfig` `config.rs:1334-1371`, `MergedCoordinationConfig` `:2009-2085`, `full_snapshot` `:1968-1976`, `main.rs:6810-6814,6855-6865`, `configuration.md:547-571`, `multi-agent-coordination.md:257`, `manifest.toml:444-454`, `commands/config.rs:497-504` (the `coordination.default_ttl_secs` assertion), `cli-commands.md:3972`.
  - It shares every file with (a) and (b). Land it first.
- **bedf0962** (Ready, unassigned).
  - Its region: `config_manager.rs:76-113`, `config.rs:1316-1318,1790-1821`, `main.rs:6807,6849,7011-7015`, `configuration.md:29,482`.
  - It owns `enforce_leases` resolution. REQ-03(b) must run after it, or fold the `enforce_leases` dedupe into it.
  - If bedf0962 deletes `JIT_ENFORCE_LEASES`, `project_config_tests.rs:178-205` loses its second env finding as well.
- **6ff14f21**. The assignment did not name it; it is in flight, with uncommitted edits in `.agents/worktrees/agent-6ff14f21`.
  - It rewrites `config.rs` tests that REQ-03 deletes (`:3186-3215`, `:3859-3911`, using `EnvGuard`), plus `commands/test_helpers.rs` and `test_utils.rs`.
  - Sequence REQ-03 after it to avoid a merge conflict.
- **a122b9b3** (Rejected) left a worktree whose commit `c65b2433b` "reject retired configuration controls" removed 780 lines of `config.rs`. Use it as prior art only. It is stale against main.

### REQ-04: unknown keys fail to load; no serde field alias

#### 1. Claim classification
- "`JitConfig` has no `deny_unknown_fields`; a test asserts 'stale config must still load' (`config.rs:2730`)": **valid-and-open**. The test is `test_load_config_with_removed_keys_still_parses` at `config.rs:2701-2742`, with the `expect` at `:2730`.
- "`agent.toml` tolerates a legacy `[behavior]` block (`agent_config.rs:21`)": **valid-and-open**. Its test is at `agent_config.rs:271-290`. A twin test for the dead `config::AgentConfig` is at `config.rs:3659-3676`.
- "Rule selectors accept `has-doc-type` and `doc_type` (`declarations/rules.rs:237,1110`)": **valid-and-open**. These are the only two `serde(alias)` in config, agent and rule types.

#### 2. Consumer sweep
`config.toml` types: none of the following carries `deny_unknown_fields` except `IconConfigToml` (`config.rs:219`):
- `JitConfig` `config.rs:12`, `VersionConfig` `:84`, `ProjectConfig` `:97`, `HierarchyConfigToml` `:205`.
- `ValidationConfig` `:253`, `DocumentationConfig` `:315`, `NamespaceConfig` `:420`, `ItemKindConfig` `:516`.
- `TomlSourceDescriptor` `:760`, reached through the hand Visitor at `:690-726`.
- `ProjectionConfig` `:1034`, `WorktreeConfig` `:1214`, `CoordinationConfig` `:1334`.

No `#[serde(flatten)]` or tagged enum appears in any config type. The `flatten` uses are all in `visualization.rs`, `output.rs` and `claim_coordinator.rs`.

All three configuration layers share `JitConfig` (`config.rs:1677-1692`), so `deny_unknown_fields` also applies to `~/.config/jit/config.toml` and `/etc/jit/config.toml`.

There are two parse paths:
- `JitConfig::load` (`config.rs:1531`).
- `declarations::parse_configuration` (`declarations/mod.rs:65-69`), the repository-image path, plus `storage/ruleset_store.rs:146`.

Both use the same derive, so one attribute set covers both.

`agent.toml` has two types; REQ-05 is the other slice:
- `agent_config::AgentConfig`/`AgentSection` (`agent_config.rs:23-41`), used in production, with `created_at` and `description` required.
- `config::AgentConfig`/`AgentIdentity` (`config.rs:1436-1482`), used only by tests, with every field optional.

Rule declaration types in `declarations/rules.rs`:
- `RawRulesFile` `:879`, `RawRule` `:886` and `Selector` `:223` lack `deny_unknown_fields`.
- The `RawAssert` family `:908-1116` already has it.
- **`label-coverage`, `label-reference` and `dependency-shape` are untyped `toml::value::Table`s** (`:923-927`). They are interpreted late in `validation/graph.rs`:
  - label coverage keys at `:860-935`.
  - dependency-shape `target`/`mode`/`transitive` at `:1380-1403`. `target` is deserialized to `Selector` at evaluation time, so a bad key becomes a `config-error` finding, not a load failure.
  - They are serialized back verbatim at `repository_state/rule_serialize.rs:267-275`.
- Two tests assert tolerance of unknown keys in these tables:
  - `test_label_coverage_inline_rule_unchanged_with_extra_unknown_key` `rules.rs:2869`.
  - `test_label_coverage_inline_rule_with_stray_kind_key_is_inert` `rules.rs:2894`.

Stale prose asserting tolerance:
- `config.rs:248-252` (ValidationConfig doc), `config.rs:415-419` (NamespaceConfig doc), `config.rs:1525-1529` (load comment).
- `agent_config.rs:20-22`, and section headers at `config.rs:2694-2698`.

Docs:
- `docs/how-to/validation-rules.md:87` documents the **alias** `has-doc-type`.
- `docs/reference/configuration.md:411` documents the canonical `has_doc_type`.
- `validation/graph.rs:1363` rustdoc says `has-doc-type`.

Writers emit the canonical spellings: `rule_serialize.rs:210` and `commands/validate.rs:2953` (`has_doc_type`).

Existing content against the new strictness:
- This repo's `.jit/config.toml` keys all map to declared fields. Checked: version, documentation, type_hierarchy, validation, namespaces, item_kinds (including the `source` descriptor), projection, and `worktree.write_policy`.
- `.jit/rules.toml` uses only declared keys. Top level: name, origin, description, when{type, state}, severity, enforce, assert. Label-coverage keys: child-link, child-type-exclude, container-from-label, criteria-section, id-pattern, marker, satisfies-namespace. All of them are read by `graph.rs`.
- Profile contribution values use current field names.
- No test, doc or fixture uses either rule alias.
- Inline test TOML strings were not exhaustively checked. The worker must run the full suite after adding the attributes. `test_utils.rs`, `test_taxonomy.rs` and `materialize.rs` tests compose configs from strings.

#### 3. Primitive verification
- **Contradicted: "fails with an error that names it".** `config show` and `config validate` print only the anyhow context `Failed to parse config.toml`; the toml cause is dropped. Reproduced: `config validate` prints `repo config: Failed to parse config.toml`, and `config show` prints `Error: Failed to parse config.toml`. The causes:
  - `config.rs:1531` wraps the error with `.context(...)`.
  - `main.rs:6989,6999` format the error with `{}`.
  - The declarations path (`issue create`, `validate`) does print the cause (`config.toml is invalid: TOML parse error ...`).
  
  REQ-04 therefore also needs `JitConfig::load` errors to carry the cause: `{:#}` or no opaque context.
- toml 0.8 with `deny_unknown_fields` reports ``unknown field `x`, expected one of ...`` with a span. That satisfies "names it" once the cause reaches the output.
- `#[serde(skip)]` fields (`templates`, `invariants`) become unknown keys in `config.toml`. That is the desired outcome.
- `config set --global` writes the user config with no post-edit parse (`commands/config.rs:243-251`). With strict loading, `jit config set --global bogus.key x` would write a file that every layered load then rejects. The repo path re-parses through `declarations_from_image` (`commands/config.rs:327`). Recommendation: `edit_config_document` parses the edited document as `JitConfig` for both scopes.

#### 4. Architecture fit
- Attribute placement: every config struct listed above, plus `RawRulesFile`, `RawRule`, `Selector`, `TomlSourceDescriptor`, and the surviving agent type.
- The three free-form assertion tables need typed `Raw*` structs carrying `deny_unknown_fields`, following the existing `RawGateRecency` pattern. The keys are those `graph.rs` reads.
  - The `kind` sugar (`expand_kind_sugar`, `rules.rs:1403`) must become a typed optional field.
  - dependency-shape `target` must parse as `Selector` at load.
  - `Assertion::{LabelCoverage, LabelReference, DependencyShape}` can keep their table payload if needed (validated at load, stored as now) or become typed. Typed is the cleaner `@/inv/semantic-types` fit but grows the change: `graph.rs`, `desugar.rs:60` and `rule_serialize.rs:267`.
- Domain purity is unaffected; all of this is declaration parsing.

#### 5. Invariant check
None.

#### 6. Footprint
- **(a) `config.toml` strictness.**
  - Change: about 15 attributes, error-cause surfacing in `JitConfig::load` and `main.rs:6989/6999`, and the `config set` post-edit parse.
  - Tests: delete the stale-config test; add one owning test per file kind ("unknown key rejected and named").
  - Size: about 150 lines.
- **(b) Rule strictness and alias removal.**
  - Change: three typed tables, alias removal, rewriting the two tolerance tests, and the docs fix at `validation-rules.md:87` and `graph.rs:1363`.
  - Size: about 300-500 lines.
- **(c) `agent.toml` strictness.** This belongs inside REQ-05's single-type change; a separate task would edit a type REQ-05 deletes.

#### 7. Overlap and sequencing
- d92f54aa REQ-01 requires that a config setting a coordination timing key be "rejected as an unknown key". No code exists yet. The only serde mechanism is `deny_unknown_fields` on `CoordinationConfig` (or `JitConfig`). So d92f54aa pre-empts part of REQ-04(a). Its REQ-04 adds a rejection test that duplicates REQ-04's owning test. REQ-04 should land after it, generalize the attribute, and fold the d92f54aa test into the general one (`@/inv/orthogonal-tests`).
- REQ-03 must precede REQ-04(a). Otherwise every user config holding `[locks]` and the like would break before the fields are deleted.

### REQ-06: one manifest wire version (v2)

#### 1. Claim classification
- "Both shipped profiles declare `manifest-version = 1` (`profiles/jit-default/manifest.toml:10`, `profiles/jit-dogfood/manifest.toml:4`)": **valid-and-open**.
- "`docs/reference/profiles.md:84` documents only version 2": **invalid-as-stated**. `profiles.md:93-99` states that the decoder "continues to accept version-1 manifests". The page documents v2 authoring and v1 acceptance; that sentence must go.
- "Twelve files under `crates/` and `scripts/` still spell version 1": **valid**. Exactly 12 (list below).
- "V1 decoding parses the TOML four times and hashes identity differently (`profile/wire.rs:71-180`)": **valid-and-open**.
  - The four parses: `wire.rs:52` (`decode_manifest`), `:72` (`decode_v1`), and `:96` twice via `v1_declares_template_field`.
  - The two hashing schemes: v1 hashes canonical JSON of the wire (`:79`, via `canonicalize` `:240-253`); v2 hashes the raw source bytes (`:156`).

#### 2. Consumer sweep
v1 manifest text (`manifest-version = 1`, `jit = `, top-level `dependencies = [...]`):
- `profiles/jit-default/manifest.toml:10-13`.
- `profiles/jit-dogfood/manifest.toml:1-7` (`dependencies = ["jit-default"]` must become `[[dependency]]`).
- Fixtures: `crates/jit/tests/fixtures/profile-packages/{synthetic-valid,planner-asset-only,planner-invalid-interpolation}/manifest.toml`.
- Inline test strings:
  - `crates/jit/src/profile/{manifest.rs:422, package.rs (v1 literal + legacy_v1_identity_hash oracle :1040-1114, :1240), package_capture.rs:871, wire.rs tests :261-300,:370-420}`.
  - `crates/jit/src/commands/profile.rs`, `crates/jit/src/test_utils.rs`.
  - `crates/jit/tests/cli_repo_workflow/{profile_cli_tests.rs, profile_edit_refresh_cli_tests.rs}`.
- `scripts/release_version_contract_test.py:172`.

Production readers of v1 key names:
- **`scripts/release-version-contract.py:466-468` reads `profile.jit`** from `profiles/jit-dogfood/manifest.toml`. It must read `compatible-jit`, and its test fixture must change with it. The original claim missed this file.

Decoder and error text:
- `wire.rs:17,42-43` ("expected {V1} or {V2}").
- The error-message test in `profile/package.rs` (`manifest-version = 9`).

Docs:
- `docs/reference/profiles.md:93-99`.

Other consumers:
- `.github/workflows/release-artifacts.yml:139,143` packages both profile directories (bytes only, version-agnostic).
- `scripts/docs-policy-phrases.toml:268` scans dogfood manifest prose only.

#### 3. Primitive verification
- The package hash (`profile/package.rs:964-971`) frames `identity_manifest`. Switching to v2 changes both shipped packages' identity hashes. Target hashes are unchanged, because they derive from canonical contributions, not manifest bytes.
- **This repo records no applied-profile hash.** There is no `.jit/profiles/`, and no 64-hex value appears in any `.jit/*.toml|json`. `jit validate` passes at HEAD, and the switch cannot break it.
- Pinned literals that change:
  - `profile/package.rs:1465` (synthetic-valid package hash). Re-derive it, or replace it with a relational assertion.
  - The `legacy_v1_identity_hash` oracle test at `package.rs:1225-1243`. Delete it.
- Adopter repos holding applied records of v1 packages would see a hash mismatch. Greenfield; out of scope.
- `scripts/generate-template-region.sh` → `regenerate_artifact template-region` renders `.jit/templates.toml` from the package model's template contributions. It does not depend on the version. After conversion, re-run it (and `jit project render`) and confirm there is no diff. No generator emits manifests; capture (`package_capture.rs`) does not write `manifest-version`.

#### 4. Architecture fit
- Delete `ManifestV1*`, `decode_v1`, `v1_declares_template_field` and `canonicalize`. Version detection then needs one parse: decode `ManifestV2` directly and reject `manifest_version != 2` after parsing, or keep the probe.
- `DecodedManifest.identity_manifest` becomes the source bytes in all cases. The field could even go, with the package hashing the manifest file bytes directly.

#### 5. Invariant check
None.

#### 6. Footprint
- One worker: wire.rs (about -130 lines), package.rs tests, 3 fixtures, 2 profiles, about 8 inline-test files, the release-contract script and its test, and profiles.md.
- Run the generators to check for drift: `scripts/generate-template-region.sh`, `jit project render`.
- One test boundary: the decoder rejects `manifest-version = 1` by name.

#### 7. Overlap and sequencing
- 3d9a0ce3:
  - Its REQ-08 lists `profile/wire.rs:240` among the five private canonical-JSON functions. That function is used only by `decode_v1`, so REQ-06 deletes it and shrinks 3d9a0ce3's scope.
  - Its REQ-04 (D-8, delete ownership records) and REQ-06/07/09-13 rework selection and package types in `commands/profile.rs` and `profile/package*.rs`. None of those children is filed yet; the filed children are gates, docs, emoji and rejection.
  - No conflict and no moot point. Land REQ-06 first; it is small and self-contained.
- e643ef66 is a design study (doc gate only). There is no code overlap. Its "capture output format" finding concerns contribution rendering, not the manifest version.
- **Coupling with REQ-10:** the dogfood manifest carries six gate contributions with `auto = true` (`profiles/jit-dogfood/manifest.toml:143-239`). Both REQ-06 and REQ-10 edit that file and change the package hash. Serialize the two workers or put them on one worker.

### REQ-09: canonical lifecycle-state tokens only

#### 1. Claim classification
- "`State::from_str` accepts `open` and `inprogress` (`domain/types.rs:162-164`)": **valid-and-open**.

#### 2. Consumer sweep
Parse sites, all through `State::from_str`:
- `main.rs:2013` (search `--state`), `:3174` (list/query `--state`), `:3585` (issue update `--state`), `:3707` (bulk update `--state`).
- `query_engine/parser.rs:187-191` (`state:` filter).

A second parser, `declarations/rules.rs:372-383` `token_to_state`, serves rule `when.state` and label-coverage `child-state`. It already rejects the aliases (its test is at `rules.rs:1803-1815`). It is a parallel copy of the state vocabulary (`@/inv/convention-convergence`).

Serde (`#[serde(rename_all = "snake_case")]`, `types.rs:16-19`) is strict and covers the issue JSON, batch create, the server and MCP. web/ uses the canonical `State` union (`web/src/types/models.ts:1`).

Tests asserting the aliases: `types.rs:2929,2931`.

Prose:
- `query_engine/error.rs:8-11` ("aliases ... also accepted").
- `rules.rs:368-370`.
- `dev/architecture/core-system-design.md:123` (`--state open`).

Non-token mentions to leave alone:
- `snapshot.rs:354-376` and `commands/graph.rs:686-706` are Debug-format regression guards.
- `transition_graph_enforcement_tests.rs:699` uses a rule name.
- In `mcp-server/test-unit.js:80`, `open` is a status-summary count field.

#### 3. Primitive verification
- `State::all()` (`types.rs:91`) and `State::as_str()` (`types.rs:71-89`) exist.
- `State::from_str` lowercases the input (`types.rs:160`); tests at `types.rs:2915-2923` assert that `BACKLOG` and `Backlog` parse. `token_to_state` also lowercases. Whether case-folding counts as "canonical tokens" is a decision (below).
- `query_engine/error.rs:12-13` `STATE_VALUES` is a hand-maintained list. It can derive from `State::all()`.

#### 4. Architecture fit
Make `State::from_str` the single parser, defined as `State::all().find(|s| s.as_str() == token)`. Make `token_to_state` delegate to it, and derive `STATE_VALUES` from `State::all()`. Domain-pure.

#### 5. Invariant check
None.

#### 6. Footprint
Small, one worker:
- Code: `types.rs`, `rules.rs`, `query_engine/error.rs`.
- Prose fixes.
- About 60 lines.
- One owning unit test: every non-canonical token is rejected, and each `as_str` round-trips. A consumer test is needed only where a site adds its own error mapping (the query parser).

#### 7. Overlap
`.agents/worktrees/agent-7d1bfe89` has uncommitted edits to `query_engine/lexer.rs`, a different file. None otherwise.

### REQ-10: gate fields `auto` and `example_integration`

#### 1. Claim classification
- "The fields are a 'deprecated authored field preserved during round trips' (`declarations/gates.rs:188`)": **valid-and-open** (`gates.rs:187-192`).
  - `auto` has no reader except the output mirror (`output.rs:2752`). Automation already reads `mode` everywhere, and every production writer derives `auto` from `mode`.
  - `example_integration` is not inert. It is wired to `gate define --example` (`cli.rs:1368-1370`, `main.rs:4225,4305`), printed by `gate show` (`main.rs:4525-4527`), and emitted in JSON (`output.rs:2731,2753`; documented at `docs/reference/cli-commands.md:1924`). Removing the field removes that CLI and JSON surface.
- "`add_gate_definition` is reached only by tests": **valid**.
  - Definition: `commands/gate.rs:1155-1191`.
  - Callers: `tests/common/harness.rs:209-220` (the `add_gate` wrapper), `fast_docs_templates/{planning_preset_tests.rs:36, template_apply_atomicity_tests.rs:399, template_apply_tests.rs:955}`.

#### 2. Consumer sweep
Production writers of `auto` and `example_integration`:
- `gate_presets.rs:63-64`.
- `commands/gate.rs:635-636` (update), `:1186` (`add_gate_definition`), `:1244-1245` (`define_gate`).
- `output.rs:2730-2753` (registry JSON mirror).
- `cli.rs:1366,1440` (`--auto` flags; these are CLI sugar for `--mode auto`, not declaration fields).
- `serialize_gate_registry` (`gates.rs:236`) always writes `auto = ...` because the field has no `skip_serializing_if`.

Authored data:
- `.jit/gates.toml`: 20 gates carry `auto = true`.
- `profiles/jit-dogfood/manifest.toml:143,162,181,200,220,239` (gate contribution values).

Projected reference:
- `docs/reference/gate-presets.md:18,62,75,88,102,116`, generated from `gate_presets/reference.rs:22-90,115` by `scripts/generate-gate-presets-reference.sh`. This is the "projected checker reference". `docs/reference/rules-and-gates.md` contains no `auto`.

Other docs:
- `docs/how-to/adopt-planning-bracket.md:238,251,264`.
- `docs/reference/cli-commands.md:1764` (`--auto` flag) and `:1924` (JSON sample).

Test struct literals: `gate_check.rs` tests (3), `gate.rs` tests (`:1588-1777,2243`), `issue.rs:887,986,1045,1282`, `output.rs:2978,3948-3989`, `invariant.rs:99`, `repository_state/mod.rs:1471`, `rules_gates_projection.rs:245`, `gates.rs:391`, and the `gate_store.rs:56,66` TOML.

Integration tests: `cli_item_validate/registry_json_tests.rs:84` (`--example`), plus the `cli_gate` tests listed by grep.

web/ and mcp-server/ do not read either field.

#### 3. Primitive verification
- The "production definition path" is `CommandExecutor::define_gate` (`commands/gate.rs:1203-1250`), reached from `main.rs` (`gate define`). It rejects `Auto` without a checker (`:1222-1226`).
- **Contradicted assumption:** `add_gate_definition(auto = true)` builds an `Auto` gate with `checker: None`, which `define_gate` refuses. Four test sites build that invalid gate (`template_apply_tests.rs:119,704,955`, `template_apply_atomicity_tests.rs:399`). Moving them to `define_gate` means supplying a checker; the natural one is `GateChecker::RepositoryValidation` for `repo-validate`.
- The other definition callers pass `auto = false`: harness `add_gate` at about 21 sites in 6 files, plus `planning_preset_tests.rs:36`. These map to `define_gate(.., GateMode::Manual, None, 100, None)`.
- `GateDefinition` (`gates.rs:153`) has no `deny_unknown_fields`. After the field is deleted, a leftover `auto = true` in an adopter's gates.toml still loads silently.

#### 4. Architecture fit
- Make harness `add_gate` delegate to `define_gate`, with signature `(key, title, description, mode, checker)`, and delete `add_gate_definition`.
- A shared `GateDefinition` test fixture builder would replace about 25 literal sites (`@/inv/shared-test-contracts`). This is optional.
- The repository's `.jit/gates.toml` must agree with the dogfood package contributions. `profile/repository_package.rs` checks the package against the live registry. Strip `auto` from both in the same change.

#### 5. Invariant check
None. `auto` is mechanism vocabulary.

#### 6. Footprint
- **(a) Delete both fields.**
  - Files: declaration, CLI `--example`, output mirror, gate show, writers, test literals, `.jit/gates.toml` (rewrite via serializer or by hand), dogfood manifest, the `reference.rs` constant, then regenerate `gate-presets.md`, plus the docs.
  - Size: about 300 lines.
- **(b) Route tests through `define_gate`.** Harness and 4 direct sites. About 100 lines. Distinct test boundary.

#### 7. Overlap and sequencing
- The dogfood manifest is shared with REQ-06 (above).
- `.agents/worktrees/agent-6ff14f21` (uncommitted) edits `commands/gate.rs`, `gate_cli_tests.rs`, `test_helpers.rs` and `tests/common/harness.rs`. Sequence (b) after it.
- REQ-20 (another slice) edits `commands/gate.rs:822,1039` and `gate_check.rs`, so same-file contention is possible.

### REQ-11: `archived_from` required on archived records

#### 1. Claim classification
- "`archived_from` keeps a fallback for archived records that lack it (`domain/types.rs:484`)": **valid-and-open**.
  - The field doc is at `types.rs:475-489`.
  - The explicit fallback is in revive: `commands/mod.rs:1030-1045`, where the `None` arm warns "archived before its pre-archive state was recorded".
  - `effective_terminal_state` (`types.rs:120-126`) has no special branch; only its doc (`:113-115`) names legacy records.
- "All 41 archived records carry it": **valid**. Recounted: 41 archived and 0 missing; no non-archived record carries the field.

#### 2. Consumer sweep
Writers:
- `commands/mod.rs:1212-1216` (transition sets and clears).
- `repository_state/archive.rs:208-211` (archive plan).

Readers: `effective_terminal_state` and `is_*` (`types.rs:120-153`), `MinimalIssue` (`types.rs:685-740`), `output.rs:1582-1608,1947`, `domain/queries.rs:142`, `main.rs:2078`, and the artifact_{discovery,inventory,plan,classifier} owners.

Tests constructing `Archived` with `None` must be revisited, e.g. `commands/archive.rs:965` `"archived-legacy"` and `:2572`.

Issue decode sites:
- Production: `storage/json.rs:132,920`, `storage/memory.rs:463,473`, `validation/repository.rs:377`.
- Test-only: `repository_state/mutation.rs` tests.

All go through the derived `Deserialize` on `Issue` (`types.rs:411`).

#### 3. Primitive verification
- A plain required serde field cannot express the rule, because the field is legitimately absent on non-archived issues. The invariant is `state == Archived ⇔ archived_from.is_some()`.
- Options:
  - (i) A decode-boundary check: `#[serde(try_from = "IssueRecord")]` or a hand `Deserialize`. Every load path then rejects the record.
  - (ii) A structural finding in `validation/repository.rs` `load_records`. Only `jit validate` and the image path see it.
- Unverified: whether schemars 0.8 honours `serde(try_from)` for `Issue`'s `JsonSchema` derive (`Issue` feeds `--schema`). The worker must check this.

#### 4. Architecture fit
Prefer (i). It is a domain-type invariant enforced at its parse boundary (`@/inv/semantic-types`). After it, the revive `None` arm is unreachable; collapse the `match` to `Some(origin) if target != origin`. `effective_terminal_state`'s doc loses its legacy clause.

#### 5. Invariant check
Purity is preserved: the domain decode stays I/O-free.

#### 6. Footprint
One worker: `types.rs`, `commands/mod.rs`, test fixture fixes, about 80 lines. One owning test: an archived record without the field fails to decode, with an error naming the field.

#### 7. Overlap
None found.

### REQ-12: mermaid classes per lifecycle state

#### 1. Claim classification
- "The exporter declares `classDef open` and has no class for `backlog`, `gated` or `rejected` (`visualization.rs:224-228`)": **valid-and-open**. The lines are now `visualization.rs:217-221`, after the cd2ed136 merge. Nodes use the classes `backlog|ready|inprogress|gated|done|rejected|archived` (`:189-197`), so three of them are undefined and `open` is unused.

#### 2. Consumer sweep
- `visualization.rs:369-388` (`test_export_mermaid_format` asserts `classDef open`).
- `crates/jit/tests/cli_repo_workflow/integration_test.rs:255` (asserts `classDef`).
- No docs, web or MCP consumer reads the class names.
- The dot exporter has its own per-state color table (`visualization.rs:128-136`).

#### 3. Primitive verification
`State::all()` and `State::as_str()` exist (`types.rs:71-101`); no strum. Class names can be `State::as_str()`; mermaid accepts underscores. That renames `inprogress` to `in_progress`.

#### 4. Architecture fit
Use one `fn` mapping `State → fill` and emit `classDef` over `State::all()`. Each node's class is `issue.state.as_str()`. Optionally share the state → style table with dot.

#### 5. Invariant check
None.

#### 6. Footprint
About 30 lines plus one owning test: the declared class set equals `State::all()`, and every node class is declared. Not a graph operation, so no proptest.

#### 7. Overlap
cd2ed136 is merged into main (`f641f2f29`); its issue is still InProgress, awaiting gates. There is no remaining code conflict.

### Cross-slice couplings
- **REQ-04 ↔ REQ-05.** Agent-config strictness must apply to the single surviving type, so do it inside REQ-05. The two types disagree on optionality: `created_at` and `description` are required in `agent_config.rs:33-36` and optional in `config.rs:1447-1450`. REQ-05 must pick one, and docs (`configuration.md:592-639`) show all three fields. `agent.toml`'s `default_ttl_secs` is documented as "parsed metadata only" (`configuration.md:635-637`). That is an inert key of the same kind REQ-03 removes; flag it to the REQ-05 owner.
- **REQ-10 ↔ REQ-20.** Both edit `commands/gate.rs` and `gate_check.rs`.
- **REQ-06 ↔ REQ-10.** Both edit `profiles/jit-dogfood/manifest.toml`.
- **REQ-09 ↔ REQ-16.** REQ-16 deletes `State::is_closed` (`types.rs:36-38`), in the same impl block as REQ-09's change. Trivial merge.
- **REQ-03 ↔ REQ-02.** `configured_worktree_paths` (REQ-02) is unrelated to `worktree.mode`. There is no shared code.
- **REQ-11 ↔ REQ-21.** `artifact_classifier.rs` test owners construct `Archived` with `archived_from: None` (`:1916,1923`).

### Open decisions
1. **Replacement for the `config validate` exit-4 lever** (`failure_lever_registry.toml:129-136`, `command_exit_code_projection_tests.rs:510-522`).
   - Options: (a) `env:JIT_ENFORCE_LEASES=<invalid>`, which bedf0962 may delete; (b) the existing `malformed-config-toml` setup kind, an invalid repo config value already reported post-dispatch (see `project_config_tests.rs:127-160`).
   - **Recommend (b).**
2. **Case-folding in state tokens (REQ-09).**
   - Options: exact snake_case match, matching serde; or keep case-insensitive matching (tests `types.rs:2915-2923`).
   - **Recommend exact match.** It gives one rule for wire, CLI and rules, and literally satisfies "canonical tokens only". `Priority::from_str` stays as is unless the planner wants symmetry.
3. **Canonical selector spelling (REQ-04).**
   - Options: keep `has_doc_type` (serde rename, writers, `configuration.md:411`); or `has-doc-type` (kebab like the assert keys, `validation-rules.md:87`).
   - **Recommend `has_doc_type`.** Both writers and the reference already use it. Fix `validation-rules.md:87` and `graph.rs:1363`.
4. **How far to type the free-form rule tables (REQ-04).**
   - Options: validate known keys at load and keep the table payload; or type `Assertion::{LabelCoverage, LabelReference, DependencyShape}` fully.
   - **Recommend typed `Raw*` structs at load with the payload kept for now.** That is the minimum REQ-04 needs. Full typing belongs to a semantic-types refactor.
5. **`gate define --auto` CLI flag (REQ-10).** It is not a declaration field. **Recommend keeping it out of scope.** REQ-10 names declaration fields; removing the flag is a separate CLI-surface choice. If it is removed, `cli-commands.md:1764` changes too.
6. **`deny_unknown_fields` on `GateDefinition` (REQ-10).** Without it, a stale `auto = true` loads silently. **Recommend adding it** in the REQ-10 worker. It is not required by REQ-04's file list, but it is the same defect class.
7. **Version requirement of `jit-dogfood` → `jit-default` in v2 (REQ-06).** v1 decoded it as `"*"` (`wire.rs:84-90`). **Recommend `"*"`,** so the conversion is a pure wire change.
8. **Where the `archived_from` invariant is enforced (REQ-11).** The options are the decode boundary or a validation finding. **Recommend the decode boundary,** subject to the schemars check.

## 842b0ac6 investigation — group C (REQ-14, 15, 16, 17, 21, 22, 25)

Verified at HEAD `a41eaacf3`. Counts are from grep/python scans over `crates/`, `web/src`, `mcp-server/lib`, `docs`, `dev`, `.agents/skills`, `.jit/schemas`; "test" means inside a `#[cfg(test)]` module or under `crates/jit/tests/`.

Claim tally (18 Background claims in slice): already-done 0, valid-and-open 14, invalid-as-stated 4 (C2, C4, C6, C10).

---

### REQ-14 — one import path per domain type

#### 1. Claim classification
- C1 `domain/mod.rs` re-exports `types::*` "for backward compatibility" — **valid-and-open** (`crates/jit/src/domain/mod.rs:44-45`). A second compat-justified re-export exists: `crate::graph::{DependencyGraph, GraphError, GraphNode}` and `crate::labels::{…}` "The original modules remain at their top-level paths for backward compatibility" (`domain/mod.rs:74-80`).
- C2 "170 references through the facade, 335 through `domain::types::`" — **invalid-as-stated**.
  - Facade (`domain::X` / `domain::{…X…}` naming a `types.rs` item): **546 occurrences in 128 files** (src 565 item refs, tests 135, server 14).
  - `domain::types::`: **339 lines in 36 files**, of which **333 are `crate::domain::types::fixture_issue`** — a `#[cfg(test)] pub(crate)` fixture (`domain/types.rs:491-492`). Only 6 non-fixture lines: `domain/projection.rs:47,216`, `commands/worktree.rs:751`, `domain/store_divergence.rs:167`, and two rustdoc lines `validation/graph.rs:87-88`. Plus `super::types` ×2 inside `domain/`.
  - No `jit::domain::types::` use in `crates/jit/tests` or `crates/server`.

#### 2. Consumer sweep
- Other dual paths created by `domain/mod.rs` re-exports (facade sites / module-path sites, excluding mod.rs): `gate_findings` 11/1, `store_divergence` 1/2, `repository_inputs` 14/5, `event_catalog` 10/3, `event_log` 16/0, `projection` 12/1, `item` **0**/39, `graph` 1 (lib.rs only)/50, `labels` **0**/32.
- Glob user: `crates/jit/tests/fast_issue/domain_queries_tests.rs:6` (`use jit::domain::*`).
- Third path at crate root: `crates/jit/src/lib.rs:45` (`Issue, Priority, State`), `:51-54` (`query_*`), `:57` (`DependencyGraph, GraphError, GraphNode`). ~10–17 consumers: `main.rs` (×6, e.g. `jit::GraphError` at 245-248, 445, 9105), `visualization.rs` ×3, tests `template_apply_tests.rs:1501-1502`, `transition_graph_enforcement_tests.rs`, `config_loading_tests.rs`, `local_rule_enforcement_tests.rs`.
- No docs/dev/skills/web/MCP reference `domain::types`.
- Stale prose: `domain/mod.rs:1-22` lists `graph`, `validation`, `labels` as domain submodules (they are crate-root modules).

#### 3. Primitive verification
- A glob `pub use types::*` re-exports `pub(crate)` items at `pub(crate)` visibility, so `crate::domain::fixture_issue` resolves under `cfg(test)` (expected Rust semantics; first compile confirms).
- Making `mod types` private turns the single-path rule into a compiler-enforced guard for `types.rs` items.

#### 4. Architecture fit
- Recommended single path: **`domain::<Type>` facade for `types.rs` items** (private `mod types;` + `pub use types::*;`), **module path (`domain::item::X`, `crate::graph::X`, `crate::labels::x`) for every other submodule**; delete the per-submodule selective re-exports and the `graph`/`labels` re-export; delete lib.rs root re-exports of domain/graph/query items (or keep exactly one path — see Open decisions).
- 2195c98f adds `IssueId`, `IssueIdPrefix`, profile identity "in the domain layer"; under this convention a type added to `types.rs` is reachable only as `domain::X`, a new submodule only by module path.

#### 5. Invariant check
None (path-only change; no layer movement).

#### 6. Footprint
- Mechanical: `sed 's/domain::types::/domain::/g'` over 36 files + 2 `super::types` + `mod types` visibility → ~40 files, ~345 line edits, zero semantic change.
- Re-export removal: ~64 facade sites for the six sibling modules + ~17 crate-root sites → ~30 files, hand edits (brace groups mix items).
- Split point: (a) `types` facade (sed-able) and (b) sibling/crate-root re-export removal are independent tasks with one test boundary each (`cargo build` + full suite).

#### 7. Overlap / sequencing
- Touches imports in most files of crates/jit → conflicts with every concurrent worker. Recommend running it **last** in the epic (sed is re-runnable after rebases).
- d23c58c0 adds a new `crate::domain::types::fixture_issue` site (`type_taxonomy.rs` test, worktree commit `7691e0e09`); sed covers it if d23c58c0 merges first.
- `domain/mod.rs:61` re-exports `render_event_reference` — also edited by REQ-22.

---

### REQ-15 — output surface with production callers

#### 1. Claim classification
- C3 `output_message!`, `output_json!`, `JsonError::invalid_state`/`invalid_priority`/`gate_validation_failed`, `ExitCode::all_codes_documentation`, `OutputContext::print_error`/`is_quiet`/`is_json` have no callers — **valid-and-open**. Zero callers anywhere in crates (only `dev/archive/**` historical prose).
  - Definitions: `output_macros.rs:6` (`output_message!`), `:47` (`output_json!`); `output.rs:1211,1217,1226`; `output.rs:622`; `output.rs:366,371,376`.
- C4 "they hide behind six blanket `allow(dead_code)`" — **invalid-as-stated**. Six exist (`output.rs:463` `JsonError`, `:469` `impl JsonError` core, `:528` `ErrorDetail`, `:553` `ExitCode`, `:592` `impl ExitCode`, `:1179` `impl JsonError` helpers) but are **inert**: `output` is `pub mod` in `lib.rs:24`, so `dead_code` never fires on these `pub` items; `#[macro_export]` macros are likewise exempt from `unused_macros`. Removing the allows produces no warning, so no lint guards REQ-15.

#### 2. Consumer sweep (beyond the listed items)
- `JsonError::from_code_text` (`output.rs:490`) — **test-only** (`output.rs:3361,3647`). Not in Background; REQ-15 requires its deletion.
- `ExitCode::description` (`output.rs:600`) — only caller is `all_codes_documentation`. Meanwhile `schema.rs:835-881` `generate_exit_codes()` hand-copies the same ten descriptions (`@/inv/single-source-prose` defect).
- Live and fine: `JsonError::new`, `with_details`, `with_suggestion(s)`, `to_json_string`, `exit_code`, `issue_not_found` (main.rs ×10), `gate_not_found` (×3), `cycle_detected` (×2), `transition_blocked`; `OutputContext::new`, `print_data` (×60), `print_info` (×30), `print_success` (×28), `print_warning` (×32); `output_data!` (main.rs ×3); `handle_json_error!` (main.rs ×18); `ExitCode::code`.
- No consumer in `crates/server`, `mcp-server`, `web`, `docs`, `.agents/skills`.
- `invalid_state` suggestion text lists retired `open` state (`output.rs:1214`) — deleted with it.

#### 3. Primitive verification
- Schema descriptions in `schema.rs:837-880` are string-identical to `ExitCode::description` (incl. BrokenPipe after line-continuation), so deriving `generate_exit_codes` from `ExitCode` leaves `docs/reference/exit-codes.md` byte-identical.
- `ErrorCode::ALL` (`output.rs` ErrorCode) is the existing enumeration pattern to mirror for an `ExitCode::ALL`.

#### 4. Architecture fit
- Delete the nine listed items + `from_code_text` + the six inert allows. Give `ExitCode::description` its production caller by deriving `CommandSchema::generate_exit_codes` from an `ExitCode::ALL` + `description()` (removes a hand-kept copy), or delete `description` and keep the schema table. Recommend derive.

#### 5. Invariant check
Deriving the exit-code table serves `@/inv/single-source-prose`; no purity concern.

#### 6. Footprint
`output.rs` (~-90 lines), `output_macros.rs` (-~40), `schema.rs` (`generate_exit_codes` → derived, ~-40/+10), tests in `output.rs` mod tests (`from_code_text` tests at 3355-3365, 3640-3655). One task, one boundary.

#### 7. Overlap
- df129d9e (state **Ready**, no dependency on 842b0ac6): REQ-11 deletes `handle_json_error!` and per-command JSON helpers; REQ-01/18 remove success bodies from main.rs (→ `output_data!`); REQ-05/17 edit `schema.rs` (`get_output_schema_for_command`). File-level overlap: `output_macros.rs`, `output.rs`, `schema.rs`, `main.rs`. None of the REQ-15 items is in df129d9e's list. After both land, `output_macros.rs` is empty — whichever lands second deletes the file and `main.rs:17 mod output_macros;`.
- df129d9e REQ-15 (one global `--json` flag) may route the flag through `OutputContext`, which could give `is_json` a caller. Recommend deleting now (D-18); df129d9e adds what it needs.
- `render_exit_code_reference` prose names `error_to_exit_code` in `main.rs` (`schema.rs:1145-1150`); df129d9e REQ-11 moves the classifier → stale prose there (df129d9e's sweep).

---

### REQ-16 — domain dead items

#### 1. Claim classification and 2. consumer sweep
| Item | Def | Production callers | Test callers | Outside crates/jit | Class |
|---|---|---|---|---|---|
| `KindScope::parse`, `parse_for`, `ItemError::InvalidScope` | `domain/item.rs:404,409,473` | none | none | none | valid |
| `KindScope::ISSUE_TOKEN`/`PROJECT_TOKEN` | `item.rs:394,396` | only `parse_for` → dead after | — | — | (add to scope) |
| `HierarchyError` | `type_taxonomy.rs:66-74` | **`InvalidLabel` constructed** by `extract_type` (`:231`), called at `:330,377,426`; every caller discards the error (`Err(_)`). `UnknownType` never constructed (`#[allow(dead_code)]` `:69`) | `type_taxonomy.rs:657-676` | none | **invalid-as-stated** |
| `ConfigError::DuplicateType` | `type_taxonomy.rs:128` (`allow(dead_code)`) | never constructed | none | none | valid |
| `detect_membership_issues` | `type_taxonomy.rs:478` (`allow(dead_code)`) | none | `crates/jit/tests/fast_rules/label_membership_validation_tests.rs` (whole 400-line file, 10 tests; `mod` at `fast_rules/main.rs:14`) | none | valid |
| `ValidationIssue::InvalidMembershipReference` | `type_taxonomy.rs:86` | constructed only in `detect_membership_issues` (`:527,550`); matched at `:599` (`generate_fixes`) and `commands/validate.rs:1509` | membership test file | none | valid |
| `State::is_closed` | `domain/types.rs:64` | none | none | none | valid |
| `MinimalIssue::state_symbol` | `domain/types.rs:746` | **`main.rs:2088`** (`print_issue_show_human`, `IssueShowResponse.dependencies: Vec<MinimalIssue>`, `output.rs:1866`) | — | — | **invalid-as-stated** |
| `DocumentReference` builders | `domain/types.rs:848-899` (each `allow(dead_code)`) | none. `with_format`, `with_assets`: zero callers (hits at `output.rs:253`, `artifact_discovery.rs:460` are `artifact_plan.rs:683`'s `with_format`). `new`, `at_commit`, `with_label`, `with_type`: test-only | 38 sites: `commands/archive.rs` 14, `domain/types.rs` 10, `projection.rs` 3, `repository_state/archive.rs` 2, 1 each in `gate_check.rs`, `declarations/rules.rs`, `artifact_conformance.rs`, `gate_execution.rs`, `validation/local.rs`; integration tests `artifact_discovery_tests.rs`, `artifact_inventory_tests.rs`, `example_rulesets_tests.rs` ×2 | web `DocumentReference` TS type (struct shape, unaffected) | valid |
| `LabelNamespaces::contains` | `domain/types.rs:3376` (`allow(dead_code)`) | none | none | none | valid |
| `GateRunStatus::Skipped` | `domain/types.rs:1111` | never produced by a run; constructed only by `--status skipped` filter parse `main.rs:1447` | none | see wire list below | valid (constructed only as a filter value) |
| `errors::not_in_git_repo` | `errors.rs:1065` | none | `errors.rs:1280` | `dev/benchmarks/**` test-name evidence (regenerated artifacts; leave) | valid |

`skipped` wire consumers (all must change with the variant):
- `crates/jit/src/main.rs:1447` arm and message `:1451` (`parse_run_status`, a hand-written token table).
- `crates/jit/src/cli.rs:1734`, `:1780` help text → flows into `jit --schema` flag description (`commands/gate/subcommands/status/flags/3/description`) and the `GateRunStatus` enum in `status-all` output schema (JsonSchema derive).
- `crates/jit/src/storage/reference.rs:215` hand-written status list → regenerates `docs/reference/storage-records.md:130` (generated artifact; run its `scripts/generate-*` entry point).
- `docs/reference/cli-commands.md:1997` (hand-written).
- `web/src/types/models.ts:29` (`GateRunSummary.status` union); `IssueDetail.tsx:552` renders unknown statuses as `·` (no `skipped` branch).
- MCP: no checked-in copy; tools read `jit --schema` at runtime (`mcp-server/lib/schema-loader.js`). No `.jit/schemas` entry.

#### 3. Primitive verification
- `extract_type` callers need only presence; `Option<String>` return lets `HierarchyError` be deleted entirely. Alternative: keep `HierarchyError { InvalidLabel }` and delete only `UnknownType`.
- `generate_fixes` and `validate.rs:1509` arms become unreachable once the variant goes; `ValidationIssue` then has one variant (`UnknownType`).

#### 4. Architecture fit
Pure deletions in `domain/`; `validate.rs` (commands) loses one match arm. Membership-reference checking that production performs lives in rules (`validation/graph.rs` label-reference assertions), so no property is lost.

#### 5. Invariant check
`detect_membership_issues` is domain-agnostic already; deletion only. Web `models.ts` union shrink follows the wire.

#### 6. Footprint / split points
- C-a taxonomy: `type_taxonomy.rs` (HierarchyError/extract_type, DuplicateType, detect_membership_issues + variant), `commands/validate.rs:1509-1518`, delete `label_membership_validation_tests.rs` + mod line. ~-550 lines.
- C-b `GateRunStatus::Skipped` wire removal: `types.rs`, `main.rs`, `cli.rs`, `storage/reference.rs` + regenerate `docs/reference/storage-records.md`, `docs/reference/cli-commands.md`, `web/src/types/models.ts`. Own test boundary (gate status `--status skipped` rejected).
- C-c small deletions: `item.rs` (parse/parse_for/InvalidScope/tokens), `types.rs` (`is_closed`, `LabelNamespaces::contains`, DocumentReference builders), `errors.rs` (`not_in_git_repo` + test). Builders: if `new`/`at_commit`/`with_label`/`with_type` go, 38 test sites switch to struct literals or a shared fixture (`@/inv/shared-test-contracts`) — ~+60 lines.

#### 7. Overlap
- d23c58c0 (in flight, `7691e0e09`, base `31fb675b5`): changes `type_taxonomy.rs:446` (`validate_orphans` tuple order) and inserts a test after `:786`. REQ-16 C-a edits `:66-74`, `:86-92`, `:121-129`, `:478-~575`, `:599`, `:652-676`. Disjoint hunks; textual merge expected clean. Sequence C-a after d23c58c0 merges.
- 2195c98f REQ-11 (`KindScopeConfig` mirrors `item::KindScope`, `config.rs:785-845`) and REQ-11 gate status enum; it depends on this epic, so deletions land first.

---

### REQ-17 — no `unimplemented!()`; no link-rewrite method

#### 1. Claim classification
- C15 `DocFormatAdapter::rewrite_links` defaults to `unimplemented!()` — **valid-and-open** (`document/adapter.rs:48-50`, macro at `:49`).

#### 2. Consumer sweep
- Only `unimplemented!`/`todo!` in workspace Rust: `document/adapter.rs:49`. None in `crates/server`, tests.
- Implementors `HtmlAdapter`, `MarkdownAdapter` (and any parser-gated adapters) do not override it; zero callers anywhere.
- Stale prose to fix with it: trait doc `adapter.rs:12` ("asset scanning and link rewriting"), method doc `:44-47`.

#### 6. Footprint
One file, ~-8 lines. Tiny; fold into REQ-16 C-c.

---

### REQ-21 — container-destination classification

#### 1. Claim classification
- C16 `ContainerDestinationState` has six variants for three outcomes — **valid-and-open** (`domain/artifact_classifier.rs:243-257`). Sole consumer `append_container_destination_blocker` (`:1602-1628`) maps: `Absent`/`OwnedByTarget`/`MarkerlessAccounted` → no blocker; `OwnedByOther(_)`/`MarkerlessWithUnaccountedEntries` → `DestinationConflict`; `Symlink` → `SymlinkArtifact`. `OwnedByOther`'s `String` is never read.

#### 2. Consumer sweep
- Constructors: `artifact_classifier.rs:549` (`_ => Absent` for document targets or empty `archive_root`), `:586-628` (`container_destination_from_evidence`).
- Tests: `:2612,2616` (asserts Accounted vs Unaccounted), `:3944`, `:3991`, `:4076-4212` (facts literals), `:2957`, `:4672` (`ArtifactClassificationFacts::default()`).
- No consumer outside `artifact_classifier.rs`; not serialized; not in web/MCP/docs.

#### 3. Primitive verification — "never-inspected location defaults to a value"
- `ArtifactLocation` derives `Default` with `#[default] Unknown` ("The caller did not inspect this path. Production planners should avoid this state", `:193-197`); `ArtifactLocationFacts`, `ContainerDestinationState` (`#[default] Absent`) and `ArtifactClassificationFacts` derive `Default`.
- Production default sites: `:966` `locations.get(&source).cloned().unwrap_or_default()` (missing evidence → `Unknown`, then falls through `_` arms silently); `:536-539` destination `Missing` when `inspect_destinations` is false or `layout.destination` is `None`; `:549` `Absent` for non-inspected container destinations.
- No code matches `ArtifactLocation::Unknown` explicitly.
- Existing typed error: `ArtifactEvidenceError::IncompleteEvidence { path }` (`:410-420`, thiserror), produced by `required_resolution_evidence` (`:495-503`). A parallel anyhow twin `required_evidence` (`:631-639`, "archive evidence is missing for {path}") serves `classification_facts_from_evidence` (`anyhow::Result`, `:519`). `classify_artifacts` returns `PlanError` (`artifact_plan.rs:1115`, hand-written Display).

#### 4. Architecture fit
- Collapse to three variants (e.g. clear / conflict / symlink). Delete `ArtifactLocation::Unknown` and the four `Default` derives; replace `:966` with a lookup that fails with an evidence error. Converge `required_evidence` onto `ArtifactEvidenceError` and make `classification_facts_from_evidence` return it (removes anyhow from this domain path). `classify_artifacts` needs either a `PlanError` variant wrapping the missing-evidence path or a return type carrying `ArtifactEvidenceError`.
- Storage side `storage/artifact_planning.rs:82-145` builds the evidence map; only its error type changes.

#### 5. Invariant check
Domain stays pure; change tightens `@/inv/semantic-types`/`convention-convergence` (one evidence-error path).

#### 6. Footprint
`artifact_classifier.rs` (~4,680 lines; edits at ~10 production sites + ~12 test sites), `artifact_plan.rs` (PlanError variant), `storage/artifact_planning.rs` (error mapping). ~+40/-60. One task; test boundary: classifier unit tests (missing location evidence → error; three outcomes).

#### 7. Overlap
- 2195c98f REQ-14 rewrites `PlanError` to derive-based (`artifact_plan.rs:1138-1187`); it depends on 842b0ac6, so REQ-21 lands first and 2195c98f converts the new variant too.
- Not in scope but adjacent: `resolve_container_destination` keeps a "legacy" short-id destination fallback (`artifact_classifier.rs:417-431`, `storage/artifact_planning.rs:27-75`, `commands/archive.rs:203-224,534`). See Open decisions.

---

### REQ-22 — reference-page renderers only in generated-artifact builds

#### 1. Claim classification
- C17 Renderers compile into adopter builds; only test-support `generated_artifacts` calls them — **valid-and-open**.

#### 2. Consumer sweep
| Renderer | Def | Gate today | Callers |
|---|---|---|---|
| `output::render_error_code_reference` | `output.rs:1035` | none | `generated_artifacts.rs:166`; tests `output.rs:3680,3705` |
| `domain::event_catalog::render_event_reference` | `event_catalog.rs:525` (+ re-export `domain/mod.rs:61`) | none | `generated_artifacts.rs`; tests `event_catalog.rs:867,877` |
| `schema::render_exit_code_reference` | `schema.rs:1134` | none | `generated_artifacts.rs:174`; test `schema.rs:1684` |
| `gate_presets::reference::render_reference_markdown` | `gate_presets/reference.rs:96` (+ re-export `gate_presets.rs:20`) | none | `generated_artifacts.rs:179`; tests `reference.rs:180,233,254-256` |
| `runtime_defaults::render_reference_markdown` | `runtime_defaults.rs:56` | none | `generated_artifacts.rs:183`; tests `runtime_defaults.rs:125,134` |
| `storage::reference::render_reference_markdown` | `storage/reference.rs:456` (+ re-export `storage/mod.rs:78` with `GateRunField`) | none | `generated_artifacts.rs:187`; tests `reference.rs:662,795,806,841` |
| `profile::template_region::render_template_registry` | `template_region.rs:167` | **already gated**: module `#[cfg(any(test, feature = "test-support"))]` (`profile/mod.rs:37-38`) | `generated_artifacts.rs:210`; test `repository_package.rs:901` |
- `generated_artifacts` gated `#[cfg(any(test, feature = "test-support"))]` (`lib.rs:20-21`); used by `crates/jit/examples/regenerate.rs` (examples build with dev-deps).
- No production caller: no `commands/`, `main.rs`, `jit project render`, or server call.
- Only-renderer helpers that would warn when gated: `storage/reference.rs` (`sample_*`, `field_presence`, `event_log_sample`, `cell`, `Presence`; `GateRunField` has no production consumer — whole module gate-able), `gate_presets/reference.rs` (`PORTABLE_CHECKER_REGISTRY_EXAMPLES`; whole module gate-able), `event_catalog.rs` render helpers (keep `event_catalog()`, used by `schema.rs:201`), `runtime_defaults.rs` (keep constants).

#### 3. Primitive verification
- Feature `test-support` exists (`crates/jit/Cargo.toml:85-95`), enabled by the self dev-dependency (`:103`) and `crates/server/Cargo.toml:66`. Every `<module>::test_support` path-constant submodule already uses `#[cfg(any(test, feature = "test-support"))]` (`output.rs:1016`, `schema.rs:1117`, `event_catalog.rs:34`, `gate_presets/reference.rs:11`, `runtime_defaults.rs:40`, `storage/reference.rs:42`). Reuse that attribute; no new feature.
- The `Cargo.toml:85-95` feature comment says definitions "compile unconditionally either way"; it is already inaccurate (`generated_artifacts`, `template_region` are cfg-gated) and needs updating with REQ-22.
- No configured gate builds the lib with `test-support` off: `scripts/cargo-ci.sh:369-492` runs `clippy --workspace --all-targets` (dev-deps unify the feature on), nextest, doctest. Dead-code warnings in the feature-off build are only visible via `cargo build --workspace` / `cargo check -p jit --lib` (used by `scripts/cargo-ci-selftest.sh:501`, not a gate).
- Intra-doc links from ungated docs to gated items (e.g. `runtime_defaults.rs:10`, `event_catalog.rs:7,504`, `output.rs:1027-1033`) break under feature-off `cargo doc`.

#### 4. Architecture fit
Apply the existing cfg attribute to each renderer, its helpers, and its re-export (`domain/mod.rs:61`, `gate_presets.rs:20`, `storage/mod.rs:78`); gate `storage::reference` and `gate_presets::reference` at module level.

#### 5. Invariant check
Supports `@/inv/bounded-rust-build-footprint`; no domain-agnostic concern.

#### 6. Footprint
8 files (`output.rs`, `schema.rs`, `domain/event_catalog.rs`, `domain/mod.rs`, `gate_presets.rs`, `gate_presets/reference.rs`, `runtime_defaults.rs`, `storage/mod.rs`/`storage/reference.rs`) + `Cargo.toml` comment; ~+15 attribute lines. Verification: `RUSTFLAGS=-Dwarnings cargo check --workspace` (feature off) plus the usual gates.

#### 7. Overlap
Same files as REQ-15 (`output.rs`, `schema.rs`) and REQ-14 (`domain/mod.rs`). Sequence REQ-15 → REQ-22 or assign to one worker.

---

### REQ-25 — every web module imported

#### 1. Claim classification
- C18 No module imports `web/src/types/hierarchy.ts` — **valid-and-open**. Only importers of `hierarchy` in `web/src` are none; git history `410bd6fd2`, `1a9112b24`.

#### 2. Consumer sweep
Full scan of `web/src` (67 files; resolver over `import`/`export from`/dynamic `import()`/`vi.mock`): every non-entry module is imported except `src/types/hierarchy.ts`. Roots not imported by design: `src/main.tsx` (`web/index.html:11`), `src/test/setup.ts` (`vitest.config.ts:9`), `src/vite-env.d.ts` (ambient), 28 `*.test.ts(x)` files. `hierarchy.ts` imports nothing, so deletion orphans nothing. Not referenced by docs.

#### 3. Primitive verification
- No unused-module tool installed (`web/node_modules/.bin` has no knip/ts-prune/madge/unimported); `eslint.config.js` uses only recommended + react-hooks + react-refresh; tsc `noUnused*` is per-file. No existing lint can guard REQ-25.

#### 5. Invariant check
`hierarchy.ts:43-45` hardcodes strategic = `level <= 2` (a domain-agnostic defect); deletion removes it.

#### 6. Footprint
Delete one 133-line file. Verification: `cd web && npm test && npm run lint && npm run build`.

---

### Cross-slice couplings
- `output_macros.rs`: REQ-15 deletes two macros; df129d9e deletes `handle_json_error!` (and likely `output_data!`). Last to land deletes the file.
- `schema.rs`: REQ-15 (`generate_exit_codes` derivation), REQ-22 (gate `render_exit_code_reference`), df129d9e REQ-05/17 (`get_output_schema_for_command`).
- `domain/mod.rs`: REQ-14 (re-exports) and REQ-22 (`render_event_reference` re-export).
- `domain/types.rs`: REQ-14 (`fixture_issue` path), REQ-16 (several deletions), group-other REQ-09/REQ-11 (`State::from_str` aliases `:162-164`, `archived_from` fallback `:484`).
- `storage/reference.rs`: REQ-16 C-b (`skipped` text `:215`) and REQ-22 (module gating); regenerate `docs/reference/storage-records.md`.
- `main.rs`: REQ-16 C-b (`parse_run_status`) vs df129d9e and other slices' claim-command edits.
- `commands/validate.rs`: REQ-16 C-a (arm `:1509`) vs REQ-01 (`validate_integrity_silent`, `:1835-2640`) — different regions.
- 2195c98f depends on 842b0ac6: REQ-14 path convention and REQ-21 `PlanError` variant feed its REQ-11/REQ-14.

### Open decisions
1. **REQ-14 path choice.** (a) `domain::<Type>` facade with private `mod types` (~345 sed-able edits, compiler-enforced); (b) `domain::types::<Type>` everywhere, delete glob (~546 hand edits). Recommend (a).
2. **REQ-14 scope of "one path".** Also remove sibling-submodule re-exports in `domain/mod.rs` and crate-root domain re-exports in `lib.rs:45,51-57`? Recommend yes (REQ-14 says each domain type), as a separate task.
3. **REQ-15 `ExitCode::description`.** Derive `schema::generate_exit_codes` from it via `ExitCode::ALL` (removes a hand copy) vs delete it. Recommend derive.
4. **REQ-16 `HierarchyError`.** Claim wrong: `InvalidLabel` is live. (a) `extract_type -> Option<String>`, delete the enum; (b) keep the enum, delete `UnknownType`. Recommend (a): all callers discard the error.
5. **REQ-16 `MinimalIssue::state_symbol`.** Claim wrong: production caller `main.rs:2088`. (a) Drop from REQ-16 scope; (b) converge it with the identical `DependencyTreeNode::state_symbol` (`output.rs:1607`) into one output-layer renderer, then delete the domain method. Recommend (b): presentation symbol belongs in output, and the twin is a convention-convergence defect. Either way REQ-16's text names it, so the criterion needs (b) or an amendment.
6. **REQ-16 `DocumentReference` builders.** Delete only the zero-caller `with_format`/`with_assets`, or all six (38 test sites rewritten). Recommend all six; tests construct the struct through one shared fixture.
7. **REQ-22 verification.** No gate builds with `test-support` off. (a) Worker-run `RUSTFLAGS=-Dwarnings cargo check --workspace` as evidence; (b) add a feature-off check step to `cargo-ci.sh` (extra build cost). Recommend (a).
8. **REQ-25 guard.** No existing lint. (a) Delete only; (b) add a vitest import-graph test or knip. Recommend (a) (D-18; REQ-25 doesn't ask for a guard).
9. **Out-of-slice shim found.** Archive "legacy" short-id destination fallback (`artifact_classifier.rs:417-431`, `storage/artifact_planning.rs:27-75`, `commands/archive.rs:203-224`). Not in 842b0ac6's Background. Recommend recording it as a follow-up rather than widening REQ-21.

## Shared-file coordination

Coordination for files that two or more unordered leaves touch. *Edge* means the regions interact and the graph orders them. *Rebase* means the regions are disjoint, so whichever leaf lands second rebases with conflicts at most textual.

| File | Leaves: region | Coordination |
|---|---|---|
| `config.rs` | `remove-inert-config-keys`: inert fields, `WorktreeMode`, `Merged*` sections; `state-config-defaults-once`: per-layer and `Merged*` accessors, default constants; `strict-config-toml-loading`: struct attributes, `load` error cause, tolerance tests; `unify-agent-configuration`: `AgentConfig`/`AgentIdentity` `:1434-1481` and its tests | Edge: inert keys → defaults once → strict loading, because the last two rewrite the same structs. Agent config: rebase |
| `main.rs` | `drop-claim-type-parameter` `:8275-8519`; `move-state-symbol-to-output` `:2088`; `remove-gate-auto-fields` `:4225,4305,4525`; `remove-inert-config-keys` `:6804-6897,7006-7018`; `state-config-defaults-once`: `config show`; `strict-config-toml-loading` `:6989,6999`; `remove-lifecycle-backfill` `:23,7873-7895`; `remove-skipped-gate-run-status` `:1447-1451` | The config leaves are ordered by edge; the rest rebase |
| `cli.rs` | `remove-gate-auto-fields` `:1366-1370`; `remove-lifecycle-backfill` `:435-440,2310-2330,3265-3870`; `remove-skipped-gate-run-status` `:1734,1780` | Rebase |
| `commands/mod.rs` | `remove-bracket-breakdown` `:31,74`; `remove-lifecycle-backfill` `:48,87`; `remove-gate-auto-fields` `:3528-3546,3675`; `require-archived-from` `:1030-1045` | Rebase (adjacent module-list lines only) |
| `commands/gate_check.rs` | `remove-linked-worktree-evidence` `:91-94,412-425`; `remove-gate-auto-fields`: test gate literals; `unify-gate-not-required-error` `:204-209`; `remove-test-only-gate-entry-points` `:928,1043-1049`; `remove-reference-builders`: one test site | The gate leaves are ordered by edge; linked evidence and builders rebase |
| `commands/validate.rs` | `remove-silent-integrity-validator` `:1835-2640`; `keep-one-enforcement-drift` `:1743,1777`; `remove-taxonomy-dead-errors` `:1509-1518` | Rebase |
| `commands/archive.rs` | `require-archived-from`: tests `:965,2572`; `remove-reference-builders`: 14 test sites | Rebase |
| `declarations/rules.rs` | `canonical-state-tokens`: `token_to_state` `:368-383`; `strict-rule-declarations`: `Selector`, `RawRule`, typed label-coverage tables carrying `child-state`; `remove-reference-builders`: one test site | Edge: state tokens → rule strictness, because the typed label-coverage struct parses `child-state` through `State::from_str`. Builders rebase |
| `domain/types.rs` | `canonical-state-tokens` `:160-164`; `remove-small-domain-helpers` `:64,3376` (ordered after it); `require-archived-from` `:113-126,411,475-489`; `move-state-symbol-to-output` `:746`; `remove-reference-builders` `:848-899` and tests; `remove-lifecycle-backfill` `:1553-2010`; `remove-skipped-gate-run-status` `:1111` | Rebase |
| `domain/artifact_classifier.rs` | `require-archived-from`: tests `:1916,1923`; `collapse-container-destination-states` `:193-257,536-639,966,1602-1628` and fact tests | Rebase |
| `domain/event_catalog.rs` | `remove-lifecycle-backfill`: the backfill tag entries; `gate-reference-renderers`: render helpers `:504-525` | Rebase |
| `gate_execution.rs` | `remove-test-only-gate-entry-points` `:26-39` and its test calls; `remove-reference-builders`: one test site | Rebase |
| `output.rs` | `prune-output-surface` `:366-622,1179-1226`; `gate-reference-renderers` `:1016-1035` (ordered after prune); `remove-gate-auto-fields` `:2730-2753`; `move-state-symbol-to-output` `:1607,1866` | Rebase |
| `repository_state/mod.rs` | `remove-linked-worktree-evidence` `:60-61` and `close` sites; `remove-repository-state-cfg-twins` `:34-51,80-99,214-230` (ordered after it); `remove-gate-auto-fields`: test literal `:1471` | Rebase |
| `repository_state/{archive,mutation,materialize}.rs` | `remove-linked-worktree-evidence`: `close` arguments; `remove-reference-builders`: archive tests; `remove-lifecycle-backfill`: mutation `:372-380,941-953,2400-2430`; `strict-config-toml-loading`: materialize test strings | Rebase |
| `storage/json.rs` | `remove-read-lock-sidecar-sweep` `:395-442,1304-1333`; `converge-memory-event-parsing`: test `:1830`; `remove-storage-cfg-twins` `:453-482` (ordered after converge) | Rebase |
| `storage/lock.rs` | `remove-read-lock-sidecar-sweep` `:440-492,1015-1080`; `single-file-lock-poll-loop` `:113-395` and tests `:512-827` | Rebase |
| `storage/mod.rs` | `remove-read-lock-sidecar-sweep`: trait `:193-224`; `gate-reference-renderers`: re-export `:78` | Rebase |
| `storage/repository_state_store_tests.rs` | `remove-linked-worktree-evidence` `:140-166,2791`; `converge-memory-event-parsing`: new matrix test after `:660` | Rebase |
| `test_utils.rs` | `adopt-manifest-version-two`: inline manifest; `strict-config-toml-loading`: inline config strings; `remove-reference-builders`: new fixture | Rebase |
| `tests/…` shared files | `bracket_coverage_gate_run_test.rs` (breakdown doc `:16-18,151`, gate TOML `:130`); `failure_lever_registry.toml` (`:129-136`, `:924-925`); `integration_test.rs` (`:255`, `:431`); `harness_demo.rs` (gate helper calls, `:502`); `template_apply_atomicity_tests.rs` (`:399`, `:768`) | Rebase |
| `profiles/jit-default/manifest.toml` | `adopt-manifest-version-two` `:10-13`; `remove-inert-config-keys` → `state-config-defaults-once` (ordered): comment block `:440-466` | Rebase (no pinned `jit-default` hash) |
| Docs | `configuration.md` (inert `:29,582-670`, defaults `:482,560,564`, agent `:592-623`); `cli-commands.md` (auto `:1764,1924`, skipped `:1997`, backfill `:3904-3925`, inert `:3970-4005`); `storage-format.md` (sweep `:77-82`, backfill `:233-256,427`); `dev/architecture/core-system-design.md` (auto `:61`, states `:123`) | Rebase |
