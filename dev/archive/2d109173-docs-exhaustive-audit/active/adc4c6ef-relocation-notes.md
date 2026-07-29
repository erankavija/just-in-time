# Root documentation relocations — notes (issue adc4c6ef)

Record for the two owner-decided relocations (plan `2d109173` §3, decisions D7/D8).

## Relocation 1 — TESTING.md → dev/TESTING.md (EXECUTED, mandatory per D7)

Root `TESTING.md` is contributor documentation (jit's own three-layer test
strategy, TestHarness usage, source layout) and no adopter doc links to it. Moved
to the contributor-documentation home with history preserved:

- `git mv TESTING.md dev/TESTING.md`.

### Three live inbound links repointed (same change)

| File:line | Before | After |
| --- | --- | --- |
| `AGENTS.md:124` | `(see TESTING.md for details)` | `(see dev/TESTING.md for details)` |
| `dev/index.md:145` | `[TESTING.md](../TESTING.md)` | `[TESTING.md](TESTING.md)` |
| `dev/architecture/core-system-design.md:402` | `` See `TESTING.md` `` | `` See `../TESTING.md` `` |

### Five outbound links inside the moved file repointed (root-relative → `../`)

The moved file's own links were authored root-relative; moving it one level down
into `dev/` required prefixing each intra-repo target with `../` so it still
resolves:

- `[AGENTS.md](../AGENTS.md)` (two occurrences, lines 4 and 332)
- `[.github/copilot-instructions.md](../.github/copilot-instructions.md)`
- `[docs/reference/jit-content-standards.md](../docs/reference/jit-content-standards.md)`
- `[crates/jit/tests/harness.rs](../crates/jit/tests/harness.rs)`
- `[crates/jit/tests/harness_demo.rs](../crates/jit/tests/harness_demo.rs)`

The `https://docs.rs/proptest` external link was left unchanged.

### Left as-is (per D7 / task scope)

- Historical records under `dev/archive/` and `dev/studies/` keep their original
  `TESTING.md` mentions (records of past state, not repointed).
- `web/TESTING.md` is a distinct file, untouched.
- Planning records under `dev/active/` (`2d109173-plan.md`, `-investigation.md`,
  `-planning-brief.md`, `transitive-reduction-validation-plan.md`) describe the
  move itself; not live inbound links, left as-is.

### REQ-01 acceptance

`TESTING.md` exists at `dev/TESTING.md`, absent from the repository root; the three
live inbound links resolve to the new path; no adopter-facing doc or root file
references the old root location.

## Relocation 2 — INSTALL.md (EVALUATED → DECLINE, keep at root)

**Verdict: DECLINE the move. `INSTALL.md` stays at the repository root; no inbound
links change; its content is untouched.**

### Rationale

Under D8 the move executes only if it is a clean win, and the decision carries an
explicit escape hatch: *if packaging-doc convention or link topology argues for
keeping it at root, decline*. Both arguments apply here, so the disciplined verdict
is to keep it:

1. **Packaging-doc convention.** `INSTALL` is the archetypal top-level packaging
   file — the GNU Coding Standards enumerate it alongside `README` as a standard
   root document, and adopters cloning the repository (and tooling that surfaces
   install instructions) expect it beside `README.md`. This is precisely the
   "packaging-doc convention" the D8 escape hatch names as a reason to keep.
2. **Link topology.** `README.md:58` routes "all installation options" to
   `INSTALL.md` — a root-README → root-INSTALL relationship that is the
   conventional adopter path. Moving `INSTALL.md` into `docs/` would send the
   first file a visitor reads off into `docs/how-to/` for something the ecosystem
   expects at the root, and would force repoints into `docs/how-to/deployment.md`
   and `docs/tutorials/quickstart.md` — both Wave-2-audited files (handoff
   serialization risk) — for no net gain.
3. **Conventional-file class.** D8's stay-list (`README.md`, `CHANGELOG.md`,
   `AGENTS.md`) illustrates the conventional root files that remain; `INSTALL`
   belongs to the same long-established conventional class.

The owner's durable lean-root preference (adopter content → `docs/`) is real, but
it is expressly subordinated to the packaging-convention escape hatch for exactly
this kind of file. Net: not a clean win — declined.

The three inbound links (`README.md:58`, `docs/how-to/deployment.md:172`,
`docs/tutorials/quickstart.md:70`) already resolve to the root `INSTALL.md` and
need no change. The pre-existing Node-floor inconsistency inside `INSTALL.md`
(`:160` says 20+, `:261` says v18+) is out of scope for this relocation task — it
is a seed for the downstream "Root and component documentation audit" task, not a
relocation defect.

### REQ-02 acceptance

Evaluation verdict recorded with rationale; declined; `INSTALL.md` remains at root
with the declining rationale recorded here.

## Tree-wide verification

Checkers run over the adopter surface plus every file this change touched
(`docs/ README.md AGENTS.md mcp-server/README.md web/README.md INSTALL.md
dev/TESTING.md dev/index.md dev/architecture/core-system-design.md`):

- **M2 links & anchors** — `OK: all links and anchors resolve` (exit 0).
- **M3 citations** — `OK: all cited paths and @/ items resolve` over the adopter
  surface (exit 0). Note: adding `AGENTS.md` to the footprint surfaces a
  pre-existing `DANGLING: @/charter/D-N` at `AGENTS.md:94` — illustrative prose
  ("citable as `@/charter/D-N` via `per:` labels"), present in the base commit,
  untouched by this change, and outside the adopter citation footprint.
- **M5 projections** — clean after commit. The transient `DRIFT` seen while the
  change was uncommitted was this change's own `AGENTS.md:124` edit (the projector
  wrote nothing to the invariant region; `git diff --stat` showed exactly the one
  line). The `## Domain Invariants` projected region was not touched.

Note on the bare `dev/` walk: running the link checker over all of `dev/` reports
several pre-existing broken links in `dev/archive/`, `dev/studies/`, and
`dev/active/` records (e.g. `dev/archive/features/file-locking-usage.md:390`
`../TESTING.md` → `dev/archive/TESTING.md`, which never pointed at the root file
and was already broken before this change). These are outside the gate's derived
footprint (`docs/` + curated adopter set) and outside this task's scope.
