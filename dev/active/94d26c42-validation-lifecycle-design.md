# Design: Lifecycle-aware validation enforcement and steering quality

Epic: `94d26c42-3f4f-4a5e-99cc-0e82475a7635`

## Problem statement

A June 2026 evaluation of the shipped SDD example ruleset (run in an isolated repo) found that JIT's structural validation fires at the wrong lifecycle moments and produces findings an agent cannot act on. Concretely:

1. **Graph rules error during the entire planning phase.** `sdd-hard-criteria-covered` (label-coverage) and `sdd-req-is-satisfied` (label-reference, `req` → `satisfies`) both report `error` severity on a legitimately in-flight epic — one whose `[hard]` criteria simply are not done yet, and whose declared `req:` labels are not yet satisfied by completed children. An author following the methodology sees a wall of errors for correct work, which trains agents to ignore validation.
2. **The one moment enforcement should bite is unguarded.** The write path runs only **local** rules (`evaluate_local`); graph rules run only in `jit validate` and gate checkers. A `--state done` transition therefore evaluates no coverage or reference-integrity rule. Verified: an epic with a declared `req:` label, an uncovered `[hard]` criterion, and zero children transitions to `done` with exit 0.
3. **Stray and unsatisfied `req:` labels are indistinguishable.** `label-reference` (`req` → `satisfies`) compares label values only against other labels. A fabricated `req:REQ-77` and a legitimate not-yet-implemented `req:REQ-01` produce the identical rule, message, and severity. The ruleset comments already concede this limit ("no rule kind compares a label value to ids extracted from description prose").
4. **Schema findings are content-free.** When section items fail to parse (prose without bullets), the json-schema rule emits `[] has less than 1 item` and `None of [] are valid under the given schema` with no section name or field path. A typo'd heading surfaces only as a missing-property error. Regex feedback renders raw tab and zero-width control characters.
5. **The engine lacks primitives** for criteria-to-check mapping, evidence freshness, and cross-graph (repo-wide) analysis, and there is **no way to measure** whether rule changes improve agent steering.

The epic closes all five gaps: lifecycle-scoped rule selection, transition-time blocking enforcement, stray-req disambiguation, actionable messages, three new engine primitives, two reworked/new example rulesets, and a two-tier steering eval (deterministic harness + LLM agent).

## Architecture recap (what exists today)

The validation engine lives in `crates/jit/src/validation/`:

- `rules.rs` — `Rule`, `Selector`, `Assertion` enum, `RuleSet` loader, `Scope` (`Local`/`Graph`). `Selector` **already has a single `state: Option<String>` field** matched in `matches_state`, but the state name is **not validated at load** and only a single value is accepted.
- `desugar.rs` — lowers the four local shorthand kinds to JSON Schema (Draft 2020-12).
- `engine.rs` — `SchemaEngine` compiles/caches validators by schema identity; `validate` and `validator_for` produce `Finding { rule, severity, message }` where `message = error.to_string()` from `jsonschema::ValidationError`. `x-jit-*` custom-keyword extension point exists (`with_keyword`).
- `local.rs` — `evaluate_local(issue, &RuleSet, repo_format) -> LocalEvaluation`. Write-path entry; skips graph rules; lazily parses body only when a rule needs `sections`. `LocalEvaluation::{blocking_rules, warnings, rejection_message}`.
- `graph.rs` — `evaluate_graph(&[&Rule], &[Issue], &HierarchyConfig, ContentFormat) -> Vec<GraphFinding>`. Pure function over the full issue slice; the caller reads the store. `GraphFinding { issue_id: Option<String>, finding }`; `config-error` findings carry `issue_id: None`. Per-kind evaluators: `evaluate_label_coverage`, `evaluate_label_reference` (supports `scope = "global" | "linked"`), `evaluate_dependency_shape`, `evaluate_type_hierarchy`. `criterion_ids(...)` extracts ids from a projected section — **this is where stray-req id extraction already lives**.
- `report.rs` — `RuleReport`, `ReportedFinding`, `ExplainReport`, `RuleOutcome`.

Command integration in `crates/jit/src/commands/`:

