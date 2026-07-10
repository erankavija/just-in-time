# REQ-01 evidence — scope-aware doc-review prompt

REQ-01: the review prompt restricts findings to a description-declared file scope
when present, and reviews the full surface when absent; behavior demonstrated by
one scoped and one unscoped run.

This document is the durable record of that demonstration. It has two parts: a
deterministic mechanism trace (how the scope reaches the reviewer) and a real
scoped reviewer run (that the reviewer honors it).

The instruction that governs the behavior is the `### Scoped audits` paragraph in
`scripts/doc-review-prompt.md:27-29`:

> When the context issue's description declares an audited scope — an explicit
> list of files or directories under review — restrict ALL findings to that
> scope. Files outside the declared scope are out of bounds even when they drift;
> report only defects whose location falls inside the declared footprint. When
> the description declares no scope, review the full surface enumerated above.

## How the scope reaches the reviewer (checkable trace)

`scripts/ai-review.sh` assembles the reviewer input in two pieces:

- `PROMPT=$(jq -r '.prompt // empty' "$JIT_CONTEXT_FILE")` (`ai-review.sh:58`) —
  the gate's prompt. For the `doc-review` gate this is `scripts/doc-review-prompt.md`
  (`.jit/gates.toml`: `prompt_file = "./scripts/doc-review-prompt.md"`), so the
  `### Scoped audits` paragraph is always in the piped prompt.
- `CONTEXT_JSON=$(jq -c 'del(.prompt)' "$JIT_CONTEXT_FILE")` (`ai-review.sh:66`) —
  every context field except the prompt, injected under a `## Context` heading in
  the heredoc (`ai-review.sh:73-80`, prompt first, then the `## Context` json
  block). The issue's `description` rides in this block as `.issue.description`
  (context shape: `GateContext` in `crates/jit/src/domain/types.rs:1231-1242`,
  fields `prompt`, `issue`, `gate`, `run_history`). A scope line written in the
  issue description therefore reaches the reviewer inside the `## Context` json.

So both sides are structural: the instruction paragraph comes from the prompt
file, the scope declaration comes from `.issue.description`. Neither is
hand-copied into the other.

## Part 1 — deterministic mechanism (dry-run, `REVIEWER_AGENT=cat`)

Two synthetic `JIT_CONTEXT_FILE`s were built, identical except for the issue
description: the SCOPED one declares `Audit scope: docs/tutorials/ only.`, the
UNSCOPED one declares no scope. Each was run through the real assembly path with
`REVIEWER_AGENT=cat`, which echoes the exact bytes the reviewer would receive.

Greps over the assembled reviewer input:

| context  | `### Scoped audits` paragraph present | scope line `Audit scope: docs/tutorials/ only.` present |
|----------|:-------------------------------------:|:-------------------------------------------------------:|
| SCOPED   | 1                                     | 1                                                       |
| UNSCOPED | 1                                     | 0                                                       |

