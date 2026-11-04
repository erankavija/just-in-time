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
agent-drivable v1.0 is out of scope, however useful in isolation. Simplification is
encouraged: a change that removes, unifies, or derives mechanism is preferred over
one that adds it (D-18).

## Decision Log

- D-1: Repository-local git-versioned JSON storage, not an external database
- D-2: Quality gates declared in `.jit/gates.toml`, not baked into the binary
- D-3: Plan-before-fan-out bracket gates a breakable container before implementation
- D-4: git optional for core commands, required only for claims and leases
- D-5: A milestone-tier steward skill sits above the epic-level execution lead
- D-6: Each item kind declares its own source of truth (markdown-first or registry-first)
- D-7: Charter decisions are project-addressable items over the vision charter
- D-8: Apply profile packages once and let git keep their history, with no ownership records to keep in sync
- D-9: Remove redundant release surfaces without removing product capabilities
- D-10: Support one Docker topology that serves the API and web UI from a repository mount
- D-11: Release v1.0 with no known dependency advisories and blocking security audits
- D-12: Keep the v1.0 MSRV on a current stable Rust release and enforce it in CI
- D-13: Give each adopter-facing fact one canonical documentation home
- D-14: Gate the v1.0 tag on every epic under the v1.0 milestone, as the dependency graph states it
- D-16: Ship v1.0 through one tag-triggered release workflow publishing one GitHub release
- D-17: Publish each mutation through one redo record over the files it writes, so its cost never scales with project history
- D-18: Prefer removing, unifying, or deriving mechanism over adding it
- D-19: Reads never scale with project history; the storage layout addresses per-key data directly
- D-20: One computation per capability; previews, scoped views, and checkers are views over it
- D-21: Reach git only through the git CLI, from one storage-owned module
- D-22: Reads never wait on writers; a lock-free capture is admitted only when no commit crossed it

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
  repository and fail with a typed `RequiresGitError` (exit 10) outside one.
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

### D-8: Profiles apply once and git keeps their history

- **Chosen:** v1.0 ships profile packages that are rendered into a repository
  once. A package is an offline directory or a packed archive; two ship:
  `jit-default` carrying the domain vocabulary a repository needs to be usable,
  and `jit-dogfood` carrying this project's workflow on top of it. Applying
  resolves the package's declared non-secret variables, composes packages in
  declared dependency order with the dependent package winning a key both
  declare, refuses two unrelated packages that declare one key differently, and
  publishes the result through one redo record (D-17). From then on the
  repository owns every value: editing it is always valid, and no ownership
  record, fingerprint, or drift check exists. `jit profile diff` reports where
  a repository differs from a package. Applying a newer version, or other
  variable values, refuses while the files it writes carry uncommitted changes,
  then writes the new rendering, and the adopter keeps local edits through git.
  The profile operation's event records what was applied. Package authors edit
  the manifest directly, and pack and add exchange packages. Remote package
  registries remain deferred.
- **Rejected:** An ownership lifecycle: durable ownership records with base
  fingerprints, three-way publication, conflict and divergence classes with
  remedies, capture back into the owning package, reconfigure, and upgrade. It
  keeps two copies of every value in sync and makes routine acts fail: a newer
  package cannot change a contributed value the adopter never touched, and
  recovering from an in-place edit takes a capture and an apply together. Also
  rejected: layering package defaults under repository overrides, which leaves
  `.jit/` an incomplete description of the configuration agents read;
  generating `.jit/` from packages, which makes editing `.jit/` an error; and
  dropping profiles, which leaves the dogfooded workflow hard to install.
- **Reasoning:** Drift reported on an edited profile-owned file comes from the
  ownership record, not from applying once: without a record an edit is an
  ordinary change, and git already shows and restores it. A newer package
  version then needs no merge engine in jit:
  written over a committed tree, it leaves every difference in git's view, and
  the adopter keeps what they changed. Core commands still work without git
  (D-4); there, `jit profile diff` is the comparison to read before applying.