- `mod.rs::validate_for_write(issue, force)` — single write-time entry; calls `evaluate_local`, blocks on `enforce` error findings (returns an `anyhow` error whose message contains "blocked by", mapped to exit 4).
- `validate.rs::{run_rules, explain_rules, evaluate_graph_rules}` — `jit validate` plumbing; `render_selector` (used by `--explain`), `group_messages`.
- `issue.rs::update_issue` and `update_issue_state` — the two transition paths. State is resolved into the in-memory issue **first** (deps + gate guards), then `validate_for_write` runs against the final shape. **Graph rules are never invoked here today.**
- `errors.rs::TransitionBlockedError` — already downcast in `main.rs::error_to_exit_code` to `ExitCode::ValidationFailed` (= 4). The natural carrier for transition-time graph blocking.

Domain: `GateState { status, updated_by, updated_at: DateTime<Utc> }` on `Issue.gates_status: HashMap<String, GateState>` — `updated_at` is the recorded gate timestamp the recency predicate reads. `jsonschema = "0.46"`. Exit code 4 = `ExitCode::ValidationFailed`.

---

## Cross-cutting design decisions

These bind multiple tasks and are decided here so implementers do not diverge. Genuinely architectural forks are deferred to **Risks and open questions** with a recommended option.

### CC-1: `when` state predicate is a list, validated at load

`Selector.state` becomes a small enum-backed type that accepts a **single state or a list**, AND-combined with `type`/`label`/`has_doc_type` exactly as today. TOML grammar:

```toml
when = { type = "epic", state = "in_progress" }      # single
when = { type = "epic", state = ["ready", "in_progress", "gated"] }  # list
```

State names are validated **at config load** (in `RawRule::into_rule` / a `Selector` post-parse pass), rejecting unknown names with a new `RuleConfigError::InvalidState { rule, value, valid }` whose message lists the valid snake_case tokens (`backlog`, `ready`, `in_progress`, `gated`, `done`, `rejected`, `archived`). `--explain` already renders the selector via `render_selector`; extend it to render a list (`state=ready|in_progress`) and the per-rule `RuleOutcome` already shows whether the rule matched (a non-matching rule does not appear in `matching_rules`, so it produces no outcome — acceptable; see CC-1a).

**CC-1a (decided):** `--explain` lists only rules whose selector matches. To satisfy "shows whether the state predicate matched," add the matched state to the rendered selector string and keep the existing behavior (matched rules appear, non-matched do not). This is the minimal change; a full "all rules with match/no-match" view is out of scope (see R-1).

### CC-2: Transition-time graph-rule evaluation (blocking)

Hook point: **`issue.rs`**, in both `update_issue` (after the state is resolved into the in-memory issue and **after** `validate_for_write` succeeds) and `update_issue_state` (the `State::Done` and `State::Ready` arms). A new executor helper:

```rust
fn enforce_transition_graph_rules(&self, issue: &Issue, target: State, force: bool)
    -> Result<Vec<String>>  // returns non-blocking warning messages
```

Behavior:

- Selects graph rules whose `when` selector matches the issue **in its target state** (the issue is passed with `state = target` already applied). This reuses CC-1 so a rule with `when = { state = "done" }` runs only at the done transition.
- Evaluates them over a **neighborhood-scoped** issue slice, not the whole repo (see CC-2a).
- A rule with `enforce = true` and severity `error` producing a finding **attributed to this issue** blocks the transition: return a `TransitionBlockedError` carrying a new `TransitionBlocker::GraphRule { rule, message }` variant. This maps to exit 4 via the existing downcast, prints the findings, and persists nothing.
- Non-enforcing or non-error findings are returned as warnings (surfaced, not blocking), matching `evaluate_local`'s split.
- `--force` bypasses blocking and logs a bypass event (reuse `Event::new_local_rule_bypassed` or add `Event::new_graph_rule_bypassed`; **decided:** add a dedicated `GraphRuleBypassed` event so the audit log distinguishes write-path from transition-path bypass).
- A **blocked transition is recorded** in the event log: add `Event::TransitionBlocked { issue_id, target, rule }` appended **before** returning the error (the transition is the auditable act). Decided: log on the blocking path only, one event per blocking rule.
- Interaction with existing gate/dependency guards: those run **first** (unchanged). Graph-rule enforcement runs **after** them and after local `validate_for_write`, so the ordering is deps → gates → local rules → graph rules. A done transition diverted to `Gated` (unpassed gates) enforces against the **gated** target state — done-keyed rules do not fire on the diversion, gated-keyed rules block it (amended during review; the original design exempted the diversion entirely).

