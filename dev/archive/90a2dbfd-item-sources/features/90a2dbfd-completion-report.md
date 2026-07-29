# Epic Complete: Generalize addressable items to declarative kinds over markdown/toml sources (90a2dbfd)

**Started:** 2026-06-26
**Completed:** 2026-06-27
**Assignee:** agent:project-lead

### Summary

The addressable-items engine is now a fully declarative "kinds over sources" system: the binary bakes in no domain kind name, `jit init` authors the kind set as an editable `[item_kinds]` table, registry-first kinds (invariant) resolve through a config-declared toml source descriptor instead of a reserved-name branch, enforcement-drift collapsed to its single meaningful direction, and CLAUDE.md's invariants section is a projection of the registry with a prose render style.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 9 / 9 (planning + breakdown + 7 impl tasks) |
| Waves executed | 4 (impl interior; bracket P/B pre-completed) |
| Rework cycles | 5 (all stale-docs; +1 atomic-write) |
| Escalations | 0 |
| Sub-agent dispatches | 7 fresh + 4 rework instructions |
| Issues created during execution | 1 (REQ-08, owner-requested) |

### Success Criteria

- [x] REQ-01 — config-declared markdown kind indexes from an arbitrary `.md`, isolated integration test — `241a0c03`
- [x] REQ-02 — registry-first kind backed by an arbitrary `.toml` via a declared field mapping — `5ff3a6ae`
- [x] REQ-03 — reserved `invariant` name retired; invariant routes through the toml descriptor — `f6318259`
- [x] REQ-04 — `jit init` emits an editable `[item_kinds]` table; no domain kind-name literal in the indexing path — `fbe0a401`
- [x] REQ-05 — enforced-but-undeclared drift direction dropped; `jit invariant check` exits 0 on the seed — `73d98a98`
- [x] REQ-06 — invariant registry projected into a region of CLAUDE.md — `96f0c39f`
- [ ] REQ-07 — rule/gate-as-item: **aspirational, deferred** (D7: blocked by the `labels.rs` value-grammar `:` collision; out of the hard floor)
- [x] REQ-08 (owner-added) — prose `id-anchor` render style for the CLAUDE.md projection — `8b293a0b`

### Wave Execution Log

**Wave 1:** 4 issues (REQ-01, REQ-02, REQ-05, REQ-06) — parallel worktree dispatch; the markdown contract test, the generic toml source descriptor, drift simplification, and the CLAUDE.md region projection.
**Wave 2:** 1 issue (REQ-03) — retire the reserved name; route `invariant_default` through the toml descriptor (`.jit/invariants.toml`, no link-fields → byte-identical items).
**Wave 3:** 1 issue (REQ-04) — demote baked defaults; `jit init` authors the complete `[item_kinds]` table (+ dogfood `definition` over glossary.md); migrate this repo's config; golden init test.
**Wave 4:** 1 issue (REQ-08, owner-added) — config-driven render `style` (`full` default, `id-anchor` prose); CLAUDE.md region re-rendered under a restored `### Domain Invariants` heading.

### Key Decisions

- **Parallel worktree dispatch for Wave 1** (4 independent, disjoint-file tasks) via the mandated dispatch script anchored on current HEAD; serial integration + gating thereafter (cargo-ci lease tests flake under concurrency).
- **Rebase each subsequent worktree onto current main before gating** — gives clean per-issue code-review diffs, validates the real combined state in cargo-ci, and makes every merge a fast-forward (no events.jsonl append-conflicts).
- **Reinstall the global `jit` from the worktree before code-review** for REQ-05 and REQ-04 — `jit-validate.sh` runs the globally-installed binary; a stale one produced a false enforced-but-undeclared finding and would have failed an otherwise-correct change.
- **Owner render-style request handled as a new in-epic task (REQ-08)**, not a scope-silent change and not a deferral; the engine-side `style` field defaults to `full` so machine-facing/separate-file output is unchanged.
- **Lead-captured the final REQ-04 doc/atomic-write fixes** when its worker entered a checklist-replay loop and stopped committing; the fixes were small, fully specified, and verified by re-gating.
- **Coherence touch-ups:** removed the vestigial `### Domain Invariants` header orphaned by REQ-06's projection; REQ-08 restored a clean hand-authored heading above the markers.

### Escalations

No escalations were required.

### Issues Discovered During Execution

- `8b293a0b` — Add a prose (id-anchor) render style for the CLAUDE.md invariant projection (created during Wave 3, reason: the owner reviewed the REQ-06 projection output and requested a prose-leaning render with the id as a readable anchor; tracked as a gated in-epic task rather than an untracked tweak).

### Holistic Quality Notes

- **Recurring stale-documentation pattern:** every behavior-change task (REQ-05, REQ-03, REQ-04) first failed code-review *solely* on stale prose describing the old behavior — most often CLI help text (`cli.rs`) and `config.rs` doc comments, which workers sweep last. The lead now pre-sweeps the whole tree for invalidated wording before invoking the gate; saved as a durable lesson to memory.
- **Decomposition coupling handled cleanly:** REQ-03 needed the invariant kind to carry a toml descriptor while REQ-04 (which removes the baked `invariant_default`) had not yet run — resolved by baking the descriptor into `invariant_default` in REQ-03 and removing the whole baked-defaults layer in REQ-04, with the kinds-vs-rules "built-in default" naming trap called out explicitly so doc sweeps did not corrupt the still-valid rule docs.
- **No indexing regression:** the 129 pre-existing addressable items (122 requirement + 7 invariant, with empty invariant links) are byte-identical before and after; the migration added 6 dogfood `definition` items from the glossary.
- **The engine is now genuinely domain-agnostic for items:** no `requirement`/`decision`/`risk`/`invariant` literal drives indexing; a different project redefines the kind set purely in config.
