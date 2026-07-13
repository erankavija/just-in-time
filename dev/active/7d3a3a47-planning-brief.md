# Planning brief — Dependency-aware container artifact archival (7d3a3a47)

**Status:** Owner-approved input to formal planning. This brief fixes product
direction and scope; it is not the implementation plan or breakdown.

**Provenance:** Distilled from rejected epic 94f873c8, superseded enhancement
f16cfe8c, dev/active/7d3a3a47-investigation.md, a read-only audit of the
adjacent gf2 repository, and an owner interview on 2026-07-11.

## Why this epic exists

JIT can archive one Markdown or HTML document plus conventionally located
assets. It cannot plan or archive the complete artifact set of a container.
Real work produces bundles: plans, reports, HTML presentations, CSS themes,
figures, CSV evidence, source excerpts, and files shared by several issues.

The JIT repository retains substantial terminal-issue material in active
locations. The stronger adopter example is gf2: the audit found 381 issue
document references spanning Markdown, HTML, CSV, PNG, source, text, and other
formats, with 374 references on terminal issues. Its presentation bundles
contain HTML with sibling styles and shared figures, while benchmark evidence
is often referenced by several issues. This is an artifact-lifecycle problem,
not merely a prose-document problem.

The replacement epic introduces one artifact planner and archive family that
support both single artifacts and whole containers. It does not revive the
rejected Phase 2 roadmap wholesale.

## Audit of the current mechanism

The authoritative code-level audit is
dev/active/7d3a3a47-investigation.md. These findings bind planning:

1. **Useful existing safety primitive.** The command stages copies, refuses
   occupied destinations, verifies before metadata commit, re-links matching
   issue references, appends an archive event, and deletes sources last. On an
   unverifiable rollback it keeps a safe duplicate. Its guarantee is
   referential consistency, not a transaction across filesystem files, issue
   JSON, and the append-only event log.
2. **Preview is incomplete.** Dry-run returns before checking active references
   and destination occupancy. Its JSON result is not an execution-faithful
   artifact plan.
3. **Discovery is shallow and convention-based.** Only Markdown and HTML are
   scanned, only one level is discovered, and movable assets are recognized by
   assets/ or stem_assets/ placement. Sibling CSS, themes, nested dependencies,
   CSV, and images are not handled as general artifacts.
4. **Sharing is not repository-wide.** Archive-time sharing is a directory
   heuristic. A reference-count classifier exists but is not used by archival.
5. **Opaque artifacts cannot use the executor.** Explicit CSV, PNG, SVG, and
   other non-text artifacts fail adapter-based scanning even though they should
   be eligible archive roots.
6. **Pinned references are unsafe to rewrite.** Relinking changes the path but
   preserves a pinned commit at which the new path normally does not exist.
7. **Cached metadata becomes stale.** Relinking preserves asset paths from the
   old layout.
8. **Post-archive verification is defective.** It compares destination-resolved
   paths with source-resolved paths, so the intended check is normally skipped.
9. **Coordination is too narrow.** Planning, staging, multiple issue saves, and
   event append are not protected as one coordinated sequence despite an
   available repository write guard.
10. **Policy matching needs normalization.** Managed paths use string prefixes
    rather than path-component containment.

The design may reuse the sequencing and rollback principles, but must not be a
loop around the current archive_document method.

## Evidence from gf2

- Its documentation configuration is only a commented example, so the current
  command refuses even dry-run. Archive execution must remain explicit opt-in.
- Most evidence lies outside JIT's default managed paths, including
  dev/bench_results, dev/plans, dev/benchmarks, and dev/presentations. Preview
  must explain exclusions rather than silently omit them.
- A completed issue links dev/active/e095a100-presentation/talk.html, which
  depends on sibling base.css and themes/rust.css. The current collector blocks
  this representative bundle.
- Permanent presentations use sibling CSS and shared figures. These must appear
  as retained dependencies, not move merely because a container uses them.
- Some artifacts are shared by active and terminal issues. A container operation
  must not force-relink the active consumer.
- Many explicit artifacts are opaque roots such as CSV and PNG. Parser support
  controls dependency discovery, not archive eligibility.
- Legacy gf2 issues do not reliably carry done_at. This reinforces deferring
  time-based archival instead of inventing fallback timestamps.

