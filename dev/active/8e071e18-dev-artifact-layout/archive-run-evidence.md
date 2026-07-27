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

## `2821e177` — dev/archive/2821e177-addressing-v2
Issue `39757bb3`. Plan eligible with no blockers. 11 relocated, 0 mirrored, 5 retained, 2 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/0efbc594-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/dev/active/0efbc594-breakdown-spec.md` | 0efbc594 |
| move | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | 0b611ccf |
| move | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/dev/active/2821e177-investigation.md` | 0b611ccf |
| move | `dev/active/37506c12-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/dev/active/37506c12-breakdown-spec.md` | 37506c12 |
| move | `dev/active/637764ef-acceptance-evidence.md` | `dev/archive/2821e177-addressing-v2/dev/active/637764ef-acceptance-evidence.md` | 637764ef |
| move | `dev/active/71ebd1e8-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/dev/active/71ebd1e8-breakdown-spec.md` | 71ebd1e8 |
| move | `dev/active/7f22d6cf-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/dev/active/7f22d6cf-breakdown-spec.md` | 7f22d6cf |
| move | `dev/active/9a7106ae-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/dev/active/9a7106ae-breakdown-spec.md` | 9a7106ae |
| move | `dev/active/bb7d57a2-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/dev/active/bb7d57a2-breakdown-spec.md` | bb7d57a2 |
| move | `dev/studies/addressing-v2-item-use-cases.md` | `dev/archive/2821e177-addressing-v2/dev/studies/addressing-v2-item-use-cases.md` | 2821e177 |
| move | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/dev/studies/addressing-v2-rule-gate-items.md` | 0b611ccf 2821e177 |
| retain | `dev/archive/features/2821e177/completion-report.md` | already under the archive root | 2821e177 |
| retain | `dev/archive/features/2821e177/showcase/base.css` | retained in place | — |
| retain | `dev/archive/features/2821e177/showcase/talk.html` | already under the archive root | 2821e177 |
| retain | `dev/archive/features/2821e177/showcase/themes/rust.css` | retained in place | — |
| retain | `docs/reference/rules-and-gates.md` | retained in place | ebb6ad45 |

### In-content citation warnings (62)

| citing site | moving artifact |
|---|---|
| `dev/active/0efbc594-breakdown-spec.md:3:162` | `dev/active/2821e177-investigation.md` |
| `dev/active/0efbc594-breakdown-spec.md:3:55` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:490:28` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:492:4` | `dev/active/637764ef-acceptance-evidence.md` |
| `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:5:14` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:6:9` | `dev/active/2821e177-investigation.md` |
| `dev/active/2821e177-investigation.md:3:49` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/37506c12-breakdown-spec.md:3:162` | `dev/active/2821e177-investigation.md` |
| `dev/active/37506c12-breakdown-spec.md:3:55` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/637764ef-acceptance-evidence.md:149:4` | `dev/active/bb7d57a2-breakdown-spec.md` |
| `dev/active/637764ef-acceptance-evidence.md:150:4` | `dev/active/71ebd1e8-breakdown-spec.md` |
| `dev/active/637764ef-acceptance-evidence.md:151:4` | `dev/active/7f22d6cf-breakdown-spec.md` |
| `dev/active/637764ef-acceptance-evidence.md:152:4` | `dev/active/0efbc594-breakdown-spec.md` |
| `dev/active/637764ef-acceptance-evidence.md:153:4` | `dev/active/9a7106ae-breakdown-spec.md` |
| `dev/active/637764ef-acceptance-evidence.md:154:4` | `dev/active/37506c12-breakdown-spec.md` |
| `dev/active/637764ef-acceptance-evidence.md:155:4` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/637764ef-acceptance-evidence.md:156:4` | `dev/active/2821e177-investigation.md` |
| `dev/active/637764ef-acceptance-evidence.md:163:34` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/637764ef-acceptance-evidence.md:169:15` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/71ebd1e8-breakdown-spec.md:3:162` | `dev/active/2821e177-investigation.md` |
| `dev/active/71ebd1e8-breakdown-spec.md:3:55` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/7f22d6cf-breakdown-spec.md:3:162` | `dev/active/2821e177-investigation.md` |
| `dev/active/7f22d6cf-breakdown-spec.md:3:55` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/8e071e18-breakdown.json:2001:14` | `dev/active/0efbc594-breakdown-spec.md` |
| `dev/active/8e071e18-breakdown.json:2002:14` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/8e071e18-breakdown.json:2003:14` | `dev/active/2821e177-investigation.md` |
| `dev/active/8e071e18-breakdown.json:2004:14` | `dev/active/37506c12-breakdown-spec.md` |
| `dev/active/8e071e18-breakdown.json:2005:14` | `dev/active/637764ef-acceptance-evidence.md` |
| `dev/active/8e071e18-breakdown.json:2006:14` | `dev/active/71ebd1e8-breakdown-spec.md` |
| `dev/active/8e071e18-breakdown.json:2007:14` | `dev/active/7f22d6cf-breakdown-spec.md` |
| `dev/active/8e071e18-breakdown.json:2008:14` | `dev/active/9a7106ae-breakdown-spec.md` |
| `dev/active/8e071e18-breakdown.json:2009:14` | `dev/active/bb7d57a2-breakdown-spec.md` |
| `dev/active/8e071e18-breakdown.json:2010:14` | `dev/studies/addressing-v2-item-use-cases.md` |
| `dev/active/8e071e18-breakdown.json:2011:14` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/8e071e18-breakdown.json:3777:1659` | `dev/studies/addressing-v2-item-use-cases.md` |
| `dev/active/8e071e18-breakdown.json:3777:1760` | `dev/studies/addressing-v2-item-use-cases.md` |
| `dev/active/8e071e18-breakdown.json:3777:1809` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/8e071e18-breakdown.json:3777:1911` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/active/9a7106ae-breakdown-spec.md:3:162` | `dev/active/2821e177-investigation.md` |
| `dev/active/9a7106ae-breakdown-spec.md:3:55` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/active/9b7b5f9c-investigation.md:324:4` | `dev/active/2821e177-investigation.md` |
| `dev/active/bb7d57a2-breakdown-spec.md:3:162` | `dev/active/2821e177-investigation.md` |
| `dev/active/bb7d57a2-breakdown-spec.md:3:55` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/archive/features/2821e177/2821e177-handoff-2.md:58:65` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/archive/features/2821e177/2821e177-handoff-2.md:59:18` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/archive/features/2821e177/2821e177-handoff-2.md:59:81` | `dev/active/2821e177-investigation.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:51:40` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:52:18` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:52:81` | `dev/active/2821e177-investigation.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:53:103` | `dev/active/71ebd1e8-breakdown-spec.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:53:144` | `dev/active/bb7d57a2-breakdown-spec.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:53:185` | `dev/active/7f22d6cf-breakdown-spec.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:53:21` | `dev/active/37506c12-breakdown-spec.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:53:226` | `dev/active/9a7106ae-breakdown-spec.md` |
| `dev/archive/features/2821e177/2821e177-handoff.md:53:62` | `dev/active/0efbc594-breakdown-spec.md` |
| `dev/archive/features/2821e177/2821e177-progress.json:192:59` | `dev/active/637764ef-acceptance-evidence.md` |
| `dev/archive/features/2821e177/completion-report.md:31:139` | `dev/active/637764ef-acceptance-evidence.md` |
| `dev/archive/features/2821e177/completion-report.md:81:18` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/archive/features/2821e177/completion-report.md:82:26` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/archive/features/2821e177/completion-report.md:83:25` | `dev/active/637764ef-acceptance-evidence.md` |
| `dev/archive/features/2821e177/showcase/talk.html:226:52` | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/studies/addressing-v2-rule-gate-items.md:196:6` | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |

