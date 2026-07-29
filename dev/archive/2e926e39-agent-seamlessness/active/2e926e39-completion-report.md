# Epic 2e926e39 — Completion Report

**Epic:** Agent-seamlessness residuals: first-guess CLI, gate-output readability, and validate scoping
**Driven by:** project-lead (autonomous orchestration)
**Closed:** 2026-06-28
**Result:** All 19 `[hard]` success criteria (REQ-01..REQ-19) met. Repository validation passes.

## Outcome

The epic was decomposed behind an approved plan bracket (planning node `8efc0d75`,
breakdown node `7f709152`), then driven to completion in 7 topological waves over the
impl interior. Every child was authored by a dispatched sub-agent in an isolated
worktree (or directly on `main` for the late single-item waves), self-verified against
the `cargo-ci` gate, integrated to `main` by the lead, then gated with both whole-tree
gates (`cargo-ci` and `code-review`) before transition to `done`. No gate was bypassed,
weakened, or removed; every failing review was reworked until it passed on its own merits.

## Success-criteria coverage (19/19)

| REQ | Summary | Landing child(ren) |
|-----|---------|--------------------|
| REQ-01 | Positional issue title + `--type` flag | 4b7407d3 (+ siblings) |
| REQ-02 | First-guess noun/flag aliases | 1e1ea81d, 4899e4b6, 5ff3a6ae |
| REQ-03 | `--gate` flag on `gate pass`/`check` | 1c94f4f1 |
| REQ-04 | Query default routing + `ready` alias | 3758ee60 |
| REQ-05 | First-guess residuals (`--auto`, `doc add --title`, transposed-arg did-you-mean, top-level `rdeps`/`list`) | a1623187 |
| REQ-06 | Command-grammar standard + canonical rename sweep (retire `jit registry`; retire `issue breakdown`) | 0d593d90, e3259063, 07657508 |
| REQ-07 | Full gate-run report (no truncation) | eb37a05a |
| REQ-08 | `gate check` history + flat report views | 073f9d13 |
| REQ-09 | Gate-prompt readability | 4597743e, f5054f15 |
| REQ-10 | Gate-output readability | 6c740a19, 6ec1dd6f |
| REQ-11 | (coverage) | 22826a75, ea717ef1 |
| REQ-12 | Per-issue `jit validate <id>` scoping | 9e9a4f81 |
| REQ-13 | Anchor-level template gates (`repo-validate`, config-declared) | 2614ecf2, 552ff75c |
| REQ-14 | `issue create --json` ↔ `show` contract parity | b920ea57 |
| REQ-15 | (coverage) | d287bd4c |
| REQ-16 | Editable gate definition (`gate update`) | 252c947b |
| REQ-17 | `ClaimRequiresGitError` (exit 10) | c4b56370 |
| REQ-18 | (coverage) | 4b063bdf |
| REQ-19 | Bounded reverse-dependency depth | 0cbc93e6 |

Scope validation (`jit validate --scope 2e926e39`) passes: no `[hard]` criterion is left
uncovered. The single non-`done` coverage-labelled sibling (313e01f6, a `WAVE-DEBUG probe`)
is `rejected`; REQ-01 remains covered by six `done` children.

## Rework and escalation summary

Reworks (all resolved to a passing `code-review` on merit):
c4b56370 ×1, 9e9a4f81 ×1, 6c740a19 ×1, 4b7407d3 ×2, 552ff75c ×3, 252c947b ×2,
e3259063 ×1, 07657508 ×2.

Escalations (1): **552ff75c / REQ-13** — whether `repo-validate` should be a Rust
built-in preset or config-declared. Resolved by the owner: config-declared in
`.jit/gates.json`, with the anchor-attach path extended to resolve registry gate keys
(not only presets). This became durable guidance for the project.

## Notable structural changes

- **`jit registry` noun retired** — reconciled into `jit gate` (define/remove/list/show);
  `--example` ported to `gate define`; `--stage` made long-only so `-s` = `--state`.
- **`jit issue breakdown` retired** — flat parent-centric breakdown removed; the bracket
  engine (`bracket_breakdown`) the breakdown workflow relies on is untouched. Tree-wide
  search over crates/docs/dev/scripts/contrib/mcp-server finds no issue-breakdown command
  reference; the MCP tool surface no longer exposes it.
- **Config-declared gates** established as the preferred pattern over Rust presets.

## Verification

- Final `cargo-ci`: fmt ok, clippy zero-warnings, **2834 tests passed, 0 failed**.
- `jit validate` (whole repo): passes.
- Both `code-review` and `cargo-ci` gates `passed` on every child before its transition.