**CC-2a Neighborhood scope (decided):** "graph neighborhood" = the issue itself plus its transitive dependency graph in both directions (the closure used by `linked_issues` extended transitively), capped to the connected component. Implementation: build the slice via the existing `DependencyGraph` reachability from the issue, then evaluate only that slice. This keeps a done transition from re-scanning the whole repo while giving coverage/reference rules the children they need. Rules authored with the new repo-wide `scope = "all"` (CC-6) are **skipped** at transition time (they are inherently whole-repo and belong in `jit validate`); document this limit.

### CC-3: Stray-req disambiguation

New graph rule kind **`criteria-label-match`** (decided name; methodology-agnostic). It compares a label-namespace's values against criterion ids extracted from a configured section — closing the gap the SDD comments concede. Config shape:

```toml
[[rules]]
name = "sdd-req-matches-a-criterion"
when = { type = "epic" }
severity = "error"
assert = { criteria-label-match = {
  namespace = "req",                    # label namespace to check (required)
  criteria-section = "success_criteria",# projection slug (optional, default success_criteria)
  marker = "[hard]",                    # optional: only ids on marked items count
  id-pattern = "REQ-[0-9]+",            # optional, default [A-Z][A-Z0-9]*-[0-9]+
} }
```

- Id extraction **reuses `graph.rs::criterion_ids`** (already the projection-layer extractor) so id-format handling is identical to coverage. No new extraction path.
- For each `namespace:<value>` label on a matching issue, if `<value>` is **not** in the extracted criterion-id set: emit a finding distinct from coverage —
  `"label 'req:REQ-77' on issue <short> names no criterion in section 'Success Criteria' (stray or invented)"`.
- The **declared-but-unsatisfied** finding stays with `label-coverage` / `label-reference` and keeps its existing wording (`"criterion 'REQ-01' ... is not satisfied by any ... child"`). The two are now textually and structurally separate findings from separate rules.
- Unit tests (in `graph.rs`): stray id (label not in section), matched id (no finding), and the **id-format-mismatch** case — criterion text `REQ-03` vs label `req:REQ-3` must report a stray (no silent normalization; exact string compare on the extracted id).

This is `Scope::Graph` (needs the body projection but is per-issue; it does not need other issues, so it is cheap and is safe to run at transition time under CC-2a).

### CC-4: Message-quality changes (all in `engine.rs`, one new render path)

jsonschema `0.46` `ValidationError` exposes `instance_path: Location`, `schema_path`, and a `kind: ValidationErrorKind`. Today `local.rs` and `engine.rs` use `error.to_string()`. Introduce a single `fn render_finding_message(error: &ValidationError, schema: &Value) -> String` in `engine.rs`, called from both `SchemaEngine::validate` and `local.rs`'s `iter_errors` loop (refactor the latter to call the engine helper so the rendering lives in one place):

