# Changelog

All notable changes to Just-In-Time (JIT) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **The normal validation suites are callable from another workflow.** `ci.yml`
  accepts `workflow_call` beside its branch-push and pull-request triggers, so
  a branch build, a pull request, and any workflow of this repository that
  calls it run one maintained definition of the repository-validation, Rust,
  exact-MSRV, MCP, and web jobs on their own commit. The call takes no input
  and no secret, so a caller cannot aim the suites at a different commit.

- **CI validates the repository's own tracked data.** A `repo-validate` job in
  `ci.yml` runs `jit validate` with no issue id — every rule over the whole
  repository plus the repository-integrity checks — against a full-depth
  checkout, since integrity resolves document references pinned to a commit
  that a shallow checkout cannot look up. It is one of the jobs a caller of
  `ci.yml` inherits.
  `.github/workflow-contract.yml` gains a `callers.require_needs` declaration
  for that promise: a reusable workflow names the jobs a caller inherits, and
  the verifier then requires each of them to exist and to carry no `if:`
  condition, and requires every job a caller runs of its own to reach the call
  through `needs`. A caller job that calls another workflow of this repository
  is exempt, so sibling calls run beside each other. Case vectors under
  `test-vectors/workflow-contract/` cover an unrouted caller job, an absent
  and a conditional promised job, a caller obligation on a workflow nothing can
  call, and the conforming shape.

- **`jit-server` shuts down gracefully instead of dying mid-connection.** The
  server awaited `axum::serve` bare, with no signal handling at all, so a
  `SIGTERM` from `jit serve --stop` (or Ctrl+C under `jit serve --fg`) killed
  the process wherever it happened to be: in-flight responses were truncated
  and the exit was a signal death rather than an exit status. On a registered
  Ctrl+C or `SIGTERM` the server now cancels one process-wide
  `CancellationToken` — cloned into the application state and from there into
  every live SSE stream, so `/api/events/stream` subscribers reach EOF instead
  of being held open behind their 15-second keepalive — and then establishes
  its sole five-second drain boundary. It calls `graceful_shutdown(None)` on
  the `axum-server` handle that owns the listener and connections to stop
  acceptance and allow an indefinite cooperative drain, so the port is
  released at once and ordinary connections get the full boundary to finish.
  At that boundary JIT samples the connection count and calls `shutdown()` to
  force-close only survivors. The process exits `0`, and its log records the
  signal, the deadline, the connection count at the signal and at expiry, the
  forced-close path, and clean completion.

### Fixed

- **Archive destinations no longer repeat the configured development root.**
  Artifact paths beneath a document or container destination are now relative
  to `documentation.development_root`, so `dev/active/plan.md` archives as
  `<container>/active/plan.md` instead of `<container>/dev/active/plan.md`.
  Planning, execution, relinking, recovery, and reruns share that canonical
  derivation, and the repository's existing archive trees and live links have
  been migrated to it.

- **An archive preview's `moving-path-citation` warnings no longer fire on
  citations that already name the archived destination.** The scan matched a
  moving artifact's source path as a plain substring. Before the canonical
  archive-relative layout above, a destination repeated the entire source path
  after its root, so a citation corrected to that pre-cutover destination still
  contained the source path and still warned — with the column shifted by the
  length of the inserted destination root.
  Repointing a citation therefore never emptied the report, and the warning set
  could not be used as a work list. The scan now reports an occurrence only
  where the surrounding text names the moving path whole: text naming a longer
  path that ends with it warns for neither, whether it is the published
  destination, an unrelated root above it, a directory whose name ends with its
  first segment, or a longer file name past its end. A citation of the source
  path alone is still reported, at the column where that path begins.

- **`jit doc check-links` honours a document reference's commit pin.** It read
  the working tree for every reference, so a reference pinned to a commit whose
  file had since been deleted was reported `missing_document` — permanently, for
  every historical reference the archival workflow pins by design — while `jit
  validate` resolved the same reference at its pin and passed. Both commands now
  apply one rule (`crate::document::resolve_document_reference` over
  boundary-captured evidence): a pinned reference resolves at its commit and an
  unpinned one in the working tree with a `HEAD` fallback. A pinned reference is
  read at its commit throughout — file, assets, and internal links — so its
  neighbourhood resolves as it stood at the pin. A pin is a claim about Git
  history, so without Git a pinned reference is reported unresolved carrying the
  boundary's reason, the answer `jit validate` already gave; unpinned references
  still resolve from the working tree, keeping link checking usable without Git.

