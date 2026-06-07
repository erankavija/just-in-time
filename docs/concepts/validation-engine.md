# Methodology-Agnostic Validation

> **Diátaxis Type:** Explanation
> **Audience:** Users and contributors who want to understand why validation is configuration, not code

JIT validates issues with ONE declarative engine driven by `.jit/rules.toml`. It
deliberately bakes in NO methodology. This document explains that choice and what
it buys you.

## The principle

A methodology is a set of conventions about *what makes an issue well-formed*:
"every epic states its success criteria", "every bug has reproduction steps",
"a release can't ship without QA sign-off". Different teams — and different kinds
of work — disagree about these conventions, and rightly so.

If JIT hard-coded one methodology, it would impose that team's conventions on
everyone. Instead, JIT provides a single, general validation primitive and lets
each project express its own conventions as **configuration**:

- conventions live in `.jit/rules.toml` (and `.jit/schemas/*.json`), versioned
  in git alongside the issues they govern;
- the engine is a pure function from *(ruleset, issue projection)* to
  *findings*; it has no notion of "requirements", "bugs", or "releases".

This mirrors JIT's broader [design philosophy](design-philosophy.md): the core
stays domain-agnostic so the same tool serves software, research, and knowledge
work without privileging any one of them.

## One primitive, many methodologies

Everything reduces to JSON Schema validation over a normalized **projection** of
the issue (its type, state, labels grouped by namespace, document types, and the
parsed sections of its description). Ergonomic shorthand kinds
(`require-label`, `require-section`, …) desugar to JSON Schema; graph kinds
(`label-coverage`, `label-reference`, `dependency-shape`) extend the same model
across the dependency DAG. There is no methodology subsystem to grow — only
rules.

Because the conventions are data, a methodology is *shipped*, not *built*. JIT
ships several worked examples (under `docs/examples/`) precisely to demonstrate
that the engine carries none of them intrinsically:

| Example                                                            | Methodology it encodes                                |
|-------------------------------------------------------------------|-------------------------------------------------------|
| [`sdd/`](../examples/sdd/rules.toml)                              | Spec-Driven Development: canonical success criteria, derived `req:`/`satisfies:` coverage and reference-integrity |
| [`bug-repro/`](../examples/bug-repro/rules.toml)                  | Bug triage: every bug must document how to reproduce it |
| [`release-checklist/`](../examples/release-checklist/rules.toml)  | Release gating: checklist, release notes, QA sign-off dependency |

SDD is the *first* application, not a privileged one. The bug-triage and
release-checklist examples use the **same grammar and the same engine** with zero
code changes — that equivalence is the whole point.

## Spec-Driven Development as configuration

SDD is worth calling out because it exercises the engine's harder features while
illustrating a discipline many teams want: the description is the **single source
of truth**, and labels are **derived** from it.

- An epic's `## Success Criteria` section lists criteria, each marked `[hard]` or
  `[aspirational]` and carrying a stable id (`REQ-01`).
- `req:<id>` (on the epic) and `satisfies:<id>` (on children) are derived from
  those criteria — never an independent source.
- A `label-coverage` graph rule treats the canonical criteria as the *source
  set* and checks that each `[hard]` one is satisfied by a child.
- A `label-reference` graph rule checks the inverse: every `satisfies:` resolves
  to a declared `req:`.

If JIT had hard-coded SDD, none of this would be inspectable or changeable. As
configuration, a team can tune it (require `[aspirational]` coverage too, change
the id format, point coverage at "any" issue rather than dependents) or replace
it wholesale.

## Why this matters

- **No lock-in to a workflow.** Adopt SDD, adapt it, or ignore it for a
  checklist or triage discipline — all without forking JIT.
- **Conventions are reviewable.** A ruleset is a small, diffable file; changing
  what "well-formed" means is a normal pull request, not a JIT release.
- **The engine stays small.** New methodologies add example files, not engine
  code, so the validation surface does not grow per workflow.

## See Also

- [How-To: Author Validation Rules](../how-to/validation-rules.md) — write your own rules, with the examples walked through
- [Design Philosophy](design-philosophy.md) — JIT's domain-agnostic core
- [Custom Gates](../how-to/custom-gates.md) — run validation as a quality gate
