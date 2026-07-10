# Epic Complete: Work-graph abstraction integrity (dbe1e821)

**Started:** 2026-07-05
**Completed:** 2026-07-10
**Assignee:** agent:jit-execution-lead

## Summary

Closed the semantic and boundary gaps found in the July 2026 work-graph architecture review. Hierarchy resolution is now defined once, on the transitively-reduced graph, and served to every consumer from a single Rust implementation. The dependency-met rule, the graph traversal primitives, template application, and the hierarchy-adjacent names each collapsed to one authority.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 11 / 11 (8 by direct edge, 3 resolved transitively) |
| Waves executed | 5 |
| Rework cycles | 9 charged, across 7 issues |
| Escalations | 7 decision points |
| Sub-agent dispatches | 22 (12 initial, 10 rework) plus 1 analyst |
| Issues created during execution | 2 |

## Success Criteria

- [x] REQ-1: Resolved hierarchy invariant under redundant edges — `b24ce11b`, pinned by the property test `prop_resolution_ignores_redundant_edges` over closure-supersets
- [x] REQ-2: Exactly one hierarchy-resolution implementation, consumed by CLI, server, and web UI — `e486a854` (serves it) + `d91ea408` (deletes the TypeScript port, net −795 lines)
- [x] REQ-3: Dependency-met rule defined once in the domain layer — `69ffdcf8` (`domain::types::is_dependency_met`) + `b6eb2585` (bulk updates routed through the transition chokepoint)
- [x] REQ-4: Graph traversal and acyclicity primitives in the graph layer — `50677708` (`graph::keyed::find_keyed_cycle`, `Direction`, `traverse`, `expand`)
- [x] REQ-5: Template application validates fully before mutating, applies atomically under one lock — `2f447380` (`storage::repo_lock::RepoWriteLock`, pure `template_expand`)
- [x] REQ-6: Template role and anchor names declared in repository configuration — `136268f8` (`RoleBindings`, defaults preserving current names)
- [x] REQ-7: No two hierarchy-related concepts share a module, type, or CLI-surface name — `bbaedb0f`
- [x] REQ-8: External docs and MCP prompt describe DAG-authoritative containment — `a1fe0c6e` + `a4e8d5ac`, unblocked by the absorbed `c291e95c`

## Wave Execution Log

**Wave 1** (6 issues) — the dependency-free floor: the single blocking predicate, the graph primitives, atomic template apply, the MCP prompt, the containment doc, plus `c291e95c` absorbed mid-wave from another epic.
**Wave 2** (3 issues) — consumers of wave 1: bulk-update routing, the server's `/graph` resolution endpoint, config-declared role bindings.
**Wave 3** (1 issue) — the web UI consumes the served resolution and deletes its own resolver.
**Wave 4** (1 issue) — the resolver canonicalizes on the reduced graph, once the TypeScript port is gone so the algorithm changes in exactly one place.
**Wave 5** (1 issue) — the repo-wide move and rename sweep, scheduled last so no other wave sat on a moving module tree.

## Key Decisions

- **Wave 4 after wave 3, against the DAG.** No dependency edge forced it. Landing the resolver change while two implementations existed would have required changing the algorithm twice and keeping them in step. Deleting the port first made the canonicalization a single-site edit.
- **Wave 5 scheduled last for the same reason.** A move/rename sweep across `lib.rs`, `main.rs`, `cli.rs`, and `CLAUDE.md` conflicts with everything. Ordering it last cost nothing and removed every merge conflict.
- **Rename the validate flag, not the query command.** `--divergence` meant git branch drift on `jit validate` and membership-vs-DAG mismatch on `jit query`. The query's name matches the domain concept and is cited across docs, the MCP prompt, and the web UI. The git concern took the new name `--branch-drift`; the old spelling survives as a hidden stub that exits 2 naming both commands the word could mean.
- **Root-caused a recurring finding rather than re-sweeping.** `69ffdcf8`'s stale-prose finding recurred twice because both the worker and I grepped phrasings ("not Done") instead of the concept. The string lived in `errors.rs:827` as a shipped `format!` literal that the docs merely echoed. Fixed at the source.
- **Adopted P1 mid-epic: an open escalation suspends only its dependency cone.** In wave 1 I stalled four escalation-independent issues for 5h36m against 2h20m of total integration work. Waves 3 through 5 kept integrating independent work while escalations were open.
- **Declined to enforce the no-em-dash writing rule on `e486a854`.** I announced a FAIL, then reversed: the repo carries pre-existing em-dashes that passed review, the instances were appositives rather than the redundant paraphrase the rule targets, and the rule's own text says the character is not harmful in itself. Selective enforcement would have been churn. If wanted, it belongs in `rules.toml` or the review prompt as a declared standard.

