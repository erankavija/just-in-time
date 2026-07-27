# Addressable items: use cases, extraction recipes, and the SSOT opportunity

**Status:** observations, post-epic (2821e177 done 2026-07-04)
**Context:** brainstorm after epic close; live population at time of writing: 427 items
(392 requirements, 13 gates, 9 rules, 7 invariants, 6 definitions) across
`jit item list | show/resolve | search` plus the link namespaces
(`satisfies:`, `per:`, `mitigates:`, `resolves:`, `enforces:`, `defines:`).

## 1. Use cases the uniform scheme opens

### Traceability chains

Items plus link labels form a typed graph over plain markdown/TOML:
invariant —`enforced-by`→ rule/gate; issue —`enforces:`→ rule; issue
—`satisfies:`→ requirement. This answers questions no single file holds:

- *"What mechanism enforces `@/invariant/INV-ATOMIC-WRITES`, and which work
  items last touched it?"* — resolve the invariant's bindings, find issues
  carrying the matching `enforces:` label, walk their commits via the `jit:`
  trailer convention.
- Coverage matrices: every `[hard]` criterion joined against the issues
  crediting it — the coverage-preview gate computes this at plan time; the
  same join works retrospectively for audits.

### Agent context injection (highest-value for jit's audience)

A dispatch prompt cites `@/invariant/INV-EVENT-LOG` or
`@/rule/coverage-preview` instead of pasting prose. The worker resolves the
address and gets the authoritative, current text. Citations never go stale the
way copied paragraphs do, and prompts shrink. The complement is verification:
extract every `@/…` address from a diff, commit message, or design doc and
fail on any that don't resolve — dangling-citation detection as a gate, the
same way `jit validate` already flags dangling `enforces:` labels.

### Impact analysis

`jit item search atomic` finds every requirement, invariant, and definition
mentioning atomicity before the atomic-write helper changes — a semantic
blast-radius estimate grep cannot give, because results are *addressable* and
can be cited in the change's own description.

### Documentation that resolves

The projection renderer already turns registries into reference docs
(`jit reference render` → `docs/reference/rules-and-gates.md`). Natural next
step: hyperlink any `@/…` address in rendered output to the item's rendering.
Definitions are the quiet enabler — glossary terms are addressable
(`@/definition/Assignee`), so docs and issue descriptions can cite a
controlled vocabulary that tooling can expand or link.

### Fleet-level, once multi-jit resolution lands

`@gf2/invariant/…` cited from another repo makes cross-project contracts
explicit: a downstream project pins the upstream invariant it depends on, and
a resolution check catches the upstream renaming or dropping it. Shared
definition registries would give a fleet-wide controlled vocabulary. The
grammar landed with the epic (REQ-09); only resolution is deferred.

## 2. What `jit item` extracts today

- **Inventory & density:** `jit item list --json | jq '[.items[].kind] |
  group_by(.) | map({(.[0]): length}) | add'` — kinds per project,
  requirements per issue; run over git history for trends.
- **Round-trip integrity:** every listed id resolves through `show` — already
  a test; also usable as a CI health probe.
- **Orphan detection by joining:** rules with `enforce = false` (declared but
  advisory), invariants with empty `enforced-by`, definitions never referenced
  by a `defines:` label, criteria with zero `satisfies:` credit.
- **Lexical entry point:** `jit item search <term>` across self-ids and text.

## 3. The SSOT opportunity — adoption is the missing half

The mechanism makes registries and issue sections the single source of truth,
but only if the surrounding tooling *cites addresses instead of copying text*.
Today the skills and docs still paste invariant/rule prose into prompts and
READMEs, which re-creates the drift problem the epic solved. Push adoption:

- **Skills** (`jit-manage`, `jit-breakdown`, `jit-parallel`,
  `jit-execution-lead`, `jit-planning-lead`, `jit-project-lead`): reference
  invariants, rules, and gates by `@/…` address in dispatch prompts, review
  protocols, and templates; instruct workers to resolve them via
  `jit item show` rather than trusting embedded copies. Lead review protocols
  can verify cited addresses resolve.
- **README and docs/**: present the addressing scheme and `jit item` as the
  way to cite project knowledge; every place that quotes an invariant or gate
  definition should carry its address so readers can resolve the current text.
- **CLAUDE.md**: the invariant projection already renders from the registry;
  extend the habit — new guidance should cite addresses
  (`@/rule/coverage-preview`) rather than restating rule behavior.
- **Conventions:** encourage `enforces:@/rule/…` / `enforces:@/gate/…` labels
  on work that changes enforcement mechanisms, and `@/…` citations in commit
  bodies where they add traceability.

The pattern to push everywhere: **write the fact once, in the registry or the
issue section that owns it; everywhere else, cite the address.**

## 4. Gaps / candidate follow-ups

- **Reverse-link query:** "who cites `@/rule/label-format`?" requires scanning
  labels manually; a `jit item backlinks <address>` would make the graph
  navigable in both directions.
- **`decision`/`risk` kinds hold zero items** — the first issue to author a
  `## Decisions` / `## Risks` section lights up `per:`/`mitigates:`
  traceability; worth dogfooding deliberately.
- **Rule descriptions:** filed as `8d2c5ff3` — descriptions make
  `jit item show @/rule/…` and the rendered reference genuinely informative.
- **Hyperlinked projections:** resolve `@/…` addresses into links when
  rendering reference docs.
