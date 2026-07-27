# Investigation: embedded `jit-dogfood` profile MVP

**Container:** `9b7b5f9c` — Add first-class reusable jit configuration profiles

**Scope authority:** the live issue description and
`dev/active/9b7b5f9c-mvp-scope-brief.md`. The older
`dev/active/9b7b5f9c-jit-profiles-planning-brief.md` is prior art only. In particular,
multiple installed profiles, local packages, dependencies, incompatibilities, variables,
sensitive inputs, shared ownership, reconfiguration, diff, upgrade, and removal are
post-1.0 work and must not be restored to this MVP
(`dev/active/9b7b5f9c-mvp-scope-brief.md:28-30`;
`dev/vision/9db27a3a-charter.md:136-149`).

This is an investigation report, not an implementation plan.

## Executive findings

- There is no product profile surface today. `jit init` accepts only a hierarchy
  template and JSON output, and the top-level command enum has no `profile` family
  (`crates/jit/src/cli.rs:39-49`). Every MVP requirement is therefore
  **valid-and-open** as an end-to-end requirement, although several safe storage,
  projection, schema, and template primitives are already available.
- The current init path is already neutral and git-optional. It initializes `.jit`,
  seeds the selected hierarchy plus default rules, and returns successfully when the
  directory is not a git repository (`crates/jit/src/main.rs:1703-1723`,
  `crates/jit/src/main.rs:1754-1800`, `crates/jit/src/commands/mod.rs:942-968`).
  Profiles must remain an explicit opt-in layered on that behavior.
- The repository has a real repository-wide write lock and strong single-artifact
  primitives, but no general all-or-nothing multi-file repository publisher.
  `atomic_write` only makes one replacement atomic
  (`crates/jit/src/storage/atomic_write.rs:1-42`), while archive publication can leave
  durable partial mutations for reconciliation after a late failure
  (`crates/jit/src/commands/archive.rs:384-440`,
  `crates/jit/src/commands/archive.rs:458-518`). REQ-04 therefore needs a deliberately
  designed transaction/rollback protocol; it cannot be justified by naming either
  primitive.
- The current planning gate assets are not portable profile assets. Built-in planning
  presets name source-checkout scripts (`crates/jit/src/gate_presets/planning.rs:48-172`),
  and `scripts/ai-review.sh` requires `jq` and fails when no reviewer is configured
  (`scripts/ai-review.sh:15-18`, `scripts/ai-review.sh:49-64`). The profile must embed
  portable assets and provide the required structured warning placeholder.
- The implementation must be a generic package engine plus one embedded data package.
  Hardcoding `jit-dogfood` taxonomy, gate names, workflow, or document layout in domain
  logic would violate the repository's domain-agnostic invariant
  (`AGENTS.md:168-178`).

## Charter and invariant resolution

The cited project knowledge was resolved before investigation:

- `@/charter/D-1`: repository-local, git-versioned JSON is the persistence model
  (`dev/vision/9db27a3a-charter.md:38-49`). A profile applies repository files; it does
  not introduce an external package database.
- `@/charter/D-2`: gate definitions are configuration in `.jit/gates.toml`, not engine
  policy (`dev/vision/9db27a3a-charter.md:51-60`). Embedded profile content may contain
  gate data, but the apply engine must remain generic.
- `@/charter/D-3`: planning before fan-out is the endorsed orchestration shape
  (`dev/vision/9db27a3a-charter.md:62-72`). The dogfood package can install that shape;
  the engine cannot assume it.
- `@/charter/D-4`: git is optional except for explicitly git-dependent operations
  (`dev/vision/9db27a3a-charter.md:74-85`; `AGENTS.md:160-162`). Profile inspection,
  dry-run, and application must not pass through claim/worktree/hook machinery.
- `@/charter/D-6`: each item kind has a declared source of truth and prose must be
  projected or cite it (`dev/vision/9db27a3a-charter.md:100-115`). The immutable
  profile manifest/package must own installed package facts; compatibility surfaces
  must derive from or cite it.
