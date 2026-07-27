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
