# Epic Complete: Addressing v2 — uniform path-based addresses; rule/gate as addressable items (2821e177)

**Started:** 2026-06-28 (created); execution sessions 2026-07-03 → 2026-07-04
**Completed:** 2026-07-04
**Assignee:** agent:jit-execution-lead

## Summary

One uniform, colon-free, kind-segmented address scheme (`@[<project>]/<kind>/<self-id>`, issue form `@/issue/<short-id>/<kind>/<self-id>`, with `<short-id>/<self-id>` input sugar) now addresses every knowledge item in jit; `rule` and `gate` became config-declared addressable kinds over `rules.toml`/`gates.toml`, invariant `enforced-by` bindings and `enforces:` work links resolve to real items, a generic projection renders the live rules/gates reference, the repository's own data and docs were migrated with a clean legacy cut (D11), and the multi-jit address form landed with local-only resolution.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 30 / 30 (6 stories, 22 tasks, 2 bracket nodes); 0 rejected |
| Waves executed | 15 (session 1: waves 1–8; session 2: 9–11; session 3: 11–15) |
| Rework cycles | 17 recorded dispatched reworks; several further findings lead-fixed directly |
| Escalations / invoker approvals | 10 touchpoints; 7 recorded plan/issue amendments (D3-A, D11-A, A-a1b6b3da, A-10db7a00, A-637764ef, A-7f22d6cf, A-7f22d6cf-v2) plus 943abd0b exclusions, 87346f52 clarification, and the 9a7106ae guidance reset |
| Sub-agent dispatches | ≈38 (22 initial implementation/doc/research dispatches + 16 rework rounds) |
| Issues created during execution | 2 (182aa0d5 in-epic; 8d2c5ff3 filed outside the epic) |

## Success Criteria

- [x] REQ-01 — uniform scheme minted, parsed, resolved (project, issue, sugar forms) — story 37506c12 (d6c3bff6, 5222aa47, 0910a381, f4b5e3f4) + 182aa0d5 (canonical minting, round-trip test)
- [x] REQ-02 — label-value grammar admits `@`-paths; single-colon split retained; re-split sites audited — story 0efbc594 (0597febe, 542f8b6d, 7a2bbe4f)
- [x] REQ-03 — `rule` as registry-first addressable kind; colon-free self-ids (origin split out; load-time colon rejection) — story 71ebd1e8 (87346f52, cdc33a0f, ae9dd95a)
- [x] REQ-04 — `gate` as addressable kind over migrated `gates.toml` with schema parity; no `gates.json` consumer remains — story bb7d57a2 (f5d35048, 943abd0b, 42898915)
- [x] REQ-05 — every invariant `enforced-by` binding resolves to a real item; resolution check fails on dangling bindings — 10db7a00
- [x] REQ-06 — `enforces:@/<rule-or-gate>` links resolve via the generic link scan; `[namespaces.enforces]` registered (live + init template) — d30695e4
- [x] REQ-07 — rules/gates reference rendered from the registries via the projection mechanism; live at `docs/reference/rules-and-gates.md` (`jit reference render`) — ebb6ad45
- [x] REQ-08 — all kinds re-addressed uniformly; repo data/docs migrated; validate green; operative-surface legacy grep clean (evidence: `dev/active/637764ef-acceptance-evidence.md`) — story 7f22d6cf (182aa0d5, c01f66dd, a8889340, 637764ef)
- [x] REQ-09 — multi-jit form accepted with resolution deferred: `[project] name` config (init-seeded, write-validated), bare `@`/`@<own-name>` resolve locally, `@<other>` returns a typed not-resolvable error — story 9a7106ae (3d9e9222, a1b6b3da)

Epic gates at close: cargo-ci **passed**, code-review **passed**, repo-validate **passed**.

## Wave Execution Log

