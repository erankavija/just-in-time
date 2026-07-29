# Investigation: 8e071e18 — Issue-based development artifact layout and serviceable archival

Verification of the container description's hypothesis against the live repository.
Sections 1–15 and A–E measured at HEAD `425c206a`; the **Addendum** (REQ-14, R1–R4, D-3/D-13/D-14/D-15)
measured at HEAD `dfb066fa` after the container was amended. Both readings re-verified current.

> **Read the addendum before planning.** It reports a blocker class
> (`unsupported-artifact-type`, 376 instances across 16 containers) that no criterion in the
> container currently addresses and that independently defeats REQ-02, REQ-09 and REQ-14.

**Headline results**

1. The two defect claims (unserviceable archival, flat-layout sprawl) are **real and reproduce
   exactly** as described. Preview output pasted in claim 4.
2. **REQ-11 and non-goal "updating the embedded profile assets" are mutually exclusive as
   written.** 55 skill files are byte-equality-asserted profile assets. See claim 13 / Open
   question Q1. This is the single most consequential finding.
3. **`cargo test` is red at HEAD**, unrelated to this epic, from that same byte-equality test.
   See claim 13. Blocks REQ-09's "gates pass" evidence until fixed.
4. **Claim 9 is false.** The `.jit-container` marker does not encode the directory name, so
   REQ-07's "with its frozen container marker updated to match" describes work that does not
   exist.
5. **REQ-01 as written does not achieve REQ-02.** Changing accessor defaults cannot make
   archival work, because policy completeness ignores defaults. See claim 2 / Q2.

---

## Claim classification (1–15)

### Claim 1 — archive refuses unmanaged selected roots — **valid-and-open** (location approximately right)

The mechanism exists as described. The claimed span `artifact_classifier.rs:838-850` lands
inside the right function but names neither its start nor the guard.

- Function: `selected_destination_roots`, `crates/jit/src/domain/artifact_classifier.rs:818-858`.
- Classification precedence, computed per candidate root in this order:
  `archived` = `contains_path(&policy.archive_root, source)` (`:833`), then `permanent` over
  `policy.permanent_paths` (`:834-837`), then `managed` over `policy.managed_paths`
  (`:838-841`), then `selected` (`:842-847`) — which for a container target is
  `owners().any(|o| o.inside_subtree && o.is_effectively_terminal() && !o.pinned)` and for a
  document target is the precomputed `document_all_terminal`.
- The guard: `if selected && !archived && !permanent && !managed` at `:848`, pushing
  `PlanBlocker::new(BlockerCode::UnmanagedSelectedRoot, Some(source))` at `:849-852` and
  returning `None` at `:853`.
- Blocker code declared at `crates/jit/src/domain/artifact_plan.rs:412`, listed in the
  `BlockerCode::ALL` contract at `:430`, wire spelling `"unmanaged-selected-root"` at `:448`.

Note the precedence is **disjunctive, not ordered**: a root that is any of archived, permanent,
or managed passes. Only a root that is none of the three blocks.

Blockers raised here are **target-level**, appended to the plan's `blockers` array — not to any
artifact's `blockers`. This is why the preview in claim 4 shows `action_counts.block: 0` while
still being ineligible. Any REQ-02 acceptance assertion must read the target-level array.

### Claim 2 — shipped `[documentation]` defaults — **valid-and-open, but the claim's framing understates the problem**

The default values are as claimed, but they are **accessor fallbacks, not a `Default` impl**,
and there is no `impl Default for DocumentationConfig` anywhere.

- `crates/jit/src/config.rs:331-339` — `DocumentationConfig::managed_paths()` returns
  `vec!["dev/active", "dev/studies", "dev/sessions"]` via `unwrap_or_else`.
- `crates/jit/src/config.rs:349-353` — `permanent_paths()` returns `vec!["docs/"]`.
- `crates/jit/src/config.rs:324-328` — `development_root()` returns `"dev"`.
- `crates/jit/src/config.rs:342-346` — `archive_root()` returns `"dev/archive"`.

**The load-bearing catch.** `PolicyStatus::from_documentation`
(`crates/jit/src/domain/artifact_plan.rs:85-97`) classifies policy **"by authored fields,
without consulting accessor defaults"** (its own doc comment, `:84`): `Configured` requires
`managed_paths.is_some() && permanent_paths.is_some() && archive_root.is_some()`. So for an
adopter who never authors a `[documentation]` table, the accessor defaults are *never* the
operative policy — the plan is `Unconfigured` and archival is refused regardless of what the
defaults say. Editing `config.rs` alone satisfies a literal reading of REQ-01 while leaving
REQ-02 unreachable for a default-initialized repository. See Q2.

**Every duplication of these defaults** (there are five, and they already disagree):

| Site | Content | Agrees with `config.rs`? |
|---|---|---|
| `crates/jit/src/config.rs:331-353` | the accessor fallbacks | authority |
| `crates/jit/src/config.rs:313,315,318` | doc comments restating each default | yes |
| `crates/jit/src/hierarchy_templates.rs:275-282` | **commented-out** scaffold emitted by `jit init` | **no** — `managed_paths = ["dev/active", "dev/sessions"]` at `:278` omits `dev/studies`, and `:281-282` advertise `design = "features"` / `session = "sessions"`, two keys that **do not exist** on `DocumentationConfig` (retired category keys) |
| `docs/reference/example-config.toml:21-26` | adopter example | yes |
| `docs/reference/configuration.md:48-52` | adopter reference | yes |
| `.jit/config.toml:11-14` | this repo's dogfood config (not shipped) | yes |

The `jit init` scaffold at `hierarchy_templates.rs:275-282` is the shipped-behaviour source that
matters for REQ-01 under the dogfooding boundary, and it is both stale and commented out.

### Claim 3 — "twelve areas" — **valid-and-open, exactly twelve**

`find dev -maxdepth 1 -mindepth 1 -type d` returns exactly 12:
`active`, `architecture`, `archive`, `benchmarks`, `design`, `eval`, `experiments`, `plans`,
`presentations`, `sessions`, `studies`, `vision`.

Against this repo's live `.jit/config.toml:11-14`:

- **Managed (3):** `dev/active`, `dev/studies`, `dev/sessions`
- **Archive root (1):** `dev/archive` — not an area to classify; it is the destination
- **Permanent (0 under `dev/`):** `permanent_paths = ["docs/"]` names nothing under `dev/`
- **Absent from both (8):** `dev/architecture`, `dev/benchmarks`, `dev/design`, `dev/eval`,
  `dev/experiments`, `dev/plans`, `dev/presentations`, `dev/vision`

So REQ-01 must classify **8 areas**, not the 2 (`presentations`, `architecture`) it names
explicitly. `dev/` also holds 3 loose files (`index.md`, `authoring-conventions.md`,
`TESTING.md`) that sit at the development root itself and belong to no area.

### Claim 4 — preview reproduces the gap — **valid-and-open, reproduces exactly**

`jit archive container 1cc809de --json` (preview, no `--execute`), real output:

```
eligible: false
policy_status: "configured"
destination_root: "dev/archive/1cc809de-repository-state-quality"
action_counts: {"move":2,"copy":1,"retain":24,"block":0,
                "already_archived":3,"pending_deletions":2}
count: 27
blockers: [
  {"code":"unmanaged-selected-root","path":"dev/architecture/repository-state-materialization.md"},
  {"code":"unmanaged-selected-root","path":"dev/presentations/1cc809de/talk.html"},
  {"code":"unmanaged-selected-root","path":"dev/presentations/cdc840ad/README.md"}
]
warnings: []
```

Exactly the three blockers claimed — the architecture document, the successor deck, and the
tombstone. Two artifact-level warnings also present, neither blocking:
`dev/archive/features/cdc840ad/showcase/talk.html` → `external-edge`;
`dev/presentations/1cc809de/vendor/reveal.js/reveal.css` → `dynamic-loading-suspected`.

Document preview on a deck owned by a terminal issue —
`jit archive document dev/presentations/9b7b5f9c/talk.html --json` (9b7b5f9c is `done`):

```
eligible: false
policy_status: "configured"
destination_root: "dev/archive"
action_counts: {"move":0,"copy":0,"retain":3,"block":0,
                "already_archived":0,"pending_deletions":0}
blockers: [{"code":"unmanaged-selected-root","path":"dev/presentations/9b7b5f9c/talk.html"}]
artifacts: dev/presentations/9b7b5f9c/base.css      retain
           dev/presentations/9b7b5f9c/talk.html     retain  warnings=[external-edge]
           dev/presentations/9b7b5f9c/themes/rust.css retain
```

Both halves of REQ-02 are genuinely blocked today. These two commands are the natural
acceptance evidence.

### Claim 5 — permanent archives by copy and retains the source — **valid-and-open, confirmed**

`crates/jit/src/domain/artifact_classifier.rs:662-675`:

```rust
let wants_destination = needs_destination.contains(&source);
let needs_source = archived_source || permanent || outside_owner || active_owner
    || unselected_unpinned || unmanaged_embedded
    || edge_source_constraints.contains(&source) || !wants_destination;
let mut action = match (wants_destination, needs_source) {
    (true, true) => ArtifactAction::Copy,
    (true, false) => ArtifactAction::Move,
    (false, _) => ArtifactAction::Retain,
};
```

`permanent` is one of seven disjuncts forcing `needs_source`, so a wanted permanent-path
artifact yields `Copy`. Source retention follows because `pending_deletions` is populated only
for `ArtifactAction::Move` (`:768-771`: the match arm is `(ArtifactAction::Move,
ArtifactLocation::Regular(identity))`), so a `Copy` schedules no deletion. `EvidenceCode::PermanentPath`
is attached at `:652`. Managed-path artifacts with no other retention reason get `Move` and a
pending deletion.

This confirms the description's reasoning for putting `dev/architecture` in `permanent_paths`.

### Claim 6 — counts — **one number is wrong (95, not 96); the others are exact**

Method: `find` over the live tree; ownership resolved by parsing a leading 8-hex-char filename
prefix and looking the short id up in `jit issue list --full --json` (703 issues); terminal =
`done` | `rejected` | `archived`.

| Claim | Measured | Verdict |
|---|---|---|
| 172 artifacts named `<short-id>-<artifact>` at area root across `dev/active` + `dev/archive` | **172** | exact |
| 126 files in `dev/active` | **126** (`maxdepth 1`; 134 recursive) | exact |
| "96 of the 126 owned by terminal issues" | **95** | **off by one** |

Full `dev/active` (`maxdepth 1`) breakdown, summing to 126:

- **103** carry a parseable short-id prefix; every one resolves to a known issue (0 unknown).
  - **95** owned by a terminal issue → REQ-09's cleanup set.
  - **8** owned by a live issue, all `backlog`: `13c69884-dfe9-4a62-b2ef-59fa4d2f76f3-plan.md`,
    `8b05a612-{investigation,plan,research}.md`, `9db27a3a-progress.json`,
    `c639cfb5-{investigation,plan,research}.md`.
- **23** carry **no** parseable short-id prefix → REQ-10's disposition set:

```
agent-validation-design.md                          json-output-standardization-plan.md
bulk-operations-plan.md                             multi-issue-bulk-operations-plan.md
ci-gate-integration-design.md                       observability-design.md
config-consolidation-documentation-requirements.md  planning-bracket-design.md
config-consolidation-plan.md                        production-polish-design.md
dependency-display-improvements-plan.md             production-stability-design.md
doc-archive-implementation-guide.md                 quiet-mode-plan.md
documentation-lifecycle-design.md                   rejection-state-design.md
documentation-lifecycle-phase2-design.md            snapshot-export-implementation-plan.md
gate-examples.md                                    transitive-reduction-validation-plan.md
gate-modification-flags-plan.md                     v1-production-readiness-scope-brief.md
gate-presets-implementation-plan.md
```

Prior art (below) already marks at least four of these superseded, which shortens REQ-10.

REQ-10 also covers `dev/studies` and `dev/sessions`: **`dev/studies` holds 19 files of which
exactly 1** (`cdc840ad-audit-2026-07-23.md`) is prefixed — 18 need a disposition, including a
2-file `perf/` subdirectory. **`dev/sessions` holds 30 files, none prefixed** — all 30 follow
`session-<date>-<topic>.md`. So REQ-10's true scope is **23 + 18 + 30 = 71 files**, not the 23
a `dev/active`-only reading implies. Sizing REQ-10 off `dev/active` alone understates it 3x.

Caveat: this file (`dev/active/8e071e18-investigation.md`) makes it 127/104/96-prefixed once
written, owned by live issue 8e071e18.

### Claim 7 — `dev/archive` mixes planner output with hand-made subdirectories — **valid-and-open, confirmed**

Actual top-level shape (`dev/archive`), three disjoint populations:

