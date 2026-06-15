# Design: Plan-before-fan-out bracket

Make planning a first-class, reviewable, gated node in the DAG, sequenced *before* the
implementation fan-out — the front-end counterpart to the done-transition coverage
enforcement shipped by epic `94d26c42`.

> Status: design under review (revision 4). This document is itself the plan artifact for
> the planning node of its own epic (dogfooding the bracket).
>
> **Revision 4** addresses the round-3 plan-review FAIL (a single factual defect): the
> preview coverage instance specified `child-state = any`, which is not a valid token
> (`child-state` is validated as a lifecycle state). "Any state" is expressed by **omitting**
> `child-state` (absent → no state match, `graph.rs:557-590`). Fixed in D13, T3, T6, and the
> success criteria; no engine token added. Also removed a spurious T2→T3 edge so Wave 0 is
> genuinely parallel.
>
> **Revision 2** addressed the round-1 plan-review FAIL: fixed the contradictory `B`
> ownership (scaffold creates `C`+`P` only; `B` born at breakdown); added D13 for the
> preview/closure assertion split; confirmed R1; made R5 mandatory; clarified T10 replaces
> `breakdown_issue`'s wiring.
>
> **Revision 3** addresses the round-2 plan-review FAIL:
> - **Transitive traversal scope:** applies to `label-coverage` **only** —
>   `criteria-label-match` is also per-issue (same-issue stray-label detection), so it was
>   removed from T3 (over-correction in rev 2).
> - **Scope vs candidate exclusion (was the blocking gap):** D14 added. `validate --scope`
>   membership (whose `when` rules fire — `B` is **in** scope) is orthogonal to
>   `child-type-exclude` (which drops `B`/`P` only as coverage *candidates*). T2 now
>   specifies the slice includes `B` and the materialize-slice + select-rules mechanics.
> - **Wave-1 ordering:** T5 now depends on T4 and T6 (it applies the `plan-review` preset
>   T6 defines and resolves the plan-doc location T4 builds). T7/T8 depend on T1.

## Problem statement

