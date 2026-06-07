# Plan — f6a704d0 scope expansion 2 (format-agnostic validation + BC hard removal + migrations + epic-gate findings)

**Epic:** f6a704d0 — Generic issue & label validation engine
**Status:** planning (awaiting plan-review green before dispatch)
**Date:** 2026-06-07
**Trigger:** epic-level code-review gate FAILED with 4 findings; user expanded scope
(interview 2026-06-07) to: fix the findings, make validation truly format-agnostic,
HARD-REMOVE the migration/backward-compat machinery, and migrate jit + gf2 as
deliverables. Pre-1.0, the ONLY repos are jit itself and ../gf2.

## 0. User-decided scope (interview)

- **#4 parser**: per-issue `content_format` field on the Issue; when absent, fall
  back to a per-repo config default; final fallback Markdown. Production validation
  must dispatch to the matching `ContentParser` so HTML/XML are exercised in
  production, not just tests.
- **BC removal**: FULL hard removal after migrating jit + gf2 — delete the
  config→rules migration, coexistence, materialize-existing, deprecated-key scan,
  and legacy `[validation]`/`[namespaces].{values,pattern,required}` parsing.
- **No-rules.toml behavior**: AUTO-SCAFFOLD the default `rules.toml` on first access
  (read-path mutation accepted), replacing the in-code fallback.
- **migration-code fate**: REMOVE entirely; `jit init` only scaffolds a fixed
  default `rules.toml` for new repos.
- **migrations**: perform this repo (already dogfooded) + gf2 as epic deliverables.
- **findings #1/#2/#3**: fix all (cheap).

## 1. Epic-gate findings (fix first; cheap, independent)

- **#1 schema-filename collision** — `serialize.rs:sanitize_rule_name` can map two
  distinct rule names to one `schemas/<name>.json`; `write_schema_files` overwrites
  without a collision check. Fix: detect collisions and either error
  (`SerializeError`) or generate guaranteed-unique filenames; add a test with two
  colliding rule names.
- **#2 `checker-command` + `enforce=true` silently ignored** — `local.rs:280-292`
  hardcodes `enforce:false` for checker-command. Per DR §4.3 checker-command is a
  validate/gate escape hatch, NOT a write-blocker. Fix: REJECT `enforce=true` on a
  `checker-command` rule at load time (`RuleConfigError::InvalidAssertion`,
  "checker-command rules cannot block writes; remove enforce"). Add loader test.
- **#3 PID-safety bug** — `gate_execution.rs:207-214` (pre-existing, from `dc647cb`,
  not this epic). Read the code, fix the unsafe PID handling (e.g. guard against
  PID reuse / missing-process before signaling). Add/adjust a test. NOTE: this is
  out-of-epic-origin but in-scope by user decision to get the epic gate green.

## 2. #4 — Format-agnostic production validation

### 2.1 Per-issue content_format field
- Add `content_format: Option<ContentFormat>` to the `Issue` domain type and its
  JSON storage (serde `#[serde(default, skip_serializing_if = "Option::is_none")]`
  so existing issue files without the field deserialize as `None` — NO mass rewrite
  of gf2's 894 / this repo's issue files needed; absent = inherit repo default).
  `ContentFormat` = enum `{ Markdown, Html, Xml }` (serde lowercase).
- Surface it on `jit issue create`/`update` (a `--content-format` flag, `--json`
  reflected) and in `jit --schema` (DR §9.3 MCP parity — a new authorable surface).
- **Plan-review Q-A**: should `content_format` instead be settable only via the
  field/edit, not a create flag? (Recommend: a create/update flag + schema, for MCP
  parity.)

### 2.2 Repo default + selection
- Add `content_format` (default `"markdown"`) to config.toml `[validation]` (a
  BEHAVIORAL key that SURVIVES the BC removal, like `default_type`).
- Selection at every projection-with-sections site: `issue.content_format`
  → else repo `content_format` → else Markdown. Centralize in ONE helper
  (e.g. `commands`-level `content_parser_for(issue) -> &dyn ContentParser`) and call
  it from `build_projection` (`local.rs`), `criterion_ids` (`graph.rs`), and any
  other `with_sections(...)` call sites (audit: `git grep with_sections`).
- **Feature gating**: HTML/XML parsers are behind cargo features. If an
  issue/repo selects a format whose parser feature is NOT compiled in, return a
  clear config error ("content_format=html requires the `html` feature"). The
  default build (Markdown) is always available. Add a `cargo-ci-features` gate run
  so the html/xml dispatch paths are exercised.
- Tests: end-to-end production validation under each format (markdown/html/xml),
  proving a rule that depends on parsed sections (e.g. require-section,
  label-coverage) fires correctly when the body is HTML/XML and the format is
  selected — i.e. the parsers are genuinely used in production.

### 2.3 Migrating existing issues
- Absent field = `None` = inherit repo default. No forced backfill. (The interview
  preview mentioned "default-fill"; using `Option` + repo-default avoids rewriting
  894 files. **Plan-review Q-B**: confirm Option-absent-inherits is acceptable vs an
  explicit backfill.)

