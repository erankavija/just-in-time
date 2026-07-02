# Tier derivation

Derive the steward's **anchor type** (most-strategic container the steward owns)
and its **boundary set** (the breakable container types it dispatches an
execution lead for) from the project's own configuration. Read type names only
from config; never hardcode a domain type name in a rule below.

Run this after pre-flight, once, and hold the result for the whole session.

## Inputs (from pre-flight)

- **STRATEGIC LIST** — the ordered `strategic_types` array from
  `.jit/config.toml`, most-strategic first (assumption A, checked below).
- **BOUNDARY SET** — the union of every `applies_to` array across all
  `[[template]]` entries in `.jit/templates.toml`.
- **LEVEL MAP** — the type-to-level map from `[type_hierarchy].types` in
  `.jit/config.toml` (lower number = more strategic). In the fallback only,
  obtain it from `jit config show-hierarchy --json`, dropping the non-type
  `message` key that command includes.

## Outputs

- **ANCHOR TYPE** — the steward anchor (one type name).
- **BOUNDARY SET** — the delegation boundary (one or more type names).
- **SHAPE** — one of: collapsed single tier, two tier, many tier. Shape
  governs how the steward operates, not what it emits; the outputs above are
  computed the same way in every shape.

## Procedure

1. **Guard the inputs.** If `.jit/templates.toml` is absent (BOUNDARY SET has
   no source) or STRATEGIC LIST is empty, do not derive on the primary path.
   Go to **Fallback**.
2. **Set the anchor.** ANCHOR TYPE = the first entry of STRATEGIC LIST.
3. **Set the boundary.** BOUNDARY SET = the `applies_to` union already gathered.
4. **Run the assumption checks** (below). Any violation stops derivation.
5. **Classify the shape.**
   - **Collapsed single tier** — STRATEGIC LIST has exactly one entry and that
     entry is a member of BOUNDARY SET (the anchor is itself the breakable
     container). ANCHOR TYPE and the sole boundary type are the same name. The
     steward's scope is the portfolio of top-level anchor containers, one
     execution lead per breakable anchor container; the vision is still
     project-wide.
   - **Two tier** — STRATEGIC LIST has two entries and the anchor is not a
     member of BOUNDARY SET. Anchor sits above the breakable tier; dispatch one
     execution lead per boundary container beneath it.
   - **Many tier** — STRATEGIC LIST has more than two entries. Anchor is still
     the first entry; BOUNDARY SET is still the `applies_to` union, ignoring any
     intermediate strategic tiers between anchor and boundary.
6. **Emit** ANCHOR TYPE, BOUNDARY SET, and SHAPE.

## Assumption checks

Both derive from the plan's Assumptions block. A violation means the ruleset
breaks an assumption the derivation depends on; stop and ask rather than guess.

- **A — strategic order.** STRATEGIC LIST is ordered most-strategic first: for
  each consecutive pair, the earlier entry's LEVEL MAP value is less than or
  equal to the later entry's. Equivalently, the anchor holds the minimum level
  among STRATEGIC LIST entries. If a later entry outranks an earlier one, the
  "first = anchor" rule is unsafe. **Stop and ask.**
- **B — boundary at or below anchor.** Every type in BOUNDARY SET has a LEVEL
  MAP value greater than or equal to the anchor's. A boundary type more
  strategic than the anchor inverts the tier relationship. **Stop and ask.**

## Fallback

Reached when `.jit/templates.toml` is missing or STRATEGIC LIST is empty. The
primary source of an output is gone, so the derivation cannot proceed
automatically; it must consult the LEVEL MAP and surface a proposal for
confirmation instead of applying one silently.

1. **Read the LEVEL MAP** from `jit config show-hierarchy --json` (drop the
   `message` key). If it is empty or the command fails, there is nothing to
   derive from. **Stop and ask.**
2. **Find the candidate anchor(s)** — the type(s) at the minimum level.
   - More than one type shares the minimum level: a genuine level tie among
     candidate anchors. **Stop and ask.**
   - Exactly one: that is the proposed anchor.
3. **Propose a boundary** — the type(s) at the next distinct level below the
   proposed anchor, or the anchor itself when no lower level exists (a collapsed
   shape).
4. **Do not apply the proposal.** Report the LEVEL-MAP-derived anchor and
   boundary as an unconfirmed proposal and **stop and ask** the invoker to
   confirm, because numeric levels record where a type sits, not which types are
   strategic anchors or breakable containers. A missing `strategic_types` or a
   missing template declaration is a configuration gap the human resolves.

## Stop and ask

Stop and report to the invoker, without guessing an anchor or boundary, when:

- `.jit/templates.toml` is missing or BOUNDARY SET is empty.
- STRATEGIC LIST is empty.
- The LEVEL MAP is empty or `jit config show-hierarchy --json` fails.
- Two or more types tie at the minimum level in the fallback (no unique anchor).
- Assumption check A fails (STRATEGIC LIST not ordered most-strategic first).
- Assumption check B fails (a boundary type is more strategic than the anchor).

The report states which condition fired and, where the fallback ran, the
LEVEL-MAP-derived proposal awaiting confirmation.

## Red flags

- Writing a domain type name (any concrete `type_hierarchy` type) into a rule
  above. Rules are placeholder-only; type names live in config and in the
  verification block below.
- Applying the fallback proposal without invoker confirmation.
- Skipping the assumption checks because the two observed rulesets happen to
  pass them.
- Treating the `message` key from `show-hierarchy --json` as a type.

---

## Worked verification (data — observed outputs, not rules)

Hand-executed against both in-repo rulesets. Concrete type names below are
quoted observed inputs and outputs with file:line cites; they are not part of
any rule.

### Ruleset 1 — this repository (two tier)

- STRATEGIC LIST = `["milestone", "epic"]` (`.jit/config.toml:31`).
- BOUNDARY SET = union of `applies_to` = `{ "epic" }` (`.jit/templates.toml:12`).
- LEVEL MAP = `{ milestone: 1, epic: 2, story: 3, ... }`
  (`.jit/config.toml:28`; confirmed via `jit config show-hierarchy --json`).

Trace: templates present and STRATEGIC LIST non-empty → primary path. Anchor =
first entry = `milestone`. Boundary = `{ epic }`. Check A: `1 <= 2` holds.
Check B: `epic` level `2` >= anchor `milestone` level `1` holds. Shape: two
entries and anchor not in boundary → **two tier**.

Result: **anchor = `milestone`, boundary = `epic`** (satisfies REQ-01).

### Ruleset 2 — research example (collapsed single tier)

- STRATEGIC LIST = `["goal"]` (`docs/examples/research/config.toml:34`).
- BOUNDARY SET = union of `applies_to` = `{ "goal" }`
  (`docs/examples/research/templates.toml:18`).
- LEVEL MAP = `{ goal: 2, experiment: 3, planning: 3, breakdown: 3 }`
  (`docs/examples/research/config.toml:31`).

Trace: templates present and STRATEGIC LIST non-empty → primary path. Anchor =
first entry = `goal`. Boundary = `{ goal }`. Check A: single entry, trivially
ordered. Check B: `goal` level `2` >= anchor `goal` level `2` holds. Shape:
exactly one entry and that entry is in the boundary → **collapsed single tier**.

Result: **anchor = `goal`, boundary = `goal`, one strategic tier** (satisfies
REQ-02). The steward's scope is the portfolio of top-level `goal` containers,
one execution lead per breakable `goal`.