- `@/charter/D-8`: this exact single embedded offline MVP is the pre-1.0 boundary;
  lifecycle and package breadth are deferred (`dev/vision/9db27a3a-charter.md:136-149`).
- `@/inv/event-log`: state changes append a typed event (`AGENTS.md:168-174`). Applying
  a profile changes repository state, so the installed record and event are part of the
  consistency design, not optional telemetry.

## Claim classification

### REQ-01 — neutral init plus explicit profile commands

**Classification: valid-and-open.**

Neutral init is already done: the CLI's `Init` variant has `hierarchy_template` and
`json` only (`crates/jit/src/cli.rs:39-49`), main dispatch seeds the chosen hierarchy and
default rules (`crates/jit/src/main.rs:1754-1800`), and storage initialization creates
the minimal index, issues directory, empty gate registry, and event log
(`crates/jit/src/storage/json.rs:595-636`). The quickstart documents plain `jit init`
(`docs/tutorials/quickstart.md:76-87`).

The open work is `jit init --profile jit-dogfood` and `jit profile apply jit-dogfood`.
No `Profile` command variant or profile flag exists (`crates/jit/src/cli.rs:39-49`). The
new init flag must be orchestration sugar over the same profile application path, after
neutral initialization, rather than a second installer.

### REQ-02 — offline, git-free, side-effect-limited application

**Classification: valid-and-open.**

The premise is compatible with current init: `CommandExecutor::init` checks for git only
to initialize worktree identity and explicitly succeeds outside git
(`crates/jit/src/commands/mod.rs:942-968`). The open profile path does not exist. It must
not reuse hook installation, which embeds scripts but writes `.git/hooks` and requires a
git repository (`crates/jit/src/commands/hooks.rs:7-8`,
`crates/jit/src/commands/hooks.rs:25-60`). No issue creation, commits, network lookup, or
agent execution is necessary to materialize an embedded package.

### REQ-03 — versioned immutable manifest and generated schema

**Classification: valid-and-open.**

No profile manifest type, package registry, embedded package directory, or generated
profile-manifest schema exists. Existing built-in hierarchy templates and gate presets
are Rust values (`crates/jit/src/hierarchy_templates.rs:18-48`,
`crates/jit/src/gate_presets/builtin.rs:11-17`), not one versioned package manifest.
Generic graph templates already have configuration data types and a loader
(`crates/jit/src/templates.rs:277-348`, `crates/jit/src/templates.rs:376-429`).

The manifest may declare identity, compatibility, registries, assets, executable bits,
and projection regions, but must not declare hooks or runtime workflow assumptions, as
the MVP brief specifies (`dev/active/9b7b5f9c-mvp-scope-brief.md:17`). A generated JSON
schema should come from the Rust manifest type, following the existing schemars-based
schema pattern (`crates/jit/src/schema.rs:160-202`), not be hand-maintained.

### REQ-04 — validated-first, locked, failure-atomic, conflict-safe apply

**Classification: valid-and-open.**

Confirmed reusable pieces:

- `RepoWriteLock` is the repository-wide outer write lock at `.jit/.repo-write.lock`
  (`crates/jit/src/storage/repo_lock.rs:1-30`,
  `crates/jit/src/storage/repo_lock.rs:57-88`). The `IssueStore` contract requires it
  for mutations (`crates/jit/src/storage/mod.rs:76-106`).
- Template apply demonstrates a useful ordering: compute a pure delta, validate it,
  acquire one repository lock, apply under the guard, and compensate issue writes on
  failure (`crates/jit/src/commands/template.rs:1-22`,
  `crates/jit/src/commands/template.rs:208-225`).
- Artifact publication can stage, verify type/identity, reject symlink traversal, and
  publish a new target with atomic no-replace semantics
  (`crates/jit/src/storage/artifact_mutation.rs:77-133`,
  `crates/jit/src/storage/artifact_mutation.rs:135-217`,
  `crates/jit/src/storage/artifact_mutation.rs:219-297`).
