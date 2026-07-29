# Epic Complete: Documentation accuracy and audience boundary cleanup (287c4051)

**Started:** 2026-07-06
**Completed:** 2026-07-10
**Assignee:** agent:jit-execution-lead

## Summary

Corrected verified drift between jit's shipped documentation and its source tree, and made the boundary explicit between adopter-facing documentation (`docs/`) and jit's own contributor documentation (`dev/`, `CLAUDE.md`). A `doc-review` quality gate now enforces both properties and passes on the documentation tree.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 12 / 12 |
| Waves executed | 5 |
| Rework cycles | 8 (across 5 issues) |
| Escalations | 4 |
| Sub-agent dispatches | 25 (8 wave-1 + 8 wave-1 reworks + 3 wave-2 + 1 wave-3 + 4 wave-4 + 1 wave-5) |
| Issues created during execution | 1 |
| doc-review gate rounds | 10 total: issue-scoped 21→5→9→0, epic-scoped 10→3→1→2→8→0 (59 findings, all fixed) |

## Success Criteria

- [x] **REQ-01** — Every shipped doc statement about CLI behavior, storage layout, and repository structure matches the current source tree — delivered by `a6e1ae3d` (CLI command reference), `d48906de` (storage-format layout), `313a7f3a` (git-requirement claims), `287f7bc9` (mcp-server README), `c29118f9` (TESTING.md), `f1aaaca7` (stale storage comment), and wave-5 gate rework on `6ce0f14e` (labels.md CLI surface, storage migration semantics, control-plane layout).
- [x] **REQ-02** — Adopter-facing docs describe the shipped surface only; repo-local dogfood configuration is relocated to `dev/` or explicitly signalled — delivered by `c7f0f690` (charter → `dev/vision/`, skill evals → `dev/eval/`), `f7c50538` (landing pages), `5514e4f5` (dev index).
- [x] **REQ-03** — Shipped docs state current behavior only, with no legacy narration — delivered by `7eec3d59` (reference-doc sweep) and gate-driven rework on `6ce0f14e` (waves 4–5). Verified by `doc-review` passing with zero findings.
- [x] **REQ-04** — A documentation review gate exists, is required on this epic, and passes before the epic completes — delivered by `6ce0f14e`. Gate `doc-review` is registered in `.jit/gates.toml`, required on this epic, and passing.
- [x] **REQ-05** — Every diagram in `docs/` is a Mermaid block; ASCII-art diagrams are forbidden — delivered by `f5eb11a2`. Verified independently: the only box-drawing characters remaining in `docs/` are directory trees and verbatim CLI output; all arrow syntax lives inside `mermaid` fences.

## Wave Execution Log

**Wave 1:** 8 issues — dependency-free doc corrections across disjoint files: CLI reference completion, self-documentation relocation, TESTING.md rewrite, storage-format layout, git-requirement claims, ASCII→Mermaid conversion, mcp-server README, and a stale storage comment plus `.gitignore` fix.

**Wave 2:** 3 issues — sequenced after wave 1 by file conflict: legacy-narration sweep of shipped reference docs, dev-index repair, docs landing-page consolidation.

**Wave 3:** 1 issue — the `doc-review` gate itself (`6ce0f14e`), depending on all eleven cleanup tasks.

**Wave 4:** 21 gate findings — the gate's first run over the merged tree found defects beyond the eleven planned tasks. Fixed as gate-driven rework under `6ce0f14e` by four workers on disjoint file sets (dw-a/b/c/d), all merged.

**Wave 5:** 14 gate findings across two rounds — round 2 surfaced 5 residuals (control-plane layout, migration-history narration, `in-flight` markers); round 3 surfaced 9 more (a nonexistent label CLI surface, a false storage-migration claim, roadmap promises, "legacy alias" framing). Round 4 passed with zero findings.

## Key Decisions

Fourteen lead decisions are recorded in `dev/active/287c4051-progress.json` (LD-1 … LD-14). The load-bearing ones:

- **LD-1/LD-2/LD-6/LD-9** — Relocation policy: live pointers repointed (jit doc links, issue descriptions, `.jit/config.toml`, `CLAUDE.md`, docs); historical records left verbatim, except two handoffs carrying a forward-looking instruction that had begun to 404.
- **LD-8** — Deleted the dead `storage::claims_log` module (user-approved). It had zero non-test consumers and serialized the claim-log sequence field as `sequence` while the live writer uses `seq`; the duplicate made code-review read a correct storage-format doc as wrong.
- **LD-10** — The gate's 21 findings were fixed under this epic as wave 4 rather than deferred (user-approved).
- **LD-11** — No em-dash sweep: the `doc-review` gate checks Mermaid and LaTeX under content standards, not punctuation. A ~150-line mechanical rewrite for zero gate benefit was declined.
- **LD-12** — The reviewer audits deeper on each pass rather than re-checking closed ground, so each round's fix set must be swept for same-class siblings before re-running. Doing so caught four defects the gate sampled but never enumerated.
- **LD-13** — Attached a `cargo-ci` gate to `6ce0f14e` (user-approved) so `code-review` skips its own `cargo test`, per `scripts/code-review-prompt.md:10`.
- **LD-14** — Dropped `dangerouslyDisableSandbox: true` from worker dispatch prompts; the prior handoff carried it as a standing ground rule, but it is an unauthorized sandbox bypass.