## 3. Backward-compat HARD removal (after migrations — see §4 ordering)

Remove (delete code + tests for):
- `crates/jit/src/validation/migration.rs` ENTIRELY (config→rules migration,
  `MigrationState`, coexistence append, materialize-existing, schema-collision-skip,
  `strip_keys_from_config`, scaffold-from-config). Keep only what `jit init` needs
  to write a fixed default `rules.toml` (move that into a small `init`-scaffold
  helper, or `serialize.rs`).
- `config.rs` deprecated-key scan (`detect_deprecated_keys` / `warn_on_deprecated`).
- Legacy enforcement keys from `ValidationConfig` (`require_type_label`,
  `label_regex`, `reject_malformed_labels`, `enforce_namespace_registry`,
  `warn_orphaned_leaves`, `warn_strategic_consistency`) and the per-namespace
  `values`/`pattern`/`required` fields from `NamespaceConfig`. KEEP: `default_type`,
  the new `content_format`; KEEP namespace `description`/`unique`/`examples` and
  `[type_hierarchy]` (the label taxonomy + hierarchy that the default rules derive
  unique/registry/type-hierarchy rules from). **Plan-review Q-C**: keep or drop
  `strictness` (currently dead/inert)? Recommend drop.
- `default_ruleset(config, namespaces)` becomes the FIXED default: it no longer
  reads the removed `[validation]` flags. It still derives namespace-unique /
  namespace-registry / type-hierarchy rules from the RETAINED registry + hierarchy.
  Effectively `default_ruleset(namespaces)`.
- `effective_rules`: if `rules.toml` present (even empty) → sole source (unchanged);
  if ABSENT → AUTO-SCAFFOLD the default `rules.toml` to disk, then read it (replaces
  the in-code fallback + warning). Remove the bootstrap-fallback-without-write path.
- `jit init`: scaffold the fixed default `rules.toml` + schemas (no migration, no
  deprecated warning, no coexistence). Re-init idempotent (rules.toml present →
  no-op).

This REVERSES the migration machinery built in 0abaddc0 but KEEPS the durable
parts: file-as-source, the RuleSet→TOML serializer, the `type-hierarchy` TOML
assert kind, the removed `default:` reservation.

## 4. CRITICAL ordering (the migration must precede the removal)

