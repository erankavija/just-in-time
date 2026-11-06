# 5046408a bootstrap/diagnostic boundary correction proposal

Status: revised proposal for independent review R2; implementation remains unauthorized. R1 was FAIL on the omitted existing storage test owner, as recorded below. No source edits. Steward design acceptance and an independent bounded design review are required before implementation. Current worktree HEAD is `376978ed05c011717ab7c2d88fab54954f045f15`; it contains the failed configured-gate evidence.

## Evidence and problem

The issue introduced a full `JitConfig` assembly in every bootstrap observation, before a command can diagnose or repair declarations. That is stronger than root/policy selection needs. Two existing contracts now fail:

1. `config validate` must report malformed or semantically invalid repository configuration through `ConfigValidationReport` and `ConfigValidateResponse`: plain findings on stdout, summary on stderr, JSON `VALIDATION_FAILED` with `{valid:false, errors:[...]}` under `error.details`, and exit4. Bootstrap instead returns a parse error/exit1.
2. `config set project.name good-name` must repair an already-invalid project name, while an edit leaving that invalid name in place must fail without writing. The existing command validates its edited document and coupled proposed state; bootstrap now rejects the invalid preimage before the command can do so.

Actual unchanged-test RED:

- `cargo test -p jit --test cli test_command_exit_codes_config_validate_invalid_value_emits_4`: failed, plain exit1 versus required exit4; `5046408a-rework-2-red-projection.log`.
- `cargo test -p jit --test cli cli_repo_workflow::project_config_tests`: 10 passed/4 failed: three diagnostic-output failures and the valid-name repair failure; `5046408a-rework-2-red-config.log`.

All logs are under `/tmp/f4d9502a-lead/`. Both commands are collected. No assertion or source has changed.

## Recommended boundary

Use one canonical parsed configuration document with typed projections, and make `config validate` an always-data-only diagnostic operation. It does not intrinsically need a code checkout, code Git discovery, or a complete `CommandExecutor`. Its only reason for reaching a code path is validation of an expressly supplied root setting; that is a value check, not authority to read or write code.

### Canonical parsing and observations

- The existing declaration owner parses `config.toml` syntax once into a canonical document representation. Full configuration assembly and bootstrap projection are views of that document. The full view retains existing `JitConfig` checks, unknown-field rules and templates/invariants assembly. There is no second `toml::from_str` fallback in bootstrap or commands.
- Bootstrap projects only fields that the invocation actually consumes, reusing `WorktreeConfig` and existing field/domain types. An env-selected root makes the declared root unnecessary for selecting code. A governed linked-data mutation still requires the original declared write policy, including when an explicit policy override exists, so the decision and audit are based on valid original authority. Unrelated fields such as project name or lease enforcement are not validated merely to select a code directory. They remain fully validated by their owning consumers and by config diagnostics.
- Typed projection should select the relevant authored fields from the one parsed document before deserializing them through the canonical types; it must not deserialize every unrelated worktree field and reproduce the same overvalidation one level down. No duplicate policy token parser, root vocabulary, silent default on failure or config-postimage policy overlay is permitted. An absent field is established by a successful parse or an admitted absent source, never by treating an error as absence.
- Storage retains one no-follow declaration/commit/root observation and its original bytes through consumer admission. Parsing/projection results do not become a second observation protocol. Later captures refresh that observation as they already do. The rework1 missing-ancestor observation, selected-data-path distinction and publication guard remain intact.
- Normal mutations still recover old committed redo first under D26/D27; only then do the original recovered declarations and explicit inputs select roots and authorize new work. `config set` never takes authority from its proposed repaired config. Its existing edited-document validation, closed capture, derived-state production and redo publication stay unchanged.

### Data-only configuration diagnostics