## Owner decisions

### D-1 — Archival is explicit opt-in

Execution requires configured documentation policy declaring managed and
permanent paths, archive root, and categories. An unconfigured repository may
receive an inventory explaining that archival is disabled, but JIT never infers
which evidence is safe to move.

Rejected: silently use default paths, which could move evidence whose lifecycle
the repository never delegated to JIT.

### D-2 — Do not relocate another container's artifacts

An artifact referenced outside the selected subtree is never relocated by that
container operation. A required dependency may be copied while the original
remains. Ambiguous cases block execution.

Rejected: global relink or a general force option.

### D-3 — Pinned references remain historical

Archive never rewrites a pinned document reference. Its historical source path
remains available; content may be copied into the archive bundle when needed.

Rejected: path-only relink or implicit conversion to working-tree provenance.

### D-4 — One category and one container-owned destination

The caller selects one configured category for the whole container operation.
The destination is a container-owned directory beneath that category, preserving
internal relative topology.

Rejected: category-per-artifact placement, which fragments bundles.

### D-5 — Initial recursive discovery is static

The first version recursively follows supported local references in Markdown,
HTML, and CSS, including CSS url() and @import. Detected local
JavaScript/runtime loading blocks execution. JIT does not guess dependencies.

### D-6 — Time-based archival is deferred

There are no retention periods, age filters, scheduled sweeps, or lifecycle
triggers. Candidate reporting uses current terminal state and policy eligibility.

Rejected: first-done, updated, or reconstructed timestamps.

### D-7 — Preview defaults; execution is explicit

Archive commands produce a complete non-mutating plan by default. Mutation
requires --execute. Execution recomputes and revalidates immediately before
mutation rather than applying stale preview data.

### D-8 — Only terminal containers can execute

Done and rejected containers may execute. Non-terminal containers may be
previewed but cannot execute, with no force override.

### D-9 — One unified archive command family

The target surface is:

    jit archive document <path> --type <category> [--execute] [--json]
    jit archive container <id> --type <category> [--execute] [--json]
    jit archive candidates [--json]

Both target types use the same planner, result schema, policy, coordination, and
executor. jit doc archive is removed completely as a clean-cut pre-v1.0
migration; it is not retained as an alias or migration stub.

### D-10 — Reachability defines a bundle

Select explicit issue-linked artifacts plus recursively reachable supported
static dependencies. Never sweep an entire source directory implicitly.
Unreferenced siblings may be reported as informational not-selected entries but
do not move.

### D-11 — Candidates are container-oriented

Candidate reporting lists terminal containers with policy status, artifact
counts, blockers, and move/copy/retain summaries. Individual artifacts are
details inside a candidate, not independent archival candidates.

## Product contract

### Planning

Document and container targets produce the same deterministic plan. Each artifact
entry includes:

- normalized source and proposed destination;
- action: move, copy, retain, or block;
- explicit-reference or embedded-dependency provenance;
- owners inside and outside the selected subtree;
- active, terminal, permanent, pinned, missing, and conflict evidence;
- supported and unsupported dependency edges;
- issue-reference changes;
- stable machine-readable warnings and blockers.

Opaque explicit artifacts are valid roots. Adapters discover edges; they do not
decide whether a root is eligible.

Preview runs every non-mutating precondition used by execution, including source
availability, normalized policy, terminal eligibility, outside owners, pinned
references, destination conflicts, and before/after local-edge reachability.

### Execution

Execution recomputes under repository coordination, stages the complete artifact
set, validates every supported local edge in the proposed layout, persists
reference changes and events, then removes only sources proven safe to remove.

The guarantee is:

> No failure loses an artifact, overwrites an existing destination, or leaves an
> issue reference relying exclusively on a missing path. When rollback cannot be
> verified, retain resolvable duplicates and report manual cleanup.

This is intentionally not described as atomic batch execution.

### Candidate reporting

jit archive candidates is read-only. It lists terminal containers and explains:

- whether documentation policy is configured;
- managed and permanent path status;
- outside-subtree, active, pinned, missing, unsupported, and conflict blockers;
- a suggested category when derivable, without replacing explicit selection;
- plan summary counts.