- **Graph-template application no longer leaves a node `ready` while a
  dependency it just wired blocks it.** `jit apply` added the edge, then read
  the dependent's dependency set back from the already-mutated map. When the new
  edge introduced no transitive redundancy the set compared equal to itself and
  a guard skipped the rest of the loop body — which held both the `ready →
  backlog` demotion and the `dependency-add` event. Applying the planning
  bracket therefore left the breakdown node and the container `ready` with unmet
  dependencies, and recorded neither event. The guard now exempts the dependent
  whose set grew, and both paths that mutate a dependency edge — `jit dep
  add`/`rm` and template application — derive readiness through the single
  domain helper `Issue::derive_readiness_correction`. `jit validate` reports any
  issue stored as `ready` while carrying unmet dependencies, and `jit validate
  --fix` demotes it to the state its dependencies imply.

### Removed

- **The legacy gate-verb aliases `pass`, `pass-all`, `check`, and `check-all`
  are gone.** Issue 949cd9d0 renamed these verbs to `evaluate`, `evaluate-all`,
  `status`, and `status-all` and kept the old spellings as silent aliases so no
  caller broke at rename time. This change drops the four aliases: invoking
  any of them now fails with clap's standard unrecognized-subcommand error,
  the same as any other unknown command. The short alias `eval` (for
  `evaluate`) is unaffected and continues to work. `jit gate --help` and the
  schema's `gate` subcommand listing now show only the canonical verbs
  (`evaluate`/`eval`, `evaluate-all`, `fail`, `status`, `status-all`, plus the
  configuration and inspection verbs), and in-repo docs and tests were swept
  to the canonical spellings.

### Changed

- **Build provenance no longer tracks Git metadata or the wall clock.**
  `crates/jit/build.rs` previously watched `.git/index`, `HEAD`, and refs and
  stamped the current time, so staging or committing unchanged Rust sources
  reran the build script and relinked every test target. It now reads only the
  four release-injection variables (`JIT_BUILD_GIT_HASH`,
  `JIT_BUILD_GIT_SHORT_HASH`, `JIT_BUILD_GIT_DIRTY`, `SOURCE_DATE_EPOCH`) and
  reports documented `unknown` fallbacks otherwise, so ordinary builds are
  reproducible and insensitive to Git-metadata-only changes. Releases and
  installs inject real provenance through the new `scripts/install-jit.sh`
  (commit, short commit, dirty flag, and commit-time `SOURCE_DATE_EPOCH`),
  which `jit version --json` reports exactly. The stale-binary guard composes
  unchanged: it judges the installed binary, which `scripts/install-jit.sh`
  stamps with the commit it was built from.

- **Debug info and incremental compilation are bounded by policy instead of
  Cargo's undocumented defaults.** Full debug sections dominated a
  representative test executable's size, and incremental state accumulated
  without bound across gate runs (baseline measured in
  `dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md`). The workspace manifest's
  `[profile.dev]` and `[profile.test]` now set `debug = "line-tables-only"`,
  keeping line-number backtraces without the full debugger payload, and both
  state `incremental = true` explicitly so ordinary interactive builds and
  test runs keep Cargo's incremental cache on purpose rather than by
  accident. `scripts/cargo-ci.sh` exports `CARGO_INCREMENTAL=0` for every
  step (fmt, clippy, test, provenance) and now runs a dedicated
  `incremental-state` step afterward that fails the gate if any non-empty
  `incremental` directory remains under the target directory the run used:
  a gate run compiles once and exits, so incremental state has no later
  rebuild to amortize its cost against.

- **Documentation projections are declared generically and rendered by one
  command.** A single `[projection.<name>]` config registry (fields `kind`,
  `mode`, `target`, `style`, optional `region-begin`/`region-end`) drives every
  projection, rendered by `jit project render [--name <name>]`. This replaces the
  bespoke `[invariant_projection]` and `[rules_gates_projection]` config tables
  and the separate `jit invariant render` / `jit reference render` commands
  (removed). Any addressable item kind projects its `- **{id}** — {text}` rows
  through the generic `id-anchor` style with no dedicated code; the built-in
  `full` style renders the rich invariant and rule+gate registry views.

- **Profile manifests declare projections instead of singleton tables.** The
  `singleton-table` contribution (with its `invariant-projection` /
  `rules-gates-projection` targets) is replaced by a `projection` contribution
  (`kind = "projection"`, `name = "<projection-name>"`, and a complete `value`
  carrying `kind`/`mode`/`target`/`style`) that merges into the `[projection.*]`
  registry, so a profile's projection is byte-equal to the config an adopter
  reads.

### Added