## Escalations

1. **`storage::claims_log` deletion** (wave 1, `d48906de`) — a dead module contradicted a correct doc. User approved deletion; `cargo-ci` green afterwards.
2. **21 gate findings beyond planned scope** (wave 4) — user approved fixing them under this epic rather than deferring, including rewriting `web/README.md` and removing two "not yet implemented" roadmap lists.
3. **`cargo-ci` gate on `6ce0f14e`** (wave 5) — `code-review` failed on 7 test failures that did not reproduce: `cargo test --workspace` run alone is green (1545 passed, 0 failed) against the reviewer's 1538 passed / 7 failed. Cause: bug `894337e2` (`find_available_port` races between probe and bind) plus claim tests contending on `.git/jit` while the reviewer runs its own `jit` commands. User approved attaching `cargo-ci`, which is strictly additive and weakens no gate.

4. **Scale of the epic-scoped `doc-review` audit** (Section 10) — the epic gate audited the whole shipped tree against all five criteria and surfaced 24 further defects across six rounds (10→3→1→2→8→0), well beyond the 21 the issue gate had found. Because these were genuine documentation defects and LD-10 had pre-approved fixing all gate findings under this epic, they were fixed as lead edits on `main` rather than escalated per-round; the trajectory was surfaced to the user at round 5.

## Issues Discovered During Execution

- `8d7fc762` — "Gate command timeout never applies after the gate verb rename" (bug, filed to epic `6eb585bc`). Surfaced by `287f7bc9` while re-verifying `mcp-server/README.md` against the code: `getTimeoutForCommand` tests the retired `check`/`check-all`/`pass` verbs, so gate commands run under the 30s default. Filed rather than fixed — mcp-server timeout policy is outside this epic's docs scope.

## Holistic Quality Notes

- **The gate audits by sampling, and deepens each round.** Findings went 21 → 5 → 9 → 0 across four runs. Each round's report closed the prior round's findings and then reached ground it had not previously examined. Fixing only the cited file:line would have guaranteed another failing round; sweeping each finding's *class* across the whole shipped surface is what converged it. Four defects were closed this way that the gate never enumerated: a sixth removed-keys narration in `configuration.md`, and three further `Legacy alias:` sites in `cli-commands.md`.

- **Documentation defects hid real product facts.** The npm package `@erankavija/jit-mcp-server` that `INSTALL.md` told adopters to install returns 404 from the registry. `storage-format.md` promised automatic schema migration that `ensure_supported_index_version` does not perform. `guarantees.md` advertised undo, replay, and time-travel debugging that no command implements. `labels.md` documented an entire label-management CLI — `jit label suggest|schema|audit|fix`, namespace CRUD, `--replace-label` — none of which exists.

- **Empirical verification beat reading.** The `labels.md` rewrite was checked by exercising its claims against a throwaway repository rather than by reading the diff. That confirmed the doc's two load-bearing claims (`--remove-label` applies after `--label`, so the pair swaps a unique label in either flag order; `--type` rejects an undeclared kind where `--label type:x` only warns) and caught one the diff would not have revealed: the MCP tools table named a `remove_label` parameter, but the generator takes flag names verbatim from `jit --schema`, which emits `remove-label`. Only the command path is underscored into the tool name.

- **Two known infrastructure bugs shaped this epic's mechanics.** `894337e2` (port race) forced the `cargo-ci`-before-`code-review` ordering, and `45e1b7e8` (the gate evidences the working tree, not the issue's worktree) forced every gate run to happen from the main working directory.

- **The docs have a systematic drift signature, not scattered typos.** The same handful of facts were wrong in file after file: `.jit/` described as holding all state (it omits gitignored runtime files and the `.git/jit/` control plane) recurred in `overview.md`, `storage-format.md`, and `scope.md`; atomic `rename()` was credited with mutual exclusion that advisory locks actually provide, in `guarantees.md` and `core-model.md`; the storage-format reference carried a wrong ID scheme (ULID vs UUID v4), an incomplete state enum, and an obsolete event shape. This is worth a standing note for future doc work: when one of these facts is found wrong, sweep the whole tree for the class rather than patching the cited line. The epic-scoped gate is the mechanism that enforces this — it audits against the criteria over the entire shipped surface, which is why REQ-04 requires it on the epic and not only on the gate issue.

- **A verification miss, caught by the gate.** The lead declared REQ-05 satisfied after a sweep whose regex looked for `-->`/`==>`; the actual arrow art in four example TOMLs used `──dep→` (box-drawing dash plus arrow glyph) and slipped through. The epic gate caught it. The sweep was widened to unicode arrows. Lesson: a verification regex must be built from the failure's actual character set, not the expected one.