- Dispatch identifies `config validate` and captures invocation inputs once. It uses the same selected-data discovery (`JIT_DATA_DIR`/upward discovery) as ordinary bootstrap. It does not construct a fallback code layout or executor.
- Preserve initialized-store checks before returning a diagnostic success/finding: missing selected store, missing index and unsupported/malformed index remain their existing repository/storage failures. Factor the existing data-root/index validation into the shared data-only storage owner and use it from both ordinary bootstrap and diagnostics. The index and all declaration bytes used by a diagnostic must belong to the **same admitted diagnostic capture**; two independently admitted successful reads do not establish their coherence. Do not create a second parser, directory walk or issue/history scan.
- Capture declarations through the same `DataReadSession`/admission owner. A pending record or crossed commit returns the existing bounded retryable conflict; diagnostics never lock writers or replay. Filesystem, capability, initialized-store, index-format and admission failures retain their typed storage/input error paths; none is relabeled as an authored-configuration finding. Only canonical authored syntax/type/semantic assembly errors enter the findings report.
- Run full canonical declaration assembly as the diagnostic operation's computation. Canonical authored syntax/type/semantic errors become the existing report; valid declarations produce the existing success report. Keep one report-building function in the config command owner, used both by its existing API and the CLI's data-only route. Keep one existing dispatch outcome builder and emitter, and retain the **registered `config_validate` leaf findings admission** (`findings ValidationFailed`) before emitting a failure. Calling the common emitter alone must not bypass that registered-leaf check. Do not copy a JSON envelope, exit mapping or plain formatter. Replace the current raw `config.toml` existence guard and broad load-error conversion: absent config still assembles and validates sibling `templates.toml`/`invariants.toml` sources, and typed storage failures remain errors.
- Preserve explicit root-setting error boundaries by reusing the shared no-follow root-value validation when a root is expressly supplied. A valid env root overrides the declared root for that value check; it does not make invalid repository declarations valid. A configured path that is missing, non-directory, unsafe or overlapping still yields the existing typed setting/path error. No default code root is needed for diagnostics when neither setting supplies one. Syntax/type failure that prevents extracting a declared path is an authored declaration finding, not a reason to invent a root.
- Preserve the existing early error for an invalid `JIT_WORKTREE_WRITE_POLICY` invocation token; do not silently reinterpret it or use a diagnostic exception to authorize mutation. The diagnostic computation receives captured invocation inputs and does not reread environment during report construction. Any proposal to change that separate established input boundary would require explicit acceptance rather than being folded into this fix.

A narrow dispatch bootstrap result can carry either a fully opened repository or the completed config-validation report. `main.rs` already passes the inferred bootstrap result into dispatch, so this need not change main or create a new command/schema leaf. The report variant is valid only for this diagnostic command and owns no mutation guards.

## Required behavior matrix

| Input/state | Ordinary operation / config repair | `config validate` data-only route |
|---|---|---|
| Malformed TOML, no env root | Required declared-root projection unavailable: fail before new work. `config set` does not gain a new syntax-repair ability; its preservation parser already requires valid TOML. | Admitted canonical parse error becomes the unchanged findings report/exit4. No default code root or code Git probe. |
| Valid TOML, invalid unrelated project name; root/policy projection valid | Bootstrap can select/authorize from required fields. Existing `config set project.name good-name` repair works; edits whose proposed full document remains invalid still fail without writing. Other consumers retain their own full-config validation. | Full assembly reports the invalid project name through the unchanged report. |
| Invalid required declared root type or value | No substitution/default. Required projection/type failure or typed path error fails before new work. | Type/semantic declaration failures are canonical findings. A typed root string naming a missing/non-directory/unsafe/overlapping path keeps the existing setting/path error boundary. |
| Invalid declared linked write policy | A governed mutation fails closed, including with an override; invalid authority is not treated as default/refuse/allow. Reads need not validate policy solely to select code. | Full assembly reports the invalid declaration; no policy is used to authorize work. |
| Valid env root and invalid declared root | Only the effective env root is needed for code selection. A mutating consumer must still validate everything required for its own work and its original linked policy. | Validate the explicit env path; full declaration assembly still reports invalid declared types/semantics rather than hiding them. An overridden string path's existence is irrelevant. |
| Malformed config plus valid env root | Read/non-governed startup can use supplied root without pretending the config parsed, if no other bootstrap field is required. Governed mutation still cannot derive an unknown declared policy. Consumers remain responsible for required full declarations. | Valid explicit path check plus canonical malformed-config finding/exit4; no executor/code authority is constructed. |
| Malformed config plus missing/invalid env-selected path | Explicit root path fails with the existing typed source/path error; do not use declared/default root. | After successful data admission, explicit input path error remains typed and precedes rendering repository declaration findings. This preserves root-selection input safety without needing to parse an unknown declared root. |
| Env root absent | Use declared root when needed/valid, else the unchanged normal-operation derivation only when admitted configuration establishes absence. | Validate a declared root if a path can be extracted; otherwise diagnose the declaration. With no authored root, perform no default code-root derivation. |
| Config absent, initialized data store, sibling declaration sources valid | Existing defaults/no-setting behavior; each consumer still validates the declarations it requires. | Existing valid-configuration report only after full assembly also validates templates/invariants; no filesystem publication or code layout. |
| Config absent, invalid sibling template/invariant declaration | Ordinary consumers retain their declaration requirements; absence of config is not permission to ignore siblings. | Canonical authored sibling errors become findings. Sibling read/capability/admission errors remain typed failures, not findings. |
| Data store/index absent or invalid | Existing repository/format failure. | Same repository/format failure, not a fabricated valid empty repository. |
| Pending redo, including malformed config postimage | Mutations/recover complete original committed work first, then project coherent recovered declarations and apply policy to new work. | Bounded retryable conflict with no locks/replay/findings from unadmitted bytes. Pending state takes precedence over root-path inspection and repository-config findings. Existing invalid invocation-token validation remains the outer input boundary. |