- **One static harness verifies every committed GitHub workflow.**
  `scripts/workflow-contract.sh` runs a pinned, checksum-verified `actionlint`
  and this repository's own structural verifier over the workflow tree, and the
  `workflow-contract` job in `ci.yml` runs both on every pull request. Each
  workflow declares what it guarantees in `.github/workflow-contract.yml` —
  required and forbidden triggers, `workflow_call` inputs and outputs,
  transitive `needs` edges, workflow- and job-level permissions, required jobs
  — instead of carrying a verifier of its own. Repository-wide rules reject any
  `continue-on-error`, any condition that survives a failed predecessor, any
  shell-level suppression of a non-zero exit, and any external `uses:` that is
  not an exact 40-character commit SHA with an upstream-version comment; the
  `uses:` scan follows local `./…` references into composite actions
  recursively. Twenty case vectors under `test-vectors/workflow-contract/` seed
  one defect class each, and `scripts/workflow-contract-selftest.sh` replays
  them plus an actionlint-only defect. `dev/workflow-contract.md` documents the
  declaration grammar and what a pin-update pull request has to establish
  before a maintainer merges it.

  Every external action in the CI, security, container, documentation, and
  release workflows moved to a resolved commit SHA in the same change, every
  workflow gained an explicit least-privilege token, and the `profile-adoption`
  test command was quoted — its `:: ` test-path filter made `ci.yml`
  unparseable to a strict YAML parser.

- **Automatic Rust build-footprint budget enforcement in the `cargo-ci` gate.**
  A committed checker, `scripts/rust-build-budget.sh`, derives the logical test
  topology and active-executable footprint from Cargo output rather than
  scanning stale `target/` artifacts: it counts integration-test targets from
  `cargo metadata` (fails above 12) and sums the unique active test-executable
  sizes from `cargo test --workspace --no-run --message-format=json` (fails
  above 2 GiB), and it asserts the debug-profile, gate-incremental, and
  dependency-feature (no remote JSON Schema resolution, one TLS backend)
  policies against the committed manifests and gate script. The budgets and
  their evidence are defined once in the checker and
  `dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md`. `scripts/cargo-ci.sh` runs it
  as a `budget` step after its test step, reusing warm Cargo artifacts (no
  second cold build), and folds a concise footprint summary into the persisted
  gate summary; over-budget or policy-drift runs fail with a diagnostic naming
  the observed value, the limit, and the corrective area. Each failure mode has
  an injectable-input regression fixture in
  `crates/jit/tests/scratch_build/rust_build_budget_checker_tests.rs` that runs
  without compilation.

