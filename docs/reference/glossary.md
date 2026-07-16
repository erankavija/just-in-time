# Glossary

> **Diátaxis Type:** Reference

## Core Concepts

- **Issue**: A unit of work tracked by JIT
- **Dependency**: A prerequisite relationship: “A depends on B” means B blocks A
- **Gate**: A quality checkpoint that must pass before completion
- **State**: Current lifecycle stage of an issue
- **Label**: A namespace:value tag for categorization
- **Assignee**: Who is responsible for an issue

## Issue States

**Backlog**: Not yet ready to work on

**Ready**: Unblocked and available to claim

**InProgress**: Actively being worked on

**Gated**: Waiting for quality gate checks

**Done**: Successfully completed

**Rejected**: Closed without implementation

Terminal state for issues that won't be completed. Common reasons include duplicates, won't-fix decisions, invalid requests, or out-of-scope work. Bypasses all gate enforcement when transitioning.

**Resolution Label**: Documents why an issue was rejected

Format: `resolution:reason` (e.g., `resolution:duplicate`, `resolution:wont-fix`). Added automatically via `jit issue reject --reason <REASON>` command.

**Archived**: Retired lifecycle state, parked out of active views

Terminality-preserving: it records the state it was entered from and keeps whatever that state meant for dependents. An issue archived from a terminal state (`Done`/`Rejected`) stays effectively terminal; one archived from a non-terminal state does not satisfy dependents. Reviving restores the recorded origin state exactly. Distinct from `jit archive`, which relocates linked documents on disk.

**Archive (command)**: `jit archive`, the dependency-aware relocation of linked documents into an on-disk mirror (`archive_root`). Distinct from the `Archived` lifecycle state; a successful `jit archive container` retires its container into `Archived`.

## Gate Types

**Precheck**: Gate that must pass before work begins

**Postcheck**: Gate that must pass before completion

**Manual Gate**: Requires human judgment

**Automated Gate**: Runs a checker script

## Label Types

**Strategic**: High-level categorization (milestone, epic)

**Tactical**: Work-level categorization (type, component)

## Dependency Relationships

**Depends On / Blocks**: “A depends on B” means B blocks A. B must reach a
terminal state before A can become ready.

**Blocked By**: A is blocked by B when A depends on B

**Transitive**: If A depends on B and B depends on C, A is transitively blocked
by C

**Transitive Reduction**: Minimal set of dependencies

## Other Terms

**DAG**: Directed Acyclic Graph (no cycles)

**Short Hash**: 8-character UUID prefix for referencing issues

**Assignee Type**: Kind prefix of an assignee (e.g. `agent:`, `human:`, `ci:`); any non-empty kind is accepted

**Event Log**: Append-only audit trail of issue state changes and related lifecycle events (not every repository mutation)
