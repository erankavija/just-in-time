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
- D-7: Charter decisions are project-addressable items over the vision charter
- D-8: Ship composable offline profile packages discovered from declared locations in v1.0 and defer the rest of the profile lifecycle
- D-9: Remove redundant release surfaces without removing product capabilities
- D-10: Support one Docker topology that serves the API and web UI from a repository mount
- D-11: Release v1.0 with no known dependency advisories and blocking security audits
- D-12: Keep the v1.0 MSRV on a current stable Rust release and enforce it in CI
- D-13: Give each adopter-facing fact one canonical documentation home
- D-14: Gate the v1.0 tag on completed profiles MVP and core maintenance
- D-15: Fix scoped validation in core rather than weakening bracket evidence
- D-16: Ship v1.0 through one tag-triggered release workflow publishing one GitHub release

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

### D-8: Composable offline profile packages before the complete lifecycle

- **Chosen:** v1.0 ships offline profile packages discovered from declared
  locations, composed by declared package-to-package dependency, and applied
  safely to fresh and existing repositories. Two packages ship: `jit-default`
  carrying the domain vocabulary a repository needs to be usable, and
  `jit-dogfood` carrying this project's workflow on top of it. Variables and
  sensitive-value handling, reconfiguration, detailed diff, three-way upgrade,
  safe removal, declared incompatibilities, search-path precedence, and
  shared-ownership semantics move intact to a post-1.0 epic.
- **Rejected:** Shipping the full profile package manager before v1.0, which
  makes a large new lifecycle subsystem the release critical path; dropping
  profiles from v1.0 entirely, which leaves the strongest dogfooded workflow
  difficult for adopters to install; and each package carrying a complete copy
  of the shared vocabulary, which is a hand-maintained duplicate held correct by
  an assertion.
- **Reasoning:** The binary carries mechanism and no instance of it
  (`@/inv/domain-agnostic`), so every type name, namespace, item kind, area
  classification, and workflow rule reaches a repository as package content.
  That makes a second package unavoidable — a usable default and this project's
  workflow are different opinions — and two packages sharing vocabulary need
  composition rather than duplication. The bounded surface is one dependency
  edge resolved at application time; the deferred list stays deferred.
- **Date:** 2026-07-14, amended 2026-07-31

### D-9: Cut redundant release surfaces, not product capabilities

- **Chosen:** Preserve the CLI, server/API, web UI, and MCP server as supported
  capabilities while removing packaging topologies, duplicated instructions,
  and compatibility surfaces that provide no distinct user outcome.
- **Rejected:** Blanket feature cuts to meet a date, which would make v1.0 less
  capable; and retaining every existing delivery shape, which multiplies
  testing, security, and documentation cost without equivalent adopter value.
- **Reasoning:** Production readiness is a coherence exercise. A smaller number
  of supported paths can be tested and documented end to end while the actual
  product surface remains available.
- **Date:** 2026-07-14

### D-10: One supported Docker topology

- **Chosen:** Ship one production Docker image in which the Rust server serves
  both the API and the built web UI. The supported Compose example runs that
  single service against a bind-mounted repository at `/repo`, with `.jit/` and
  linked project documents kept in their repository context.
- **Rejected:** Separate API and web images, a CLI image, and the current
  all-in-one image. They duplicate native CLI distribution, split one product
  deployment into avoidable services, or advertise processes and proxy routes
  that are not actually supervised as one production service.
- **Reasoning:** One image has a clear runtime contract, correct PID 1 and signal
  behavior, one health check, one repository mount, and one smoke-testable user
  path. Native binaries remain the primary CLI distribution and MCP retains its
  own package distribution.
- **Date:** 2026-07-14

### D-11: Zero known dependency advisories at release

- **Chosen:** Resolve every advisory reported by the Rust, web, and MCP
  production-dependency audits before v1.0. Audit jobs fail hard, use no blanket
  allowlists, and are required by the release workflow.
- **Rejected:** Advisory-only CI, `continue-on-error`, install fallbacks that
  hide audit failures, and carrying known advisories into v1.0 with a later-fix
  note.
- **Reasoning:** A stable release cannot claim production readiness while known
  vulnerable or unmaintained dependency paths are accepted by automation.
  Removing or replacing a dependency is part of fixing the gap when an in-place
  upgrade is unavailable.
- **Date:** 2026-07-14

### D-12: Current-stable MSRV for v1.0

- **Chosen:** Rust 1.97 is the v1.0 MSRV because it is the current stable release
  on 2026-07-14. CI builds and tests the workspace with exactly the declared
  MSRV; if stable advances before the v1.0 tag, the declaration is refreshed so
  it is never more than one stable release behind.