1. **Empty-projection special case.** When the error is the `minItems`/`contains` failure on a `sections.<slug>.items` array that is empty (the prose-without-bullets case), detect it from `instance_path` ending in `.../items` with an empty instance and emit:
   `"section '<Heading>' has no list items; items must be Markdown bullets (lines starting with '- ')"`. The heading is recovered from the slug via a reverse map the projection already knows (pass the section's original heading through, or de-slugify; **decided:** thread the heading by having `criteria`/section schemas carry the original heading in an `x-jit-section-heading` annotation the renderer reads — no lossy de-slugify).
2. **Instance-path inclusion.** All other findings prepend the readable instance path: `"at sections.success_criteria.items: <message>"`. Empty path renders as the projection root.
3. **Readable character classes.** Before emitting any regex into a message, pass it through a `humanize_regex` helper that replaces control/zero-width chars with their escape (`\t`, `​`) and renders common classes readably (`\s` stays `\s`, never a literal tab). Applies to `label-value-pattern` findings and any `pattern` keyword failure.
4. **did-you-mean for `require-section`.** When a `require-section` schema fails because the required slug is absent, compare the required heading against the **headings actually present** in the projection (available in `sections` keys) using Levenshtein distance; if the nearest is within threshold **≤ 2 edits OR ≤ 20% of heading length** (whichever is larger), append `did you mean '<Present Heading>'?`. Decided algorithm: Levenshtein on the slugified forms (so `Sucess Criteria` vs `Success Criteria` matches), threshold as above. This needs the present section headings, so `require-section` rendering must receive the projection (it already validates against it).

The empty-projection and did-you-mean cases require the renderer to see both the schema and the projection; pass both into `render_finding_message`.

### CC-5: New rule kinds — criteria-to-check mapping and gate-result recency

**CC-5a `criteria-to-check` (task 690f618a).** New `Scope::Graph` kind (per-issue, like CC-3). Asserts every criterion in a section maps to a verifiable check. "Check" is defined two ways, either of which satisfies a criterion:

```toml
assert = { criteria-to-check = {
  criteria-section = "success_criteria",
  marker = "[hard]",                  # optional filter
  id-pattern = "REQ-[0-9]+",          # optional
  # a criterion id is "checked" if EITHER:
  gate-prefix = "verify:",            # the issue carries a required gate keyed verify:<id>, OR
  check-namespace = "checks",         # the issue carries a label checks:<id>
} }
```

- A criterion is mapped if the issue has a `gates_required` entry `"<gate-prefix><id>"` **or** a label `"<check-namespace>:<id>"`. At least one of `gate-prefix`/`check-namespace` must be configured (else config-error).
- Unmapped criterion finding: `"criterion 'REQ-01' has no verification: expected gate 'verify:REQ-01' or label 'checks:REQ-01'"` (names the id and what is missing).
- **No methodology names in engine code.** "Nyquist" appears only in the example ruleset/docs, never in Rust. The kind is generic criteria-to-check.

**CC-5b `gate-recency` (task 765688e1).** New kind. Asserts the issue's gate results are no older than a configured age, read from `GateState.updated_at`.

```toml
assert = { gate-recency = {
  max-age-days = 7,        # required (or max-age-hours)
  gates = ["code-review"], # optional: which gates; default = all gates_required
} }
```

- **Clock injection is mandatory.** The pure engine must not read wall-clock. `evaluate_graph`'s signature gains a `now: DateTime<Utc>` parameter threaded to the recency evaluator (the caller in `validate.rs`/`issue.rs` passes `Utc::now()`; tests pass a fixed instant). This is the cleanest injection point because `evaluate_graph` is the single graph entry. (Alternative: a `Clock` trait — rejected as heavier than one parameter; see R-2.)
- Finding for a stale/missing gate: `"gate 'code-review' result is <N> days old (max 7)"` or `"gate 'code-review' has no recorded result"`. Age computed as `now - updated_at`.
- Deterministic and testable via the injected `now`.

Both kinds are `Scope::Graph` and thus available at transition time (CC-2) and in `jit validate`.

### CC-6: Repo-wide graph scope (`scope = "all"`)

Add a third value to the existing `scope` config key on the graph kinds that resolve across issues — **`label-reference`** and the new **`criteria-label-match`** (where cross-epic id collisions are meaningful). Current values `"linked"` / `"global"` stay; add `"all"` as an explicit synonym semantics-wise for `label-reference` (it already has `global`). The **new** capability is a repo-wide collision/duplication check expressed via `criteria-label-match` or a thin `label-uniqueness` kind:

**Decided:** ship the cross-epic example on a new kind **`label-uniqueness`** rather than overloading `scope` on existing kinds, because "the same id is declared by two unlinked parents" is a *uniqueness* assertion, not a reference one:

```toml
assert = { label-uniqueness = {
  namespace = "req",   # each value in this namespace must be declared by at most one matching issue
  scope = "all",       # required: repo-wide
} }
```

- Semantics: group all matching issues by each `namespace:<value>`; any value owned by ≥2 issues yields one finding naming the value and the colliding issue short-ids.
- **Performance:** single pass — one `HashMap<value, Vec<short_id>>` build over the issue slice, then report groups with len ≥ 2. O(n) in issues × labels, no N² scan. Document that repo-wide rules run only in `jit validate` (not at transition time, CC-2a) and are measured against a hundreds-of-issues fixture in the harness (task 3a86f34e success criterion).
- `scope = "all"` documented in `docs/reference/configuration.md`.

### CC-7: Reworked SDD example (task 490f1f99)

Sketch of the new `docs/examples/sdd/rules.toml` rule set:

```toml
# Local (write-path), unchanged — always-on structure:
sdd-epic-has-criteria-section   require-section            enforce=true
sdd-criteria-are-well-formed    json-schema spec-body.json enforce=true
sdd-req-id-format               label-value-pattern        error

# Stray-req disambiguation (CC-3), always-on (cheap, per-issue):
sdd-req-matches-a-criterion     criteria-label-match { namespace="req", marker="[hard]" }  error

# Coverage + derivation — LIFECYCLE-SCOPED so planning is quiet:
[[rules]]
name = "sdd-hard-criteria-covered"
when = { type = "epic", state = "done" }        # only at/after done
severity = "error"
enforce = true                                  # blocks the done transition (CC-2)
assert = { label-coverage = { marker = "[hard]", child-state = "done", child-link = "dependents" } }

[[rules]]
name = "sdd-req-is-satisfied"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { label-reference = { from = "req", to = "satisfies", scope = "linked" } }

[[rules]]
name = "sdd-satisfies-references-a-req"
when = { label = "satisfies:*" }                # any state; a dangling satisfies is always wrong
severity = "warn"
assert = { label-reference = { from = "satisfies", to = "req", scope = "linked" } }
```

Result: an in-flight epic (state `in_progress` with incomplete children) matches none of the coverage/derivation `state = "done"` rules, so `jit validate` yields **zero error findings** (success criterion 1). The stray-req rule still fires on a fabricated `req:`. A `done` transition runs coverage + derivation under CC-2 and **blocks** if a `[hard]` criterion is uncovered (success criteria 2). Example comments and `docs/concepts/validation-engine.md` are rewritten to describe: local-on-write, lifecycle-scoped graph rules, transition-time enforcement.

### CC-8: Non-software example (task 872cdccd)

**Decided domain: research program.** New `docs/examples/research/` modeling `goal` → `experiment` → `evidence` with no epic/milestone:

- Types: `type:goal`, `type:experiment` (configured hierarchy, no epic/milestone references).
- Sections: a goal body has `## Hypotheses` (each `H-N:`) and `## Success Criteria`; an experiment has `## Method` and `## Evidence`.
- Label namespaces: `hyp:` on goals (declared hypotheses), `tests:` on experiments (which hypothesis an experiment tests), `evidence:` for recorded evidence.
- Rules demonstrate state predicates (a goal's hypothesis-coverage rule is `state = "done"`-scoped) and transition enforcement (a goal cannot reach `done` until each `[hard]` hypothesis has a `done` experiment that `tests:` it), reusing `label-coverage` and `criteria-label-match` with research namespaces. No rule references epic/milestone types. Added to the examples index and the concepts doc alongside SDD/bug-repro/release-checklist.

### CC-9: Steering-scenario harness (task 7aacfd89) and LLM eval (task 6a6af4e3)

**Harness placement (decided):** a new **integration** test file `crates/jit/tests/steering_scenarios.rs` that drives the **real `jit` binary** in isolated temp repos (the eval scenarios assert real exit codes and real CLI output, which the in-process harness cannot exercise for the binary's exit-code path). It runs in CI as part of the standard `cargo test` integration layer.

- **Data-driven format:** scenarios live as fixtures under `crates/jit/tests/fixtures/steering/<name>/`: a `setup.jsonl` (sequence of `jit` commands to build the repo state) or a `scenario.toml` describing `{ ruleset = "sdd", steps = [...], expect = { exit, contains = [...], enforcement_point = "write|validate|transition" } }`. Decided: one `scenario.toml` per scenario; the test file enumerates the directory and runs each — **adding a scenario requires only a new fixture dir, no Rust** (success criterion 4).
- Each scenario asserts: the finding **messages** (substring match list), the process **exit code**, and the **enforcement point** (which command surfaced it: a create/update write, `jit validate`, or a `--state` transition).
- Seed scenarios (from the eval): sloppy spec body, prose-without-bullets, typo'd heading (`Sucess Criteria`), stray `req:` label, pending (legitimate) `req:` label, premature `done` transition.

**LLM eval (decided):** a separate runnable under `dev/eval/steering/` (a script + harness, **not** a `cargo test`; it needs network/agent access and is not deterministic). It reuses the **same fixture `scenario.toml` files** as the deterministic harness so the two stay in lockstep. Each run:

- Spins up a fully isolated temp repo (`jit init` in a tmpdir; **never** touches a production `.jit` — enforced by setting the repo root to the tmpdir and asserting cwd isolation, per the project memory on eval repo isolation).
- Drives a real agent: present the scenario's failing command + the **error text only**, let the agent attempt a fix, re-run, repeat until green or a cap.
- Records **iterations-to-green** and **rule-compliance** per scenario, reports the **mean over ≥3 runs** with per-scenario stability (always / sometimes / never complies).
- Documented re-run procedure so results are comparable after rule/message changes.

The `5a25c590` success criterion ("sloppy-epic scenario reaches a valid write in ≤2 iterations using only error text") is asserted by the LLM eval, with the deterministic harness guaranteeing the *message content* the agent relies on.

---

## Per-task design

### cca3e80b — State predicates in rule selectors

**Files:** `validation/rules.rs` (Selector type + load-time validation + `RuleConfigError::InvalidState`), `commands/validate.rs` (`render_selector` for lists), `docs/how-to/validation-rules.md`, `docs/reference/configuration.md`.

Change `Selector.state` from `Option<String>` to `Option<StatePredicate>` where `StatePredicate` deserializes from either a string or a list of strings (serde `untagged` or a custom `Deserialize`). `matches_state` returns true if the issue's state token is in the set. Validate each name against the seven `state_token` values at load (new error variant). Keep AND-combination with other dimensions. Update `render_selector` to join a list with `|`. Per CC-1/CC-1a.

### bc86f54c — Blocking graph-rule evaluation at state transitions

**Files:** `commands/issue.rs` (both transition paths), `commands/mod.rs` (new `enforce_transition_graph_rules` helper next to `validate_for_write`), `errors.rs` (new `TransitionBlocker::GraphRule` variant + constructor + Display + JSON), `domain/types.rs` (new `Event::TransitionBlocked` + `Event::GraphRuleBypassed` constructors), `docs/concepts/validation-engine.md`, example `rules.toml` comments.

Implements CC-2/CC-2a. The helper: resolve effective graph rules, filter by `when.matches(issue_with_target_state)`, skip `scope = "all"` kinds, build the neighborhood slice via `DependencyGraph` reachability, call `evaluate_graph(rules, &slice, &hierarchy, repo_format, now)`, partition findings into blocking (enforce + error + attributed to this issue) vs warnings, return `TransitionBlockedError` on blocking unless `force`. Append `TransitionBlocked` events before erroring; append `GraphRuleBypassed` on a forced override after the save.

### 59782dde — Distinguish stray req labels from unsatisfied ones

**Files:** `validation/rules.rs` (new `Assertion::CriteriaLabelMatch { config }`, raw-assert wiring, scope = Graph), `validation/graph.rs` (new `evaluate_criteria_label_match`, reusing `criterion_ids`), `validation/desugar.rs` (return `None` for the new kind), `docs/examples/sdd/rules.toml`. Per CC-3. Unit tests in `graph.rs` cover stray, matched, and `REQ-03` vs `req:REQ-3`.

### 5a25c590 — Actionable validation error messages

**Files:** `validation/engine.rs` (`render_finding_message`, `humanize_regex`, did-you-mean), `validation/local.rs` (call the shared renderer instead of `error.to_string()`), `validation/desugar.rs` (emit the `x-jit-section-heading` annotation on `require-section`/section schemas so the renderer can name the heading). Per CC-4. Add unit tests for each of the four message behaviors.

### 490f1f99 — Lifecycle-aware SDD example ruleset

**Files:** `docs/examples/sdd/rules.toml` (rewrite per CC-7), `docs/examples/sdd/schemas/` (unchanged unless the heading annotation is added there), `docs/concepts/validation-engine.md`, `crates/jit/tests/example_rulesets_tests.rs` (update SDD assertions for the new lifecycle behavior). Depends on cca3e80b, bc86f54c, 59782dde.

### 690f618a — Criteria-to-check mapping rule kind + Nyquist example

**Files:** `validation/rules.rs` (`Assertion::CriteriaToCheck`), `validation/graph.rs` (`evaluate_criteria_to_check`), `validation/desugar.rs` (None), new `docs/examples/nyquist/rules.toml` (+ schemas), `docs/examples` index, `example_rulesets_tests.rs`. Per CC-5a. No methodology names in Rust.

### 765688e1 — Gate-result recency predicate + fresh-evidence example

**Files:** `validation/rules.rs` (`Assertion::GateRecency`), `validation/graph.rs` (`evaluate_gate_recency` + thread `now`), `validation/graph.rs` signature change to `evaluate_graph(..., now: DateTime<Utc>)` and **all its callers** (`commands/validate.rs::evaluate_graph_rules`, `commands/issue.rs` transition helper, `example_rulesets_tests.rs`, `graph.rs` doctests/tests), new `docs/examples/fresh-evidence/rules.toml`, docs index. Per CC-5b. This signature change touches every graph caller — sequence it early or coordinate (see conflict map).

### 3a86f34e — Repo-wide graph rule scope + cross-epic example

**Files:** `validation/rules.rs` (`Assertion::LabelUniqueness`, accept `scope = "all"`), `validation/graph.rs` (`evaluate_label_uniqueness`, single-pass), new `docs/examples/cross-epic/rules.toml`, `docs/reference/configuration.md` (scope semantics), `example_rulesets_tests.rs` + a performance assertion fixture (hundreds of issues). Per CC-6.

### 872cdccd — Non-software example ruleset

**Files:** new `docs/examples/research/` (rules.toml + schemas), `docs/examples` index, `docs/concepts/validation-engine.md`, `example_rulesets_tests.rs`. Per CC-8. Depends on cca3e80b (state predicates) + bc86f54c (transition enforcement) being usable; uses 59782dde/690f618a kinds.

### 7aacfd89 — Deterministic steering-scenario harness

**Files:** new `crates/jit/tests/steering_scenarios.rs`, new `crates/jit/tests/fixtures/steering/*/scenario.toml`. Per CC-9. Depends on all behavior tasks (it regression-tests their output) — sequence last among the behavior work.

### 6a6af4e3 — LLM-agent steering eval

**Files:** new `dev/eval/steering/` (harness script + README procedure), reuses `fixtures/steering/*/scenario.toml`. Per CC-9. Depends on 7aacfd89 (shared fixtures) and the behavior tasks.

---

## Key decisions (trade-offs)

- **State predicate as a list type, validated at load** (CC-1): catches typo'd states immediately rather than silently never-matching. Chosen over leaving `state` a free string.
- **Reuse `TransitionBlockedError` for transition-time blocking** (CC-2): it already maps to exit 4 and renders remediation; a new error type would duplicate the exit-code plumbing.
- **Neighborhood = transitive dependency closure** (CC-2a): bounds transition-time cost while still seeing children; repo-wide rules are explicitly excluded from the transition path and live in `jit validate`.
- **`now` as a parameter to `evaluate_graph`** (CC-5b): minimal, keeps the engine pure, no `Clock` trait ceremony. Cost: a one-line signature change rippling to all callers.
- **`criteria-label-match` and `label-uniqueness` as new kinds** (CC-3, CC-6) rather than overloading existing kinds: each assertion stays single-purpose, findings stay distinct, and `scope` semantics don't get muddied.
- **Single message renderer in `engine.rs`** (CC-4): both `local.rs` and `engine.rs` route through it, so message quality is fixed in one place and cannot drift.
- **Methodology lives in examples, not engine** (CC-5a, CC-8): no "Nyquist"/"research" strings in Rust; the engine ships generic primitives.
- **Deterministic harness as a binary-driving integration test; LLM eval as a separate non-CI runnable sharing fixtures** (CC-9): CI stays deterministic; the agent eval is reproducible but lives outside the test gate.

## Implementation order notes (file-conflict map)

Tasks that touch `crates/jit/src/validation/`:

| Task | rules.rs | graph.rs | engine.rs | local.rs | desugar.rs | report.rs |
|------|----------|----------|-----------|----------|------------|-----------|
| cca3e80b (state predicates) | **edit** (Selector, error) | — | — | — | — | — |
| 59782dde (stray-req) | **edit** (new Assertion) | **edit** (evaluator) | — | — | edit (None arm) | — |
| 5a25c590 (messages) | — | — | **edit** (renderer) | **edit** (call renderer) | edit (heading annot.) | — |
| 690f618a (criteria-to-check) | **edit** (new Assertion) | **edit** (evaluator) | — | — | edit (None arm) | — |
| 765688e1 (gate-recency) | **edit** (new Assertion) | **edit** (evaluator + `now` sig) | — | — | edit (None arm) | — |
| 3a86f34e (repo-wide scope) | **edit** (new Assertion/scope) | **edit** (evaluator) | — | — | edit (None arm) | — |

**Hot files:**

- **`rules.rs`** is touched by cca3e80b, 59782dde, 690f618a, 765688e1, 3a86f34e — all adding an `Assertion` variant + `RawAssert` field + `into_assertion` branch + scope derivation. These conflict on the same enum and `RawAssert` struct. Recommend: land cca3e80b first (it changes `Selector`, not `Assertion`), then serialize the four new-kind tasks through `rules.rs` (each appends one variant) — or have one engineer own the `rules.rs` enum additions for all four in a single coordinated change while others write the `graph.rs` evaluators.
- **`graph.rs`** is touched by 59782dde, 690f618a, 765688e1, 3a86f34e — each adds an independent `evaluate_*` fn and a `match` arm in `evaluate_one`. The per-fn additions are parallel-safe; the shared conflict is the `evaluate_one` dispatch match and (for 765688e1) the `evaluate_graph` signature. Recommend: **land 765688e1's `now` signature change first** so the other three rebase onto the final signature, then dispatch the three evaluators in parallel.
- **`desugar.rs`** — each new kind adds itself to the `None` arm of `desugar`; trivial, serialize or merge-resolve.
- **`engine.rs` / `local.rs`** — only 5a25c590 touches these; no conflict with the kind tasks. Can run fully in parallel with the `rules.rs`/`graph.rs` work.

Command/error/event files: bc86f54c owns `issue.rs`, `mod.rs` (new helper), `errors.rs`, `domain/types.rs` events — isolated from the validation-module tasks except it consumes cca3e80b (state matching) and the new `now` parameter from 765688e1.

Example/docs/test tasks (490f1f99, 872cdccd, 7aacfd89, 6a6af4e3 and each kind's example) edit `docs/examples/**` and `example_rulesets_tests.rs`. `example_rulesets_tests.rs` is a shared hot file across every example task — coordinate or split per-methodology test modules.

**Recommended dispatch waves:**
1. cca3e80b (Selector) and 5a25c590 (messages) — independent, parallel.
2. 765688e1 first lands the `evaluate_graph` `now` signature; then 59782dde, 690f618a, 3a86f34e evaluators in parallel (with `rules.rs` enum additions coordinated).
3. bc86f54c (transition enforcement) once cca3e80b + 765688e1 signature are in.
4. Example/docs tasks (490f1f99, 872cdccd, 690f618a/765688e1/3a86f34e examples) after their kinds exist.
5. 7aacfd89 then 6a6af4e3 last (regression + eval over finished behavior).

## Success criteria (from the epic)

- A correctly authored in-flight parent issue with declared `req:` labels and incomplete children produces zero error-severity findings from `jit validate`.
- An issue cannot transition to `done` while an enforcing graph rule fails for it (blocked, exit 4, findings shown).
- A `req:` label whose id is absent from the canonical criteria section yields a finding distinct from a declared-but-unsatisfied one.
- Schema findings name the failing section/field; the empty-section case states that items must be Markdown bullets.
- Criteria-to-check mapping, gate-result recency, and repo-wide graph scope are each available as engine primitives and demonstrated by an example ruleset under docs/examples/.
- A non-software example ruleset demonstrates the engine on a hierarchy without epics or milestones.
- A deterministic steering-scenario suite runs in CI, and an LLM-agent eval measures iterations-to-green against the same scenarios.

## Risks and open questions

- **R-1 (`--explain` non-matching rules):** The spec asks `--explain` to "show whether the state predicate matched." Today `--explain` lists only matching rules. Options: (a) keep current behavior and just render the matched state in the selector string (recommended, minimal); (b) list every rule with a match/no-match flag and the reason (larger change to `explain_rules` + `RuleOutcome`). Recommend (a); revisit if agents need the full view.
- **R-2 (clock injection shape):** `now` as an `evaluate_graph` parameter (recommended) vs a `Clock` trait on the executor. The parameter ripples to all callers once; a trait is more invasive but reusable if other time-dependent rules appear. Recommend the parameter; promote to a trait only if a second time-dependent consumer emerges.
- **R-3 (transition-time scope vs gate-recency at done):** A `gate-recency` rule scoped `state = "done"` and `enforce = true` would block done on stale evidence — desirable for the fresh-evidence example, but it means a slow review makes a previously-passing issue un-completable. Recommend documenting this as intended and keeping recency rules `warn` unless a methodology explicitly opts into blocking (the fresh-evidence example uses `enforce = true` to demonstrate the blocking path; real rulesets choose).
- **R-4 (repo-wide rule cost at scale):** `label-uniqueness` `scope = "all"` is O(n) but runs over `store.list_issues()` on every `jit validate`. For thousands of issues this is acceptable; the task mandates a measured fixture. Open question: whether to cache the cross-issue index across rules in one validate run (recommended optimization if multiple repo-wide rules coexist) — defer until a second repo-wide kind exists.
- **R-5 (LLM eval determinism/cost):** The agent eval is inherently variable and may need network/model access; it is deliberately outside CI. Open question: which agent/model the eval drives and where it runs (local vs cloud). Recommend pinning a model in the eval README and reporting the model id with results so comparisons across rule changes are apples-to-apples.
- **R-6 (`scope = "all"` overlap with `global`):** `label-reference` already has `scope = "global"`. Adding `"all"` risks two names for one concept. Recommend: do **not** add `"all"` to `label-reference`; reserve `"all"` for the new `label-uniqueness` kind only, and document `global == whole repo` for `label-reference` to avoid two synonyms.
