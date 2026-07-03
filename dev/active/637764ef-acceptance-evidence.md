# Acceptance evidence — 637764ef (uniform re-addressing)

Captured against HEAD prior to this task's commit, with the installed `jit` binary
rebuilt from that same tree (`jit 0.2.1 (commit c943698e, ...)`, no crate changes
between the previously-installed build commit and this HEAD, confirmed via
`git log --oneline 359a307d..HEAD -- crates/` returning zero commits before this
task's own fixes were applied).

## Step 1 — all five kinds resolve; bare vs. qualified `satisfies:` count

### `jit item show @/invariant/INV-DAG-ACYCLIC --json` (real invariant self-id; `INV-01`
does not exist in this repo's registry — the real self-ids are descriptive, e.g.
`INV-LABEL-FORMAT`, `INV-DAG-ACYCLIC` — so `INV-DAG-ACYCLIC` is used as the concrete example)

```json
{
  "issue_full_id": null,
  "issue_title": null,
  "item": {
    "kind": "invariant",
    "qualified_id": "@/invariant/INV-DAG-ACYCLIC",
    "scope": "@",
    "self_id": "INV-DAG-ACYCLIC",
    "text": "Cycle detection runs before every dependency operation; the graph stays acyclic."
  }
}
```

### `jit item list --kind requirement --json`

`count: 388`. Resolves through the uniform scheme, e.g.
`"qualified_id": "@/issue/0597febe/requirement/REQ-01"`.

### `jit item list --kind decision --json` / `jit item list --kind risk --json`

Both return `count: 0`. Verified this is real (not a query defect): no issue in
`.jit/issues/*.json` carries a non-empty `decisions` or `risks` array
(`python3` scan over `glob('.jit/issues/*.json')` — `decisions total: 0`,
`risks total: 0`). No decision or risk items exist anywhere in live data yet.

### `jit item list --kind definition --json`

`count: 6` — `@/definition/Issue`, `@/definition/Dependency`, `@/definition/Gate`,
`@/definition/State`, `@/definition/Label`, plus one more, all resolving under
`@/definition/<self-id>`.

### `jit item show 0597febe/REQ-01 --json` (sugar form, live requirement)

```json
{
  "issue_full_id": "0597febe-a888-40f5-8b29-fb86484e3a06",
  "issue_title": "Widen the label-value grammar to accept @-prefixed addresses",
  "item": {
    "kind": "requirement",
    "qualified_id": "@/issue/0597febe/requirement/REQ-01",
    "scope": "0597febe",
    "self_id": "REQ-01",
    "text": "[hard] REQ-01: validate_label/label_regex() accepts enforces:@/rule/label-format (bare @, no project name) and enforces:@myproject/gate/cargo-ci (@ + project name), each with a /-delimited path after the @-segment."
  }
}
```

Sugar form resolves to the canonical `@/issue/<short-id>/requirement/<self-id>`
qualified id. All five kinds (invariant, requirement, decision, risk, definition)
resolve correctly under the uniform scheme; decision/risk correctly resolve to
zero because no such items exist yet.

### Bare vs. qualified `satisfies:` count (re-verified)

```
$ grep -ohE '"satisfies:[^"]+"' .jit/issues/*.json | wc -l
95
$ grep -ohE '"satisfies:[^"/]+"' .jit/issues/*.json | wc -l   # bare, no slash
95
$ grep -ohE '"satisfies:[^"]*/[^"]*"' .jit/issues/*.json | wc -l   # qualified, contains a slash
0
```

**95 bare `satisfies:REQ-NN` occurrences, 0 qualified-form.** This has grown from
the investigation's confirmed baseline of 86 (as expected — later epic waves added
labeled issues); the qualified-form count is unchanged at 0. Also re-confirmed
`per:`, `mitigates:`, `resolves:`, `enforces:` link namespaces have zero occurrences
in `.jit/issues/*.json` (unchanged from the investigation's finding — link labels of
that kind are not yet in use; only `invariants.toml`'s `enforced-by` bindings use
the `enforces`-style link, and those already use the uniform `@/rule/...` /
`@/gate/...` form).

## Step 2 — `jit validate` / `./scripts/jit-validate.sh` clean exit

```
$ set -o pipefail
$ ./scripts/jit-validate.sh
✓ Repository validation passed
$ echo $?
0
```

Ran directly (exit code captured, not read after a piped `tail`/`grep`) both before
and after the fixes in Step 3 below. Clean in both cases.

