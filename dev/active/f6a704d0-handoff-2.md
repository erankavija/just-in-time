# Handoff — Generic issue & label validation engine (f6a704d0) — session 2

**Date:** 2026-06-07
**Session number:** 2
**Prior handoffs:** `dev/active/f6a704d0-handoff.md` (session 1). Read it too; its Traps still apply.

## Current state

- Epic `f6a704d0`: state `in_progress` (claimed agent:project-lead). The ORIGINAL 12
  children are ALL DONE and gated. A user-directed **Scope Expansion 2** added 4 new
  children (below), so the epic is NOT complete.
- main HEAD: run `git log --oneline -1`. Working tree clean (`.claude/worktrees/` is
  gitignored).
- This repo was **dogfood-migrated** to `.jit/rules.toml` (8 rules) + `.jit/schemas/`;
  `[validation]` reduced to strictness+default_type. `jit validate` green.
- Epic-level gates: cargo-ci PASSED; code-review FAILED (4 findings → became the
  expansion). Those epic gate runs are now STALE; re-run at epic completion.

## What just happened (session 2)

- Completed wave 6: `a0f0f342` (defaults consolidation; user-approved custom-label_regex
  validate-path deviation, DR §8.3a) and `a6daa05d` (SDD examples, linked-scope) — both
  merged, gated green, done.
- Completed wave 7: `0abaddc0` — original partial migration FAILED DR §8.2/§8.3
  (orphan/strategic graph warnings had no rules.toml surface). User directed a
  scope-expansion (expansion 1): full single-source migration. Implemented over 2 rework
  rounds (D6 template + idempotency; then materialize-existing + schema-no-clobber),
  plan-reviewed. Added the `type-hierarchy` TOML assert kind, a RuleSet→TOML serializer,
  file-as-source `effective_rules`, removed the `default:` reservation. Dogfooded this
  repo. Merged, gated green, done.
- Ran epic-level gates → code-review FAILED with 4 findings (schema-name collision;
  checker-command enforce ignored; pre-existing gate PID-safety bug; production
  hardcodes MarkdownContentParser). 
- User EXPANDED SCOPE AGAIN (expansion 2, via interview): fix the 4 findings; make
  validation truly format-agnostic; HARD-REMOVE the migration/backward-compat machinery;
  migrate jit + gf2. Created plan `dev/active/f6a704d0-expansion2-plan.md` (plan-reviewed,
  revised to v2 with 6 must-fixes incorporated — see §6a of that plan). Created 4 child
  tasks (below) + wired deps. Amended epic success criteria (see epic description "Scope
  Expansion 2").
- Analyzed gf2 migration (see below).

## CONTINUATION UPDATE (later in session 2)

Expansion-2 progress since the above was written:
- **Task A (`5e79ba48`) — DONE + gated.** schema-name collision guard, checker-command
  enforce rejected at load, gate PID-safety (reap-then-signal + a `kill(-1)`/wrap guard
  the code-review gate additionally required). Merged to main.
- **Task B (`e93af54b`) — DONE + gated** (cargo-ci + cargo-ci-features + code-review).
  per-issue `content_format` + repo default + production parser dispatch (both local +
  graph sites) + feature-not-compiled error + CI `--features html,xml` job. Code-review
  also required two follow-ups (now fixed): `jit issue show --json` exposes
  content_format; `--content-format inherit`/`default` clears an override back to None
  (update is tri-state `Option<Option<ContentFormat>>`).
- **Task C (`2fb9f910`) — DONE.** gf2 migrated to rules.toml: installed the new `jit`
  AND `jit-server` globally (`cargo install --path crates/jit` + `crates/server`, from
  main which has A+B but NOT D), stopped gf2's old server (PID 2406670), ran `jit init`
  in `../gf2` (migrated `require_type_label`; 8 rules + 3 schemas; config reduced to
  behavioral keys), verified `jit validate` IDENTICAL to baseline (exit 1 on gf2's
  PRE-EXISTING isolated-issue integrity error + 117 warnings — not introduced by us),
  committed in the gf2 repo (`49d2c38`), restarted `jit-server` on :3000 (healthy, HTTP
  200; pid file is gitignored, updated to the live PID). This repo also re-verified green
  (exit 0) under the new global binary.
- **REMAINING: Task D (`d4188154`, BC hard removal — UNBLOCKED, 1/1 dep done), then epic
  completion.**
- **BINARY-INSTALL NOTE:** the global `jit`/`jit-server` are from main@(A+B), which still
  contains the migration/BC code. After Task D lands (removes that code), RE-INSTALL both
  (`cargo install --path crates/jit --force` + `--path crates/server --force`) and restart
  gf2's server so the deployed binaries match the slimmed code. gf2's already-migrated
  rules.toml stays valid (post-D loader still reads all rule kinds; re-init = no-op).
- **Rate-limit note (user, this session):** the `code-review` gate's AI reviewer is near
  its rate limit; `cargo-ci`/`cargo-ci-features` are LOCAL (free) — only `code-review`
  consumes the budget. Remaining code-review runs needed: Task D + epic completion (≈2),
  plus any rework. If a `code-review` run returns a rate-limit error, write a handoff and
  stop. Task C has NO code-review gate (operational), so it spends none.
- main HEAD after A+B: run `git log --oneline -1` (the B-fix merge + commits).

## What to do next — execute Scope Expansion 2 (follow the plan)

THE SPEC is `dev/active/f6a704d0-expansion2-plan.md` (read §0 scope, §1-§4, §6a v2
must-fixes). 4 child tasks (get ids: `jit graph deps f6a704d0`):

- **Task A — epic-review findings** (`5e79ba48`): #1 schema-name collision guard in
  `serialize.rs`; #2 reject `checker-command`+`enforce=true` at load (in `into_rule`,
  rules.rs ~745-755; DR §4.3 = escape hatch, not write-blocker); #3 PID-safety
  reap-then-signal fix in `gate_execution.rs:207-251` (signal PGID BEFORE `child.wait()`
  reaps the leader). Gates cargo-ci+code-review. Independent — dispatch first.
