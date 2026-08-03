# How-To: Custom Gates

> **Diátaxis Type:** How-To Guide

Quality gates enforce process requirements before issues can be completed. This guide shows how to define and use custom gates for your workflow.

## Create Your First Gate

### Manual Gate (Simple)

Manual gates are reminders that require human judgment:

```bash
# Define a code review gate
jit gate define code-review \
  --title "Code Review" \
  --description "Code must be reviewed by another developer" \
  --stage postcheck \
  --mode manual

# Add to an issue
jit gate add $ISSUE code-review

# Later, mark as passed
jit gate evaluate $ISSUE code-review --by "human:reviewer"
```

**Use manual gates for:**
- Code reviews
- Design approvals
- Security audits
- Documentation review

### Automated Gate (With Checker)

Automated gates run scripts to verify conditions:

```bash
# Define a test gate with automated checker
jit gate define tests \
  --title "All Tests Pass" \
  --description "Full test suite must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

# Add to an issue
jit gate add $ISSUE tests

# Run the checker
jit gate evaluate $ISSUE tests
# ✓ Gate 'tests' passed

# Inspect the most recent recorded gate runs
jit gate status-all $ISSUE
```

**Use automated gates for:**
- Running tests
- Linters (clippy, eslint, etc.)
- Build verification
- Security scans

## Manual vs Automated Gates

### When to Use Manual Gates

**Appropriate for:**
- Subjective quality checks (code review, design approval)
- Human judgment required (security review, UX evaluation)
- External dependencies (stakeholder sign-off)
- Process reminders (TDD: write tests first)

**Example: TDD Reminder**
```bash
jit gate define tdd-reminder \
  --title "TDD Reminder" \
  --description "Write tests before implementation" \
  --stage precheck \
  --mode manual

# Reminds developers to write tests first
# No automation - relies on process discipline
```

### When to Use Automated Gates

**Appropriate for:**
- Objective, programmatic checks (tests pass, code compiles)
- Repeatable verification (lint rules, formatting)
- Fast feedback loops (under 5 minutes)
- CI/CD integration

**Example: Clippy Linter**
```bash
jit gate define clippy \
  --title "Clippy Lints Pass" \
  --description "No clippy warnings" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy --all-targets -- -D warnings" \
  --timeout 120
```

### Combining Both

Most workflows use both manual and automated gates:

```bash
# Automated quality checks
jit gate add $ISSUE tests clippy fmt

# Manual process gate
jit gate add $ISSUE code-review

# Nothing runs on its own. When the work is done, evaluate each gate: the
# automated ones run their checker, the manual one records an attestation.
jit gate evaluate $ISSUE tests
jit gate evaluate $ISSUE clippy
jit gate evaluate $ISSUE fmt
jit gate evaluate $ISSUE code-review --by human:alice
```

## Declare the Files a Checker Reads

A gate may declare the repository files its checker reads, as roots with glob
exclusion patterns, in a `[gates.inputs]` table beside `[gates.checker]` in
`.jit/gates.toml`:

```toml
[[gates]]
key         = "tests"
title       = "All Tests Pass"
description = "Run the test suite"
stage       = "postcheck"
mode        = "auto"

[gates.checker]
type            = "exec"
command         = "cargo test --workspace"
timeout_seconds = 300

[gates.inputs]
roots   = ["Cargo.toml", "Cargo.lock", "crates"]
exclude = ["crates/*/fixtures/**"]
```

A root is a repository-relative path: a file, or a directory standing for
everything beneath it. An exclusion pattern removes paths a root would
otherwise claim — `*` stays inside one path segment, `**` spans segments, and
the match is against the whole repository-relative path. Both are validated
when the registry is parsed, so an absolute root or an uncompilable pattern is
a registry error rather than a surprise at evaluation time. `roots` must name
at least one path.

### What Declaring Inputs Does

Evaluating a gate that declares inputs first digests the content of every file
beneath a declared root that no exclusion matches. A file present beneath a
declared root is in the digest unless a declared pattern removes it: an
uncommitted source file changes what a compiler reads, so it changes the digest,
and so does a file your `.gitignore` names. It is a digest of content, not of
modification times: a checkout that rewrites timestamps without changing bytes
produces the same value.

**The ignore rules are not consulted.** What a project declines to version is a
different question from what a checker reads, and checkers routinely read
ignored material — an installed `node_modules` tree decides what a test suite
runs against. Dropping such a path would leave the digest still while the
checker's inputs moved, which is the one failure this mechanism must not have.
Generated material your checker does not read therefore leaves the set by being
declared out, where a reader can see it:

```toml
[gates.inputs]
roots   = ["web"]
exclude = ["web/node_modules", "web/dist"]
```

Naming a directory excludes everything beneath it, and the walk skips it whole
rather than descending — so taking a large generated tree out of a gate's inputs
costs one line and no traversal. A pattern that reaches only into a directory
(`web/build/**`) still removes the paths it matches, but the walk descends to
find them.

Two things are never in the digest and need no pattern. The `.git` directory is
where content is versioned rather than content a checker reads. And a checker's
own build output is not an input at all — it is a function of the inputs,
produced after the digest is taken — so leave it out of `roots` rather than
excluding it.

The digest is bound to the gate's own declaration as well as to that content,
so editing the checker — its command, timeout, working directory, environment,
or prompt — or editing the input declaration itself yields a different digest
over identical files.

When a prior run of that same gate already recorded the same digest, the
evaluation takes that run's verdict — pass or fail — and records it without
executing the checker. This is what lets a batch of issues sitting on one
unchanged tree pay for a whole-tree checker once instead of once per issue.

A reused verdict stays distinguishable from an independently derived one. Its
run record names the run it was taken from, and `jit gate status <issue> <gate>
--json` reports the distinction:

```json
{
  "run_id": "e4c1…",
  "key": "tests",
  "status": "passed",
  "inputs_digest": "9a3f…",
  "origin": { "derivation": "reused", "source_run": "7b2b…" }
}
```

A run that executed its own checker reports `{"derivation": "executed"}`. The
report text and findings behind a reused verdict live at the named source run.
Reuse is not an exemption: the verdict was produced by a real checker execution
over an input set the digest proves identical.

### When Not to Declare Inputs

**A gate that declares no inputs executes its checker on every evaluation.**
That is the right declaration — the absence of one — whenever the verdict is
not a function of repository content alone:

