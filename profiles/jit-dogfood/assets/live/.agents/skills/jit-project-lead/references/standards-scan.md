# Standards scan

Deterministic scan of every project issue and every in-scope document against
the canonical content standards (`.jit/reference/content-standards.md`).
Each violation is emitted as one finding, classified `mechanical` (one fixed
correction) or `judgment` (needs a reader's verdict).

Runner:
`.agents/skills/jit-project-lead/scripts/standards-scan.sh [<project-root>]`.
Root defaults to the current directory and must contain `.jit/`. The scanner
reads its scope from `.jit/config.toml`; it works against any jit project.

## Run

```
.agents/skills/jit-project-lead/scripts/standards-scan.sh > findings.jsonl
```

- stdout: one JSON finding per line (schema below), totally ordered.
- stderr: a one-line count summary; never mixed into stdout.
- exit 0 on completion; exit 2 on bad invocation (`.jit/` missing; `python3`,
  `gawk`, or `jit` absent; or the CLI issue listing failing).

Feed `findings.jsonl` to the mechanical auto-fixer (it consumes the
`mechanical` lines) and to the sweep report/workflow (it consumes all lines).

## Determinism (identical findings on unchanged input)

The scanner reads issues from the CLI listing sorted by id, documents in
`LC_ALL=C` sorted order, and lines in file order. No timestamp, hostname, or
random value enters a finding. All findings are sorted by
`(target_kind, target, rule, line, detail)` before printing. Two runs against
an unchanged project therefore produce byte-identical stdout.

## Scope

Derived from `.jit/config.toml` `[documentation]`:

- **Every issue** returned by `jit issue list --full --json` — title, labels,
  description. Issue state does not affect issue scope; all issues are scanned.
- **Every markdown file** under each `permanent_paths` entry (default `docs/`).
- **Every live markdown file** under `<development_root>/active` (default
  `dev/active`).

Persisted issue data (title, description, labels, state, document linkage) is
read only through the jit CLI — never by parsing `.jit/issues/*.json` directly.
Only `.jit/config.toml` is read as a file, for the `[documentation]` scope,
because no CLI surface exposes it; it is configuration, not issue persistence.

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
| STD-LABEL-SLUG | labels | judgment | a strategic membership label (namespace drawn from config `[type_hierarchy.label_associations]`, e.g. `epic:`/`story:`/`milestone:` in this repo) has an 8-hex short id for its value (a meaningful bucket slug cannot be derived from a hash) |

### Issue description structure

| Rule | Class | Trigger |
|---|---|---|
| STD-STRUCT-SUMMARY | judgment | no summary line precedes the first heading (the required 1–2 sentence opening summary is missing) |
| STD-STRUCT-BACKGROUND | judgment | a non-trivial issue (a substantial context region before Success Criteria) carries no `## Background` heading; simple leaf tasks are exempt |
| STD-SC-MISSING | mechanical | no Success Criteria heading (case-tolerant; accepts Acceptance Criteria / Definition of Done / bare Criteria) |
| STD-CRIT-UNMARKED | mechanical | a section bullet with no `[hard]`/`[aspirational]` marker (checkbox stripped first) |
| STD-CRIT-REQID | mechanical | marker present but the id is not a well-formed `REQ-NN:` — that is `REQ-` + exactly two digits + colon. Wrong digit count (`REQ-1`, `REQ-123`), a missing colon, or a missing/misspelled `REQ` token all trigger it |
| STD-CRIT-ACTION | judgment | criterion phrased as an action, not an outcome (opens with an imperative verb: `Implement`, `Add`, `Create`, `Build`, `Write`, `Refactor`, …) |
| STD-HEADING-H1 | mechanical | `#` (H1) used in a description body |
| STD-HEADING-DEEP | mechanical | heading deeper than `###` |
| STD-ANTIPATTERN-SECTION | mechanical | `## Depends on` / `## Dependencies` / `## Children` heading (duplicates the DAG) |

A well-formed criterion is `[hard]`/`[aspirational]` + a stable `REQ-NN` id
(exactly two digits) + `:`, e.g. `- [hard] REQ-01: Returns X for input Y.`

### Issue standalone-readability (judgment)

| Rule | Trigger |
|---|---|
| STD-STANDALONE | bare pronoun opening (`It`/`This`/…), or a bare sibling short-id reference with no surrounding context |
| STD-CROSSREF | sibling cross-reference phrasing (`same as`, `per A2`, `see above`, `the previous issue`, …) |
| STD-TRACKER-MECHANICS | a spelled-out `jit <subcommand>` command, or naming/instructing the gate or reviewer (`gate must pass`, `the reviewer`, `adversarial review`, …) in the description |

These are surfaced as candidates. The human or lead verdict on each happens
downstream in the sweep workflow, not in the scanner.

### Content rules (issue descriptions and documents)

| Rule | Class | Trigger |
|---|---|---|
| STD-ASCII-ART | judgment | box-drawing glyphs or `+---+` ASCII boxes (use Mermaid) |
| STD-PLAINTEXT-MATH | judgment | plain-text math tokens (`sum_`, `sqrt_`, `= sum `, …) outside `$…$` (use LaTeX) |
| STD-MATH-SLASH-FRAC | judgment | a slash fraction (`a/b`) inside a `$$…$$` display block (prefer `\frac{a}{b}`) |
| STD-MATH-BARE-VAR | judgment | a bare variable outside math mode — a single-letter variable (excluding `a`/`A`/`I`) or sub/superscript notation (`N_0`, `x^2`); inline `$…$` spans are stripped first as correct |
| STD-MATH-MULTI-LABEL | judgment | a multi-letter sub/superscript label not wrapped in `\text{…}` (`x_{avg}` should be `x_{\text{avg}}`) |
| STD-MATH-INLINE-LIMITS | judgment | inline `$…$` math carrying display-style `\sum`/`\int` limits (move to a `$$…$$` block) |

## Canonical rule coverage

Every rule in `.jit/reference/content-standards.md` maps to a scan check
below. Deterministic rules get a mechanical check; rules that need a reader's
verdict get a judgment emission (surfaced as a candidate). No rule is left
unevaluated.

| Canonical rule (section → statement) | Check | Class |
|---|---|---|
| Issue Descriptions → Required structure: leading 1–2 sentence summary | STD-STRUCT-SUMMARY | judgment |
| Issue Descriptions → Required structure: `## Background` for non-trivial issues (omit for simple leaf tasks) | STD-STRUCT-BACKGROUND | judgment |
| Issue Descriptions → `## Success Criteria` mandatory | STD-SC-MISSING | mechanical |
| Issue Descriptions → criteria are verifiable outcomes, not actions | STD-CRIT-ACTION | judgment |
| Issue Descriptions → every criterion prefixed `[hard]`/`[aspirational]` + `REQ-NN`; none unmarked | STD-CRIT-UNMARKED, STD-CRIT-REQID | mechanical |
| Issue Descriptions → `##` headings only, never `#` or deeper than `###` | STD-HEADING-H1, STD-HEADING-DEEP | mechanical |
| Issue Descriptions → present-tense imperative voice for criteria | STD-CRIT-ACTION | judgment |
| Issue Descriptions → math/diagram: apply the standards below | math + diagram rules (this table) | — |
| Anti-patterns → no `## Depends on` section | STD-ANTIPATTERN-SECTION | mechanical |
| Anti-patterns → no `## Children` blow-by-blow | STD-ANTIPATTERN-SECTION | mechanical |
| Anti-patterns → no sibling cross-references | STD-CROSSREF, STD-STANDALONE | judgment |
| Anti-patterns → issue must read standalone | STD-STANDALONE | judgment |
| Anti-patterns → no tracker mechanics (commands, reviewer/gate naming) | STD-TRACKER-MECHANICS | judgment |
| Issue Titles → no embedded id/ordinal/`feat(...)` prefix | STD-TITLE-EMBEDDED-ID | mechanical |
| Issue Titles → reword `<`/`>` (stored escaped) | STD-TITLE-ANGLE | judgment |
| Strategic Labels → kebab slug, never the 8-hex short id | STD-LABEL-SLUG | judgment |
| Diagrams → use Mermaid, no ASCII art | STD-ASCII-ART | judgment |
| Mathematics → use LaTeX, no plain-text equations | STD-PLAINTEXT-MATH | judgment |
| Mathematics → variable names in math mode (`$x$` not `x`, `$N_0$` not `N_0`) | STD-MATH-BARE-VAR | judgment |
| Mathematics → `\text{…}` for multi-letter labels in math | STD-MATH-MULTI-LABEL | judgment |
| Mathematics → `\frac{a}{b}` over `a/b` for display fractions | STD-MATH-SLASH-FRAC | judgment |
| Mathematics → inline `\sum`/`\int` without display limits | STD-MATH-INLINE-LIMITS | judgment |
| JIT-tooling pitfalls → `validate --fix` is the transitive-reduction tool | not a content property (CLI behavior note); no scan check | — |
| JIT-tooling pitfalls → `issue update --label` appends | not a content property (CLI behavior note); no scan check | — |
| JIT-tooling pitfalls → title HTML-escaping | STD-TITLE-ANGLE (same as Issue Titles) | judgment |
| JIT-tooling pitfalls → strategic-heading matching is case-tolerant | scanner complies: STD-SC-MISSING is case-tolerant | — |

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

- `jit`, `python3`, `base64`, and `gawk` on PATH.
- Read access to `.jit/` and the in-scope document trees. The scanner never
  writes project state.
