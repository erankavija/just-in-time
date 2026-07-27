# Pitfall reconciliation against the epic's criteria

Every finding the lead deferred during this epic's execution, tested against the epic's fifteen
`[hard]` criteria before the epic's own gates run. Recording a finding in the progress file's
`surfaced_pitfalls` does not make it out of scope: the test is whether any criterion names its
subject, and a pitfall whose subject a criterion names is a criterion violation rather than a
follow-up.

Source: `dev/active/8e071e18-progress.json`, `surfaced_pitfalls` P1–P30.

## Method

For each pitfall, read its subject, then read every `[hard]` criterion on `8e071e18` and ask whether
one names that subject. A criterion the plan deliberately scoped away from still binds — a plan
binds the fan-out shape, not the criterion. Where the answer is no, the pitfall is a follow-up and
the completion report carries it.

## Pitfalls already closed

Twenty-four pitfalls were closed during execution and are re-verified here rather than re-argued.
P4, P5, P2, P6 and P3 carried an explicit "verify at wave N" instruction from their own
dispositions; all five were verified at HEAD this session and are listed with their evidence.

| id | subject | closure verified at HEAD |
|---|---|---|
| P2 | stale adopter examples of the documentation policy | `docs/reference/configuration.md:47-74` carries the eight-area managed set and the six permanent entries; the stale three-area set and `permanent_paths = ["docs/"]` are gone |
| P3 | `config.rs` accessor doc comments restate the default path lists | `crates/jit/src/config.rs:352-353,372-373` cite `SHIPPED_DOCUMENTATION_POLICY` instead of enumerating |
| P4 | the command reference calls a directory link target an unsupported edge | `docs/reference/cli-commands.md:265-267` states it is navigation contributing no artifact entry, no edge and no warning; no `unsupported-edge-target` mention survives |
| P5 | the command reference states the archive slug falls back to the issue title | `docs/reference/cli-commands.md:336-337` states labels and the short id are the whole input and the title has no part in it |
| P6 | the adopter configuration reference gains no `issue_scoped_areas` key | documented at `docs/reference/configuration.md:70,145` and `docs/reference/example-config.toml:44`; `citation_scan_roots` at `configuration.md:190` |
| P12 | planning-lead eval fixtures state the flat plan-doc path | fixed session 5 under owner direction |
| P14 | pre-existing stale doc comments in a rewritten file | fixed in `d2870182` |
| P17 | the planning-bracket guide and both example registries teach the flat plan-doc pattern | a repository-wide grep for `dev/active/{container.id}` over `docs/`, `.agents/`, `profiles/` and `crates/` returns nothing outside this epic's own historical records |
| P23 | the `doc` field's Rust doc comment leads with the flat plan-doc path | `crates/jit/src/templates.rs:333-341` now gives `{container.dir}/plan.md` and states the token is left verbatim without a declared area |
| P1, P7, P10, P15, P20, P21, P22, P26 and the remaining filed entries | — | closed in earlier sessions by owner decision, by a filed issue inside the epic, or by a measurement correction; dispositions stand unchanged |

## Pitfalls open at the epic gate

Six remain open. Each is tested below.

### P18 — an eligible plan can still refuse at execution, in the general case

**Criterion test.** REQ-02 requires an *eligible plan* for a presentation deck owned by a terminal
issue, and reports it. REQ-09 requires the cleanup to run through `jit archive container --execute`
with `jit validate` passing afterwards; all 24 executions ran and validation passes. REQ-15 requires
a container whose remaining blockers were all directory targets to yield an eligible plan. No
criterion names agreement between preview and execution as a general property.

The measured instance — eight containers previewing eligible and refusing at execution on an anchor
fragment — was filed as `ac45f567` inside this epic and closed. What remains is the broader design
question of whether eligibility should run the proposed-layout check at all.

**Verdict: follow-up, not a criterion violation.**

### P19 — the proposed-layout check diverges from discovery on backslash separators

**Criterion test.** `ac45f567`'s criteria name anchor fragments and query strings. No criterion on
the epic names separator normalization. The divergence is latent: all 24 containers execute cleanly,
so no reference in this repository takes that path.