### Other plan warnings (1)

- `external-edge` at `dev/archive/features/2821e177/showcase/talk.html`

## `53e3fa36` — dev/archive/53e3fa36-agent-ergonomics
Issue `6f6fe80e`. Plan eligible with no blockers. 2 relocated, 0 mirrored, 1 retained, 1 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | `dev/archive/53e3fa36-agent-ergonomics/dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | 2937919f 53e3fa36 |
| move | `dev/studies/session-mining-jit-improvements.md` | `dev/archive/53e3fa36-agent-ergonomics/dev/studies/session-mining-jit-improvements.md` | 53e3fa36 |
| retain | `dev/archive/features/53e3fa36-completion-report.md` | already under the archive root | 53e3fa36 |

### In-content citation warnings (7)

| citing site | moving artifact |
|---|---|
| `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md:38:361` | `dev/studies/session-mining-jit-improvements.md` |
| `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md:4:19` | `dev/studies/session-mining-jit-improvements.md` |
| `dev/active/8e071e18-breakdown.json:2781:14` | `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` |
| `dev/active/8e071e18-breakdown.json:2782:14` | `dev/studies/session-mining-jit-improvements.md` |
| `dev/active/8e071e18-breakdown.json:3777:6287` | `dev/studies/session-mining-jit-improvements.md` |
| `dev/active/8e071e18-breakdown.json:3777:6394` | `dev/studies/session-mining-jit-improvements.md` |
| `dev/archive/features/53e3fa36-progress.json:9:18` | `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` |

## `2e926e39` — dev/archive/2e926e39-agent-seamlessness
Issue `6116d362`. Plan eligible with no blockers. 5 relocated, 0 mirrored, 1 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | `dev/archive/2e926e39-agent-seamlessness/dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | 8efc0d75 |
| move | `dev/active/2e926e39-completion-report.md` | `dev/archive/2e926e39-agent-seamlessness/dev/active/2e926e39-completion-report.md` | 2e926e39 |
| move | `dev/presentations/2e926e39/base.css` | `dev/archive/2e926e39-agent-seamlessness/dev/presentations/2e926e39/base.css` | — |
| move | `dev/presentations/2e926e39/talk.html` | `dev/archive/2e926e39-agent-seamlessness/dev/presentations/2e926e39/talk.html` | 2e926e39 |
| move | `dev/presentations/2e926e39/themes/rust.css` | `dev/archive/2e926e39-agent-seamlessness/dev/presentations/2e926e39/themes/rust.css` | — |
| retain | `docs/reference/cli-command-grammar.md` | retained in place | 527bf0a6 |

### In-content citation warnings (6)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:2552:14` | `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` |
| `dev/active/8e071e18-breakdown.json:2553:14` | `dev/active/2e926e39-completion-report.md` |
| `dev/active/8e071e18-breakdown.json:2554:14` | `dev/presentations/2e926e39/base.css` |
| `dev/active/8e071e18-breakdown.json:2555:14` | `dev/presentations/2e926e39/talk.html` |
| `dev/active/8e071e18-breakdown.json:2556:14` | `dev/presentations/2e926e39/themes/rust.css` |
| `dev/active/8e071e18-dev-artifact-layout/archive-run-evidence.md:169:4` | `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` |

### Other plan warnings (1)

- `external-edge` at `dev/presentations/2e926e39/talk.html`

## `4a00b2b0` — dev/archive/4a00b2b0-agent-validation
Issue `5c09146f`. Plan eligible with no blockers. 3 relocated, 0 mirrored, 0 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/agent-validation-design.md` | `dev/archive/4a00b2b0-agent-validation/dev/active/agent-validation-design.md` | 4a00b2b0 |
| move | `dev/studies/ai-tool-worktree-compatibility.md` | `dev/archive/4a00b2b0-agent-validation/dev/studies/ai-tool-worktree-compatibility.md` | 4a00b2b0 |
| move | `dev/studies/worktree-merge-analysis.md` | `dev/archive/4a00b2b0-agent-validation/dev/studies/worktree-merge-analysis.md` | 4a00b2b0 |

### In-content citation warnings (16)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:1653:1080` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-breakdown.json:1689:14` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-breakdown.json:2725:14` | `dev/active/agent-validation-design.md` |
| `dev/active/8e071e18-breakdown.json:2726:14` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-breakdown.json:2727:14` | `dev/studies/worktree-merge-analysis.md` |
| `dev/active/8e071e18-breakdown.json:3605:1174` | `dev/active/agent-validation-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:1272` | `dev/active/agent-validation-design.md` |
| `dev/active/8e071e18-breakdown.json:3777:2101` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-breakdown.json:3777:2207` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-breakdown.json:3777:6595` | `dev/studies/worktree-merge-analysis.md` |
| `dev/active/8e071e18-breakdown.json:3777:6694` | `dev/studies/worktree-merge-analysis.md` |
| `dev/active/8e071e18-investigation.md:1308:4` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-investigation.md:1309:4` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-investigation.md:1310:4` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-investigation.md:1360:4` | `dev/studies/ai-tool-worktree-compatibility.md` |
| `dev/active/8e071e18-investigation.md:1363:22` | `dev/studies/ai-tool-worktree-compatibility.md` |

### Other plan warnings (1)

- `external-edge` at `dev/studies/ai-tool-worktree-compatibility.md`

## `7095769d` — dev/archive/7095769d-code-smell-cleanup
Issue `a45bc29c`. Plan eligible with no blockers. 1 relocated, 0 mirrored, 0 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/7095769d-smell-hunt-report.md` | `dev/archive/7095769d-code-smell-cleanup/dev/active/7095769d-smell-hunt-report.md` | 7095769d |

### In-content citation warnings (1)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:2836:14` | `dev/active/7095769d-smell-hunt-report.md` |

