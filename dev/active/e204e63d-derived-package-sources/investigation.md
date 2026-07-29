# Investigation — e204e63d Derived profile assets and projected policy documentation

Read-only fact-finding for the plan. Every finding carries a `file:line` citation.
Line numbers reflect the tree at the time of writing (branch `main`, HEAD `e48d30a5`).

---

## 1. Claim classification

### C1 — 61 live assets + 1 region source; byte + executable-mode equality asserted

**valid-and-open, with one correction to the region half.**

- `profiles/jit-dogfood/manifest.toml` declares **64 `[[asset]]` entries**: 3 install-only
  (`manifest.toml:320-330`, sources under `assets/install/`) and **61 live**
  (`manifest.toml:334-583`, sources under `assets/live/`), plus **1 `[[region]]`**
  (`manifest.toml:585-589`, source `assets/live/regions/agents-jit-guidance.md`,
  target `AGENTS.md`, `region-id = "dogfood-guidance"`, `placement = "append"`).
  On disk the package carries 65 asset files (3 install + 62 live: 61 assets + the region
  source) plus `manifest.toml`; `jit profile show jit-dogfood --json` reports
  `"file_count": 66`, `"byte_size": 430232`.
- `crates/jit/src/profile/dogfood.rs:437` `test_live_assets_match_every_declared_source_tree_consumer`
  asserts, for each of the 61 live assets: byte equality against the live repository file
  (`dogfood.rs:446-458`) and executable-mode equality in both directions
  (`dogfood.rs:459-473`, `#[cfg(unix)]`, `mode() & 0o111 != 0` vs `asset.executable`).
- **Correction:** the region is *not* covered by byte equality of a whole file. The region
  loop (`dogfood.rs:483-522`) extracts the body between `<!-- jit:dogfood-guidance:begin -->`
  and `<!-- jit:dogfood-guidance:end -->` in the live `AGENTS.md`, and — because that body
  itself nests the `jit:invariants` projection region — normalizes **both** sides through
  `render_managed_document` with a placeholder invariants claim (`dogfood.rs:492-513`)
  before comparing. Executable mode is not checked for the region. So the region pair is
  guarded on *prose inside the region, modulo the nested invariants sub-region*, not on bytes.
- Seven live assets are declared `executable = true`
  (`.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh`,
  `.../dispatch-worker-worktree.sh`, `.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py`,
  `.agents/skills/jit-project-lead/scripts/standards-fix.sh`, `.../standards-scan.sh`,
  `scripts/ai-review.sh`, `contrib/gates/ai-review.sh`).

### C2 — the test walks declared→live only

**valid-and-open.** The loop at `dogfood.rs:446-450` iterates
`package.manifest().assets` filtered by `JIT_DOGFOOD_LIVE_SOURCE_PREFIX`; nothing enumerates
repository files. The nearest existing guard,
`test_live_assets_cover_source_consumers_and_exclude_install_only_state`
(`dogfood.rs:380-407`), asserts a hand-written four-target spot check
(`dogfood.rs:390-395`) — a sample, not a completeness check, and itself a hand-maintained
literal list.

Note the adjacent primitive that *does* exist: package validation rejects a file present in
the package directory but undeclared in the manifest —
`ProfilePackageError::ExtraContent` (`crates/jit/src/profile/package.rs:155`), raised at
`package.rs:302-309`. That closes package-tree→manifest completeness; it says nothing about
repository-tree→manifest completeness, which is REQ-05's gap.

### C3 — every live asset's `target` is its `source` minus `assets/live/`

**already-done / true.** Verified mechanically over the parsed manifest: 61/61 live assets
satisfy `source["assets/live/".len()..] == target`; zero mismatches. The mapping is purely
mechanical, so a derived manifest can compute one side from the other.

### C4 — the two `plan` declarations differ ONLY in three description strings

**valid-and-open / exactly true.** Parsed both TOML declarations and diffed recursively:

| field | `.jit/templates.toml` | `manifest.toml` contribution |
|---|---|---|
| `description` (template) | `.jit/templates.toml:11` "Plan-before-fan-out bracket: planning node P and breakdown node B." | `manifest.toml:278` "Plan-before-fan-out bracket with planning and breakdown review." |
| `nodes[0].description` | `.jit/templates.toml:30` | `manifest.toml:291` |
| `nodes[1].description` | `.jit/templates.toml:37` | `manifest.toml:298` |

Everything else — `name`, `applies_to`, `anchors` (incl. `gates = ["repo-validate"]`),
both nodes' `role`/`type`/`gates`/`doc_area`/`doc`/`labels`/`depends_on`, `anchor_edges`,
`transforms` — is structurally identical after parse.

**Out-of-scope neighbours worth flagging to the plan:** two further copies of the `plan`
template exist at `docs/examples/sdd/templates.toml:13-45` and
`docs/examples/research/templates.toml`. The SDD copy carries the *same* planning-node
description string as `.jit/templates.toml:30` (verified: that string occurs in
`docs/examples/sdd/templates.toml` and nowhere else outside `dev/archive/`). D-1 names only
`.jit/templates.toml`; the example files are adopter-facing illustrations of a *different*
ruleset and are not in REQ-02's scope, but a reviewer will notice them.

### C5 — `jit profile show jit-dogfood --json` exposes the full `plan` contribution

**already-done / true.** Ran it: top-level keys `byte_size`, `file_count`, `manifest`,
`origin`, `package_hash`, `target_hashes`; `manifest` carries `asset`, `contribution`,
`profile`, `region`. The `templates`/`plan` contribution is emitted whole, including all
three description strings, `anchors`, both `nodes`, `anchor_edges` and `transforms`. A
repository-local generator needs no new CLI surface. Command implementation:
`crates/jit/src/commands/profile.rs`.

### C6 — configuration.md restates the full lists; example-config.toml restates a subset and says so

**valid-and-open / true.**

- `docs/reference/configuration.md:47-77` — a fenced `toml` block restating all three
  shipped lists verbatim (8 `managed_paths`, 6 `permanent_paths`, 4 `issue_scoped_areas`),
  matching `SHIPPED_DOCUMENTATION_POLICY` (`crates/jit/src/config.rs:488-524`) entry for
  entry. `configuration.md:78-84` cites the constant by name but nothing binds the values.
  The block correctly omits `citation_scan_roots`, which `jit init` does not scaffold
  (stated at `configuration.md:196-199`).
- `docs/reference/example-config.toml:19-46` — a deliberate subset
  (`managed_paths = ["dev/active","dev/experiments"]`,
  `permanent_paths = ["dev/architecture","dev/index.md"]`,
  `issue_scoped_areas = ["dev/active"]`) with the disclaiming comment at
  `example-config.toml:26-29`: "These entries illustrate the authoring forms rather than the
  shipped classification". D-3 replaces this subset with the shipped table.

### C7 — `jit config get documentation` reports the effective table, not the shipped constant

**valid-and-open / true.** Ran it in this repository: the output includes
`citation_scan_roots` with 8 entries, a key `SHIPPED_DOCUMENTATION_POLICY` does not carry at
all (`crates/jit/src/config.rs:460-479` — the struct has exactly `development_root`,
`archive_root`, `managed_paths`, `permanent_paths`, `issue_scoped_areas`). Those 8 entries
come from this repository's own `.jit/config.toml:50-59`. The list values happen to coincide
today because `.jit/config.toml:10-11` states it "adopts the shipped classification
verbatim" — a hand-maintained coincidence, not a binding. Under the CLAUDE.md dogfooding
boundary this command is not a legitimate source for adopter documentation.
`docs/reference/configuration.md:82-89` already says so in prose.

### C8 — `generate_config_toml` renders the constant; `jit init` has no print-only flag

**already-done / true, with the construct named precisely.**

- `HierarchyTemplate::generate_config_toml` (`crates/jit/src/hierarchy_templates.rs:63`) is a
  method returning a `String`. Its inline `format!` template contains the `[documentation]`
  table at `hierarchy_templates.rs:288-297`; the substituted values come from
  `SHIPPED_DOCUMENTATION_POLICY` at `hierarchy_templates.rs:329-336`, with the array bodies
  produced by the free function `render_policy_paths` (`hierarchy_templates.rs:22`).
- `jit init --help` offers exactly `--hierarchy-template`, `-q/--quiet`, `--profile`,
  `--json`, `-h/--help`. No flag prints the scaffolded config without creating a repository.
  D-4's "run `jit init` into a throwaway directory" is therefore the only shipped-code-path
  route, as its rejection note assumes.

### C9 — `BINARY_BUILD_INPUTS` omits the live consumer roots

**valid-and-open / true.** `crates/jit/src/domain/build_provenance.rs:117-127`:

```rust
const BINARY_BUILD_INPUTS: &[&str] = &[
    "Cargo.toml", "Cargo.lock", "crates/jit/Cargo.toml", "crates/jit/Cargo.lock",
    "crates/jit/build.rs", "crates/jit/src/", "profiles/jit-dogfood/",
    "scripts/hooks/pre-commit", "scripts/hooks/pre-push",
];
```

