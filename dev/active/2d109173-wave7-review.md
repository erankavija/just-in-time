# Wave 7 lead review — residual doc-review findings (2d109173)

**Date:** 2026-07-11
**Reviewer:** agent:jit-execution-lead
**Source:** epic review `4fca2c67` (13 blocking source-contradicting defects in previously unreviewed surface)
**Verdict:** PASS — all three children accepted.

Wave 7 partitioned the 13 findings across three disjoint-footprint tasks. Each
fix was verified against the current source tree; both child gates
(`repo-validate`, `docs-mechanical` whole-surface) pass.

## 33b2c714 — lifecycle, lease, durability

- **F1** README.md — Mermaid lifecycle now shows `[*] → ready` for dependency-free
  creation and `[*] → backlog` for unmet-dependency creation.
- **F2** core-model.md:1076 — `jit claim acquire` records the assignee; the
  same-assignee `jit issue claim` afterward only promotes ready work to in_progress.
- **F4** claim.md:10 — advisory lease reframed: serialized acquisition, writes not
  blocked unless `[worktree].enforce_leases` requires a lease (`off` does not block).
- **F5** guarantees.md:62 — atomic-write guarantee scoped to replacement writes
  (temp-file+rename); `events.jsonl` documented as a locked append, not a replacement.
- **F6** guarantees.md:460 — expired lease evicted during a later acquisition; a
  Ready issue with any assignee stays excluded from `jit query available`.
- **F7** guarantees.md:536 — Gitless surface corrected: `jit doc add/list/archive`
  work without git; only `jit doc show --at <commit>` needs a revision.

## 85aae6d2 — coordination, deployment

- **F3** guarantees.md:308 — ordinary `jit issue claim` described as lifecycle
  assignment; exclusive coordination directed to `jit claim acquire`.
- **F8** multi-agent-coordination.md:204 — inert compatibility fields no longer
  presented as operational controls.
- **F9** deployment.md:147 — `JIT_LOCK_TIMEOUT` default corrected to `5` seconds
  (`crates/jit/src/storage/json.rs:115`).
- **F13** multi-agent-coordination.md:175 — source-precedence arrow art replaced
  with a Mermaid `flowchart`.

## 58bc3826 — storage, dependency

- **F10** glossary.md:54 — dependency direction corrected: "A depends on B" means
  B blocks A; B must reach a terminal state before A is ready.
- **F11** storage-format.md:284 — invented per-agent heartbeat files removed;
  `jit claim heartbeat` documented as not writing a per-agent file.
- **F12** storage-format.md:187 — hand-copied event-tag enumeration replaced with a
  citation to the `Event` enum (`crates/jit/src/domain/types.rs`).

## Gate evidence

All three children: `repo-validate` pass, `docs-mechanical` (whole `docs/` surface)
pass. The final epic `doc-review` re-run is the authoritative whole-surface check.
