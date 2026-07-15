# Handoff — Core maintenance (6eb585bc) — session 1

**Date:** 2026-07-06
**Session number:** 1 (usability batch; the earlier 8-child batch predates the handoff convention — see dev/active/6eb585bc-batch-report-2026-07-05.md)
**Prior handoffs:** None

## Current state

- Epic: `6eb585bc` — state: backlog (finite v1.0 prerequisite; complete and close
  it before production-readiness source freeze per `@/charter/D-14`; assigned
  agent:jit-execution-lead assign-only)
- Wave in progress: none; waves 1–16 complete, waves 17–19 pending (session boundary was invoker-directed after wave 16)
- Children summary: 16 batch children done this session (74fbdb69, b1586c0d, dc3bef62, a05b87ae, 30a3b5c1, c8518f2a, d0f88ee2, 043ae624, cc42a69b, 7fe5c743, 27338abc, 0ab468ba, 31e12d2b, 62f3bebd, 1a63ef75, b3a54e25); 4 open children remain: c291e95c (MCP tool-set re-curation, wave 17), 1d59070d (gate define checker implies auto, wave 18), 0daba57d (refused delete exits nonzero, wave 19), c505031a (deprecated gate aliases removal — pre-existing, NOT in this batch's waves)
- Active claims: none (all workers relieved; b3a54e25 was lead-direct)
- Open escalations: none
- Progress file: `dev/active/6eb585bc-progress.json` (current; includes invoker directives in `note`)

## What just happened

- Filed 15 issues from a usability audit + 4-day session-transcript mining (report: dev/active/6eb585bc-usability-audit-2026-07-05.md); invoker pinned all open design decisions via interview before execution.
- Executed 16 serial waves; every child passed cargo-ci + code-review gates. Rework rounds: 74fbdb69 ×1, a05b87ae ×1 (+1 lead-initiated pre-gate extension), c8518f2a ×2, 043ae624 ×1, cc42a69b ×1, 7fe5c743 ×2, 0ab468ba ×1 (+ lead-run backfill closing REQ-02), 31e12d2b ×2, 1a63ef75 ×2, b3a54e25 ×5 (lead-direct docs; single-contradiction-per-round chain); first-pass green: b1586c0d (after lead surgical fix), dc3bef62, 30a3b5c1, d0f88ee2, 27338abc, 62f3bebd.
- Landed surfaces now dogfooded lead-side: {count,<collection>} envelope, gate key/status, repeatable AND --label, typed exit codes + startup JSON, issue status/children/progress, query count/divergence, config get, structured gate findings (+ ai-review.sh emits the block), graph export --full + lifecycle timestamps (backfill run: 435/464 issues), hierarchy in core (web UI consumes it), wrong-verb hints, help cross-refs, init/export --json.
- Declared INV-DOMAIN-AGNOSTIC (advisory) in .jit/invariants.toml, rendered into AGENTS.md.
- AGENTS.md refreshed (invoker-approved edits incl. Agent Workflow Quick Reference); README fully rewritten (b3a54e25); core-model.md lifecycle docs made archive/revive-accurate with Mermaid diagram.
- Cleaned all 29 membership-label divergences (containment edges via --reduce, labels stripped from 2 rejected issues, WAVE-DEBUG probe deleted via JIT_ALLOW_DELETION=1). `jit validate` fully green, divergences 0.
- Filed from dogfood friction (invoker-interviewed): c291e95c, 1d59070d, 0daba57d.

## What to do next

- [ ] Wave 17: dispatch c291e95c (MCP curated tool-set re-curation; sonnet). mcp-server test-integration.js currently fails (<50 bound vs 68 tools) — deliberate re-curation per issue criteria.
- [ ] Wave 18: dispatch 1d59070d (gate define --checker-command implies auto; evaluate-on-manual must not silently pass; sonnet).
- [ ] Wave 19: dispatch 0daba57d (refused delete exits nonzero + JSON envelope; sonnet).
- [ ] Decide c505031a's slot (deprecated gate alias removal — pre-existing child, never waved). Ask the invoker whether to append it to this batch or leave for the next.
- [ ] Interview the invoker on the archived-resurrection question (Open questions below) and file if approved.
- [ ] After all children resolve: write the batch completion report (model:
  dev/active/6eb585bc-batch-report-2026-07-05.md), link it via `jit doc add`, run
  the epic's completion gates, and close `6eb585bc` before production-readiness
  source freeze. This supersedes the former standing living-container decision
  per `@/charter/D-14`.

## Traps — do not repeat these

- **Do NOT argue scope with the code-review gate.** It reads criterion scope maximally literally and it is always cheaper to comply: "every command that emits a collection" included `graph deps` and `gate status-all` (74fbdb69 round 1); "the web UI consumes the core resolution" meant the EXERCISED GraphView path, not a parallel resolver (31e12d2b round 2); "existing issues are backfilled" meant THIS repo's .jit state, satisfied by the lead running the migration (0ab468ba round 2).
- **Sweep the whole doc for the same claim before gating lifecycle-doc changes.** b3a54e25 burned 4 review rounds on one contradiction chain (README↔core-model archived state), each round surfacing the next copy: States list → transition rules → Done description. `grep -n "cannot transition\|terminal\|archived"` across the target doc FIRST, fix every instance in one commit.
- **Workers' `# Examples` and layering omissions are the two recurring finding classes.** Every new public API (incl. CommandExecutor methods and public fields) needs `# Examples`; orchestration/persistence must live in commands//storage/, never main.rs (config get, 7fe5c743, setup_gitattributes all failed on this). Pre-bake both into dispatch prompts (session prompts already do — keep it).
- **`gate define --checker-command` without `--mode auto` silently creates a MANUAL gate; `gate evaluate` on it reports Passed with no run.** Filed as 1d59070d. Until fixed, always pass `--mode auto` when defining exec gates (scripts and tests included).
- **`jit issue delete` refusal exits 0.** Without JIT_ALLOW_DELETION=1 it prints a refusal and succeeds by exit code (filed 0daba57d). Never chain `delete && commit`; verify the issue is gone.
- **Mid-wave, the installed jit lags the workspace.** Use `./target/debug/jit` for post-wave checks until the post-acceptance `cargo install --path crates/jit`. Hit live on b1586c0d (`gates` key missing from installed binary's output).
- **rust-analyzer diagnostics arriving after worker commits are stale buffer snapshots.** Every wave produced scary E0063/E0308/E0004 diagnostics that `cargo build --workspace` disproved. Trust the build, not the IDE stream.
- **zsh does not word-split unquoted variables.** `for pair in "a b"; do cmd $pair; done` passes ONE argument. Use explicit arguments or `${=pair}`. Cost one confusing round in the divergence cleanup.
- **`gate check-all` READS; `gate evaluate` RUNS.** Post-949cd9d0 semantics; check-all on never-run gates prints "has not been run yet".
- **Worker idle pings are not completion.** Workers idle mid-CI and after mailbox races; check `git log` + working tree before assuming done or stalled (62f3bebd, 31e12d2b both resumed fine after a nudge).

## Open questions needing invoker input

- Question: file the archived-resurrection loophole?
  - Context: live-verified that done → archived → ready succeeds, resurrecting a completed issue into the active lifecycle; core-model.md now documents current behavior factually.
  - Options: (a) file a bug making terminal states archive-but-not-revive (archived preserves prior terminality), (b) accept as intended escape hatch and add an advisory validate warning, (c) leave as-is.
  - Recommendation: (a) — it silently un-terminals Done, which contradicts INV-GATE-SEMANTICS' spirit.

## Reference artefacts

- Epic: `jit issue show 6eb585bc`; children rollup: `jit issue children 6eb585bc`
- Progress: dev/active/6eb585bc-progress.json (wave plan, rework counts, dogfood-friction log, invoker directives)
- Audit report: dev/active/6eb585bc-usability-audit-2026-07-05.md (linked to epic)
- Prior batch report: dev/active/6eb585bc-batch-report-2026-07-05.md (linked to epic)
- Conventions decided this batch: memory notes jit-output-conventions, no-legacy-narration-in-docs, dogfood-new-jit-features, interview-before-filing-issues (lead memory directory)
- Review protocol: ~/.agents/skills/jit-execution-lead/references/lead-review-protocol.md (six tiers + no-argue)