- Region splicing is a pure operation that preserves bytes outside managed markers and
  errors on malformed/missing markers (`crates/jit/src/validation/projection.rs:176-225`).

Contradicted primitives/claims:

- `atomic_write` is atomic for one target and intentionally replaces an existing file;
  it is not a multi-target transaction and does not give no-overwrite conflict semantics
  (`crates/jit/src/storage/atomic_write.rs:1-42`).
- Schema/rule scaffolding writes several schema artifacts and the rule registry one by
  one; a middle failure has no set-wide rollback
  (`crates/jit/src/storage/ruleset_store.rs:28-50`).
- Gate/config stores are single-file atomic stores only
  (`crates/jit/src/storage/config_store.rs:43-77`,
  `crates/jit/src/storage/gate_store.rs:75-126`).
- Archive stages and preflights a set, but sequential publication records partial
  durable mutations for reconciliation when a later publish fails. It explicitly is
  not the required “every target unchanged” transaction
  (`crates/jit/src/commands/archive.rs:384-440`,
  `crates/jit/src/commands/archive.rs:458-518`).
- Some control-plane writes use narrower named locks, such as `rules.lock`, rather than
  the global repository lock (`crates/jit/src/commands/mod.rs:1014-1018`). Profile apply
  must explicitly take `RepoWriteLock` and keep all precondition rechecks and publication
  under it.

Therefore a final-state plan and a generic multi-file publication/rollback mechanism are
open work. Preflight must classify every target as absent, byte-identical, compatible
managed-region merge, or conflict; it must validate the complete resulting repository
before mutation; and it must preserve identical files byte-for-byte. Idempotence follows
only if a second plan has no writes and does not append a second “applied” event.

There is one additional load-bearing consistency issue: event append is a separate
fallible append under the repository/event locks (`crates/jit/src/storage/json.rs:828-858`).
The design must specify how profile record publication and the audit event remain
consistent if event append fails; simply appending after successful file publication
would violate a literal all-target-unchanged-on-command-failure contract.

### REQ-05 — installed record and typed audit event

**Classification: valid-and-open.**

There is no `.jit/profiles/` record. Events are a typed enum represented in the event
catalog, not arbitrary tags: `EventTag::ALL`, string mappings, and scope mappings are
closed enumerations (`crates/jit/src/domain/event_catalog.rs:65-193`), while the event
enum is schemars-tagged (`crates/jit/src/domain/types.rs:1060-1068`). The CLI schema
projects the event catalog (`crates/jit/src/schema.rs:194-202`). A repository-scoped
profile-applied event therefore requires a real enum/catalog addition, schema/sample
coverage, and reference projection; a log string alone would not satisfy the current
event model.

### REQ-06 — reject unsafe paths and package declarations

**Classification: valid-and-open.**

Shared relative-path validation rejects empty paths, leading `/`, and `..` segments
(`crates/jit/src/storage/path_errors.rs:46-73`). Repository reads/writes also canonicalize
existing paths and reject escapes (`crates/jit/src/storage/json.rs:1029-1080`,
`crates/jit/src/storage/json.rs:1115-1189`). The artifact mutation layer is stronger for
publication: it rejects any symlink component and verifies staged identities before
no-replace publication (`crates/jit/src/storage/artifact_mutation.rs:77-217`).

Those are reusable but do not finish REQ-06. There is no manifest validation for
executable declarations, interpolation tokens, duplicate/overlapping targets, invalid
projection ownership, or unsupported repository format. The shared validator's textual
leading-slash check is also not by itself a complete cross-platform absolute-path test
(`crates/jit/src/storage/path_errors.rs:46-73`); package validation should use platform
`Path` semantics and test Windows-style inputs as data even when CI runs on Unix.

### REQ-07 — list/show/apply UX, JSON, dry-run, schema

**Classification: valid-and-open.**