- **Rejected:** A stale conservative MSRV maintained without evidence, which
  increases compatibility burden; and an unpinned `stable`-only policy, which
  does not state or test the actual minimum compiler contract.
- **Reasoning:** A recent explicit compiler baseline reduces dependency and CI
  complexity while remaining reproducible for adopters and release builders.
- **Date:** 2026-07-14

### D-13: Canonical documentation homes instead of repeated prose

- **Chosen:** Each adopter-facing workflow or volatile fact has one canonical
  documentation page. README, installation, deployment, and component guides
  provide audience-specific entry points and link to that source instead of
  restating commands, matrices, or guarantees.
- **Rejected:** Keeping near-identical walkthroughs in several files, which
  makes every release change a multi-file synchronization task; and deleting
  useful discoverability, which would make concise documentation harder to find.
- **Reasoning:** Linking preserves navigation while pruning maintenance cost and
  enforces `@/inv/single-source-prose` across the public documentation surface.
- **Date:** 2026-07-14

### D-14: The v1.0 tag consumes the two completed upstream delivery streams

- **Chosen:** Keep the release boundary directly dependent on profiles MVP
  `9b7b5f9c` and core maintenance `6eb585bc`. Both reach a terminal state before
  the `v1.0.0` tag is created; the release work consumes their delivered
  contracts without changing or duplicating them.
- **Rejected:** Creating an intermediate v1.0 core-maintenance checkpoint only
  to make the dependency terminal; keeping core maintenance intentionally open
  as a living epic; and copying either upstream stream's work into the release
  container, which creates competing ownership.
- **Reasoning:** The existing DAG expresses the intended ordering. Planning and
  independent release hardening can proceed while core maintenance finishes, but
  tagging requires the completed core fixes and the profile quickstart. A
  synthetic checkpoint would add lifecycle ceremony without changing that
  contract.
- **Date:** 2026-07-15

### D-15: Scoped bracket validation is a core-maintenance prerequisite

- **Chosen:** Capture the scoped-validation defect under core maintenance and
  fix it before production-readiness breakdown can pass. Scoped rules evaluate
  only issues in the requested container subtree while resolving legitimate
  pointers against the complete issue index; production planning retains the
  failed coverage evidence and reruns the gate with the fixed engine.
- **Rejected:** Removing the valid historical `brackets:2821e177` label,
  inventing a cross-epic dependency, treating the coverage failure as a missing
  production criterion, or bypassing the gate. Each alternative would corrupt
  authoritative project data or weaken the plan-before-fan-out guarantee instead
  of correcting the validator.
- **Reasoning:** Global validation is clean, both referenced historical issues
  exist, and the failure appears only under `jit validate --scope 8b05a612` when
  an unrelated bracket is checked against a partial issue map. Fixing that
  project-wide validation primitive belongs to the already-prioritized core
  maintenance stream, consistent with D-3 and D-14.
- **Date:** 2026-07-15

### D-16: One tag-triggered workflow publishes one GitHub release

- **Chosen:** v1.0 publishes a single GitHub release, produced by one workflow
  that reacts to a maintainer-pushed annotated tag. That workflow runs the normal
  validation suites and the blocking audits on the tagged commit, builds the
  Linux x86_64 musl archive and the MCP tarball, smoke-tests the extracted
  binaries, and publishes the release with checksums, license texts, and release
  notes. Package-registry and container-registry publication stay out of v1.0, so
  the release requires no stored publishing credential and no registry account,
  and the supported container path is an image the operator builds from the
  repository.
- **Rejected:** A multi-workflow candidate-publication protocol built around an
  immutable candidate escrow, versioned transition ledgers, a fixed-ref tag
  authority primitive, and reconstruction from expired evidence. It is bespoke to
  a single version rather than reusable across releases, it manufactures the
  irreversibility it defends against by requiring immutable releases before the
  tag, and its ongoing cost — protected environments, a repository App with
  write authority, and hosted cross-workflow fixtures — outlives the one release
  it serves.
- **Reasoning:** The failure modes that protocol addressed are real but cheap
  here. The project has no downstream consumers and no prior tags, and the
  release has one atomic publication target, so recovery from a botched
  publication is deleting a tag and cutting another one. The supply-chain
  properties worth keeping — blocking audits, full-SHA action pins, and a smoke
  gate ahead of publication — are retained directly; the transaction protocol
  wrapped around them does not amortize across a single release. Distribution
  breadth is a separate decision, taken once the release path itself is proven.
- **Date:** 2026-07-29
