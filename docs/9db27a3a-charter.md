# Charter: Version 1.0 Production Release

> Strategic container: 9db27a3a · milestone:v1.0

## Vision

Just-In-Time ships its first stable production release: a CLI-first,
repository-local issue tracker built for AI-agent workflows. The release is
coherent when it delivers a complete quality-gate system, dependency DAGs with
cycle detection, machine-consumable JSON storage under `.jit/`, multi-agent
coordination with file locking and leases, and user documentation complete
enough that an agent or human can operate the tool without reading its source.
Every epic under this milestone is judged against that outcome: work that does
not move the tracker toward a dependable, self-describing v1.0 an agent can drive
end to end is out of scope, however useful in isolation.

## Decision Log

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
  The tracker dogfoods this: its own gates (repo-validate, plan-review,
  coverage-preview, breakdown-review, code-review) live in the registry, not in
  code.
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