`profiles/jit-dogfood/` is at `:124`. Absent: `.agents/skills/`, `contrib/`,
`.jit/reference/`, and `scripts/` broadly — only the two hook files at `:125-126` are
listed, and none of the four packaged `scripts/*` live assets (`ai-review.sh`,
`code-review-prompt.md`, `plan-review-prompt.md`, `breakdown-review-prompt.md`) is covered.
Matching is prefix-based for a trailing-slash entry and exact otherwise
(`build_provenance.rs:130-136`). So once the packaged tree is derived from those paths,
editing a live consumer alone leaves the staleness report silent — REQ-06 is real.

### C10 — `build.rs` reads nothing ambient and emits only four `rerun-if-env-changed`

**already-done / true.** `crates/jit/build.rs:24-27` emits exactly
`JIT_BUILD_GIT_HASH`, `JIT_BUILD_GIT_SHORT_HASH`, `JIT_BUILD_GIT_DIRTY`,
`SOURCE_DATE_EPOCH` as `rerun-if-env-changed`. No `rerun-if-changed` anywhere. It reads
`PROFILE` and `TARGET` from cargo (`build.rs:42-43`) and emits six `rustc-env` values. The
header comment (`build.rs:3-22`) records why: watching `.git/index`/HEAD/refs or the wall
clock relinked every test target on a metadata-only git operation (jit:5d862134).

---

## 2. Answers to Q1–Q10

### Q1 — `include_dir!` with an `$OUT_DIR`-rooted path; metadata embedding

Registry source read at
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/include_dir_macros-0.7.4/src/lib.rs`.

**Path expansion — `$OUT_DIR` works.** `include_dir!` takes one string literal
(`lib.rs:18-29`) and passes it to `resolve_path` (`lib.rs:163-190`). `resolve_path` scans for
every `$` and expands the following identifier through `get_env` — there is **no allowlist**
of variable names; any environment variable visible to the proc macro resolves
(`lib.rs:170-186`). On stable, `get_env` is `std::env::var` (`lib.rs:258-261`); the
`nightly` feature swaps in `proc_macro::tracked_env::var` (`lib.rs:253-256`). Expansion is
single-pass, not recursive (test `dont_resolve_recursively`, `lib.rs:294-306`). An
unresolvable variable is a compile error (`MissingVariable`, `lib.rs:176-178`). Cargo sets
`OUT_DIR` for the crate's own compilation whenever the package has a build script, so
`include_dir!("$OUT_DIR/jit-dogfood")` expands correctly.

**Metadata — none embedded with `default-features = false`.** `expand_file`
(`lib.rs:80-103`) appends `.with_metadata(...)` only when `metadata(path)` returns `Some`,
and `metadata` returns `None` unless `cfg!(feature = "metadata")` (`lib.rs:110-112`).
`include_dir-0.7.4/Cargo.toml:60-63` declares `default = []`, `metadata = [...]`,
`nightly = [...]`; `crates/jit/Cargo.toml:43` pins
`include_dir = { version = "0.7.4", default-features = false }`, and `Cargo.lock:1171-1177`
shows `include_dir` with only `include_dir_macros` as a dependency (no `glob`). So **no
mtime/atime/ctime is baked in** — REQ-04 is not threatened from this direction.
`crates/jit/src/profile/package.rs:907-918` already pins this contract in a test
(`test_embedding_dependency_contract_has_no_optional_features_and_production_tree`) by
asserting the exact dependency line.

**File bytes are embedded via `include_bytes!` with the canonicalized absolute path**
(`lib.rs:81-90`). The absolute path is a compile-time argument to `include_bytes!` and does
not enter the produced data; the path stored in the `File` value is the root-relative,
`/`-normalized form (`lib.rs:93-96`, `normalize_path` at `lib.rs:130-137`). So an
`$OUT_DIR`-rooted embed produces the same relative source keys as today's
`$CARGO_MANIFEST_DIR`-rooted one, and machine-specific absolute paths do not leak into the
embedded values.

**One consequential detail for REQ-03/REQ-05.** `track_path` (`lib.rs:263-266`) is a
**no-op on stable** — it only calls `proc_macro::tracked_path::path` under the `nightly`
feature. Directory *membership* is therefore not registered with cargo. What *is* registered
is each file's content, because `include_bytes!` makes rustc list the file in its dep-info.
Verified empirically: `target/debug/jit.d` and `target/debug/libjit.d` each list exactly 66
`profiles/jit-dogfood/...` paths (65 asset files + `manifest.toml`) and **no directory
entry**. Consequence today: editing a packaged file rebuilds; *adding* a file under
`profiles/jit-dogfood/assets/` does not invalidate the crate on its own.

### Q2 — `rerun-if-changed` obligations

**Directory-level declarations are sufficient.** Cargo book, offline copy at
`~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/share/doc/rust/html/cargo/reference/build-scripts.html`
(rendered text, lines 495-502):

> `cargo::rerun-if-changed=PATH` — The `rerun-if-changed` instruction tells Cargo to re-run
> the build script if the file at the given path has changed. Currently, Cargo only uses the
> filesystem last-modified "mtime" timestamp to determine if the file has changed. It
> compares against an internal cached timestamp of when the build script last ran.
> **If the path points to a directory, it will scan the entire directory for any
> modifications.**

Also (same page, lines 485-490): if a build script emits *no* `rerun-if` instruction, cargo
falls back to re-running it whenever any file in the package changes. `crates/jit/build.rs`
currently emits only `rerun-if-env-changed`, which counts as a `rerun-if` instruction, so
today the script does *not* re-run on source edits.

Prior art in this workspace for a directory-level declaration:
`crates/server/build.rs:2` — `println!("cargo:rerun-if-changed=../../web/dist/");`.

**Paths a derivation would have to declare.** Derived from the 61 live asset targets, the
minimal set of roots is:

| root | live assets packaged from it |
|---|---|
| `.agents/skills/` | 55 |
| `scripts/` | 4 (`ai-review.sh`, `breakdown-review-prompt.md`, `code-review-prompt.md`, `plan-review-prompt.md`) |
| `contrib/gates/` | 1 (`ai-review.sh`) |
| `.jit/reference/` | 1 (`content-standards.md`) |

Plus, for the region source, whatever authors `assets/live/regions/agents-jit-guidance.md`
(today that file has no live counterpart — its live counterpart is a *region inside*
`AGENTS.md`, so `AGENTS.md` would have to be declared if the region source were derived
too), plus `profiles/jit-dogfood/manifest.toml` itself.

**Caveats the plan must weigh, not guesses:**
- mtime-based, so a `git checkout`/`git stash` that rewrites an untouched file bumps its
  mtime and re-runs the script. This is the exact class of spurious invalidation
  jit:5d862134 removed from this build script (`crates/jit/build.rs:6-15`); a directory
  watch on `.agents/skills/` re-introduces sensitivity to branch switches. Whether the
  re-run then *relinks* depends on whether the script rewrites OUT_DIR bytes — a
  content-compare-before-write copy keeps the OUT_DIR mtimes stable and the crate fresh.
- `.jit/reference/` sits inside `.jit/`, the repository's own tracker data root. Declaring
  `.jit/reference/` (not `.jit/`) keeps issue-JSON churn out of the build inputs; declaring
  `.jit/` would make every issue mutation re-run the build script.

### Q3 — ordering guarantee (build.rs before the crate's `include_dir!` expansion)

**Confirmed from offline material.** Same cargo book page, "Life Cycle of a Build Script",
lines 221-234:

> Just before a package is built, Cargo will compile a build script into an executable (if it
> has not already been built). It will then run the script, which may perform any number of
> tasks. […] **Once the build script successfully finishes executing, the rest of the package
> will be compiled.** Scripts should exit with a non-zero exit code to halt the build if
> there is an error.

The crate's proc-macro expansion happens during "the rest of the package will be compiled",
strictly after the script exits successfully. REQ-04's ordering premise holds and can be
asserted with this citation.

**Two real hazards the same page raises, both load-bearing here:**

1. **OUT_DIR is not cleaned** (lines 254-260):
   > Cargo does not clean or reset `OUT_DIR` between builds. The contents of this directory
   > may persist across rebuilds, even if the build script is re-run. […] Build scripts
   > should not rely on `OUT_DIR` being empty […] If a script requires a clean directory, it
   > is currently responsible for managing or cleaning up any files or subdirectories it
   > creates.

   A live consumer that is *deleted* or *renamed* would leave its stale copy in OUT_DIR, and
   `include_dir!` reads the whole directory — producing a package that still ships the
   removed file. The derivation must remove its own subtree (or reconcile it) before writing.
   This is a different failure than the "silently stale binary" the issue Notes name, and it
   is not hypothetical.
2. **The build script's cwd is the package root** (line 238-239: "the build script's current
   directory is the root directory of the build script's package"), i.e. `crates/jit/`. The
   repository root is `../..` — the same relative walk the existing tests use
   (`dogfood.rs:441`).

### Q4 — `.jit/templates.toml` today, and TOML region splicing

**The file in full** is 48 lines. Structure and comment placement:

- **Lines 1-7** — a 7-line file header comment, entirely *outside* the `[[template]]` block.
- **Line 8** — blank.
- **Lines 9-48** — the single `[[template]]` block (`.jit/templates.toml:9`), with
  sub-tables at `:14` (`[[template.anchors]]`), `:24` and `:32` (`[[template.nodes]]`),
  `:41` (`[[template.anchor_edges]]`), `:46` (`[[template.transforms]]`).
- **Comments *inside* the block:** trailing inline comments at `:12`, `:15`, `:22`, `:26`,
  `:27`, `:28`, `:38`, `:42`, `:43`, `:48`; standalone comment blocks at `:16-21` (a six-line
  explanation of `repo-validate`, citing REQ-13 of issue 552ff75c and
  `@/invariant/gate-semantics`), `:40`, and `:44`.
- Keys are **column-aligned** (`name        = "plan"`) and sub-tables are **two-space
  indented** — neither is preserved by any TOML serializer this repository has.

Practical consequence for D-1: a generated region can preserve the lines 1-7 header (it is
outside the block) but **cannot preserve any of the in-block commentary or the alignment**
unless the generator emits them as literal text. The six-line `repo-validate` rationale at
`:16-21` is genuine repository knowledge that would be lost by a naive round-trip; the plan
should decide explicitly where it goes (e.g. moved above the region markers, or carried in
the manifest as a comment the generator re-emits).

**Does existing machinery splice a marked region into a TOML file?**

- The **byte-level splicer is format-agnostic and reusable.**
  `render_managed_document` (`crates/jit/src/repository_state/managed_document.rs:65`) takes
  arbitrary `begin`/`end` delimiters as `Vec<u8>`
  (`managed_document.rs:32-34`) and preserves every byte outside them
  (`managed_document.rs:105-109`). It hardcodes nothing markdown-specific. `#`-prefixed
  TOML comment markers would work.