## Step 3 — tree-wide legacy-address grep

Command (corrected from the literal instruction — the original exclusion patterns
used a `^\./` anchor, but `grep -r` on this system does not prefix matched paths
with `./`, so `.jit/gate-runs/` exclusion silently failed to match on the first
pass; re-run with anchors matching the actual `path/to/file` format):

```
grep -rn '@/[A-Z]' --include='*.rs' --include='*.md' --include='*.toml' --include='*.json' . \
  | grep -v '^\.git/' | grep -v '^target/' | grep -v '^\.claude/' | grep -v '^\.jit/gate-runs/'
```

Raw match count with the literal (buggy) exclusion pattern, before any fix: 42
lines — 6 of these are `.jit/gate-runs/*/result.json` gate-review transcripts that
the exclusion should have dropped but didn't (the `^\./\.jit/gate-runs/` anchor
never matches, because `grep -r .` here does not prefix paths with `./`). The
other 36 are hits outside `.jit/gate-runs/`. Of those 36, 2 were the genuine stray
hits fixed above (`.jit/config.toml`, `hierarchy_templates.rs`), leaving 34.
Re-running with the corrected exclusion pattern (`^\.jit/gate-runs/`, no leading
`./`) after the fix confirms: 34 hits total, 0 in `.jit/gate-runs/`, 0 matching the
two fixed lines.

### Genuine hits — FIXED

| Path:line | Before | After |
|---|---|---|
| `.jit/config.toml:100` | `examples = ["enforces:@/INV-01", "enforces:@/rule/label-format", "enforces:@/gate/cargo-ci"]` | `examples = ["enforces:@/invariant/INV-01", ...]` |
| `crates/jit/src/hierarchy_templates.rs:165` | same stale example, inside the `enforces` namespace template emitted by `jit init` | same fix |

Both were a stale example left un-migrated inside an `examples` array where the
other two entries in the same array (`enforces:@/rule/label-format`,
`enforces:@/gate/cargo-ci`) already used the uniform form — exactly the "stray
.rs test fixture or doc" class the task called out. The fix mirrors the
established precedent from this epic's own doc-hygiene task (`a8889340`):
`@/INV-01` → `@/invariant/INV-01`. Purely textual (a config example string and a
Rust string-literal template), no behavior change. Verified: `cargo fmt --all -- --check`
clean, `cargo clippy --workspace --all-targets` zero warnings, full
`cargo test --workspace` green (1389+ tests across all binaries, 0 failed), and
`./scripts/jit-validate.sh` still passes after editing the live `.jit/config.toml`.
No test pins the old literal string.

### Remaining 34 hits — categorized