## `6eb585bc` — dev/archive/6eb585bc-core-maintenance
Issue `fccedb73`. Plan eligible with no blockers. 17 relocated, 7 mirrored, 10 retained, 6 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| copy | `dev/active/73482aa1-rust-build-efficiency.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/73482aa1-rust-build-efficiency.md` | 73482aa1 |
| copy | `dev/benchmarks/rust-build-efficiency/baseline.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/baseline.json` | — |
| copy | `dev/benchmarks/rust-build-efficiency/optimized.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/optimized.json` | 26f97dc2 |
| copy | `dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json` | — |
| copy | `dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json` | — |
| copy | `dev/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json` | — |
| copy | `dev/benchmarks/rust-build-efficiency/report.md` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/report.md` | 26f97dc2 4e22a20d |
| move | `dev/active/3e12ffbd-batch-export-design.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/3e12ffbd-batch-export-design.md` | 3e12ffbd |
| move | `dev/active/3e12ffbd-batch-export-use-cases.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/3e12ffbd-batch-export-use-cases.md` | 3e12ffbd |
| move | `dev/active/450db193-generic-projection-design.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/450db193-generic-projection-design.md` | 450db193 |
| move | `dev/active/45a140ae-archived-semantics.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/45a140ae-archived-semantics.md` | 45a140ae |
| move | `dev/active/71be6ae9-code-review-reliability.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/71be6ae9-code-review-reliability.md` | 71be6ae9 |
| move | `dev/active/71be6ae9-live-review-report.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/71be6ae9-live-review-report.md` | 71be6ae9 |
| move | `dev/active/73482aa1-completion-report.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/73482aa1-completion-report.md` | 73482aa1 |
| move | `dev/active/76cb968b-citation-check.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/76cb968b-citation-check.md` | 76cb968b |
| move | `dev/active/76cb968b-ssot-adoption-sweep.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/76cb968b-ssot-adoption-sweep.md` | 76cb968b |
| move | `dev/active/76cb968b-sweep-table.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/76cb968b-sweep-table.md` | 76cb968b |
| move | `dev/active/949cd9d0-gate-verb-semantics.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/949cd9d0-gate-verb-semantics.md` | 949cd9d0 |
| move | `dev/active/af4c901a-derive-default-rules-at-load.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/af4c901a-derive-default-rules-at-load.md` | af4c901a |
| move | `dev/active/b4e55aa2-code-review-live-verification.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/b4e55aa2-code-review-live-verification.md` | 155a2d43 |
| move | `dev/active/b4e55aa2-ground-code-review-policy.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/b4e55aa2-ground-code-review-policy.md` | b4e55aa2 |
| move | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | d74a9ed1 |
| move | `dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` | 8d4f7084 |
| move | `dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` | 8d4f7084 |
| retain | `CHANGELOG.md` | retained in place | b1586c0d d0f88ee2 |
| retain | `crates/jit/tests/cli_issue/verb_hint_tests.rs` | retained in place | d0f88ee2 |
| retain | `dev/archive/6eb585bc-batch-report-2026-07-05.md` | already under the archive root | 6eb585bc |
| retain | `dev/archive/6eb585bc-completion-report.md` | already under the archive root | 6eb585bc |
| retain | `dev/archive/6eb585bc-usability-audit-2026-07-05.md` | already under the archive root | 6eb585bc |
| retain | `dev/archive/76cb968b-completion-report.md` | already under the archive root | 76cb968b |
| retain | `dev/archive/a9b5dd08/dev/active/a9b5dd08-archive-directory-slugs.md` | already under the archive root | a9b5dd08 |
| retain | `dev/archive/b4e55aa2-completion-report.md` | already under the archive root | b4e55aa2 |
| retain | `docs/reference/cli-commands.md` | retained in place | b1586c0d d0f88ee2 |
| retain | `scripts/benchmark-rust-build.sh` | retained in place | — |

### In-content citation warnings (48)

| citing site | moving artifact |
|---|---|
| `dev/active/71be6ae9-code-review-reliability.md:203:11` | `dev/active/71be6ae9-live-review-report.md` |
| `dev/active/73482aa1-progress.json:7:90` | `dev/active/73482aa1-completion-report.md` |
| `dev/active/76cb968b-ssot-adoption-sweep.md:77:970` | `dev/active/76cb968b-sweep-table.md` |
| `dev/active/76cb968b-ssot-adoption-sweep.md:87:233` | `dev/active/76cb968b-citation-check.md` |
| `dev/active/8e071e18-breakdown.json:1813:14` | `dev/active/3e12ffbd-batch-export-design.md` |
| `dev/active/8e071e18-breakdown.json:1814:14` | `dev/active/3e12ffbd-batch-export-use-cases.md` |
| `dev/active/8e071e18-breakdown.json:1815:14` | `dev/active/450db193-generic-projection-design.md` |
| `dev/active/8e071e18-breakdown.json:1816:14` | `dev/active/45a140ae-archived-semantics.md` |
| `dev/active/8e071e18-breakdown.json:1817:14` | `dev/active/71be6ae9-code-review-reliability.md` |
| `dev/active/8e071e18-breakdown.json:1818:14` | `dev/active/71be6ae9-live-review-report.md` |
| `dev/active/8e071e18-breakdown.json:1819:14` | `dev/active/73482aa1-completion-report.md` |
| `dev/active/8e071e18-breakdown.json:1820:14` | `dev/active/76cb968b-citation-check.md` |
| `dev/active/8e071e18-breakdown.json:1821:14` | `dev/active/76cb968b-ssot-adoption-sweep.md` |
| `dev/active/8e071e18-breakdown.json:1822:14` | `dev/active/76cb968b-sweep-table.md` |
| `dev/active/8e071e18-breakdown.json:1823:14` | `dev/active/949cd9d0-gate-verb-semantics.md` |
| `dev/active/8e071e18-breakdown.json:1824:14` | `dev/active/af4c901a-derive-default-rules-at-load.md` |
| `dev/active/8e071e18-breakdown.json:1825:14` | `dev/active/b4e55aa2-code-review-live-verification.md` |
| `dev/active/8e071e18-breakdown.json:1826:14` | `dev/active/b4e55aa2-ground-code-review-policy.md` |
| `dev/active/8e071e18-breakdown.json:1827:14` | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` |
| `dev/active/8e071e18-breakdown.json:1828:14` | `dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` |
| `dev/active/8e071e18-breakdown.json:1829:14` | `dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` |
| `dev/active/8e071e18-breakdown.json:3459:1338` | `dev/active/73482aa1-completion-report.md` |
| `dev/active/8e071e18-investigation.md:1137:5` | `dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` |
| `dev/active/8e071e18-investigation.md:1140:5` | `dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` |
| `dev/active/9b7b5f9c-investigation.md:325:4` | `dev/active/76cb968b-ssot-adoption-sweep.md` |
| `dev/archive/2d109173-investigation.md:192:106` | `dev/active/76cb968b-citation-check.md` |
| `dev/archive/2d109173-investigation.md:215:6` | `dev/active/76cb968b-citation-check.md` |
| `dev/archive/6eb585bc-handoff-2.md:10:108` | `dev/active/73482aa1-completion-report.md` |
| `dev/archive/6eb585bc-handoff-2.md:46:10` | `dev/active/73482aa1-completion-report.md` |
| `dev/archive/6eb585bc-handoff-3.md:72:47` | `dev/active/45a140ae-archived-semantics.md` |
| `dev/archive/6eb585bc-handoff-4.md:94:119` | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` |
| `dev/archive/6eb585bc-handoff-4.md:94:32` | `dev/active/45a140ae-archived-semantics.md` |
| `dev/archive/6eb585bc-handoff-4.md:94:75` | `dev/active/3e12ffbd-batch-export-design.md` |
| `dev/archive/6eb585bc-progress.json:254:210` | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` |
| `dev/archive/6eb585bc-progress.json:55:274` | `dev/active/73482aa1-completion-report.md` |
| `dev/archive/76cb968b-completion-report.md:25:160` | `dev/active/76cb968b-sweep-table.md` |
| `dev/archive/76cb968b-completion-report.md:28:78` | `dev/active/76cb968b-citation-check.md` |
| `dev/archive/76cb968b-progress.json:4:139` | `dev/active/76cb968b-ssot-adoption-sweep.md` |
| `dev/archive/b4e55aa2-completion-report.md:36:214` | `dev/active/b4e55aa2-code-review-live-verification.md` |
| `dev/archive/b4e55aa2-handoff.md:53:17` | `dev/active/b4e55aa2-ground-code-review-policy.md` |
| `dev/archive/cdc840ad-handoff-12.md:62:111` | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` |
| `dev/archive/cdc840ad-handoff-12.md:62:56` | `dev/active/af4c901a-derive-default-rules-at-load.md` |
| `dev/archive/cdc840ad-investigation.md:1095:4` | `dev/active/af4c901a-derive-default-rules-at-load.md` |
| `dev/archive/cdc840ad-investigation.md:1098:4` | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` |
| `dev/archive/cdc840ad-investigation.md:1100:4` | `dev/active/450db193-generic-projection-design.md` |
| `dev/archive/cdc840ad-research.md:1265:244` | `dev/active/af4c901a-derive-default-rules-at-load.md` |
| `dev/archive/cdc840ad-research.md:931:4` | `dev/active/af4c901a-derive-default-rules-at-load.md` |
| `dev/archive/cdc840ad-research.md:936:4` | `dev/active/af4c901a-derive-default-rules-at-load.md` |