- **69 loose files** at archive root, all `<short-id>-<artifact>.{md,json}` — the largest
  population, and the shape the epic wants to retire.
- **2 planner-shaped directories**: `7d3a3a47/` (bare short id, legacy) and
  `a9b5dd08-use-strategic-label-slugs-in-archive-directory-n/` (title-slugged). Both carry a
  `.jit-container` marker.
- **5 hand-made semantic directories**: `bug-fixes/`, `features/`, `refactorings/`,
  `sessions/`, `studies/`. None carries a marker.

D-9 keeps the 5 semantic directories as legacy. Note `features/` is itself internally
inconsistent: `features/25064508/` and `features/2821e177/` are per-issue directories, while
`features/2fbd2a82-completion-report.md`, `features/53e3fa36-progress.json`,
`features/90a2dbfd-*`, `features/94d26c42-validation-lifecycle-design.md` and
`features/d0b85bff-archive-json-plan.md` are loose prefixed files.

### Claim 8 — title-derived slug fallback truncates mid-word — **valid-and-open; exactly ONE directory needs renaming**

Naming code: `preferred_container_destination_root`,
`crates/jit/src/domain/artifact_classifier.rs:1054-1081`.

Precedence (three steps, not two):

1. Collect `type:*` label values (`:1059-1063`). **Only if exactly one type label exists**
   (`issue_types.len() == 1`, `:1064`) look up its membership namespace via
   `hierarchy.get_membership_namespace` (`:1065`).
2. Collect that namespace's label values; **only if exactly one exists** (`values.len() == 1`,
   `:1074`) use it.
3. Otherwise fall back to `issue.title` — `archive_container_slug(strategic_value.unwrap_or(&issue.title))`
   at `:1076`.

Result is `join_path(archive_root, &format!("{}-{slug}", container_short_id(&issue.id)))`
(`:1077-1080`), so the short id always prefixes.

`archive_container_slug` (`:1084-1107`): lowercases alphanumerics, collapses every
non-alphanumeric run to a single `-`, truncates to `const MAX_CHARS: usize = 48` (`:1085`) with
`.take(MAX_CHARS)` (`:1100`), trims a trailing `-` (`:1101`), and substitutes the literal
`"container"` when empty (`:1102-1104`). The truncation is a raw character cut with no word
boundary — hence the mid-word `...-directory-n`.

**Exhaustive audit of existing archive directories** (recomputed both candidate slugs for each):

| Directory | Slug source | Marker | REQ-07 action |
|---|---|---|---|
| `dev/archive/7d3a3a47` | **bare short id** (legacy, predates a9b5dd08) | `7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc` | none — already conformant |
| `dev/archive/a9b5dd08-use-strategic-label-slugs-in-archive-directory-n` | **TITLE FALLBACK** | `a9b5dd08-c6ae-4add-99f1-275289e43f0c` | rename to `dev/archive/a9b5dd08` |

**Exactly one directory requires renaming.** The 5 hand-made semantic directories are not
short-id-shaped and carry no marker, so they are out of scope per D-9.

Pleasing detail for the plan: a9b5dd08 is `type:enhancement`, and `.jit/config.toml`'s
`[type_hierarchy.label_associations]` maps only `epic`/`milestone`/`story`. So it has no
membership namespace, meaning under REQ-07's no-fallback rule its name becomes the bare
`dev/archive/a9b5dd08` — **which is exactly the shape the resolver already supports as
`legacy_root`** (`artifact_classifier.rs:241-249`). The rename converges on an existing
supported shape rather than inventing one.

### Claim 9 — marker content encodes the directory name — **INVALID AS STATED**

The `.jit-container` marker content is **the full container UUID and nothing else**. It has no
relationship to the directory name.

- **Written** at `crates/jit/src/repository_state/archive.rs:121-135`: for a
  `PlanTarget::Container { id }`, `let bytes = format!("{id}\n").into_bytes()` (`:123`),
  published to `format!("{}/.jit-container", plan.destination_root())` (`:122`) as a
  `RepositoryAction::WriteFile` with `expected: ExpectedPreimage::Absent` (`:132`).
- **Read** at `crates/jit/src/domain/artifact_classifier.rs:213-227`: scans the archive root's
  immediate children and matches
  `ArtifactEvidence::File(owner) if owner.trim_ascii() == container_id.as_bytes()` (`:222`).
- Confirmed on disk: `dev/archive/a9b5dd08-use-strategic-label-slugs-in-archive-directory-n/.jit-container`
  contains `a9b5dd08-c6ae-4add-99f1-275289e43f0c`; `dev/archive/7d3a3a47/.jit-container`
  contains `7d3a3a47-c03c-473d-b1c2-c89eefcc9bbc`.
- The doc comment at `artifact_classifier.rs:136` states the contract: *"Its `.jit-container`
  marker names this exact full container id."*

**Consequence for REQ-07.** "every pre-existing title-slugged archive directory is renamed
**with its frozen container marker updated to match**" describes work that does not exist —
there is nothing in the marker to update. Directory-name independence is precisely what makes
the rename safe: after `git mv`, the marker still matches, `resolve_container_destination`
re-adopts the directory by marker (`:213-239`), and the plan converges. REQ-07's marker clause
should be reworded to assert the marker **remains valid and continues to resolve** after
renaming. As written it is unsatisfiable, and a reviewer checking it literally will find no
code to point at.

### Claim 10 — `reference_changes` remapping — **valid-and-open; scope is narrower than the phrase suggests, and a warning channel already exists**

**What it rewrites: jit document *link records* only.** `ReferenceChange`
(`crates/jit/src/domain/artifact_plan.rs:350-363`) is
`{ issue: String, document_index: usize, from_path: String, to_path: String }` — a durable
issue-document relink addressed by issue id and index into that issue's document list.

Built at `crates/jit/src/domain/artifact_classifier.rs:755-767`, one per selected owner, and
suppressed entirely when the source is already archived and explicit (`:755-756`). Applied on
execution via `crates/jit/src/repository_state/archive.rs:172`.

**What it does NOT rewrite: document content.** No markdown-path rewriting exists anywhere in
the archive path. Confirmed by the absence of any content-mutating action: execution emits only
`RepositoryAction::WriteFile` (`archive.rs:83`, `:129`), `DeleteFile` (`:258`), and
`CreateDirectory` (`:397`); the only `WriteFile` with synthesized bytes is the `.jit-container`
marker. So D-8's "execution rewrites no document content" is **already true today** — REQ-08's
second half needs a regression test, not an implementation.

**A plan-warning channel already exists**, at both levels:

- `PlanWarning { code: WarningCode, path: Option<String> }` —
  `crates/jit/src/domain/artifact_plan.rs:576-591`, path normalized in the constructor (`:588`).
- Target-level `warnings: []` and per-artifact `warnings: []` are both already in the JSON
  envelope (see claim 4 output; the `dynamic-loading-suspected` warning demonstrates the
  per-artifact channel live).

**The cost REQ-08 must budget for.** `WarningCode` is a **closed schema-v1 contract**:
`pub enum WarningCode` has 8 variants (`artifact_plan.rs:494-503`), `pub const ALL: [Self; 8]`
(`:507-516`), and stable kebab spellings (`:519-530`). It is golden-tested at
`crates/jit/tests/fast_docs_templates/artifact_plan_model_tests.rs:333`
(`WarningCode::ALL.map(WarningCode::as_str)`), with the sibling `BlockerCode::ALL` assertion at
`:315`. Adding a 9th code for in-content citation breakage requires updating the array length,
the golden test, and — per prior art `7d3a3a47-plan.md:357-359` — carries an explicit
schema-versioning decision. Flag this in the plan; it is not a free addition.

Detecting in-content citations is genuinely new work: the planner reads document **edges**
(`artifact_edges`, used at `artifact_classifier.rs:862`), which is asset/link graph data, not a
scan of arbitrary prose path mentions. REQ-08 needs to decide what counts as a "citation" — see Q4.

### Claim 11 — directory-per-issue precedents — **valid-and-open, all three confirmed**

| Precedent | Confirmed |
|---|---|
| `dev/presentations/<short-id>/` | 4 instances: `1cc809de/`, `2e926e39/`, `9b7b5f9c/`, `cdc840ad/` |
| `dev/archive/features/<short-id>/` | 4 instances: `25064508/`, `2821e177/`, `cdc840ad/`, `dbe1e821/` |
| planner `<short-id>-<slug>/` | `preferred_container_destination_root`, `artifact_classifier.rs:1054-1081`; on disk `dev/archive/a9b5dd08-...` and legacy `dev/archive/7d3a3a47/` |

D-2 cites `dev/archive/features/25064508/` as the filename model. That directory is
**conformant**: `completion-report.md`, `plan.md`, `progress.json`, `dogfood-9ac9fdac.md`,
`showcase/` — no short-id prefixes. But the sibling `features/2821e177/` is **not**:
`2821e177-handoff.md`, `2821e177-handoff-2.md`, `2821e177-progress.json` alongside an
unprefixed `completion-report.md`. The precedent is real but not uniform; cite 25064508
specifically.

### Claim 12 — `.agents/skills/**` hard-codes flat paths — **valid-and-open**

Confirmed and enumerated exhaustively in the **Consumer inventory** section below.

### Claim 13 — embedded profile assets ship skill content — **valid-and-open, and materially worse than "a separate follow-up"**

Profile assets are embedded from `profiles/jit-dogfood/` via `include_dir!` at
`crates/jit/src/profile/dogfood.rs:8-9`, with live-consumer sources under the prefix
`assets/live/` (`JIT_DOGFOOD_LIVE_SOURCE_PREFIX`, `:12`).

`profiles/jit-dogfood/manifest.toml` declares **55** assets under
`assets/live/.agents/skills/` (lines 334–554). The flat pattern **is** duplicated there —
31 occurrences of `dev/active` across `profiles/`, including
`profiles/jit-dogfood/manifest.toml:289` (`doc = "dev/active/{container.short_id}-plan.md"`)
and every skill file that carries it in the root tree.

