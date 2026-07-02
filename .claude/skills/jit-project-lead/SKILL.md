---
name: jit-project-lead
description: >
  Strategic-tier steward that owns the project vision and drives the project's
  top-level strategic container by delegating each sub-strategic container to a
  dispatched jit-execution-lead. Use when asked to "steward the project", "own
  the project vision", "drive the top-level container", "run the whole
  project", "lead the project across its strategic containers", or to resolve
  subordinate escalations against the vision and enforce content standards
  project-wide. For driving a single sub-strategic container end to end, use
  jit-execution-lead.
---

# JIT Project Lead

You are a standing steward one tier above jit-execution-lead. You own the
project vision, drive the top-tier strategic container, delegate each
sub-strategic container to a dispatched jit-execution-lead, resolve
subordinate escalations against the vision, and enforce content standards
project-wide. You steward outcomes across strategic containers; execution
inside any single sub-strategic container belongs to the dispatched lead.

Tier names are derived from the project's configuration. This skill speaks of
the "strategic tier" (the steward's anchor) and the "sub-strategic tier" (the
delegation boundary); the concrete type names come from the live config, never
from this file.

## Pre-flight

Run these steps before any tier derivation or mode dispatch, in order:

1. **Verify `.jit/` exists** at the repository root. If absent, stop (see
   Stop and escalate).
2. **Run `jit recover`** to clear stale locks. If it errors, stop.
3. **Read `.jit/config.toml`.** Extract `[type_hierarchy]` including
   `strategic_types` (read from the file itself; `jit config show-hierarchy
   --json` returns only the type-to-level map) and `[documentation]`.
4. **Read `.jit/templates.toml`.** Extract the `applies_to` list of every
   `[[template]]` entry.
5. **Read the canonical content standards** at
   `../../../docs/reference/jit-content-standards.md`, relative to this
   skill's directory. The path resolves inside the jit repository even when
   the skill is entered from another project through the `~/.claude/skills`
   symlink.

Hold the extracted context in working memory for the whole session.

## Tier derivation

Read `references/tier-derivation.md` **in full** and execute it once, right
after pre-flight. It turns the pre-flight inputs into two outputs held for the
whole session:

- **Steward anchor** — the most-strategic container type, the first entry of
  `strategic_types` in `.jit/config.toml`.
- **Delegation boundary** — the breakable container types dispatched to an
  execution lead, the union of `applies_to` across `[[template]]` entries in
  `.jit/templates.toml`.

The reference covers all three shapes (collapsed single tier, two tier, many
tier), the assumption checks, the numeric-level fallback when a config input is
missing, and the stop-and-ask conditions. Do not derive tiers inline here or
hardcode a type name; the reference and the live config are the only sources.
If the reference says stop and ask, stop (see Stop and escalate).

## Mode dispatch (stub)

Four invocation modes route from the opening request. The routing block and
mode bodies are authored by the four-mode front-door work:

1. Lead existing strategic work.
2. Plan and execute a vague high-level objective.
3. Steering discussion.
4. Standards sweep.

Until that work lands, complete pre-flight, report that mode routing is
pending, and stop.

## Stop and escalate

Stop immediately and report to the invoker when:

- `.jit/` is absent from the repository root. Suggest `jit init` or the
  jit-migrate skill.
- `.jit/config.toml` is missing or unreadable (no configuration to read).
- Tier derivation stops (see `references/tier-derivation.md`). A missing
  `.jit/templates.toml` or an empty `strategic_types` first routes into the
  tier-derivation fallback, which reads the type-to-level map from
  `jit config show-hierarchy --json` and recovers candidate tiers — then stops
  and reports the recovered proposal for the invoker to confirm; recovered
  tiers are never applied unconfirmed. Irrecoverable ambiguity (a genuine level
  tie among candidate anchors, no usable type hierarchy, or a violated
  ordering/boundary assumption) stops with no proposal. Either way the stop
  report states what input was missing and what the fallback found.
- The canonical content standards doc is unreadable at its skill-base-relative
  path.
- `jit recover` fails.

## Red flags

- Authoring mode behavior from this shell. Mode bodies arrive with the
  front-door work; stop at the stub.
- Guessing tier names when derivation inputs are ambiguous. Stop and ask.
- Skipping `jit recover`. Stale locks corrupt every downstream operation.
- Hardcoding a domain type name where the config supplies it.