## `71373e37` — dev/archive/71373e37-docs-lifecycle
Issue `95b01080`. Plan eligible with no blockers. 6 relocated, 2 mirrored, 1 retained, 1 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| copy | `dev/active/documentation-lifecycle-design.md` | `dev/archive/71373e37-docs-lifecycle/dev/active/documentation-lifecycle-design.md` | 71373e37 |
| copy | `dev/studies/documentation-organization-strategy.md` | `dev/archive/71373e37-docs-lifecycle/dev/studies/documentation-organization-strategy.md` | 165cf162 cfb3ba94(outside) |
| move | `dev/active/doc-archive-implementation-guide.md` | `dev/archive/71373e37-docs-lifecycle/dev/active/doc-archive-implementation-guide.md` | 896ff7df |
| move | `dev/active/snapshot-export-implementation-plan.md` | `dev/archive/71373e37-docs-lifecycle/dev/active/snapshot-export-implementation-plan.md` | a8f2f04b |
| move | `dev/sessions/session-2024-12-24-check-links-incomplete.md` | `dev/archive/71373e37-docs-lifecycle/dev/sessions/session-2024-12-24-check-links-incomplete.md` | fb6e2e31 |
| move | `dev/sessions/session-2025-12-22-doc-consolidation.md` | `dev/archive/71373e37-docs-lifecycle/dev/sessions/session-2025-12-22-doc-consolidation.md` | 6f6b842a |
| move | `dev/sessions/session-2025-12-27-doc-archive.md` | `dev/archive/71373e37-docs-lifecycle/dev/sessions/session-2025-12-27-doc-archive.md` | 896ff7df |
| move | `dev/studies/documentation-lifecycle-strategy.md` | `dev/archive/71373e37-docs-lifecycle/dev/studies/documentation-lifecycle-strategy.md` | 71373e37 |
| retain | `dev/archive/studies/authoring-conventions-draft.md` | already under the archive root | 71373e37 fb6e2e31 |

### In-content citation warnings (22)

| citing site | moving artifact |
|---|---|
| `dev/active/8b05a612-investigation.md:60:4` | `dev/sessions/session-2025-12-22-doc-consolidation.md` |
| `dev/active/8e071e18-breakdown.json:2610:14` | `dev/active/doc-archive-implementation-guide.md` |
| `dev/active/8e071e18-breakdown.json:2611:14` | `dev/active/snapshot-export-implementation-plan.md` |
| `dev/active/8e071e18-breakdown.json:2612:14` | `dev/sessions/session-2024-12-24-check-links-incomplete.md` |
| `dev/active/8e071e18-breakdown.json:2613:14` | `dev/sessions/session-2025-12-22-doc-consolidation.md` |
| `dev/active/8e071e18-breakdown.json:2614:14` | `dev/sessions/session-2025-12-27-doc-archive.md` |
| `dev/active/8e071e18-breakdown.json:2615:14` | `dev/studies/documentation-lifecycle-strategy.md` |
| `dev/active/8e071e18-breakdown.json:3648:2582` | `dev/active/snapshot-export-implementation-plan.md` |
| `dev/active/8e071e18-breakdown.json:3648:2690` | `dev/active/snapshot-export-implementation-plan.md` |
| `dev/active/8e071e18-breakdown.json:3691:1304` | `dev/active/doc-archive-implementation-guide.md` |
| `dev/active/8e071e18-breakdown.json:3691:1409` | `dev/active/doc-archive-implementation-guide.md` |
| `dev/active/8e071e18-breakdown.json:3734:1248` | `dev/sessions/session-2024-12-24-check-links-incomplete.md` |
| `dev/active/8e071e18-breakdown.json:3734:1364` | `dev/sessions/session-2024-12-24-check-links-incomplete.md` |
| `dev/active/8e071e18-breakdown.json:3734:1594` | `dev/sessions/session-2025-12-22-doc-consolidation.md` |
| `dev/active/8e071e18-breakdown.json:3734:1705` | `dev/sessions/session-2025-12-22-doc-consolidation.md` |
| `dev/active/8e071e18-breakdown.json:3734:1944` | `dev/sessions/session-2025-12-27-doc-archive.md` |
| `dev/active/8e071e18-breakdown.json:3734:2049` | `dev/sessions/session-2025-12-27-doc-archive.md` |
| `dev/active/8e071e18-breakdown.json:3777:3970` | `dev/studies/documentation-lifecycle-strategy.md` |
| `dev/active/8e071e18-breakdown.json:3777:4076` | `dev/studies/documentation-lifecycle-strategy.md` |
| `dev/active/8e071e18-investigation.md:647:52` | `dev/active/doc-archive-implementation-guide.md` |
| `dev/active/8e071e18-investigation.md:650:28` | `dev/studies/documentation-lifecycle-strategy.md` |
| `dev/archive/2d109173-investigation.md:216:6` | `dev/sessions/session-2024-12-24-check-links-incomplete.md` |

### Other plan warnings (1)

- `external-edge` at `dev/studies/documentation-organization-strategy.md`