The durable fix is one shared reference-path normalizer between `resolve_reference`
(`crates/jit/src/domain/artifact_discovery.rs`) and `validate_proposed_layout`
(`crates/jit/src/domain/artifact_classifier.rs`), rather than a second hand-aligned `replace` — two
hand-aligned resolvers is what produced both defects.

**Verdict: follow-up, not a criterion violation.**

### P25 — a directory link target is navigation to discovery and a missing asset to link checking

**Criterion test.** REQ-15 states that a link target resolving to a directory produces no
`unsupported-artifact-type` blocker and that a container whose remaining blockers were all directory
targets yields an eligible plan. Both hold. `@/issue/8e071e18/decision/D-18` scopes the decision to
artifact discovery. Neither reaches `jit doc add`'s asset scanner or `jit doc check-links`, which
still report such a link as a missing asset, nor `jit validate`'s message for a directory linked
directly as a document.

**Verdict: follow-up, not a criterion violation.** It is the largest of the six and the one most
likely to reach an adopter.

### P27 — archival leaves empty source directories

**Criterion test.** REQ-01, REQ-03 and REQ-10 all range over *artifacts* — a file a document
reference names, a file remaining under a managed area, a file carrying no issue prefix. An empty
directory is none of those. Git records no empty directory, so the litter is invisible under version
control and vanishes on clone; jit is git-optional (`@/charter/D-4`), so in a repository without
version control the emptied directories would persist.

**Verdict: not a criterion violation.** Carried to the completion report.

### P28 — a worktree is the wrong vantage point for a claim about path resolution

**Criterion test.** This is an execution hazard for dispatched workers, not a property of the
product. No criterion names it. It was carried into every dispatch prompt this session, and both
session-7 workers reported their path claims as worktree-scoped rather than repository-scoped
without error.

**Verdict: not a criterion violation.** It belongs in the handoff traps, where it is.

### P29 — the conformance report attributes an owner from the filename, not from the document reference

**Criterion test.** REQ-06 requires an advisory report listing "every artifact under a managed
issue-scoped area that sits outside its owning issue's canonical directory". The question is what
makes an issue the *owning* one. `@/issue/8e071e18/decision/D-2` drops the short-id prefix from
filenames inside an issue directory, so under the shipped convention a file's location is what names
its owner, and a prefix-less file inside an issue's directory is that issue's artifact. Nothing here
sits outside its owning issue's canonical directory, and the live report lists 45 artifacts
correctly.

The residual is narrower than REQ-06's subject: a document reference can name an owner that
disagrees with the location — six records this epic wrote sit in
`dev/active/8e071e18-dev-artifact-layout/` while their references name `aa38b236`, `93aaa2b2`,
`079ea42e`, `2d7ae27e`, `5a19fffb` and `66c467c1` — and the report is silent on that disagreement
because it never consults document references.

**Verdict: not a criterion violation.** Carried to the completion report for an owner decision.

### P30 — the skill eval baseline cites two evaluation records that do not exist

**Criterion test.** `dev/eval/lead-skills-eval-baseline.md:18` cites
`.agents/skills/jit-execution-lead/evals/results.md` and `:19` cites
`.agents/skills/jit-planning-lead/evals/results.md`; neither directory holds a `results.md`. The
file also cites transcript paths that do not resolve. `dev/eval` is a permanent area, and no epic
criterion ranges over it: REQ-09 and REQ-10 scope themselves to the managed areas and to `dev/active`,
`dev/studies` and `dev/sessions`. The citations predate this epic and no archival run touched them.

**Verdict: not a criterion violation, and outside the epic.** Carried to the completion report.

## Standing

No open pitfall is a violation of any of the epic's fifteen `[hard]` criteria. Three are genuine
product follow-ups (P18, P19, P25), two are observations about behaviour no criterion ranges over
(P27, P29), one is an execution hazard recorded in the handoff traps (P28), and one is a pre-existing
defect outside the epic (P30). The completion report names all seven.
