# Charter: Version 1.0 Production Release

> Strategic container: 9db27a3a · milestone:v1.0

## Vision

Just-In-Time ships its first stable production release: a CLI-first,
repository-local issue tracker whose primary user is an AI agent driving work end
to end. Every surface is machine-consumable first and human-pretty second — plain
JSON under `.jit/`, `--json` on every command, dependency DAGs with cycle
detection, quality gates, and multi-agent coordination through file locking and
leases. The engine stays domain-agnostic (`@/inv/domain-agnostic`): type names,
label vocabularies, gate keys, templates, and workflow shapes come from repository
configuration, never from baked-in assumptions. The release is coherent when an
agent or human can operate the tracker end to end from its own documentation
without reading its source. Every epic under this milestone is judged against that
outcome; work that does not move the tracker toward a dependable, self-describing,
agent-drivable v1.0 is out of scope, however useful in isolation.

## Decision Log

- D-1: Repository-local git-versioned JSON storage, not an external database
- D-2: Quality gates declared in `.jit/gates.toml`, not baked into the binary
- D-3: Plan-before-fan-out bracket gates a breakable container before implementation
- D-4: git optional for core commands, required only for claims and leases
- D-5: A milestone-tier steward skill sits above the epic-level execution lead
- D-6: Each item kind declares its own source of truth (markdown-first or registry-first)
- D-7: Charter decisions are addressable `@/charter/D-N` items over the vision charter

## Decision Details

### D-1: Repository-local git-versioned JSON storage

- **Chosen:** All tracker state is plain JSON under `.jit/` (issues, events,
  claims, config), versioned by the project's own git history.
- **Rejected:** An external database (SQLite or a server-backed store) — it adds
  a runtime dependency, its binary state does not diff or merge in review, and it
  splits the source of truth away from the repository the work lives in.
- **Reasoning:** JSON-in-git is machine-consumable, human- and agent-diffable,
  branches and merges with the code it tracks, and needs no process to run. The
  append-only `events.jsonl` and per-issue files reconcile through ordinary git
  merges.
- **Date:** 2026-07-03

### D-2: Config-declared gates over hardcoded presets

- **Chosen:** Quality gates are declared in `.jit/gates.toml` and referenced by
  key; a project defines its own gate set and checkers there.
- **Rejected:** Baking a fixed set of gate presets into the Rust binary — every
  new or tuned gate would require recompiling and reinstalling `jit`.
- **Reasoning:** Projects set their own quality bar without touching the engine.
  The tracker dogfoods this: its own gates (`cargo-ci`, `jit-validate`,
  `code-review`, and the planning-bracket trio) live in the registry, not in code.
- **Date:** 2026-07-03

### D-3: Plan-before-fan-out bracket for breakable containers

- **Chosen:** A breakable container is bracketed by a planning node and a
  breakdown node, instantiated by the `plan` template via `jit apply plan`, with
  gates that must pass before any implementation child is dispatched.
- **Rejected:** Direct breakdown with no plan or coverage gate — decompose a
  container straight into tasks and start work.
- **Reasoning:** Coverage of the container's `[hard]` success criteria is proven
  at plan time, before code is written. A gap surfaces as a failed coverage gate
  on the breakdown node, not as rework discovered mid-implementation.
- **Date:** 2026-07-03

### D-4: git optional for core, required only for claims and leases

- **Chosen:** Core commands run without git; only the claim and lease commands
  (`jit claim acquire/release/renew/heartbeat/status/list`) require a git
  repository and fail with a typed `ClaimRequiresGitError` (exit 10) outside one.
- **Rejected:** Requiring git for every command — a uniform but heavier
  dependency.
- **Reasoning:** The tracker must run in non-git contexts, but multi-agent
  coordination genuinely needs git worktree identity and branch tracking to name
  a claimant. The dependency is scoped to exactly the commands that cannot work
  without it.
- **Date:** 2026-07-03

### D-5: A milestone-tier steward skill above the epic lead

- **Chosen:** A separate `jit-project-lead` skill stewards the milestone-tier
  container and delegates each sub-strategic container to a dispatched
  `jit-execution-lead` (this epic, f2532a2d).
- **Rejected:** Extending `jit-execution-lead` to also cover milestone scope —
  one skill spanning two tiers.
- **Reasoning:** Each lead keeps a single-tier scope and a thin skill file. The
  steward owns vision and cross-container coherence; the execution lead owns the
  interior of one container. Merging them would grow one skill across two
  responsibilities and blur where a decision belongs.
- **Date:** 2026-07-03

### D-6: Per-kind source of truth for addressable items

- **Chosen:** Every addressable item kind declares its source of truth in
  `[item_kinds]`: description-embedded kinds (requirement, decision, risk) are
  markdown-first; registry-backed kinds (invariant, rule, gate) are
  registry-first and projected into markdown by `jit … render`. The item index is
  always a projection of the declared source, never itself the source.
- **Rejected:** One uniform substrate for every kind — all-markdown loses the
  structured TOML registries the engine already reads and machine-edits, while
  all-registry forces prose-embedded requirements and decisions out of the text
  that defines them.
- **Reasoning:** Each kind lives where its author naturally edits it: criteria and
  decisions inside the issue prose, invariants/rules/gates in their TOML
  registries. A per-kind declaration keeps exactly one source of truth per kind
  while a single addressing scheme (`@/<kind>/<self-id>`) spans both substrates.
- **Date:** 2026-06-27

### D-7: Charter decisions as addressable items

- **Chosen:** The vision charter's decisions are a project-scope, markdown-first
  `charter` item kind. Each summary row under `## Decision Log`
  (`- D-N: <one-liner>`) is addressable as `@/charter/D-N`; the full entries sit
  under `## Decision Details`. An issue cites a decision with the reused `per:`
  link namespace (`per:@/charter/D-N`).
- **Rejected:** A new dedicated link namespace for charter references — redundant,
  since resolution is by qualified id and `per` already carries decision links
  unambiguously; and leaving the charter as unaddressable prose — decisions could
  then be neither cited nor machine-validated, so a dangling reference would go
  undetected.
- **Reasoning:** Reusing the proven markdown-first kind machinery (the
  `definition` kind precedent) turns every logged decision into a citable,
  `jit validate`-checked anchor with no new engine surface. The bullet-index /
  details split keeps the addressable rows short and stable while the full
  rationale stays readable below them.
- **Date:** 2026-07-07