## Alternative: conditional diagnostic fallback

The smaller-looking alternative retains normal config-validation bootstrap when root projection works and emits a data-only report only for a typed canonical declaration/projection failure. It can preserve today's successful-path root checks and restore the immediate malformed-config exit/report test.

Its disadvantages are structural: validation has no intrinsic need for a code executor; parseable and unparseable inputs traverse different initialized-store, root-selection and presentation routes; each route must compute exactly the same report from admitted inputs; and fixing semantic repair still requires the shared bootstrap projection. It saves little after shared discovery/initialization/report computation are correctly factored. Recommendation: do not keep a full code bootstrap merely because the old handler context requires one. Prefer the always-data-only route with the explicit value/error boundaries above.

Rejected approaches: return default config after assembly failure; pick a dummy code root; catch arbitrary bootstrap errors and force exit4; deserialize a second private subset directly from bytes; use proposed config to authorize its own repair; or change projection/response assertions. Each either loses authority/admission, duplicates the parser, or fails the existing repair/report contract.

## Exact proposed edit footprint

Ten files, no new file or Cargo target. The R1 correction adds an existing maintained storage test owner; the number of owners newly outside the maintained footprint remains three:

| File | Status relative to maintained accepted footprint | Bounded change |
|---|---|---|
| `crates/jit/src/declarations/mod.rs` | Already named by accepted design step3 as shared assembly owner; original worker brief required escalation before changing this owner. It is **not newly omitted from the maintained design**, but the new parsed-document/projection responsibility needs material acceptance. | One canonical syntax parse and full/projection views; owning pure tests. |
| `crates/jit/src/config.rs` | Accepted | Canonical typed worktree projection/full-view integration; reuse field types, no new policy parser. |
| `crates/jit/src/storage/repository_state_store.rs` | Accepted | Shared admitted declaration observation supports projections/full diagnostic results without eager unrelated validation. Keep D25 and rework1. |
| `crates/jit/src/storage/repository_state_store_tests.rs` | Already in the maintained accepted storage footprint; omitted from the first correction list and restored for R1 F1. | Mechanically migrate the three eager `config.worktree` accesses and observation-binding fixture to the canonical projection/full-view API. Preserve all absence/publication/replacement/admission semantics; retain this owner for any necessary index-plus-declarations composition test. |
| `crates/jit/src/storage/json.rs` | Accepted | Factor/reuse data-only initialized-store/index validation; no history scan or parallel absence protocol. |
| `crates/jit/src/commands/bootstrap.rs` | Accepted | Shared selected-data discovery and explicit root-value validation; only required bootstrap projection, original policy/recovery ordering. |
| `crates/jit/src/dispatch/mod.rs` | Accepted | Explicit repository-versus-config-diagnostic bootstrap result and routing, captured inputs. |
| `crates/jit/src/commands/config.rs` | **Additional owner requiring approval** | One data-only validation computation/report builder; existing executor API delegates; existing repair algorithm unchanged. |
| `crates/jit/src/dispatch/config.rs` | **Additional owner requiring approval** | One existing validation outcome/emitter path usable with the data-only report. |
| `crates/jit/tests/cli_repo_workflow/project_config_tests.rs` | **Additional test owner requiring approval** | Preserve all existing assertions; add only diagnostic input/authority/pending-record CLI wiring cases not owned at storage. |