- **Consequences:** The ownership machinery is deleted rather than repaired:
  applied-profile records, claims and fingerprints, profile validation and its
  checks inside `jit validate`, three-way publication, conflict remedies,
  capture's reconciliation, reconfigure, and upgrade. No released record format
  exists, so a repository holding applied-profile records drops them with no
  migration boundary (`@/inv/canonical-cutover`).
- **Date:** 2026-07-14, revised 2026-10-15

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

### D-14: The v1.0 tag consumes every v1.0 epic

- **Chosen:** The task that cuts the `v1.0.0` tag depends on every epic under
  the v1.0 milestone, and each reaches a terminal state before the tag is
  created. The dependency graph is the list; this entry names none. Release work
  consumes the delivered contracts without changing or duplicating them.
- **Rejected:** A hand-kept list of epics here, which goes stale as epics join
  or close (`@/inv/single-source-prose`); intermediate checkpoints created only
  to make a dependency terminal; and copying an upstream epic's work into the
  release container, which creates competing ownership.
- **Reasoning:** The DAG already expresses the ordering. Planning and release
  hardening proceed while upstream epics finish, and the tag waits for all of
  them, so v1.0 ships the simplified, consistent surface rather than the one
  those epics replace.
- **Date:** 2026-07-15, revised 2026-10-20

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

### D-17: Mutations commit through a redo record over the files they write

- **Chosen:** An issue mutation's cost scales with the files it writes, never
  with the event log or other project history. The repository lock serializes
  writers of repository state; machine-local control files (worktree identity,
  server pid and log, claims) stay outside the lock and the record. A mutation
  reads the registries and the issues its derivation consults, plus the plan
  documents a transition rule reads; issue-linked documents stay with
  validation. Immediately before commit it compares the live bytes of every file
  it writes with the bytes it read, and checks the event log's length and tail,
  returning a retryable conflict on any difference. Its commit point is one
  synced redo record under the data root's gitignored `tmp/`, holding each
  directory it makes that no other operation's path lies beneath, the complete
  new bytes and mode of each written file, each deleted path, and the event
  append as an offset and its bytes. Every record appends an event, so the
  event log's length counts the repository's commits. Paths are relative to the data root or the
  worktree, so projections, profile assets, and archived documents publish
  through the same record. Applying the record makes each directory, replaces
  each file by temp-file and rename, removes each deleted path, writes the
  append at its offset, and deletes the record. Opening a session replays a leftover record
  idempotently. `jit init` creates an absent data root through a small publisher
  of its own (a synced stage directory and a no-replace rename), because no
  `tmp/` exists before it, and writes the data root's `.gitignore` naming
  `tmp/`. The change leaves the storage layer with less code than it has now.
- **Rejected:** Keeping the rollback kernel and making its hashing cheaper, which
  still rewrites the whole event log on every mutation and keeps the code;
  independent atomic writes of the issue file and the event line, which let a
  crash leave an issue changed without its event (`@/inv/event-log`); dropping
  the preimage check, which lets jit and a concurrent `git checkout` or editor
  silently overwrite each other; and revalidating every file a mutation read,
  which costs a full reread to catch staleness in files the mutation does not
  write.
- **Reasoning:** On an 876-issue adopter repository a label update fell from
  1.8 s to 110 ms once mutations stopped capturing documents and repeating
  captures, and the remainder is dominated by the old mechanism: the full
  event-log rewrite and about eight whole-file digests of it per publication,
  plus rereading and hashing every issue at apply. Coupled-state convergence and
  the concurrent-writer check are the guarantees worth keeping; a redo record
  over the written files delivers both. After a crash before the commit point
  the old state stands, and after it the next session completes the new state,
  so no rollback, backup, or staging is needed. The accepted cost is that an
  external edit to a file a mutation read but does not write is not detected
  during that mutation; that makes one result stale, and corrupts nothing.
- **Date:** 2026-09-19, amended 2026-10-01

### D-18: Simplification is encouraged