- The **projection layer is markdown-only in practice and item-kind-bound.**
  `crates/jit/src/repository_state/projection.rs:1-33` states the two modes and that every
  body renderer produces markdown (`render_id_anchor_rows`, `render_invariants_markdown`,
  `render_rules_and_gates_markdown`). `ProjectionConfig` requires `kind` naming one or more
  *addressable item kinds* (`crates/jit/src/config.rs:1152-1156`) and there is no `template`
  item kind. `region-begin`/`region-end` are freely configurable strings
  (`config.rs:1165-1172`), defaulting to HTML comments (`config.rs:1002-1011`).
- **No non-markdown projection target exists today.** The three declared projections
  (`.jit/config.toml:255-279`) target `AGENTS.md` (twice) and
  `docs/reference/rules-and-gates.md`.

So: reuse `render_managed_document` for TOML splicing; do **not** try to express D-1 as a
`[projection.*]` table. D-4's rejection note ("a projection over a new engine item kind […]
would need a source mode for a compiled constant that no declared kind has") is correct and
extends to D-1 for the same structural reason.

**Loader tolerance:** `.jit/templates.toml` is parsed by `TemplateRegistry::load`
(`crates/jit/src/templates.rs:1120`, exercised on the real repository file by
`test_repo_plan_template_parses` at `templates.rs:1113-1126`). TOML comments are ignored by
the parser, so `#`-comment region markers are invisible to every consumer.

### Q5 — where REQ-05's completeness check can live

**(a) Does the existing dogfood test reach the repo root?** Yes.
`dogfood.rs:441` — `let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");`.
The same idiom appears at `dogfood.rs:645`, `dogfood.rs:660`,
`crates/jit/src/profile/package.rs:914-915`, and `crates/jit/src/templates.rs:1114-1117`.
An inline `#[cfg(test)]` module can therefore reach and walk the repository root.

**(b) Is `git2` usable from a test in this crate?** Yes. `crates/jit/Cargo.toml:30` declares
`git2 = "0.21"` as an ordinary, non-optional, non-feature-gated dependency of the `jit`
library, so it is in scope for inline `#[cfg(test)]` modules and for `tests/` targets alike.
Shelling out to `git ls-files` is also established prior art in this repository's tests —
`crates/jit/tests/provenance_contract/repository_inventory.rs:33-41` runs
`git ls-files --cached --others --exclude-standard`.

**(c) What a plain filesystem walk picks up.** Walked the four packaged roots (excluding
`.git`, `node_modules`, `target`) and diffed against the 61 declared targets:

| root | files on disk | declared | **undeclared** |
|---|---|---|---|
| `.agents/skills` | 107 | 55 | **52** |
| `scripts` | 36 | 4 | **32** |
| `contrib` | 5 | 1 | **4** |
| `.jit/reference` | 1 | 1 | **0** |

