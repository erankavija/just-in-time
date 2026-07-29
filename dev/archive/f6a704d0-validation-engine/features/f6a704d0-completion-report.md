# Completion Report

## Epic Complete: Generic Issue & Label Validation Engine (f6a704d0)

**Started:** 2026-06-06
**Completed:** 2026-06-08
**Assignee:** agent:project-lead

### Summary

Delivered a configurable, declarative validation engine for issues and labels built
on JSON Schema, with rules in `.jit/rules.toml` as the single source of truth. SDD
ships as example rules + schemas + docs. The legacy `[validation]`/namespace-constraint
config and the entire migration/backward-compat machinery were hard-removed; both
pre-1.0 consumers (this repo and ../gf2) were migrated to `rules.toml` as deliverables.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 16 / 16 (12 original + 4 scope-expansion-2) |
| Waves executed | 8 |
| Rework cycles | ~17 dispatched reworks + 5 lead-applied gate-driven fix rounds on Task D / epic |
| Escalations | 7 (6 prior sessions + 1 this session) |
| Sub-agent dispatches | ~30 (workers + reworks + adversarial/plan reviewers) |
| Issues created during execution | 4 (expansion-2 tasks) + 1 gate definition (cargo-ci-features) |

### Success Criteria

- [x] Rules load from `.jit/rules.toml`; schemas inline or from `.jit/schemas/`; loader rejects mixing shorthand and raw schema — 2fa7c882
- [x] Selector matrix (type, label, state, doc-type) matches issues; applicable rules union — 2fa7c882, 25ad2a02
- [x] Local rules evaluate on create/update; per-rule `enforce` (default false); `--force` bypasses — 25ad2a02
- [x] Graph rules (coverage, reference-integrity, dependency-shape) run only in validate/gate checkers — a7176f28
- [x] (AMENDED) Validation against the enriched projection via a per-issue `content_format` parser with repo default + Markdown fallback; HTML/XML (behind features) exercised in production; CI `--features html,xml` job — 2f50a3b0, 00525fe0, e93af54b
- [x] JSON Schema validation via the `jsonschema` crate; shorthand kinds desugar to JSON Schema — 7df61f62, 6297d67b
- [x] (AMENDED, hard removal) Complete set of existing checks re-expressed as default rules; legacy enforcement keys + migration machinery REMOVED; `rules.toml` sole source — a0f0f342, 0abaddc0, d4188154
- [x] (AMENDED) `jit init` scaffolds a fixed default; absent `rules.toml` builds defaults in-memory (no write-on-read), materialized only by init — 0abaddc0, d4188154
- [x] `jit validate [<id>] [--json] [--explain]` reports rule name, severity, message; gates invoke via `$JIT_ISSUE_ID` — b8ba1b10
- [x] SDD ships as example rules + schemas + docs; description criteria canonical, labels derived; >=1 non-SDD example — a6daa05d
- [x] `cargo clippy`/`cargo fmt` clean; tests cover engine, projection, parsers, desugaring, migration — all children (cargo-ci + cargo-ci-features gates)
- [x] (aspirational) `x-jit-*` custom-keyword extension point demonstrated — 33f23ec7
- [x] (expansion-2) jit AND gf2 migrated to `rules.toml` as deliverables — 2fb9f910 (+ this repo dogfooded)
- [x] (expansion-2) Epic-review correctness findings fixed: schema-name collision, checker-command enforce rejection, gate-execution PID-safety — 5e79ba48; plus comprehensive PID-safety in `lock_cleanup` (epic-gate finding, fixed this session)

### Wave Execution Log

- **Wave 1:** projection + Markdown parser (2f50a3b0); rule model + config loader (2fa7c882).
- **Wave 2:** JSON Schema core + keyword extension (7df61f62); HTML/XML parsers (00525fe0).
- **Wave 3:** shorthand desugaring (6297d67b); x-jit-* keyword (33f23ec7); graph rules (a7176f28).
- **Wave 4:** local rule evaluation with write-time enforcement (25ad2a02).
- **Wave 5:** jit validate + gate-checker integration (b8ba1b10).
- **Wave 6:** re-express existing checks as default rules (a0f0f342); SDD examples + docs (a6daa05d).
- **Wave 7:** jit init scaffolding + existing-repo full migration (0abaddc0).
- **Wave 8 (scope expansion 2):** epic-review findings (5e79ba48); format-agnostic production validation (e93af54b); migrate gf2 + verify this repo (2fb9f910); hard-remove legacy config + migration BC (d4188154).

