# Concepts documentation audit — notes (736a069e)

Footprint: `docs/concepts/` (9 files, ~3,339 lines). Binary verified at HEAD
(`jit 0.2.1`, commit `8e4acd98`). All source claims checked against HEAD.

## Mechanical-check results (scoped to `docs/concepts/`)

- **M1 (flag inventory vs `--schema` + `cli.rs`):** after fixes, the only residual
  flags are `--allow-empty` (a `git commit` flag, not jit) and `--quiet` (a real
  global flag, `crates/jit/src/cli.rs:29`). No invented jit flags remain.
- **M2 links/anchors, M3 citations, M5 projections:** all clean (`OK`).
- **M4 (box-drawing / diagram art):** all hits fall inside the two known-benign
  directory trees — `overview.md` `.jit/` tree and `guarantees.md`
  `.jit/`+`.git/jit/` tree. **Zero diagram-shaped ASCII/box art.** REQ-04 satisfied.

## Drift classes swept and fixed (all in `guarantees.md` / `core-model.md`)

1. **Wrong event-type surface (REQ-08 seed + class).** `guarantees.md` used
   externally-tagged PascalCase events (`{"IssueUpdated":{…}}`) and named several
   non-existent variants (`IssueUnclaimed`, `DependencyAdded`, `DependencyRemoved`,
   `GateChecked`). Real events are internally tagged, snake_case
   (`{"type":"issue_state_changed",…}`) per `crates/jit/src/domain/types.rs:1307`
   and the live `events.jsonl`. Fixed the event-type list, the JSONL examples
   (incl. the state-change example: `from`/`to`, not `field`/`old_value`/`new_value`),
   and `--event-type IssueUpdated` → `issue_state_changed`.
2. **Wrong claim-lock filename (REQ-07 seed).** `guarantees.md` lock list and the
   mermaid participant said `claims.index.lock`; the actual lock is
   `locks/claims.lock` (`crates/jit/src/storage/claim_coordinator.rs:400`),
   matching the tree at the bottom of the same file. Also corrected the per-issue
   lock `.issues/{id}.lock` → `issues/{id}.lock` (dir is `issues/`, not `.issues/`).
3. **Invented CLI surface.** `jit events tail --follow` (no `--follow` flag exists;
   schema/`cli.rs` show only `-n`/`--json`) → replaced the "event-driven" example
   with an explicit poll loop. `jit query all --filter "…"` (`query all` has no
   `--filter`; the boolean filter language lives on `issue claim-next` / `issue
   update` only) → reframed the "Boolean queries" block. `jit issue release <id>
   --reason "…"` (reason is a **positional** arg, `cli.rs`) → `jit issue release
   <id> "…"` (2 occurrences).
4. **Over-credited `jit validate` behavior.** Docs claimed `jit validate` cleans
   `.tmp` files "older than 5 minutes", evicts stale leases, and rebuilds the index.
   Verified `validate_with_fix` (`crates/jit/src/commands/validate.rs:49`) only
   applies type-hierarchy, transitive-reduction, and pending-transition fixes;
   `validate_leases` (`:1699`) only **reports** expired/dangling leases with fix
   commands. Temp cleanup is a **1-hour** recovery sweep on the **claim** path
   (`temp_cleanup.rs`, threshold `3600`, called from `claim.rs`/`claim_coordinator.rs`),
   not a `jit validate` action. Rewrote Partial-Write-Recovery, Stale-Temporary-Files,
   and Graceful-Degradation examples to the real behavior; `validate --leases`
   documented as reporting (not auto-evicting).
5. **Wrong `--json` output shape.** `core-model.md:583`
   `jit issue show --json | jq 'gates_status'` → `jq '.gates'`: the show `--json`
   output uses a `gates` array, while `gates_status` (object) is the **on-disk**
   field. The on-disk JSON structure block (`gates_required` + `gates_status`) is
   accurate and was left as-is. Also fixed the malformed filter value
   `--filter "epic:auth"` → `--filter "label:epic:auth"`.

## Volatile facts with no projection surface (REQ-06, cited to source, recorded for follow-up)

- **Event-type catalog** — no `jit` projection surface exists. Cited inline to
  `crates/jit/src/domain/types.rs`. *Follow-up candidate:* an event-type reference
  or `jit`-projected list (mirrors invariant/rule/gate projections).
- **Storage lock filenames & timeout constants** — `.index.lock`,
  `issues/{id}.lock`, `.events.lock`, `locks/claims.lock`; lock timeout default
  5 s (`json.rs:115`); temp-cleanup threshold 1 h (`temp_cleanup.rs`). No projection
  surface; `docs/reference/storage-format.md` is the reference doc. Prose now states
  the real constants ("one hour", "default 5 seconds" was already correct).

## Judgment calls (left unchanged)

- **Dogfooding illustrative output** (`design-philosophy.md` §Dogfooding:
  `jit query all` "Found 87 issues" with epic groupings; `jit gate list` showing
  `tests/clippy/fmt`). Left as-is: explicitly framed as "this very repository uses
  JIT" illustrative output, not product-total claims or a live-registry render.
  Rewriting to the current registry would chase a moving target and manufacture
  nitpicks against blocks the text already signals as illustrative.
- **`jit-server`** (`scope.md`, `design-philosophy.md`): valid — `crates/server`
  ships a `jit-server` binary (`crates/server/Cargo.toml`), distinct from the
  `jit serve` subcommand. Not drift.
