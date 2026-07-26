# Standards sweep

The steward's fourth mode, run end to end to bring the whole project's issues and
documents into content-standards compliance. It composes the tested scan and fix
scripts, then reports the result: scan every issue and document against the
canonical rules (`.jit/reference/content-standards.md`), auto-apply every
mechanical correction, and record the auto-fixed items and the human-judgment
items in one report with the two kept visibly separate.

This reference composes two scripts it does not re-implement:
`.agents/skills/jit-project-lead/scripts/standards-scan.sh`
(`references/standards-scan.md`) and
`.agents/skills/jit-project-lead/scripts/standards-fix.sh`
(`references/standards-fix.md`). Read both references
for their JSONL schemas before running the mode.

## When to invoke

Invoke the standards-sweep mode when the request is to make the project's
tracked content compliant with the content standards: sweep the issues and
documents, fix what is mechanically fixable, and surface what needs a decision.
It runs project-wide (every issue plus every in-scope document, per the
scanner's scope), not against one container. It is orthogonal to the steward's
container-dispatch drive; it neither dispatches leads nor changes issue
lifecycle state.

## What it changes automatically vs leaves open

- **Auto-applied** — only the mechanical corrections, each written through the
  jit CLI by the fixer (`references/standards-fix.md`): a single unambiguous
  rewrite per mechanical finding. No judgment item, no clean item, is touched.
- **Left open** — every judgment violation, reported for a human or the lead to
  decide. The mode never auto-changes a judgment item; a judgment finding needs a
  reader's verdict the scripts cannot supply.

## Workflow

Run from the project root (must contain `.jit/`). The mode is three steps: scan
once to a file, fix from that same file, then build one report from both.

1. **Scan to a file.** Capture every finding, judgment included, so the same scan
   feeds both the report's judgment section and the fixer:

   ```
   .agents/skills/jit-project-lead/scripts/standards-scan.sh > findings.jsonl
   ```

   Scanner exit 2 stops the mode (see Stop and escalate). An empty
   `findings.jsonl` on exit 0 means the project is already compliant; still write
   the report (both sections empty) so the run has a durable, reviewable result.

2. **Fix from that file.** Feed the captured findings to the fixer so the fix set
   matches the scanned set exactly, with no re-scan drift between what the report
   shows and what was corrected:

   ```
   .agents/skills/jit-project-lead/scripts/standards-fix.sh --findings findings.jsonl > applied.jsonl
   ```

   The fixer applies each mechanical correction through the jit CLI and emits the
   fix ledger (`applied.jsonl`): one record per correction, `action` of
   `applied`, `dry-run`, or `skipped`. It leaves every judgment finding and every
   clean item byte-identical. Fixer exit 2 stops the mode. The pipe form
   (`standards-scan.sh | standards-fix.sh`) is not used here: it discards the
   scanner findings, and the report's judgment section needs them.

3. **Build one report** from `findings.jsonl` (judgment section) and
   `applied.jsonl` (auto-fixed section), per Report format below. The judgment
   section is drawn from the pre-fix findings; the fixer leaves judgment findings
   untouched, so they are still accurate after the fix step.

## Report format

One report document with exactly two sections that never intermix: every
auto-fixed correction in the first, every open judgment item in the second. A row
belongs to one section only.

### Location (config-derived)

Read `.jit/config.toml` `[documentation]`; never hardcode a directory. The report
is operational state, so it lives under the active root, as the progress artifact
does (`references/progress-artifact.md`):

- **Active root** — `<development_root>/active`, the `[documentation]` key of the
  same name.
- **Report directory** — obtained from `jit doc dir <strategic-container-id>
  <development_root>/active`, the steward's own progress directory.
- **Report path** — `<report-directory>/standards-sweep.md`. One project-wide
  report; a later run overwrites it (the fix and scan are idempotent, so a
  re-run on unchanged content reproduces the same file).

> Example: with `development_root = "notes"`, `jit doc dir <container-id>
> notes/active` prints the directory the report lands in; `standards-sweep.md`
> lands inside it. Derive the root from configuration.

Link the report to the steward's strategic container so it surfaces under
`jit doc list`:

```
jit doc add <strategic-container-short-id> <report-path> \
    --doc-type notes --label "Standards sweep"
```

### Structure

Title, a one-line count summary, then the two sections in this fixed order. Use
`##` headings only (content-standards heading rule). Every run produces this same
shape:

```markdown
# Standards sweep report

Auto-fixed: <N> correction(s). Needs a decision: <M> item(s).

## Auto-fixed

| Target | Rule | Correction applied |
|---|---|---|
| <issue short-id or doc path> | <rule id> | <fixer ledger detail> |

## Needs a decision

| Target | Rule | Why it needs a decision |
|---|---|---|
| <issue short-id or doc path> | <rule id> | <rationale> |
```

**Auto-fixed section.** One row per `applied.jsonl` record with `action ==
"applied"`. `Target` is the record's `target` (issue short-id or repo-relative
doc path), `Rule` its `rule`, `Correction applied` its `detail` (what the fixer
did). Select and stably order:

```bash
python3 -c 'import json,sys
for line in open(sys.argv[1], encoding="utf-8"):
    r=json.loads(line)
    if r.get("action")=="applied":
        print("\t".join(map(str,[r["target_kind"],r["target"],r["rule"],r["line"],r["detail"]])))' \
    applied.jsonl | sort
```