No profile commands or output types exist. Clap command metadata is projected into the
command schema (`crates/jit/src/schema.rs:160-202`), but success schemas are manually
matched by command path (`crates/jit/src/schema.rs:456-554`). New commands therefore
need both CLI definitions and explicit typed success-schema coverage. Repository
convention requires JSON for every command and count envelopes for lists
(`AGENTS.md:154-162`). `jit --schema` integration coverage already checks full command
exposure (`crates/jit/tests/integration_schema.rs:41-68`).

Dry-run must return the same deterministic final-state plan execution consumes, including
per-target action/reason/conflict and no writes. `profile show` should expose package and
compatibility facts; `profile list` should be a count-wrapped collection.

### REQ-08 — dogfood taxonomy, planning template, standards, epic coverage

**Classification: valid-and-open.**

The repository-local source data exists in `.jit/config.toml` and
`.jit/templates.toml`; the `plan` graph template is already declarative and anchored to
configured roles/gates (`.jit/templates.toml:1-28`). The graph template engine is generic
and loads `.jit/templates.toml` (`crates/jit/src/templates.rs:277-348`,
`crates/jit/src/templates.rs:376-429`). That is prior art, not an embedded adopter
package.

The package must select only reusable dogfood methodology. This repository adds local
`charter`/`definition` kinds and development-document lifecycle on top of what `jit init`
ships (`AGENTS.md:91-105`); copying the current `.jit/` tree wholesale would violate the
adopter-vs-dogfood boundary. Epic-level planning coverage must be installed as package
configuration/content, not hardcoded as an engine condition.

### REQ-09 — portable gates, built-in checks, warning placeholder

**Classification: valid-and-open.**

Current gate checkers support only an external `Exec` checker
(`crates/jit/src/domain/types.rs:939-963`). Built-in presets include planning definitions
that invoke `./scripts/ai-review.sh` and `./scripts/coverage-preview.sh`
(`crates/jit/src/gate_presets/planning.rs:48-172`; `docs/reference/gate-presets.md:31-56`).
The AI review script requires source-checkout assets, `jq`, and a configured reviewer;
without one it exits as an error (`scripts/ai-review.sh:15-18`,
`scripts/ai-review.sh:49-64`). It has no warning-pass placeholder.

Thus neither “portable” nor “usable immediately” is already met. Deterministic checks
should become generic built-in checker operations where possible; prompts/scripts that
remain assets must be embedded by the package and not depend on the jit source checkout.
The placeholder must emit a passing structured findings block with an explicit warning,
using the existing structured findings model rather than silently claiming review.

### REQ-10 — skills, standards, AGENTS region, empty invariant registry/projections

**Classification: valid-and-open.**

The source repository contains JIT skills under `.agents/skills/`, content standards at
`docs/reference/jit-content-standards.md`, and local invariant/rules projection regions
in `.jit/config.toml` (`.jit/config.toml:205-225`). Region splicing is already generic
(`crates/jit/src/validation/projection.rs:176-289`), as is the rules/gates projection
path (`crates/jit/src/validation/rules_gates_projection.rs:210-258`). There is no profile
that installs canonical adopter paths, rewrites skill references, creates an empty
adopter-owned invariant registry, or preserves non-managed AGENTS prose.

Installed skills must refer to their installed canonical documentation locations rather
than this source checkout. AGENTS changes must be managed-region splices only. The empty
invariant registry must remain adopter-owned data; engine defaults must not acquire this
repository's invariant vocabulary.

### REQ-11 — documentation path: preferred quickstart, reference, advanced manual

**Classification: valid-and-open.**

The current quickstart still presents plain `jit init` and manual next steps
(`docs/tutorials/quickstart.md:76-87`). Gate preset documentation says init writes an
empty registry and describes source-relative planning scripts
(`docs/reference/gate-presets.md:1-29`, `docs/reference/gate-presets.md:31-56`). No
profile command reference exists. Documentation must make the embedded profile the
preferred dogfood onboarding path while retaining manual configuration as advanced,
and should cite/project authoritative package facts rather than repeat file inventories.

