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

Reached when `.jit/templates.toml` is missing (BOUNDARY SET has no source) or
STRATEGIC LIST is empty. The fallback always ends in a stop: it gathers the
LEVEL MAP and recovers candidate tiers first, so the stop report proposes
something concrete instead of a bare "config missing" — but recovered tiers are
a **proposal for the invoker to confirm, never an applied result**. Keep
whichever primary input is present authoritative when recovering the other.

1. **Read the LEVEL MAP** from `jit config show-hierarchy --json` (drop the
   `message` key). If it is empty or the command fails, there is no usable
   hierarchy to derive from. **Stop and ask** (report that the input was missing
   and the hierarchy was unusable).
2. **Recover the anchor.**
   - STRATEGIC LIST is present: ANCHOR TYPE = its first entry.
   - STRATEGIC LIST is empty: ANCHOR TYPE = the unique type at the minimum
     level. If two or more types share the minimum level, that is a genuine
     level tie among candidate anchors. **Stop and ask** (report the tie and the
     tied types).
3. **Recover the boundary.**
   - BOUNDARY SET is present (templates existed; only STRATEGIC LIST was empty):
     keep it.
   - BOUNDARY SET has no source (templates missing): BOUNDARY SET = the type(s)
     at the next distinct level below the anchor, or the anchor itself when no
     lower level exists (a collapsed shape).
4. **Run the assumption checks** on the recovered ANCHOR TYPE and BOUNDARY SET,
   exactly as the primary path does. A violation stops derivation with no
   proposal.
5. **Report and stop for confirmation.** Present the recovered ANCHOR TYPE,
   BOUNDARY SET, and SHAPE to the invoker as a proposal: state which input was
   missing, what `show-hierarchy` returned, and how each output was recovered.
   Do not apply the proposal or proceed past derivation until the invoker
   confirms it. On confirmation, emit the confirmed values and record that they
   came from the numeric-level fallback so downstream reports note the
   recovered (not config-declared) source.

## Stop and ask

Only the primary path (both config inputs present, assumption checks passing)
emits tiers without the invoker. Every other outcome stops and reports. Two
stop flavors:

**Stop with a proposal** — the fallback recovered candidate tiers from the
LEVEL MAP. Report the missing input, what `show-hierarchy` returned, and the
recovered ANCHOR TYPE / BOUNDARY SET / SHAPE; wait for the invoker to confirm
before applying them. A missing `.jit/templates.toml` or an empty STRATEGIC
LIST always lands here (or below, if recovery also fails): route it through
**Fallback** first so the stop report is informative, never stop bare on the
missing input alone, and never apply the recovered tiers unconfirmed.

**Stop with no proposal** — the derivation is irrecoverably ambiguous:

- The LEVEL MAP is empty or `jit config show-hierarchy --json` fails (no usable
  hierarchy) — reached via the fallback.
- Two or more types tie at the minimum level with no STRATEGIC LIST to break the
  tie (no unique anchor) — reached via the fallback.
- Assumption check A fails (STRATEGIC LIST not ordered most-strategic first).
- Assumption check B fails (a boundary type is more strategic than the anchor).

Every stop report states which input was missing (if any) and what the fallback
found before it stopped.

## Red flags

- Writing a domain type name (any concrete `type_hierarchy` type) into a rule
  above. Rules are placeholder-only; type names live in config and in the
  verification block below.
- Stopping bare the moment a config input is missing instead of routing it
  through the fallback first, so the stop report carries no recovered context.
- Applying a fallback-recovered anchor or boundary without the invoker's
  confirmation — the fallback proposes, the invoker decides.
- Skipping the assumption checks because the two observed rulesets happen to
  pass them.
- Treating the `message` key from `show-hierarchy --json` as a type.

---

## Worked verification

For a two-tier configuration, suppose STRATEGIC LIST is
`["portfolio", "initiative"]`, BOUNDARY SET is `{ "initiative" }`, and LEVEL
MAP assigns levels `1` and `2` respectively. Both assumptions hold, so the
anchor is `portfolio`, the boundary is `initiative`, and the shape is two tier.

For a collapsed configuration, suppose STRATEGIC LIST is `["objective"]`,
BOUNDARY SET is `{ "objective" }`, and LEVEL MAP assigns `objective` level
`1`. Both assumptions hold, so anchor and boundary are both `objective` and
the shape is collapsed single tier.
