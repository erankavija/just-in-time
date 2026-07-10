# Investigation — Exhaustive documentation audit and drift removal (2d109173)

**Purpose:** ground-truth verification for the planning node (P) of this epic's `plan` bracket. Read-only; no `.jit/` state changed except a `cargo install --path crates/jit` reinstall (see Note on tooling below). All findings below are `file:line` evidence or a command transcript.

**Method note:** the installed `jit` binary was 53 commits behind `HEAD` (`0f01959d` vs `844e959f` at investigation start). `crates/jit/src/cli.rs` had zero commits in that range, so the stale binary's `--schema`/`--help` output was already correct, but it was reinstalled from `HEAD` before any command below was trusted (`cargo install --path crates/jit`, confirmed `jit --version` now reports `844e959f`).

---

## 1. In-scope surface inventory

| Area | Files | Approx. lines |
|---|---|---|
| (1) `docs/concepts/` | 9 `.md` | 3,339 |
| (2) `docs/reference/` | 11 `.md` + `example-config.toml` | 7,079 (of which `cli-commands.md` alone is 3,819) |
| (3) `docs/how-to/` + `docs/tutorials/` | 10 + 4 `.md` | 4,452 + 867 = 5,319 |
| (4) `docs/examples/` | 7 rulesets, 14 files total (mostly TOML/JSON, no prose) | 1,173 |
| (5) root + component READMEs | `README.md`, `INSTALL.md`, `TESTING.md`, `mcp-server/README.md`, `web/README.md` | 1,353 |

Per-file breakdown (line counts) verified via `wc -l`:
- concepts: `hierarchy-resolution.md` 102, `containment-and-completion.md` 128, `validation-engine.md` 150, `overview.md` 156, `scope.md` 257, `planning-bracket.md` 271, `design-philosophy.md` 489, `guarantees.md` 589, `core-model.md` 1197.
- reference: `rules-and-gates.md` 51, `glossary.md` 70, `item-addresses.md` 178, `example-config.toml` 181, `jit-content-standards.md` 183, `cli-command-grammar.md` 211, `storage-format.md` 306, `worktree-validate.md` 329, `claim.md` 450, `configuration.md` 504, `labels.md` 797, `cli-commands.md` 3819.
- how-to: `research-projects.md` 149, `deployment.md` 174, `knowledge-work.md` 208, `adopt-planning-bracket.md` 329, `validation-rules.md` 416, `multi-agent-coordination.md` 455, `troubleshooting.md` 489, `dependency-management.md` 629, `software-development.md` 634, `custom-gates.md` 969.
- tutorials: `README.md` 88, `quickstart.md` 177, `parallel-work-worktrees.md` 250, `first-workflow.md` 352.
- root/component: `web/README.md` 54, `README.md` 244, `TESTING.md` 336, `INSTALL.md` 351, `mcp-server/README.md` 368.

**Examples — "all seven rulesets" claim: confirmed, count = 7.** `docs/examples/{bug-repro,cross-epic,fresh-evidence,nyquist,release-checklist,research,sdd}/`. All seven are loaded and exercised by `crates/jit/tests/example_rulesets_tests.rs:102-108` (`"sdd","bug-repro","release-checklist","fresh-evidence","nyquist","cross-epic","research"`), so their TOML correctness is already mechanically checked by cargo tests — the docs-epic's job is narrative/reference accuracy, not schema validity.

**Triage signal for examples (reference-grade vs illustrative), file-footprint facts only (no verdict):**
- `sdd/` and `research/` each carry 4 files (`config.toml`, `rules.toml`, `schemas/*.json`, `templates.toml`) — the two "full-stack" examples composing multiple registry types.
- `bug-repro/` carries 2 files (`rules.toml` + `schemas/bug-body.json`).
- `cross-epic/`, `fresh-evidence/`, `nyquist/`, `release-checklist/` are single-file (`rules.toml` only).
- Referenced-in-prose counts (grep hits of the directory name across `docs/`, `README.md`, `INSTALL.md`): `research` 3, `sdd` 3, `bug-repro` 1, `cross-epic` 2, `fresh-evidence` 1, `nyquist` 1, `release-checklist` 1.

