# Epic Complete: Dependency-aware container artifact archival (7d3a3a47)

**Started:** 2026-07-13  
**Completed:** 2026-07-14  
**Assignee:** agent:jit-execution-lead

## Summary

JIT now plans and executes dependency-aware archival for a document or completed container, preserving supported local bundles, ownership, and issue references while refusing unsafe mutations. It also reports every terminal container candidate without mutation and has removed the superseded single-document archive implementation.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 10 / 10 implementation issues; 12 / 12 including planning and breakdown |
| Waves executed | 10 |
| Rework cycles | 11 across six issues |
| Escalations | 2 |
| Sub-agent dispatches | 23, including rework and independent audits |
| Issues created during execution | 1 |

## Success Criteria

- [x] REQ-01: Produces a deterministic archive plan containing every issue-linked artifact in a container subtree and every supported embedded local dependency of its working-tree artifacts — delivered by `28fb4e6a`, `b45ad545`, and `971c67f4`.
- [x] REQ-02: Classifies each artifact as move, copy, retain, or blocked using active references, sharing, managed-path policy, and destination conflicts — delivered by `b45ad545`, `971c67f4`, and `0c86d83b`.
- [x] REQ-03: Archives an eligible container without losing files, overwriting destinations, or leaving dangling issue document references — delivered by `67f4ded1` and `f205aa7e`.
- [x] REQ-04: Preserves functional relative links for supported bundles, including HTML presentations with sibling CSS, theme files, and figures — delivered by `971c67f4`, `0c86d83b`, and `f205aa7e`.
- [x] REQ-05: Reports container-oriented archival candidates using current terminal state, configured documentation policy, artifact ownership, and plan blockers without mutating the repository — delivered by `a9f8e04d` and the discovered-gap fix `b3fd1d73`.
- [x] REQ-06: Supports structured JSON previews and is verified against Markdown, HTML, CSS, CSV, PNG, and SVG fixtures representative of JIT and gf2 — delivered by `41aa6056`.

The clean-cut removal task `271dc224` retired the legacy archive surface after the replacement workflow and MCP schema were complete.

## Wave Execution Log

**Wave 1:** `28fb4e6a` — established the versioned, canonical artifact-plan schema and diagnostic taxonomy.  
**Wave 2:** `67f4ded1` — added storage-owned staging, publishing, relinking, rollback, and event primitives.  
**Wave 3:** `b45ad545` — resolved container roots and direct ownership from the configured DAG and document metadata.  
**Wave 4:** `971c67f4` — recursively discovered supported Markdown, HTML, CSS, script, and asset dependencies without following symlinks or pins.  
**Wave 5:** `0c86d83b` — classified artifacts as move, copy, retain, or block from ownership, policy, topology, and filesystem facts.  
**Wave 6:** `41aa6056` — exposed deterministic document/container previews and canonical JSON/human rendering.  
**Wave 7:** `f205aa7e` — executed eligible plans with guarded mutation, referential consistency, rollback, residue handling, and durable events.  
**Wave 8:** `a9f8e04d` — added the complete read-only terminal-container candidate report.  
**Wave 9:** `271dc224` — removed the obsolete document-archive CLI/API/event/MCP surface while preserving historical event readability.  
**Wave 10:** `b3fd1d73` — converted non-regular artifact inputs into path-specific diagnostics so one directory link cannot abort candidate enumeration.

## Key Decisions

- Kept the safety contract at referential consistency under the repository write guard, with staged writes, destination checks, rollback, and git history as the recovery channel instead of claiming literal cross-file atomicity.
- Preserved bundle topology so supported relative links continue to work, rather than introducing generalized link rewriting.
- Treated pinned references as historical and non-relocating, and separated direct ownership from repository-wide embedded ownership so only direct references are relinked.
- Used a canonical schema-version-1 plan as the shared preview/execution contract; execution consumes the same evaluated plan and refuses every blocker.
- Removed the legacy surface only after new CLI, JSON, event, documentation, and MCP behavior was verified.
- Represented directory and other non-regular sources as `unsupported-artifact-type`, while keeping unexpected metadata and storage failures fatal; repository-wide ownership discovery prunes only the representable unsupported branch.

## Escalations

- `971c67f4` exceeded the normal two-cycle rework limit on dynamic-loader coverage. The invoker authorized an exceptional third cycle; bare same-directory URL forms were covered and all gates passed.
- `41aa6056` exceeded the normal two-cycle rework limit on archive-preview documentation structure. The invoker authorized an exceptional third cycle; the reference was clarified and both doc gates passed.

## Issues Discovered During Execution

- `b3fd1d73` — Report unreadable artifacts without aborting archive candidates (created during wave 10 after epic doc review reproduced a repository-wide `dev/active/` directory link aborting the complete candidate report).

## Holistic Quality Notes

- Every one of the 12 resolved descendants is Done with all required gates passed; the epic passes `repo-validate`, `doc-review`, and `docs-mechanical`.
- The trusted holistic review inspected the complete tagged implementation history and found the archival contract accurate. Its final discoverability finding was corrected by linking README's archive examples to the canonical archive reference.
- The exact dogfood command `jit archive candidates --json` completed on this repository with 104 candidates, 63 eligible plans, and 552 unsupported-directory diagnostics, without mutation or omission.
- Deferred time-based retention, Git LFS policy, generalized link rewriting, and richer symlink behavior remain explicit non-goals rather than hidden partial implementations.
