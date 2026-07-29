# Epic Complete: Structured project knowledge in jit: addressable items, qualified ids, and projected invariants (25064508)

**Started:** 2026-06-25
**Completed:** 2026-06-26
**Assignee:** agent:project-lead

### Summary

Generalized jit's regex-parsed "requirements" into a first-class, domain-agnostic **addressable-item** model: four item kinds (`requirement`, `decision`, `risk` issue-scoped/markdown-first; `invariant` project-scoped/registry-first) with globally-unique qualified ids `<scope>/<self-id>`, a `jit item list/show/search/resolve` CLI, an `.jit/invariants.toml` registry that projects into a configurable doc target, and a bidirectional enforcement-drift check exposed via `jit invariant render/check`.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 14 / 14 implementation issues (+ planning `0d753ba5` and breakdown `5f5735c3`, pre-completed) |
| Waves executed | 7 |
| Rework cycles | 15 (across all issues) |
| Escalations | 1 |
| Sub-agent dispatches | 29 (14 initial + 15 rework) |
| Issues created during execution | 0 |

### Success Criteria

- [x] REQ-01: globally-unique qualified id `<scope>/<self-id>`, derived not stored, scope = issue short-id or `@` — **56ab0224** (issue scope) + **61b17603** (project `@` scope)
- [x] REQ-02: kinds declared as the six-tuple in config; no kind name hardcoded in engine logic — **56ab0224** + **1e1ea81d** (six-tuple + `kind=` sugar)
- [x] REQ-03: per-kind source-of-truth direction (markdown-first vs registry-first) — **1e1ea81d** (field) + **b629686b/be3eb6ca** (markdown) + **21558ace/69bd8cd5** (registry)
- [x] REQ-04: requirement/decision/risk/invariant all implemented, indexed, queryable across both substrates — **b629686b**, **be3eb6ca**, **93480b00**, **205472c4**, **69bd8cd5**, **72cdf315** (cross-substrate demo)
- [x] REQ-05: invariants in `.jit/invariants.toml` with statement/kind/enforced-by — **21558ace** (schema+loader) + **69bd8cd5** (acceptance)
- [x] REQ-06: invariants project into a configurable doc target, no hardcoded doc filename — **0d593d90** (region + separate-file modes, config-driven)
- [x] REQ-07: bidirectional enforcement-drift check runnable via validate — **52ad3b1f** (declared-but-unenforced + enforced-but-undeclared, built-in pass)
- [x] REQ-08: generic node→item link namespaces over qualified ids; label-coverage/criteria-to-check shown compatible — **56ab0224** + **b9966826** (cross-kind link resolution) + **b121fd51** (coverage-rule compatibility)
- [x] REQ-09: `jit item list/show/search/resolve` + `jit invariant render/check` — **56ab0224** (`item`) + **0d593d90** (`render`) + **52ad3b1f** (`check`)
- [x] REQ-10: indexes/qualified ids are pure projections; graceful degradation — **56ab0224** + **61b17603**

### Wave Execution Log

**Wave 1** (1 issue): `56ab0224` — foundational addressable-item model, qualified ids, `jit item` CLI, generic link namespaces, label-coverage compatibility (incl. qualified-link crediting in the validation engine).
**Wave 2** (2 issues): `61b17603` scoped addressing with `@` project scope via a config-driven markdown source through `IssueStore::read_repo_file`; `1e1ea81d` six-tuple kind registry + additive `kind=` coverage sugar.
**Wave 3** (4 issues): `21558ace` invariants.toml schema + config-time loader (both paths); `b629686b` decision kind (built-in default); `be3eb6ca` risk kind (built-in default); `b121fd51` coverage-rule compatibility assertions.
**Wave 4** (2 issues): `93480b00` invariants indexed as a project-scoped registry-first kind (`@/<self-id>`, registry authoritative); `205472c4` decision+risk cross-kind acceptance.
**Wave 5** (1 issue): `69bd8cd5` end-to-end registry-first invariant acceptance.
**Wave 6** (3 issues): `0d593d90` configurable doc projection (region/separate-file) + `jit invariant render` + path-safe `IssueStore::write_repo_file`; `b9966826` cross-kind link resolution + dangling-link validation finding; `72cdf315` cross-substrate generality demo.
**Wave 7** (1 issue): `52ad3b1f` bidirectional enforcement-drift check (built-in validate pass) + `jit invariant check`.

### Key Decisions

- **Project-scope source (escalated, user-approved):** `@/<self-id>` resolves via a config-driven markdown project-scope source in `61b17603`; the invariant registry adds a registry-first project source alongside it in Group C. (See Escalations.)
- **Kinds ship as built-in defaults:** decision/risk/invariant ship as domain-layer built-in default kinds (mirroring `requirement_default`), so `jit item list --kind <k>` works out of the box, while the validation engine stays kind-name-literal-free. Required by the literal reading of each kind's "returns items" criterion.
- **`jit invariant render` placed in `0d593d90`:** the projection engine had to be production-reachable, so the `render` CLI landed with the projection (Wave 6) rather than waiting for `52ad3b1f`; `52ad3b1f` added `check` + drift.
- **Enforcement drift is a built-in validate pass gated on registry presence:** `jit validate` reports drift whenever `.jit/invariants.toml` exists (no opt-in rule needed) and stays dormant otherwise, so the live repo and its CI are unaffected. An unloadable enforcement source is reported as drift, not a hard error.
- **Storage-boundary discipline:** project-source reads/writes go through `IssueStore::read_repo_file`/`write_repo_file` (path-validated, canonicalized against symlink escape, atomic), never direct `std::fs` in command/validation layers.

### Escalations

- **Project-scope item source for `61b17603` (architectural).** `@/<self-id>` had to resolve in the real CLI, but the only project source the epic defined (the invariant registry) is a later issue that already depends on `61b17603` (so it couldn't be a dependency without a cycle). Presented three options; the user chose a **config-driven markdown project-scope source**, with the invariant registry adding a registry-first project source later. Implemented as approved.

### Issues Discovered During Execution

No additional work items were created. One pre-existing observation logged for follow-up:
- The default-repo namespace-registry rule does not include the item link namespaces (`satisfies`/`per`/`mitigates`/`resolves`/`enforces`), so such labels also trigger a `default:namespace-registry` validation warning. Pre-existing and separate from this epic's item-link feature; candidate follow-up.
- `storage::heartbeat::tests::test_background_thread_updates_heartbeat` is a pre-existing flaky timing test (occasionally trips under gate-run load; passes on re-run). Not introduced by this epic.

### Holistic Quality Notes

- **The model proved genuinely generic.** Adding the `decision` and `risk` kinds required essentially zero engine changes — they slotted in as domain-layer defaults over the same `index_items`/`resolve_link_label` paths, validating the foundation's "no kind name in engine logic" design.
- **Layer boundaries held under pressure.** Two separate reviews caught direct filesystem I/O leaking into the command/validation layers; both were routed back through new path-safe `IssueStore` methods, keeping persistence behind storage.
- **Strict, literal criteria enforcement drove most rework.** 15 rework cycles across 8 issues, almost all from the code-review gate reading criteria literally (kinds must ship, not just be configurable; `@/X` must resolve in production; drift must surface in `jit validate`; `# Examples` on every public API). Quality is high as a result; the dominant lead lesson was to relay the *entire* review finding set (including non-headline Documentation notes) each round.
- All four kinds, both substrates, resolve through one code path (asserted by `72cdf315`); the validation engine contains no kind-name literal in production logic.
