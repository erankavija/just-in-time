# Audit notes — Reference documentation audit (7c283e95)

Footprint: `docs/reference/` except `cli-commands.md` — 11 files, ~3,260 lines.
Binary verified at HEAD (`jit --version` commit `8e4acd98` = `git rev-parse HEAD`).

## Mechanical bar — results

- **M1 (invented-flag guard):** clean. Residue is all benign: `--path` (cargo
  flag in `cargo install --path`), `--quiet`/`--help`/`--schema` (global flags
  absent from `--schema`, present in `cli.rs`), `--add-label` (`visible_alias`,
  `cli.rs:986`).
- **M2 (links/anchors):** `OK: all links and anchors resolve`.
- **M3 (citations):** `OK: all cited paths and @/ items resolve`.
- **M4 (box-drawing / diagram-shaped art):** all hits are in
  `storage-format.md:14-27` (`.jit/` tree) and `:251-256` (`.git/jit/` tree) —
  both directory-tree listings, the known-benign carve-out. **Zero
  diagram-shaped art; REQ-04 satisfied, nothing to convert.**
- **M5 (projection freshness):** `OK: projections fresh`. `rules-and-gates.md`
  generated region (lines 22-53) is fresh; not hand-edited.

## Drift classes swept and fixed

### 1. Exit-code inaccuracy in `claim.md` (REQ-01/REQ-04)

`claim.md` documented resource-not-found conditions as exit `1`; the binary
returns exit `3` (`ExitCode::NotFound`, `main.rs:145-149` maps
`IssueNotFoundError`/`LeaseNotFoundError` → `NotFound`). Verified empirically in
an isolated temp repo with `JIT_AGENT_ID` set (renew/heartbeat need an agent
identity or they fail earlier with exit 1 on "no agent identity", which masked
the real code during a first pass):

- `acquire` issue-not-found → 3 (was 1)
- `release` issue-not-found / no-active-lease → 3 (was 1); "no acting identity"
  stays 1 (split into two rows)
- `renew` lease-not-found → 3 (was 1)
- `heartbeat` lease-not-found → 3 (was 1)
- `force-evict` lease-not-found → 3 (was 1); missing required `--reason` → 2
  (clap usage error, `InvalidArgument`), not 1

Sweep confirmed the class is isolated to `claim.md`: `worktree-validate.md` exit
codes are correct (`worktree info/list` no-git → 1 verified; `validate`
coordination-scope failure → 1, e.g. `--branch-drift` verified), `config
validate` exit codes (0/1/2) match `main.rs:5516-5519`, and `labels.md`
write-block exit 4 is correct (verified).

### 2. Invented CLI surface in `configuration.md` (REQ-01)

`configuration.md:455` referenced `jit invariant list`, which does not exist
(`jit invariant` has only `check`/`render`; the command exits 2). Fixed to
`jit item list --kind invariant` (verified working; invariants are addressable
items). Swept every `jit <cmd> <subcmd>` reference in the footprint against
`--schema`: this was the only invented one.

## Missing-projection-surface facts (REQ-06 — recorded for follow-up filing)

- **Per-command exit-code mappings have no projection surface.** `jit --schema`
  projects the global exit-code *taxonomy* (0/1/2/3/4/5/6/10) but not which
  error each command returns. `claim.md` and `worktree-validate.md` hand-state
  per-command codes that live only in `crates/jit/src/main.rs`
  (`error_to_exit_code`) and the per-command `std::process::exit(...)` calls.
  These are cite-source-only today; a follow-up could project a
  command→exit-code map into `--schema` so the reference tables derive rather
  than hand-copy. (Group-C follow-up input.)

## Judgment calls

- **Exit-code rows reordered ascending** in `claim.md` (e.g. `0,1,1,1,3`) so the
  mixed codes read cleanly; content, not just order, changed only where the code
  was wrong.
- **`rules-and-gates.md` left untouched** in its generated region (M5 fresh);
  only its hand-authored preamble (lines 1-21) was reviewed — accurate, no
  change.
- **`example-config.toml`** shows `bug`/`enhancement` types and a customized
  `types` map, but states the shipped default explicitly (line 43) and is framed
  as an editable example template, so no repo-local-vs-shipped signal defect.
- **`configuration.md:86` "(currently inert)"** for `strictness` is present-tense
  fact about current behavior, not legacy/future narration — kept.
- **Doc-link deferred to lead:** did not run `jit doc add` (hard rule: never
  write under `.jit/`; the lead is the sole writer). The lead should link this
  file to 7c283e95.