Epic `94d26c42` fixed the *back end* of planning rigor: coverage / criteria rules are
lifecycle-scoped and now bite at the container `→ done` transition. The *front end* is
still unguarded. A container (epic, or whatever a ruleset's top type is) today conflates
three roles:

1. the planning artifact (a design doc, produced out-of-band),
2. the fan-out trigger (breakdown, run speculatively), and
3. the fan-in acceptance node (coverage at `done`).

Breakdown happens before anyone reviews whether the decomposition is right; plan review
lives entirely outside the graph. There is no checkpoint where a plan is reviewed and
approved before work fans out, and no place coverage can be checked at plan time rather
than at the very end.

## The bracket

A **breakable container** `C` (a type a ruleset *declares* as requiring planning) is
bracketed by two function-typed children:

- **`P` — `type:planning`.** Produces the plan; carries an **agent plan-quality gate**.
- **`B` — `type:breakdown`.** Created **by the breakdown step, not at scaffold**; carries
  a **deterministic coverage-preview gate** and a `brackets:<C-id>` label naming its
  container.

At scaffold only `C` and `P` exist. The breakdown step creates `B` and the impl subgraph
and splices them into the `C → P` link as a **spine**:

```
Scaffold:    C ──dep→ P
Breakdown:   C ──dep→ {impl subgraph} ──dep→ B ──dep→ P
```

Read as precedence `P > B > impl > C`: plan first, then breakdown, then the work, then
the container closes. The DAG is always kept in transitively-reduced form (an invariant
jit already maintains: `dependency.rs` reduces on `add_dependency`, `jit validate`
enforces/`--fix`es it repo-wide).

### Edge geometry (decided)

- **`C` → impl: sinks only.** Only the issues immediately before container completion
  carry a dependency edge to `C`. Reduced form drops any redundant `C → (non-sink)` edge.
- **impl → `B`: sources only.** Only entry impl issues (no intra-subgraph predecessor)
  depend on `B`; internal chains carry the rest. This transitively gates *all* impl
  behind the approved breakdown.
- **`C → P` after breakdown:** the scaffold's direct `C → P` edge is removed automatically
  by transitive reduction once the impl spine connects `C → impl → B → P`.
- **`B` → `P`:** `B` depends on `P` (breakdown after plan approved). `B` is created at
  breakdown, so this edge and `impl → B` appear together.
- **Retrofit (`jit plan <existing-C>`):** `C`'s pre-existing upstream dependencies
  **move onto `P`** (the front of the spine), so planning waits on upstream work and `C`
  becomes the pure closure node at the back of its own bracket.

### The consequence that drives the engine work

Because the DAG is reduced, `C` links only to its **sink** impl children, and the
`label-coverage` rule's traversal (`validation/graph.rs::children_of`) walks **direct
adjacency only** (this concerns `label-coverage` **only**; `criteria-to-check` and
`criteria-label-match` are both per-issue rules and are unaffected). So a `[hard]` criterion
satisfied by a *non-sink* impl issue currently reads as uncovered. This is a latent limitation in the existing coverage rule for *any* deep
hierarchy, made acute by the spine. Therefore:

**Coverage traversal becomes transitive by default**, and a new **`child-type-exclude`**
option both drops excluded-type nodes from candidates *and stops traversal through them*.
In a bracket the boundary is the `type:breakdown` node `B`, so coverage collects exactly
the impl interior between `C` and `B`; in a plain hierarchy it walks to the leaves. One
unified rule. (No back-compat mode: the only consumer is `../gf2`.)

## Cross-cutting decisions

| # | Decision |
|---|----------|
| D1 | **Enforcement = opt-in ruleset + skills.** Engine stays domain-agnostic; the SDD/research example rulesets declare breakable types and wire the gates; the skills re-sequence. No hard engine requirement. |
| D2 | **Ordering = dependency edges only.** No ordering rule kind; the spine wiring enforces order. |
| D3 | **Coverage at both ends.** Breakdown gate runs the coverage primitives over the drafted children (plan-time); the existing `→ done` transition enforcement stays (closure-time). |
| D4 | **Reject → drafts stay in Backlog** for revision. Archive only on explicit abandon. |
| D5 | **Function lives in the type axis, not a separate role label.** `type:planning` / `type:breakdown` are real types that *adopting rulesets* add to their hierarchy as valid children of breakable containers. No `role:` namespace. (jit types already mix altitude and function — `bug`, `chore`, `spike`.) |
| D6 | **`brackets:<C-id>` label on `B`** is the sole scope pointer for `validate --scope`; no longer doubling as a classifier. |
| D7 | **Coverage traversal is transitive by default**, bounded by `child-type-exclude` (drops a type from coverage *candidates* and halts the coverage walk through it). This bounds only the coverage rule's child traversal — distinct from `--scope` membership (D14). |
| D8 | **Plan-quality gate (agent) on `P`; coverage gate (deterministic) on `B`** via `jit validate --scope <C-id>` as the gate checker. Quality vs coverage split. |
| D9 | **Plan-doc location is configurable** (inline issue body *or* an external path template). File validation is mandatory: a **boundary** doc-location resolver loads the content and feeds the **pure** engine — the engine never touches the filesystem. |
| D10 | **Scaffolding: both** `jit issue create --with-planning` (bracket at creation) **and** `jit plan <id>` (retrofit an existing container). |
| D11 | **Examples on SDD and research** (goal→experiment, no epic) to prove domain-agnosticism. |
| D12 | **Skill changes in scope.** `jit-breakdown` and `project-lead` re-sequenced; configurable, reading breakable types from the ruleset. |
| D13 | **Preview vs closure are different assertions, one rule kind.** At breakdown time drafted children sit in Backlog, so the breakdown gate checks *mapping exists* (any child state) while the `→ done` closure check needs *mapping done* (`child-state = done`). State-projection cannot bridge this — the `child-state` filter itself differs. Both are instances of `label-coverage` differing only by the `child-state` knob: the closure instance sets `child-state = "done"`; the **preview** instance **omits `child-state` entirely** — the evaluator treats an absent `child-state` as "any state" (`graph.rs:557-590`); the literal `"any"` is *not* a valid token (`child-state` is validated as a lifecycle state, `rules.rs:393`). The preview instance is keyed on `type:breakdown` and resolves its container via `brackets:` (D6/T3), so it fires only at the transient `B` node, not on every `validate` of an in-progress `C`. |
| D14 | **Scope membership ≠ coverage-candidate exclusion.** `validate --scope <C>` builds the issue slice and decides whose `when` rules are *evaluated*; the preview rule lives on `B`, so `B` must be **in the slice** (its rule fires). `child-type-exclude` (D7) is a separate, coverage-rule-internal concern: when that fired rule walks `C`'s children to tally coverage, it drops `B`/`P` as *candidates* and halts the walk at them. So `B` is simultaneously an evaluation source (in scope) and a non-coverer (excluded candidate). The two traversals must not be conflated — this was the round-2 review's blocking gap. |

## Task breakdown (12 tasks, 5 waves)

Dependencies are intra-epic; `→` means "depends on".

**Wave 0 — engine primitives (parallel)**
- **T1 `[planning]` config block.** `breakable-types`, plan-doc location template, gate
  preset names; load + load-time validation. Files: `config.toml` loader, validation.
- **T2 `jit validate --scope <id>`.** Deterministic, exit-coded validation over a
  container's bracket subtree. **Scope membership** = the container's transitive dependency
  closure **including `B`** (bounded so it stops at `B`, the gate node; it need not pull in
  `P`/upstream). For each in-slice issue, evaluate the rules whose `when` selector matches
  it — so the preview rule keyed on `type:breakdown` runs because `B` is in scope. This is
  **orthogonal to `child-type-exclude`** (D14): scope decides whose rules fire; exclusion
  governs only the coverage walk's candidates. Mirror the materialize-slice + select-rules
  precision of transition enforcement (`commands/mod.rs:614-639`, `722-748`). Files:
  `commands/validate.rs`, `cli.rs`. (Independent of T3 — Wave 0 is genuinely parallel.)
- **T3 Transitive coverage + `child-type-exclude` + container indirection.** Make
  **`label-coverage`** traverse the transitive closure by default. Leave the per-issue rules
  untouched: `criteria-to-check` maps a criterion to the *same* issue's gates/labels
  (`graph.rs:1015-1091`) and `criteria-label-match` does same-issue stray-label detection
  (`graph.rs:1179-1244`) — neither traverses children, and adding traversal would change
  their semantics. Add
  `child-type-exclude` (excludes *and* halts traversal through listed types). Add
  **container indirection**: a `label-coverage` rule matching `B` resolves its
  criteria-bearing container from the `brackets:<C-id>` label, so a rule keyed on
  `type:breakdown` evaluates `C`'s criteria coverage (D13). The `child-state` knob already
  exists and an **absent** `child-state` already means "any state" in the evaluator
  (`graph.rs:557-590`), so the preview instance simply **omits** `child-state` (closure sets
  `"done"`) — no parser change, and `"any"` must *not* be introduced as a token. Files:
  `validation/graph.rs` (`children_of` → closure walk; container resolution),
  `validation/rules.rs`.

**Wave 1 — scaffolding & boundary.** Intra-wave order: T4 and T6 first, then T5 (it
consumes both). The waves are coarse buckets; the per-task `→` edges are authoritative.
- **T4 Doc-location resolver** (boundary). Resolve inline body vs external path template,
  load content, feed pure projection/validation. Files: boundary in `commands/`,
  `validation/` entry. (→ T1)
- **T6 Gate presets.** `plan-review` (agent, on `P`) + `coverage-preview` (on `B`,
  checker = `jit validate --scope <C>` with `C` resolved from `B`'s `brackets:` label). The
  coverage-preview *rule* is the preview instance of `label-coverage` (omits `child-state`
  = any state, D13); the gate just runs scoped validate. Files: gate preset registry. (→ T2, T3)
- **T5 Scaffolding command.** `jit plan <id>` + `jit issue create --with-planning`:
  create `P` (`type:planning`), wire `C → P`, move existing upstream deps to `P`, apply the
  plan-quality preset. **Does not create `B`** — that is the breakdown step's job (T10).
  Files: `cli.rs`, new `commands/plan.rs`. (→ T1, T4, T6 — needs the doc-location resolver
  to set `P`'s plan-doc location, and the `plan-review` preset it applies.)

**Wave 2 — examples & docs (→ engine)**
- **T7 SDD example** declares `epic` breakable, adds `planning`/`breakdown` types, both
  gates, `child-type-exclude`, lifecycle wording. (→ T1, T3, T6; end-to-end demo also
  exercises T5 scaffold.)
- **T8 Research example** (`goal` breakable, no epic) proves agnosticism. (→ T1, T3, T6.)
- **T9 Docs**: `docs/concepts` bracket explainer + how-to. (→ T5, T7)

**Wave 3 — skills (→ engine + examples)**
- **T10 `jit-breakdown` re-sequenced**: consume the approved plan → create `B`
  (`type:breakdown`, `brackets:<C-id>`, coverage-preview gate) depending on `P` → draft
  impl children in Backlog → wire the spine (sources → `B`, sinks → `C`; reduction drops
  `C → P`) → run the coverage-preview gate. **Replaces the spine wiring of the existing
  `breakdown_issue`** (`breakdown.rs:37-95`), which is parent-centric (copies parent deps
  to every child, makes the parent depend on *all* children) and does not produce
  source/sink-only spine edges — T10 adds a new breakdown path or reworks that helper and
  must not reuse its wiring as-is. (→ T5, T6)
- **T11 `project-lead` re-sequenced**: scaffold `C`+`P` → plan-quality gate → breakdown
  → coverage gate → implementation; read breakable types from the ruleset; configurable
  opt-out. (→ T5, T10)

**Wave 4 — eval (last)**
- **T12 Steering scenarios** reusing the `7aacfd89`/`6a6af4e3` harness: plan rejected at
  the quality gate; coverage gap caught at the breakdown gate; clean green path. (→ T10, T11)

## Success criteria

- A breakable container scaffolds to `C → P` (`P` carries a plan-quality gate); the
  breakdown step splices `B` and the impl subgraph into a reduced `C → impl → B → P` spine;
  `C`/`B`/`P` are correctly typed.
- The breakdown gate **blocks** when the drafted children leave a `[hard]` criterion
  uncovered — checked as *mapping exists* (the preview rule omits `child-state`, so children
  in any state count) over the impl interior, transitively (exit 4, findings shown). The
  `→ done` closure check separately requires *mapping done* (`child-state = "done"`).
- Coverage traversal sees non-sink impl issues; `child-type-exclude` excludes bracket
  nodes and halts descent at them.
- `jit plan <existing>` retrofits a container, moving its upstream deps onto `P`.
- The bracket is demonstrated on SDD (epic) **and** research (goal, no epic) with no
  `epic`/`planning`/`breakdown` strings in Rust.
- `jit-breakdown` and `project-lead` drive the full plan → review → breakdown → coverage
  → implement sequence; a steering scenario suite regression-tests it.

## Risks / open questions

- **R1 (RESOLVED) Type-hierarchy edges at the spine front.** Confirmed during plan review:
  type-hierarchy validation is label-only and does **not** validate dependency edges
  (`type_hierarchy.rs:368-382`; graph eval delegates to those label-only functions,
  `validation/graph.rs:1144-1172`). So `P`'s edge to moved upstream deps of arbitrary type
  raises no spurious findings. No action needed.
- **R2 `validate --scope` boundary vs whole-repo rules.** Repo-wide kinds
  (`label-uniqueness`, `scope = "all"`) are excluded from `--scope` exactly as they are
  excluded from transition-time enforcement (CC-2a). Document the limit.
- **R3 Drafted-children visibility.** Drafts sit in Backlog and depend on `B` (not done),
  so `query_ready` never surfaces them pre-approval — verify no path claims them early.
- **R4 Agent plan-quality gate harness.** Reuse the existing code-review-style
  command-backed agent gate mechanism; define the reviewer prompt/checker in T6.
- **R5 Type declaration has THREE synced sources when `.jit/rules.toml` exists.**
  Discovered while dogfooding: adding a type to `config.toml [type_hierarchy].types`
  updates the domain/graph hierarchy (`get_type_hierarchy`, so `jit validate` graph rules
  pass) but NOT the write-path `default:type-hierarchy-known` rule, which reads a *baked*
  enum in `.jit/schemas/default-type-hierarchy-known.json` (rules.toml is "the SOLE source
  when present"). So a new `type:planning`/`type:breakdown` must be added to BOTH the config
  hierarchy AND that frozen schema, or the write path warns. **T1 must make this fix
  mandatory**: regenerate `default-type-hierarchy-known.json` from `[type_hierarchy]` on
  config change, eliminating the dual source. Documenting alone is insufficient — an
  adopting ruleset would otherwise pass graph hierarchy while still warning on every write.
  This is a latent config/validation split independent of the bracket.