## `94f873c8` — dev/archive/94f873c8-docs-lifecycle-p2
Issue `a21185df`. Plan eligible with no blockers. 3 relocated, 0 mirrored, 0 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/5c060496-raw-assets-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/dev/active/5c060496-raw-assets-design.md` | 5c060496 |
| move | `dev/active/abfd6016-multi-format-doc-rendering-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/dev/active/abfd6016-multi-format-doc-rendering-design.md` | abfd6016 |
| move | `dev/active/documentation-lifecycle-phase2-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/dev/active/documentation-lifecycle-phase2-design.md` | 94f873c8 |

### In-content citation warnings (13)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:2430:14` | `dev/active/5c060496-raw-assets-design.md` |
| `dev/active/8e071e18-breakdown.json:2431:14` | `dev/active/abfd6016-multi-format-doc-rendering-design.md` |
| `dev/active/8e071e18-breakdown.json:2432:14` | `dev/active/documentation-lifecycle-phase2-design.md` |
| `dev/active/8e071e18-breakdown.json:3459:1730` | `dev/active/abfd6016-multi-format-doc-rendering-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:2334` | `dev/active/documentation-lifecycle-phase2-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:2447` | `dev/active/documentation-lifecycle-phase2-design.md` |
| `dev/active/8e071e18-investigation.md:648:4` | `dev/active/documentation-lifecycle-phase2-design.md` |
| `dev/active/abfd6016-progress.json:64:18` | `dev/active/abfd6016-multi-format-doc-rendering-design.md` |
| `dev/archive/7d3a3a47/dev/active/7d3a3a47-investigation.md:138:173` | `dev/active/5c060496-raw-assets-design.md` |
| `dev/archive/7d3a3a47/dev/active/7d3a3a47-investigation.md:43:247` | `dev/active/5c060496-raw-assets-design.md` |
| `dev/archive/7d3a3a47/dev/active/7d3a3a47-investigation.md:43:376` | `dev/active/5c060496-raw-assets-design.md` |
| `dev/archive/7d3a3a47/dev/active/7d3a3a47-investigation.md:46:119` | `dev/active/documentation-lifecycle-phase2-design.md` |
| `dev/archive/7d3a3a47/dev/active/7d3a3a47-investigation.md:49:158` | `dev/active/documentation-lifecycle-phase2-design.md` |

## `cfb3ba94` — dev/archive/cfb3ba94-docs
Issue `d1ae9782`. Plan eligible with no blockers. 1 relocated, 1 mirrored, 2 retained, 1 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| copy | `dev/studies/documentation-organization-strategy.md` | `dev/archive/cfb3ba94-docs/dev/studies/documentation-organization-strategy.md` | cfb3ba94 |
| move | `dev/sessions/session-2026-01-01-example-md-migration.md` | `dev/archive/cfb3ba94-docs/dev/sessions/session-2026-01-01-example-md-migration.md` | d6dc4dfa |
| retain | `dev/archive/studies/docs-audit-plan.md` | already under the archive root | cfb3ba94 |
| retain | `docs/concepts/design-philosophy.md` | retained in place | c8355d70 |

### In-content citation warnings (4)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:1883:14` | `dev/sessions/session-2026-01-01-example-md-migration.md` |
| `dev/active/8e071e18-breakdown.json:3734:3173` | `dev/sessions/session-2026-01-01-example-md-migration.md` |
| `dev/active/8e071e18-breakdown.json:3734:3277` | `dev/sessions/session-2026-01-01-example-md-migration.md` |
| `dev/active/8e071e18-dev-artifact-layout/archive-run-evidence.md:57:4` | `dev/sessions/session-2026-01-01-example-md-migration.md` |

### Other plan warnings (1)

- `external-edge` at `dev/studies/documentation-organization-strategy.md`

## `a4e3cfb0` — dev/archive/a4e3cfb0
Issue `1c4b7a3c`. Plan eligible with no blockers. 1 relocated, 0 mirrored, 0 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/studies/documentation-tooling-evaluation.md` | `dev/archive/a4e3cfb0/dev/studies/documentation-tooling-evaluation.md` | a4e3cfb0 |

### In-content citation warnings (3)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:3142:14` | `dev/studies/documentation-tooling-evaluation.md` |
| `dev/active/8e071e18-breakdown.json:3777:5069` | `dev/studies/documentation-tooling-evaluation.md` |
| `dev/active/8e071e18-breakdown.json:3777:5160` | `dev/studies/documentation-tooling-evaluation.md` |

## `2d109173` — dev/archive/2d109173-docs-exhaustive-audit
Issue `8ea5b13d`. Plan eligible with no blockers. 16 relocated, 0 mirrored, 10 retained, 6 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/2b9a80fb-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/2b9a80fb-audit-notes.md` | 2b9a80fb |
| move | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/36d5451e-audit-notes.md` | 36d5451e |
| move | `dev/active/36d5451e-review-round-4.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/36d5451e-review-round-4.md` | 36d5451e |
| move | `dev/active/36d5451e-review-round-5.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/36d5451e-review-round-5.md` | 36d5451e |
| move | `dev/active/36d5451e-review-round-6.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/36d5451e-review-round-6.md` | 36d5451e |
| move | `dev/active/4c33d0e5-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/4c33d0e5-audit-notes.md` | 4c33d0e5 |
| move | `dev/active/6d82de03-followup-manifest.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/6d82de03-followup-manifest.md` | 6d82de03 6d82de03 |
| move | `dev/active/6d82de03-lead-review.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/6d82de03-lead-review.md` | 6d82de03 |
| move | `dev/active/736a069e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/736a069e-audit-notes.md` | 736a069e |
| move | `dev/active/7c283e95-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/7c283e95-audit-notes.md` | 7c283e95 |
| move | `dev/active/8682e95a-gapfill-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/8682e95a-gapfill-notes.md` | 8682e95a |
| move | `dev/active/99f4a2b4-req02-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/99f4a2b4-req02-evidence.md` | 99f4a2b4 |
| move | `dev/active/a70bac75-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/a70bac75-audit-notes.md` | a70bac75 |
| move | `dev/active/adc4c6ef-relocation-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/adc4c6ef-relocation-notes.md` | adc4c6ef |
| move | `dev/active/b8924a6d-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/b8924a6d-audit-notes.md` | b8924a6d |
| move | `dev/active/d24008f0-req01-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/dev/active/d24008f0-req01-evidence.md` | d24008f0 |
| retain | `dev/archive/2d109173-completion-report.md` | already under the archive root | 2d109173 |
| retain | `dev/archive/2d109173-final-rework-review.md` | already under the archive root | 2c990aa0 47aaad9d b35c267a |
| retain | `dev/archive/2d109173-investigation.md` | already under the archive root | 3c4cab67 |
| retain | `dev/archive/2d109173-plan.md` | already under the archive root | 2b9a80fb 36d5451e 3c4cab67 4c33d0e5 6d82de03 736a069e 7c283e95 8682e95a 99f4a2b4 a70bac75 adc4c6ef b8924a6d d24008f0 |
| retain | `dev/archive/2d109173-wave7-review.md` | already under the archive root | 33b2c714 58bc3826 85aae6d2 |
| retain | `dev/archive/bug-fixes/2d109173-planning-brief.md` | already under the archive root | 2d109173 |
| retain | `dev/reference/configuration.md` | retained in place | — |
| retain | `mcp-server/README.md` | retained in place | — |
| retain | `mcp-server/curated-tools.json` | retained in place | — |
| retain | `scripts/docs-check-selftest.sh` | retained in place | 99f4a2b4 |

### In-content citation warnings (32)

