# Handoff: df129d9e planning complete

Planning and breakdown for df129d9e are done. The container is ready for `jit-execution-lead`.

## State

- Planning node 0ba00a6f is done. plan-review passed on run 2; run 1 failed on unresolved audit sources, unwired external sequencing and oversized facade and item handler leaves.
- Breakdown node 27387b92 is done. coverage-preview and breakdown-review both passed on run 1. Breakdown-review raised two advisories: `executor-facade-issues` is borderline in size (about 390 mechanical call sites), and the pre-handler footprints name the not-yet-existing `crates/jit/src/dispatch/`.
- [breakdown.json](breakdown.json) produced 78 issues (73 tasks, 5 stories) with 227 internal edges. Seven sources depend on 27387b92, and the epic depends on the five stories.
- External re-homes are wired as listed in [plan.md](plan.md#external-dependency-re-homes). `global-json-flag` waits on 2d10f1dd, 3754095f, 16d63038 and 8673dc5e. The other listed edges are implied through it, f37d228e included.

## Escalations for the steward

- **2195c98f ordering (D-21).** Recommended order: the CLI-dispatch and command-id stages of 2195c98f land after this epic's handler migrations. Its error-code vocabulary stage lands before `retire-unproduced-error-codes`. Wire those edges once 2195c98f's breakdown exists.
- **842b0ac6 claims (D-22).** Its unused claim storage type-parameter task should land before `claims-executor-family`. Add that edge once 842b0ac6's breakdown creates the task.
- **0f8efc37 ownership (D-11).** This epic produces the CLI repository bootstrap and the executor search entry point. 0f8efc37 REQ-01 and REQ-08 should consume them rather than build parallel ones.
- **`hidden_commands()` (D-12).** It duplicates `mcp-server/curated-tools.json`'s exclusion policy. REQ-05 pins it here, so removing it is a follow-up candidate under D-18.
- **CI clippy bar (D-14).** GitHub CI runs clippy with `--all-features`, and the cargo-ci gate does not. Aligning the two is outside REQ-06.

No issue is blocked on a user decision.