### Key Decisions

- Validator cache keyed by canonical serialized schema identity (not rule name), with the decision record and issue criterion amended (user-approved).
- Custom `label_regex` validate-path deviation documented (DR §8.3a) rather than adding a write-only rule concept (user-approved).
- Expansion 1: full single-source migration (added `type-hierarchy` TOML assert kind, RuleSet→TOML serializer, file-as-source) after the first partial migration failed the single-source contract.
- Expansion 2: hard-remove all backward-compat machinery (only jit + gf2 exist pre-1.0, both migrated) rather than maintaining migration/coexistence code.
- MF4: absent `rules.toml` builds defaults in-memory (no write-on-read) — safer than the literal "auto-scaffold on read" interview answer; materialize only via `jit init` (plan-review-approved deviation).
- Task D scaffold lock: derive the control plane from the repo that OWNS the storage root (its working tree's `.git`), not the ambient process cwd, so a nested `.jit` never borrows an unrelated ancestor's control plane (code-review-driven).

### Escalations

- Wave 2 (00525fe0): feature-gated parity not covered by default cargo-ci → added dedicated `cargo-ci-features` gate (user choice).
- Wave 2 (7df61f62): contradictory cache-keying review rounds → amended DR §5.2 + issue criterion to schema-identity (user approved).
- Wave 4 (25ad2a02): 6 review rounds + flaky external-network test → adversarial reviewer adopted; flaky test pointed at loopback mock (user approved).
- Wave 6 (a0f0f342): custom label_regex parity has no write-only rule representation → documented deviation (user chose).
- Wave 7 (0abaddc0): partial-migration planning failure → user expanded scope to full single-source migration.
- Wave 8 (epic gate): 4 epic-review findings → user expanded scope (expansion 2) and plan-reviewed.
- This session (d4188154/epic): code-review gate blocked on stale failing-test history + a no-network sandbox for pre-existing serve/claim tests → user refreshed the gate environment; refreshing cargo-ci provided clean evidence and both gates passed.

### Issues Discovered During Execution

- 5e79ba48, e93af54b, 2fb9f910, d4188154 — the four scope-expansion-2 tasks created after the epic-level code-review surfaced findings (user-directed scope expansion).
- `cargo-ci-features` gate definition — created to exercise feature-gated HTML/XML paths.
- Comprehensive PID-safety: epic-level code-review found a second unguarded PID cast in `storage/lock_cleanup.rs` (`u32::MAX as i32 == -1` → `kill(-1)` risk); fixed with a checked conversion + regression test (commit 1e6f7ce).

### Holistic Quality Notes

- Every child passed cargo-ci + code-review gates plus the 6-tier lead review; an adversarial reviewer ran before each code-review gate (established process).
- The code-review gate backend was switched to codex (gpt-5.5) mid-final-task; it surfaced three real, sequential determinism/safety issues in the Task D scaffold path (HashMap ordering, write-lock, ambient-cwd control-plane) and a fourth pre-existing PID-safety gap — all fixed and regression-tested.
- `rules.toml` is now the sole validation source across both consumer repos; behavior preserved (this repo: re-init = empty diff; gf2: validate identical to baseline — 117 warnings + 1 pre-existing isolated-issue error).

### Deployment status

- `jit` and `jit-server` binaries reinstalled globally from slimmed main (commit 1e6f7ce). gf2 validates at baseline with the new binary.
- REMAINING (operational, user's call): the running gf2 server (:3000, PID 2788255) and this repo's server (:3001, PID 54723) are still on the pre-Task-D binary. They function correctly (the slimmed loader reads the existing rules.toml), but can be restarted at your convenience to run the slimmed code.