| citing site | moving artifact |
|---|---|
| `dev/active/36d5451e-review-round-4.md:38:4` | `dev/active/36d5451e-audit-notes.md` |
| `dev/active/6d82de03-followup-manifest.md:256:6` | `dev/active/2b9a80fb-audit-notes.md` |
| `dev/active/6d82de03-followup-manifest.md:297:6` | `dev/active/36d5451e-audit-notes.md` |
| `dev/active/6d82de03-followup-manifest.md:337:6` | `dev/active/36d5451e-audit-notes.md` |
| `dev/active/8b05a612-investigation.md:61:4` | `dev/active/36d5451e-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1744:14` | `dev/active/2b9a80fb-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1745:14` | `dev/active/36d5451e-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1746:14` | `dev/active/36d5451e-review-round-4.md` |
| `dev/active/8e071e18-breakdown.json:1747:14` | `dev/active/36d5451e-review-round-5.md` |
| `dev/active/8e071e18-breakdown.json:1748:14` | `dev/active/36d5451e-review-round-6.md` |
| `dev/active/8e071e18-breakdown.json:1749:14` | `dev/active/4c33d0e5-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1750:14` | `dev/active/6d82de03-followup-manifest.md` |
| `dev/active/8e071e18-breakdown.json:1751:14` | `dev/active/6d82de03-lead-review.md` |
| `dev/active/8e071e18-breakdown.json:1752:14` | `dev/active/736a069e-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1753:14` | `dev/active/7c283e95-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1754:14` | `dev/active/8682e95a-gapfill-notes.md` |
| `dev/active/8e071e18-breakdown.json:1755:14` | `dev/active/99f4a2b4-req02-evidence.md` |
| `dev/active/8e071e18-breakdown.json:1756:14` | `dev/active/a70bac75-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1757:14` | `dev/active/adc4c6ef-relocation-notes.md` |
| `dev/active/8e071e18-breakdown.json:1758:14` | `dev/active/b8924a6d-audit-notes.md` |
| `dev/active/8e071e18-breakdown.json:1759:14` | `dev/active/d24008f0-req01-evidence.md` |
| `dev/active/8e071e18-progress.json:1501:154` | `dev/active/2b9a80fb-audit-notes.md` |
| `dev/active/99f4a2b4-req02-evidence.md:15:104` | `dev/active/99f4a2b4-req02-evidence.md` |
| `dev/active/9b7b5f9c-investigation.md:328:4` | `dev/active/d24008f0-req01-evidence.md` |
| `dev/active/ca832358/req05-archival-execution-evidence.md:142:23` | `dev/active/4c33d0e5-audit-notes.md` |
| `dev/archive/2d109173-handoff-2.md:50:17` | `dev/active/36d5451e-audit-notes.md` |
| `dev/archive/2d109173-handoff-3.md:46:25` | `dev/active/36d5451e-review-round-6.md` |
| `dev/archive/2d109173-handoff-3.md:47:24` | `dev/active/36d5451e-audit-notes.md` |
| `dev/archive/2d109173-handoff-4.md:56:21` | `dev/active/6d82de03-followup-manifest.md` |
| `dev/archive/2d109173-handoff.md:28:305` | `dev/active/d24008f0-req01-evidence.md` |
| `dev/archive/2d109173-handoff.md:65:22` | `dev/active/99f4a2b4-req02-evidence.md` |
| `dev/archive/2d109173-handoff.md:65:63` | `dev/active/d24008f0-req01-evidence.md` |

### Other plan warnings (1)

- `missing-edge-target` at `dev/reference/configuration.md`

## `9ac9fdac` — dev/archive/9ac9fdac-graph-templates
Issue `f320923c`. Plan eligible with no blockers. 6 relocated, 0 mirrored, 2 retained, 1 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/57269494-apply-plan-doc.md` | `dev/archive/9ac9fdac-graph-templates/dev/active/57269494-apply-plan-doc.md` | 57269494 |
| move | `dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` | `dev/archive/9ac9fdac-graph-templates/dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` | 7d88e37d |
| move | `dev/active/9ac9fdac-completion-report.md` | `dev/archive/9ac9fdac-graph-templates/dev/active/9ac9fdac-completion-report.md` | 9ac9fdac |
| move | `dev/active/9ac9fdac-graph-templates-showcase/base.css` | `dev/archive/9ac9fdac-graph-templates/dev/active/9ac9fdac-graph-templates-showcase/base.css` | — |
| move | `dev/active/9ac9fdac-graph-templates-showcase/talk.html` | `dev/archive/9ac9fdac-graph-templates/dev/active/9ac9fdac-graph-templates-showcase/talk.html` | 9ac9fdac |
| move | `dev/active/9ac9fdac-graph-templates-showcase/themes/rust.css` | `dev/archive/9ac9fdac-graph-templates/dev/active/9ac9fdac-graph-templates-showcase/themes/rust.css` | — |
| retain | `dev/archive/features/25064508/dogfood-9ac9fdac.md` | already under the archive root | 9ac9fdac fc414353 |
| retain | `docs/concepts/planning-bracket.md` | retained in place | 04656f7b(outside) c8aa199e |

### In-content citation warnings (8)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:2128:14` | `dev/active/57269494-apply-plan-doc.md` |
| `dev/active/8e071e18-breakdown.json:2129:14` | `dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` |
| `dev/active/8e071e18-breakdown.json:2130:14` | `dev/active/9ac9fdac-completion-report.md` |
| `dev/active/8e071e18-breakdown.json:2131:14` | `dev/active/9ac9fdac-graph-templates-showcase/base.css` |
| `dev/active/8e071e18-breakdown.json:2132:14` | `dev/active/9ac9fdac-graph-templates-showcase/talk.html` |
| `dev/active/8e071e18-breakdown.json:2133:14` | `dev/active/9ac9fdac-graph-templates-showcase/themes/rust.css` |
| `dev/active/9b7b5f9c-investigation.md:320:4` | `dev/active/57269494-apply-plan-doc.md` |
| `dev/archive/9ac9fdac-handoff.md:62:49` | `dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` |

### Other plan warnings (1)

- `external-edge` at `dev/active/9ac9fdac-graph-templates-showcase/talk.html`

## `90a2dbfd` — dev/archive/90a2dbfd-item-sources
Issue `89b7f534`. Plan eligible with no blockers. 2 relocated, 0 mirrored, 1 retained, 1 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/90a2dbfd-be54-46f2-b84a-b19382c6b0f2-plan.md` | `dev/archive/90a2dbfd-item-sources/dev/active/90a2dbfd-be54-46f2-b84a-b19382c6b0f2-plan.md` | fbc37f87 |
| move | `dev/active/90a2dbfd-kinds-over-sources.md` | `dev/archive/90a2dbfd-item-sources/dev/active/90a2dbfd-kinds-over-sources.md` | 90a2dbfd |
| retain | `dev/archive/features/90a2dbfd-completion-report.md` | already under the archive root | 90a2dbfd |

### In-content citation warnings (7)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:2669:14` | `dev/active/90a2dbfd-be54-46f2-b84a-b19382c6b0f2-plan.md` |
| `dev/active/8e071e18-breakdown.json:2670:14` | `dev/active/90a2dbfd-kinds-over-sources.md` |
| `dev/archive/2821e177-addressing-v2/dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md:280:4` | `dev/active/90a2dbfd-kinds-over-sources.md` |
| `dev/archive/2821e177-addressing-v2/dev/active/2821e177-investigation.md:180:53` | `dev/active/90a2dbfd-kinds-over-sources.md` |
| `dev/archive/2821e177-addressing-v2/dev/active/2821e177-investigation.md:196:4` | `dev/active/90a2dbfd-kinds-over-sources.md` |
| `dev/archive/2821e177-addressing-v2/dev/active/7f22d6cf-breakdown-spec.md:47:4` | `dev/active/90a2dbfd-kinds-over-sources.md` |
| `dev/archive/2821e177-addressing-v2/dev/studies/addressing-v2-rule-gate-items.md:208:32` | `dev/active/90a2dbfd-kinds-over-sources.md` |

