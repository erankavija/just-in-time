# Standards fix

Applies safe, rule-based corrections to every **mechanical** finding emitted by
the standards scan (`references/standards-scan.md`). The fixer consumes the
scanner's JSONL, and for each `mechanical` finding whose rule has a single
unambiguous correction it rewrites the source issue so the flagged violation no
longer holds. Every `judgment` finding (STD-LABEL-SLUG among them) and every
issue/document with no mechanical finding is left byte-for-byte unchanged.

Runner:
`.agents/skills/jit-project-lead/scripts/standards-fix.sh [<project-root>]`.
Root defaults to the current directory and must contain `.jit/`. Like the
scanner, it works against any jit project.

## Run

```
# Run the scanner internally, then fix:
.agents/skills/jit-project-lead/scripts/standards-fix.sh > applied.jsonl

# Or consume a pre-computed findings file (or a pipe):
.agents/skills/jit-project-lead/scripts/standards-scan.sh > findings.jsonl
.agents/skills/jit-project-lead/scripts/standards-fix.sh --findings findings.jsonl > applied.jsonl
.agents/skills/jit-project-lead/scripts/standards-scan.sh | .agents/skills/jit-project-lead/scripts/standards-fix.sh > applied.jsonl

# Report the corrections without writing them:
.agents/skills/jit-project-lead/scripts/standards-fix.sh --dry-run
```

- stdout: one JSON record per correction (schema below), applied or skipped.
- stderr: a one-line count summary; never mixed into stdout.
- exit 0 on completion; exit 2 on bad invocation (`.jit/` missing; `python3`,
  `gawk`, or `jit` absent; or an internal scan failing).

Findings precedence: an explicit `--findings <file>`, then a pipe on stdin,
otherwise the fixer runs the scanner against `<project-root>` itself.

## Storage boundary

Every issue mutation is routed through the jit CLI: `jit issue show` reads the
current title and description, `jit issue update` writes the correction. The
fixer never parses or writes `.jit/issues/*.json` directly, so issue writes
preserve the CLI's configured event and atomicity contracts. All `jit` calls run inside
the target root, so the fixer only ever touches the project it was pointed at.

`jit issue update` is invoked with `--force` so a still-imperfect issue (one
that retains untouched `judgment` findings) cannot let an `enforce` validation
rule block the mechanical correction. The bypass is logged by jit.

## What gets corrected

Each row is a mechanical rule the scanner emits, with the single unambiguous
correction the fixer applies. A criterion fix can, in the pathological
all-ids-reserved case, be reported `skipped` instead of applied (see *Fresh
`REQ-NN` id allocation*). STD-LABEL-SLUG is not listed here: it is a `judgment`
finding (a bucket slug cannot be derived from a hash), so the fixer never sees
it — it flows through untouched like every other judgment finding.

| Rule | Class | Correction |
|---|---|---|
| STD-CRIT-UNMARKED | applied | Insert a `[hard]` marker and a fresh `REQ-NN` id before the criterion text. `[hard]` is the standard's default for an unmarked item (see the content standards). The bullet prefix (`- ` and an optional `[ ]`/`[x]` checkbox) is preserved. |
| STD-CRIT-REQID | applied | Keep the existing `[hard]`/`[aspirational]` marker; replace the malformed id with a fresh well-formed `REQ-NN:`. The criterion statement is preserved verbatim. |
| STD-TITLE-EMBEDDED-ID | applied | Strip a leading conventional-commit prefix (`feat(scope): `), an embedded short-id/position-code/ordinal prefix (`abc1234/S0: `, `abc1234/foo`, `abc1234: `, `S0/W1: `, `1. `), and any `(jit:<hex>)` token — every shape the scanner flags, including a hex prefix followed by a bare `/` with no colon. If stripping would leave an empty title, the title is left unchanged and the finding is reported skipped. |
| STD-SC-MISSING | applied | Append a `## Success Criteria` heading at the end of the description. (An issue with no Success Criteria section also has no criteria, so this never collides with a criterion fix.) |
| STD-HEADING-H1 | applied | Promote the `#` heading to `##`. |
| STD-HEADING-DEEP | applied | Clamp a heading deeper than `###` back to `###`. |
| STD-ANTIPATTERN-SECTION | applied | Remove the DAG-duplicating section: the `## Depends on` / `## Dependencies` / `## Children` heading and its body, up to (not including) the next heading or end of description. The DAG is canonical, so the section carries no content worth keeping. |

### Fresh `REQ-NN` id allocation

A well-formed id is `REQ-` + exactly two digits. For an issue whose criteria
need fixing, the fixer collects the two-digit numbers already used by
well-formed criteria in that issue's Success Criteria section (the *reserved*
set), then walks the criteria that need a fix in ascending line order,
assigning the smallest unused two-digit number to each. This is deterministic,
collision-free, and leaves every well-formed criterion's id untouched. (In the
pathological case where all of `01`–`99` are reserved, the criterion is left
unchanged and reported skipped.)

## Documents

No mechanical rule in the scanner targets a document. The heading, criterion,
title, and label rules are all issue-only; the content rules that fire on
documents (ASCII art, plain-text math, the LaTeX-notation rules) are all
`judgment`. The fixer therefore never mutates a document — every document
finding, being `judgment` or unsupported, is left byte-identical. Were a
mechanical document rule ever added, its writer must use the temp-file +
repository-configured atomic publication pattern for files under `permanent_paths` and
live `dev/active`.

## Determinism and idempotence

A correction is a pure function of the findings and the current content: no
timestamp, hostname, or random value enters it, and ids are allocated in line
order. Running the fixer a second time produces no further change — the scanner
reports no finding on an already-fixed item, so nothing is selected. The only
records a repeat run emits are the `skipped` records for genuinely unfixable
findings (a criterion whose issue has every `REQ-NN` id reserved), which never
mutate anything.

## Output schema

Each stdout line is a JSON object:

| Field | Value |
|---|---|
| `target_kind` | `"issue"` or `"document"` |
| `target` | issue short-id (8 hex) or repo-relative document path |
| `rule` | the corrected (or skipped) rule id |
| `line` | 1-based source line, or `0` for whole-item corrections (title, label, missing section) |
| `action` | `"applied"`, `"dry-run"`, or `"skipped"` |
| `detail` | what was done, or why it was skipped |

This record set is the fix ledger: for every correction it names the issue
changed and the rule corrected (REQ-04), and for every excluded/unsupported
finding it names why nothing was done.

## Requirements

- `jit`, `python3`, `base64`, and `gawk` on PATH.
- Read/write access to the target `.jit/` through the jit CLI. The fixer writes
  no project file directly.