- **Chosen:** A change that removes, unifies, or derives mechanism is preferred
  over one that adds it. When a defect, a friction report, or a feature request
  can be met by deleting a command, record, flag, or configuration key, by
  folding two mechanisms into one, or by deriving a fact the repository already
  states, that is the solution to choose, and its plan names the code and
  procedure it removes. Reversing or narrowing an earlier decision to reach a
  simpler design is legitimate, and is recorded here as an amendment.
- **Rejected:** Meeting each reported friction with a new feature, which grows
  the surface an agent must learn with every fix; and keeping a mechanism
  because an earlier decision chose it after the reasoning behind it stopped
  holding.
- **Reasoning:** The primary user is an agent operating the tracker from its
  documentation (Vision), and every mechanism adds commands, records, and
  failure modes that agent must learn and recover from. The friction a
  downstream execution lead reported on 2026-10-15 came mostly from mechanisms
  interacting: two gate-verdict reuse paths that disagreed about commits, and
  profile ownership records kept in sync with the registries they described.
  Before v1.0 no released format constrains deletion
  (`@/inv/canonical-cutover`), so simplifying now is cheapest.
- **Date:** 2026-10-15

### D-19: Reads never scale with project history

- **Chosen:** The storage layout addresses per-key data directly: reading one
  issue's state, gate results, or lease touches files sized by that key, never
  the whole history. The event log, the claims log, and gate-run history are
  append-only records, read only by commands that report history. The layout
  changes to make this hold; no migration boundary exists before v1.0.
- **Rejected:** Derived indexes over the current layout, which add files that
  must stay coherent with their sources; and periodic compaction, which only
  bounds cost between compactions.
- **Reasoning:** D-17 made writes independent of history; reads are the
  remaining cost an agent pays as a project ages. Restructuring removes the
  scans instead of maintaining structures that hide them (D-18).
- **Date:** 2026-10-20

### D-20: One computation per capability

- **Chosen:** Each capability is computed one way. Validation computes one
  report over the captured repository and filters it for an issue, a scope, an
  explanation, or a gate checker. A preview runs the same capture and
  derivation as its operation, read-only. A rule, derivation, or parser exists once and every
  caller goes through it.
- **Rejected:** Scoped or preview variants that reimplement part of the full
  computation for speed or convenience. They drift: preview and apply disagree,
  and a per-issue view becomes slower than the whole report it approximates.
- **Reasoning:** Consistency is proven once, against the one computation
  (`@/inv/convention-convergence`), and every view inherits it.
- **Date:** 2026-10-20

### D-21: git through the CLI, from one storage-owned module

- **Chosen:** Every git access goes through one storage-owned module that runs
  the git CLI. The libgit2 binding is removed. Repository layout facts are
  detected once per invocation and passed down. Core commands still work
  without git (D-4).
- **Rejected:** libgit2 everywhere, which keeps a large dependency, its
  certificate loading on every read, and its advisory surface (D-11); and a
  module hiding both backends, which keeps two mechanisms.
- **Reasoning:** jit needs a handful of git facts per invocation. One spawn per
  needed fact through the tool adopters already have is the smaller, uniformly
  testable mechanism (D-18).
- **Date:** 2026-10-20

### D-22: Reads never wait on writers

- **Chosen:** Reads never wait on writers. A read captures without locks and is
  admitted only when no redo record was present and the event log did not grow
  across its capture; it retries a bounded number of times and then reads under
  the repository lock, which also completes a leftover record. Every redo record
  appends at least one event, so the log's length is the repository's commit
  counter.
- **Rejected:** A shared/exclusive repository lock, which still blocks readers
  on writers and lets a stream of readers starve writers; and re-reading the
  read set to validate a capture, which is unsound when a freed inode is reused
  for bytes that return to a prior value.
- **Reasoning:** Many agents read one repository while others write it. A read
  that serializes behind every writer and every other reader makes the slowest
  mutation everyone's latency. The admission check reuses the record and the
  event log that D-17 already maintains, so concurrency adds one check rather
  than a lock mode (D-18).
- **Date:** 2026-10-24