## `d7bfd4a4` — dev/archive/d7bfd4a4-observability
Issue `74a51f37`. Plan eligible with no blockers. 3 relocated, 0 mirrored, 0 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/observability-design.md` | `dev/archive/d7bfd4a4-observability/dev/active/observability-design.md` | d7bfd4a4 |
| move | `dev/plans/metrics-713ff59d.md` | `dev/archive/d7bfd4a4-observability/dev/plans/metrics-713ff59d.md` | 713ff59d |
| move | `dev/plans/stalled-detection-c802a9b0.md` | `dev/archive/d7bfd4a4-observability/dev/plans/stalled-detection-c802a9b0.md` | c802a9b0 |

### In-content citation warnings (5)

| citing site | moving artifact |
|---|---|
| `dev/active/8e071e18-breakdown.json:2944:14` | `dev/active/observability-design.md` |
| `dev/active/8e071e18-breakdown.json:2945:14` | `dev/plans/metrics-713ff59d.md` |
| `dev/active/8e071e18-breakdown.json:2946:14` | `dev/plans/stalled-detection-c802a9b0.md` |
| `dev/active/8e071e18-breakdown.json:3605:2504` | `dev/active/observability-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:2596` | `dev/active/observability-design.md` |

## `ad601a15` — dev/archive/ad601a15-parallel-work
Issue `60c398d3`. Plan eligible with no blockers. 15 relocated, 0 mirrored, 0 retained, 0 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/design/cli-quality-improvements.md` | `dev/archive/ad601a15-parallel-work/dev/design/cli-quality-improvements.md` | 4b2cb4cd |
| move | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/dev/design/worktree-parallel-work.md` | 65e7dccd 7051d24e 730d25b5 909c78cc 92bf3a9b ad601a15 b74af86f f0235aa4 f84945f7 |
| move | `dev/experiments/worktree-manual-coordination-experiment.md` | `dev/archive/ad601a15-parallel-work/dev/experiments/worktree-manual-coordination-experiment.md` | ad601a15 |
| move | `dev/sessions/session-20260103-parallel-work-design-review.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260103-parallel-work-design-review.md` | ad601a15 |
| move | `dev/sessions/session-20260111-validate-implementation.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260111-validate-implementation.md` | f8e58e7d |
| move | `dev/sessions/session-20260115-cli-quality-phase3.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260115-cli-quality-phase3.md` | 4b2cb4cd |
| move | `dev/sessions/session-20260115-phase3-issue7-complete.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260115-phase3-issue7-complete.md` | 4b2cb4cd |
| move | `dev/sessions/session-20260115-story-review-f023.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260115-story-review-f023.md` | f0235aa4 |
| move | `dev/sessions/session-20260118-f849-manual-testing.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260118-f849-manual-testing.md` | f84945f7 |
| move | `dev/sessions/session-20260201-cli-enforcement-82b17394.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260201-cli-enforcement-82b17394.md` | 82b17394 |
| move | `dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` | 5e1d5f02 |
| move | `dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` | 1bdc5395 |
| move | `dev/sessions/session-20260201-refactor-items-5-7.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-20260201-refactor-items-5-7.md` | 1bdc5395 |
| move | `dev/sessions/session-claim-coordination-parallel.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-claim-coordination-parallel.md` | b74af86f |
| move | `dev/sessions/session-claim-coordination-review.md` | `dev/archive/ad601a15-parallel-work/dev/sessions/session-claim-coordination-review.md` | b74af86f |

### In-content citation warnings (81)

| citing site | moving artifact |
|---|---|
| `crates/jit/src/storage/claim_coordinator.rs:13:22` | `dev/design/worktree-parallel-work.md` |
| `crates/jit/tests/cli_repo_workflow/config_get_tests.rs:369:42` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-breakdown.json:1653:1149` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-breakdown.json:1653:1196` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-breakdown.json:1653:1260` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-breakdown.json:1653:1329` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-breakdown.json:1690:14` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-breakdown.json:3001:14` | `dev/design/cli-quality-improvements.md` |
| `dev/active/8e071e18-breakdown.json:3002:14` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-breakdown.json:3003:14` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-breakdown.json:3004:14` | `dev/sessions/session-20260103-parallel-work-design-review.md` |
| `dev/active/8e071e18-breakdown.json:3005:14` | `dev/sessions/session-20260111-validate-implementation.md` |
| `dev/active/8e071e18-breakdown.json:3006:14` | `dev/sessions/session-20260115-cli-quality-phase3.md` |
| `dev/active/8e071e18-breakdown.json:3007:14` | `dev/sessions/session-20260115-phase3-issue7-complete.md` |
| `dev/active/8e071e18-breakdown.json:3008:14` | `dev/sessions/session-20260115-story-review-f023.md` |
| `dev/active/8e071e18-breakdown.json:3009:14` | `dev/sessions/session-20260118-f849-manual-testing.md` |
| `dev/active/8e071e18-breakdown.json:3010:14` | `dev/sessions/session-20260201-cli-enforcement-82b17394.md` |
| `dev/active/8e071e18-breakdown.json:3011:14` | `dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` |
| `dev/active/8e071e18-breakdown.json:3012:14` | `dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` |
| `dev/active/8e071e18-breakdown.json:3013:14` | `dev/sessions/session-20260201-refactor-items-5-7.md` |
| `dev/active/8e071e18-breakdown.json:3014:14` | `dev/sessions/session-claim-coordination-parallel.md` |
| `dev/active/8e071e18-breakdown.json:3015:14` | `dev/sessions/session-claim-coordination-review.md` |
| `dev/active/8e071e18-breakdown.json:3274:1259` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-breakdown.json:3274:1366` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-breakdown.json:3734:3338` | `dev/sessions/session-20260103-parallel-work-design-review.md` |
| `dev/active/8e071e18-breakdown.json:3734:3456` | `dev/sessions/session-20260103-parallel-work-design-review.md` |
| `dev/active/8e071e18-breakdown.json:3734:3694` | `dev/sessions/session-20260111-validate-implementation.md` |
| `dev/active/8e071e18-breakdown.json:3734:3808` | `dev/sessions/session-20260111-validate-implementation.md` |
| `dev/active/8e071e18-breakdown.json:3734:3870` | `dev/sessions/session-20260115-cli-quality-phase3.md` |
| `dev/active/8e071e18-breakdown.json:3734:3979` | `dev/sessions/session-20260115-cli-quality-phase3.md` |
| `dev/active/8e071e18-breakdown.json:3734:4036` | `dev/sessions/session-20260115-phase3-issue7-complete.md` |
| `dev/active/8e071e18-breakdown.json:3734:4149` | `dev/sessions/session-20260115-phase3-issue7-complete.md` |
| `dev/active/8e071e18-breakdown.json:3734:4210` | `dev/sessions/session-20260115-story-review-f023.md` |
| `dev/active/8e071e18-breakdown.json:3734:4318` | `dev/sessions/session-20260115-story-review-f023.md` |
| `dev/active/8e071e18-breakdown.json:3734:4374` | `dev/sessions/session-20260118-f849-manual-testing.md` |
| `dev/active/8e071e18-breakdown.json:3734:4484` | `dev/sessions/session-20260118-f849-manual-testing.md` |
| `dev/active/8e071e18-breakdown.json:3734:4542` | `dev/sessions/session-20260201-cli-enforcement-82b17394.md` |
| `dev/active/8e071e18-breakdown.json:3734:4657` | `dev/sessions/session-20260201-cli-enforcement-82b17394.md` |
| `dev/active/8e071e18-breakdown.json:3734:4720` | `dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` |
| `dev/active/8e071e18-breakdown.json:3734:4837` | `dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` |
| `dev/active/8e071e18-breakdown.json:3734:4902` | `dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` |
| `dev/active/8e071e18-breakdown.json:3734:5019` | `dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` |
| `dev/active/8e071e18-breakdown.json:3734:5084` | `dev/sessions/session-20260201-refactor-items-5-7.md` |
| `dev/active/8e071e18-breakdown.json:3734:5193` | `dev/sessions/session-20260201-refactor-items-5-7.md` |
| `dev/active/8e071e18-breakdown.json:3734:6161` | `dev/sessions/session-claim-coordination-parallel.md` |
| `dev/active/8e071e18-breakdown.json:3734:6270` | `dev/sessions/session-claim-coordination-parallel.md` |
| `dev/active/8e071e18-breakdown.json:3734:6327` | `dev/sessions/session-claim-coordination-review.md` |
| `dev/active/8e071e18-breakdown.json:3734:6434` | `dev/sessions/session-claim-coordination-review.md` |
| `dev/active/8e071e18-investigation.md:1304:77` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1305:104` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1306:75` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1307:73` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1308:88` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1309:62` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1310:62` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1311:4` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-investigation.md:1311:83` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1316:2` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1355:35` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-investigation.md:1360:62` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-investigation.md:1362:43` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:1370:55` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/active/8e071e18-investigation.md:1383:58` | `dev/design/worktree-parallel-work.md` |
| `dev/active/8e071e18-investigation.md:783:5` | `dev/design/worktree-parallel-work.md` |
| `dev/archive/2d109173-docs-exhaustive-audit/dev/active/4c33d0e5-audit-notes.md:145:4` | `dev/design/worktree-parallel-work.md` |
| `dev/archive/4a00b2b0-agent-validation/dev/studies/ai-tool-worktree-compatibility.md:210:37` | `dev/design/worktree-parallel-work.md` |
| `dev/archive/4a00b2b0-agent-validation/dev/studies/ai-tool-worktree-compatibility.md:226:39` | `dev/design/worktree-parallel-work.md` |
| `dev/archive/4a00b2b0-agent-validation/dev/studies/ai-tool-worktree-compatibility.md:229:39` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/archive/4a00b2b0-agent-validation/dev/studies/ai-tool-worktree-compatibility.md:81:66` | `dev/design/worktree-parallel-work.md` |
| `dev/archive/features/dbe1e821/dbe1e821-progress.json:177:669` | `dev/design/worktree-parallel-work.md` |
| `dev/experiments/worktree-manual-coordination-experiment.md:377:47` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-20260103-parallel-work-design-review.md:34:26` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-20260103-parallel-work-design-review.md:358:21` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-20260118-f849-manual-testing.md:60:24` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-claim-coordination-parallel.md:143:24` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-claim-coordination-parallel.md:188:24` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-claim-coordination-parallel.md:308:16` | `dev/design/worktree-parallel-work.md` |
| `dev/sessions/session-claim-coordination-parallel.md:309:24` | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/sessions/session-claim-coordination-parallel.md:82:24` | `dev/design/worktree-parallel-work.md` |
| `docs/how-to/multi-agent-coordination.md:472:21` | `dev/design/worktree-parallel-work.md` |
| `docs/tutorials/parallel-work-worktrees.md:257:21` | `dev/design/worktree-parallel-work.md` |

### Other plan warnings (1)

- `external-edge` at `dev/design/worktree-parallel-work.md`

## `2fbd2a82` — dev/archive/2fbd2a82-planning-bracket
Issue `deb50576`. Plan eligible with no blockers. 4 relocated, 0 mirrored, 3 retained, 1 already archived.

### Artifacts

| action | source | destination | owners |
|---|---|---|---|
| move | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/dev/active/planning-bracket-design.md` | 2fbd2a82 8646a474 |
| move | `dev/active/planning-bracket-showcase/base.css` | `dev/archive/2fbd2a82-planning-bracket/dev/active/planning-bracket-showcase/base.css` | — |
| move | `dev/active/planning-bracket-showcase/talk.html` | `dev/archive/2fbd2a82-planning-bracket/dev/active/planning-bracket-showcase/talk.html` | 2fbd2a82 |
| move | `dev/active/planning-bracket-showcase/themes/rust.css` | `dev/archive/2fbd2a82-planning-bracket/dev/active/planning-bracket-showcase/themes/rust.css` | — |
| retain | `dev/archive/features/2fbd2a82-completion-report.md` | already under the archive root | 2fbd2a82 |
| retain | `docs/concepts/planning-bracket.md` | retained in place | 04656f7b c8aa199e(outside) |
| retain | `docs/how-to/adopt-planning-bracket.md` | retained in place | 04656f7b |