**docs/ files outside the five named areas:**
- `docs/index.md` (5,312 bytes) — the consolidated landing page (per 287c4051 REQ-02, `f7c50538`).
- `docs/README.md` (160 bytes, 4 lines) — a GitHub-directory-README stub that only points to `docs/index.md` (`docs/README.md:3`). Zero inbound links to it (expected for a directory-README convention); not a merge candidate.

Neither is assigned to one of the five areas by the brief. The plan should either fold them into area (5) (root + component READMEs) or explicitly name a sixth bucket — they are load-bearing (docs/index.md is the landing page) and currently un-owned by the decomposition.

**Also outside scope but present:** `CHANGELOG.md` at root (not in the five areas — brief and `doc-review-prompt.md:23` both explicitly exclude it) and `web/TESTING.md` (exists, not named in the brief's in-scope list, which only names `web/README.md`).

---

## 2. The seven drift classes — truth verification + spot-check for live instances

Verified against `HEAD` (844e959f) source. Classification: **truth confirmed** (what the brief states as ground truth checks out against source) × **drift status** (still present in a spot-check, or absent/fixed).

### Class 1 — `.jit/` described as all state
**Truth confirmed.** Three locations verified: `.jit/` versioned data + gitignored runtime files (`crates/jit/src/storage/json.rs` write paths; `.jit/`'s gitignored files listed at `docs/reference/storage-format.md:35-37`: `worktree.json`, `server.log`, `server.pid.json`, `*.lock`, `tmp/`), and `.git/jit/` shared control plane (`crates/jit/src/storage/claim_coordinator.rs:206,377-378,400,656,703` — `claims.index.json`, `claims.jsonl`, `heartbeat/`, `locks/claims.lock`).

**Drift status: fixed in the three files 287c4051 explicitly touched** (`docs/concepts/scope.md:192-193`, `docs/concepts/overview.md:57-59`, `docs/reference/storage-format.md:5-8,35-40,239-256` all state the three-location split correctly).

**Drift status: still live elsewhere.** `docs/concepts/guarantees.md:85-88` ("Multi-agent safety" bullet list) names lock files that don't match source:
- `.index.lock` (guarantees.md:85) — correct, matches `crates/jit/src/storage/json.rs:189`.
- `.issues/{id}.lock` (guarantees.md:86) — wrong: the actual directory is `issues/` (no leading dot); the file itself is right (`with_extension("lock")` on the issue path, `json.rs:190`).
- `claims.index.lock` (guarantees.md:87, and repeated at guarantees.md:95,97 inside a Mermaid diagram) — **wrong lock file**. The actual claim lock is `locks/claims.lock` under `.git/jit/`, not `.jit/claims.index.lock` (`crates/jit/src/storage/claim_coordinator.rs:400,498,604,790,861,916,966`). `claims.index.json` is a real file but it is data, not the lock.
- The bullet list omits the repository-level lock (`.repo-write.lock`, `crates/jit/src/storage/repo_lock.rs:43`) entirely, even though the same file's later section (`guarantees.md:355-366`) correctly documents the two-directory split with accurate filenames. The two sections of the same doc disagree with each other.

### Class 2 — atomic `rename()` over-credited
**Truth confirmed.** Lock order is repository → index → issue (`crates/jit/src/storage/json.rs:182,192,679`: "Lock order: repository write lock first, then index, then issue"); `rename()` supplies per-write atomicity, not mutual exclusion.

**Drift status: fixed.** `docs/concepts/core-model.md:1040-1051` states this correctly ("Advisory locks serialize the writes; the POSIX `rename()` makes each write atomic... Because the locks serialize step 2 against step 4..."). No instance found restating the old over-credited framing.

### Class 3 — storage-format specifics
**Truth confirmed on all four sub-facts:**
- ID scheme is UUID v4, not ULID (`crates/jit/src/domain/types.rs:552` and 15+ other `Uuid::new_v4()` call sites; zero `ulid`/`Ulid` hits anywhere in `crates/jit/src`).
- State enum is 7 values including `Gated`/`Archived` (`crates/jit/src/domain/types.rs:28-42`).
- Events use `#[serde(tag = "type", rename_all = "snake_case")]` flat objects (`crates/jit/src/domain/types.rs:1307-1308`), variants `IssueCreated, IssueClaimed, IssueStateChanged, GatePassed, GateFailed, GateAdded, GateRemoved, IssueCompleted, IssueDeleted, IssueReleased` (types.rs:1310-1413), tag strings `issue_created`, `issue_claimed`, `issue_state_changed`, etc. (types.rs:2010-2016).
- Gate runs live at `gate-runs/<run_id>/result.json` (`crates/jit/src/storage/json.rs:38-39,157`).

**Drift status: known symptom-strings absent** (spot-checked `ULID`, `event_type` literal — zero hits in the in-scope surface).

**Drift status: a new, undocumented-by-the-brief instance of the same class is live**, and it is a self-contradiction inside the repo (the correct fact is documented in one file, wrong in two others):
- `docs/reference/storage-format.md:162-166` ("Event Types") states the tag values correctly: `issue_created`, `issue_claimed`, ..., `issue_state_changed` with example `{"type":"issue_state_changed",...,"from":"ready","to":"in_progress"}` (storage-format.md:158).
- `docs/concepts/guarantees.md:170` shows `jit events query --event-type IssueUpdated` — `IssueUpdated` is not a variant name and not a tag string; it would silently match zero events.
- `docs/reference/cli-commands.md:3756-3757` shows `jit events query --event-type state_changed --limit 100 --json | jq -r '[.[] | select(.new_state == "done")] | length'` — wrong tag value (`state_changed`, missing the `issue_` prefix) **and** wrong field name (`new_state` should be `to`, confirmed at `crates/jit/src/domain/types.rs:1343-1344`). This script would run without error and always report 0.

### Class 4 — future/roadmap narration stated as behavior
**Drift status: known symptom-strings absent.** Zero hits for `before 1.0`, `not yet implemented`, `roadmap`, `coming soon`, `planned feature`, `undo`/`replay`/`time-travel` across the full in-scope surface. Consistent with 287c4051's completion report (`guarantees.md` undo/replay/time-travel removed, npm 404 and migration-promise fixed).

### Class 5 — legacy narration of the current surface
**Drift status: known symptom-strings absent.** Zero hits for `legacy alias`, `were removed`, `in-flight`, `replacement for the old` in the in-scope surface (the few `no longer` hits found — `core-model.md:798`, `cli-commands.md:1560` — are label-value descriptions like `resolution:obsolete — No longer relevant`, not narration about jit's own removed features).

### Class 6 — nonexistent CLI surface
**Drift status: known symptom-strings absent.** Zero hits for `--replace-label`, `label suggest`, `label schema`, `label audit`, `label fix`. `docs/reference/labels.md` now documents the real `jit label` family (`namespaces`, `values`, `add`, `remove`/`rm`) confirmed against `jit --schema`'s `label` subcommand list.

### Class 7 — prerequisite/environment drift
**Truth:** CI pins Node 20 (`.github/workflows/ci.yml:204,240`, `security-audit.yml:47`, `release.yml:79,107`); no `engines` field constrains a floor in `mcp-server/package.json`/`web/package.json`.

**Drift status: live and internally inconsistent.** `INSTALL.md` states two different Node floors in the same file: `INSTALL.md:160` — "Node.js 20+ (for MCP server and Web UI)" vs. `INSTALL.md:261` — "**Node.js** (v18+): Required for the MCP server..."; `mcp-server/README.md:324` also says "Should be v18 or later". None of the three cites a source; CI truth is 20.

**Drift status: fixed.** The Docker quick-start already documents the data-volume-init prerequisite (`INSTALL.md:70-73`: "Initialize the shared data volume first — the API server refuses to start against an uninitialized directory..."), matching `crates/server/src/main.rs:69`. Git-as-optional framing is also already correct (`INSTALL.md:259`: "Core issue tracking is Git-optional, but advisory leases... need a Git repository").

### Summary table

| Class | Truth confirmed | Known-symptom drift still present in spot-check |
|---|---|---|
| 1. `.jit/` as all state | yes | yes — `guarantees.md:85-97` (wrong claim-lock filename, internally inconsistent with its own later section) |
| 2. rename() over-credited | yes | no (spot-checked instance fixed) |
| 3. storage-format specifics | yes | yes — new instance: `guarantees.md:170`, `cli-commands.md:3756-3757` (bad `--event-type` values + wrong field name) |
| 4. future/roadmap narration | yes | no (symptom strings absent) |
| 5. legacy narration | yes | no (symptom strings absent) |
| 6. nonexistent CLI surface | yes | no (symptom strings absent) |
| 7. prerequisite/environment drift | yes | yes — `INSTALL.md:160` vs `INSTALL.md:261` vs `mcp-server/README.md:324` (18 vs 20) |

This is a spot-check, not the exhaustive sweep (that is the epic's job per the brief). It shows: (a) all seven classes' underlying "truth" statements in the brief are accurate against current source; (b) the specific symptom instances 287c4051 is known to have fixed stay fixed; (c) new, previously-uncatalogued instances of classes 1, 3, and 7 are live, meaning the exhaustive pass has real work to find even beyond a re-check of the known symptom list.

---

## 3. REQ-05 grounding — top-level command family doc-home gap list

`jit --schema` (rebuilt binary, `commit 844e959f`) lists **27 top-level command families**: `apply, claim, config, dep, doc, events, gate, graph, hooks, init, invariant, issue, item, label, list, migrate, query, rdeps, recover, reference, search, serve, snapshot, status, validate, version, worktree`.

**Important schema-reading caveat:** `jit --schema`'s `hidden` field (e.g. `"serve": {"hidden": true, ...}` in the raw JSON) does **not** mean "absent from the schema" or "not a real command." Per `crates/jit/src/schema.rs:144-146`: "Commands hidden from the default MCP tool listing. These are still present in the schema and executable, but the MCP server filters them..." `serve` and `init` are both marked `hidden` for MCP-tool-list purposes only (`schema.rs:147-189`, `hidden_commands()` set includes `"init"` and `"serve"` alongside `claim_*`, `config_*`, etc.) — `init` obviously has a doc home (`docs/reference/cli-commands.md:923`), confirming `hidden` is irrelevant to REQ-05's "present in `jit --schema`" test. All 27 top-level keys count.

**Cross-check against the in-scope surface, by heading/dedicated-section in `docs/reference/cli-commands.md` plus dedicated files:**

| Family | Doc home | Evidence |
|---|---|---|
| apply | yes | `cli-commands.md:3141` "## Template Commands" / `### jit apply` |
| claim | yes | `docs/reference/claim.md` (dedicated file) |
| config | yes | `cli-commands.md:3436` "## Configuration" |
| dep | yes | `cli-commands.md:2480` "## Dependency Commands" |
| doc | yes | `cli-commands.md:2710` "## Document Commands" |
| events | **partial** | conceptual/format coverage only: `storage-format.md:148` "## Event Log Format", `storage-format.md:162` "### Event Types", `guarantees.md:117` "### Event Logging" — but no CLI reference section for `jit events query`/`tail` flags anywhere in scope; `grep -n "jit events tail" docs/reference/cli-commands.md` returns zero hits |
| gate | yes | `cli-commands.md:1593` "## Gate Commands", `:2184` "## Gate Preset Commands" |
| graph | yes | `cli-commands.md:2831` "## Graph Commands" |
| hooks | yes | `cli-commands.md:3384` "## Git Hook Commands" |
| init | yes | `cli-commands.md:923` "### jit init" |
| invariant | yes | `cli-commands.md:3253` "## Registry Projection Commands" |
| issue | yes | `cli-commands.md:1022` "## Issue Commands" |
| item | yes | `cli-commands.md:3185` "## Item Commands" |
| label | yes | `docs/reference/labels.md` (dedicated file) |
| list | yes | alias table, `cli-commands.md:100,115` |
| migrate | yes | `cli-commands.md:3408` "## Maintenance Commands" |
| query | yes | `cli-commands.md:2576` "## Query Commands" |
| rdeps | yes | alias table, `cli-commands.md:101,116`, and `:2890` |
| recover | yes | `cli-commands.md:3127` "### jit recover" |
| reference | yes | `cli-commands.md:3282` "### jit reference render" |
| search | yes | `cli-commands.md:3296` "## Repository Search" |
| **serve** | **no — gap** | zero hits for `jit serve` (the subcommand) anywhere in scope. All "serve"-adjacent hits are about the separate `jit-server`/`jit-mcp-server` **binaries** (`INSTALL.md`, `README.md`, `docs/how-to/deployment.md`, `web/README.md`) — a distinct surface from `jit serve`, the CLI subcommand that daemonizes `jit-server` (`crates/jit/src/cli.rs:427-452`; `jit serve --help` confirms flags `--port`, `--stop`, `--status`, `--fg`, `--log`, `--web-dir`, `--json`, none of which are documented anywhere in scope) |
| snapshot | yes | `cli-commands.md:3351` "## Snapshot Commands" |
| status | yes | `cli-commands.md:3029` "### jit status" |
| validate | yes | `cli-commands.md:3069` "### jit validate" |
| version | yes | `cli-commands.md:977` "## Version and Provenance" |
| worktree | yes | `docs/reference/worktree-validate.md` (dedicated file, covers `jit worktree` + `jit validate` together) |

**REQ-05 gap list: 1 hard gap (`serve`), 1 partial gap (`events`, has concept coverage but no CLI reference entry).** Whether `events`' concept-only coverage satisfies "an adopter-facing documentation home" is a plan-level call; I present the fact, not a verdict, since REQ-05's text says "documentation home," not specifically "CLI reference entry."

---

## 4. REQ-04 grounding — diagram inventory

**Existing Mermaid blocks: 29** (` ```mermaid ` fences), distributed: concepts 9 files→5 have blocks (`core-model.md` 9, `planning-bracket.md` 3, `containment-and-completion.md` 2, `design-philosophy.md` 2, `guarantees.md` 1), reference 2 (`jit-content-standards.md` 1, `labels.md` 1), how-to/tutorials 6 (`dependency-management.md` 5, `parallel-work-worktrees.md` 1), root/component 4 (one each in `README.md`, `TESTING.md`, `mcp-server/README.md`, `web/README.md`).

**Box-drawing characters (`─│┌┐└┘├┤┬┴┼`) outside Mermaid fences: 6 files**, all confirmed to be **directory/file-tree listings**, not diagrams: `TESTING.md:34-40`, `docs/concepts/overview.md:92-100`, `docs/concepts/guarantees.md:357-366`, `mcp-server/README.md:143-150`, `docs/reference/storage-format.md:14-27,251-256`. One exception is `docs/how-to/dependency-management.md:139-140,419`, which uses `├─`/`└─` inside a comment block presenting **verbatim sample CLI output** (a `jit graph deps` tree render), not a hand-authored diagram.

This matches the existing gate's own carve-out at `scripts/doc-review-prompt.md:65`: "Directory and file-tree listings and verbatim CLI output stay plain text and are not diagrams — do not flag them." No box-drawing+arrow combination (the `──dep→` pattern that was the actual 287c4051 REQ-05 bug, per its completion report) was found anywhere in the current in-scope surface, including `docs/examples/` — that specific defect class is fixed and absent.

**Inline unicode arrows (`→` and similar) outside Mermaid fences: 224 occurrences across 23 files.** Sampled five files (`scope.md`, `core-model.md`, `cli-commands.md`, `dependency-management.md`, `quickstart.md`) — every occurrence found is a **single arrow used as prose shorthand** within one line (e.g. `scope.md:18` "Plan → Implement → Test → Review → Deploy"; `quickstart.md:18` "states: backlog → ready → in_progress → done"; `core-model.md:316-330` "→ A stays blocked..."), not a multi-line ASCII-art spine.

**Open question for the plan, not resolved here:** does REQ-04's "no ASCII or arrow-art diagrams, including unicode-arrow spines" reach these 224 single-line prose arrows, or only multi-line/structural arrow diagrams? The existing `doc-review` gate prompt (`scripts/doc-review-prompt.md:65`) currently only instructs the reviewer to flag box-drawing characters forming boxes/arrows — it does **not** currently flag bare inline `→` prose usage. If the epic intends the stricter reading, ~224 sites across 23 files need a decision (rewrite to words, or accept as non-diagram prose) and the gate prompt itself would need updating to enforce it consistently, which is new scope beyond the current gate contract.

---

## 5. REQ-06 grounding — projection/citation surface matrix

Confirmed live and resolvable:
- `jit invariant render` — projects the invariant registry into `[invariant_projection]`'s configured target. `jit config get invariant_projection --json` → `{"mode":"region","style":"id-anchor","target":"CLAUDE.md"}` (region markers documented at `.jit/config.toml:211-215`).
- `jit reference render` — projects rule + gate registries into `[rules_gates_projection]`'s target. `jit config get rules_gates_projection --json` → `{"mode":"region","style":"full","target":"docs/reference/rules-and-gates.md"}` (`.jit/config.toml:221-226`).
- `jit item list --kind invariant|rule|gate` — 9 invariants, 9 rules, 15 gates registered and resolvable (command output captured in full during this investigation).
- `jit item show @/inv/single-source-prose` resolves: qualified id `@/invariant/single-source-prose`, text "Every fact with a single source of truth reaches prose by projection or citation; volatile facts (counts, enumerations, registry contents) are stated structurally or derived, and a hand-maintained copy is a staleness defect."
- `jit --schema` — full command/flag/type/exit-code shape, confirmed working (27 top-level families, `types` map with `Issue`, `State`, `Priority`, `ErrorResponse`).

**Coverage by REQ-06's named volatile-fact categories:**

| Category | Projection/citation surface exists? |
|---|---|
| Invariant text | **Yes** — `jit invariant render` → CLAUDE.md (region-projected, live) |
| Rule/gate text | **Yes** — `jit reference render` → `docs/reference/rules-and-gates.md` (region-projected, live) |
| CLI shapes (commands/flags) | **Partial** — `jit --schema` is a live, complete citation source, but there is no *projector* that writes schema content into docs automatically; docs must be manually kept in sync and mechanically diffed against the schema (see §6 — no such diff script exists yet) |
| Enums (e.g. `State`, `Priority`) | **Partial, and the source itself has a bug.** `jit --schema`'s `types.State.enum` (`crates/jit/src/schema.rs:463-469`, hardcoded list) is `["backlog","ready","in_progress","gated","done","archived"]` — **missing `"rejected"`**, even though the real `State` enum has 7 values including `Rejected` (`crates/jit/src/domain/types.rs:28-42`). Any doc/citation built by trusting `jit --schema`'s `types` map for the State enum would silently omit a value. This is a `schema.rs` bug, out of the docs epic's scope to fix, but material to how REQ-06 citations against `--schema` should be verified (cross-check against `domain/types.rs` directly for this one type, not just `--schema`) — a follow-up candidate, matching the brief's own pattern of filing out-of-scope engine bugs surfaced during this epic (cf. brief's `--description` help-string note). |
| Storage specifics (ID scheme, event shape, gate-runs layout) | **None.** No `jit`-side projection or citation surface exists for these facts — they live only in Rust source (doc comments, struct/enum definitions). Per the brief's own constraint, these become cite-source-file (e.g. cite `crates/jit/src/domain/types.rs:28` for the State enum) plus a follow-up issue for a future projection surface, not something built inside this epic. |

---

## 6. Mechanical checks tooling — exists vs. must be written

| Check named in the brief | Exists? | Where |
|---|---|---|
| Schema-vs-docs command/flag inventory diff | **No script.** The only present mechanism is the AI reviewer manually cross-referencing `jit --schema`/`cli.rs` against docs per `scripts/doc-review-prompt.md:16-18,27-37` — judgment-based, not a deterministic diff. Nothing under `scripts/` does this mechanically (full listing checked: `agent-init-demo-project.sh, ai-review.sh, cargo-ci.sh, code-review-prompt.md, coverage-preview.sh, doc-review-prompt.md, fix_timestamps.py, generate-coverage-badges.sh, hooks/, jit-validate.sh, migrate_timestamps.py, plan-review-prompt.md, test-*.sh, validate-setup.sh` — none diff schema against docs). Must be written. |
| Link/anchor resolver (GitHub-style anchoring) | **No script**, but a documented ad-hoc method exists: `dev/active/76cb968b-citation-check.md` records a one-off grep-based citation-resolution recipe (run manually 2026-07-06 at commit `e70268af`, not committed as a script). Internal links are confirmed **uniformly relative markdown links** (213 `](...)` hits with `.md` targets across the in-scope surface, zero root-absolute links) — feasible for a grep-based resolver. `jit doc check-links` exists (`crates/jit/src/commands/document.rs:789`, tested at `crates/jit/tests/check_links_tests.rs`) but its `--scope` only accepts `all` or `issue:<ID>` (`crates/jit/src/document/scope.rs`) — it validates documents *attached to jit issues*, not arbitrary files under `docs/`. It cannot be pointed at the adopter-facing `docs/` tree as-is. The doc-review gate prompt (`scripts/doc-review-prompt.md:57-59`) already specifies the check in prose ("Extract every markdown link... For each intra-document anchor (`#heading`), confirm a matching heading exists...") but as an AI-judgment instruction, not a script. Must be written new. |
| Source-citation existence check | **No script**, same ad-hoc-recipe situation as above (`76cb968b-citation-check.md` covers `@/…` addressable-item citations specifically, resolved via `jit item list --json` — that part *is* scriptable today using `jit item show`/`jit item list`, just not yet packaged as a repo script). File-path citations (e.g. "see `crates/jit/src/cli.rs`") have no existing check at all. |
| Arrow-art grep (unicode arrows incl. `──`, `→`) | **No committed script.** `scripts/doc-review-prompt.md:65` specifies the character set to check (`│ ─ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ╭ ╮ ╰ ╯`) but only as AI-reviewer instruction text, and it explicitly does **not** cover bare arrow glyphs (`→` etc.) — see §4's open question. A standalone grep-based script is straightforward to write (I used exactly this technique during this investigation) but doesn't exist as a repo artifact today. |

**`doc-review` gate invocation and area-scoping:** registered at `.jit/gates.toml:127-146` — `type = "exec"`, `command = "./scripts/ai-review.sh"`, `prompt_file = "./scripts/doc-review-prompt.md"`, `stage = "postcheck"`, `auto = true`. **The gate cannot currently be scoped to a single doc area.** `scripts/doc-review-prompt.md:9-11` hardcodes the full in-scope surface ("The `docs/` tree... The root adopter docs... `mcp-server/README.md` and `web/README.md`") with no parameter for narrowing to e.g. just `concepts/`. `scripts/ai-review.sh` has no `SCOPE`/area env var (grep for `SCOPE`/`scope`/`$1` returned nothing). Running the gate as-is on an area-scoped task would audit the *whole* tree, including not-yet-fixed areas — which would fail the gate on unrelated findings. The plan needs to either (a) add scope parameterization to the gate's prompt/script (new infrastructure work), or (b) accept that per-area "doc-review passes" in the acceptance bar means something other than the literal existing gate invocation until the last area lands.

---

## 7. Deletion/merge policy grounding — link structure and orphan candidates

**Link structure: confirmed uniformly relative markdown links.** 213 `.md`-targeting `](...)` links found across the in-scope surface; a sample includes patterns like `../concepts/core-model.md#dependencies`, `cli-commands.md#jit-migrate-lifecycle-timestamps`, `../../CLAUDE.md`, `../../crates/jit/tests/example_rulesets_tests.rs`. Zero root-absolute (`](/...)`) links found. This confirms grep-based inbound-link discovery is feasible: for any candidate deletion/merge target, `grep -rn '<filename>' docs/ README.md ...` (matching both bare filename and relative-path forms) finds every inbound link that needs repointing.

**Orphan/redundancy candidates observed in passing (candidates, not verdicts):**
- `docs/README.md` — a 4-line stub pointing at `docs/index.md` (§1). Not redundant in a harmful sense (standard GitHub directory-README convention, zero inbound links to itself), but worth the plan explicitly deciding whether it's "in scope" as area-5-adjacent.
- No other clearly-orphaned or wholly-duplicate doc surfaced during this investigation's sampling; a real duplication/orphan sweep needs the exhaustive per-area pass itself (this investigation did not read every file end-to-end).

---

## Prior-art sweep

Searched `dev/active/` and `dev/archive/` for study/decision/session docs recording facts about the docs surface.

- **`dev/archive/287c4051-completion-report.md`** — read in full (see epic context above); its "Holistic Quality Notes" section is the direct source of the seven drift classes and independently confirms the "same fact wrong in file after file" pattern this investigation also found live in a new instance (§2, Class 3).
- **`dev/active/76cb968b-citation-check.md`** — a full manual verification (2026-07-06, commit `e70268af`) that every `@/…` addressable-item citation across authored markdown (`.claude/skills/`, `README.md`, `docs/`, `CLAUDE.md`) and Rust comments resolves against the live item index (453 items, zero dangling citations after adjudicating 6 non-index tokens as intentional test/example fixtures). Directly relevant to REQ-06 and REQ-02: it establishes that `@/…` citation hygiene was clean as of that commit, using a grep recipe rather than a script (§6).
- **`dev/sessions/session-2024-12-24-check-links-incomplete.md`** — a stale (2024-12-24) session note describing an *earlier, incomplete* implementation of `jit doc check-links` that lacked internal-link validation, git-based asset checks, and tests. Superseded: the command now has a dedicated test file (`crates/jit/tests/check_links_tests.rs`) and is documented (`docs/reference/cli-commands.md:2805-2816`). Still relevant background for why `jit doc check-links` scopes only to jit-tracked documents, not arbitrary `docs/` files (§6) — that was a known, named limitation from the start ("Scope Filtering: `--scope all` / `--scope issue:ID`", session note lines 26-29), not a regression.
- **`dev/archive/studies/docs-audit-plan.md`** — an older, unrelated docs audit (epic `cfb3ba94`, story `5326b331`, dated 2026-02-04, completed at commit `678da3c`). Findings (broken `labels.md` link, draft-status markers, claim-command syntax inconsistency, TDD-content duplication) are historical and pre-date both 287c4051 and this epic; not verified against current source since they are 5+ months and two doc epics old, but flagged as precedent that this kind of drift (e.g., `jit issue claim` vs `jit claim acquire` documentation clarity, item 2 in that report) has recurred before and could be worth a fresh spot-check by the exhaustive pass.
- No other `dev/active`/`dev/archive` document was found to catalogue facts specifically about the *current* docs-surface drift beyond what's already cited in the brief and the 287c4051 completion report.

---

## Summary of what could not be fully verified

- Exhaustive per-file correctness of the entire ~19,000-line in-scope surface — explicitly out of scope for this investigation (that is the epic's job); findings above are targeted spot-checks against the seven named drift classes plus REQ-04/05/06 grounding, not a line-by-line audit.
- Whether `docs/examples/`' "reference-grade vs illustrative" triage should follow the file-footprint split observed (§1) — presented as signal, not a decision; the brief assigns that triage to the plan/execution, not to this investigation.
- Whether REQ-04's "unicode-arrow spines" phrase is meant to reach single-line inline prose arrows (224 occurrences, §4) — flagged as an open interpretive question, not resolved.