**Needs-a-decision section.** One row per `findings.jsonl` finding with
`classification == "judgment"`, plus one row per `applied.jsonl` record with
`action == "skipped"` (a mechanical finding the fixer genuinely could not
correct, e.g. a criterion whose issue has every `REQ-NN` id reserved). `Target`
and `Rule` come straight from the record. `Why it needs a decision` is the
rule's rationale from the table below for a judgment finding, or the skip
`detail` for a skipped record. The section is the union of two selections —
never drop the skipped-mechanical rows, or a finding the fixer could not correct
vanishes from the report entirely. Select both and stably order the combined
set:

```bash
python3 -c 'import json,sys
for path in sys.argv[1:]:
    for line in open(path, encoding="utf-8"):
        r=json.loads(line)
        if r.get("classification")=="judgment" or r.get("action")=="skipped":
            print("\t".join(map(str,[r["target_kind"],r["target"],r["rule"],r["line"],r["detail"]])))' \
    findings.jsonl applied.jsonl | sort
```

Both sections are empty when their selection is empty; the section heading still
prints so the report's shape is invariant.

### Judgment-rule rationale

The scanner emits the offending text, not a rationale. The report supplies the
rationale per judgment rule from this fixed map, so the same rule always reads the
same way. Every judgment rule in `references/standards-scan.md` is covered:

| Rule | Why it needs a decision |
|---|---|
| STD-TITLE-ANGLE | Title carries `<`/`>`; a reword that preserves the intended meaning is an authoring choice. |
| STD-LABEL-SLUG | No meaningful kebab slug is derivable from an 8-hex hash; a human must name the bucket. |
| STD-STRUCT-SUMMARY | The missing 1–2 sentence opening summary must be authored; only the issue's intent supplies it. |
| STD-STRUCT-BACKGROUND | Whether the issue is non-trivial enough to need `## Background`, and what it says, is an editorial call. |
| STD-CRIT-ACTION | Rephrasing an action into a verifiable outcome can change meaning; it needs the author's intent. |
| STD-STANDALONE | Resolving a bare pronoun or bare short-id reference requires knowing the referent to inline. |
| STD-CROSSREF | Inlining a sibling cross-reference requires deciding what context to copy so the issue reads standalone. |
| STD-TRACKER-MECHANICS | Removing a spelled-out command, or gate/reviewer naming, means rewriting the described work. |
| STD-ASCII-ART | Converting an ASCII-art diagram to Mermaid requires interpreting what the diagram depicts. |
| STD-PLAINTEXT-MATH | Transcribing plain-text math to LaTeX requires reading the intended expression. |
| STD-MATH-SLASH-FRAC | Choosing `\frac{a}{b}` over a slash is a formatting judgment on the display expression. |
| STD-MATH-BARE-VAR | Deciding a bare token is a variable (needs math mode) versus prose requires reading its context. |
| STD-MATH-MULTI-LABEL | Wrapping a multi-letter super/subscript in `\text{…}` requires confirming it is a label, not a product. |
| STD-MATH-INLINE-LIMITS | Moving inline limits to a display block is a layout judgment on the expression. |

## Determinism

Two runs on unchanged project content produce the same report. The scanner is
deterministic (`references/standards-scan.md`), the fixer is a pure function of
findings and content (`references/standards-fix.md`), the section selections are
`sort`-ordered, and the rationale map is fixed. No timestamp or hostname enters
the report.

## Stop and escalate

Stop the mode and report to the invoker (the human when the steward runs
standalone) when:

- **Scanner exit 2** — bad invocation: `.jit/` missing at the project root, or
  `python3`, `base64`, `gawk`, or `jit` absent from PATH, or the CLI issue listing failed. The
  scan produced no findings; do not proceed to fix or report.
- **Fixer exit 2** — the same class of bad invocation for the fixer (missing
  `.jit/`, missing `python3`/`base64`/`gawk`/`jit`, or an internal scan failing). Mechanical
  corrections did not complete; report the scan result and stop.
- **Report path cannot be resolved** — `development_root` is missing,
  `[documentation]` is unreadable, or `jit doc dir` rejects the active root as an
  undeclared issue-scoped area, so the report directory has no location. Stop
  rather than write to a guessed directory (as `references/progress-artifact.md`
  stops on the same condition).

## Red flags

- Re-implementing the scanner or fixer instead of composing the tested scripts.
  The single unambiguous corrections and the finding schema live in those two.
- Using the `standards-scan.sh | standards-fix.sh` pipe and then having no
  findings for the judgment section. Scan to a file first, fix from that file.
- Building the judgment section from a post-fix re-scan. Use the pre-fix
  `findings.jsonl`; a re-scan invites drift between the reported and fixed sets.
- Intermixing the two sections, or moving a skipped mechanical row into
  Auto-fixed. Auto-fixed is applied corrections only; everything left open is in
  Needs a decision.
- Auto-changing a judgment item, or hand-editing an issue or document in this
  mode. Mechanical fixes go through the fixer; judgment items are reported, never
  silently changed.
- Hardcoding `dev/` or `dev/active` instead of reading `development_root`.
- Composing the report path as
  `<development_root>/active/standards-sweep-report.md` directly instead of
  resolving the directory with `jit doc dir` first.
- Piping scanner or fixer stderr (the one-line count summary) into the report or
  into the JSONL files. Only stdout is data.
