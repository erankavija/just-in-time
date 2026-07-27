# Archive run evidence — issue-based development artifact layout (8e071e18)

One section per archival execution, appended by the run that produced it. Each section records the plan the run executed and the in-content citation warnings it reported, which are taken from the plan preview: execution builds its plan without citation evidence, so `--execute --json` reports no citation warnings at all.

Verified per run: every relocated artifact present at its destination and absent from its source, every mirrored artifact present at both, every retained artifact still at its source and carrying no reference change, no artifact with a live owner relocated, the container marker written, and an execution event recorded naming the destination root.

## `14303b30` — dev/archive/14303b30-phase5-2
Issue `e536ea5a`. Plan eligible with no blockers. 8 relocated, 1 mirrored, 4 retained, 3 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| copy | `dev/active/json-output-standardization-plan.md` | `dev/archive/14303b30-phase5-2/dev/active/json-output-standardization-plan.md` | 0db719b1 32f804f1(outside) |
| move | `dev/active/bulk-operations-plan.md` | `dev/archive/14303b30-phase5-2/dev/active/bulk-operations-plan.md` | 44104a30 |
| move | `dev/active/ci-gate-integration-design.md` | `dev/archive/14303b30-phase5-2/dev/active/ci-gate-integration-design.md` | 14303b30 |
| move | `dev/active/gate-examples.md` | `dev/archive/14303b30-phase5-2/dev/active/gate-examples.md` | 14303b30 |
| move | `dev/active/gate-modification-flags-plan.md` | `dev/archive/14303b30-phase5-2/dev/active/gate-modification-flags-plan.md` | 93d1caf7 |
| move | `dev/active/quiet-mode-plan.md` | `dev/archive/14303b30-phase5-2/dev/active/quiet-mode-plan.md` | ecc49a02 |
| move | `dev/sessions/session-2025-12-21-short-hash-progress.md` | `dev/archive/14303b30-phase5-2/dev/sessions/session-2025-12-21-short-hash-progress.md` | 003f9f83 7d6218cf 9072e447 |
| move | `dev/sessions/session-2025-12-29-quiet-flag-implementation.md` | `dev/archive/14303b30-phase5-2/dev/sessions/session-2025-12-29-quiet-flag-implementation.md` | ecc49a02 |
| move | `dev/studies/short-hash-implementation-plan.md` | `dev/archive/14303b30-phase5-2/dev/studies/short-hash-implementation-plan.md` | 003f9f83 |
| retain | `dev/archive/bug-fixes/gate-enforcement-bug-analysis.md` | already under the archive root | 544fd21b |
| retain | `dev/archive/bug-fixes/gate-preview-analysis.md` | already under the archive root | 3342dc7c |
| retain | `dev/archive/bug-fixes/state-transition-feedback-design.md` | already under the archive root | 4ae60108 |
| retain | `docs/tutorials/first-workflow.md` | retained in place | — |

### In-content citation warnings (26)

| citing site | moving artifact |
|---|---|
| `dev/active/8b05a612-investigation.md:43:463` | `dev/active/gate-examples.md` |
| `dev/active/8e071e18-breakdown.json:2187:14` | `dev/active/bulk-operations-plan.md` |
| `dev/active/8e071e18-breakdown.json:2188:14` | `dev/active/ci-gate-integration-design.md` |
| `dev/active/8e071e18-breakdown.json:2189:14` | `dev/active/gate-examples.md` |
| `dev/active/8e071e18-breakdown.json:2190:14` | `dev/active/gate-modification-flags-plan.md` |
| `dev/active/8e071e18-breakdown.json:2191:14` | `dev/active/quiet-mode-plan.md` |
| `dev/active/8e071e18-breakdown.json:2192:14` | `dev/sessions/session-2025-12-21-short-hash-progress.md` |
| `dev/active/8e071e18-breakdown.json:2193:14` | `dev/sessions/session-2025-12-29-quiet-flag-implementation.md` |
| `dev/active/8e071e18-breakdown.json:2194:14` | `dev/studies/short-hash-implementation-plan.md` |
| `dev/active/8e071e18-breakdown.json:3605:1315` | `dev/active/ci-gate-integration-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:1408` | `dev/active/ci-gate-integration-design.md` |
| `dev/active/8e071e18-breakdown.json:3648:1182` | `dev/active/bulk-operations-plan.md` |
| `dev/active/8e071e18-breakdown.json:3648:1269` | `dev/active/bulk-operations-plan.md` |
| `dev/active/8e071e18-breakdown.json:3648:1626` | `dev/active/gate-modification-flags-plan.md` |
| `dev/active/8e071e18-breakdown.json:3648:1721` | `dev/active/gate-modification-flags-plan.md` |
| `dev/active/8e071e18-breakdown.json:3648:2465` | `dev/active/quiet-mode-plan.md` |
| `dev/active/8e071e18-breakdown.json:3648:2547` | `dev/active/quiet-mode-plan.md` |
| `dev/active/8e071e18-breakdown.json:3691:1461` | `dev/active/gate-examples.md` |
| `dev/active/8e071e18-breakdown.json:3691:1541` | `dev/active/gate-examples.md` |
| `dev/active/8e071e18-breakdown.json:3734:1427` | `dev/sessions/session-2025-12-21-short-hash-progress.md` |
| `dev/active/8e071e18-breakdown.json:3734:1534` | `dev/sessions/session-2025-12-21-short-hash-progress.md` |
| `dev/active/8e071e18-breakdown.json:3734:2101` | `dev/sessions/session-2025-12-29-quiet-flag-implementation.md` |
| `dev/active/8e071e18-breakdown.json:3734:2214` | `dev/sessions/session-2025-12-29-quiet-flag-implementation.md` |
| `dev/active/8e071e18-breakdown.json:3777:6446` | `dev/studies/short-hash-implementation-plan.md` |
| `dev/active/8e071e18-breakdown.json:3777:6544` | `dev/studies/short-hash-implementation-plan.md` |
| `dev/sessions/session-2026-01-01-example-md-migration.md:190:3` | `dev/active/gate-examples.md` |