### REQ-12 — unit, integration, failure-injection, cross-platform tests

**Classification: valid-and-open.**

No profile tests exist. Relevant test patterns do: template apply has failure and
concurrent-writer coverage, including a no-git storage-root-lock case
(`crates/jit/tests/template_apply_atomicity_tests.rs:700-712`), and artifact mutation
tests cover symlink, conflict, and identity behavior
(`crates/jit/tests/artifact_mutation_storage_tests.rs:14-41`,
`crates/jit/tests/artifact_mutation_storage_tests.rs:63-198`). The repository mandates
unit, harness, and CLI integration layers plus boundary/failure/concurrency cases
(`AGENTS.md:122-132`).

The open test matrix must cover manifest/schema drift, human/JSON output, init/apply
equivalence, git-free/offline operation, dry-run equivalence, byte-identical idempotence,
conflicts, malformed/unsupported packages, traversal/absolute/symlink races, executable
mode, managed-region preservation, injected failure at every publish/event step, and
cross-platform path and mode behavior.

## Prior-art sweep

- `dev/active/9b7b5f9c-jit-profiles-planning-brief.md` contains useful immutable-package,
  deterministic-plan, semantic-merge, and ownership ideas, but its lifecycle and package
  breadth are expressly superseded by the MVP scope brief
  (`dev/active/9b7b5f9c-mvp-scope-brief.md:28-30`). It must not be treated as current
  definition of done.
- The validation-engine study says “Profiles are not a concept” and uses `profile:sdd`
  merely as a label example (`dev/archive/features/f6a704d0-validation-engine.md:58-59`).
  The new repository-package noun must not be confused with validation labels.
- The completed template-apply work documents prior atomicity defects around a custom
  lock and timestamp-changing rollback (`dev/archive/features/dbe1e821/dbe1e821-completion-report.md:28`,
  `dev/archive/features/dbe1e821/dbe1e821-completion-report.md:67`). Current template
  apply now uses the repository lock and exact issue rollback; that history is a warning
  against introducing a profile-only lock or lossy reconstruction.
- `dev/active/57269494-apply-plan-doc.md` records the planning template and coverage
  coupling that the package must carry portably, while the current source is
  `.jit/templates.toml:1-28` and planning gate preset data
  (`crates/jit/src/gate_presets/planning.rs:48-172`).
- `dev/active/2821e177-investigation.md` and
  `dev/active/76cb968b-ssot-adoption-sweep.md` are prior art for config-driven
  rules/gates projection and single-source prose. The applicable invariant is explicit
  in `AGENTS.md:177-178`.
- `dev/active/d24008f0-req01-evidence.md:136-140` records the distinction between
  adopter-facing shipped defaults and this repository's dogfood layer. The same boundary
  is summarized in `AGENTS.md:91-105` and is essential when choosing profile contents.

Historical completion/session material is evidence, not a live consumer to rewrite.

## Consumer sweep

The change adds commands and migrates the authoritative source of reusable dogfood
configuration. The following is the whole-tree direct-consumer inventory. `.jit/issues`,
`.jit/events.jsonl`, and `.jit/gate-runs` contain historical state references; they must
not be rewritten as product consumers.

### Init and initialization contract

Product code:

- `crates/jit/src/cli.rs`
- `crates/jit/src/main.rs`
- `crates/jit/src/commands/mod.rs`
- `crates/jit/src/hierarchy_templates.rs`
- `crates/jit/src/schema.rs`
- `crates/jit/src/storage/json.rs`
- `crates/jit/src/storage/config_store.rs`
- `crates/jit/src/storage/ruleset_store.rs`
- `crates/jit/src/storage/gate_store.rs`
- `crates/jit/src/storage/gitattributes.rs`
- `crates/jit/src/storage/discovery.rs`
- `crates/jit/src/storage/errors.rs`
- `crates/jit/src/validation/defaults.rs`
- `crates/jit/src/validation/serialize.rs`

