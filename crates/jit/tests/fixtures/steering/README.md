# Steering scenario fixtures

Each subdirectory here is one deterministic steering scenario exercised by
`crates/jit/tests/steering_scenarios.rs`. **Adding a new scenario requires
only a new fixture directory containing a `scenario.toml`** — no Rust changes.

## `scenario.toml` schema

```toml
# Which ruleset to install from docs/examples/<ruleset>/.
# The runner copies <ruleset>/rules.toml -> .jit/rules.toml and
# <ruleset>/schemas/ -> .jit/schemas/ into a fresh temp repo.
ruleset = "sdd"

# Sequence of CLI steps to execute against the isolated repo.
# Each step is a table with:
#   argv     — jit subcommand + arguments as a string array (required).
#              The "jit" binary itself is omitted; write ["issue", "create", ...]
#   capture  — "id" (default) | "none" — "id" means extract the UUID from this
#              step's output and store it in the slot named by `id_slot`.
#              Use "none" for steps that do not create an issue.
#   id_slot  — name of the variable that receives the captured UUID (string).
#              Captured ids are referenced in later steps as "$<slot>", e.g.
#              "$epic" expands to the full UUID captured under slot "epic".
#              The short-id prefix (first 8 hex chars) is also substituted for
#              "$<slot>_short".
#   expect   — per-step assertion (optional, see below).
#              When omitted the step is still asserted to exit 0.
#
[[steps]]
argv = ["issue", "create", "--title", "My Epic", "--label", "type:epic"]
capture = "id"          # extract UUID from output
id_slot = "epic"        # store as "$epic"
# expect omitted -> exit 0 is asserted implicitly

[[steps]]
argv = ["issue", "update", "$epic", "--state", "done"]
capture = "none"
expect = { exit = 4, contains = ["sdd-hard-criteria-covered"], enforcement_point = "transition" }

# Per-step `expect` table:
#   exit              — expected process exit code (integer, required when present).
#                       When the whole `expect` block is omitted, exit 0 is asserted.
#   contains          — list of substrings that must appear in stderr+stdout combined
#   not_contains      — list of substrings that must NOT appear
#   enforcement_point — "write" | "validate" | "transition".
#                       Identifies the COMMAND CLASS of the step that carries this
#                       expect block:
#                         "write"      -> the step argv starts with "issue create"
#                                         or "issue update" (without --state)
#                         "validate"   -> the step argv starts with "validate"
#                         "transition" -> the step argv includes "issue update --state"
#                       The runner derives the command class from argv and asserts
#                       it matches this field.  It is NOT a separate failure-location
#                       oracle; it acts as documentation pinned by the exit/contains
#                       assertions on the same step.
```

## Step variable substitution

Before each step's `argv` is executed the runner substitutes:

- `$<slot>` with the full UUID captured in `id_slot = "<slot>"` by an earlier step
- `$<slot>_short` with the 8-character short-id prefix

Substitution is literal string replacement; no shell quoting is applied.

## Enforcement points

`enforcement_point` is an annotation on the step whose exit code and/or output
substrings are asserted.  It declares the command class of that specific step and
is verified by the runner against the step's argv — it is not a separate internal
failure-location oracle.

| `enforcement_point` | Command class of the step carrying the `expect` |
|---|---|
| `write` | `jit issue create` or `jit issue update` (no `--state`) — write-path local rules |
| `validate` | `jit validate` — graph rules checked at validate time |
| `transition` | `jit issue update --state <target>` — transition-time graph rules |

## Existing scenarios

| Directory | Enforcement | What it tests |
|---|---|---|
| `sloppy-spec-body/` | write | Prose without bullets in Success Criteria is blocked at create time (exit 4) |
| `typo-heading/` | write | `## Sucess Criteria` typo produces a did-you-mean hint (exit 4) |
| `stray-req/` | validate | `req:REQ-77` absent from criteria is caught by `jit validate` (exit 1) |
| `pending-req-quiet/` | validate | In-flight epic with req:REQ-01 and no children produces zero error findings (exit 0) |
| `premature-done/` | transition | Transitioning an epic to done without covered criteria is blocked (exit 4) |
| `happy-path-done/` | transition | Epic with a done child satisfying REQ-01 reaches done successfully (exit 0) |
| `bracket-coverage-gap/` | validate | The bracket's breakdown gate (`validate --scope <C>`) blocks an uncovered `[hard]` criterion (exit 4) |
| `bracket-happy-path/` | validate / transition | A well-formed bracket passes the breakdown gate (exit 0) and the spine walks to done end-to-end |
| `bracket-plan-rejected/` | transition | The bracket's plan-review gate (deterministic FAILING stand-in checker) blocks; the unpassed gate blocks P's done transition (exit 4) |

## Fresh-evidence scenario note

A `gate-recency` scenario that asserts on stale gate timestamps requires
back-dating the `updated_at` field in the gate status, which cannot be
expressed via `jit` CLI commands alone (the CLI always writes the current
timestamp). Since `scenario.toml` steps are CLI-only, a fresh-evidence
scenario cannot be expressed cleanly without file-editing primitives. It is
therefore omitted from this harness; the `gate-recency` kind is covered by the
`example_rulesets_tests.rs` unit tests that inject a fixed clock directly into
`evaluate_graph`.