The 52 undeclared files under `.agents/skills` are, by shape:
`*/evals/**` (evals.json, results.md, setup-test-repo.sh, fixtures/**, transcripts/**),
`*/trigger_eval.json`, `*/trigger_eval_results.json`,
`jit-project-lead/scripts/test-standards-fix.sh`, `.../test-standards-scan.sh`, and
`.agents/skills/jit-project-lead/references/.gitkeep`.
The 32 under `scripts/` are every non-packaged script and prompt in the repository
(`cargo-ci.sh`, `docs-*.sh`, `install-jit.sh`, `hooks/*`, …). The 4 under `contrib/` are
`contrib/README.md` and `contrib/gates/prompts/{code-review,security-audit,test-adequacy}.md`.

**This is the decisive number for D-2's shape.** `scripts/` and `contrib/` are *not* usable
as declared packaged roots — 32 of 36 and 4 of 5 files would need exclusions, which inverts
the intent. Either the declared roots must be finer-grained (per-skill directories plus
per-file entries for the five `scripts/`+`contrib/` assets), or the exclusion vocabulary
must be pattern-based (`*/evals/`, `trigger_eval*.json`, `test-*.sh`, `.gitkeep`) rather
than a path list. `.agents/skills/` with ~5 exclusion patterns is tractable;
`scripts/`-as-a-root is not.

Also note `.agents/worktrees/` exists on disk as a sibling of `.agents/skills/` (it holds
other agents' live git worktrees). A walk rooted at `.agents/skills` avoids it; a walk
rooted at `.agents` would traverse gigabytes.

**(d) `.gitignore` coverage.** `.gitignore` (30 lines) ignores `/target/`, `**/target/`,
`.jit/**/*.lock`, `.jit-bootstrap.lock`, `.jit/server.log`, `.jit/server.pid.json`,
`.jit/worktree.json`, `.jit/tmp/`, `mcp-server/node_modules/**`, `mcp-server/jit-schema.json`,
`coverage.json`, `badges/`, `.agents/worktrees/` (`:27`), `.claude/` (`:30`). **Nothing under
the four packaged roots is ignored**, and `git status --porcelain --ignored` over those roots
is empty. Tracked counts match the disk walk exactly (107 / 36 / 5 / 1). So *today* a
filesystem walk and a git-tracked query return identical sets.

**Recommendation for the plan:** the two approaches differ only in behaviour under
contributor scratch files. A git-tracked query (`git ls-files`) makes an untracked scratch
file invisible to the check — the desirable behaviour, and consistent with the existing
`repository_inventory.rs` precedent — but adds a git dependency to a test, which
`@/charter/D-4` tolerates for tests but which would make the check silently vacuous outside
a checkout. A filesystem walk needs no git but reports a contributor's scratch file as a
packaging defect. A hybrid (walk the filesystem, exclude what `git check-ignore` rejects) is
the most complex and is not warranted by the current data.

### Q6 — `docs-mechanical` extension shape

**Fan-out.** `scripts/docs-mechanical.sh` is the gate entrypoint. It is deliberately not
`set -e` (`docs-mechanical.sh:2-4`) so every checker runs. It runs three children through the
`run` helper (`docs-mechanical.sh:80-92`) at `docs-mechanical.sh:94-96`:

```
run "M2 links & anchors" "$here/docs-check-links.sh" "${FOOTPRINT[@]}"
run "M3 citations"       "$here/docs-check-citations.sh" "${FOOTPRINT[@]}"
run "M5 projections"     "$here/docs-check-projections.sh"
```

**Conventions a new checker must follow:**

1. **Naming.** `scripts/docs-check-<concern>.sh`, with an `M<n>` label in its header comment
   (`docs-check-links.sh:5`, `docs-check-citations.sh:5`, `docs-check-projections.sh:5` all
   cite "mechanical check M<n> of the `docs-mechanical` gate (epic 2d109173)").
2. **Exit codes, uniform across all three.** `0` pass (each prints an `OK: …` line),
   `1` genuine finding, `2` environment/usage error. Documented in every header
   (`docs-check-links.sh:42-45`, `docs-check-citations.sh:50-55`,
   `docs-check-projections.sh:26-29`). The orchestrator maps `1 → finding`, anything else
   nonzero `→ env_error`, and **exit 2 dominates exit 1**
   (`docs-mechanical.sh:86-90`, `:98-102`).
3. **Footprint argument handling.** Two shapes exist and a new checker must pick one
   explicitly. Footprint-taking checkers require at least one positional path and exit 2 on
   none (`docs-check-links.sh:47-50`, `docs-check-citations.sh:58-60`); they encode no
   default path list. Configured-target checkers take *no* footprint
   (`docs-check-projections.sh:23-24` "takes no footprint — targets are configured") and are
   invoked without `"${FOOTPRINT[@]}"`. **A freshness check for the documentation policy
   belongs in the second class** — its two targets are named facts, not a caller-supplied
   surface.
4. **Both comparison sides derived live.** This is stated as the design rule in every
   header and in the gate description (`.jit/gates.toml:171`: "each deriving both comparison
   sides live from the tree so it encodes no product facts"). `docs-check-projections.sh:12-16`
   is the model: target paths come from `jit project render --json`'s own
   `.projections[].target`, never a hardcoded path.
5. **Prerequisite guards up front**, each exiting 2 with a named tool
   (`docs-check-projections.sh:31-38` guards `jq` and "inside a git work tree";
   `docs-mechanical.sh:42-49` guards `jit` and `jq`).
6. **Self-test registration.** `scripts/docs-check-selftest.sh` is a separate harness (not
   run by the gate) that proves, per checker, both directions: seed the defect → assert
   nonzero; clean → assert zero (`docs-check-selftest.sh:8-10`). A new checker adds a block
   in the same shape as `docs-check-selftest.sh:55-64` (links), `:66-81` (citations), or
   `:121-152` (projections). Two constraints it enforces: the harness must never mutate the
   real repository (`docs-check-selftest.sh:12-17`) — the projection block clones the repo
   into a scratch dir (`:126-127`) precisely because rendering writes to tracked targets —
   and its own exit codes are `0`/`1`/`2` (`:19-22`).
7. **Gate wiring.** `.jit/gates.toml:167-191` declares the `docs-mechanical` gate:
   `stage = "postcheck"`, `mode = "auto"`, `priority = 100`, `auto = true`,
   `[gates.checker] type = "exec"`, `command = "./scripts/docs-mechanical.sh"`,
   `timeout_seconds = 300`, `pass_context = false`, and
   `[gates.checker.env] DOCS_FOOTPRINT = "docs/"`. Adding a checker means editing
   `docs-mechanical.sh:94-96` and the gate `description` at `.jit/gates.toml:171` (which
   enumerates the three checkers by name — itself a hand-maintained enumeration that will go
   stale, worth noting under `@/invariant/single-source-prose`).

**Sizing:** one new ~60-100 line bash script in the shape of `docs-check-projections.sh`,
one line in `docs-mechanical.sh`, one self-test block, one gate-description edit.

**One conflict to resolve:** D-4's generator "runs `jit init` into a throwaway directory".
`docs-check-projections.sh` has a documented in-place side effect (`:18-21`: rendering writes
to the target files, then it `git diff`s). A policy-freshness checker in that same shape
would have to *regenerate* the two documentation regions and diff — meaning the generator
itself must be runnable from the checker, and must be safe to run against the working tree.

### Q7 — consumer sweep

See §4 for the complete inventory. Summary of counts: profile-package consumers span 6
production modules, 5 test suites, 1 script, 1 CI job, 1 MCP test file; the `plan` template
has 4 on-disk declarations and ~20 production consumers; the documentation-policy lists are
restated in 4 authored locations outside the constant; the stale-binary surface has 1
production entry point, 2 test suites, and 3 documentation pages.

### Q8 — build-footprint budget

`scripts/rust-build-budget.sh` enforces two numeric budgets and four policy assertions:

- **REQ-01, integration-test target count** (`rust-build-budget.sh:36`,
  `MAX_INTEGRATION_TARGETS=12`), derived from `cargo metadata --no-deps` counting targets
  whose `kind` contains `"test"` (`:139-143`).
  **Current count: 11** (verified live) — `jit`: `cli_gate`, `cli_issue`,
  `cli_item_validate`, `cli_query_graph`, `cli_repo_workflow`, `fast_docs_templates`,
  `fast_issue`, `fast_rules`, `provenance_contract`, `scratch_build`; `jit-server`:
  `document_api_tests`. **Exactly one slot remains.** If the plan adds a new integration-test
  target it consumes the last slot; if it adds two, the gate fails. Both REQ-02's and
  REQ-05's checks are better placed in an existing suite (or as an inline `#[cfg(test)]`
  module in `crates/jit/src/profile/dogfood.rs`, where the current drift test already lives).
- **REQ-02, active test-executable bytes** (`rust-build-budget.sh:37`, 2 GiB), summed by
  `stat`ing each unique executable path emitted by
  `cargo test --workspace --no-run --message-format=json` with `profile.test == true`
  (`:148-166`).
- **Policy assertions** (`:168-219`): `[profile.dev]`/`[profile.test]` `debug =
  "line-tables-only"`; `scripts/cargo-ci.sh` exports `CARGO_INCREMENTAL=0`;
  `jsonschema` sets `default-features = false` and no `resolve-*`/`tls-*`; `ureq` enables
  `rustls` and not `native-tls`.

**No budget counts OUT_DIR bytes.** The byte budget stats *test executables only*
(`:157-163`); `cargo metadata` counts targets, not artifacts. A build.rs copying ~66 files
(~430 KB, per `jit profile show`'s `byte_size: 430232`) into `OUT_DIR` adds ~430 KB per build
profile under `target/` and is invisible to both measured budgets. The **embedded** bytes are
unchanged — the same content is embedded today from the source tree, so per-executable size
does not move.

The residual `bounded-rust-build-footprint` exposure is not size but **invalidation**: the
build script becomes mtime-sensitive to `.agents/skills/`, `scripts/`, `contrib/gates/`, and
`.jit/reference/`. If the script rewrites OUT_DIR unconditionally, every branch switch
relinks every test target — the exact regression jit:5d862134 fixed, and one the budget
checker does *not* measure. A content-compare-before-write copy avoids it; the plan should
require that behaviour explicitly and cover it (see §8).

### Q9 — prior art

See §3.

### Q10 — reproducibility harness

Two files under `crates/jit/tests/provenance_contract/`:

**`version_cli_tests.rs`** — five tests:
- `test_global_version_flag_reports_local_provenance` (`:6`) — `jit --version` succeeds and
  its text contains the package version, `commit`, `profile`. Not ignored.
- `test_version_command_reports_human_readable_provenance_without_repo` (`:22`) — `jit
  version` outside a `.jit` repository prints the six labelled fields. Not ignored.
- `test_version_command_reports_json_provenance_without_repo` (`:45`) — `jit version --json`
  parses and carries `package`, `version`, `git_commit`, `git_short_commit`, `git_dirty`,
  `build_profile`, `build_timestamp`, `target`. Not ignored.
- `test_version_build_without_injected_provenance_reports_unknowns` (`:87`,
  `#[ignore]`) — REQ-05: a build with no injected provenance reports `"unknown"` commit
  fields, null dirty, `"unknown"` timestamp — i.e. no ambient git or clock leaks in.
- `test_injected_provenance_is_reported_reproduced_and_invalidated` (`:104`,
  `#[ignore]`) — REQ-03/REQ-04: injected values are reported exactly; **repeating the build
  with the same injected environment yields an identical `version --json` document**
  (`:135-139`); changing an injected value changes the reported value; `dirty=true` reports
  `true`.
- Plus `test_workspace_declares_and_inherits_rust_version` (`:160`) — MSRV structure guard.

**`build_provenance_metadata_stability_tests.rs`** — one test,
`test_metadata_only_change_relinks_no_test_targets` (`:83`, `#[ignore]`): seeds an isolated
git repository from the workspace's repository-input inventory, cold-builds all test targets,
then (1) stages a new non-source file and rebuilds, asserting **zero** non-fresh
`profile.test` artifacts, and (2) commits it and rebuilds, asserting zero again.

Both `#[ignore]` suites are run with `--ignored` by `scripts/cargo-ci.sh`'s `provenance`
step (stated at `build_provenance_metadata_stability_tests.rs:14-19` and
`version_cli_tests.rs:70-77`).

**Verdict for REQ-04: existing coverage does not assert what REQ-04 says.** The strongest
existing property is *identical reported provenance metadata* across two builds with
identical injected environment (`version_cli_tests.rs:135-139`) — a JSON-document
comparison, not a binary-byte comparison. Nothing hashes or compares the produced
executable. REQ-04's "byte-for-byte reproducible" therefore needs **new** coverage, not an
extension, unless the plan re-reads REQ-04 as "the embedded package content is identical
across builds", which *is* extensible from the existing shape.

**A concrete blocker the plan must handle.**
`crates/jit/tests/provenance_contract/repository_inventory.rs:60` unconditionally bans
`.jit` from the seeded fixture:

```rust
fn is_banned_input(path: &Path) -> bool {
    path.starts_with(".jit")
        || path.starts_with(".git")
        || path.starts_with(".agents/worktrees")
        || path.components().any(|c| matches!(c.as_os_str().to_str(), Some("target" | "node_modules")))
}
```

`.jit/reference/content-standards.md` is a packaged live asset
(`manifest.toml:559-561`). The moment the packaged tree is derived from live paths, the
build script running inside that fixture repository will not find it, and
`test_metadata_only_change_relinks_no_test_targets` will fail at the cold baseline build.
The same filter also bans `.agents/worktrees` (harmless) but keeps `.agents/skills`
(needed). This is a required, non-obvious edit that the plan must schedule.

---

## 3. Prior-art sweep

Sources: `dev/archive/9b7b5f9c-jit-profiles/dev/active/9b7b5f9c-{plan,research}.md`,
`dev/archive/9b7b5f9c-jit-profiles/9b7b5f9c-completion-report.md`, `dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-investigation.md`,
`dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-plan.md`, `dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-progress.json`,
`dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-handoff-2.md`, `dev/active/73482aa1-rust-build-efficiency.md`,
`dev/vision/9db27a3a-charter.md`, `dev/active/c639cfb5-*.md`.

### Already recorded, supports the work

- **P7** (`dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-progress.json`, `surfaced_pitfalls`) — "no projection binds
  the adopter documentation policy lists to SHIPPED_DOCUMENTATION_POLICY […] they went stale
  from wave 1 to wave 3 and blocked two code reviews (E2 on b3fc1a92, F1 on f88a16d9)".
  Disposition: closed session 5, owner decided, **folded into e204e63d REQ-01**.
- **P15** (same file) — "nothing binds the plan-doc declaration in `.jit/templates.toml` to
  its packaged profile twin […] The only test naming both files is the dogfood profile
  assertion, which greps each for stale PHRASES." Folded into **e204e63d REQ-02**. Verified
  against code: that test is
  `test_planning_bracket_docs_describe_builtin_review_placeholders`
  (`crates/jit/src/profile/dogfood.rs:659`), whose second loop (`dogfood.rs:683-703`)
  reads `.jit/templates.toml`, both `docs/examples/*/templates.toml`, three docs pages and
  `profiles/jit-dogfood/manifest.toml` and asserts each contains none of six stale phrases.
  P15's characterization is accurate.
- **`dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-handoff-2.md:64`** — "Do NOT hand-copy a shipped constant into
  adopter docs without a citation back to it. That is what made the config reference go
  stale from wave 1 to wave 3 and cost two review rounds."
- **`@/charter/D-13`** — "Give each adopter-facing fact one canonical documentation home",
  reasoning cites `@/invariant/single-source-prose`. Directly supports D-3/D-4.
- **`@/invariant/single-source-prose`** — "…a hand-maintained copy is a staleness defect",
  enforced by `@/gate/code-review` (`.jit/invariants.toml`).

### Recorded decisions that D-1..D-4 reopen or reverse — must be named in the plan

1. **`9b7b5f9c-research.md:192` explicitly rejected a build.rs-generated OUT_DIR tree.**
   The alternatives table rejected "custom `build.rs` scans the tree and generates an
   index/archive in `OUT_DIR`" because it "creates bespoke recursive traversal, escaping/code
   generation, deterministic ordering, Cargo change tracking, and generated-output tests…
   That complexity is not justified for one small data tree." Codified as **D1** in
   `9b7b5f9c-plan.md:183`, which lists "custom build generation" among rejected alternatives.
   **REQ-03 requires exactly that mechanism.** The plan must record why the original
   judgment no longer holds — the justification already exists in the issue's own Background
   (two blocked code reviews, silent-omission risk, 61 hand-maintained pairs) but must be
   stated as *superseding*, not as filling a blank.
2. **`9b7b5f9c-plan.md:186` (D4) made the package the authority and the live paths its
   rendered consumers** — "Contributors edit package assets, then render." The shipped code
   still encodes that direction: the drift assertion's message is
   `"{} drifted from the package"` (`crates/jit/src/profile/dogfood.rs:456`), and
   `profiles/jit-dogfood/manifest.toml:332-333` calls `assets/live/` "the renderer's
   data-owned selection rule". **REQ-03 reverses this** — live becomes the authority,
   packaged becomes derived. The renderer D4 specified was never built; what shipped is a
   hand-maintained duplicate plus a byte assertion. The reversal is defensible (it matches
   how skills are actually edited) but it is a reversal.
   **Note the resulting asymmetry the plan must resolve:** D-1 makes the *manifest* the
   authority for the `plan` template while REQ-03 makes the *live tree* the authority for
   assets. Both directions in one manifest is coherent but needs saying out loud, or a
   reviewer reads it as inconsistency.
3. **Issue 8e071e18's Q1/D-11 chose "edit both trees by hand" deliberately.**
   `dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-investigation.md:935-943` weighed (a) bring `profiles/…/assets/live/`
   into scope and edit both, (b) keep the non-goal, (c) relax the drift test — and
   recommended (a); `dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-plan.md:457` formalized it. So the current state is
   a chosen simplification, not an oversight. e204e63d supersedes it.
4. **jit:5d862134 forbids ambient build inputs.** `crates/jit/build.rs:6-15` and
   `dev/active/73482aa1-rust-build-efficiency.md:58-62` record that watching `.git/index`,
   HEAD, refs, or the clock relinked every test target (~179 s). A directory-level
   `rerun-if-changed` on live roots does not watch git metadata, so it does not violate the
   letter of that decision — but it does make the build script mtime-sensitive to a branch
   switch. **Constraint, not contradiction**; see Q2 and §8.
5. **`@/charter/D-8`** (`dev/vision/9db27a3a-charter.md:138-151`) restricts v1.0 to the
   single embedded offline profile with no new lifecycle surface. D-4's choice of a
   repository-local generator over a new `jit` command is consistent with it, and the
   charter is the reason to cite for that rejection.
6. **`dev/archive/9b7b5f9c-jit-profiles/9b7b5f9c-completion-report.md:101-104`** — "Future projection changes must
   therefore preserve the public profile/schema/MCP and installed-checker journey rather than
   silently invalidating it", with `9b7b5f9c-plan.md:155` "No criterion removes or renames a
   public surface". **Constraint:** D-2's declared root+exclusion list must be an *additive*
   manifest schema change. `ProfileManifest`'s wire shape is public through
   `jit profile show --json` and the MCP `jit_profile_show` tool
   (`mcp-server/test-integration.js:356-357` asserts `shown.manifest.profile.id`).

No recorded decision forbids any of D-1..D-4.

---

## 4. Consumer inventory

This report owns the inventory; the plan cites it. Paths are repository-relative from
`/home/vkaskivuo/Projects/just-in-time/`.

### 4.1 Consumers of `profiles/jit-dogfood/**` (and its on-disk existence)

**Production code**
- `crates/jit/src/profile/dogfood.rs:8-9` — `include_dir!("$CARGO_MANIFEST_DIR/../../profiles/jit-dogfood")`, the sole embed site.
- `crates/jit/src/profile/dogfood.rs:12` — `JIT_DOGFOOD_LIVE_SOURCE_PREFIX = "assets/live/"`.
- `crates/jit/src/profile/dogfood.rs:37` `jit_dogfood_package()`, `:42` `jit_dogfood_gate()`, `:70` `jit_dogfood_planning_gate_keys()`.
- `crates/jit/src/profile/mod.rs:21-22` — re-exports.
- `crates/jit/src/profile/package.rs` — `EmbeddedProfilePackage::from_dir` (`:42`), `from_files` (`:47`), bounds `MAX_EMBEDDED_PROFILE_FILES = 512` (`:15`), `MAX_EMBEDDED_PROFILE_BYTES = 4 MiB` (`:18`), `validate_manifest` (`:232`), `MissingContent` (`:299`), `ExtraContent` (`:307`).
- `crates/jit/src/commands/profile.rs` — `jit profile list/show/apply`.
- `crates/jit/src/commands/init.rs` — `jit init --profile`, writes `.jit/profiles/jit-dogfood.json`.
- `crates/jit/src/commands/validate.rs:418,472,538` — `jit_dogfood_package()` in derived-state repair.
- `crates/jit/src/repository_state/profile_apply.rs` — applies contributions/assets/regions.
- `crates/jit/src/gate_presets/builtin.rs` — derives the built-in preset inventory from `jit_dogfood_planning_gate_keys()`.

**Tests**
- `crates/jit/src/profile/dogfood.rs:119-705` — the 13 inline tests listed in §1.
- `crates/jit/src/profile/package.rs:907-918` — `test_embedding_dependency_contract_has_no_optional_features_and_production_tree`: asserts the exact `include_dir` dependency line in `crates/jit/Cargo.toml` **and** that `profiles/jit-dogfood/manifest.toml` exists on disk (`:914-916`).
- `crates/jit/tests/fast_rules/derived_state_repair_tests.rs:15` — `PROFILE_RECORD = ".jit/profiles/jit-dogfood.json"`.
- `crates/jit/tests/cli_item_validate/derived_state_repair_tests.rs:13` — same constant.
- `crates/jit/tests/cli_repo_workflow/repo_discovery_tests.rs:128,140,168,196`.
- `crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs`.
- `crates/jit/tests/cli_issue/failure_lever_registry.toml:590` — fault-injection lever over the profile record.
- `mcp-server/test-integration.js:353,356,357,361,368,372` — MCP `jit_profile_list/show/apply` against `jit-dogfood`.

**Build/packaging config**
- `crates/jit/Cargo.toml:43` — `include_dir = { version = "0.7.4", default-features = false }`.
- `Cargo.lock:1171-1177` — `include_dir` 0.7.4 + `include_dir_macros`.
- No `include`/`exclude` key in `crates/jit/Cargo.toml` — publication would rely on cargo's git-tracked default.

**Scripts / CI**
- `scripts/docs-check-citations.sh:127` — excludes `*/profiles/*/assets/install/*` from citation scanning. **A derivation that moves `assets/live/` out of the tree changes what this glob is protecting against; re-check it.**
- `.github/workflows/ci.yml:181-209` — `profile-adoption` job, running `cargo test --locked -p jit --test cli_repo_workflow profile_acceptance_tests::`.
- Every job that builds `jit` implicitly requires the packaged tree at compile time.

**Documentation**
- `docs/reference/profiles.md:56-58,82` — describes the manifest as "the package inventory: … assets, managed regions, hashes, and executable declarations". Adopter-facing; must stay true after D-2 adds a root/exclusion list.

### 4.2 Consumers of `.jit/templates.toml` / the `plan` template

**On-disk declarations of a `plan` template (4)**
- `.jit/templates.toml:9-48` (this repository's live declaration).
- `profiles/jit-dogfood/manifest.toml:271-307` (the packaged contribution).
- `docs/examples/sdd/templates.toml:13-45`.
- `docs/examples/research/templates.toml`.

**Production code (loader and apply engine)**
- `crates/jit/src/templates.rs` — `TemplateRegistry`, `GraphTemplate`, `RoleBindings`, `AnchorBindings`, `from_toml_str`, `load`.
- `crates/jit/src/repository_state/path.rs:230-231` — `TEMPLATES: Data = "templates.toml"`.
- `crates/jit/src/repository_state/profile_apply.rs:93,181` — `KeyedArrayTarget::Templates → ".jit/templates.toml"` / `"templates.toml"`.
- `crates/jit/src/repository_state/materialize.rs:198,224,748`.
- `crates/jit/src/repository_state/mod.rs:489,494` — config-load wiring.
- `crates/jit/src/config.rs:64-70,1609-1644` — `JitConfig.templates`.
- `crates/jit/src/commands/template.rs`, `template_expand.rs`, `breakdown.rs`, `graph.rs:420-424`, `plan_doc.rs:5`, `validate.rs:762-782`.
- `crates/jit/src/gate_presets.rs:8,21,170`, `gate_presets/builtin.rs`, `gate_presets/planning.rs:25`, `gate_presets/reference.rs`.
- `crates/jit/src/commands/gate.rs:1333,1453,1892` — `apply_gate_preset`.
- `crates/jit/src/cli.rs:99-116`, `main.rs:676`, `output.rs:1385`, `schema.rs:633,1780`.
- `crates/jit/src/validation/repository.rs:32,1068-1069`.

**Tests that read the *repository's own* file (these are what D-1's region must not break)**
- `crates/jit/src/templates.rs:1113-1126` — `test_repo_plan_template_parses`, loads the real `.jit/` and asserts `applies_to`, planning type, breakdown type.
- `crates/jit/src/profile/dogfood.rs:683-703` — stale-phrase grep over `.jit/templates.toml`, both example `templates.toml`, three docs pages, and `profiles/jit-dogfood/manifest.toml`.

**Other tests**
- `crates/jit/tests/fast_docs_templates/{template_apply_tests.rs, bracket_breakdown_tests.rs, templates_loader_tests.rs, planning_preset_tests.rs, template_binding_tests.rs, template_apply_atomicity_tests.rs, batch_export_tests.rs}`.
- `crates/jit/tests/fast_docs_templates/sdd_bracket_tests.rs:146-160` and `research_bracket_tests.rs:156-172` — load `docs/examples/{sdd,research}/templates.toml` and assert each declares `plan`.
- `crates/jit/tests/cli_repo_workflow/{apply_cli_tests.rs, template_binding_cli_tests.rs, steering_scenarios.rs:177}`.
- `crates/jit/tests/cli_query_graph/scope_validation_tests.rs`, `crates/jit/tests/cli_gate/bracket_coverage_gate_run_test.rs:151`, `crates/jit/tests/fast_rules/derived_state_repair_tests.rs:124,173`.

**Documentation**
- `docs/reference/configuration.md:24,330-360` — "Template bindings (`.jit/templates.toml`)".
- `docs/reference/cli-commands.md:2937,3119-3142` — `jit apply`.
- `docs/reference/storage-format.md:22,36,285-286`.
- `docs/concepts/planning-bracket.md:26-141`, `docs/how-to/adopt-planning-bracket.md:30-253`, `docs/how-to/custom-gates.md:625-626`.
- `README.md:240`, `CHANGELOG.md:12`, `.jit/config.toml:77`.

**Agent skills (and their packaged twins, identical line numbers)**
- `.agents/skills/jit-project-lead/{SKILL.md:35,61,203; references/tier-derivation.md:15,31,68,111; references/wave-layering.md:18,120}`.
- `.agents/skills/jit-breakdown/SKILL.md:17`, `.agents/skills/jit-planning-lead/SKILL.md:24,33`.
- `.agents/skills/jit-execution-lead/{SKILL.md:61,106; references/bracketed-breakdown.md:9,13}`.
- Eval harnesses under `.agents/skills/jit-project-lead/evals/` and `.agents/skills/jit-planning-lead/evals/` assert `.jit/templates.toml` presence as a precondition. **These are among the 52 undeclared files under `.agents/skills/` (§Q5c).**

### 4.3 Restatements of the documentation-policy lists

**The authority**
- `crates/jit/src/config.rs:488-524` — `SHIPPED_DOCUMENTATION_POLICY`; struct at `:460-479`; accessors at `:349-436`; `DocumentationConfig` optional overrides at `:310-337`.

**Authored second copies (what REQ-01/D-3 target)**
- `docs/reference/configuration.md:47-77` — full lists.
- `docs/reference/example-config.toml:19-46` — illustrative subset.
- `.jit/config.toml:9-49` — this repository's own table, "adopts the shipped classification verbatim" (`:10-11`). **Repository-local policy under the dogfooding boundary; out of REQ-01's scope, but it is a third hand-maintained copy of the same values.**
- `.jit/gates.toml:185` — fallback-footprint comment referencing `[documentation].permanent_paths`.

**Derived (already correct)**
- `crates/jit/src/hierarchy_templates.rs:22` (`render_policy_paths`), `:63` (`generate_config_toml`), `:288-297` (template), `:329-336` (substitutions).
- `scripts/docs-mechanical.sh:41-65` — derives its default footprint live from `jit config get documentation`'s `.permanent_paths`.
- `.agents/skills/jit-project-lead/scripts/standards-scan.sh:39,65-82,406` — parses `[documentation]` out of `.jit/config.toml` at runtime.

**Consumers of the accessors (not restatements)**
- `crates/jit/src/domain/artifact_classifier.rs:47-172`, `artifact_directory.rs:79-376`, `artifact_conformance.rs:116,295-299`, `artifact_plan.rs:77-90`.
- `crates/jit/src/commands/{document.rs:126,171; plan_doc.rs:438; template_expand.rs:662,981; config.rs:409-534; archive.rs (~32 test-fixture sites)}`.

**Tests asserting the shipped values**
- `crates/jit/src/config.rs:2399-2646` — unit tests comparing fallbacks to `SHIPPED_DOCUMENTATION_POLICY` field by field.
- `crates/jit/tests/cli_repo_workflow/init_tests.rs:769-981` — asserts a freshly-initialized repository's `[documentation]` table equals the constant field for field, and rejects unknown keys (`:867`).
- `crates/jit/tests/cli_repo_workflow/{archive_preview_cli_tests.rs, artifact_conformance_cli_tests.rs, doc_show_tests.rs, config_get_tests.rs}` — literal small lists in fixtures, not restatements of the shipped policy.
- `crates/jit/tests/fast_docs_templates/artifact_plan_model_tests.rs:126-158`.

**Docs prose referencing area names**
- `docs/reference/glossary.md:79`, `docs/reference/cli-commands.md:298,2665`.

**Negative result (verified):** `mcp-server/` and `web/` contain **no** reference to
`managed_paths`, `permanent_paths`, `issue_scoped_areas`, `SHIPPED_DOCUMENTATION_POLICY`,
`templates.toml`, `assets/live`, `include_dir`, or `BINARY_BUILD_INPUTS`. The only
`jit-dogfood` hits in either are the six MCP integration-test lines listed in §4.1.

### 4.4 Consumers of `BINARY_BUILD_INPUTS` / the stale-binary report

- `crates/jit/src/domain/build_provenance.rs:117-127` (the inventory), `:130-136`
  (`is_binary_build_input`), `:138-148` (`binary_build_inputs_changed`), `:150-186`
  (`assess_binary_provenance`, `BinaryProvenance`, `StaleBinaryReason`), `:197-350` (tests,
  incl. `is_binary_build_input("profiles/jit-dogfood/manifest.toml")` at `:312`).
- `crates/jit/src/domain/mod.rs:12-13,30`.
- `crates/jit/src/commands/gate_check.rs:628-741` (`stale_binary_reason_for_repo`,
  `CommandExecutor::stale_binary_reason`), `:1306` (gate-run wiring), `:4206-4433` (tests).
- `crates/jit/src/main.rs:536-660` (exit code 10, `STALE_BINARY`), `:1839,1868`
  (`refuse_if_stale_gate_child`).
- `crates/jit/src/errors.rs:384-503` — `StaleBinaryError`.
- `crates/jit/src/output.rs:850,1079,3392` — `ErrorCode::StaleBinary ↔ "STALE_BINARY"`.
- `crates/jit/src/build_info.rs:5-34`, `crates/jit/build.rs:24-43`, `crates/jit/src/cli.rs:5,25`.
- `crates/jit/src/gate_execution.rs:96-97`.
- `crates/jit/tests/scratch_build/stale_binary_json_exit_tests.rs`,
  `crates/jit/tests/scratch_build/stale_binary_child_process_tests.rs`,
  `crates/jit/tests/scratch_build/main.rs:9-10`.
- `crates/jit/tests/provenance_contract/{version_cli_tests.rs, build_provenance_metadata_stability_tests.rs, repository_inventory.rs, main.rs}`.
- `scripts/install-jit.sh:9,16,48,51`; `scripts/benchmark-session-cost.sh:171,185,188`.
- `docs/reference/error-codes.md:29`; `docs/reference/cli-commands.md:669-670,1749-1862,3856-3862`;
  `CHANGELOG.md:45-51,110`; `README.md:53`; `AGENTS.md:19`.

**Standing oddity worth a line in the plan (not a defect to fix here):**
`scripts/hooks/pre-commit` and `scripts/hooks/pre-push` are in `BINARY_BUILD_INPUTS`
(`build_provenance.rs:125-126`), and `target/debug/jit.d` confirms they *are* compiled in via
`include_str!` from `crates/jit/src/commands/`. So the entries are correct, and the
inventory's doc comment (`build_provenance.rs:110-116`, "the profile and hook paths cover the
files embedded by `include_dir!`/`include_str!` in production code") is accurate. REQ-06's
addition follows the same rule: what the binary embeds is a build input.

### 4.5 `install-jit.sh`, `verify-commit-builds.sh`, hooks

- **`scripts/install-jit.sh`** — header `:4-17` explains the wrapper exists because
  `crates/jit/build.rs` reads no ambient git; `:28-34` resolves the repository root from the
  script's own location and runs `cargo install --path "$repo_root/crates/jit"`. It compiles
  **the working tree**, not a commit: it injects real git provenance
  (`JIT_BUILD_GIT_HASH`/`SOURCE_DATE_EPOCH`, `:48,51`) but never verifies that
  `profiles/jit-dogfood/**` matches the named commit. Unaffected by a derivation, except that
  the derived tree must exist at install time — which it will, since it is generated.
- **`scripts/verify-commit-builds.sh`** — the sharpest dependency on `profiles/` being
  **git-tracked**. Header `:16-23`: "This script judges the COMMIT, not the tree. It resolves
  the named commit's tracked sources with `git archive` into a throwaway directory and builds
  there… staged changes, unstaged edits, and untracked files never enter it." Extraction at
  `:133` (`git archive --format=tar "$commit" | tar -x -C "$src"`), build at `:143-144`.
  **Consequence for REQ-03:** once the packaged tree is derived, the *live* sources
  (`.agents/skills/**`, `scripts/**`, `contrib/gates/**`, `.jit/reference/**`) become
  compile-time inputs, so they must all be git-tracked at the named commit or this script
  fails or builds a wrong package. They are all tracked today (§Q5d). This script is also the
  natural home for a REQ-04 reproducibility assertion — it already builds a commit in
  isolation.
- **Git hooks.** `scripts/hooks/pre-commit`, `scripts/hooks/pre-push`,
  `scripts/hooks/README.md` — none mentions `profiles/`, `include_dir`, or build provenance;
  they enforce lease/claim validation and branch-drift checks. Not installed in this
  checkout (`.git/hooks/` holds only `*.sample`; `core.hooksPath` unset), so they do not run
  today. **No `.githooks/` directory exists.**

---

## 5. Primitive verification

| Property the plan will assert | Verdict | Evidence |
|---|---|---|
| `build.rs` runs to completion before the crate's own compilation (and thus before `include_dir!` expansion) | **Confirmed** | Cargo book, `.../doc/rust/html/cargo/reference/build-scripts.html` rendered lines 221-234: "Just before a package is built… Once the build script successfully finishes executing, the rest of the package will be compiled." |
| `include_dir!` accepts an `$OUT_DIR`-rooted path | **Confirmed** | `include_dir_macros-0.7.4/src/lib.rs:26` → `resolve_path` `:163-190`; expands any `$IDENT` via `get_env` `:176`; stable `get_env` = `std::env::var` `:258-261`. No variable allowlist. |
| `include_dir` with `default-features = false` embeds no file metadata | **Confirmed** | `lib.rs:99-102` calls `.with_metadata` only when `metadata()` is `Some`; `lib.rs:110-112` returns `None` unless `cfg!(feature = "metadata")`. `include_dir-0.7.4/Cargo.toml:60-62` `default = []`. `crates/jit/Cargo.toml:43` sets `default-features = false`. `Cargo.lock:1171-1177` shows no optional deps pulled. |
| `include_dir!` registers each embedded file with cargo for change detection | **Confirmed** | `lib.rs:81-90` emits `include_bytes!(<abs path>)`; `target/debug/jit.d` and `target/debug/libjit.d` each list all 66 `profiles/jit-dogfood/...` paths. |
| `include_dir!` registers *directory membership* with cargo | **Contradicted** | `lib.rs:263-266` — `track_path` is a no-op unless the `nightly` feature is on; the dep-info files list only files, no directory entries. Adding a file to the package tree does not invalidate the crate today. |
| Directory-level `cargo:rerun-if-changed` suffices to catch edits anywhere beneath | **Confirmed** | Cargo book, same page, lines 501-502: "If the path points to a directory, it will scan the entire directory for any modifications." Detection is mtime-based (lines 497-500). |
| OUT_DIR is a clean slate each build | **Contradicted** | Cargo book lines 254-260: "Cargo does not clean or reset `OUT_DIR` between builds… the script is responsible for managing or cleaning up any files or subdirectories it creates." A deleted live consumer would linger in the derived package. |
| `jit profile show --json` is a stable machine surface exposing the whole manifest | **Confirmed** | Ran it: emits `manifest.{profile,contribution,asset,region}` with the full `plan` contribution value. Command in `crates/jit/src/commands/profile.rs`; wire shape also asserted through MCP at `mcp-server/test-integration.js:356-357`. |
| The docs-mechanical checkers derive both comparison sides live | **Confirmed** | `docs-check-projections.sh:12-16` (targets from `jit project render --json`); `docs-check-links.sh:8-11`; `docs-check-citations.sh:6-7`; gate description `.jit/gates.toml:171`. |
| The package's own manifest↔files completeness is already enforced | **Confirmed** | `ProfilePackageError::ExtraContent` `crates/jit/src/profile/package.rs:155`, raised `:302-309`; `MissingContent` raised `:298-300`. |
| Region splicing is byte-oriented and format-agnostic (usable on TOML) | **Confirmed** | `crates/jit/src/repository_state/managed_document.rs:32-34` (`Vec<u8>` delimiters), `:65-109` (byte-preserving outside claimed regions). |
| The `[projection.*]` surface can generate a TOML `[[template]]` block | **Contradicted** | `ProjectionConfig.kind` requires addressable item kinds (`crates/jit/src/config.rs:1152-1156`); no `template` kind exists; every body renderer emits markdown (`crates/jit/src/repository_state/projection.rs:1-33`). |
| REQ-04's byte-for-byte reproducibility already has coverage | **Contradicted** | The strongest existing property is identical *reported provenance JSON* across two identically-injected builds (`crates/jit/tests/provenance_contract/version_cli_tests.rs:135-139`). No test compares binary bytes or embedded-package hashes across builds. |
| The provenance fixture can build a derived package | **Contradicted** | `crates/jit/tests/provenance_contract/repository_inventory.rs:60` bans `.jit` from the seeded repository; `.jit/reference/content-standards.md` is a packaged live asset (`profiles/jit-dogfood/manifest.toml:559-561`). |
| The build-footprint budget would notice a ~430 KB OUT_DIR tree | **Contradicted** | `scripts/rust-build-budget.sh:139-143` counts cargo targets; `:148-166` sums test-executable file sizes. Neither reads OUT_DIR. |

---

## 6. Architecture fit

**Primitives to reuse**

- `render_managed_document` (`crates/jit/src/repository_state/managed_document.rs:65`) — the
  byte-preserving region splicer, already exercised on a nested-region case by the existing
  drift test (`crates/jit/src/profile/dogfood.rs:492-513`). Reuse it for the `.jit/templates.toml`
  generated region rather than writing a second splicer.
- `EmbeddedProfilePackage::from_dir` + `validate_manifest` (`crates/jit/src/profile/package.rs:42,232`)
  — already rejects undeclared package files (`ExtraContent`) and missing declared sources
  (`MissingContent`). A derived tree gets both checks for free.
- `jit profile show --json` — the machine surface D-1's generator reads; no new CLI needed (C5).
- `HierarchyTemplate::generate_config_toml` (`crates/jit/src/hierarchy_templates.rs:63`) —
  the shipped code path D-4's generator exercises through `jit init`.
- `repository_inventory::repository_inputs` (`crates/jit/tests/provenance_contract/repository_inventory.rs:29`)
  — the established `git ls-files --cached --others --exclude-standard` idiom, if REQ-05's
  check goes the git-tracked route.
- `docs-check-projections.sh` — the structural model for a D-4 freshness checker (derive
  targets live, re-render, `git diff`).
- `crates/server/build.rs:2` — in-repo prior art for a directory-level `rerun-if-changed`.

**Layer boundaries to respect** (CLAUDE.md "Separation of Concerns")

- `crates/jit/build.rs` is outside the layered library entirely; it may do filesystem I/O
  freely. It must not import from `crates/jit/src/` (a build script cannot depend on the crate
  it builds), so any shared logic between the derivation and the runtime package model has to
  be duplicated or extracted into a separate crate/module compiled by both. **The plan should
  state which, since a duplicated asset-inventory parser would itself be a second copy.**
- `domain/` and `graph/` must stay pure and I/O-free. `build_provenance.rs` lives in `domain/`
  and is pure (`is_binary_build_input` is a string-prefix predicate,
  `build_provenance.rs:130-136`); REQ-06's change is adding entries to a `const &[&str]`
  (`:117-127`) — no I/O introduced.
- Storage/repository-state owns persistence; a documentation generator that runs `jit init`
  into a temporary directory is a *repository-local script*, not engine code, and must live
  under `scripts/` (D-4 already places it there implicitly).

---

## 7. Architectural-invariant check

### `@/invariant/domain-agnostic`

"Engine logic is domain-agnostic: type names, label vocabularies, gate keys, templates, and
workflow shapes come from repository configuration (`.jit/`), never from hardcoded domain
assumptions. The one sanctioned exception is the planning-bracket preset trio…"

| Change | Verdict |
|---|---|
| REQ-03/REQ-05: build.rs derives the packaged tree from live roots | **Risk.** The list of live roots (`.agents/skills/`, `scripts/`, `contrib/gates/`, `.jit/reference/`) is a *repository-local* fact. If it is hardcoded in `crates/jit/build.rs`, it lands in a file that ships with the crate — but build.rs is not engine logic and does not affect adopter behaviour, and the packaged profile is by definition jit's own workflow package. **Cleanest placement: declare the roots and exclusions in `profiles/jit-dogfood/manifest.toml` itself** (D-2 already says "declared root-and-exclusion list"), so build.rs reads them from data rather than embedding them. That keeps the domain literal in the package, where it belongs, and gives REQ-05's test the same declaration to read. |
| REQ-02/D-1: generated `plan` region in `.jit/templates.toml` | **Preserved.** The `plan` template stays repository configuration; the manifest that authors it is package data, not engine code. No new engine hardcoding. Watch that the generator itself does not hardcode role names (`planning`/`breakdown`) — it should serialize whatever the contribution contains. |
| REQ-01/D-3/D-4: generated `[documentation]` blocks | **Preserved.** `SHIPPED_DOCUMENTATION_POLICY` (`crates/jit/src/config.rs:488`) is already the sanctioned scaffolding constant, explicitly documented as "scaffolded configuration" (`config.rs:483-486`) that an adopter reclassifies. Deriving documentation *from* it introduces no new engine assumption. The generator is a repository-local script. |
| REQ-06: adding roots to `BINARY_BUILD_INPUTS` | **Preserved.** The inventory is already a repository-local path list in `domain/`, documented as such (`build_provenance.rs:110-116`). Adding `.agents/skills/`, `scripts/…`, `contrib/gates/`, `.jit/reference/` extends an existing repository-local list; it introduces no domain vocabulary. **But note:** this const ships in the adopter binary and is checked against *any* repository the binary validates. Entries are repository-shaped facts about *jit's own* checkout that will be evaluated in an adopter's tree. That is already true of `profiles/jit-dogfood/` today; the change widens it. Worth an explicit line in the plan's decision log. |

### `@/invariant/single-source-prose`

Every planned change *reduces* hand-maintained copies. Two residual copies the plan should
name rather than leave silent:

- `.jit/gates.toml:171` — the `docs-mechanical` gate description enumerates its three
  checkers by name. Adding a fourth makes this a stale enumeration.
- `docs/reference/profiles.md:56-58` — describes the manifest's contents in prose; D-2's
  root/exclusion list makes that description incomplete.
- `.jit/config.toml:9-49` — a third hand-maintained copy of the documentation lists,
  claiming (`:10-11`) to adopt the shipped classification verbatim, with nothing binding it.
  Out of REQ-01's scope (it is repository-local policy, not adopter documentation), but the
  invariant applies. Flag; do not silently expand scope.

### `@/invariant/bounded-rust-build-footprint`

- **Test-target count is the binding constraint: 11 of 12 used** (§Q8). At most one new
  integration-test target. Both REQ-02's and REQ-05's guards should go into an existing suite
  or an inline `#[cfg(test)]` module.
- OUT_DIR bytes are unmeasured and would grow by ~430 KB per profile — real but invisible to
  the budget, and not a violation of the invariant's stated clauses.
- **The genuine exposure is invalidation, not size**: a directory-level `rerun-if-changed`
  makes the build script mtime-sensitive to branch switches, and an unconditional OUT_DIR
  rewrite would then relink every test target — the regression jit:5d862134 removed and the
  invariant's rationale (`dev/active/73482aa1-rust-build-efficiency.md:58-62`). The plan
  should require a content-compare-before-write copy and cover it with an extension of
  `build_provenance_metadata_stability_tests.rs`'s existing "zero non-fresh test artifacts"
  assertion.

### CLAUDE.md dogfooding boundary

"Derive shipped behavior from initialization code and templates, not from this repository's
live `.jit/` contents."

- **D-4 satisfies it by construction** — it reads what `jit init` writes into a throwaway
  directory, i.e. the initialization code path, not `.jit/config.toml`. C7 confirms the
  alternative (`jit config get documentation`) would violate it.
- **D-1 sits on the boundary and should be stated carefully.** The manifest is packaged
  *shipped* content; `.jit/templates.toml` is *repository-local* configuration. D-1 makes the
  shipped artifact the authority for the repository-local one — that is the correct direction
  for the boundary and should be said so explicitly, because REQ-03 points the *other* way
  for assets (§3, item 2).
- **REQ-05's completeness check is repository-local by nature** (it asks "is every live
  consumer packaged?"), so its declared roots and exclusions are repository-local policy.
  Keeping them in `profiles/jit-dogfood/manifest.toml` keeps that fact inside the package
  rather than in engine code.

---

## 8. Open risks — load-bearing items not fully verified

1. **REQ-04 "byte-for-byte reproducible" has no existing test and no verified definition.**
   Nothing in the repository compares two builds' binary bytes. Even in principle, a plain
   `cargo build` of this workspace is not obviously bit-reproducible across target
   directories (absolute paths reach debuginfo). **Unverified.** The plan should either
   restate REQ-04's observable as "the embedded package content is identical across builds
   from identical sources and identical explicit environment" — extensible from
   `version_cli_tests.rs:104-155`'s existing shape and mechanically checkable via
   `EmbeddedProfilePackage::hashes().package` — or accept new, genuinely byte-comparing
   coverage as scope. Asserting the strong form without grounding would be a defect.

2. **Whether a build.rs derivation can avoid relinking on a branch switch is unverified in
   this workspace.** The mechanism (content-compare before write, preserving OUT_DIR mtimes)
   is sound in principle, but `rerun-if-changed` fires on mtime and I did not measure whether
   an unchanged-content re-run leaves every test target fresh here. This is the direct
   descendant of jit:5d862134's ~179 s regression and deserves a measured check, not an
   assumption.

3. **The OUT_DIR-persistence hazard (deleted live consumer lingering in the derived package)
   is documented by cargo but not exercised anywhere here.** It is a distinct failure from
   the "silently stale binary" the issue Notes name and needs its own coverage.

4. **The build-script/library code-sharing question is unresolved.** A build script cannot
   import the crate it builds. If the derivation must parse
   `profiles/jit-dogfood/manifest.toml`'s asset/root declarations, and the runtime package
   model parses the same file, the parser is duplicated unless extracted. Duplicating it
   would create exactly the class of second copy this epic removes. I did not find an
   existing shared-build-logic crate in this workspace to reuse.

5. **`scripts/docs-check-citations.sh:127` excludes `*/profiles/*/assets/install/*` from
   citation scanning.** I did not determine whether removing `assets/live/` from the tree
   changes what that exclusion protects, or whether the live sources themselves then need
   an exclusion. Cheap to check when the derivation shape is chosen.

6. **`docs/examples/{sdd,research}/templates.toml` carry two further `plan` declarations**,
   one sharing a description string verbatim with `.jit/templates.toml:30`. D-1 does not
   cover them and REQ-02 does not require it. Whether a reviewer will treat that as an
   incomplete REQ-02 is a judgment call the plan should pre-empt with an explicit
   out-of-scope note.

7. **Neither `scripts/hooks/pre-commit` nor `scripts/hooks/pre-push` is installed in this
   checkout** (`.git/hooks/` holds only samples; `core.hooksPath` unset). Any plan step that
   relies on a hook to enforce something will not fire for contributors in this state.
