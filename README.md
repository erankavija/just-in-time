# Just-In-Time Issue Tracker (design scaffold)

This repository will host a minimal, CLI-first issue tracking system focused on:
- Explicit dependency graphs (DAG) between issues
- Quality gating to enforce process requirements before issues become ready/done
- Machine-consumable, deterministic, versionable plain-text storage

This PR contains only the design and repository skeleton (no implementation). Review the detailed design at docs/design.md.

Acceptance criteria for this PR:
- Documentation describes the data model, CLI surface, gating model, dependency rules, file layout, and implementation phases.
- Skeleton storage files and a sample gate registry are included.
- No executable business logic is added in this change.

Next steps after merging:
- Decide language/tooling for CLI implementation.
- Implement Phase 1 (storage init, issue create/list/show, per-issue files).