- **Waves 1–5 (session 1):** address parser core, per-(scope,kind) uniqueness, sugar expansion, resolver rewire; story 37506c12 closed.
- **Waves 6–8 (session 1):** label-grammar widening + labels/defaults lockstep + re-split audit; story 0efbc594 closed.
- **Wave 9 (session 2, worktrees):** rule origin/id split, gates.toml store, `[project]` name config, named-project binding (amended A-a1b6b3da).
- **Wave 10 (session 2, worktrees; amended A-10db7a00):** rule kind, gates.json sweep, rule docs, gate kind, enforced-by rebind — red-validate window managed with deferred reviews.
- **Wave 11 (sessions 2–3):** story closes 71ebd1e8, bb7d57a2; 9a7106ae closed after a 4-round config-layering rework (`storage/config_store.rs` ← `commands/config.rs` ← thin CLI; every-write `project.name` validation).
- **Wave 12 (session 3, worktrees):** `enforces` link resolution (d30695e4); canonical minting + legacy kindless resolution removal (182aa0d5).
- **Wave 13 (session 3):** rules/gates reference projection (ebb6ad45); prior-epic doc sweep (c01f66dd); CLI help update (a8889340, largely pre-delivered by 182aa0d5's sweep, lead-finished).
- **Wave 14 (session 3):** acceptance evidence + stray-fix task 637764ef (evidence file linked; 2 stray legacy examples fixed).
- **Wave 15 (session 3):** story 7f22d6cf closed (4 review rounds, amendments A-7f22d6cf/-v2); epic gates green; epic done.

## Key Decisions

- Separate `AddressScope` type accepted over extending `Scope` (exhaustive matches at unmodifiable call sites) — worker judgment, lead-ratified.
- Projection target `docs/reference/rules-and-gates.md`; renderer mirrors the invariant-projection pattern including a CLI verb (`jit reference render`); live render dogfooded.
- Config persistence layering settled as storage store (`storage/config_store.rs`) ← command orchestration (`commands/config.rs`) ← thin CLI, after the reviewer enforced the boundary across four rounds.
- All parallel dispatches ran the manual worktree protocol (SHA-anchored worktrees + post-wave leak checks); zero worker leaks across three waves of parallel work.
- Historical records (dev/** process docs, done-issue descriptions, archives) are quoted, not rewritten: legacy forms survive only inside process records, per the invoker-set class boundary (A-7f22d6cf-v2).

## Escalations

- **f4b5e3f4 / D3 false premise (session 1):** item-backed sugar fallback (D3-A); legacy-path removal deferred to 7f22d6cf (D11-A). Approved.
- **a1b6b3da false premise (session 2):** rebind to `AddressScope::NamedProject` + own the resolver wiring; dep on 3d9e9222. Approved (A-a1b6b3da).
- **10db7a00 sequencing deadlock (session 2):** rewired before story closes to clear a red-validate window; reviews deferred until green. Approved (A-10db7a00).
- **943abd0b REQ-06 exclusions & 87346f52 init-scaffold clarification (session 2):** description amendments. Approved.
- **9a7106ae rework limit (session 3):** MAX reached + same-root-cause architecture finding ×3; invoker reset the counter with "proper scoping and discipline"; closed two rounds later.
- **637764ef exception classes (session 3):** REQ-03's enumerated exceptions didn't cover surfaced historical-record categories; exception list amended (A-637764ef). Approved.
- **7f22d6cf exception boundary (session 3):** after three rounds of exception-list whack-a-mole, the boundary was restated as a class — operative surfaces only (`crates/`, `docs/`, operative `.jit/`), all of `dev/**` exempt as process records (A-7f22d6cf, A-7f22d6cf-v2). Approved.

## Issues Discovered During Execution

- **182aa0d5** — Re-address minted qualified ids and remove the legacy kindless resolution path (created session 2: story 7f22d6cf's REQ-04 had no implementing task).
- **8d2c5ff3** — Rule descriptions enhancement (schema field, seeded texts, text-field flip, richer reference rendering) — filed 2026-07-04 at the invoker's request as a follow-up outside the epic, wired onto ebb6ad45.

## Holistic Quality Notes

- The stale-doc-comment finding class dominated rework across all three sessions; the only method that closes it in one round is a manual read of every doc block in touched files (regex sweeps reliably miss rephrased instances).
- Reviewers enforce the command/storage layering boundary progressively (one layer deeper per round); new persistence surfaces should land storage-first to avoid the 9a7106ae pattern.
- Enumerated exception lists in grep-style criteria lose to every fresh review round; class boundaries (e.g. "all of dev/**") are mechanically checkable and stable.
- One worker (637764ef) independently caught and corrected a defect in its own instructed grep exclusion pattern and flagged two scope judgment calls instead of guessing — the honest-evidence contract worked as designed.
- Cross-worker coherence held: the `config_store`/`gate_store`/`ruleset_store` storage pattern, kind-segmented address forms, and event-logging discipline are uniform across all six stories.

## Deliverables

- Design brief: `dev/studies/addressing-v2-rule-gate-items.md` (linked to epic)
- Plan with amendments: `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md`
- Acceptance evidence: `dev/active/637764ef-acceptance-evidence.md` (linked to 637764ef)
- Rendered reference: `docs/reference/rules-and-gates.md` (linked to ebb6ad45)
- Showcase deck: `dev/archive/features/2821e177/showcase/talk.html` (16 slides, fitcheck-clean)
- This report: `dev/archive/features/2821e177/completion-report.md`