- **Stale-binary detection on the gate path.** `jit gate evaluate` (and every
  path that runs an automated checker: `gate pass`, `gate pass-all`, and a
  state transition's pre/postchecks) refuses to run a checker when BOTH (1)
  the repository under review can resolve the running binary's build commit
  in its own history (the repository the binary was built from, or a clone or
  fork sharing that history), AND (2) that commit no longer matches the
  repository's current `HEAD`, or the binary was built from a dirty tree — a
  gate verdict from such a binary is not evidence about the tree under
  review. This is checked in two places: the evaluator itself refuses before
  spawning any checker (a typed, exit-code-10 error naming the build commit
  and the fix, `scripts/install-jit.sh`; no gate run is recorded for
  that refusal), and — since a checker script that itself shells out to `jit`
  (e.g. `scripts/jit-validate.sh`) resolves that `jit` from `PATH`
  independently of the evaluator — any such child process self-checks too, so
  a stale child makes the gate run FAIL with the refusal visible in the
  recorded run's stdout/stderr, rather than silently producing a misleading
  verdict. Silent otherwise (an unrelated repository, no git, or an
  unresolvable build commit), so an ordinary installed release validating a
  different repository is unaffected.

- **Help cross-references from mutation/inspection commands to the reporting
  commands that answer "what happened".** `jit issue show --help` now names
  `jit issue status` (compact one-line view), `jit gate status-all`/`jit gate
  status <id> <gate>` (per-issue gate readiness/history), and summarizes the
  JSON response's top-level fields (`dependencies`, `unmet_dependencies`,
  `gates`, `documents`, etc.) so a caller doesn't have to run `--json` and
  inspect the shape to discover them. `jit issue create`/`update` and `jit doc
  add`/`remove` `--help` now point at `jit events query --issue-id`/`jit
  events tail` for verifying a recorded change. Gate failure output (`jit gate
  evaluate`'s error message and JSON suggestions, and the gate-blocked
  transition error's remediation) now also names `jit gate status <id> <gate>
  --all`, the run-history view, alongside the existing single-run and
  readiness commands. Top-level `jit --help`/`-h` now names `jit --schema` for
  JSON response shapes and exit code documentation.

- **Canonical hierarchy resolution in the core, shared by the CLI and web UI.**
  Parent, children, cluster, and rank per node are now resolved once in the core
  library (`jit::graph::hierarchy`) treating the **dependency DAG as
  authoritative and membership labels as advisory** — the model previously lived
  only in the web UI's TypeScript, forcing external tools to re-port it. A
  container is any type below the configured leaf level; a node's parent is the
  nearest dominating container, its cluster is the strategic root, and its rank is
  the longest dependency-path depth. New surfaces:
  - **`jit graph tree [<root-id>] --json`** emits the resolved parent/children/
    cluster/rank per node (`{count, root, nodes}` envelope); a root id scopes the
    view to that node's dependency closure.
  - **`jit graph export --format json --full`** nodes gain the same four
    additive resolution fields as `graph tree` (`parent`, `children`, `cluster`,
    `rank`). The default summary shape is byte-for-byte unchanged.
  - **`GET /graph`** on the web server carries each node's resolved `parent`,
    `children`, `cluster`, `rank`, and `type` value, computed by the core
    resolver over the repository's configured type levels.
  - **`jit --schema`** publishes the `graph tree` response shape
    (`GraphTreeResponse`) alongside the other command output schemas.
  - **`jit query divergence [--json]`** reports membership labels the DAG does not
    back (an issue labeled `epic:foo` that the `foo` epic does not depend on).
    `jit validate` surfaces the same as an advisory `divergence_count` that never
    changes its exit status.

  The web UI reads those served fields; the core resolver is the sole
  implementation, pinned by the fixture `test-vectors/hierarchy_resolution.json`.
  See [Hierarchy Resolution](docs/concepts/hierarchy-resolution.md).

- **Full-record bulk graph export (`jit graph export --format json --full`) and
  issue lifecycle timestamps.** The JSON graph export gains a `--full` flag that
  emits the complete issue record for each node — every field of the on-disk
  `issues/<id>.json` file, including `assignee`, `labels`, `gates_status` (each
  gate's key/status/`updated_by`/`updated_at`), `dependencies`, `description`,
  `created_at`/`updated_at`, and the new lifecycle timestamps — alongside the
  same `edges` list. This lets a bulk consumer read every node's full record in
  one call instead of globbing the issue files and streaming the event log.
  Without `--full` the output is byte-identical to the previous summary shape
  (`id`, `short_id`, `title`, `state`, `priority`, `labels` + edges); `--full`
  applies only to `--format json` (combining it with `dot`/`mermaid` is a usage
  error, exit 2).

  The issue record now stores three lifecycle timestamps written **once**, at
  the transition: `first_ready_at` (first time the issue enters `ready`,
  including the dependency-free auto-promotion at creation), `claimed_at` (first
  claim/assignment), and `done_at` (first time it reaches `done` — re-opening and
  re-completing does not overwrite it). All three are optional and omitted from
  JSON when unset. They are carried on the stored issue record and surface in the
  full-fidelity single-issue view `jit issue show --json` and in the `--full`
  graph export; the compact `issue status` projection stays lean and omits them.

  These fields are additive and optional, so they do **not** bump the repository
  `schema_version` (still `2`): the issue record does not use serde
  `deny_unknown_fields`, so an older binary ignores the unknown keys and a newer
  binary defaults them when reading an older file. Documented in
  [cli-commands.md § `jit graph export`](docs/reference/cli-commands.md#jit-graph-export)
  and [storage-format.md § lifecycle timestamps](docs/reference/storage-format.md#lifecycle-timestamps).

  **Migration for existing repositories:** run `jit migrate lifecycle-timestamps`
  once to backfill the timestamps for pre-existing issues from `.jit/events.jsonl`
  (first Ready transition, first claim, first Done transition). It is idempotent
  (a second run writes nothing) and fills only still-absent fields. Issues whose
  event log carries no relevant transition stay unset. `--json` reports
  `{issues_scanned, issues_updated}`.

- **Structured gate findings in machine output.** An automated checker can
  append a machine-readable block to its stdout, fenced by the line-exact markers
  `<<<JIT-FINDINGS-JSON` / `JIT-FINDINGS-JSON>>>`, carrying
  `{verdict, summary, findings:[{id, severity, summary, file?, line?}]}`. jit
  parses it once at gate-run record time and surfaces the parsed structure as a
  `findings` object on the run across the gate views (`gate status` latest-run
  and `--all` history, `gate status-all`, and the gate-blocked transition error
  envelope's `checker_result`). Raw stdout is kept alongside it, and the
  structure is retained even in the lean `status-all` projection that drops raw
  stdout for passing runs. A new findings view, `jit gate status <id> <gate>
  --findings`, prints only the verdict and findings — one greppable finding per
  line as text, or `{key, run_id, has_findings, verdict, summary, findings}`
  with `--json`. The contract is opt-in and degrades gracefully: a checker that
  emits no block, or a malformed block, yields no `findings` field and no error,
  leaving existing plain-text behaviour unchanged. Documented in
  [custom-gates.md](docs/how-to/custom-gates.md#structured-findings-machine-readable-output);
  the bundled `scripts/ai-review.sh` is the first conforming checker.

- **`jit issue children <id>` — a container's direct children at a glance.**
  Lists the container's immediate dependencies (depth 1), each rendered exactly
  like `issue status` (one greppable line, ascending short-id order), or as the
  `{container: {short_id, title, state}, count, issues: [...]}` envelope with
  `--json` (`issues` is the same compact status projection). Containment follows
  the dependency DAG — a container's children are the issues it directly
  depends on; membership labels are advisory and not consulted. A non-container
  leaf simply lists nothing; for a deep rollup use `jit graph deps <id>
  --depth`. Replaces the per-child `show`/`status` loop agents ran to see where
  each child stands. A dependency edge pointing at a missing issue is surfaced
  in an optional `dangling` array (text: a `dangling:` line) rather than
  silently dropped, following the `issue show` `dangling_dependency_ids`
  precedent; a real storage error still propagates.

- **`jit issue progress <id>` — counts by state and a done/total rollup over a
  container's direct children.** Text prints the container line then `by state:
  backlog=… …` and `done <done>/<total> (<percent>%)  open …  rejected …`;
  `--json` emits `{container, count, by_state:[{state,count}], total, done,
  rejected, open, percent}`. `by_state` lists every lifecycle state (zero-count
  states included). Terminal-state semantics: `done` and `rejected` are counted
  distinctly (a rejected child is terminal but not delivered), `open` is every
  non-terminal child (`total − done − rejected`), and `done/total`/`percent`
  measure delivery. Totals cover resolvable children only; a broken dependency
  edge is surfaced in `dangling` (as for `issue children`). Membership follows
  the dependency DAG.

- **`jit query count --by state [--label ns:v ...]` — the same state rollup over
  a label bucket.** Aggregates every issue matching all `--label` patterns
  (ANDed; none given aggregates the whole repository) into the same
  `{count, by_state, total, done, rejected, open, percent}` shape as
  `issue progress`, minus the `container` header. This is the advisory-grouping
  counterpart to `issue progress`: DAG containment for the former, shared labels
  for the latter. `--by` is typed (`state` today); an unknown value is a usage
  error (exit 2). State counts enumerate the domain `State` enum, so the shape
  stays complete and stable.

- **`jit issue status <id>...` — the compact "where does this issue stand"
  view.** Prints state, per-gate status, and still-unmet dependencies as one
  greppable line per issue (`<short_id> [<state>] gates: <key>=<status>,...
  unmet: <short_id>,... title: <title>`; empty sections read `none`), or as one
  small object per issue with `--json`
  (`{short_id, state, gates:[{key,status}], unmet_dependencies:[short_id,...],
  title}`). It accepts multiple ids in argument order; two or more with `--json`
  use the `{"count": N, "issues": [...]}` list envelope. This replaces the
  hand-rolled `jq`/`python` projections agents previously reconstructed from
  full issue JSON. The unmet-dependency filter follows readiness semantics — a
  dependency is met exactly when it is terminal (`done`/`rejected`), the same
  test `jit query ready` applies.

- **`issue show --json` now exposes `unmet_dependencies`.** The full record
  gains an `unmet_dependencies` array — the subset of `dependencies` that are
  not yet met (state not terminal), each as `{id, short_id, title, state}` —
  computed by the same readiness-consistent predicate. Additive: existing fields
  are unchanged, and the array is always present (empty `[]` when nothing is
  blocking), so callers no longer recompute the filter client-side.

- **`jit config get` now covers the whole configuration surface.** The
  dotted-key accessor previously recognized only a hand-mapped subset
  (`worktree.*`, `coordination.*`, `global_operations.*`, `locks.*`,
  `events.*`); it now walks the full `config.toml` schema generically,
  including `type_hierarchy` (e.g. `type_hierarchy.strategic_types`,
  `type_hierarchy.types.epic`), `namespaces` (e.g.
  `namespaces.type.unique`), `item_kinds`, `documentation`, `validation`,
  `project`, and `version`. An intermediate key returns the whole subtree
  (`jit config get documentation`) rather than erroring; an unknown
  top-level key fails exit 2 naming the valid sections, and an unknown
  nested key fails exit 2 naming the missing segment. The five
  system/user/repo-layered sections keep resolving exactly as before;
  every other section reads the repo's `config.toml` only, with no
  built-in defaults layered in.

- **Wrong-verb hints for observed wrong-guess spellings.** `jit dep
  remove`/`delete`, `jit issue rm`/`remove`/`complete`/`edit`, `jit gate
  rm`/`delete`, `jit doc rm`/`delete`, and `jit label add`/`rm`/`remove` now
  fail fast (exit 2) with a message naming that group's canonical command
  (`jit dep rm`, `jit issue delete`, `jit issue update --state
  done`/`--label`/`--remove-label`, `jit gate remove`, `jit doc remove`)
  instead of clap's generic "unrecognized subcommand" error. These are hints,
  not new aliases — the wrong verb still fails, and canonical spellings are
  unchanged. `jit label --help` now also clarifies that the `label` group
  manages the namespace registry, not an issue's labels.

- **Repeatable, AND-combined `--label` filter across the query family.**
  `jit issue list`, the top-level `jit list` alias, the bare `jit query` form,
  and `jit query all`/`available`/`blocked`/`strategic`/`closed` now accept
  `--label`/`-l` multiple times; an issue is returned only when it matches
  every pattern given (wildcard `namespace:*` patterns still supported per
  occurrence). This matches `jit issue search --label`'s existing repeatable
  AND semantics. A single `--label` occurrence behaves exactly as before.

- **`jit init --json` and `jit graph export --json`.** `jit init` accepts
  `--json`, reporting `{repository_root, data_dir, repository_id,
  hierarchy_template, created_paths, modified_paths, message}` —
  `repository_id` is the git worktree id (`null` outside a git repository),
  `created_paths` lists only the files this run created (empty on a
  re-init), and `modified_paths` covers the one file init can update in
  place: an existing `.gitattributes` without the jit merge-driver block
  gets the block appended and is reported there rather than in
  `created_paths`. An unknown `--hierarchy-template` name emits the standard
  `--json` error envelope (`INVALID_ARGUMENT`, exit `2`). `jit graph export`
  gains `--json`, sugar for `--format json` on stdout; combining it with an
  explicit `--format dot`/`--format mermaid` is a usage error (exit `2`), and
  it composes with `--full`.

### Changed

- **Uniform JSON list envelope across list- and query-family commands.** Every
  command that emits a collection now wraps it in `{"count": N, "<collection>":
  [...]}`, where `count` equals the length of the collection and the key is
  plural and collection-typed. A single parse path now works for every
  list/query command — agents no longer need bare-array or dual-shape
  fallbacks. Collection keys: `issues` (`issue list`, `list`, `query
  all`/`available`/`blocked`/`strategic`/`closed`, `issue search`, multi-id
  `issue show`), `results` (`search`), `gates` (`gate list`), `presets` (`gate
  preset list`), `events` (`events tail`/`query`), `documents` (`doc list`),
  `assets` (`doc assets list`), `leases` (`claim list`/`status`), `namespaces`
  (`label namespaces`), `values` (`label values`), `templates` (`config
  list-templates`), `items` (`item list`/`search`), `worktrees` (`worktree
  list`), `roots` (`graph roots`), `dependents` (`graph rdeps`, `rdeps`),
  `nodes` (`graph deps`), `results` (`gate status --all`/`--limit`),
  `gates` (`gate status-all`), and `findings` (`invariant check`). Inner
  record shapes are unchanged. Where a command already carried an aggregate
  count, `count` is the size of the named collection specifically: `graph deps`
  keeps `summary.total` (unique dependencies across the whole tree) while `count`
  is the number of top-level `nodes`; `gate status-all` keeps `total` / `passed`
  (readiness tallies) while `count` is the number of `gates` entries.
- **Unified gate field naming across JSON outputs.** Everywhere a gate appears
  in `--json` output it is now identified by `key` and its state by `status`,
  matching `issue show`'s existing `gates[].key` — the larger, pre-existing
  surface. See the Migration section below for the renamed fields.
- **Typed exit codes for prefix and batch-usage errors; JSON envelope for
  startup failures.** Argument-class failures that previously fell through to the
  generic exit `1` are now classified:
  - An **ambiguous id prefix** (matches multiple issues) and a **too-short id
    prefix** (fewer than 4 characters) are argument errors (exit `2`), carrying
    the distinguishing `code` `AMBIGUOUS_ID` / `INVALID_ID_PREFIX` under
    `--json`. Human messages are unchanged.
  - The **batch-mode usage guards** on `jit issue update --filter` (mutually
    exclusive id/filter, and the `--content-format` / `--type` / description-flag
    rejections) now exit `2`, matching clap's own usage errors, and emit a JSON
    envelope under `--json`.
  - The **misplaced query-filter guard** (`--state`/`--assignee`/`--priority`/
    `--label`/`--full`/`--json` given before a `jit query` subcommand, where they
    would be silently dropped) now exits `2` and emits a JSON envelope under
    `--json`, matching the other query-family usage guards.
  - `jit dep rm <from> <target>` now validates **both** id arguments identically:
    a too-short or ambiguous prefix in either position is the same argument error
    (exit `2`), where previously a short `<target>` was silently reported as "not
    found" (exit `0`) while a short `<from>` exited `1`.
  - `jit dep add <from> <target>...` now emits the refined `INVALID_ID_PREFIX` /
    `AMBIGUOUS_ID` code (exit `2`) under `--json` for a too-short or ambiguous
    prefix in either the `<from>` or any `<target>` position, instead of the
    generic `DEPENDENCY_ERROR` (exit `1`). The non-`--json` exit code was already
    `2`; this aligns the `--json` code with it.
  - **Startup failures under `--json`** (repository not found, repository format
    too new) now emit a structured error object on stdout (`code`
    `REPOSITORY_NOT_FOUND` / `REPOSITORY_FORMAT_TOO_NEW`) while keeping their
    exit codes (`3` / `10`) and the human line on stderr. Previously `--json`
    produced an empty stdout for these.

### Fixed

- **`jit dep add` with multiple targets is now atomic.** Previously, a variadic
  add (`jit dep add <from> <to1> <to2> ...`) applied edges one at a time, so an
  edge that failed validation (e.g. a redundant edge under the default
  `--reduce`-less policy) left any earlier, already-applied edges persisted —
  the exit code no longer meant "nothing changed." Every requested edge is now
  validated against the would-be-final graph (every edge of the call applied at
  once, so a violation that only emerges from the COMBINATION of two edges in
  the same call is also caught) before anything is written; if any edge fails,
  none of them are added and no event is logged for any of them. The error now
  names every rejected edge, not only the first, and under `--json` carries a
  `details.rejected` array of `{from, to, code, message}` per rejected edge. A
  batch mixing an id-resolution failure with a graph-validation failure exits
  with the resolution failure's code (resolution runs before graph
  validation).
- **Doubled "Error: Error:" prefix on `ActionableError` paths.** Any command
  surfacing an `ActionableError`-derived failure (e.g. an already-claimed
  lease, a missing acting identity, the claims-require-git failure) now prints
  exactly one `Error:` prefix. `ActionableError::to_error_message()` no longer
  embeds its own prefix; the top-level CLI printer is the sole place that adds
  it.
- **`jit issue claim` on an issue already assigned to a different assignee**
  now names the current assignee and states that re-claiming as that same
  assignee succeeds (and promotes it to `in_progress`), instead of the bare
  "Issue is already assigned".
- **`jit claim acquire`/`jit claim release` outside a git repository** now
  distinguishes two causes instead of always suggesting `git init`: no git
  repository at all (still hints `git init`) vs. a git repository with no
  commits yet, so `HEAD` doesn't resolve to a branch (hints making an initial
  commit instead). Same typed error and exit code (`10`) for both; only the
  message differs.
- **`jit issue claim --help`** documents the idempotent same-assignee
  re-claim and the in_progress promotion.
- **Help text cross-references between assignment and lease commands.**
  `jit issue assign`/`claim`/`release`/`unassign` (assignee bookkeeping) and
  `jit claim acquire`/`release` (exclusive, time-boxed leases) share verbs but
  are different mechanisms; each command's `--help` now names its counterpart.
- **A downstream reader closing the pipe mid-write no longer panics.** Piping
  any command into something that exits early (`jit query all | head -1`,
  `jit --schema | head -c1`) used to surface Rust's raw panic banner (`thread
  'main' panicked ...: Broken pipe (os error 32)`, plus a backtrace hint) and
  exit `101`, because most of `main.rs`'s output goes through direct
  `println!`/`print!` calls that panic on a write error. `jit` now exits
  quietly with `141` (`128 + SIGPIPE`, the exit status a shell reports for a
  process a signal actually terminated), matching how everyday Unix pipelines
  compose.

### Migration

- **BREAKING — `jit dep add` with multiple targets no longer partially
  applies on failure.** A variadic add where one target fails validation used
  to leave every edge before the failure persisted; it now leaves the
  dependency set completely unchanged. Scripts that relied on the partial
  application (e.g. retrying only the failed target) must instead retry the
  whole batch. The `--json` success/error response no longer includes an
  `errors` array — a failure is now the command's `Err`/nonzero-exit path,
  carrying every rejected edge under `error.details.rejected` instead.
- **BREAKING — exit codes for prefix and batch-usage errors changed from `1` to
  `2`.** Scripts that branch on the exit code of an ambiguous/too-short id prefix,
  a `jit issue update --filter` usage guard, a misplaced pre-subcommand `jit
  query` filter, or `jit dep rm` with a bad id must
  treat `2` (invalid argument) as the failure code for these cases. A short
  `<target>` to `jit dep rm` that previously succeeded (exit `0`, reported under
  `not_found`) now fails with exit `2`; pass a ≥4-character prefix or the full id.
  `jit dep add` with a too-short/ambiguous prefix already exited `2`, but its
  `--json` `code` changes from `DEPENDENCY_ERROR` to `INVALID_ID_PREFIX` /
  `AMBIGUOUS_ID`. Consumers on `--json` can branch on the new `code` values
  (`AMBIGUOUS_ID`, `INVALID_ID_PREFIX`) instead of the exit code. No
  human-readable messages changed.
- **Additive — startup failures emit JSON on stdout under `--json`.** Callers of
  any command with `--json` in an uninitialized repository, or against a
  repository whose on-disk format is newer than the binary, now receive a parsable
  `{"error": {...}}` object on stdout (previously stdout was empty). Exit codes
  (`3` / `10`) and stderr are unchanged; consumers that only read the exit code
  are unaffected.

- **BREAKING — `jit issue show <id> <id> …` with `--json`.** Passing two or more
  ids previously emitted a bare JSON array (`[ {…}, {…} ]`). It now emits the
  list envelope `{"count": N, "issues": [ {…}, {…} ]}`. Consumers that indexed
  the top-level array must read `.issues` instead (e.g. `jq '.issues[]'` in
  place of `jq '.[]'`). Single-id `issue show --json` is unchanged: it still
  returns a bare issue object.
- **BREAKING — `jit graph deps --json` node collection renamed.** The node
  collection key changed from `tree` to `nodes` (`{"count": N, "nodes": [...]}`).
  Consumers must read `.nodes` instead of `.tree`; each node's inner shape
  (including nested `children`) is unchanged, and `summary` / `issue_id` /
  `depth` remain alongside.
- **Additive — `count` field added.** `gate preset list`, `doc assets list`, and
  `gate status-all` gained a top-level `count` alongside their existing
  collection. `doc assets list` continues to carry its `summary` object; its
  `count` mirrors `summary.total` (the length of `assets`). `gate status-all`
  keeps its `results` / `not_run` / `total` / `passed` keys; its `count` is the
  length of `gates`, not a readiness tally. Existing consumers that
  ignored unknown keys are unaffected.
- **Documentation — `gate status --all`/`--limit`.** The history view already
  emitted `{"count": N, "results": [...]}`; the envelope is now stated in the
  command help and the CLI reference (no shape change).
- **BREAKING — gate identification unified to `key` across all JSON output.**
  Every place a gate previously appeared under `gate_key` now appears under
  `key`; `gate status-all`'s collection also renamed `gate_statuses` to
  `gates`. Affected shapes:
  - `gate status-all` (`check-all`): `{"count": N, "gates": [{"key": ..., "status": ...}], ...}`
    (was `gate_statuses`, entries carried `gate_key`). The `results` entries
    (one per recorded automated run) also switch from `gate_key` to `key`.
  - `gate status`/`check`: the latest-run view, the `--all`/`--limit` history
    view's `results` entries, and the `--stdout`/`--stderr` flat view all carry
    `key` instead of `gate_key`.
  - `gate evaluate`/`pass` and `gate fail`: the success response's `gate_key`
    field is now `key`.
  - `gate evaluate-all`/`pass-all`: each entry in the `gates` array carries
    `key` instead of `gate_key` (the `gates` collection key itself was already
    correct).
  - `gate define` and `gate remove`: the response's `gate_key` field is now
    `key`.
  - A gate-blocked transition error's `error.details.blockers[]` entries
    (`type: "gate"`) carry `key` instead of `gate_key`.
  - A failed `gate evaluate`/`pass` error's `error.details` carries `key`
    instead of `gate_key`, and its nested `checker_result` is now the same lean
    run-summary shape used elsewhere (`key`/`status`/...) instead of the raw
    stored gate-run record (so it also drops `schema_version` and a duplicate
    `issue_id`).
  - The web UI's single gate-run endpoint (`GET
    /issues/:id/gate-runs/:run_id`) now returns the same run-summary shape
    (`key`, no `schema_version`/duplicate `issue_id`) instead of the raw stored
    record.
  Consumers must read `.key` wherever they previously read `.gate_key`, and
  `.gates` wherever they previously read `.gate_statuses` from `gate
  status-all`. The on-disk gate-run storage format
  (`.jit/gate-runs/**`) and `events.jsonl` gate-related event entries are
  unaffected — both keep `gate_key` as an internal/audit field; only the
  `--json` command output surface changed.
