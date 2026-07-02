# Standards scan

Deterministic scan of every project issue and every in-scope document against
the canonical content standards (`docs/reference/jit-content-standards.md`).
Each violation is emitted as one finding, classified `mechanical` (one fixed
correction) or `judgment` (needs a reader's verdict).

Runner: `scripts/standards-scan.sh [<project-root>]` (this skill's `scripts/`).
Root defaults to the current directory and must contain `.jit/`. The scanner
reads its scope from `.jit/config.toml`; it works against any jit project, not
only this repo.

## Run

```
scripts/standards-scan.sh > findings.jsonl
```

- stdout: one JSON finding per line (schema below), totally ordered.
- stderr: a one-line count summary; never mixed into stdout.
- exit 0 on completion; exit 2 on bad invocation (`.jit/` missing, or `jq` /
  `gawk` absent).

Feed `findings.jsonl` to the mechanical auto-fixer (it consumes the
`mechanical` lines) and to the sweep report/workflow (it consumes all lines).

## Determinism (identical findings on unchanged input)

The scanner reads issue files in glob order, documents in `LC_ALL=C` sorted
order, and lines in file order. No timestamp, hostname, or random value enters
a finding. All findings are sorted by
`(target_kind, target, rule, line, detail)` before printing. Two runs against
an unchanged project therefore produce byte-identical stdout.

## Scope

Derived from `.jit/config.toml` `[documentation]`:

- **Every issue** in `.jit/issues/*.json` — title, labels, description. Issue
  state does not affect issue scope; all issues are scanned.
- **Every markdown file** under each `permanent_paths` entry (default `docs/`).
- **Every live markdown file** under `<development_root>/active` (default
  `dev/active`).

Out of scope (historical record): `archive_root`, the other `managed_paths`
(e.g. `dev/studies`, `dev/sessions`), and `dev/active` documents whose owning
issue is Done.

### dev/active exemption rule

A `dev/active` document is live unless its owning issue is Done. Ownership is
determined deterministically by either signal:

1. **Filename prefix** — a leading `<short-id>-` (8 hex chars) naming an issue.
2. **Doc linkage** — the document path appears in some issue's `documents[]`.

If a resolved owner is in state `done`, the document is exempt. A document with
no owner is live and is scanned.

## Rule catalog

Rule ids are stable. `where` names the field scanned; issue-only rules do not
run against documents.

### Issue title / labels

| Rule | Where | Class | Trigger |
|---|---|---|---|
| STD-TITLE-EMBEDDED-ID | title | mechanical | leading short-id/ordinal, `feat(...):` prefix, or `(jit:…)` |
| STD-TITLE-ANGLE | title | judgment | `&lt;`/`&gt;`/`<`/`>` present; reword needed |
| STD-LABEL-SLUG | labels | mechanical | `epic:`/`story:`/`milestone:` value is an 8-hex short id |

### Issue description structure

| Rule | Class | Trigger |
|---|---|---|
| STD-SC-MISSING | mechanical | no Success Criteria heading (case-tolerant; accepts Acceptance Criteria / Definition of Done / bare Criteria) |
| STD-CRIT-UNMARKED | mechanical | a section bullet with no `[hard]`/`[aspirational]` marker (checkbox stripped first) |
| STD-CRIT-REQID | mechanical | marker present but no well-formed `REQ-NN:` id |
| STD-HEADING-H1 | mechanical | `#` (H1) used in a description body |
| STD-HEADING-DEEP | mechanical | heading deeper than `###` |
| STD-ANTIPATTERN-SECTION | mechanical | `## Depends on` / `## Dependencies` / `## Children` heading (duplicates the DAG) |

### Issue standalone-readability (judgment)

| Rule | Trigger |
|---|---|
| STD-STANDALONE | bare pronoun opening (`It`/`This`/…), or a bare sibling short-id reference with no surrounding context |
| STD-CROSSREF | sibling cross-reference phrasing (`same as`, `per A2`, `see above`, `the previous issue`, …) |
| STD-TRACKER-MECHANICS | a spelled-out `jit <subcommand>` command in the description |

These are surfaced as candidates. The human or lead verdict on each happens
downstream in the sweep workflow, not in the scanner.

### Content rules (issue descriptions and documents)

| Rule | Class | Trigger |
|---|---|---|
| STD-ASCII-ART | judgment | box-drawing glyphs or `+---+` ASCII boxes (use Mermaid) |
| STD-PLAINTEXT-MATH | judgment | plain-text math tokens (`sum_`, `sqrt_`, `= sum `, …) outside `$…$` (use LaTeX) |

## Output schema

Each stdout line is a JSON object:

| Field | Value |
|---|---|
| `target_kind` | `"issue"` or `"document"` |
| `target` | issue short-id (8 hex) or repo-relative document path |
| `rule` | rule id from the catalog |
| `classification` | `"mechanical"` or `"judgment"` |
| `line` | 1-based line in the description/document; `0` for whole-item rules (title, labels, missing section) |
| `detail` | offending text, tabs stripped, for locating the hit |

## Requirements

- `jq` and `gawk` on PATH.
- Read access to `.jit/` and the in-scope document trees. The scanner never
  writes project state.