The paragraph is present in both (governing instruction always shipped). The
scope line is present only in the scoped assembly. In the unscoped assembly the
same paragraph governs via its final sentence ("When the description declares no
scope, review the full surface").

Salient excerpts from the SCOPED assembled input:

Instruction paragraph (from the prompt region):

```
### Scoped audits

When the context issue's description declares an audited scope — an explicit
list of files or directories under review — restrict ALL findings to that scope.
Files outside the declared scope are out of bounds even when they drift; report
only defects whose location falls inside the declared footprint. When the
description declares no scope, review the full surface enumerated above.
```

Scope declaration reaching the reviewer inside the injected `## Context` block
(the `del(.prompt)` json from `ai-review.sh:66`):

```
## Context

```json
{"schema_version":1,"issue":{"id":"synthetic-scoped","title":"Scoped doc audit demo","description":"Audit scope: docs/tutorials/ only.\n\nReview the tutorials for drift against the source tree.","state":"InProgress"},"gate":{"key":"doc-review","title":"AI Documentation Review"},"run_history":[]}
```
```

The UNSCOPED assembly's `## Context` json carries no scope line:

```
{"schema_version":1,"issue":{"id":"synthetic-unscoped","title":"Full-surface doc audit demo","description":"Review the shipped documentation surface for drift against the source tree.","state":"InProgress"},"gate":{"key":"doc-review","title":"AI Documentation Review"},"run_history":[]}
```

## Part 2 — behavioral corroboration (one real scoped run)

A real review was run against the current worktree via the repo's configured
reviewer (`REVIEWER_AGENT="codex exec"`, per `.jit/gates.toml`), on the SCOPED
synthetic context above (`description` scope = `docs/tutorials/`).

The tree carries a KNOWN real defect OUTSIDE that scope:
`docs/reference/cli-commands.md:938` cites `.jit/claims.jsonl`, but no such file
exists — advisory work leases live under `.git/jit/`, not `.jit/`. A full-surface
review would report it; a scope-honoring review must not.

Result: the reviewer confined every finding to `docs/tutorials/` and did NOT
report the out-of-scope `.jit/claims.jsonl` defect. Verified over the run output:

- mentions of `claims.jsonl` / `cli-commands.md` / `docs/reference`: 0
- distinct finding file paths in the machine-readable findings block:
  `docs/tutorials/first-workflow.md`, `docs/tutorials/parallel-work-worktrees.md`,
  `docs/tutorials/quickstart.md` — all inside the declared footprint.

Codex session (full transcript): `019f4e24-6d25-7ba3-a31e-25db8149acb0`.

Verbatim reviewer output (stdout, as produced by `codex exec`):

```
# Documentation Review — `docs/tutorials/`

Scope was restricted to `docs/tutorials/`. Files outside that directory were used only to verify claims.

## 1. Claims match the current source tree

Blocking findings:

- `docs/tutorials/quickstart.md:151-153` says an attempted transition to `done` with an unpassed gate “Transitions to 'gated' instead.” Current transition handling rejects the operation and leaves the issue unchanged. Remediation: state that the command fails because the gate is unpassed, then evaluate the gate and retry completion.
- `docs/tutorials/first-workflow.md:153-155` says newly created tasks are unavailable because their gates have not passed. Issue creation automatically places dependency-free issues in `ready`, and `query available` filters on ready, unassigned, and unblocked—not gate status. Remediation: say the three tasks are available immediately, or define the gates as prechecks and explain that they block claiming rather than availability.
- `docs/tutorials/first-workflow.md:164-180` evaluates automated gates backed by `cargo test`, but the tutorial only requires an initialized project directory and never creates a Cargo project. The automated evaluations therefore cannot reliably pass, preventing the documented workflow. Remediation: add a Rust/Cargo project prerequisite and create the named integration-test target, or use deterministic checker commands that work in the disposable tutorial project.
- `docs/tutorials/parallel-work-worktrees.md:116-119` says acquiring a lease “Prevents other agents from claiming the same issue.” The lease surface is distinct from `jit issue claim`, which performs assignment bookkeeping and does not consult the exclusive lease as described here. Remediation: say the lease prevents another `jit claim acquire` for the issue, and explicitly distinguish it from `jit issue claim`.
- `docs/tutorials/quickstart.md:18-22` says a `type:*` label is required, while the same tutorial later says labels are optional at lines 51-70. The shipped configuration automatically supplies the default `type:task` label when callers omit one. Remediation: explain that users may omit labels because creation supplies the configured default type; do not instruct users that they must provide `type:*`.

Other named commands and flags inspected in the tutorials exist in the clap surface, including `gate status-all`, worktree commands, claim renewal/eviction flags, graph depth, event filtering, and JSON creation output.

## 2. Adopter surface versus repo-local dogfood configuration

No findings. The scoped tutorials use shipped hierarchy types and namespaces or tutorial-defined gates; they do not present the identified repository-local dogfood registries as shipped defaults.

## 3. Current behavior only

No findings. The phrase “no longer blocked” in `quickstart.md:130` describes the immediate tutorial result, not legacy product behavior.

## 4. No hardcoded counts that silently rot

No findings. Numeric task and timing totals describe fixed tutorial scenarios rather than volatile product or registry totals.

## 5. Links and referenced paths resolve

No findings. Scoped Markdown links, anchors, and inline paths resolve. External URLs were syntactically valid.

## 6. Content-standards conformance

Blocking finding:

- `docs/tutorials/quickstart.md:1-11` repeats the document’s `# Quickstart` heading and metadata block. This creates two top-level titles and violates the document-heading structure. Remediation: remove the duplicate heading and metadata block at lines 7-11.

The scoped diagram uses Mermaid. Plain-text blocks in the worktree tutorial are CLI output, not prohibited ASCII diagrams. No mathematical notation requiring LaTeX was found.

## Addressable-item citations

No findings. There are no `@/<kind>/<self-id>` citations in the scoped tutorials.

## Complete finding enumeration

1. **F1 — High:** Quickstart incorrectly says a blocked `done` transition changes the issue to `gated` (`docs/tutorials/quickstart.md:151`).
2. **F2 — High:** First Workflow incorrectly says pending postcheck gates make newly created tasks unavailable (`docs/tutorials/first-workflow.md:153`).
3. **F3 — High:** First Workflow’s automated `cargo test` gates cannot reliably pass under its stated prerequisites (`docs/tutorials/first-workflow.md:24`).
4. **F4 — High:** Worktree tutorial conflates exclusive work leases with the separate `jit issue claim` assignment operation (`docs/tutorials/parallel-work-worktrees.md:116`).
5. **F5 — Medium:** Quickstart contradicts shipped default-type behavior by calling `type:*` labels required while later calling labels optional (`docs/tutorials/quickstart.md:21`).
6. **F6 — Medium:** Quickstart contains a duplicate top-level heading and metadata block (`docs/tutorials/quickstart.md:7`).

Total findings: 6

VERDICT: FAIL
```

Every reported location falls inside `docs/tutorials/`. The known out-of-scope
defect at `docs/reference/cli-commands.md:938` is absent — the reviewer honored
the declared footprint, which is exactly the REQ-01 scoped behavior.

## Conclusion

- Scoped context → findings confined to the declared footprint (Part 2, a real
  `codex exec` run; corroborated by the Part 1 trace showing the scope line
  reaching the reviewer).
- Unscoped context → the same instruction paragraph governs with its full-surface
  fallback, no footprint line injected (Part 1).

Both branches of REQ-01 are demonstrated. `scripts/doc-review-prompt.md:27-29`
is unchanged by this evidence work.