Direct contract tests and fixtures:

- `crates/jit/tests/init_tests.rs`
- `crates/jit/tests/init_item_kinds_golden.rs`
- `crates/jit/tests/project_config_tests.rs`
- `crates/jit/tests/default_rules_registry_derivation_tests.rs`
- `crates/jit/tests/type_hierarchy_schema_regen_tests.rs`
- `crates/jit/tests/format_compat_cli_tests.rs`
- `crates/jit/tests/worktree_init_tests.rs`
- `crates/jit/tests/cross_worktree_tests.rs`
- `crates/jit/tests/integration_test.rs`
- `crates/jit/tests/integration_schema.rs`
- `crates/jit/tests/config_get_tests.rs`

Live docs, examples, scripts, and packaging:

- `AGENTS.md`, `README.md`, `INSTALL.md`, `CHANGELOG.md`
- `docs/tutorials/quickstart.md`
- `docs/tutorials/parallel-work-worktrees.md`
- `docs/concepts/core-model.md`
- `docs/concepts/guarantees.md`
- `docs/how-to/adopt-planning-bracket.md`
- `docs/how-to/custom-gates.md`
- `docs/how-to/multi-agent-coordination.md`
- `docs/how-to/software-development.md`
- `docs/reference/cli-commands.md`
- `docs/reference/configuration.md`
- `docs/reference/example-config.toml`
- `docs/reference/gate-presets.md`
- `docs/reference/labels.md`
- `docs/reference/storage-format.md`
- `mcp-server/index.js`
- `mcp-server/lib/schema-loader.js`
- `mcp-server/curated-tools.json`
- `mcp-server/test-unit.js`
- `mcp-server/test-integration.js`
- `scripts/agent-init-demo-project.sh`
- `scripts/test-concurrent-mcp.sh`
- `scripts/test-label-hierarchy-walkthrough.sh`
- `docker/entrypoint.sh`

The MCP server generates tools from live `jit --schema`
(`mcp-server/index.js:16-23`), and its curation test requires every generated command to
appear in include or exclude with a rationale (`mcp-server/test-unit.js:294-314`). Adding
profile commands therefore necessarily updates `mcp-server/curated-tools.json` and MCP
tests; it is not only a Rust CLI change.

### Planning template, gate preset, and projection consumers

Product code/config:

- `.jit/config.toml`, `.jit/templates.toml`, `.jit/gates.toml`, `.jit/invariants.toml`
- `crates/jit/src/cli.rs`
- `crates/jit/src/main.rs`
- `crates/jit/src/schema.rs`
- `crates/jit/src/templates.rs`
- `crates/jit/src/commands/breakdown.rs`
- `crates/jit/src/commands/gate.rs`
- `crates/jit/src/commands/template.rs`
- `crates/jit/src/commands/template_expand.rs`
- `crates/jit/src/commands/validate.rs`
- `crates/jit/src/domain/item.rs`
- `crates/jit/src/gate_presets/builtin.rs`
- `crates/jit/src/gate_presets/manager.rs`
- `crates/jit/src/gate_presets/planning.rs`
- `crates/jit/src/storage/memory.rs`
- `crates/jit/src/storage/mod.rs`
- `crates/jit/src/storage/errors.rs`
- `crates/jit/src/validation/defaults.rs`
- `crates/jit/src/validation/drift.rs`
- `crates/jit/src/validation/projection.rs`
- `crates/jit/src/validation/rules.rs`
- `crates/jit/src/validation/rules_gates_projection.rs`

Direct tests/fixtures/examples:

- `crates/jit/tests/apply_cli_tests.rs`
- `crates/jit/tests/bracket_breakdown_tests.rs`
- `crates/jit/tests/bracket_coverage_gate_run_test.rs`
- `crates/jit/tests/command_exit_code_projection_tests.rs`
- `crates/jit/tests/exit_code_tests.rs`
- `crates/jit/tests/fixtures/steering/valid.toml`
- `crates/jit/tests/fixtures/steering/invalid.toml`
- `crates/jit/tests/fixtures/steering/empty.toml`
- `crates/jit/tests/planning_preset_tests.rs`
- `crates/jit/tests/research_bracket_tests.rs`
- `crates/jit/tests/scope_validation_tests.rs`
- `crates/jit/tests/sdd_bracket_tests.rs`
- `crates/jit/tests/template_apply_tests.rs`
- `crates/jit/tests/template_binding_cli_tests.rs`
- `crates/jit/tests/template_binding_tests.rs`
- `crates/jit/tests/templates_loader_tests.rs`
- `docs/examples/sdd/config.toml`
- `docs/examples/sdd/rules.toml`
- `docs/examples/sdd/templates.toml`
- `docs/examples/research/config.toml`
- `docs/examples/research/rules.toml`
- `docs/examples/research/templates.toml`

Live docs/scripts/agent consumers:

- `docs/concepts/planning-bracket.md`
- `docs/how-to/adopt-planning-bracket.md`
- `docs/how-to/custom-gates.md`
- `docs/reference/cli-commands.md`
- `docs/reference/gate-presets.md`
- `docs/reference/rules-and-gates.md`
- `docs/reference/jit-content-standards.md`
- `scripts/ai-review.sh`
- `scripts/coverage-preview.sh`
- `.agents/skills/jit-planning-lead/SKILL.md` and its `references/`
- `.agents/skills/jit-breakdown/SKILL.md` and its `references/`
- `.agents/skills/jit-execution-lead/SKILL.md` and its `references/`
- `.agents/skills/jit-project-lead/SKILL.md` and its `references/`
- `.agents/skills/jit-manage/SKILL.md` and its `references/`
- `.agents/skills/jit-parallel/SKILL.md` and its `references/`
- `.agents/skills/jit-migrate/SKILL.md` and its `references/`

The package should not blindly embed all these files. They are the consumers that need
compatibility, derivation, documentation, or tests when the embedded package becomes the
authoritative reusable bundle. Active and archived development reports remain historical
records and must not be mechanically edited.

## Primitive verification matrix

| Asserted property | Verdict | Evidence and consequence |
|---|---|---|
| One repo-wide lock | **confirmed** | `RepoWriteLock` and its lock-order/reentrancy contract exist (`crates/jit/src/storage/repo_lock.rs:1-30`, `crates/jit/src/storage/repo_lock.rs:109-158`). Profile application must use it, not invent `profiles.lock`. |
| Validated-first template-style orchestration | **confirmed as a pattern** | Template apply computes/validates before one locked mutation (`crates/jit/src/commands/template.rs:1-22`, `crates/jit/src/commands/template.rs:208-225`). Its issue-specific rollback cannot publish arbitrary files. |
| Atomic single-file replacement | **confirmed** | Temp file plus rename (`crates/jit/src/storage/atomic_write.rs:1-42`). It overwrites and does not make a set atomic. |
| Atomic no-replace new artifact publication | **confirmed** | Verified staging plus hard-link publication (`crates/jit/src/storage/artifact_mutation.rs:135-297`). Reuse for new files where supported. |
| Safe managed-region calculation | **confirmed** | Pure splice preserves non-managed bytes and validates markers (`crates/jit/src/validation/projection.rs:176-225`). Publication remains separate. |
| Generic all-or-nothing multi-file transaction already exists | **contradicted** | Rule/schema and archive operations can partially progress (`crates/jit/src/storage/ruleset_store.rs:28-50`; `crates/jit/src/commands/archive.rs:458-518`). New transactional work is required. |
| Archive executor can be reused as that transaction | **contradicted** | It records failed-execution mutations for reconciliation after partial publication (`crates/jit/src/commands/archive.rs:458-518`). Its staging/preflight ideas are reusable, its atomicity is not. |
| Existing planning scripts are portable/offline | **contradicted** | They reference source-relative paths and require `jq`/reviewer environment (`crates/jit/src/gate_presets/planning.rs:48-172`; `scripts/ai-review.sh:15-18`, `scripts/ai-review.sh:49-64`). |
| Existing path defenses can be reused | **confirmed with gaps** | Canonical containment and no-symlink artifact mutation exist (`crates/jit/src/storage/json.rs:1029-1080`; `crates/jit/src/storage/artifact_mutation.rs:77-217`). Package-specific declarations and cross-platform absolute paths remain open. |
| Typed event/catalog/schema projection can be reused | **confirmed** | Event enum, catalog, and schema projection exist (`crates/jit/src/domain/types.rs:1060-1068`; `crates/jit/src/domain/event_catalog.rs:65-193`; `crates/jit/src/schema.rs:194-202`). A profile event is still a new variant. |
| Profile idempotence follows from atomic writes | **contradicted** | Atomic writes say nothing about semantic no-op plans or duplicate events. Idempotence must be defined by byte/action comparison and one installed-state transition. |

