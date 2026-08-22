# v1.0 Production Readiness Scope Brief

**Strategic parent:** 9db27a3a
**Decisions:** `@/charter/D-9` through `@/charter/D-13`
**Date:** 2026-07-14

Make the v1.0 tag a reproducible, secure, supportable release while reducing packaging and documentation bloat without removing the CLI, server/API, web UI, or MCP capabilities.

## Background

The product behavior and test base are mature, but the release path is not yet a production contract. Dependency audits currently report actionable findings, security jobs are advisory, public distribution and version alignment are incomplete, Docker exposes several overlapping topologies, and installation/deployment facts are repeated across documents. Core correctness issues already owned by `6eb585bc` remain with that active execution lead and are dependencies, not duplicate work here.

## Success Criteria

- [hard] REQ-01: The supported v1.0 capability set remains the native CLI, Rust server/API, built web UI, and MCP server; removals in this epic are limited to redundant packaging, deployment topologies, compatibility aliases already scheduled for removal, and repeated prose.
- [hard] REQ-02: `cargo audit`, `npm audit --omit=dev` in `web/`, and `npm audit --omit=dev` in `mcp-server/` report no advisories at the release candidate; dependencies without a safe upgrade are removed or replaced, and no blanket advisory allowlist is introduced.
- [hard] REQ-03: Security automation fails on audit-tool installation errors and findings, contains no `continue-on-error` or `|| true` escape hatch, runs on pull requests and the release commit, and is a required dependency of the release workflow.
- [hard] REQ-04: A `v1.0.0` release is permitted only when the tag, Rust crate manifests, MCP package, server, and documented compatibility versions agree; the normal Rust, web, MCP, repository-validation, and security suites have passed on the tagged commit.
- [hard] REQ-05: Every documented installation path is backed by a published, smoke-tested artifact. The GitHub release carries the supported native binaries, license texts, checksums, and an SBOM; the MCP package is published through its documented package channel; release notes and changelog describe compatibility and upgrade expectations.
- [hard] REQ-06: Docker support consists of one production image in which the Rust server serves both API and built web UI, runs as a non-root user with correct PID 1 and signal behavior, has a health check, and operates on a whole repository bind-mounted at `/repo` so `.jit/` and linked project documents retain repository context.
- [hard] REQ-07: The CLI image, split API/web images, and current all-in-one image are removed; the Compose example runs the one supported service; pull requests build and runtime-smoke the image; version tags publish immutable release images while `main` tags remain development-only.
- [hard] REQ-08: README, installation, deployment, Docker, MCP, and component documentation each have a distinct audience purpose and link to canonical workflow/reference pages instead of repeating commands, support matrices, configuration facts, or guarantees. Pruning preserves discoverability and passes mechanical plus adversarial documentation review.
- [hard] REQ-09: Workspace `rust-version` is 1.97 and CI builds/tests with exactly that compiler; immediately before the v1.0 tag it is compared with current stable and refreshed if it would be more than one stable release behind. The policy and update procedure are documented once.
- [hard] REQ-10: A clean release-candidate rehearsal from a fresh checkout verifies native install and `jit init`, profile quickstart, CLI JSON schema, server/API/web startup, MCP package startup, Docker startup and repository persistence, and all supported upgrade/install instructions without relying on uncommitted files or a source-checkout-only asset.
- [hard] REQ-11: Work already filed under core maintenance, including CLI output contracts, validation correctness, gate behavior, and build-efficiency tasks, is consumed through the DAG and verified at release acceptance rather than reimplemented in this epic.

## Risks

- RISK-01: Security upgrades can force API or behavior changes; tests and release rehearsal must distinguish required migration from regression.
- RISK-02: Docker consolidation can accidentally drop web routing, repository-relative documents, or signal handling; runtime smoke must exercise each contract.
- RISK-03: Documentation pruning can remove discovery along with repetition; entry-point coverage and link checks are required.
- RISK-04: Parallel core-maintenance changes can overlap release files; this epic must rebase after that stream and avoid duplicate ownership.