**The blocking coupling.** `test_live_assets_match_every_declared_source_tree_consumer`
(`crates/jit/src/profile/dogfood.rs:437-474`) asserts, for every declared live asset, that the
repo file is a **byte-exact copy** of the packaged source (`:451-458`, "drifted from the
package"), plus matching executable mode (`:459-473`). Every REQ-11 target file is a declared
asset — e.g. `jit-planning-lead/SKILL.md` (manifest `:456`), `jit-execution-lead/SKILL.md`
(`:350`), `handoff-template.md` (`:378`), `progress-file.md` (`:386`), `jit-manage/SKILL.md`
(`:412`), `jit-breakdown/SKILL.md` (`:334`), `investigator-prompt.md` (`:464`),
`researcher-prompt.md` (`:472`), `standards-scan.md` (`:525`), `standards-sweep.md` (`:529`),
`progress-artifact.md` (`:517`), `vision-charter.md` (`:541`).

So editing a `.agents/skills/` file **without** its `profiles/` twin fails a unit test in
`cargo test`, and therefore the `cargo-ci` gate. REQ-11 and the non-goal "Updating the embedded
profile assets that ship `.agents/skills` content" (D-11) cannot both hold. See **Q1**.

**Pre-existing red build at HEAD.** This test **fails right now**, before any work on this epic:

```
test profile::dogfood::tests::test_live_assets_match_every_declared_source_tree_consumer ... FAILED
panicked at crates/jit/src/profile/dogfood.rs:453:13:
assertion `left == right` failed:
  .agents/skills/jit-execution-lead/SKILL.md drifted from the package
```

Working tree is clean (`git status --porcelain` empty). HEAD commit `425c206a` edited
`.agents/skills/jit-execution-lead/SKILL.md` (1 line changed, `:268`) and
`.agents/skills/jit-execution-lead/references/lead-review-protocol.md` (14 lines added after
`:120`) and touched **zero** files under `profiles/` (`git show --name-only 425c206a | grep -c
profiles` → 0). Both files are declared assets (manifest `:350`, `:382`).

REQ-09 requires `jit validate` to pass; the gates additionally require `cargo test`. **This
must be repaired before any REQ-09 evidence is credible**, and it is a live demonstration of
exactly the coupling Q1 asks about. `jit validate` itself currently passes
(`valid: true`, 0 errors, 0 warnings) — the failure is confined to `cargo test`.

Two files also exist only in the root tree (`evals/`, `trigger_eval*.json`,
`jit-project-lead/scripts/test-standards-*.sh`, `references/.gitkeep`); these are undeclared
and correctly unconstrained by the test.

### Claim 14 — `dev/plans` and `dev/vision` are mixed — **partly invalid: `dev/plans` is not mixed**

**`dev/plans` — 6 files, ALL issue-associated, ALL terminal.** Not mixed by ownership; mixed
only by *prefix position* (2 prefix, 4 suffix):

| File | Position | Issue state |
|---|---|---|
| `5dbc3548-deletion-tracking.md` | prefix | `done` |
| `7004d5b6-reorganize-commands.md` | prefix | `done` |
| `benchmarks-8d80b5dd.md` | **suffix** | `rejected` |
| `error-recovery-0587a73a.md` | **suffix** | `rejected` |
| `metrics-713ff59d.md` | **suffix** | `rejected` |
| `stalled-detection-c802a9b0.md` | **suffix** | `rejected` |

The whole area is terminal-owned, so it could be classified managed and archived wholesale.
The suffix form is a distinct wrinkle: a prefix-based resolver will not recognize 4 of these 6.

**`dev/vision` — 4 files, genuinely mixed:**

| File | Prefix | Issue state |
|---|---|---|
| `9db27a3a-charter.md` | `9db27a3a` | **`backlog` (LIVE)** |
| `document-graph-implementation-plan.md` | none | — |
| `document-viewer-implementation-plan.md` | none | — |
| `knowledge-management-vision.md` | none | — |

`dev/vision/9db27a3a-charter.md` is **projection input**: it is the charter source cited by
`.jit/config.toml:202` (`source = "dev/vision/9db27a3a-charter.md"`) and by `CLAUDE.md:100` /
`AGENTS.md:100` as what `jit project render` projects from. It is owned by a live issue and
must not be archived or moved without repointing the projection source. `dev/vision` should be
**permanent**, and the per-file call D-3 asks for is really a per-file call for `dev/vision`
only — `dev/plans` can be decided as a whole.

### Claim 15 — adopter docs state the classification in several places — **valid-and-open**

Canonical-home candidate and the competing statements are catalogued in **Documentation
inventory** below. Summary: `docs/reference/configuration.md` §`[documentation]` (`:45-88`) is
the strongest anchor — it already owns the config keys and the move/copy semantics, and already
disclaims itself as an example rather than a vocabulary (`:84-87`). It covers **2 of 4** fact
categories; the path convention lives only in `docs/how-to/adopt-planning-bracket.md:267-269`,
and the command mechanics only in `docs/reference/cli-commands.md:188-357`.

The sharpest REQ-12 target is **`dev/index.md:19-104`**, which independently narrates a
retention-window-and-category-subfolder lifecycle that contradicts the shipped mechanism
(`cli-commands.md:255-256` says archival "has no age, retention, category, suggestion, or
default target behavior"; `:299` gives the real `<archive_root>/<container-short-id>-<slug>/`
destination). `dev/authoring-conventions.md` contradicts **itself**: `:65-71` shows
`mv dev/active/... dev/archive/features/` while `:279-280` states "archival has no category
input."

---

## Design-surface findings (A–E)

### A. REQ-04 resolver command

**Existing surface.** Top-level `Commands` enum, `crates/jit/src/cli.rs:40`. Relevant arms:
`Doc(DocCommands)` (`:172`), `Archive(ArchiveCommands)` (`:176`), `Item(ItemCommands)` (`:288`),
`Project(ProjectCommands)` (`:306`), `Validate {...}` (`:353`).

- `jit doc` (`DocCommands`, `cli.rs:1833`): `add`, `list`, `remove`, `show`, `history`, `diff`,
  `assets`, `check-links`. All operate on an **already-known path** — none derives one.
- `jit archive` (`ArchiveCommands`, `cli.rs:1985`): `candidates`, `document`, `container`. All
  preview-or-execute; none exposes a bare path.

**Recommended home: `jit doc`.** `jit archive` is defined as "Preview dependency-aware artifact
archival plans" and its every subcommand produces a plan; a pure path resolver does not belong
there. `jit doc` already owns the document-path vocabulary. A `jit doc path <issue>` /
`jit doc dir <issue>` fits without stretching either group's stated purpose.

**Helpers to reuse** (all already exist; the resolver should not re-derive):

- `crates/jit/src/domain/type_taxonomy.rs:215-217` — `get_membership_namespace(type_name)`.
- `crates/jit/src/domain/artifact_classifier.rs:1084-1107` — `archive_container_slug`, already
  `pub`, already Unicode-safe, bounded and non-empty. Reuse verbatim; do not write a second
  slugifier.
- `crates/jit/src/config.rs:324-353` — `DocumentationConfig` accessors for the area root.
- `crates/jit/src/commands/plan_doc.rs:115-124` — `resolve_plan_doc_location`, the existing
  template-based plan-doc resolver.
- `crates/jit/src/commands/template_expand.rs:429-484` — `InterpolationContext::interpolate`,
  and `crates/jit/src/commands/template.rs:526-540` — `render_template_document_path`. Two
  independent `{container.short_id}` substitution engines already exist; REQ-04 should reuse,
  not add a third.

**`--json` envelope for a single value.** The list envelope `{"count": N, "<collection>": [...]}`
does not apply. The established single-value shape is a flat named object — `jit config get
documentation.archive_root --json` returns `{"key": ..., "value": ...}`, and `jit item show
--json` returns `{"item": {...}, "issue_full_id": ..., "issue_title": ...}`. Follow that.

**Out-of-scope APIs, located so the plan can avoid them.**
`VirtualPath` is declared at `crates/jit/src/repository_state/path.rs:139` with impls at
`:144`, `:202`, `:258`. It is the repository-state layer's validated path newtype; per F1 it
has **no join/child API**, which is why ~78 composite sites use the fallible constructor.
`InMemoryStorage::issue_vpath` is at `crates/jit/src/storage/memory.rs:339`, and its only
consumer is `crates/jit/src/storage/memory.rs:481` (`Self::data_entry_bytes(state,
&Self::issue_vpath(id))`) — a single call site, so scoping it out costs nothing.

A resolver in `commands/` returning a `String`/`PathBuf` touches neither. Keep it that way.

### B. REQ-06 advisory report

Three candidate mechanisms exist:

1. **Non-enforced rule in `.jit/rules.toml`.** Severity and enforcement are independent:
   `Strictness::blocks(self, enforce: bool, severity: Severity)`
   (`crates/jit/src/validation/strictness.rs:58`) gates blocking on `enforce`, so
   `severity = "error", enforce = false` reports without blocking. Live precedent at
   `.jit/rules.toml:20-25` and `:28-33` (both `severity = "error"`, `enforce = false`), and
   `:86-91`/`:94-99` (`severity = "warn"`, `enforce = false`).
   **Caveat:** D-7 explicitly rejects "a blocking validation rule", and the non-goals repeat it.
   A non-enforced rule is technically non-blocking, but it rides inside `jit validate`, whose
   exit code the repo-validate gate consumes. Prefer option 2 unless the owner says otherwise.
2. **A dedicated report command.** No conformance-report precedent exists — prior art confirms
   "conformance" appears only as a *golden test*, never a report or command
   (`7d3a3a47-plan.md:357-359`). This would be new surface but carries the cleanest guarantee
   of "blocks no state transition": a command that only ever prints cannot gate anything.
3. **The existing plan-warning channel** (`PlanWarning`, `artifact_plan.rs:576-591`) — already
   non-blocking and already in the JSON envelope, but scoped to an archive plan, not to a
   standing audit of an area.

**Recommendation: option 2**, a `jit doc` report subcommand sharing the REQ-04 resolver. It
satisfies "blocks no state transition" by construction and sits beside the resolver it depends
on. Note `.agents/skills/jit-project-lead/scripts/standards-scan.sh` is an existing shell-level
precedent for exactly this kind of advisory area scan (it reads `development_root` from config
at `:84` rather than hardcoding).

### C. REQ-03 slug source — a real precedence gap, already half-solved

The mapping is `[type_hierarchy.label_associations]`, read through
`HierarchyConfig::get_membership_namespace` (`crates/jit/src/domain/type_taxonomy.rs:215-217`),
a thin `self.label_associations.get(type_name).map(String::as_str)`. Reverse iteration is
available via `membership_namespaces()` (`:222-224`). This repo maps
`{epic = "epic", milestone = "milestone", story = "story"}` (`.jit/config.toml`).

**Multi-label behaviour is already defined — as "give up", twice** (both in
`preferred_container_destination_root`, `artifact_classifier.rs:1054-1081`):

- Several `type:*` labels → `issue_types.len() == 1` is false (`:1064`) → no membership lookup.
- Several values in the membership namespace → `values.len() == 1` is false (`:1074`) → none.

Today both cases fall through to the title. **REQ-03 forbids the title fallback, so the plan
must say what replaces it.** The consistent answer is the bare `<area>/<short-id>` directory —
it is what REQ-03 already specifies for the no-membership-label case, it needs no new
tie-breaking rule, and it matches the `legacy_root` shape the resolver already handles
(`:241-249`). Ambiguity resolution by sorting or by hierarchy level would be new policy; ask
before adding it (**Q3**).

Worth noting: 8e071e18 itself carries `epic:dev-artifact-layout` **and** `milestone:v1.0` — two
membership-namespace labels, but only one `type:epic`, so step 1 selects the `epic` namespace
and step 2 finds exactly one `epic:` value. It resolves cleanly. The ambiguous case needs an
issue with two type labels or two values in one namespace.

### D. REQ-05 filename generators

Four independent generators; REQ-05 must reach all four or it will be partially true.

1. **Template interpolation (engine).** `.jit/templates.toml:28` —
   `doc = "dev/active/{container.short_id}-plan.md"`; also
   `profiles/jit-dogfood/manifest.toml:289`. Substituted by
   `InterpolationContext::interpolate` (`commands/template_expand.rs:429-484`),
   `render_template_document_path` (`commands/template.rs:526-540`), and
   `resolve_plan_doc_location` (`commands/plan_doc.rs:115-124`).
2. **Archive planner (engine).** `artifact_destination_root`
   (`artifact_classifier.rs:1041-1046`) and `preferred_container_destination_root` (`:1054-1081`).
   These name the *directory*; the mirrored file keeps its source basename.
3. **Skill prose (agent-composed).** The bulk — every `(a)` row in the consumer inventory.
   No engine involvement; an agent reads the pattern and writes the file.
4. **`jit init` scaffold.** `crates/jit/src/hierarchy_templates.rs:275-282` (commented out,
   and stale — see claim 2).

The `jit apply` template path (1) is the only one with a real resolver today, and it is the one
whose output REQ-05 changes shape for.

### E. Test topology

**Budget mechanism.** `scripts/rust-build-budget.sh` declares
`readonly MAX_INTEGRATION_TARGETS=12` (`:39`) and
`readonly MAX_EXECUTABLE_BYTES=$((2 * 1024 * 1024 * 1024))` (`:40`), enforced at `:142` and
`:165`. It is run by `scripts/cargo-ci.sh` after normal compilation (`rust-build-budget.sh:8-10`)
and self-tested by `crates/jit/tests/scratch_build/rust_build_budget_checker_tests.rs`.

**Current headroom: 2.** There are no `[[test]]` entries in `crates/jit/Cargo.toml` (targets are
auto-discovered from `crates/jit/tests/*/main.rs`), and exactly **10** exist: `cli_gate`,
`cli_issue`, `cli_item_validate`, `cli_query_graph`, `cli_repo_workflow`,
`fast_docs_templates`, `fast_issue`, `fast_rules`, `provenance_contract`, `scratch_build`.
(`common/` and `fixtures/` have no `main.rs` and are shared modules, not targets.)

**Reuse these suites — adding a new target is unnecessary:**

| Concern | Suite |
|---|---|
| archive plan model, warning/blocker golden contract | `crates/jit/tests/fast_docs_templates/artifact_plan_model_tests.rs` (`:315` blockers, `:333` warnings) |
| artifact discovery | `crates/jit/tests/fast_docs_templates/artifact_discovery_tests.rs` |
| archive CLI preview + JSON envelope | `crates/jit/tests/cli_repo_workflow/archive_preview_cli_tests.rs` (`:702` already asserts absence of `unmanaged-selected-root`) |
| `[documentation]` config surface | `crates/jit/tests/cli_repo_workflow/config_get_tests.rs` |
| terminal-state semantics | `crates/jit/tests/fast_issue/archived_semantics_tests.rs` |
| template interpolation → doc path | `crates/jit/tests/fast_docs_templates/template_apply_tests.rs`, `template_binding_tests.rs` |
| classifier unit tests | in-module `mod tests`, `crates/jit/src/domain/artifact_classifier.rs:1154-2141` |
| profile asset/manifest integrity | in-module, `crates/jit/src/profile/dogfood.rs:437` |

`crates/jit/tests/cli_repo_workflow/archive_preview_cli_tests.rs:702` is the natural home for
REQ-02's acceptance test — it already asserts the negative on this exact blocker code.

---

## Prior art

- **`dev/archive/1cc809de-completion-report.md`** — the named evidence. **F2** (`:78`):
  *"`dev/presentations` is outside `[documentation].managed_paths`, so deck archival cannot use
  the product's own archival workflow."* **F1** (`:77`) is the `VirtualPath` join/child gap and
  `InMemoryStorage::issue_vpath` panic — correctly a non-goal here. The archival note (`:89`,
  `:94-100`) records the `git mv` bypass, the dangling reference, the hand-repaired
  `jit validate` failure, and the previewed 3-blocker result this investigation reproduced
  verbatim. Recommended follow-ups there: widen `managed_paths`, **or** teach the planner the
  feature-archive shape. This epic takes the first.
- **`dev/archive/a9b5dd08-.../active/a9b5dd08-archive-directory-slugs.md`** — the design
  behind the current slug resolver. Two phases (`:27-51`): derive a preferred name from the
  single own-type membership label, then reconcile against the archive root's immediate
  children by `.jit-container` marker. Its REQ-05 *introduced* the ≤48-char title fallback that
  this epic's REQ-07 now removes — so REQ-07 is a **deliberate reversal of a shipped, reviewed
  decision**, not a bug fix. The plan should say so; `a9b5dd08` passed `code-review` and
  `doc-review`. Its REQ-03 (freeze the name at first archive) and REQ-04 (adopt legacy id-only
  directories with no migration) are what make the single rename safe.
- **`dev/archive/7d3a3a47/active/7d3a3a47-plan.md`** — the archival mechanism's own design.
  **D-12** (`:90-109`): the destination is one container-owned directory mirroring the
  repository-relative source subtree — rationale is relative-offset preservation,
  collision-freedom, and invertibility for rerun convergence. **D-21** (`:766-773`): archival
  takes *no category input anywhere*. **D-1** (`:620-623`): three-state policy completeness;
  defaults never authorize mutation — the direct source of the claim-2 catch. **D-25**
  (`:800-805`): container membership is the resolved-hierarchy children closure, not the
  dependency closure.
- **Superseded, safe to retire under REQ-10** (each carries a supersession banner at `:3-6`):
  `dev/active/documentation-lifecycle-design.md`, `dev/active/doc-archive-implementation-guide.md`,
  `dev/active/documentation-lifecycle-phase2-design.md`. `dev/active/config-consolidation-plan.md`
  is *partially* superseded (banner covers only its archive-category discussion; header still
  says `Status: Active`). `dev/studies/documentation-lifecycle-strategy.md` self-declares
  SUPERSEDED (`:3-14`). `dev/studies/documentation-organization-strategy.md` is mixed: its
  `docs/`↔`dev/` split and config shape (`:144-151`) match today's config, but its
  category-based archive proposal (`:171-180`) is dead.
- **Rejected options a new plan must not re-propose**: caller-selected archive categories,
  `doc_type`-derived category inference, content-addressed archive layout, short-id-only
  destination segments, stripping the managed prefix per artifact, dependency-closure
  membership, retaining a legacy `jit doc archive` alias (all `7d3a3a47-plan.md`); external-tool
  link rewriting and prohibiting relative shared links (`documentation-lifecycle-phase2-design.md:640-659`).
- **No prior document proposes** a directory-per-issue *source* layout, a resolver *command*, or
  a conformance *report*. All three are new surface. "Resolver" exists only as internal domain
  naming (`7d3a3a47-plan.md:343`, `a9b5dd08-archive-directory-slugs.md:27`); "conformance" only
  as a golden test (`7d3a3a47-plan.md:357-359`).

---

## Consumer inventory (REQ-11 and REQ-04 scope)

Classification: **(a)** hard-coded flat pattern a resolver should replace · **(b)**
example/illustration/test fixture · **(c)** live path to an existing file or real production code.

### `.agents/skills/**` — class (a), the REQ-11 working set

Every row below is **also** a declared profile asset (see claim 13), so each edit is coupled.

**jit-planning-lead**
| path:line | string |
|---|---|
| `SKILL.md:13` | ``- `dev/active/<C-short-id>-plan.md`: shared design, decisions, risks, and a generated overview.`` |
| `SKILL.md:14` | ``- `dev/active/<C-short-id>-breakdown.json`: the complete authoritative issue graph…`` |
| `SKILL.md:41` | ``` `dev/active/<C-short-id>-investigation.md` to `P`. Consumer and file inventories ``` |
| `SKILL.md:57` | `  dev/active/<C>-breakdown.json --config .jit/config.toml \` |
| `SKILL.md:58` | `  --plan dev/active/<C>-plan.md --known-source <every-valid-source-id> ... \` |
| `SKILL.md:62` | `  dev/active/<C>-breakdown.json dev/active/<C>-plan.md --write` |
| `SKILL.md:64` | `  dev/active/<C>-breakdown.json dev/active/<C>-plan.md --check` |
| `SKILL.md:65` | `jit issue batch-create --from-json dev/active/<C>-breakdown.json --dry-run --json` |
| `references/investigator-prompt.md:36` | ``(`dev/active/<C-id>-investigation.md` under the project's development docs path — read`` |
| `references/researcher-prompt.md:42` | ``1. Write to the project's development docs path (e.g. `dev/active/<C-id>-research.md` —`` |
| `references/plan-doc-template.md:7` | ``> [<C-short-id>-breakdown.json](<C-short-id>-breakdown.json).`` — relative link; **REQ-05 changes this to a bare `breakdown.json`** |

**jit-execution-lead**
| path:line | string |
|---|---|
| `SKILL.md:29` | ``8. **Resumable state.** Persist progress to `dev/active/<short-id>-progress.json`…`` |
| `SKILL.md:92` | ``5. **Resume check.** If `dev/active/<short-id>-progress.json` exists:`` |
| `SKILL.md:96` | ``- Also read every `dev/active/<short-id>-handoff*.md` in order (oldest to newest).`` |
| `SKILL.md:141` | ``6. **Persist the wave plan** to `dev/active/<short-id>-progress.json` per `references/progress-file.md`.`` |
| `SKILL.md:250` | ``Save to `dev/active/<epic-short-id>-handoff-<N>.md` where `<N>` is 1 more than the highest existing handoff index…`` |
| `SKILL.md:254` | ``3. **Update the progress file.** Ensure `dev/active/<epic-short-id>-progress.json` reflects…`` |
| `references/progress-file.md:3` | ``Persist the wave plan to `dev/active/<epic-short-id>-progress.json`:`` |
| `references/handoff-template.md:5` | ``Save to the project's managed docs path (typically `dev/active/<epic-short-id>-handoff.md`, or `dev/active/<epic-short-id>-handoff-<N>.md`…)`` |
| `references/handoff-template.md:23` | ``- Progress file: `<managed-docs-root>/<short-id>-progress.json` (reflects the above)`` |
| `references/architect-agent-prompt.md:51` | ``path as `<managed-docs-root>/<short-id>-<slug>.md`.`` |
| `references/explorer-agent-prompt.md:49` | ``as `<managed-docs-root>/<short-id>-<slug>.md`.`` |
| `references/doc-agent-prompt.md:54` | ``- Development docs → the project's managed paths (typically `dev/active/`)`` |

**jit-manage**
| path:line | string |
|---|---|
| `SKILL.md:57` | ``   the project's doc paths (plans/designs in `dev/active/`) and linked to its`` |
| `SKILL.md:214` | ``` `<managed-docs-root>/<short-id>-<slug>.md` (derive slug from title: ``` |
| `SKILL.md:219` | `   git add <managed-docs-root>/<short-id>-<slug>.md` |
| `SKILL.md:225` | `   jit doc add <id> <managed-docs-root>/<short-id>-<slug>.md \` |

`SKILL.md:214` derives the slug **from the title** — directly contrary to REQ-03/D-1. This is
the clearest single instance of the anti-pattern the epic exists to remove.

**jit-breakdown**
| path:line | string |
|---|---|
| `SKILL.md:23` | ``passed, and locate the linked `dev/active/<C>-breakdown.json`. If the approved`` |

**jit-parallel**
| path:line | string |
|---|---|
| `references/agent-prompt-template.md:31` | ``implementation plan to `<managed-docs-root>/<short-id>-<slug>.md`. The plan must include:`` |
| `references/agent-prompt-template.md:41` | `   - path: "<managed-docs-root>/<short-id>-<slug>.md"` |

**jit-project-lead** — already partly config-driven; these compose from `development_root`
rather than hardcoding `dev/`, so they are (a) only in the weaker sense of spelling out the
`<root>/active/<short-id>-<artifact>` shape:
| path:line | string |
|---|---|
| `references/progress-artifact.md:22` | ``- **Active root** — `<development_root>/active`, where `development_root` is the`` |
| `references/progress-artifact.md:26` | ``` `<development_root>/active/<strategic-container-short-id>-progress.json`. ``` |
| `references/progress-artifact.md:44` | `  "charter_path": "<permanent-root>/<short-id>-charter.md",` |
| `references/progress-artifact.md:138` | `1. **Resolve the progress path** from config (active root + anchor short id).` |
| `references/vision-charter.md:22` | ``- **Charter path** — `<permanent-root>/<strategic-container-short-id>-charter.md`.`` |
| `references/vision-charter.md:129` | `1. **Resolve the charter path** from config (permanent root + anchor short id),` |
| `references/templates/vision-charter.md:7` | `  - Write this to <permanent-root>/{{STRATEGIC_CONTAINER_SHORT_ID}}-charter.md,` |
| `references/standards-scan.md:42` | ``- **Every live markdown file** under `<development_root>/active` (default`` |
| `references/standards-sweep.md:84` | ``- **Active root** — `<development_root>/active`, the `[documentation]` key of the`` |
| `references/standards-sweep.md:86` | ``- **Report path** — `<development_root>/active/standards-sweep-report.md`. One`` |
| `SKILL.md:85` | ``` `<development_root>/active/`. It is the one-tier-up analogue of the execution ``` |

Two of these files already name the anti-pattern explicitly —
`references/standards-sweep.md:224` and `references/progress-artifact.md:167`:
*"Hardcoding `dev/` or `dev/active` instead of reading `development_root`."* REQ-04 generalizes
a rule the repo already believes in.

**Class (c) in `.agents/skills/`** — real working code, no change needed:
`jit-project-lead/scripts/standards-scan.sh:84` reads `development_root` out of config via
`gawk`; `jit-execution-lead/scripts/dispatch-worker-worktree.sh:15,21-23,37` and the
`worktree-mode.md` / `container-dispatch.md` / `worktree-dispatch-protocol.md` `<short-id>`
patterns are **git worktree directories, not doc artifacts** — explicitly out of REQ-11 scope.

**Class (b) in `.agents/skills/`** — eval and transcript material, not live instructions:
`jit-planning-lead/evals/evals.json:12,21,30`; `jit-execution-lead/evals/setup-test-repo.sh:161,163,194`;
`jit-execution-lead/evals/transcripts/*.completion-report.md:40,85`;
`jit-project-lead/evals/transcripts/routing-mode-{1,2,3,4}.completion-report.md:15,16`;
`jit-project-lead/scripts/test-standards-{scan,fix}.sh` fixtures.
These are **not** declared profile assets, so they are safely editable in isolation.

### `.jit/**`

| path:line | string | class |
|---|---|---|
| `.jit/templates.toml:28` | `  doc         = "dev/active/{container.short_id}-plan.md"` | **(a)** — hardcodes `dev/active/` instead of reading `development_root` |
| `.jit/templates.toml:29` | `description = "…Author the plan at {doc} and link it to this node once written."` | (a) |
| `.jit/config.toml:11-14` | `development_root`/`managed_paths`/`archive_root`/`permanent_paths` | (c) live dogfood config |
| `.jit/config.toml:202` | `source = "dev/vision/9db27a3a-charter.md"` | **(c)** — projection input; see claim 14 |
| `.jit/reference/content-standards.md:87,101` | `<short-id>/S0: …`, `epic:<short-id>` | (b) title/label conventions, not paths |
| `.jit/gates.toml:87,149` | `jit:<short-id>` commit attribution | (b) commit tag, not a path |
| `.jit/rules.toml`, `.jit/invariants.toml`, `.jit/schemas/*.json` | — | no hits |

### `crates/**` — production (c)

`config.rs:311-353` (config struct + default accessors) · `plan_doc.rs:115-124`
(`resolve_plan_doc_location`) · `template_expand.rs:429-484` (`InterpolationContext::interpolate`)
· `template.rs:526-540` (`render_template_document_path`) · `artifact_classifier.rs:1041-1046`
(`artifact_destination_root`), `:1054-1081` (`preferred_container_destination_root`),
`:32-99,611-617,833-841` (policy plumbing) · `commands/archive.rs:194-297,562-645` ·
`storage/artifact_planning.rs:32-90` · `storage/claim_coordinator.rs:13` (live cross-reference
to `dev/design/worktree-parallel-work.md`).

**(b) in `crates/**`:** `hierarchy_templates.rs:275-282` (commented scaffold — but see claim 2,
it is stale and *should* change) · `config.rs:2129` · `templates.rs:334,495,655,701,1125,1149,1231`
· `domain/event_catalog.rs:349` · `profile/dogfood.rs:213-214` (leak-detection deny-list) ·
test fixtures in `gate_execution.rs:934,950,965,1000`, `document/assets.rs:365,437-471`,
`output.rs:248-302`, `commands/gate_check.rs:3798,3813`,
`artifact_classifier.rs:1493-2134` (~100 occurrences after the `mod tests` boundary at `:1154`),
and template fixtures in `tests/fast_docs_templates/{template_apply_tests.rs:47,210,
templates_loader_tests.rs:43,template_binding_tests.rs:50,83,template_apply_atomicity_tests.rs:326,655,678}`,
`tests/cli_repo_workflow/{template_binding_cli_tests.rs:57,apply_cli_tests.rs:43}`.

### `profiles/**` — the coupled twin (claim 13 / D-11)

`profiles/jit-dogfood/manifest.toml:334-554` declares 55 skill assets;
`profiles/jit-dogfood/manifest.toml:289` carries `doc = "dev/active/{container.short_id}-plan.md"`.
All 31 `dev/active` occurrences under `profiles/` mirror the `.agents/skills` rows above.

### `docs/**`, `scripts/**`, root — remaining consumers

| path:line | string | class |
|---|---|---|
| `docs/how-to/adopt-planning-bracket.md:82` | `doc = "dev/active/{container.id}-plan.md"   # where P's plan doc lives` | (a) adopter-facing |
| `docs/how-to/adopt-planning-bracket.md:267,269` | ``- `dev/active/<C-id>-plan.md`: …`` / ``- `dev/active/<C-id>-breakdown.json`: …`` | **(a)** the only adopter-facing statement of the convention |
| `docs/how-to/adopt-planning-bracket.md:277,278,281` | `dev/active/<C-id>-breakdown.json` / `--plan dev/active/<C-id>-plan.md` | (a) |
| `docs/examples/sdd/templates.toml:25`, `docs/examples/research/templates.toml:27` | `doc = "dev/active/{container.id}-plan.md"` | (a) shipped example configs |
| `docs/reference/configuration.md:50,51` | `managed_paths = [...]` / `archive_root = "dev/archive"` | (b) reference example |
| `docs/reference/example-config.toml:21,24` | same | (b) |
| `docs/reference/cli-commands.md:194,195,200` | `jit archive document dev/active/design.md [--json\|--execute]` | (b) |
| `docs/how-to/custom-gates.md:148` | `{ "path": "dev/active/my-plan.md", … }` | (b) |
| `scripts/benchmark-session-cost.sh:38,51,74` | `OUT_DIR="${SESSION_BENCH_OUT_DIR:-dev/studies/perf}"` | **(a)** writes into a managed area from a hardcoded path |
| `scripts/benchmark-session-cost-selftest.sh:89` | `template="$repo_root/dev/studies/perf/session-cost-27ffbd2d.json"` | (c) live file |
| `scripts/rust-build-budget.sh:34,142,165`, `scripts/cargo-ci.sh:88`, `CHANGELOG.md:43,85`, `dev/TESTING.md:255` | `dev/active/73482aa1-rust-build-efficiency.md` | **(c)** live cross-references to one existing file — **6 references that break if 73482aa1 is archived under REQ-09** |
| `scripts/docs-check-citations.sh:29` | ``(`dev/active/<plan>.md` §2, M3)`` | (b) |
| `CLAUDE.md:100`, `AGENTS.md:100` | `dev/vision/9db27a3a-charter.md` | (c) projection source |
| `dev/authoring-conventions.md:68,69` | `mv dev/active/authentication-design.md dev/archive/features/` | (b) **stale**, contradicts `:279-280` |
| `dev/authoring-conventions.md:245,268,271,290,310,322-323,332,344-345` | `dev/active/...` samples | (b) |
| `web/src`, `mcp-server/src` | — | **no hits** |

**73482aa1 is the sharpest REQ-08/REQ-13 test case.** Issue `73482aa1`'s artifact is cited from
6 places, of which 3 are **executable shell** (`rust-build-budget.sh:34,142,165` and
`cargo-ci.sh:88` are comments, but the budget script's *error messages* at `:142` and `:165`
print the path to the operator). If 73482aa1 is terminal, REQ-09 archives the file and every
one of those citations breaks — with no `jit validate` failure, because none is a document
link record. This is exactly the class REQ-08 must warn about, and it is already present.

---

## Documentation inventory (REQ-12)

**Canonical-home candidate: `docs/reference/configuration.md` §`[documentation]` (`:45-88`).**
Covers config keys (`:48-52`) and move-vs-copy semantics (`:55-58`, *"Selected documents in
`permanent_paths` are copied to the mirror while their source remains in place; 'permanent'
prevents source deletion, not mirror publication"*), the three-state policy status (`:74-82`),
and already carries the dogfooding disclaimer (`:84-87`). It is what `README.md:241` points at.
**Missing:** the path convention entirely, and the command mechanics.

Competing statements to collapse into pointers:

| Location | What it duplicates | Class |
|---|---|---|
| `docs/reference/example-config.toml:15-27` | keys + semantics, in comments | adopter |
| `docs/reference/glossary.md:36,38` | `archive_root` / `jit archive` definitions | adopter |
| `docs/reference/cli-commands.md:267-272` | policy-status framing (`:188-357` command mechanics is legitimately its own home) | adopter |
| `docs/how-to/adopt-planning-bracket.md:267-281` | the only adopter statement of the path convention | adopter |
| `README.md:200-204,241` | teaser bullet + usage | adopter |
| **`dev/index.md:19-104`** | **a whole conflicting lifecycle narrative** — retention windows, category subfolders, per-area permanence claims the config does not back | contributor |
| `dev/authoring-conventions.md:5-11,39-105,261-284` | area split, asset patterns, move/copy — and self-contradicts at `:65-71` vs `:279-280` | contributor |

No generated projection currently renders any of these facts. The `<!-- jit:*:begin -->` regions
in `AGENTS.md`, `CLAUDE.md`, `README.md`, `docs/reference/{configuration,cli-commands,
rules-and-gates,gate-presets,storage-format,example-config.toml}`, and
`docs/concepts/guarantees.md` project charter/invariants/rules/gates only. So REQ-12 has no
existing projection to extend and must either pick a prose home or add a projection.

---

## Primitive verification

| Property | Verdict | Evidence |
|---|---|---|
| `@/inv/atomic-writes` — temp-file + atomic-rename for replacement | **CONFIRMED** | `crates/jit/src/storage/atomic_write.rs:111` (`fs::rename(&tmp, path)`) |
| `@/inv/atomic-writes` — verified staging + no-replace publication | **CONFIRMED** | `crates/jit/src/storage/external_publish.rs:65,107,181` and `storage/transaction_staging.rs:8`, all `options.write(true).create_new(true)`. The archive marker write uses `expected: ExpectedPreimage::Absent` (`repository_state/archive.rs:132`), so an occupied destination is never overwritten |
| Archive execution is one recoverable transaction | **CONFIRMED** | `finalize_archive_execution`, `crates/jit/src/repository_state/archive.rs:19-20` — doc comment *"Close a captured archive plan into one recoverable repository transaction."* Emits only `RepositoryAction::{WriteFile,DeleteFile,CreateDirectory}` (`:83,129,258,397`) |
| Deletions are identity-guarded (no lost-update on archive) | **CONFIRMED** | `PendingDeletion { source, content_identity }` (`artifact_plan.rs:365-372`) — *"Identity that must still match immediately before removal"*; populated only for `Move` (`artifact_classifier.rs:768-779`) |
| `@/inv/derived-state-coherence` — archival relinks from one final view | **CONFIRMED** | `reference_changes` collected across all artifacts at `repository_state/archive.rs:172` and applied within the same transaction that publishes and deletes |
| `jit validate` resolves document references against the closed image | **CONFIRMED** | `crates/jit/src/validation/repository.rs:458` — *"Document references resolve against boundary-acquired evidence in the closed…"*. Live run: `{"valid": true, "error_count": 0, "warning_count": 0}` — clean REQ-09 baseline |
| **`jit validate` catches in-content markdown path citations** | **CONTRADICTED** | No such check. `jit validate` resolves **document link records** only. The 6 citations of `73482aa1-rust-build-efficiency.md` in shell scripts and CHANGELOG are invisible to it. This is precisely why REQ-08 needs a warning rather than relying on validation |
| Execution rewrites no document content | **CONFIRMED (already true)** | No content-mutating action exists in the archive path; the only synthesized bytes are the `.jit-container` marker (`repository_state/archive.rs:123`) |
| `cargo test` green at HEAD | **CONTRADICTED** | `profile::dogfood::tests::test_live_assets_match_every_declared_source_tree_consumer` FAILS at `crates/jit/src/profile/dogfood.rs:453`. See claim 13 |

---

## Architecture fit

Layer assignment per REQ, respecting CLAUDE.md's mandatory boundaries:

| REQ | Layer | Notes |
|---|---|---|
| REQ-01 | **config + init scaffold** | `crates/jit/src/config.rs:331-353` accessors and `crates/jit/src/hierarchy_templates.rs:275-282` scaffold. No domain change. See Q2 — the scaffold is the part that actually changes shipped behaviour |
| REQ-02 | **no new code** | Falls out of REQ-01. Acceptance is a CLI-integration assertion in `tests/cli_repo_workflow/archive_preview_cli_tests.rs` |
| REQ-03 | **domain, pure** | New pure fn beside `preferred_container_destination_root` (`artifact_classifier.rs:1054`). Reuse `archive_container_slug` (`:1084`) and `get_membership_namespace` (`type_taxonomy.rs:215`). I/O-free |
| REQ-04 | **cli.rs + commands/** | Arg parsing and rendering in `cli.rs`/`output.rs`; orchestration in `commands/`; the derivation itself stays in the REQ-03 domain fn |
| REQ-05 | **config/templates + skill prose** | `.jit/templates.toml:28` and `profiles/jit-dogfood/manifest.toml:289` are configuration, not code — no engine change needed for the template path |
| REQ-06 | **commands/ + domain** | Pure predicate in domain ("is this artifact inside its owning issue's canonical directory"); traversal and rendering outside |
| REQ-07 | **domain (code) + repo data (rename)** | Delete the title fallback at `artifact_classifier.rs:1076`; the rename is a one-directory `git mv` |
| REQ-08 | **domain (new `WarningCode`) + storage (evidence)** | Warning construction is pure; the content scan that finds citations needs file bytes, so evidence acquisition belongs at the storage/planning boundary (`storage/artifact_planning.rs`), matching how existing evidence is gathered |
| REQ-09 | **repo data only** | `jit archive container --execute` runs |
| REQ-10 | **repo data only** | Dispositions |
| REQ-11 | **`.agents/skills/` + `profiles/` prose** | Coupled — see Q1 |
| REQ-12 | **docs prose** | — |
| REQ-13 | **domain, already satisfied** | Legacy tolerance already exists (`artifact_classifier.rs:241-249`); needs regression tests, not new behaviour |

---

## Invariant check

- **`@/inv/domain-agnostic`** — *the principal risk.* The literals `"dev/active"`,
  `"dev/studies"`, `"dev/sessions"`, `"dev/archive"`, `"docs/"` already sit in engine code at
  `crates/jit/src/config.rs:331-353`. REQ-01 tempts a plan to add `"dev/presentations"` and
  `"dev/architecture"` there, deepening a `dev/`-shaped assumption in the engine.
  The invariant's sanctioned exception is **only** the planning-bracket preset trio, so this
  would not be covered. The defensible reading: these are *fallbacks* for an unauthored table
  and are never operative policy (`PolicyStatus::from_documentation`,
  `artifact_plan.rs:85-97`, ignores them) — the shipped classification properly belongs in the
  `jit init` scaffold (`hierarchy_templates.rs:275-282`), which is **configuration jit writes
  for the adopter**, not engine logic. **Recommend the plan put REQ-01's classification in the
  scaffold and treat the `config.rs` accessors as a compatibility fallback.** That satisfies
  REQ-01, achieves REQ-02, and moves *toward* the invariant instead of away from it.
  REQ-03/04/06 stay clean as long as the area root comes from `development_root` and the
  namespace from `label_associations` — both already config-driven.
- **`@/inv/single-source-prose`** — REQ-12 is a direct application. The claim-2 table shows the
  defaults already duplicated in 5 places with the `jit init` scaffold **already stale** — a
  live instance of the staleness defect the invariant names. Any REQ-01 edit must sweep all of
  them or projection must replace the copies. Note this report's own counts (172/126/95/23) are
  volatile facts; the plan should derive rather than re-copy them.
- **`@/inv/derived-state-coherence`** — preserved. REQ-09 runs through the existing transactional
  executor, which is the point of D-10.
- **`@/inv/atomic-writes`** — preserved; no REQ introduces a new write path. The REQ-07 rename is
  a `git mv`, outside the invariant's scope (it governs jit's own file replacement).
- **`@/inv/bounded-rust-build-footprint`** — headroom is 2 targets (10 of 12). Every REQ maps to
  an existing suite (design surface E), so no new target is needed. A plan that adds one burns
  half the remaining headroom for no reason.
- **`@/inv/semantic-test-assertions` / `@/inv/shared-test-contracts`** — the `WarningCode::ALL`
  golden test (`tests/fast_docs_templates/artifact_plan_model_tests.rs:333`) is the sanctioned
  single canonical suite for the stable external contract, so REQ-08's new code belongs there
  and nowhere else.

---

## Open questions for the owner

**Q1 (blocking REQ-11) — REQ-11 and the profile-asset non-goal are mutually exclusive.**
55 `.agents/skills` files are byte-equality-asserted profile assets
(`crates/jit/src/profile/dogfood.rs:437-458`); every REQ-11 target is one of them. Editing the
root copy alone fails `cargo test`. Options: **(a)** bring `profiles/jit-dogfood/assets/live/`
into scope and drop the non-goal — mechanically forced, smallest real change, keeps the package
shippable; **(b)** keep the non-goal and restrict REQ-11 to non-asset files — but that excludes
every SKILL.md, making REQ-11 nearly vacuous; **(c)** relax the drift test — contradicts D-5's
dogfooding boundary. **Recommend (a).** Note the choice is partly pre-made: HEAD is already
red for this exact reason and must be repaired regardless.

**Q2 (blocking REQ-01/REQ-02) — where does the shipped classification live?**
`PolicyStatus::from_documentation` (`artifact_plan.rs:85-97`) ignores accessor defaults, so
editing `config.rs:331-353` satisfies REQ-01's letter while leaving REQ-02 unreachable for a
default-initialized repo. The classification must reach the **`jit init` scaffold**
(`hierarchy_templates.rs:275-282`), which is currently commented out, missing `dev/studies`,
and advertising two retired keys. Confirm REQ-01 means "an initialized repository archives
these areas out of the box" — and if so, whether the scaffold should be **uncommented** (a
behaviour change for every new repo: policy becomes `configured` rather than `unconfigured`).

**Q3 (REQ-03) — what replaces the title fallback when membership is ambiguous?**
Two ambiguity cases exist and both currently fall through to the title
(`artifact_classifier.rs:1064`, `:1074`). Recommend both collapse to the bare
`<area>/<short-id>` directory — no new tie-breaking policy, and it matches the `legacy_root`
shape already supported (`:241-249`). Confirm, or specify a tie-break.

**Q4 (REQ-08) — what counts as an "in-content path citation"?**
The planner reads structured document edges, not arbitrary prose. Candidate definitions: any
substring matching a moving artifact's path in a **markdown link target** only; or in any text
including code fences and shell comments. The 73482aa1 case argues for the broad reading — 3 of
its 6 citations are shell comments and operator-facing error strings, invisible to a
link-only scan. Also confirm the scan's scope: repository-wide, or only files under
`development_root`? (Repository-wide is needed to catch `scripts/` and `CHANGELOG.md`.)
Note the cost: a new `WarningCode` variant extends a closed schema-v1 contract
(`artifact_plan.rs:494-516`, golden-tested at `artifact_plan_model_tests.rs:333`).

**Q5 (REQ-07) — the marker clause is unsatisfiable as written.**
The `.jit-container` marker holds only the full container UUID
(`repository_state/archive.rs:123`), so there is nothing to "update to match" a renamed
directory. Reword to assert the marker **remains valid and continues to resolve the renamed
directory**, and note that REQ-07's real scope is **one** directory
(`dev/archive/a9b5dd08-use-strategic-label-slugs-in-archive-directory-n` →
`dev/archive/a9b5dd08`). Also confirm the plan should record that REQ-07 **reverses a shipped,
reviewed decision** (a9b5dd08's own REQ-05 introduced this fallback).

**Q6 (REQ-10) — scope is 3x the implied size.**
Read literally, REQ-10 covers `dev/active` (23 unprefixed) **plus** `dev/studies` (18 of 19
unprefixed) **plus** `dev/sessions` (all 30 unprefixed) = **71 files** needing a recorded
disposition. Confirm that is intended, or narrow the criterion. At least 5 of the 23 in
`dev/active` already carry supersession banners, which shortens the work but not the count.

**Q7 (REQ-09/REQ-13) — the `73482aa1` citation cluster.**
`dev/active/73482aa1-rust-build-efficiency.md` is cited from `scripts/rust-build-budget.sh:34,142,165`,
`scripts/cargo-ci.sh:88`, `CHANGELOG.md:43,85`, and `dev/TESTING.md:255`. If 73482aa1 is
terminal, REQ-09 archives it and all 6 break silently — `jit validate` will still pass, because
none is a document link record. Decide now: exempt it, repoint the citations by hand, or accept
the breakage as REQ-08 warning output. REQ-13 ("the cleanup breaks no document reference") is
ambiguous about whether a shell-comment citation is a "document reference".

**Q8 (claim 14) — `dev/plans` is not mixed; `dev/vision` holds projection input.**
All 6 `dev/plans` files are terminal-owned (4 of them use a **suffix**, not prefix, short id —
a prefix-based resolver will not recognize them). `dev/vision/9db27a3a-charter.md` is owned by a
**live** issue and is the projection source named in `.jit/config.toml:202` and `CLAUDE.md:100`.
Recommend `dev/plans` → managed (archivable wholesale) and `dev/vision` → permanent. D-3's
"per-file call" is then needed for `dev/vision` only.

---
---

# REQ-14 and follow-up items (R1–R5)

Measured at HEAD `dfb066fa` (`jit:8e071e18 amend epic criteria and apply plan bracket`).
Only `dev/active/8e071e18-investigation.md` is untracked; nothing else is dirty.

**Where each item is answered:**

| Item | Section |
|---|---|
| R1 — is eligibility all-or-nothing? + real previews | [R1](#r1--one-blocker-makes-the-whole-plan-ineligible) |
| R2 — cleanest mechanism for REQ-14, (a) vs (b), call sites | [R2](#r2--cleanest-mechanism-for-req-14) |
| R3 — outside-subtree owner ⇒ `Copy` | [R3](#r3--outside-subtree-owner-yields-copy) |
| R4 — can `permanent_paths` hold a file path? | [R4](#r4--permanent_paths-file-entries-work) |
| R5 — citation clusters for the three newly-managed areas | [Cost of the D-13 reclassification](#cost-of-the-d-13-reclassification--citations-that-break) |
| REQ-14 premise, blast radius, layer assignment | [Headline](#headline-a-blocker-class-no-criterion-covers), [Pinned decisions](#pinned-decisions--premise-verification), [Layer assignment](#layer-assignment-for-req-14) |

**Still true after the branch moved:** the profile-asset drift of claim 13 persists
(`.agents/skills/jit-execution-lead/SKILL.md` still differs from its
`profiles/jit-dogfood/assets/live/` twin), so `cargo test` remains red. Q1 and the claim-13
finding stand unchanged.

## Headline: a blocker class no criterion covers

`jit archive candidates --json` evaluates all **125** effectively-terminal containers:
**85 eligible, 40 ineligible.** Two independent blocker populations produce those 40:

| Population | Level | Instances | Containers affected | Covered by a criterion? |
|---|---|---|---|---|
| `unmanaged-selected-root` | target | 101 | 29 | **yes** — D-13 + D-14 clear all 101 |
| `unsupported-artifact-type` | artifact | 376 | 16 | **no** |

Of the 40 ineligible containers: 24 blocked by paths only, **11 blocked by
`unsupported-artifact-type` only**, 5 blocked by both. So **16 containers stay ineligible after
every REQ in this container is satisfied**, and 11 of those have no path-classification problem
at all — nothing in the epic touches them.

This matters because REQ-02, REQ-09 and REQ-14 all require an *eligible* plan, and eligibility
is all-or-nothing (see R1). REQ-09 in particular ("every `dev/active` artifact owned by a
terminal issue is archived through `jit archive container --execute`") cannot be satisfied for
those 16 containers as the criteria stand.

**Root cause, pinned.** `dev/index.md` links its sibling directories with trailing-slash
markdown links — `dev/index.md:21` `### 🚧 [active/](active/) - Active Development`, and the
same shape at `:28,31,34,45,52,59,62,65,72,75` (11 links). `docs/index.md` does the same for the
`docs/` subtree, and `dev/benchmarks/rust-build-efficiency/report.md` links a `raw` directory.
Artifact discovery follows those edges, resolves each target to
`ArtifactLocation::Unsupported` (a directory is not a regular file), and
`crates/jit/src/domain/artifact_classifier.rs:712-716` pushes
`BlockerCode::UnsupportedArtifactType` with `action = Block`.

The 25 distinct blocked directory targets: `dev/{active,architecture,archive,design,eval,
experiments,plans,presentations,sessions,studies,vision}`, `docs/{concepts,how-to,reference,
tutorials,examples}`, `docs/examples/{bug-repro,cross-epic,fresh-evidence,nyquist,
release-checklist,research,sdd}`, `mcp-server`, and
`dev/benchmarks/rust-build-efficiency/raw`. The first 24 appear in 15 candidate plans each; the
`raw` directory in 16.

Neither `dev/index.md` nor `docs/index.md` is linked to any issue — both are reached
transitively as `embedded` artifacts (`dev/index.md` provenance `['embedded']`, evidence
`['outside-owner','unmanaged-path']`, action `copy`) from `docs/index.md:82,100`
(`[Development Documentation](../dev/index.md)`). **D-13 making `dev/index.md` a permanent file
entry does not help** — the block lands on its link *targets*, not on the file.

Three remediations, none currently in scope. **(i)** Treat a directory-typed embedded edge
target as not-an-artifact and skip it rather than blocking — a pure-domain change at
`artifact_classifier.rs:712-716`, fixes all 16 containers at once, and is arguably the correct
semantics since a directory link is a navigation aid, not a document. **(ii)** Rewrite the
trailing-slash directory links in `dev/index.md` and `docs/index.md` to point at a file — repo
data only, but `docs/index.md` is adopter-facing and the links are legitimate markdown.
**(iii)** Accept 16 containers as unarchivable and narrow REQ-09. **Recommend (i)**; see **Q9**.

## R1 — one blocker makes the whole plan ineligible

**Confirmed: all-or-nothing, and it spans both blocker levels.**
`crates/jit/src/domain/artifact_plan.rs:997-1001`:

```rust
let eligible = policy_status == PolicyStatus::Configured
    && blockers.is_empty()
    && artifacts.iter().all(|artifact| {
        artifact.action != ArtifactAction::Block && artifact.blockers.is_empty()
    });
```

`blockers` is the **target-level** vector that `selected_destination_roots` appends to
(`artifact_classifier.rs:849-852`), so a single `UnmanagedSelectedRoot` anywhere in the subtree
sets `eligible: false` for the entire container. There is no partial-plan or per-entry
eligibility. The third conjunct is why the `unsupported-artifact-type` population above is
independently fatal.

**Real previews.** The 13 owner issues of the 29 non-`dev/`, non-`docs/` linked documents sit in
three epics — `epic:jit-project-lead` → **f2532a2d**, `epic:core-maintenance` → **6eb585bc**,
`epic:docs-exhaustive-audit` → **2d109173** (all `done`). Previews only, no `--execute`:

```
jit archive container f2532a2d --json
  eligible: false | policy: configured | count: 173
  destination_root: dev/archive/f2532a2d-jit-project-lead
  action_counts: {move:11, copy:90, retain:47, block:25, already_archived:1, pending_deletions:11}
  TARGET blockers: 26 (all unmanaged-selected-root)
    .agents/skills/jit-execution-lead/trigger_eval.json
    .agents/skills/jit-execution-lead/trigger_eval_results.json
    .agents/skills/jit-planning-lead/evals/evals.json
    .agents/skills/jit-planning-lead/evals/setup-test-repo.sh
    .agents/skills/jit-planning-lead/trigger_eval.json
    .agents/skills/jit-planning-lead/trigger_eval_results.json
    .agents/skills/jit-project-lead/evals/evals.json
    .agents/skills/jit-project-lead/evals/results.md
    .agents/skills/jit-project-lead/references/container-dispatch.md
    .agents/skills/jit-project-lead/references/mode-routing.md
    .agents/skills/jit-project-lead/references/parent-escalation.md
    .agents/skills/jit-project-lead/references/progress-artifact.md
    .agents/skills/jit-project-lead/references/standards-fix.md
    .agents/skills/jit-project-lead/references/standards-scan.md
    .agents/skills/jit-project-lead/references/standards-sweep.md
    .agents/skills/jit-project-lead/references/templates/vision-charter.md
    .agents/skills/jit-project-lead/references/vision-charter.md
    .agents/skills/jit-project-lead/scripts/standards-fix.sh
    .agents/skills/jit-project-lead/scripts/standards-scan.sh
    .agents/skills/jit-project-lead/scripts/test-standards-scan.sh
    .agents/skills/jit-project-lead/trigger_eval.json
    .agents/skills/jit-project-lead/trigger_eval_results.json
    dev/eval/lead-skills-eval-baseline.md
    dev/eval/skill-eval-adjudication.md
    dev/eval/skill-triggers/README.md
    dev/eval/skill-triggers/run_trigger_eval.py

jit archive container 6eb585bc --json
  eligible: false | policy: configured | count: 140
  destination_root: dev/archive/6eb585bc-core-maintenance
  action_counts: {move:15, copy:90, retain:10, block:25, already_archived:6, pending_deletions:15}
  TARGET blockers: 6 (all unmanaged-selected-root)
    CHANGELOG.md
    crates/jit/tests/cli_issue/verb_hint_tests.rs
    dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json
    dev/benchmarks/rust-build-efficiency/optimized.json
    dev/benchmarks/rust-build-efficiency/report.md
    dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py

jit archive container 2d109173 --json
  eligible: false | policy: configured | count: 139
  destination_root: dev/archive/2d109173-docs-exhaustive-audit
  action_counts: {move:16, copy:90, retain:8, block:25, already_archived:6, pending_deletions:16}
  TARGET blockers: 1 (unmanaged-selected-root)
    scripts/docs-check-selftest.sh
```

All three carry `block: 25` — the `unsupported-artifact-type` population, which the path
decisions do not touch.

**Document-link census confirms the team lead's inventory exactly.** 312 linked documents across
703 issues: `dev/` 267, `.agents/` 25, `docs/` 16, repository root 2, `scripts/` 1, `crates/` 1.
The 25 `.agents/skills/**` links belong to 10 `done` issues (6662f738, 41aa1b75, 02a2bbb9,
e7d41080, 6c5f70ad, 634b2382, eff48a6e, 0b7e864d, 66aeee5f, 206bd960); the remaining 4
non-`dev`/non-`docs` links belong to 3 more `done` issues (b1586c0d and d0f88ee2 → `CHANGELOG.md`,
d0f88ee2 → `crates/jit/tests/cli_issue/verb_hint_tests.rs`, 99f4a2b4 →
`scripts/docs-check-selftest.sh`). 13 owning issues, all terminal.

## R2 — cleanest mechanism for REQ-14

**Recommend (a), scoped exactly as D-14 already words it** — "outside the development root" —
rather than the blanket "matches no configured area" the question poses. That distinction is
what makes (a) safe.

**Why not blanket (a).** Dropping the blocker for *any* unmatched path silently converts a real
misconfiguration signal into silent copying. An adopter who typos `managed_paths = ["dev/activ"]`
would get every artifact quietly copied to the mirror with no diagnostic, because
`PolicyStatus` only catches a *wholly* absent or incomplete table
(`artifact_plan.rs:85-97`), not a wrong path. Scoping the fallback to "outside
`development_root`" preserves the blocker for exactly the case it was designed to catch — an
unclassified area *inside* the development root — which after D-13 can only arise from a new
area or a typo.

**Why not (b), explicit `permanent_paths` entries.** It is a maintenance treadmill: today the
list would need `.agents/`, `CHANGELOG.md`, `crates/`, `scripts/`, and any future linked root;
the first new link to an unlisted root re-blocks the plan with no warning until someone previews
it. Worse, `contains_path` is a prefix matcher (R4), so `permanent_paths = ["crates/"]` makes
the *entire* Rust source tree permanent — the same blast radius as (a) but achieved by
enumeration and therefore silently incomplete.

**Call sites that rely on the current blocking behaviour**, and what changes:

| Site | Current role | Under scoped (a) |
|---|---|---|
| `artifact_classifier.rs:848-853` | pushes the blocker **and** `return None`, excluding the root from `selected_destination_roots` | must both stop blocking and start returning the root, else the artifact gets no destination and silently retains |
| `artifact_classifier.rs:647` — `unmanaged_embedded = embedded && !managed && !permanent && !archived_source` | feeds `EvidenceCode::UnmanagedPath` (`:655`) | narrows as paths become permanent; fewer `unmanaged-path` evidence flags |
| `artifact_classifier.rs:663-670` — `needs_source` disjunction | `unmanaged_embedded` is one disjunct forcing `Copy` | **already delivers D-14's semantics** — see below |
| `artifact_classifier.rs:1779` (unit test) | asserts the blocker fires | must be re-scoped to an in-development-root path |
| `crates/jit/tests/cli_repo_workflow/archive_preview_cli_tests.rs:702` | asserts the blocker's absence | already the right shape for REQ-14's acceptance |
| `artifact_plan.rs:427-441` — `BlockerCode::ALL` (13 codes), golden-tested at `tests/fast_docs_templates/artifact_plan_model_tests.rs:315` | schema-v1 wire vocabulary | `UnmanagedSelectedRoot` **must stay in `ALL`** even if it becomes rarer; removing it is a schema break |

**The strongest argument for (a): the machinery already exists.** An *embedded* artifact outside
every configured area is already handled exactly as D-14 wants — `unmanaged_embedded` (`:647`)
forces `needs_source` (`:668`), which yields `Copy` (`:672`), and `Copy` schedules no
`PendingDeletion` (`:768-771`). Live proof in the 2d109173 preview: `dev/index.md`, an embedded
artifact under no managed or permanent root, is planned `action: copy` with evidence
`['outside-owner','unmanaged-path']` — copied to the mirror, source retained. REQ-14 therefore
extends an existing, already-correct behaviour from embedded artifacts to *explicitly linked*
ones, rather than inventing new semantics. That is a small, well-precedented change.

REQ-14's second half — "executing such a plan relocates no file outside the development root" —
follows automatically from `Copy`, and is already guaranteed by `:768-771` restricting
`PendingDeletion` to `ArtifactAction::Move`. It needs a regression test, not an implementation.

## R3 — outside-subtree owner yields Copy

**Confirmed.** `crates/jit/src/domain/artifact_classifier.rs:633-640`:

```rust
let direct_outside_owner = matches!(target, PlanTarget::Container { .. })
    && owners.iter().any(|owner| !owner.inside_subtree);
let outside_owner = direct_outside_owner
    || repository_embedded.iter().any(|owner| embedded_owner_is_outside(owner, target));
```

`outside_owner` is then one of the eight disjuncts of `needs_source` (`:663-670`), and
`(wants_destination = true, needs_source = true)` maps to `ArtifactAction::Copy` (`:672`).
Because `Copy` produces no `PendingDeletion` (`:768-771`), the source is retained.

Two qualifications the plan should not lose: the check is **container-targets only**
(`matches!(target, PlanTarget::Container { .. })`), and `Copy` requires
`wants_destination` — an artifact nobody wants a destination for is `Retain` (`:674`)
regardless of ownership. Live confirmation in the claim-4 preview:
`dev/architecture/repository-state-materialization.md` carries evidence `['outside-owner']`
(owner 39e1c091 sits outside the 1cc809de subtree).

## R4 — `permanent_paths` file entries work

**Confirmed: an exact file path is a valid entry.** `crates/jit/src/domain/artifact_classifier.rs:1130-1138`:

```rust
pub fn contains_path(root: &str, candidate: &str) -> bool {
    let root = normalize_artifact_path(root);
    let candidate = normalize_artifact_path(candidate);
    !root.is_empty()
        && (candidate == root
            || candidate.strip_prefix(&root).is_some_and(|suffix| suffix.starts_with('/')))
}
```

The `candidate == root` disjunct matches an exact file path, and the `strip_prefix` disjunct
requires the next character to be `/`, so a file entry matches that file and nothing else. So
D-13's three explicit file entries — `dev/index.md`, `dev/TESTING.md`,
`dev/authoring-conventions.md` — are individually classifiable. The prefix arm also cannot
partially match a sibling: `dev/index.md` will not match `dev/index.md.bak` (the suffix would be
`.bak`, not starting with `/`).

Entries are normalized and de-duplicated through `normalized_paths` (`:1140-1148`), which drops
empties, so a trailing slash is harmless — `"docs/"` and `"docs"` behave identically.

**D-13's rejection of a bare `dev/` entry is correct.** `contains_path("dev", "dev/active/x.md")`
strips to `/active/x.md`, which starts with `/`, so it matches — a bare `dev/` permanent entry
would indeed make every managed area permanent and silently defeat move-on-archive. The premise
holds exactly as D-13 states it.

## Pinned decisions — premise verification

| Decision | Premise | Verdict |
|---|---|---|
| **D-3** adoption independent of archival classification | that a managed area can stay flat and still archive | **holds** — nothing in the classifier consults naming; `contains_path` matches paths only, and destination is the mirrored source path |
| **D-13** areas classified as listed | that all 12 areas + 3 root files are covered | **holds** — the 8 previously unclassified areas of claim 3 are exactly the 8 D-13 assigns |
| **D-13** bare `dev/` rejected because matching is by prefix | prefix matching | **holds** — R4 |
| **D-13** `dev/vision` permanent because the charter is a configured item-kind source | that `.jit/config.toml` names it | **holds** — `.jit/config.toml:202` `source = "dev/vision/9db27a3a-charter.md"`; also cited at `CLAUDE.md:100` and `AGENTS.md:100`. Additionally its owner 9db27a3a is `backlog` (live), so it would not be selected anyway |
| **D-13** `dev/benchmarks` managed because the generator recreates its output directory | that the generator does so | **holds** — `scripts/benchmark-rust-build.sh:98` `OUT_DIR="${BENCH_OUT_DIR:-dev/benchmarks/rust-build-efficiency}"`, an overridable default the script recreates |
| **D-13** `dev/design`, `dev/benchmarks`, `dev/experiments` are revision-specific work products of terminal issues | that no live issue owns them | **holds** — `dev/design` 17 links / 17 terminal, `dev/benchmarks` 5/5, `dev/experiments` 1/1, `dev/eval` 4/4, `dev/presentations` 4/4, `dev/plans` 6/6, `dev/architecture` 1/1. Zero live owners in every area D-13 reclassifies except `dev/vision` (1 live), which D-13 makes permanent |
| **D-14** linked artifacts outside the development root | 29 such links, 13 terminal owners | **holds** — census above matches the team lead's count exactly |
| **D-15** `dev/sessions` and `dev/studies` are homogeneous dated records | that a blanket call is safe | **holds for `dev/sessions`** (30 files, 0 prefixed, uniform `session-<date>-<topic>.md`); **partially for `dev/studies`** — 19 files, 1 prefixed (`cdc840ad-audit-2026-07-23.md`), and it holds a `perf/` subdirectory with 4 files and 2 documents that are live cross-reference targets (`dev/studies/documentation-organization-strategy.md` is cited by `dev/authoring-conventions.md`; `scripts/benchmark-session-cost-selftest.sh:89` reads `dev/studies/perf/session-cost-27ffbd2d.json` at runtime). A blanket reclassification must not move that JSON — see Q10 |

**D-13 + D-14 are jointly complete against the measured blocker set.** All 101
`unmanaged-selected-root` blockers across the 125 candidates fall into areas the two decisions
classify, with none left over:

| Blocked area | Blockers | Covered by |
|---|---|---|
| `.agents/**` | 43 | D-14 |
| `dev/design` | 23 | D-13 managed |
| `dev/benchmarks` | 8 | D-13 managed |
| `dev/eval` | 8 | D-13 permanent |
| `dev/plans` | 7 | D-13 managed |
| `dev/presentations` | 6 | D-13 managed |
| `dev/architecture` | 2 | D-13 permanent |
| `dev/experiments` | 1 | D-13 managed |
| `scripts/**` | 1 | D-14 |
| `crates/**` | 1 | D-14 |
| repository root (`CHANGELOG.md`) | 1 | D-14 |
| **total** | **101** | — |

This is a clean result: the classification decisions are sufficient for every path blocker that
actually occurs. What they do not reach is the `unsupported-artifact-type` population.

## Cost of the D-13 reclassification — citations that break

`dev/design` and `dev/benchmarks` become **managed**, so their linked artifacts **move** on the
next container archive. Every citation below is in-content prose or code — invisible to
`jit validate`, which resolves document link records only. This is the REQ-08 warning surface
and the REQ-09 repoint list.

### `dev/design/` — live citations (dated historical records excluded)

| path:line | citation | Kind |
|---|---|---|
| `crates/jit/src/storage/claim_coordinator.rs:13` | ``//! See design doc: `dev/design/worktree-parallel-work.md` - "Claim Acquisition Algorithm"`` | **production Rust module doc comment** |
| `crates/jit/tests/cli_repo_workflow/config_get_tests.rs:369` | `// (docs/reference/configuration.md, dev/design/worktree-parallel-work.md)` | test comment |
| `docs/tutorials/parallel-work-worktrees.md:257` | ``- Design document: `dev/design/worktree-parallel-work.md``` | **adopter-facing** |
| `docs/how-to/multi-agent-coordination.md:472` | ``- Design document: `dev/design/worktree-parallel-work.md``` | **adopter-facing** |
| `dev/studies/ai-tool-worktree-compatibility.md:81` | ``The JIT architecture design (`dev/design/worktree-parallel-work.md`) explicitly states:`` | contributor study |
| `dev/studies/ai-tool-worktree-compatibility.md:210` | ``` `dev/design/worktree-parallel-work.md` and `docs/tutorials/parallel-work-worktrees.md` should: ``` | contributor study |
| `dev/studies/ai-tool-worktree-compatibility.md:226` | ``- `dev/design/worktree-parallel-work.md` — JIT architecture design`` | contributor study |
| `dev/experiments/worktree-manual-coordination-experiment.md:377` | ``- Design: `dev/design/worktree-parallel-work.md``` | contributor experiment — **also managed under D-13**, so both ends move |
| `dev/design/phase2-collapse-expand.md:248` | ``- `dev/design/subgraph-clustering-layout.md` - Overall clustering design`` | intra-area |
| `dev/design/phase3-advanced-features.md:297` | ``- `dev/design/subgraph-clustering-layout.md` - Overall design`` | intra-area |
| `dev/design/phase3-advanced-features.md:298` | ``- `dev/design/phase2-collapse-expand.md` - Phase 2 plan (prerequisite)`` | intra-area |

`dev/design/worktree-parallel-work.md` is the hot spot: **9 terminal issues** link it (ad601a15,
65e7dccd, 7051d24e, 730d25b5, 909c78cc, 92bf3a9b, b74af86f, f0235aa4, f84945f7) and **8 live
citations** point at it, two of them adopter-facing and one a production Rust doc comment.
Archiving it moves the file out from under all eight.

The three intra-area citations are self-consistent only if all three files move **in the same
plan**. `phase2-collapse-expand.md` (owner 6f678db0, `done`) and `subgraph-clustering-layout.md`
(owner d4290046, `done`) belong to different containers, so a per-container archive will move
one and leave the other — breaking `phase2-collapse-expand.md:248` until the second container
archives. Worth calling out as a sequencing hazard for REQ-09.

**Not citations** (verified, listed to forestall false positives): `docs/concepts/core-model.md:173,296`
use `"path": "dev/design/auth-design.md"` as an **illustrative JSON sample** — no such file
exists. `docs/concepts/guarantees.md:556` says `Referenced in: dev/design.md`, a sample
document name inside example output, not the `dev/design/` directory.

### `dev/benchmarks/` — live citations

| path:line | citation | Kind |
|---|---|---|
| `dev/TESTING.md:257` | `[dev/benchmarks/rust-build-efficiency/report.md](benchmarks/rust-build-efficiency/report.md)` | **contributor doc; a resolvable relative markdown link that breaks on move** |
| `dev/TESTING.md:291` | same | same |
| `scripts/benchmark-rust-build.sh:49` | `# Outputs (under BENCH_OUT_DIR, default dev/benchmarks/rust-build-efficiency):` | script comment |
| `scripts/benchmark-rust-build.sh:57` | `#   BENCH_OUT_DIR           output directory (default dev/benchmarks/rust-build-efficiency)` | script comment |
| `scripts/benchmark-rust-build.sh:98` | `OUT_DIR="${BENCH_OUT_DIR:-dev/benchmarks/rust-build-efficiency}"` | **executable default — recreates the directory, per D-13's premise** |
| `scripts/benchmark-rust-build.sh:345` | `# Derives dev/benchmarks/.../pre-change-test-inventory.json from a completed` | script comment |
| `dev/benchmarks/rust-build-efficiency/report.md:33` | ``branch shows zero diff against HEAD outside `dev/benchmarks/` after the run.`` | intra-area, self-referential |
| `dev/benchmarks/rust-build-efficiency/report.md:343` | ``so the two revisions' evidence can sit side by side under `dev/benchmarks/``` | intra-area |

**Not a citation:** `web/src/components/Document/renderers/ImageRenderer.test.tsx:25,35` use
`path: 'dev/benchmarks/figures/result.png'` as a test fixture literal; no such file exists.

**Correction to the brief.** `scripts/cargo-ci.sh:88` does **not** cite `dev/benchmarks` — it
cites `dev/active/73482aa1-rust-build-efficiency.md` (`# dev/active/73482aa1-rust-build-efficiency.md,
Baseline table`). It belongs to the 73482aa1 cluster of claim 12 / Q7, not to this list.
`dev/TESTING.md:257,291` are correctly identified.

### `dev/experiments/` — live citations

The area holds exactly one file, `dev/experiments/worktree-manual-coordination-experiment.md`
(linked by ad601a15, `done`), and it attracts exactly one live citation:

| path:line | citation | Kind |
|---|---|---|
| `dev/studies/ai-tool-worktree-compatibility.md:229` | ``- `dev/experiments/worktree-manual-coordination-experiment.md` — original parallel work experiment`` | contributor study |

Note the coupling: that same study cites `dev/design/worktree-parallel-work.md` three times
(`:81,210,226`), so `dev/studies/ai-tool-worktree-compatibility.md` alone carries **4** of the
citations that D-13's managed reclassification breaks. It is the single highest-value repoint
target. The file itself sits in `dev/studies`, already managed and unprefixed, so D-15's blanket
`dev/studies` reclassification decides its own fate independently — if it is archived in the
same pass, the citations move with it and the breakage is cosmetic; if it stays, all four break.
Sequence matters here.

The experiment file also cites back at `dev/design` (`dev/experiments/worktree-manual-coordination-experiment.md:377`,
already listed above), so `dev/design` and `dev/experiments` cross-reference each other and are
owned by different containers — the same sequencing hazard flagged for the intra-`dev/design`
citations.

**Combined REQ-09 repoint list: 20 live citations** — 11 for `dev/design`, 8 for
`dev/benchmarks`, 1 for `dev/experiments` — of which 2 are adopter-facing (`docs/`), 1 is a
production Rust doc comment, 1 a test comment, 5 are shell-script lines (1 of them,
`scripts/benchmark-rust-build.sh:98`, executable code whose default path would point at a moved
directory), and 11 are contributor-doc prose. Adding the 6 citations of
`dev/active/73482aa1-rust-build-efficiency.md` from claim 12 gives **26 in-content citations**
that REQ-09 must repoint and REQ-08 must warn about.

Concentration is high and works in the plan's favour: **`dev/design/worktree-parallel-work.md`
attracts 8 of the 20** and `dev/benchmarks/rust-build-efficiency/report.md` a further 6, so two
files account for 14. Repointing is a bounded, well-identified task rather than a broad sweep.

## Claim 15 and item 14, revisited under the pinned decisions

`dev/plans` (managed) and `dev/vision` (permanent) are now decided, so only the cost remains.

- **`dev/plans` → managed.** All 6 files are terminal-owned, so the whole area archives. **No
  live citation of `dev/plans` exists anywhere** outside `dev/archive/**` and historical
  `dev/active/**` records — verified by the same sweep. Cost: zero broken citations. The only
  wrinkle is the 4 suffix-form filenames (`benchmarks-8d80b5dd.md`, `error-recovery-0587a73a.md`,
  `metrics-713ff59d.md`, `stalled-detection-c802a9b0.md`), which a prefix-based REQ-04 resolver
  will not recognize — they archive correctly regardless, because archival membership comes from
  document links, not filename parsing.
- **`dev/vision` → permanent.** Cost: zero. Its one linked file is owned by a live issue
  (9db27a3a, `backlog`) and so is never selected; permanence additionally guarantees the
  `.jit/config.toml:202` projection source is never relocated even after 9db27a3a completes.
  This is the decision that protects `jit project render`.
- **Canonical documentation home** — unchanged from claim 15: `docs/reference/configuration.md`
  §`[documentation]` (`:45-88`). D-13 enlarges what it must state (8 managed + 3 permanent areas
  + 3 file entries, versus today's 3 + 1), which strengthens the `@/inv/single-source-prose`
  argument for stating the classification **once** and citing it elsewhere. `dev/index.md:19-104`
  remains the sharpest conflict — and note that under D-13 it becomes a *permanent file entry*
  whose own directory links are the source of the `unsupported-artifact-type` population, so
  REQ-12 and Q9 touch the same file.

## Layer assignment for REQ-14

| Work | Layer | Site |
|---|---|---|
| Scope the fallback to "outside `development_root`" | **domain, pure** | `artifact_classifier.rs:833-853` (`selected_destination_roots`) — needs `development_root` threaded into `ArtifactClassificationPolicy` (`:29-75`), which today carries only `managed_paths`, `permanent_paths`, `archive_root` |
| Retain `UnmanagedSelectedRoot` in the schema vocabulary | **domain** | `artifact_plan.rs:412,430,448` — do not remove |
| "Relocates no file outside the development root" | **already guaranteed** | `artifact_classifier.rs:768-771` restricts `PendingDeletion` to `Move`; needs a regression test only |
| Acceptance test | **CLI integration** | `crates/jit/tests/cli_repo_workflow/archive_preview_cli_tests.rs:702` — existing assertion shape |

Note `ArtifactClassificationPolicy` (`artifact_classifier.rs:29-75`) does **not** currently carry
`development_root`, though `DocumentationConfig::development_root()` exists
(`config.rs:324-328`). Threading it through is the one structural change REQ-14 needs, and it
stays inside the domain layer.

**`@/inv/domain-agnostic` check for REQ-14:** the scoped fallback compares against the
configured `development_root`, introducing no `dev/`-shaped literal into engine code. It moves
*toward* the invariant, since the current blocker's blast radius is defined by path lists alone.

## Additional open questions

**Q9 (blocking REQ-02/REQ-09/REQ-14) — the `unsupported-artifact-type` population.**
376 instances across 16 containers, 11 of which have no other blocker. No criterion covers it,
and eligibility is all-or-nothing (R1), so REQ-09 cannot complete for those 16. Root cause is
directory-target markdown links in `dev/index.md:21,28,31,34,45,52,59,62,65,72,75`,
`docs/index.md`, and `dev/benchmarks/rust-build-efficiency/report.md`, blocked at
`artifact_classifier.rs:712-716`. Recommend adding a criterion for remediation **(i)** — skip
directory-typed embedded edge targets rather than blocking them, a pure-domain change that fixes
all 16 at once and is defensible semantics (a directory link is navigation, not a document).
Confirm, or narrow REQ-09 to the containers that can actually archive.

**Q10 (D-15) — `dev/studies` is not fully homogeneous.**
The blanket reclassification is safe for `dev/sessions` (30 files, uniformly dated records) but
`dev/studies` holds 19 files including one issue-prefixed artifact
(`cdc840ad-audit-2026-07-23.md`), a `perf/` subdirectory, and two live cross-reference targets:
`scripts/benchmark-session-cost-selftest.sh:89` **reads
`dev/studies/perf/session-cost-27ffbd2d.json` at runtime**, and
`dev/studies/documentation-organization-strategy.md` is cited from
`dev/authoring-conventions.md`. Confirm the blanket call carves out the runtime-read JSON, or
accept that the selftest script needs repointing in the same change.

**Q11 (REQ-14 wording) — "blocks no container archive plan" is stronger than the mechanism can deliver.**
Because eligibility is all-or-nothing, REQ-14 read literally promises that a plan containing an
out-of-root linked artifact is eligible — but such a plan can still be ineligible for unrelated
reasons (Q9's directory links; a `destination-conflict`; a non-terminal owner). Recommend
narrowing the criterion to its own second clause, which is precisely testable: *no
`unmanaged-selected-root` blocker is reported for a source, script, agent-asset, or
repository-root path, and execution relocates no file outside the development root.* As written,
a reviewer checking REQ-14 against f2532a2d will find `eligible: false` and read the criterion
as unmet even after the work is correctly done.