## Architecture fit and protected boundaries

The repository's boundary is explicit: domain logic is pure/I/O-free, storage owns all
persistence, commands orchestrate, and CLI/output own presentation
(`AGENTS.md:134-145`). The fitting decomposition is consequently:

- Pure domain/package logic: parse and validate a versioned manifest, resolve one
  embedded package, compute target contents and semantic managed-region merges, classify
  actions/conflicts, and validate a complete final-state model. This logic accepts data;
  it must not open files, acquire locks, or know source-checkout paths.
- Storage boundary: containment/no-follow checks, snapshot/stage, repository lock,
  no-replace/new-file publication, exact replacement/rollback, directory/fsync behavior,
  executable mode application, installed-record persistence, and event append.
- Command layer: resolve repository and package, produce the pure plan, render dry-run,
  acquire/recheck/apply through storage, and map typed errors.
- CLI/output/schema: `init --profile`, `profile list/show/apply`, human text, JSON envelopes,
  and generated input/output schema.

The engine must not contain literals for `jit-dogfood`, `milestone`, `epic`, `planning`,
specific gate keys, `.agents/skills`, or AGENTS projection content. The invariant says
type names, label vocabulary, gates, templates, and workflow shapes come from repository
configuration (`AGENTS.md:176-178`). Those values belong to embedded package data. A
generic built-in checker enum may describe operations such as running repository
validation, but it must not encode dogfood gate names or workflow policy.

Likewise, the package must not change shipped neutral defaults. `jit init` currently
creates empty gates and generic hierarchy/rules (`crates/jit/src/storage/json.rs:595-636`,
`crates/jit/src/main.rs:1754-1800`), and D-8 requires explicit adoption. Compatibility
with existing built-in planning presets may derive their definitions/assets from the
same package source, but removing/deprecating preset commands is lifecycle work outside
this MVP.

## Planning constraints handed to synthesis

1. Keep every lifecycle feature in the deferred list out of the MVP, even if the old
   brief contains an attractive design for it.
2. Treat the manifest/package as the single source for reusable dogfood facts. Avoid a
   second handwritten inventory in Rust presets, docs, tests, or generated schema.
3. Do not claim failure atomicity until late failures—including mode changes, replacements,
   installed-record write, and event append—have exact rollback/failure-injection proof.
4. Preserve neutral init by making `init --profile` invoke the same generic apply engine
   after successful minimal init; no hidden profile selection.
5. Use the repository-wide lock and recheck the complete plan under it. A profile-only
   lock would recreate a previously fixed concurrency defect.
6. Keep all dogfood concepts in package data. The generic engine can know package,
   manifest, target, managed region, checker operation, and transaction concepts only.
7. Include the MCP schema/curation surface, docs, examples, scripts, skills, and projection
   consumers in acceptance evidence; a Rust-only consumer sweep is incomplete.
8. Preserve existing preset behavior where it remains public, but do not reintroduce
   upgrade/removal/deprecation lifecycle scope under the guise of compatibility.