No edits proposed to `main.rs`, `cli.rs`, `commands/mod.rs`, `storage/mod.rs`, generated exit-code/schema facts, existing projection tests, lease-enforcement tests, Git-floor/Docker work or unrelated feature owners. Existing canonical docs describe the public settings; no new setting is introduced. Lead owns any maintained-design amendment and acceptance record.

## Independent diagnostics design review R1 and resolution for R2

R1 verdict remains recorded as **FAIL — design review only**, in `/tmp/f4d9502a-lead/5046408a-diagnostics-design-review-r1.md`. Its semantic assessment was sound; the blocking finding was: **“The exact nine-file footprint excludes the storage tests that consume the replaced eager configuration representation.”** This revision addresses that proposal defect; it does not claim R2 approval or authorize implementation.

| R1 item | Revision evidence |
|---|---|
| F1: omitted `storage/repository_state_store_tests.rs` | Exact footprint now names ten files, including this existing owner. Owning validation explicitly migrates current direct accesses at 3316/3333/3354 and binding fixture at 3193–3202; all absence/publication/admission assertions stay in place. No eager/default/test-only compatibility field. Still only three owners are new to the maintained footprint. |
| Index/declaration composition clarification | Initialized-store and declarations must come from the same admitted diagnostic capture, not independently successful reads. Existing storage suite owns any missing composition property. |
| Registered findings admission clarification | The data-only route retains the registered leaf's `findings ValidationFailed` admission as well as the common outcome builder/emitter. No new leaf or copied output. |
| Absent-config advisory clarification | Valid success requires valid sibling declaration sources; full assembly always sees templates/invariants even when config is absent. Raw existence gating and catch-all load-error findings are replaced at the canonical config operation. |
| Error/input clarification | Typed storage/capability/admission/index failures remain errors. Only canonical authored assembly failures are findings. Invocation inputs are captured once and not reread during report construction. |

Required next step: independent bounded R2 review of this revision, followed by steward acceptance and the implementation/rework-limit ruling. Source and tracker stay frozen until those decisions and a new implementation dispatch.

## Owning validation and acceptance sequence

1. Steward accepts the semantics/matrix and exact footprint; independent bounded design reviewer checks canonical parsing, data-only diagnostic boundary, required policy/root projection and report preservation. Resolve findings before implementation.
2. Preserve the actual RED above. Add pure projection/diagnostic-owner tests before fixes; retain every existing config report/repair assertion. Storage owns pending/no-follow/admission races once, in the existing `repository_state_store_tests.rs` module. Mechanically migrate its observation-binding fixture (current lines 3193–3202) and `config.worktree` accesses (current lines 3316, 3333, 3354) to the canonical API. Where a test asserts full defaults, use the canonical full view; where it asserts selected-root evidence, use the required-root projection. Preserve every absence/no-write/publication/replacement/admission assertion and the current test ownership. Do not retain an eager compatibility field, manufacture a partial/default `JitConfig`, add a test-only representation, or move/copy these tests elsewhere. Add index-plus-declarations composition coverage here only if the existing tests cannot already observe it. CLI tests prove routing, registered findings admission, input/error precedence and plain/JSON output without repeating the storage race matrix.
3. Run failing projection case, project-config and other relevant config CLI boundaries; run all CLI **and contracts** tests without fail-fast to expose hidden siblings, as the lead directs. Run affected library owners, fmt and all-target Clippy. Do not repeat unrelated library/doctest suites solely for freshness.
4. Freeze and return all failures/reruns, cumulative resolution table and source hash; lead reruns full configured Cargo and subsequent reviews.

Until steps1's acceptance/review are complete, the current regressions remain known and the source stays frozen. No implementation is authorized by this proposal.
