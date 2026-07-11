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

### 3. Incomplete command-topology enumeration in `cli-command-grammar.md` (REQ-01 — rework, attempt 1)

The noun/bare-verb enumeration (`:23-35`) read as exhaustive but omitted four
top-level families present in `jit --schema`. Reconciled against `cli.rs`:
`reference` (`cli.rs:295`, `Reference(ReferenceCommands)`) and `migrate`
(`cli.rs:409`, `Migrate(MigrateCommands)`) are subcommand groups → added to the
nouns list. `list` (`cli.rs:61`) and `rdeps` (`cli.rs:177`) are top-level
**convenience aliases** routing to `jit issue list` / `jit graph rdeps` → added a
"Top-level convenience aliases" note rather than listing them as nouns. Bare-verb
list (init/status/validate/search/version/recover/serve/apply) was already
complete. Swept the footprint: this is the only command-topology enumeration.

### 4. Event-log contract wrong in `storage-format.md` (REQ-01/REQ-06 — rework, attempt 1)

Two gaps missed on the first pass (which checked only that the *listed* tags
matched source, not completeness or the universal-field claim):
- **(a) `issue_id` is not universal.** Verified against the full `Event` enum
  (`crates/jit/src/domain/types.rs:1308-1581`, 20 variants): 5 are repo-/
  registry-scoped and carry NO `issue_id` — `document_archived` (`:1426`),
  `gate_definition_created`/`updated`/`removed` (`:1544`/`:1531`/`:1557`),
  `lifecycle_timestamps_backfilled` (`:1573`). Reworded the "every event carries
  `issue_id`" claim: `id`+`timestamp` universal, `issue_id` issue-scoped only.
- **(b) event-type list was a 10-of-20 subset.** The authoritative `get_type`
  match (`types.rs:2010-2029`) has exactly 20 arms (an exhaustive match — the
  count is **20**, not 21). Replaced the subset with the source-of-truth citation
  (`Event` enum in `types.rs`) plus all 20 tags, grouped issue-scoped (15) vs
  repo/registry-scoped (5).

### 5. Overstated stale-binary field preservation in `storage-format.md` (REQ-01 — rework, attempt 2)

The "Compatibility" paragraph (`:120`) claimed a stale binary "neither drops nor
misreads" the additive lifecycle-timestamp fields — the "neither drops" half is
false. Verified: the `Issue` struct (`crates/jit/src/domain/types.rs:482`) has an
explicit field per key with `#[serde(default, skip_serializing_if=...)]` and NO
unknown-field capture (no `#[serde(flatten)]` catch-all, no `deny_unknown_fields`
anywhere in the file). Without `deny_unknown_fields` an old binary READS a newer
file without error (unknown keys ignored) and does not misread existing data —
which is the real reason `schema_version` stays `2` — but unknown keys are not
captured, so a stale-binary rewrite (any mutation → full save) DROPS them.
Reworded to: read-safe (justifies no schema bump) but not write-preserving (stale
rewrite drops unknown keys); a current binary round-trips them and the
`jit migrate lifecycle-timestamps` backfill can reconstruct them from the event
log. Sweep: this was the only stale-binary/round-trip field-preservation claim in
the footprint (configuration.md:457 unknown-key exit, rules-and-gates.md:14
projection byte-preservation, item-addresses.md:48 address round-trip are all
unrelated and correct).

## Missing-projection-surface facts (REQ-06 — recorded for follow-up filing)

- **Per-command exit-code mappings have no projection surface.** `jit --schema`
  projects the global exit-code *taxonomy* (0/1/2/3/4/5/6/10) but not which
  error each command returns. `claim.md` and `worktree-validate.md` hand-state
  per-command codes that live only in `crates/jit/src/main.rs`
  (`error_to_exit_code`) and the per-command `std::process::exit(...)` calls.
  These are cite-source-only today; a follow-up could project a
  command→exit-code map into `--schema` so the reference tables derive rather
  than hand-copy. (Group-C follow-up input.)
- **The event-tag set has no projection surface.** `jit --schema` does not emit
  the `Event` enum's tag set, so `storage-format.md`'s event-type list is
  cite-source-only (`crates/jit/src/domain/types.rs`). It is now cited to source
  rather than hand-copied, but a follow-up could project the tags (and their
  scope: which carry `issue_id`) so the list derives. (Group-C follow-up input.)

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

## Correction to first-pass "verified clean" claims (rework attempt 1)

The first-pass report over-claimed on two counts, both corrected above:
- "storage-format.md fully verified clean" held only for the tags that WERE
  listed; it missed event-type-list completeness (10 of 20) and the false
  universal-`issue_id` claim (findings §4). storage-format.md is now accurate for
  the full 20-variant `Event` enum.
- "all CLI claims verified" missed that cli-command-grammar.md's noun/bare-verb
  enumeration was an incomplete subset of the top-level families (finding §3).
The first pass verified that stated facts matched source but did not test
enumerations for completeness against the full `--schema` / `Event`-enum sets;
this pass does.