**(b) Named exception — `dev/active/*-breakdown-spec.md`, `2821e177-investigation.md`,
`...-plan.md`** (23 hits, matches the task's exception list literally):

| File | Hit count |
|---|---|
| `dev/active/bb7d57a2-breakdown-spec.md` | 2 |
| `dev/active/71ebd1e8-breakdown-spec.md` | 2 |
| `dev/active/7f22d6cf-breakdown-spec.md` | 5 |
| `dev/active/0efbc594-breakdown-spec.md` | 2 |
| `dev/active/9a7106ae-breakdown-spec.md` | 2 |
| `dev/active/37506c12-breakdown-spec.md` | 2 |
| `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | 5 |
| `dev/active/2821e177-investigation.md` | 3 |

All quote the legacy `@/INV-01` / `@/INV-*` form verbatim while describing decision
D11 or characterizing the migration's own blast radius — process records of the
migration itself, exactly as the exception describes. No handoff/progress files
(`2821e177-handoff*.md`, `2821e177-progress.json`) matched the grep at all.

**(b-extended, judgment call) — `dev/studies/addressing-v2-rule-gate-items.md`** (3 hits:
lines 27, 191, 198):

Not literally named in the exception list, but this file is the epic's own
foundational design brief — both `2821e177-investigation.md:3` and
`2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:5` explicitly cite it as
"Grounded by `dev/studies/addressing-v2-rule-gate-items.md` (design brief)."
The epic's own doc-hygiene task (`c01f66dd`, commit `b3f2e2c1`) already touched
this exact file surgically — it added a "Resolved — D11" annotation below open
point 11 but deliberately left the original open-point question text (which
quotes `@/INV-*` and `@/INV-01`) unedited, treating it as a historical record of
what was asked. Line 27 (`enforces:@/INV-DAG-ACYCLIC`) is in the same
"process record" register — a finding recorded in the design brief at the time
it predated the kind-segmented scheme. I extended the same treatment (leave
unedited) to all three hits for consistency with the precedent `c01f66dd` already
set on this very file, rather than re-opening a file the epic's own hygiene task
already closed out. Flagging this explicitly since it required judgment beyond
the literal exception list — happy to fix if review disagrees.

**dev/archive/** scope decision** — `dev/archive/features/25064508/completion-report.md:67`
(1 hit, `@/X` as a generic placeholder in retrospective prose):

This is the literal precedent case the task itself names in step 4. The
investigation's REQ-08 finding already classified this exact file as
"documentation hygiene, not a correctness requirement." Per the task's own
instruction, archived/frozen historical artifacts are out of scope, left as
historical snapshots — decision confirmed, no edit made.

Separately, `dev/archive/features/25064508/showcase/talk.html` (named in the task
as another example) contains 6 matches of `@/[A-Z]` but is an `.html` file, outside
this grep's `--include` extensions (`*.rs`, `*.md`, `*.toml`, `*.json`) by
construction — it never surfaces in this scan. Confirmed it is a frozen
presentation deck about the already-shipped, already-archived `25064508` feature
(dated `Jun 26`, predates this epic). Same scope decision applies: out of scope,
historical snapshot, not touched.

**Live jit-issue descriptions (new category, not in the original exception list)** —
7 hits across `.jit/issues/*.json`: `182aa0d5` (done), `42898915` (done), `c01f66dd`
(done), `a8889340` (done), `25064508` (done), `637764ef` (this task, in_progress),
`7f22d6cf` (this task's parent story, backlog).

Each quotes `@/INV-01`/`@/INV-*`/`@/INV-02` as illustrative "before" text describing
the very migration bug/step that issue's own work addressed — functionally the
same "process record of the migration itself" the named exception (b) describes
for `dev/active/*.md` docs, just stored as issue description text instead of a
standalone doc. Not edited, for two reasons: (1) editing an issue's description
field would rewrite historical acceptance-criteria/background text for issues
that are mostly already `done` — akin to rewriting a merged commit message: it
misrepresents what was actually asked/found at the time; (2) doing so would
require either a `jit` command (explicitly out of scope — "do NOT run ... any
other jit state mutation; the lead ... handles all lifecycle state") or a direct
hand-edit of `.jit/issues/*.json`, which bypasses `IssueStore`'s atomic-write path
entirely and is not something this task should do unsupervised. Flagging this
category explicitly for the lead's/review's call, same as the design-brief
judgment call above.

## Step 4 — `dev/archive/**` scope decision (explicit paragraph)

Archived/frozen historical artifacts under `dev/archive/**` are out of scope for
this migration and are left as historical snapshots, matching the investigation's
own REQ-08 classification of the `25064508` completion report as "documentation
hygiene, not a correctness requirement." Two concrete instances were checked:
`dev/archive/features/25064508/completion-report.md:67` (1 grep hit, generic `@/X`
placeholder in retrospective prose — confirmed genuinely archived, predates this
epic) and `dev/archive/features/25064508/showcase/talk.html` (6 literal `@/INV-01`
examples in a frozen presentation deck about the already-shipped feature, outside
this grep's file-extension scope by construction). Neither is edited. No other
`dev/archive/**` file surfaced anything requiring reconsideration of this
decision — a broader `grep -rl '@/[A-Z]' dev/archive/` (no extension filter) finds
only these same two files.

## Summary

- Step 1: PASS — all five kinds resolve; 95 bare / 0 qualified `satisfies:` labels.
- Step 2: PASS — `jit validate` and `./scripts/jit-validate.sh` both exit 0.
- Step 3: 2 genuine stray hits found and fixed (`.jit/config.toml`,
  `crates/jit/src/hierarchy_templates.rs`); `cargo fmt`/`clippy`/full `cargo test
  --workspace` all clean after the fix. Remaining 34 hits categorized: 23 match
  the named exception literally, 3 extend it by judgment call to the epic's
  design brief (precedent-consistent with `c01f66dd`), 1 matches the named
  `dev/archive/**` precedent exactly, and 7 are a new category (live issue
  descriptions quoting the legacy form as historical background) left unedited
  because editing them would require out-of-scope jit-state mutation.
- Step 4: `dev/archive/**` scope decision recorded and confirmed against both
  named example files.
