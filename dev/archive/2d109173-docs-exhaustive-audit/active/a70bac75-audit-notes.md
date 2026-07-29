# a70bac75 — Example rulesets audit notes

Footprint: `docs/examples/` — 7 rulesets, 14 files, ~1,054 authored lines (TOML + JSON;
no markdown). TOML schema validity is already cargo-tested
(`crates/jit/tests/example_rulesets_tests.rs`); this audit verifies narrative and
reference accuracy against HEAD (`8e4acd98`) only.

Binary guard: `jit 0.2.1 (commit 8e4acd98)` prefix-matches `git rev-parse HEAD`.

## REQ-07 triage (per-ruleset classification)

| Ruleset | Files | Class | Depth applied |
|---|---|---|---|
| `sdd/` | 4 (config, rules, templates, `schemas/spec-body.json`) | **Reference-grade** | Exhaustive: every rule kind, every `label-coverage`/`label-reference` knob, the closure-vs-preview split, all lifecycle-when claims, the `plan` template mechanism, and the body schema cross-checked against source. |
| `research/` | 4 (config, rules, templates, `schemas/research-bodies.json`) | **Reference-grade** | Exhaustive, same as `sdd/`. Domain-agnostic twin (`goal`/`experiment`/`hyp:`/`tests:`); verified no software-vocabulary leaks and that engine reads the breakable type from the template. |
| `bug-repro/` | 2 (rules, `schemas/bug-body.json`) | Illustrative | Full: `require-section` + `json-schema` (both local), schema shape over `sections.reproduction.items`. |
| `cross-epic/` | 1 (rules) | Illustrative | Full: `label-uniqueness` `scope="all"` (only permitted value), the validate-only / not-at-transition claim, the O(n·k) single-pass performance claim. |
| `fresh-evidence/` | 1 (rules) | Illustrative | Full: `gate-recency` `max-age-days`/`gates` filter, injected-clock determinism, enforce-at-done blocking. |
| `nyquist/` | 1 (rules) | Illustrative | Full: `criteria-to-check` `marker`/`gate-prefix`/`check-namespace`, always-on-warn vs done-scoped-enforce pairing. |
| `release-checklist/` | 1 (rules) | Illustrative | Full: `require-section` + `require-doc-type` (local) + `dependency-shape` `target`/`mode`/`transitive` (graph). |

Classification matches the file-count signal (4-file full-stack → reference-grade;
single-file → illustrative; `bug-repro/` 2-file → illustrative). Because each illustrative
file is short, every factual claim in it was still individually source-verified; the class
distinction governed how much *cross-claim lifecycle-interaction* analysis each received.

## Rework (doc-review attempt 1): enforcement/severity prose drift in release-checklist

The initial "zero drift" conclusion was WRONG. The `doc-review` gate found one real
drift class the first pass missed: prose in `docs/examples/release-checklist/rules.toml`
that conflated blocking-ness with severity and over-credited what blocks the `done`
transition. Two instances fixed:

- **F1 (header, lines 6-8).** The header said a release "must (a) carry a Checklist
  section, (b) have an attached release-notes document, and (c) depend on a QA sign-off
  issue" before done — implying all three block. In fact `release-has-checklist` is
  `warn` (advisory), `release-has-notes-doc` is `error` + `enforce = true` (the only rule
  that blocks `done`), and `release-depends-on-qa-signoff` is `error` with no `enforce`
  (reported, non-blocking at transition). Rewrote the header to state each rule's real
  strength and that only (b) blocks completion.
- **F2 (QA-signoff rule comment, line 34).** Called the non-enforced `error`-severity
  finding "a non-blocking warning". Non-enforced means non-blocking; it does not downgrade
  the severity to `warn`. Rewrote to: an `error`-severity finding that is non-blocking
  because enforcement — not severity — gates a transition.

**Class sweep across the rest of `docs/examples/`.** Re-audited every blocking/severity
claim in all seven rulesets against each rule's actual `severity`/`enforce`:
`bug-repro`, `cross-epic`, `fresh-evidence`, `nyquist`, `sdd`, `research` all describe
their rules correctly — `warn` rules say "reports/surfaces without blocking",
`enforce = true` rules say "blocks", and non-enforced `error` graph rules
(`cross-epic-req-uniqueness`, `sdd-req-matches-a-criterion`,
`research-hyp-label-matches-hypothesis`) use "reported/caught immediately", never "blocks".
No other instance of the drift class exists; `release-checklist/rules.toml` was the sole
offender.

Triage correction: `release-checklist/` remains **illustrative**, but its prose carried
semantic drift the file-count signal did not surface — illustrative class does not imply
prose is automatically accurate, only that it gets less cross-claim analysis. The lesson:
verify every severity/enforce claim against the rule's own flags even in short files.

## Verification result (rest of footprint): claims match HEAD source

Every other verifiable claim was checked against HEAD source and holds. Consistent with
plan §"Technical soundness" (REQ-04 largely satisfied, investigation §4):

- **Rule kinds (REQ-01/04):** all 11 named kinds exist in `crates/jit/src/validation/`
  (`require-section`, `json-schema`, `label-value-pattern`, `require-doc-type`,
  `criteria-label-match`, `label-coverage`, `label-reference`, `dependency-shape`,
  `gate-recency`, `criteria-to-check`, `label-uniqueness`). Local-vs-graph labels in the
  comments match `Assertion::scope()` (`rules.rs:868`).