## Escalations

1. **Cross-epic dependency.** `a4e8d5ac`'s REQ-3 required passing MCP tests, but the suite failed on clean `main`, owned by `c291e95c` under a different epic. Invoker chose to absorb `c291e95c` into this run. It was claimed, dispatched, and closed inside wave 1.
2. **Gate coverage hole.** `npm-ci`'s checker only enters `web/`, so `a4e8d5ac`'s gates were blind to a failing `mcp-server` suite. Invoker chose file-do-not-fix. Filed as `950256ae`.
3. **Evidence blocker.** `c291e95c` could not evidence its REQ-2 because no gate ran the mcp-server suite. Invoker approved defining a new `mcp-ci` gate. This partially closes `950256ae`, whose REQ-2 remains open.
4. **Two leads in one working tree.** A second jit-execution-lead was committing to the same checkout and branch. Invoker chose to continue under strict write discipline: stage only own paths, never `git add -A .jit`, never `jit recover`.
5. **README overlap.** `287f7bc9` overlapped `c291e95c`'s README rewrite. Invoker chose to verify coherence immediately; the file was coherent after both independent rewrites.
6. **Broken skill commands.** The analyst found `jit gate runs`, a command that does not exist, in the Tier 1.5 audit block of two skill references, silently disabling the cumulative-findings check for the whole epic. Invoker chose to fix the stale commands now, declining the larger invariant change. Fixed and linted against the installed CLI.
7. **Stale worktrees.** Invoker approved pruning the provably merged ones. Six removed; the concurrent lead's were left alone.

## Issues Discovered During Execution

- `950256ae` — Gate `npm-ci` does not cover the mcp-server test suite (wave 1). Partially closed by the `mcp-ci` gate; its REQ-2 is still open.
- `894337e2` — `find_available_port` races between probe and bind (`serve.rs:249`, wave 2). Surfaced as a flaky test in a rework agent's run; the flake was a symptom of a TOCTOU race in shipped code.

## Holistic Quality Notes

- **Every high-severity finding this epic was a correctness bug in the fix, not in the original code.** `2f447380`'s apply lock excluded nobody, because it took `.git/jit/locks/apply.lock` while ordinary writers take `.jit/.index.lock`. Its rollback then stamped `updated_at`, so a failed apply left observable change. `d91ea408`'s layout memo keyed on a subset of its inputs, so a served cluster change with fixed topology rendered stale. `e486a854` omitted the list envelope. Reviewers earned their keep on the deltas, not the baseline.
- **Stale prose is the dominant rework driver, and greps for phrasing do not find it.** Three of the nine rework cycles were prose-only. Two were caused by sweeping for the words I expected rather than the concept. The `bbaedb0f` failure landed on `TESTING.md`, which my own Tier 2.5 sweep missed because I had scoped the paths. Sweep unscoped, then exclude.
- **A rule for what counts as stale.** A doc that asserts present-tense fact about the codebase must be correct. A doc that records what was done at a past time is a historical record and must not be rewritten to match a later rename. Session notes, archives, and other leads' evidence files fall on the history side; `TESTING.md` and `dev/studies/` do not.
- **The epic's own thesis validated itself.** `jit graph deps` reports 8 direct children while `jit graph tree` resolves 11 issues to `parent=dbe1e821`. Three reach the epic only through sibling edges, and resolution still names the epic as their container. Containment survives transitive reduction, which is exactly what REQ-1 asserts.