1. **Wave X1 (parallel):** Task A (findings #1/#2/#3) + Task B (#4 content_format).
   Both land on main, gated.
2. **Wave X2:** Migrate gf2 (Task C) using the CURRENT migration code (still
   present at this point): install the new binary (built from main after X1), stop
   gf2 server, `git` snapshot gf2 `.jit`, `jit init` in gf2 → rules.toml + schemas +
   strip keys, verify `jit validate` unchanged, restart server, commit gf2. Confirm
   this repo is already migrated (re-verify). This is the LAST use of the migration
   code.
3. **Wave X3:** Task D (BC hard removal). Now that jit + gf2 both have `rules.toml`,
   delete the migration/legacy-config machinery. Rebuild; both repos read their
   `rules.toml`; `jit validate` still green on both. Reinstall the slimmed binary;
   gf2 + jit verified.
4. **Epic completion:** re-run epic cargo-ci + code-review gates (now the 4 findings
   are fixed and the format-agnostic + single-source contract holds); produce
   completion report; mark epic done; archive.

Dependencies: D depends on C (and A); C depends on A (correct serializer) and the
binary; B is independent and should land before C so gf2/jit get the final parser
behavior (though content_format doesn't change rules.toml content).

## 5. Epic success-criteria updates (Phase 0, lead)

- Criterion 5 → "Validation runs against the enriched projection using a content
  parser selected per-issue (`content_format`) with a per-repo default and Markdown
  fallback; HTML/XML parsers (behind features) are exercised in production, all
  behind one parser trait."
- Criterion 7/8 → reflect hard removal: "Legacy `[validation]`/namespace-constraint
  config and the migration machinery are REMOVED; `rules.toml` is the sole source,
  auto-scaffolded when absent; jit + gf2 are migrated."
- Add: "Epic-review correctness findings fixed: schema-name collision, checker-
  command enforce rejection, gate-execution PID safety."

## 6. DR updates (Phase 0, lead)

- §8.2/§8.4: replace the migration/coexistence/fallback model with: fixed default
  `rules.toml` scaffolded by `jit init`, auto-scaffold-on-absent, NO legacy config
  support (hard removal; only jit + gf2 existed pre-1.0 and both are migrated).
- §6/projection: document per-issue `content_format` + repo default + production
  parser dispatch + feature-gating error.
- Note the removal supersedes 0abaddc0's migration design (kept: file-as-source,
  serializer, type-hierarchy kind, reservation removal).

## 6a. Plan-review revisions (v2 — PLAN-NEEDS-REVISION resolved)

The plan reviewer confirmed all findings and returned 6 must-fixes; folded in here.

- **MF1 — exact post-removal `default_ruleset(namespaces)` contract.** After deleting
  the six `[validation]` flags + `NamespaceConfig.{values,pattern,required}`, the
  fixed default MUST emit exactly: (1) `default:label-format` error/enforce=true
  always; (2) `default:namespace-registry` error/**enforce=false** when registry
  non-empty; (3) `default:type-hierarchy-known` error/enforce=false always;
  (4) `default:namespace-unique:<ns>` error/enforce=true per unique namespace;
  (5) `default:orphan-leaf` + `default:strategic-consistency` **UNCONDITIONAL**
  warn/enforce=false (the warn_* flags defaulted true, so unconditional preserves
  behavior). DROPPED (no longer config-derivable): require-type-label,
  label-format-custom, namespace-values/pattern/required. Reviewer verified this
  reproduces THIS repo's committed 8-rule rules.toml byte-for-byte → re-init no-op.
- **MF2 — fresh-init richness decision: ACCEPT LEAN DEFAULT.** Deleting the namespace
  constraint fields means a fresh `jit init` no longer emits namespace
  values/pattern/required starter rules. We ACCEPT the leaner default (consistent
  with file-as-source: a repo that wants those constraints authors them in
  rules.toml). Update `intended_default_config`/template + its tests
  (`hierarchy_templates.rs:515-565`) to stop carrying/asserting the rich
  constraints. (Rationale: those are starter convenience, not backward-compat; the
  single-user reality makes leaner fresh-init fine.)
- **MF3 — blast radius add `config_manager.rs:206-208`** (copies NamespaceConfig
  values/pattern/required into the domain registry) — must be updated when those
  fields are removed. Also update docs `docs/reference/example-config.toml:98-112`,
  `configuration.md`, `labels.md`; and remove/adjust all full-`ValidationConfig`
  struct literals (defaults.rs tests, serialize.rs tests, commands/mod.rs
  effective_rules literal, hierarchy_templates `intended_default_config`).
- **MF4 — NO write-on-read auto-scaffold.** `effective_rules()` on an ABSENT
  rules.toml builds the default IN MEMORY (no disk write, no error) — preserving the
  read-path safety for gates/server/multi-worktree/read-only. Materialize to disk
  ONLY from `jit init` (under the existing write lock, temp+rename). This honors the
  user's "no error when missing; gets materialized" intent without read-path
  mutation. (Deviation from the literal "auto-scaffold on access" interview answer —
  flagged to user; functionally identical for jit + gf2 which already have the file.)
- **MF5 — CI features job is load-bearing.** `ci.yml` runs tests with DEFAULT
  features only; the cross-format test is `#![cfg(all(feature="html",feature="xml"))]`
  and is NEVER EXECUTED in CI today. Task B MUST add a `cargo test -p jit --features
  html,xml` job (and the issue carries the `cargo-ci-features` gate) so the production
  HTML/XML dispatch the user wants is actually exercised.
- **MF6 — #3 PID fix is reap-then-signal ordering.** The real bug: on timeout
  `wait_with_timeout` does `child.kill()` + `child.wait()` (reaps the leader,
  `gate_execution.rs:250-251`) BEFORE the caller sends `kill(-pgid, SIGKILL)`
  (`:213`); once reaped the PGID can be recycled → signals an unrelated group. Fix:
  signal the process GROUP before reaping the leader (and drop the redundant inner
  kill/wait, or reorder). Put the checker-command `enforce=true` rejection (#2) in
  the outer `into_rule` (rules.rs ~745-755) where `enforce` is in scope.

Suggestions adopted: KEEP `strictness` (inert, minimize churn); Task-D acceptance
check = `jit init` in this repo yields empty `git diff` on rules.toml; record the
deprecated-warning deletion in the DR; re-verify gf2's migrated rules.toml uses only
loader-supported kinds during Task C. Task B owns BOTH the local.rs AND graph.rs
parser threading + the `jit --schema` content_format parity test.

VERDICT after revisions: ready to dispatch (plan-review must-fixes incorporated).

## 7. Risks / open questions for the plan reviewer

- **Q-A**: `content_format` create/update flag + `--schema`/MCP parity, or
  field-only? (recommend flag + schema)
- **Q-B**: Option-absent-inherits-repo-default vs explicit backfill of 894 issues?
  (recommend Option-absent)
- **Q-C**: drop `strictness` (dead) during removal? (recommend drop)
- **Q-D**: after BC removal, `default_ruleset` still derives rules from the
  namespace registry + type_hierarchy in config.toml — is that consistent with
  "no config-derived rules", or should the registry/hierarchy ALSO move into
  rules.toml? (Recommend: registry/hierarchy are TAXONOMY, not enforcement config;
  they stay in config.toml and `jit init` serializes the derived rules once. Confirm.)
- **Q-E**: feature selected but not compiled → hard error vs warn+Markdown?
  (recommend hard error)
- **Q-F**: ordering safety — any reason gf2 migration can't precede removal cleanly?
  Worktree/disk pressure during multi-task waves (prune promptly).
- **Q-G**: does removing `migration.rs` break the dogfooded rules.toml already on
  main for this repo? (It should not — rules.toml is data; only the code that
  GENERATED it is removed. Confirm the serializer/scaffold path that init still
  needs is retained.)