- **Rule-kind config knobs (REQ-04):** verified against the `Raw*` deserializers and the
  `toml::Table` parsers — `gate-recency` (`max-age-days`, `gates`), `criteria-to-check`
  (`marker`/`gate-prefix`/`check-namespace`/`id-pattern`), `criteria-label-match`
  (`namespace`/`criteria-section`/`marker`/`id-pattern`), `label-coverage`
  (`criteria-section`/`marker`/`id-pattern`/`satisfies-namespace`/`child-state`/`child-link`/
  `child-type-exclude`/`container-from-label`, `graph.rs:696-784`), `label-reference`
  (`from`/`to`/`scope`∈{global,linked}, `graph.rs:1054-1086`), `dependency-shape`
  (`target`/`mode`∈{must,should}/`transitive`, `graph.rs:1194-1238`), `label-uniqueness`
  (`namespace`/`scope`="all" only, `rules.rs:1335-1346`), `require-doc-type` (`doc-type`),
  `label-value-pattern` (`namespace`/`regex`).
- **Scope-token disambiguation (cross-epic):** `label-uniqueness` uses `"all"`;
  `label-reference` uses `"global"`/`"linked"` — the two are distinct in source, matching
  the comment. `label-uniqueness.is_repo_wide_at_transition()==true` (`rules.rs:921`)
  confirms the validate-only claim; the sdd/research linked-scope rules
  (`is_repo_wide_at_transition()==false`) confirm those run at the done transition.
- **CLI surface (REQ-01):** `apply`, `validate --scope`, `query strategic`, `dep add`,
  `issue update --state --force` all present in `jit --schema`. M1 invented-flag guard over
  the footprint returned empty (every `--flag` mentioned is real).
- **Template mechanism (REQ-04):** `move-upstream-to-role` transform kind exists
  (`templates.rs:203`); anchors/nodes/roles/`depends_on`/`anchor_edges`/`transforms` and the
  templates-loader validation invariants match `templates.rs`.
- **Engine semantics (REQ-04):** exit-code 4 = "Validation failed" (`--schema` taxonomy);
  `TransitionBlockedError` (`errors.rs:672`); `GraphRuleBypassed` event (`types.rs:1515`);
  `gate-recency` computes `now - GateState.updated_at` against an injected clock, never
  wall-clock (`graph.rs:18-19, 1300-1368`).
- **Config keys:** `[version] schema = 2` is current (`config_manager.rs:264`);
  `strategic_types` lives under `[type_hierarchy]` (`config_manager.rs:271`); `[validation]`
  `strictness`/`default_type` both exist (`config.rs:295-300`).
- **State enum:** `backlog/ready/in_progress/gated/done` per `types.rs:28-42`.
- **Storage paths (REQ-02):** all cited paths are `.jit/{config,rules,templates}.toml` and
  `.jit/schemas/` — all real. M3 reports all cited paths and `@/` items resolve.
- **Design-doc citation (REQ-02):** `dev/active/planning-bracket-design.md` exists; the
  `D1/D5/D9/D11/D13` decision refs are plain-prose pointers into that cited file.

## Mechanical-check results (all clean)

- `./scripts/docs-mechanical.sh docs/examples/` → M2 links/anchors OK, M3 citations OK,
  M5 projections fresh, exit 0.
- M1 invented-flag guard → empty (clean).
- M4 box-drawing (`U+2500–257F`) → no hits (clean). No diagram-shaped blocks exist;
  inline precedence notations (`P > B > impl > C`) and edge notations (`B → P`, `C → B`) are
  inline arrows, not diagrams (task carve-out; REQ-03 satisfied).

## Judgment calls

1. **fresh-evidence `rules.toml:5` "no longer trustworthy" (REQ-05).** Describes the
   methodology's rationale — stale gate evidence loses trust over time — not a past state of
   jit's own behavior. Excluded per `doc-review-prompt.md` §3 (illustrative, not a product
   claim). Not a defect.
2. **`strictness = "loose"` (sdd/research config).** `ValidationConfig.strictness` is
   serde-inert (`config.rs:290`, "the inert `strictness`"). The examples only *set* the key;
   they make no prose claim about what it does. A valid, currently-accepted key with no
   behavioral assertion → not a defect.
3. **`bc86f54c` issue-id reference (fresh-evidence `rules.toml:18`).** Resolves to a real
   issue (`jit issue show bc86f54c`). It attributes transition enforcement to its
   implementing issue; kept as a valid citation.
4. **Diagram adjudication (REQ-03/04).** No box-drawing or multi-line arrow-spine art. The
   `## Success Criteria` example blocks in nyquist and the section-slug tables are verbatim
   authoring examples / reference tables, not diagrams.

## REQ-06 follow-up facts

None. Every volatile fact in the footprint is either self-describing example config content
(rule kinds, knobs — the example *is* the fact) or cited inline to its source (the design
doc path, the implementing issue id). No hand-copied count or enumeration with an unused
projection surface was found, so there is nothing to file for follow-up.
