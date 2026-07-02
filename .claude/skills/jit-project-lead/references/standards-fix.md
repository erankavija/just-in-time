# Standards fix

Applies safe, rule-based corrections to every **mechanical** finding emitted by
the standards scan (`references/standards-scan.md`). The fixer consumes the
scanner's JSONL, and for each `mechanical` finding whose rule has a single
unambiguous correction it rewrites the source issue so the flagged violation no
longer holds. Every `judgment` finding, every excluded rule, and every
issue/document with no mechanical finding is left byte-for-byte unchanged.

Runner: `scripts/standards-fix.sh [<project-root>]` (this skill's `scripts/`).
Root defaults to the current directory and must contain `.jit/`. Like the
scanner, it works against any jit project.

## Run

```
# Run the scanner internally, then fix:
scripts/standards-fix.sh > applied.jsonl

# Or consume a pre-computed findings file (or a pipe):
scripts/standards-scan.sh > findings.jsonl
scripts/standards-fix.sh --findings findings.jsonl > applied.jsonl
scripts/standards-scan.sh | scripts/standards-fix.sh > applied.jsonl

# Report the corrections without writing them:
scripts/standards-fix.sh --dry-run
```

- stdout: one JSON record per correction (schema below), applied or skipped.
- stderr: a one-line count summary; never mixed into stdout.
- exit 0 on completion; exit 2 on bad invocation (`.jit/` missing; `jq`,
  `gawk`, or `jit` absent; or an internal scan failing).

Findings precedence: an explicit `--findings <file>`, then a pipe on stdin,
otherwise the fixer runs the scanner against `<project-root>` itself.

## Storage boundary

Every issue mutation is routed through the jit CLI: `jit issue show` reads the
current title and description, `jit issue update` writes the correction. The
fixer never parses or writes `.jit/issues/*.json` directly, so INV-EVENT-LOG
and INV-ATOMIC-WRITES stay intact for issue writes. All `jit` calls run inside
the target root, so the fixer only ever touches the project it was pointed at.

`jit issue update` is invoked with `--force` so a still-imperfect issue (one
that retains untouched `judgment` findings) cannot let an `enforce` validation
rule block the mechanical correction. The bypass is logged by jit.

## What gets corrected

Each row is a mechanical rule the scanner emits. A rule is **applied** when a
single unambiguous correction exists, or **excluded** when it does not (the
finding is then reported with `action: "skipped"` and left unchanged).

| Rule | Class | Correction |
|---|---|---|
| STD-CRIT-UNMARKED | applied | Insert a `[hard]` marker and a fresh `REQ-NN` id before the criterion text. `[hard]` is the standard's default for an unmarked item (see the content standards). The bullet prefix (`- ` and an optional `[ ]`/`[x]` checkbox) is preserved. |
| STD-CRIT-REQID | applied | Keep the existing `[hard]`/`[aspirational]` marker; replace the malformed id with a fresh well-formed `REQ-NN:`. The criterion statement is preserved verbatim. |
| STD-TITLE-EMBEDDED-ID | applied | Strip a leading conventional-commit prefix (`feat(scope): `), an embedded short-id/position-code/ordinal prefix (`abc1234/S0: `, `S0/W1: `, `1. `), and any `(jit:<hex>)` token — exactly the shapes the scanner flags. If stripping would leave an empty title, the title is left unchanged and the finding is reported skipped. |
| STD-SC-MISSING | applied | Append a `## Success Criteria` heading at the end of the description. (An issue with no Success Criteria section also has no criteria, so this never collides with a criterion fix.) |
| STD-HEADING-H1 | applied | Promote the `#` heading to `##`. |
| STD-HEADING-DEEP | applied | Clamp a heading deeper than `###` back to `###`. |
| STD-ANTIPATTERN-SECTION | applied | Remove the DAG-duplicating section: the `## Depends on` / `## Dependencies` / `## Children` heading and its body, up to (not including) the next heading or end of description. The DAG is canonical, so the section carries no content worth keeping. |
| STD-LABEL-SLUG | **excluded** | No single safe correction. The offending value is an 8-hex short id; a meaningful `epic:`/`story:`/`milestone:` kebab slug names the strategic bucket and cannot be derived from a hash. Renaming needs a human-chosen slug, so the fixer never edits the label. |

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
atomic-rename pattern (INV-ATOMIC-WRITES) for files under `permanent_paths` and
live `dev/active`.

## Determinism and idempotence

A correction is a pure function of the findings and the current content: no
timestamp, hostname, or random value enters it, and ids are allocated in line
order. Running the fixer a second time produces no further change — the scanner
reports no finding on an already-fixed item, so nothing is selected. The only
records a repeat run emits are the `skipped` records for excluded rules
(e.g. STD-LABEL-SLUG), which never mutate anything.

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

- `jit`, `jq`, and `gawk` on PATH.
- Read/write access to the target `.jit/` through the jit CLI. The fixer writes
  no project file directly.

## Tests

`scripts/test-standards-fix.sh` (this skill's `scripts/`) is a self-contained
suite. It builds a temporary jit fixture (`jit init` + seeded issues and
documents), runs the scanner to produce findings, runs the fixer, then
re-scans and asserts:

- **REQ-01/02** — no fixable mechanical finding survives the fixer (every one is
  gone from the re-scan; STD-LABEL-SLUG is the only mechanical rule allowed to
  remain, by exclusion), covering each rule: the malformed-`REQ-NN` variants,
  an unmarked criterion, an embedded-id title, a missing Success Criteria
  heading, an H1 and a too-deep heading, and an anti-pattern section;
- a well-formed `REQ-NN` criterion in a partially-malformed issue is preserved
  verbatim (its number stays reserved, never reused);
- **REQ-03** — a clean issue, a judgment-only issue, a mixed issue's judgment
  line, the excluded 8-hex label, and a judgment-only `docs/` document are all
  left byte-identical, while the mixed issue's mechanical criterion is fixed;
- **REQ-04** — the fixer's report records the issue and rule for every
  correction, and the exclusion for STD-LABEL-SLUG;
- idempotence — a second fixer run applies no correction.

Run it directly; it needs `jit`, `jq`, and `gawk` on PATH:

```
.claude/skills/jit-project-lead/scripts/test-standards-fix.sh
```

Exit 0 when all assertions pass, 1 otherwise.
