# Planning brief — Exhaustive documentation audit and drift removal (2d109173)

**Status:** input to the planning node (P) of this epic's `plan` bracket. The plan authored at `dev/active/2d109173-plan.md` builds on this brief; the brief is not the plan.

**Provenance:** distilled from the execution of epic 287c4051 (documentation accuracy) and a requirements interview with the invoker on 2026-07-10. Full drift evidence: `dev/archive/287c4051-completion-report.md` (Holistic Quality Notes) and its round-by-round gate records.

## Why this epic exists

287c4051 installed the `doc-review` gate and fixed 59 findings across ten gate rounds. That gate audits by **sampling** and descends into fresh territory each pass — the finding counts (issue-scoped 21→5→9→0, epic-scoped 10→3→1→2→8→0) never re-checked closed ground. Whole areas (`how-to/`, most of `examples/`, `tutorials/`) were never sampled. A clean `doc-review` run therefore does **not** prove the tree is free of drift; it proves the sampled subset is clean. This epic replaces sampling with an exhaustive, mechanically-checkable pass so no stale claim survives, and applies structural prevention so the highest-risk facts cannot silently rot again.

## The seven drift classes (what to hunt)

Each class is a single fact that was wrong in multiple files. When one instance is found, sweep the class across the whole in-scope surface — do not patch only the cited line.

1. **`.jit/` described as all state.** Truth: three locations — `.jit/` versioned data (JSON issues, `events.jsonl`, TOML config/registries); `.jit/` gitignored runtime state (`worktree.json`, `server.log`, `server.pid.json`, `*.lock`, `tmp/`); `.git/jit/` shared control plane (`claims.jsonl`, `claims.index.json`, `heartbeat/`, `locks/claims.lock`).
2. **Atomic `rename()` over-credited.** Truth: advisory locks (repository → index → issue) serialize writes; `rename()` only makes each write atomic. Competing claims fail because locks serialize read-verify-write, not because rename "fails for the loser."
3. **storage-format specifics.** ID scheme is UUID v4 (not ULID); State enum is 7 values incl. `gated`, `archived`; events are `#[serde(tag="type", rename_all="snake_case")]` flat objects (`issue_created`, `issue_state_changed`, `issue_claimed`, `gate_passed/failed/added/removed`, `issue_completed/deleted/released`), not an `event_type`/`data` envelope; gate runs are `gate-runs/<run-id>/result.json`, not per-issue dirs.
4. **Future/roadmap narration stated as behavior.** e.g. "before 1.0", undo/replay/time-travel, "After Publishing" for an npm package that 404s, implicit schema migration that does not exist.
5. **Legacy narration of the current surface.** "legacy alias", "were removed", "replacement for the old", "in-flight" — verbs and keys were renamed and the docs narrated the change instead of stating today's behavior.
6. **Nonexistent CLI surface.** `labels.md` documented `--replace-label`, `jit label suggest|schema|audit|fix`, namespace CRUD — none exist. Reads like docs written against a planned design the implementation diverged from.
7. **Prerequisite / environment drift.** Node version floors, "Node only for MCP" (web needs it), Git framed as optional doc-versioning (leases/worktrees need a repo with resolvable `HEAD`), Docker quick start missing the data-volume init the server refuses to start without.

Root cause: violations of `@/inv/single-source-prose` — volatile facts hand-copied into prose instead of projected or cited, so they rotted when the code moved.

## Decisions locked in the interview (bind the plan)

- **Decomposition axis:** by doc area, five broad areas with disjoint file footprints — (1) `concepts/`, (2) `reference/`, (3) `how-to/` + `tutorials/`, (4) `examples/` (all seven rulesets), (5) root `README.md`/`INSTALL.md`/`TESTING.md` + `mcp-server/README.md` + `web/README.md`.
- **Per-area acceptance bar (exhaustive, not sampled):** every documented command/flag verified against `jit --schema`; every source-path/code citation resolves to a real file; every internal link and heading anchor resolves; every diagram is Mermaid; then `doc-review` passes on the area.
- **Gap-filling (REQ-05):** cover only top-level `jit` command families absent from the docs entirely; flag-level gaps handled by the corrective pass, not treated as new-doc mandates.
- **Structural prevention (REQ-06):** citations + existing projection surfaces only (`@/inv`, `@/rule`, `@/gate`, `jit --schema`, `jit invariant render`, `jit reference render`). No new `jit`/engine features. Missing projection surfaces are filed as follow-ups, not built here.
- **Deletion / merge:** a task may delete a wholly-obsolete or redundant doc, or merge it into another, repointing inbound links, with the rationale recorded on the task.
- **Examples:** audit all seven rulesets, but triage reference-grade vs illustrative first; depth follows the triage; record it.
- **Style:** factual accuracy only; punctuation/tone rewrites are not an acceptance criterion (287c4051 LD-11).
- **Non-goals:** no changes to `dev/` contributor docs or `CLAUDE.md`.

## Notes for the planning agent

- The mechanical checks in the acceptance bar are scriptable — the plan should specify the exact commands per area (a `jit --schema` command-inventory diff, a link/anchor resolver reproducing GitHub's em-dash→double-hyphen anchoring, a source-citation existence check, an arrow-art grep that includes unicode arrows such as `──` and `→`).
- Coverage of the epic's six `[hard]` REQs by the area children is what the `coverage-preview` gate on B will check at breakdown; ensure each REQ maps to concrete area-task work.
- A known CLI help-string bug surfaced while filing this epic: `jit issue create --description`'s `--help` text describes content-format, not the description. It is a `cli.rs` string, out of this docs epic's scope — candidate follow-up.