- **Task B — format-agnostic** (`e93af54b`): per-issue `content_format: Option<ContentFormat>`
  on Issue (serde default+skip; NO mass rewrite), repo default `content_format` in
  config.toml, centralized `content_parser_for(issue)` used at BOTH `local.rs:337`
  (build_projection) AND `graph.rs:496` (criterion_ids); feature-selected-but-not-compiled
  = hard error; `--content-format` create/update flag + `jit --schema`/MCP parity test;
  ADD a CI `cargo test -p jit --features html,xml` job (the cross-format test is
  `#![cfg(all(feature=html,xml))]` and is NEVER run in CI today). Gates
  cargo-ci+cargo-ci-features+code-review. Independent of A.
- **Task C — migrate gf2 + verify this repo** (`2fb9f910`): depends on A,B. OPERATIONAL
  (lead): `cargo install --path crates/jit`; stop gf2's running server; `git` snapshot
  gf2 `.jit`; `jit init` in ../gf2 (→ rules.toml+schemas, strips `require_type_label`);
  verify `jit validate` unchanged; restart server; commit gf2. This is the LAST use of
  the migration code (Task D deletes it). Re-verify this repo still green.
- **Task D — BC hard removal** (`d4188154`): depends on C. Delete `migration.rs`,
  coexistence/materialize/deprecated-scan, legacy `[validation]` flags +
  `NamespaceConfig.{values,pattern,required}` (also `config_manager.rs:206-208`),
  config-derived rule gating; `default_ruleset(namespaces)` = the FIXED 5-kind default
  (see plan §6a MF1: label-format e/true, namespace-registry e/false, type-hierarchy-known
  e/false, namespace-unique:<ns> e/true, orphan-leaf+strategic-consistency UNCONDITIONAL
  warn). `effective_rules` absent→in-memory default (NO write-on-read); init materializes.
  ACCEPT lean fresh-init default (MF2 — update intended_default_config + tests). Update DR
  §8.2/§8.4 + docs/reference/example-config.toml + configuration.md/labels.md. Acceptance:
  `jit init` in this repo → empty `git diff` on rules.toml. Gates cargo-ci+code-review.
- **Epic completion (Section 10):** re-run epic cargo-ci+code-review; completion report;
  mark epic done; archive both progress files + handoffs.

PROCESS (per established bar, memory `feedback-adversarial-review-before-gate`): for each
code task — dispatch worker in a fresh worktree anchored to current main (use
`scripts/dispatch-worker-worktree.sh <short-id>`), then run an ADVERSARIAL reviewer BEFORE
the code-review gate, fix findings, merge, run gates (verify recorded status, not exit
code), prune the worktree promptly (disk runs ~93% full).

