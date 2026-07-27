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
