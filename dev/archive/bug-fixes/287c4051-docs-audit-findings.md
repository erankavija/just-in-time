# Documentation audit findings (2026-07-06)

Source-verified audit behind epic 287c4051 (Documentation accuracy and audience boundary
cleanup). Five parallel auditors swept CLAUDE.md, docs/, dev/, README, and TESTING.md
against `crates/jit` source; every finding below was independently re-verified by an
adversarial pass (46/46 confirmed). CLAUDE.md corrections landed directly ahead of the
epic; the findings here are the ones the epic's child issues carry.

## Per-document findings

### TESTING.md (task c29118f9)

- Paths reference the pre-workspace `cli/` crate root; modules described as flat files
  (`commands.rs`, `graph.rs`) are directories under `crates/jit/src/`.
- "Total: 123 tests (as of 2025-11-30)" and per-layer counts (31/14/78); `crates/jit/tests/`
  alone now holds ~126 test files.
- proptest listed under Future Improvements; property tests exist
  (`storage/claim_coordinator_proptests.rs`, `type_hierarchy.rs`, `graph/hierarchy.rs`).
- See Also links escape the repo (`../.github/...`, `../cli/...`); ROADMAP.md does not exist.
- TestHarness sketch is wrong: backing is `InMemoryStorage` (no temp dir), no `data_dir`
  method, field is `pub storage: InMemoryStorage`; `with_item_kinds` and
  `create_issue_with_desc` are undocumented. Truth: `crates/jit/tests/harness.rs`.

### docs/concepts/guarantees.md (task 313a7f3a)

- "What works without git" lists agent claiming/coordination; `jit claim` leases require
  git and fail with `ClaimRequiresGitError` (exit 10, `commands/claim.rs:32-51`). The
  doc's own multi-agent example runs `jit claim acquire`. Assignment (`jit issue claim`)
  is the part that works without git.

### docs/reference/storage-format.md (task d48906de)

- `claims.jsonl` placed in `.jit/`; lease control plane is `.git/jit/` (claims.index.json,
  claims.jsonl, heartbeat/, locks/ — `storage/worktree_paths.rs:52`).
- Directory diagram omits `rules.toml`, `templates.toml`, `invariants.toml`, `schemas/`.

### docs/reference/cli-commands.md (task a6e1ae3d)

- "Complete command reference" (docs/index.md:45, README.md:212) with empty stub sections:
  `## Document Commands` and `## Status and Validation` contain only HTML comments;
  README.md:191 links into the former.
- Missing families: `item`, `invariant`, `reference`, `apply`, `snapshot`, `hooks`,
  top-level `search`; no `issue create`/`update` sections.
- Line 1190: "so callers no longer recompute the filter client-side" (legacy narration).

### Legacy narration and registry drift (task 7eec3d59)

- labels.md:122: "The older `.jit/label-namespaces.json` file is no longer used" — the
  file is referenced nowhere else in the repo.
- labels.md:124 lists six pre-declared namespaces; the init scaffold declares seven
  (`enforces` missing, `hierarchy_templates.rs:132-165`).
- cli-command-grammar.md:122,147: "formerly bound", "the conformance work must" — the
  migration is complete in `cli.rs`; the doc narrates it as in flight.

### Audience-boundary leaks (task c7f0f690)

- `docs/9db27a3a-charter.md`: v1.0 milestone charter (self-doc) at docs/ root.
- `docs/reference/skill-eval-adjudication.md`, `docs/reference/lead-skills-eval-baseline.md`:
  eval records for this repo's `.claude/skills`.
- `docs/reference/rules-and-gates.md`: rendered region shows this repo's dogfood registries
  (cargo-ci, npm-ci, code-review) in the adopter tree without signalling it. The
  `[rules_gates_projection]` target points here — keep it valid.
- README.md:178: `.claude/skills` sync sentence (repo-local convention in the product README).

### Landing pages (task f7c50538)

- docs/README.md duplicates docs/index.md (quick links, structure, getting-started).
- Both omit `docs/examples/` (7 example rulesets that validation-rules.md links 18 times)
  and three reference docs (rules-and-gates.md, worktree-validate.md,
  lead-skills-eval-baseline.md).

### dev/ index and lifecycle (task 5514e4f5)

- dev/index.md:100,115 link nonexistent AGENTS.md.
- Directory Structure omits `design/`, `eval/`, `experiments/`, `plans/`, `presentations/`.
- Loose at dev/ root, outside the lifecycle: `docs-audit-plan.md`,
  `d0b85bff-archive-json-plan.md`, `f766b092-remove-printing-plan.md` (marked COMPLETE).

### mcp-server/README.md (task 287f7bc9)

- "60+ tools" hardcoded in four places; the generator currently emits 106 from the schema.

### Code hygiene (task f1aaaca7)

- `storage/json.rs:497` comment: "Config.toml is managed by humans, not auto-created" —
  `jit init` scaffolds it via `seed_project_config` (`main.rs:1578`).
- `.jit/tmp/` (created by `commands/document.rs:1638`) is the one runtime artifact not
  gitignored.

## Dogfooding inventory (context for boundary decisions)

Shipped by `jit init`: `index.json`, empty `gates.toml`, `events.jsonl`, template-generated
`config.toml` (4-type hierarchy, 7 namespaces incl. `enforces`, 6 item kinds), `rules.toml`
with 8 default rules, `.gitattributes`. Gate presets (`rust-tdd`, `minimal`, `python-tdd`,
`js-tdd`, `security-audit`, planning-bracket trio) live in code. Never scaffolded:
`templates.toml`, `invariants.toml`, `[invariant_projection]`, `[rules_gates_projection]`,
`[documentation]`.

This repo's dogfood layer: 13 gates wired to repo scripts (`cargo-ci`, `npm-ci`,
`jit-validate`, `repo-validate`, `code-review` via `./scripts/ai-review.sh`; preset-named
gates locally customized); `planning`/`breakdown`/`bug`/`enhancement` types;
`brackets:`/`satisfies:` namespaces (scaffold's `priority` namespace dropped); the `plan`
template; the `definition` item kind over the glossary; 8 invariants projected into
CLAUDE.md; the `dev/` doc lifecycle config.

Adopter docs must describe the shipped surface; the dogfood layer is signalled as this
repository's configuration wherever it appears in shipped text.