### In-content citation warnings (17)

| citing site | moving artifact |
|---|---|
| `crates/jit/tests/fast_docs_templates/bracket_breakdown_tests.rs:2:6` | `dev/active/planning-bracket-design.md` |
| `crates/jit/tests/fast_docs_templates/research_bracket_tests.rs:27:7` | `dev/active/planning-bracket-design.md` |
| `crates/jit/tests/fast_docs_templates/sdd_bracket_tests.rs:19:7` | `dev/active/planning-bracket-design.md` |
| `dev/active/2fbd2a82-14ba-4e6e-90f6-e0c34f0f912c-plan.md:18:9` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:2373:14` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:2374:14` | `dev/active/planning-bracket-showcase/base.css` |
| `dev/active/8e071e18-breakdown.json:2375:14` | `dev/active/planning-bracket-showcase/talk.html` |
| `dev/active/8e071e18-breakdown.json:2376:14` | `dev/active/planning-bracket-showcase/themes/rust.css` |
| `dev/active/8e071e18-breakdown.json:3274:1482` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:3274:1598` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:3274:1709` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:3459:1251` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:2636` | `dev/active/planning-bracket-design.md` |
| `dev/active/8e071e18-breakdown.json:3605:2734` | `dev/active/planning-bracket-design.md` |
| `dev/archive/2d109173-docs-exhaustive-audit/dev/active/a70bac75-audit-notes.md:102:38` | `dev/active/planning-bracket-design.md` |
| `dev/archive/features/2fbd2a82-progress.json:6:18` | `dev/active/planning-bracket-design.md` |
| `dev/archive/features/dbe1e821/dbe1e821-progress.json:177:626` | `dev/active/planning-bracket-design.md` |

### Other plan warnings (1)

- `external-edge` at `dev/active/planning-bracket-showcase/talk.html`