- a checker scoped to one issue (a review of that issue's own commits) returns
  a different verdict per issue over one identical tree;
- a checker that consults the clock, the network, or machine state (a
  vulnerability audit against a remote advisory database) is not a function of
  the repository at all;
- a checker that reads files the repository rewrites on every evaluation.

Declare roots that cover everything the checker reads, and exclusions that cover
only what it does not. A checker that reads a file no declared root covers — or
one a declared pattern excludes — can reuse a verdict that the file's change
would have overturned. Where excluding a large installed tree is the right
trade, prefer excluding it alongside a lockfile that stays in the digest: the
lockfile moves when the installed versions do.

`jit gate evaluate <issue> <gate> --force` executes the checker regardless of
any reusable verdict.

## Environment Variables

Every gate checker receives these environment variables:

| Variable | Description |
|----------|-------------|
| `JIT_ISSUE_ID` | Full issue ID being checked |
| `JIT_GATE_KEY` | Gate key (e.g., `tests`) |
| `JIT_STAGE` | `precheck` or `postcheck` |
| `JIT_ISSUE_DOCS` | JSON array of the issue's linked documents (see below) |
| `JIT_CONTEXT_FILE` | Path to context JSON (only when `--pass-context` is set) |

### `JIT_ISSUE_DOCS`

Every gate checker receives `JIT_ISSUE_DOCS`, a JSON array describing the
issue's linked documents (the same set `jit doc list <issue>` shows) — so a
checker script can inline a plan, design doc, or amendment into its review
prompt instead of relying on the description alone.

Each element has three fields, all string or `null`:

```json
[
  { "path": "dev/active/my-plan.md", "doc_type": "design", "label": "Implementation Plan" },
  { "path": "NOTES.md", "doc_type": null, "label": null }
]
```

| Field | Description |
|-------|-------------|
| `path` | Document path, relative to the repository root |
| `doc_type` | Document type hint (e.g. `design`, `implementation`, `notes`), or `null` if unset |
| `label` | Human-readable label, or `null` if unset |

When the issue has no linked documents, `JIT_ISSUE_DOCS` is still set, to the
empty array `[]` — never absent or unset — so a checker can parse it
unconditionally:

```bash
#!/bin/bash
# Inline every linked document's path into a review prompt
echo "$JIT_ISSUE_DOCS" | jq -r '.[].path'
```

Doc selection or filtering (e.g. only `doc_type == "design"`) is checker
policy — `JIT_ISSUE_DOCS` lists every linked document unfiltered.

Custom environment variables can be set with `--env` when defining a gate:

```bash
jit gate define ai-review \
  --title "AI Review" \
  --description "AI-powered code review" \
  --mode auto \
  --checker-command "./contrib/gates/ai-review.sh" \
  --env REVIEWER_AGENT="your-reviewer-command"
```

These are passed to the checker process alongside the built-in variables. Use `--env` multiple times to set several variables. Review wrappers only prompt the configured command; choose a tool that reads the prompt from standard input, writes its review to standard output, and supports an inspection-only mode.

```bash
#!/bin/bash
# Checker scripts can use these variables
echo "Checking gate $JIT_GATE_KEY for issue $JIT_ISSUE_ID"
echo "Custom env: REVIEWER_AGENT=$REVIEWER_AGENT"
```

## Write Gate Checker Scripts

`exec`-type automated gates execute shell commands. Native in-process checker types
need no script; see [Portable checker types](../reference/gate-presets.md#portable-checker-types).
Follow these patterns when writing an `exec` checker.

### Exit Codes

Gates use standard exit codes:

- **0** - Gate passed
- **Non-zero** - Gate failed

```bash
#!/bin/bash
# Example checker script

# Run tests
if cargo test --quiet; then
  echo "✓ All tests passed"
  exit 0
else
  echo "✗ Tests failed"
  exit 1
fi
```

### Structured Findings (Machine-Readable Output)

By default a checker's stdout is stored and shown as one freetext blob. A
checker can *additionally* emit its verdict and individual findings as
structured data by appending a **findings block** to stdout. jit parses the
block once, when it records the run, and surfaces it as data across the gate
views (`gate status`, `gate status --all`, `gate status-all`, and the
gate-blocked transition error), while keeping the raw stdout available
alongside.

This is an **opt-in contract**: a checker that emits no block has its raw stdout
recorded and exposes no structured finding fields.

**The block.** Two line-exact fence markers wrap a single JSON object:

```
<<<JIT-FINDINGS-JSON
{"verdict":"fail","summary":"2 issues found","findings":[
  {"id":"F1","severity":"high","summary":"non-atomic write","file":"src/x.rs","line":42,"references":["@/inv/atomic-writes"]},
  {"id":"F2","severity":"low","summary":"prefer iterator combinator"}
]}
JIT-FINDINGS-JSON>>>
```

**Schema of the JSON object:**

| Field | Type | Notes |
|-------|------|-------|
| `verdict` | string | Checker-declared verdict, typically `"pass"` / `"fail"`. Any string is accepted. |
| `summary` | string | One-line summary of the run. |
| `findings` | array | Zero or more findings (below). |

**Each finding:**

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | Finding identifier, e.g. `"F1"`. |
| `severity` | string | Checker-defined, e.g. `"high"` / `"medium"` / `"low"`. |
| `summary` | string | One-line description. |
| `file` | string, optional | Path the finding refers to. Omit when not location-specific. |
| `line` | integer, optional | Line within `file`. |
| `references` | array of strings, optional | Opaque policy identifiers that govern the finding. Omit it or use an empty array when none apply. |

**Rules:**

- The markers must each be **alone on their own line** (surrounding whitespace
  is trimmed). The JSON payload between them may span multiple lines, and the
  whole block may sit inside a markdown code fence — the fence lines are
  ignored.
- If several complete blocks appear, the **last** one wins. Emit the real block
  at the very end of your report so an example quoted earlier never shadows it.
- **Graceful degradation:** no block, a begin marker with no matching end, or
  malformed JSON inside the fence all resolve to *no structured findings* — the
  run is recorded normally with the field simply absent. A malformed block is
  never an error, so a typo in your JSON silently drops the structured view
  rather than failing the gate; validate your JSON while developing a checker.
- Missing optional fields inside a well-formed block default to empty strings
  (`id`, `severity`, `summary`) rather than rejecting the whole block.
- The block does **not** change the exit-code contract. The verdict inside the
  block is advisory metadata; the gate's pass/fail is still decided by the
  process exit code.

**Minimal conforming checker:**

```bash
#!/bin/bash
# Emit a findings block, then fail the gate.
cat <<'BLOCK'
<<<JIT-FINDINGS-JSON
{"verdict":"fail","summary":"1 issue","findings":[{"id":"F1","severity":"high","summary":"bug","file":"src/x.rs","line":10}]}
JIT-FINDINGS-JSON>>>
BLOCK
exit 1
```

Inspect the parsed findings with:

```bash
jit gate status <ISSUE_ID> <GATE_KEY> --findings          # findings + verdict, one per line
jit gate status <ISSUE_ID> <GATE_KEY> --findings --json   # structured JSON
```

The bundled `contrib/gates/ai-review.sh` is a conforming checker: it instructs
the review agent to append this block after the human-readable findings list.

### Ground a Repository Review in Canonical Policy

Keep transport and repository policy separate. The `ai-review.sh` checker is a
tool-agnostic wrapper. The reviewer emits human-readable findings, a structured
findings block, and a terminal verdict. The wrapper transports the prompt and
context and determines its checker exit code from the terminal verdict. jit
parses and persists the structured findings block. A repository-specific prompt
owns the inspection procedure but should not copy an engineering rubric or
registry statement.

For each affected path, that prompt should direct the reviewer to load every
applicable `AGENTS.md` from the repository root toward the path. The closer file
may specialize broader guidance. The reviewer can then collect relevant
qualified IDs from those instructions, the issue and its relationship labels,
linked documents, attributable changes, and directly implicated behavior. Keep
discovery bounded to that impact; do not sweep the whole item registry.

Resolve each collected item and follow its configured source of truth before
using it in a judgment. For example, a registry-first invariant is read from
its registry, while an issue-scoped markdown-first requirement is read from its
issue section. A rendered projection helps detect drift but does not replace
the canonical source.

When a resolved item governs a finding, put its qualified ID in the finding's
optional `references` array. This preserves addressable traceability without
making the generic findings model interpret repository policy. Existing
checkers and stored findings remain compatible because `references` may be
absent or empty.

### Best Practices

**1. Make checkers fast** (target: under 5 minutes)
```bash
# Good: Focused test subset
cargo test --lib

# Avoid: Slow integration tests in gate
# cargo test --all  # Too slow for quick feedback
```

**2. Provide clear output**
```bash
# Good: Specific error message
echo "✗ Clippy found 3 warnings in src/main.rs"

# Avoid: Generic failure
echo "Failed"
```

**3. Use working directory option for multi-crate repos**
```bash
jit gate define backend-tests \
  --title "Backend Tests" \
  --description "Backend test suite" \
  --mode auto \
  --checker-command "cargo test" \
  --working-dir "crates/backend"
```

**4. Set appropriate timeouts**
```bash
# Fast checks: 60-120 seconds
--timeout 60   # Linters, formatters

# Test suites: 300-600 seconds
--timeout 300  # Unit tests
--timeout 600  # Integration tests
```

### Example: Multi-Step Checker

```bash
#!/bin/bash
# scripts/quality-gate.sh - Composite checker

set -e  # Exit on first error

echo "Running quality checks..."

# Step 1: Format check
echo "1/3 Checking formatting..."
cargo fmt --check

# Step 2: Linter
echo "2/3 Running clippy..."
cargo clippy --all-targets -- -D warnings

# Step 3: Tests
echo "3/3 Running tests..."
cargo test --lib

echo "✓ All quality checks passed"
exit 0
```

Register the script as a gate:
```bash
jit gate define quality \
  --title "Quality Checks" \
  --description "Format, lint, and test" \
  --mode auto \
  --checker-command "./scripts/quality-gate.sh" \
  --timeout 300
```

## Context-Aware Gates

Standard checkers are stateless — they run a command and check the exit code. Context-aware gates receive structured data about the issue, gate definition, prompt instructions, and previous run history.

### Enabling Context

Add `--pass-context` when defining a gate:

```bash
jit gate define review \
  --title "Code Review" \
  --description "AI-powered code review" \
  --mode auto \
  --pass-context \
  --prompt "Review the implementation for correctness and style." \
  --checker-command "./contrib/gates/ai-review.sh"
```

The checker receives a `JIT_CONTEXT_FILE` env var pointing to a JSON file:

```json
{
  "schema_version": 1,
  "prompt": "Review the implementation for correctness and style.",
  "issue": {
    "id": "...", "title": "...", "description": "...",
    "state": "in_progress", "priority": "high",
    "documents": [], "labels": [], "gates": [],
    "dependencies": [
      { "id": "...", "title": "Setup database schema", "state": "done", "priority": "high" },
      { "id": "...", "title": "Implement auth module", "state": "in_progress", "priority": "medium" }
    ]
  },
  "gate": {
    "key": "review", "title": "Code Review",
    "description": "AI-powered code review", "stage": "postcheck"
  },
  "run_history": []
}
```

The `dependencies` array contains enriched summaries of upstream issues — their current state, title, and priority — so context-aware gates can reason about prerequisite work.

### Prompt Files

Use `--prompt-file` for version-controlled prompts:

```bash
jit gate define review \
  --title "Code Review" \
  --description "AI review" \
  --mode auto \
  --pass-context \
  --prompt-file "docs/review-prompt.md" \
  --checker-command "./contrib/gates/ai-review.sh"
```

`--prompt-file` takes precedence over `--prompt`. The file is read at check time, so updates take effect without redefining the gate.

### Run History

Each subsequent run includes at most the latest result for the same issue and gate in `run_history`. This compact history enables iterative workflows without recursively injecting several full review narratives:

The `issue.gates` projection contains the other required gates' latest evidence. It omits the gate currently being evaluated because that recorded status necessarily predates the in-flight run; use the top-level `gate` object for the current definition and `run_history` for its prior result.

```bash
# First run: run_history is empty
jit gate evaluate $ISSUE review

# Second run: run_history holds the first run's metadata and exit code. When
# structured findings exist, they are retained while stdout and stderr are
# omitted. Legacy unstructured runs retain stdout; stderr is always omitted.
jit gate evaluate $ISSUE review
```

### Example: AI Review Script

A production-ready AI review script is provided in `contrib/gates/ai-review.sh`. It pipes the gate context into an AI agent CLI (set via `REVIEWER_AGENT`) and parses a `VERDICT: PASS` / `VERDICT: FAIL` from the output.

Applying a profile that packages it installs it at that path, executable. From
a source checkout, copy it to `contrib/gates/ai-review.sh` in your own
repository and make it executable.

```bash
# Define the gate with --env to set the reviewer agent
jit gate define ai-review \
  --title "AI Code Review" \
  --description "AI-powered code review" \
  --mode auto --stage postcheck \
  --pass-context \
  --prompt-file "contrib/gates/prompts/code-review.md" \
  --checker-command "./contrib/gates/ai-review.sh" \
  --env REVIEWER_AGENT="your-reviewer-command" \
  --timeout 120
```

`REVIEWER_AGENT` is tool-agnostic: configure any prompt-consuming reviewer in its inspection-only mode. The wrapper supplies the prompt and context and determines the checker exit code from the terminal verdict; jit parses and persists the structured findings block from the checker output. Put repository policy discovery and judgment in the repository-specific prompt, not in this shared wrapper.

### Prompt Library

Ready-to-use prompt templates are provided in `contrib/gates/prompts/`:

| Prompt | Description |
|--------|-------------|
| `code-review.md` | General review — correctness, style, error handling, simplicity |
| `security-audit.md` | OWASP Top 10 checklist with severity ratings |
| `test-adequacy.md` | Test coverage vs requirements, edge cases, naming conventions |

Reference them with `--prompt-file`:

```bash
jit gate define security-audit \
  --title "Security Audit" \
  --description "OWASP Top 10 security check" \
  --mode auto --pass-context \
  --prompt-file "contrib/gates/prompts/security-audit.md" \
  --checker-command "./contrib/gates/ai-review.sh" \
  --env REVIEWER_AGENT="your-reviewer-command"
```

The prompts reference the context JSON structure (issue, gate, documents, dependencies, run_history) and end with the verdict format. Customize them or use them as starting points for your own.

## Prechecks vs Postchecks

Gates can run at two stages in the workflow:

### Prechecks (Before Work Begins)

Run when issue transitions **to** `in_progress` state.

**Purpose:** Ensure prerequisites are met before starting work.

**Use for:**
- TDD reminders (write tests first)
- Design approval required
- Prerequisites verified (dependencies installed, environment configured)

**Example: TDD Precheck**
```bash
jit gate define tdd-precheck \
  --title "TDD: Tests Exist" \
  --description "Verify test file exists before implementation" \
  --stage precheck \
  --mode auto \
  --checker-command "test -f tests/feature_test.rs"
```

**Workflow:**
```bash
# Issue requires TDD precheck
jit issue claim $ISSUE agent:me

# Precheck runs automatically
# If it fails: claim returns a blocker and the issue remains ready
# If passes: Issue transitions to in_progress
```

### Postchecks (After Work Completes)

Must pass before an issue can complete as `done`.

**Purpose:** Verify work quality before completion.

**Use for:**
- Tests pass
- Code review complete
- Documentation updated
- Build succeeds

**Example: Test Postcheck**
```bash
jit gate define tests \
  --title "All Tests Pass" \
  --description "Test suite must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib"
```

**Workflow:**
```bash
# Attempt to finish; unpassed gates move the issue to gated
jit issue update $ISSUE --state done

# Run the required automated postcheck and inspect the result
jit gate evaluate $ISSUE tests
jit gate status-all $ISSUE

# Once all required statuses pass, request completion again
jit issue update $ISSUE --state done
```

### Choosing Stage

| Gate Type | Stage | Reason |
|-----------|-------|--------|
| TDD reminder | Precheck | Ensure tests written before code |
| Design approval | Precheck | Validate approach before implementation |
| Tests pass | Postcheck | Verify implementation works |
| Code review | Postcheck | Quality check after completion |
| Linter | Postcheck | Enforce style after writing |
| Security scan | Postcheck | Verify no vulnerabilities introduced |

## Gate Presets and Templates

Gate presets are pre-configured bundles of quality gates that dramatically reduce setup time. Instead of defining and adding gates individually, apply entire workflows in seconds.

### Where Presets Come From

Every preset is declared by the project that uses it, under
`.jit/config/gate-presets/`. Gate keys, titles, and checkers (a test runner, a
linter, a formatter, a security audit) are domain vocabulary, so the bundles that
carry them belong to the repository rather than to the binary.
[Gate Presets](../reference/gate-presets.md) states the preset contract and the
portable checker syntax a bundled gate can use.

A `plan`-style graph template gates its planning and breakdown nodes by name,
resolving each name against the project's presets and then its gate registry.
The template itself is optional, project-declared configuration: `jit init` does
not scaffold `templates.toml`. A project that wants the
[planning bracket](../concepts/planning-bracket.md) declares its own `plan`
template in `.jit/templates.toml`, along with the gate keys that template names
(this repository does so).

**List available presets:**
```bash
jit gate preset list
```

Each line names one preset with its description and gate count. To inspect the
gates a preset carries, run `jit gate preset show <name>`.

### Declaring a Project Preset

Capture a repeated gate set — a Rust CI bundle, a security review, your team's
standard workflow — as a project-defined preset stored in
`.jit/config/gate-presets/`. Configure one reference issue with the gates you
want, then capture them:

**Step 1: Define the gates and add them to a reference issue**
```bash
jit gate define tests \
  --title "All tests pass" --description "cargo test must pass" \
  --stage postcheck --mode auto \
  --checker-command "cargo test" --timeout 300

jit gate define clippy \
  --title "Clippy lints pass" --description "No clippy warnings" \
  --stage postcheck --mode auto \
  --checker-command "cargo clippy --all-targets -- -D warnings" --timeout 120

jit gate define fmt \
  --title "Code formatted" --description "cargo fmt --check must pass" \
  --stage postcheck --mode auto \
  --checker-command "cargo fmt --check" --timeout 30

jit gate define code-review \
  --title "Code review completed" --description "Another developer reviewed the code"

jit issue create --title "Reference issue"
jit gate add abc123 tests clippy fmt code-review
```

**Step 2: Capture the issue's gates as a preset**
```bash
jit gate preset create abc123 rust-ci
```

This writes the bundle to `.jit/config/gate-presets/<name>.json` (here
`rust-ci.json`) with those four gates.
Commit the JSON file to share the preset with your team.

**View preset details:**
```bash
jit gate preset show rust-ci
```

The output names the preset and its description, then each bundled gate with its
key, title, stage, mode, and — for automated gates — the checker command and
timeout.

### Applying a Preset

```bash
# Create issue
jit issue create --title "Implement user authentication"

# Apply the rust-ci preset (adds its gates in one command)
jit gate preset apply rust-ci def456
```

The issue now requires every gate the preset bundles, and each of those gates is
defined in `.jit/gates.toml` — `apply` inserts the definition for any key the
registry does not already carry.

### Customizing Preset Application

Filter which gates to apply using command options:

**Skip precheck gates:**
```bash
# Apply only the preset's postcheck gates
jit gate preset apply rust-ci def456 --no-precheck
```

**Skip postcheck gates:**
```bash
# Apply only the preset's precheck gates
jit gate preset apply rust-ci def456 --no-postcheck
```

**Exclude specific gates:**
```bash
# Skip clippy if not using the linter
jit gate preset apply rust-ci def456 --except clippy

# Skip multiple gates
jit gate preset apply rust-ci def456 --except clippy --except fmt
# Applies every other gate the preset bundles
```

**Override timeouts:**
```bash
# Increase timeout for slow CI
jit gate preset apply rust-ci def456 --timeout 600
# Every automated gate in the preset gets a 600s timeout instead of its own
```

**Combine filters:**
```bash
# Hotfix workflow: no precheck, no linter, fast timeout
jit gate preset apply rust-ci def456 --no-precheck --except clippy --timeout 60
# Applies the preset's postcheck gates except clippy, each with a 60s timeout
```

### Batch Operations

Apply presets to multiple issues at once:

**Multiple issues directly:**
```bash
jit gate preset apply rust-ci abc123 def456 ghi789
# Applies to all three issues
```

**From query results:**
```bash
# Apply to all issues in an epic
jit query all --label "epic:auth" --json | jq -r '.issues[].id' | xargs jit gate preset apply rust-ci

# Apply to all ready issues
jit query available --json | jq -r '.issues[].id' | xargs jit gate preset apply rust-ci
```

### Evolving a Preset

Refine a captured preset by reconfiguring a reference issue and re-capturing under
a new name:

**Step 1: Configure one issue perfectly**
```bash
# Apply an existing preset and customize
jit gate preset apply rust-ci abc123 --except fmt
jit gate add abc123 security-scan
```

**Step 2: Save as a new preset**
```bash
jit gate preset create abc123 team-standard
```

**Output:**
```
Created preset 'team-standard' at .jit/config/gate-presets/team-standard.json
```

**Step 3: Use everywhere**
```bash
# List shows the captured preset
jit gate preset list
# team-standard - Custom preset created from issue abc123 (5 gates)

# Apply to any issue
jit gate preset apply team-standard def456
```

### Custom Preset Storage

Custom presets are stored as JSON files in `.jit/config/gate-presets/`:

```bash
# View custom preset file
cat .jit/config/gate-presets/team-standard.json
```

```json
{
  "name": "team-standard",
  "description": "Custom preset created from issue abc123",
  "gates": [
    {
      "key": "tests",
      "title": "All tests pass",
      "description": "cargo test must pass",
      "stage": "postcheck",
      "mode": "auto",
      "checker": {
        "type": "exec",
        "command": "cargo test",
        "timeout_seconds": 300,
        "working_dir": null,
        "env": {}
      }
    },
    ...
  ]
}
```

**Managing custom presets:**
- Edit JSON files directly for fine-tuning
- Delete files to remove presets
- Share files with team via git

### Practical Workflows

**Quick Start New Issue:**
```bash
jit issue create --title "New feature"
jit gate preset apply rust-ci $ISSUE_ID
# Ready to work with full quality pipeline
```

**Team Onboarding:**
```bash
# Document team standards
jit gate preset create reference-issue team-workflow

# Team members apply to their issues
jit gate preset apply team-workflow their-issue
# Instant consistency across team
```

**Different Requirements by Type:**
```bash
# Full workflow for features
jit gate preset apply rust-ci feature-issue

# Lighter workflow for docs
jit gate preset apply docs-ci doc-issue --except code-review
jit gate add doc-issue spell-check

# Custom for infrastructure
jit gate preset apply team-infra infra-issue
```

### Comparing with Manual Gate Definitions

Presets and manual definitions reach the same gate set. Prefer presets for
consistency and speed; reach for manual definitions when a gate has no preset.

**Manual approach:**
```bash
# Define each gate individually
jit gate define tests \
  --title "All Tests Pass" \
  --description "cargo test must pass" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo test --lib" \
  --timeout 300

jit gate define clippy \
  --title "Clippy Clean" \
  --description "No clippy warnings" \
  --stage postcheck \
  --mode auto \
  --checker-command "cargo clippy --all-targets -- -D warnings" \
  --timeout 120

# ... repeat for each gate

# Add to each issue
jit gate add $ISSUE tests clippy fmt code-review
```

**Preset approach:**
```bash
# One command
jit gate preset apply rust-ci $ISSUE
```

**Benefits:**
- **Fewer commands**: one preset apply instead of a separate define and add per gate
- **No mistakes**: Preset definitions are captured once from a working issue and reused
- **Consistent**: Same gates on every issue
- **Shareable**: Team uses identical workflows
- **Customizable**: Filter options for special cases

## Common Workflows

For complete workflow examples including TDD and CI/CD integration, see [Software Development](software-development.md).

### Workflow 1: Code Quality Pipeline

```bash
# Define quality gates
jit gate define fmt --mode auto --stage postcheck --checker-command "cargo fmt --check"
jit gate define clippy --mode auto --stage postcheck --checker-command "cargo clippy -- -D warnings"
jit gate define tests --mode auto --stage postcheck --checker-command "cargo test"

# Apply to all issues in epic (batch mode still uses --add-gate)
jit issue update --filter "label:epic:auth" --add-gate fmt --add-gate clippy --add-gate tests

# Developer evaluates the required automated gates
jit gate evaluate $ISSUE fmt
jit gate evaluate $ISSUE clippy
jit gate evaluate $ISSUE tests

# Inspect gate status, then complete only after every required status passes
jit gate status-all $ISSUE
jit issue update $ISSUE --state done
```

### Workflow 3: Manual + Automated Review

```bash
# Automated checks
jit gate add $ISSUE tests clippy

# Manual review
jit gate add $ISSUE code-review

# Attempt completion; the unpassed gates move the issue to gated
jit issue update $ISSUE --state done

# Run automated gates and record manual review
jit gate evaluate $ISSUE tests
jit gate evaluate $ISSUE clippy
jit gate evaluate $ISSUE code-review --by "human:alice"

# Manual approval may complete a gated issue; otherwise retry explicitly after
# status-all reports every required gate as passed.
jit gate status-all $ISSUE
jit issue update $ISSUE --state done
```

## Troubleshooting Gate Failures

### Common Issues and Solutions

#### "Repository not initialized"

```bash
Error: .jit directory not found
```

**Solution:** Run `jit init` in your project directory first.

#### "Cycle detected"

```bash
Error: Adding dependency would create a cycle
```

**Solution:** Check your dependency graph with `jit graph export --format dot` or `jit graph deps --depth 0` and remove circular references. Dependencies must form a directed acyclic graph (DAG).

#### "Invalid label format"

```bash
Error: Invalid label format: 'milestone-v1.0'
Expected format: 'namespace:value'
```

**Solution:** Use colon separator: `--label "milestone:v1.0"`

#### Optional type labels

A project may use `--type task` or configure `[validation].default_type` to add a
type label. The type namespace permits at most one value; a missing type is only
a problem when that project configures a default or a rule requiring it.

#### "Refusing to run gate ... this jit binary predates the tree under review"

```bash
Error: Refusing to run gate 'tests' for issue abc123: this jit binary
predates the tree under review
```

**Solution:** this fires only when BOTH hold: the repository under validation
can resolve the running binary's build commit in its own history (the
repository the binary was built from, or a clone/fork sharing that history),
AND that commit no longer matches the repository's current `HEAD` (or the
binary was built from a dirty tree) — so its verdict would not be evidence
about the change under review. Otherwise (an unrelated repository, no git, or
an unresolvable build commit) it stays silent — an installed release
validating a different repository is unaffected. Rebuild and reinstall with
`scripts/install-jit.sh` (it injects build provenance around `cargo install
--path crates/jit`, so the reinstalled binary reports the commit it was built
from), then re-run the gate. See [the `jit gate
evaluate` exit-code contract](../reference/cli-commands.md#jit-gate-evaluate)
for the full condition and how this differs when it's a checker's own child
`jit` (not the evaluator) that is stale.

#### "Orphaned task"

```bash
Warning: Issue is an orphaned task (no epic:* or milestone:* label)
```

**Solution:** Add parent label: `jit issue update $ISSUE --label "epic:auth"` or use `--orphan` flag to explicitly allow orphaned issues.

### Validation and Recovery

```bash
# Check repository health
jit validate

# Automatically fix issues
jit validate --fix

# Preview fixes without applying
jit validate --fix --dry-run
```

### Getting Help

```bash
# Command-specific help
jit issue create --help
jit dep add --help
jit gate define --help

# List available commands
jit --help

# Check configured label namespaces
jit label namespaces

# View existing label values
jit label values milestone
jit label values epic
```

## Advanced Topics

### Adapting Gates to Your Domain

Gates are domain-agnostic quality checkpoints. The examples in this guide focus on software development, but the patterns apply broadly:

**Research**: Literature review, peer review, data validation, statistical significance
**Writing**: Outline approval, editor review, spell check, word count targets, fact-checking
**Design**: Stakeholder approval, user testing, accessibility checks, brand compliance
**Operations**: Change approval, rollback plan, monitoring setup, incident review

The key insight: **any workflow with quality requirements can use gates**.

## See Also

- [Core Model - Gates](../concepts/core-model.md#gates) - Conceptual understanding
- [CLI Reference - Gate Commands](../reference/cli-commands.md#gate-commands) - Complete command syntax
- [First Workflow Tutorial](../tutorials/first-workflow.md) - Gate usage in practice