It performs no age or retention filtering.

## Owner amendment (2026-07-13)

Document categorization is scrapped as a legacy idea. This supersedes the
category clauses of D-4 and D-9: the archive command family takes no category
input (`--type` disappears from the target surface), destinations derive solely
from the archive root and the container-owned mirror layout, candidate
reporting suggests no category, and the `[documentation.categories]` table
retires with the legacy command. The container-owned-destination and
unified-command-family halves of D-4/D-9 stand unchanged.

## Owner amendment (2026-07-13, proportionality)

The concurrency and crash contract is scoped explicitly: safety guarantees
hold under the repository write guard against concurrent JIT writers and
detect benign concurrent modification before destructive steps; concurrent
external mutation of the working tree during an archival operation is out of
contract, with git history as the recovery channel in versioned repositories.
Consequences, all owner-approved: no write-ahead intent events, publication
receipts, or destination-provenance tracking; a single archive event per
mutating execution; deletion safety is verify-recorded-hash-then-delete under
the held guard; rerun convergence by recomputation replaces crash-recovery
protocols; a cross-filesystem archive root is a clean error rather than a
staging design. Detected dynamic or module loading in relocated bundle
members warns rather than blocks. Pinned references remain untouched and
commit-canonicalized, but their commit-resolved dependency closures are
outside relocation scope and are not enumerated (REQ-01 is scoped to
working-tree artifacts accordingly). The goal is a safe, easy archival
convention with low maintenance burden, not adversarial filesystem
guarantees.

## Reconcile the epic criteria

REQ-01 through REQ-04 and REQ-06 remain aligned with the approved direction.

REQ-05 currently requires done_at. Formal planning should amend it to:

> Reports container-oriented archival candidates using current terminal state,
> configured documentation policy, artifact ownership, and plan blockers without
> mutating the repository.

This removes time behavior per D-6 and makes ownership analysis explicit.

## Recommended decomposition

### Story A — Artifact plan model and resolver

Build pure deterministic artifact-graph and action-classification logic:
container closure, explicit opaque roots, recursive Markdown/HTML/CSS edges,
normalized paths, repository-wide owners, pinned/permanent semantics, stable
JSON, and complete blockers.

### Story B — Unified archive CLI and safe executor

Introduce jit archive document and jit archive container, remove jit doc archive,
and execute recomputed plans with staged validation, coordination,
reference/event updates, safe rollback, metadata refresh, and retry behavior.

### Story C — Container candidate reporting

Add the read-only container candidate query using the shared planner. Report
configuration and policy exclusions rather than hiding them. Include no retention
or time semantics.

Stories B and C depend on A and can proceed independently afterward.

## Required verification

- Pure unit/property tests: deterministic closure, normalized paths, cycles,
  stable ordering, ownership classification, action decisions, and before/after
  reachability.
- Harness tests: document/container resolution, opaque roots, permanent paths,
  outside owners, pinned references, absent configuration, terminal rules,
  candidate reporting, and JSON envelopes.
- Filesystem/failure-injection tests: occupied destinations, concurrent or stale
  plans, partial staging, partial relink, event failure, deletion failure,
  retry/idempotency, and exact preview non-mutation.
- Fixtures: Markdown links; HTML to sibling CSS; CSS imports and url(); CSV, PNG,
  and SVG roots; permanent shared figures; active outside consumers; identical
  filenames; missing edges; pinned commits; and dependency cycles.

## Explicit non-goals

- Time-based retention, automatic archival, scheduled sweeps, or lifecycle
  triggers.
- General link rewriting or asset normalization.
- JavaScript/runtime dependency inference.
- AsciiDoc, reStructuredText, or MDX adapters without demand.
- Git LFS download or policy enforcement.
- Parallel execution.
- Literal transactionality across all persistence surfaces.
- Implicit directory ownership.

## Inputs to formal planning

The formal plan must:

1. Reconcile REQ-05 before applying coverage labels.
2. Treat the investigation as grounding and address every relevant defect.
3. Preserve domain, storage, command, and CLI boundaries from AGENTS.md.
4. Specify the archive-plan JSON schema and blocker taxonomy before CLI fan-out.
5. Keep candidates, preview, and execution as consumers of one plan model.