## gf2 migration analysis (for Task C)

gf2 (`../gf2`): no rules.toml/schemas yet; active `[validation]` = strictness,
default_type, `require_type_label=false` (rest commented out); namespaces type/resolution/
ppc-kernel (unique), component (non-unique); NO value/pattern/required constraints; 894
issues; a RUNNING jit server; uses the OLD installed binary (commit d5ff3c4c, pre-epic).
Simulated `jit init` outcome: migrates 1 key (require_type_label), writes 8-rule rules.toml
(label-format, namespace-registry, type-hierarchy-known, namespace-unique:{ppc-kernel,
resolution,type}, orphan-leaf, strategic-consistency) + 3 schemas, strips the key; behavior
preserved (loose/warn-only). Low risk; issue data untouched. Must restart the server with
the new binary.

## Traps — do not repeat these

- ALL session-1 traps still apply (see `dev/active/f6a704d0-handoff.md` Traps): `jit gate
  pass` exits 0 even on FAIL verdict (read recorded status via `jit gate check <id> <gate>
  --json` .status, and `.jit/gate-runs/<run_id>/result.json` .stdout); code-review verdict
  parser reads the LAST non-blank line; confirm a FRESH run_id (stale runs read as passed);
  prune worktrees promptly (shared disk ~93%); validator cache keying is settled
  (schema-identity); SDD examples live under docs/examples, not live .jit.
- **`jit issue create` uses `-t/--title` (NOT positional) and has NO `--type` flag — set
  type via `--label type:task`.** Wasted several calls here.
- **DISK: a `cargo test` that builds an inner binary fails with "Disk quota exceeded" when
  the disk is full** — the version_cli_tests build a jit binary in a temp dir. This masked
  as a cargo-ci FAIL once this session (exit 101); freeing ~14GB (pruning merged worktrees
  + `rm -rf /tmp/claude-1000/.tmp*`) fixed it. Always prune merged worktrees + branches
  immediately after merge.
- **Do NOT `git add -A` when a worktree dir lives under the repo root** — it stages the
  worktree as an embedded gitlink. `.claude/worktrees/` is now gitignored (commit on main);
  keep it so.
- **Do NOT run `jit init`/any writing command against the LIVE repo `.jit`** during
  development — workers test in `mktemp -d` only. (Task C's gf2 migration is the deliberate
  exception, done by the lead with a git snapshot first.)
- **Auto-scaffold-on-read was REJECTED by plan review** (concurrency/RO/multi-worktree;
  gf2 has a running server). The user's interview answer said "auto-scaffold on access" but
  the SAFE implementation (plan §6a MF4) is: read path builds default IN MEMORY (no write);
  only `jit init`/write materializes. Flagged to the user; proceed with the safe approach
  unless they object.
- **BC removal ordering is load-bearing:** migrate gf2 (Task C) BEFORE deleting the
  migration code (Task D). The slimmed loader still parses label-value-pattern + raw
  json-schema rules, so gf2's pre-removal rules.toml stays readable.
- **Removing struct fields will break full-`ValidationConfig` struct literals** scattered in
  tests + commands/mod.rs effective_rules + hierarchy_templates `intended_default_config`;
  plan §6a MF3 lists the sites.
- **`backward-compat / migration edge cases are LARGELY WASTED EFFORT`** — only jit + gf2
  exist pre-1.0 (user feedback; memory `feedback-jit-single-user-no-backward-compat`). This
  is WHY expansion 2 hard-removes the machinery. Don't re-add backward-compat gold-plating.

## Open questions needing user input

- None blocking. One flagged DEVIATION the user may override: auto-scaffold-on-read →
  changed to in-memory-default-on-read + materialize-on-init (plan §6a MF4). Proceed unless
  the user objects.

## Reference artefacts

- Plans: `dev/active/f6a704d0-expansion2-plan.md` (current, v2), `…-0abaddc0-fullmigration-plan.md`,
  `…-validation-engine-plan.md`.
- Decision record: `dev/active/f6a704d0-validation-engine.md` (§8.2/8.2a/8.3/8.3a/8.4 amended).
- Progress: `dev/active/f6a704d0-progress.json`.
- Tasks: A `5e79ba48`, B `e93af54b`, C `2fb9f910`, D `d4188154`.
- Dispatch/leak scripts: `~/.claude/skills/project-lead/scripts/`.
