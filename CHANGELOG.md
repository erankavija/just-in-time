# Changelog

All notable changes to Just-In-Time (JIT) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **`jit config get` now covers the whole configuration surface.** The
  dotted-key accessor previously recognized only a hand-mapped subset
  (`worktree.*`, `coordination.*`, `global_operations.*`, `locks.*`,
  `events.*`); it now walks the full `config.toml` schema generically,
  including `type_hierarchy` (e.g. `type_hierarchy.strategic_types`,
  `type_hierarchy.types.epic`), `namespaces` (e.g.
  `namespaces.type.unique`), `item_kinds`, `documentation`, `validation`,
  `project`, and `version`. An intermediate key returns the whole subtree
  (`jit config get documentation`) rather than erroring; an unknown
  top-level key fails exit 2 naming the valid sections, and an unknown
  nested key fails exit 2 naming the missing segment. The five
  system/user/repo-layered sections keep resolving exactly as before;
  every other section reads the repo's `config.toml` only, with no
  built-in defaults layered in.

- **Wrong-verb hints for observed wrong-guess spellings.** `jit dep
  remove`/`delete`, `jit issue rm`/`remove`/`complete`/`edit`, `jit gate
  rm`/`delete`, `jit doc rm`/`delete`, and `jit label add`/`rm`/`remove` now
  fail fast (exit 2) with a message naming that group's canonical command
  (`jit dep rm`, `jit issue delete`, `jit issue update --state
  done`/`--label`/`--remove-label`, `jit gate remove`, `jit doc remove`)
  instead of clap's generic "unrecognized subcommand" error. These are hints,
  not new aliases — the wrong verb still fails, and canonical spellings are
  unchanged. `jit label --help` now also clarifies that the `label` group
  manages the namespace registry, not an issue's labels.

- **Repeatable, AND-combined `--label` filter across the query family.**
  `jit issue list`, the top-level `jit list` alias, the bare `jit query` form,
  and `jit query all`/`available`/`blocked`/`strategic`/`closed` now accept
  `--label`/`-l` multiple times; an issue is returned only when it matches
  every pattern given (wildcard `namespace:*` patterns still supported per
  occurrence). This matches `jit issue search --label`'s existing repeatable
  AND semantics. A single `--label` occurrence behaves exactly as before.

### Changed

- **Uniform JSON list envelope across list- and query-family commands.** Every
  command that emits a collection now wraps it in `{"count": N, "<collection>":
  [...]}`, where `count` equals the length of the collection and the key is
  plural and collection-typed. A single parse path now works for every
  list/query command — agents no longer need bare-array or dual-shape
  fallbacks. Collection keys: `issues` (`issue list`, `list`, `query
  all`/`available`/`blocked`/`strategic`/`closed`, `issue search`, multi-id
  `issue show`), `results` (`search`), `gates` (`gate list`), `presets` (`gate
  preset list`), `events` (`events tail`/`query`), `documents` (`doc list`),
  `assets` (`doc assets list`), `leases` (`claim list`/`status`), `namespaces`
  (`label namespaces`), `values` (`label values`), `templates` (`config
  list-templates`), `items` (`item list`/`search`), `worktrees` (`worktree
  list`), `roots` (`graph roots`), `dependents` (`graph rdeps`, `rdeps`),
  `nodes` (`graph deps`), `results` (`gate status --all`/`--limit`),
  `gates` (`gate status-all`), and `findings` (`invariant check`). Inner
  record shapes are unchanged. Where a command already carried an aggregate
  count, `count` is the size of the named collection specifically: `graph deps`
  keeps `summary.total` (unique dependencies across the whole tree) while `count`
  is the number of top-level `nodes`; `gate status-all` keeps `total` / `passed`
  (readiness tallies) while `count` is the number of `gates` entries.
- **Unified gate field naming across JSON outputs.** Everywhere a gate appears
  in `--json` output it is now identified by `key` and its state by `status`,
  matching `issue show`'s existing `gates[].key` — the larger, pre-existing
  surface. See the Migration section below for the renamed fields.
- **Typed exit codes for prefix and batch-usage errors; JSON envelope for
  startup failures.** Argument-class failures that previously fell through to the
  generic exit `1` are now classified:
  - An **ambiguous id prefix** (matches multiple issues) and a **too-short id
    prefix** (fewer than 4 characters) are argument errors (exit `2`), carrying
    the distinguishing `code` `AMBIGUOUS_ID` / `INVALID_ID_PREFIX` under
    `--json`. Human messages are unchanged.
  - The **batch-mode usage guards** on `jit issue update --filter` (mutually
    exclusive id/filter, and the `--content-format` / `--type` / description-flag
    rejections) now exit `2`, matching clap's own usage errors, and emit a JSON
    envelope under `--json`.
  - The **misplaced query-filter guard** (`--state`/`--assignee`/`--priority`/
    `--label`/`--full`/`--json` given before a `jit query` subcommand, where they
    would be silently dropped) now exits `2` and emits a JSON envelope under
    `--json`, matching the other query-family usage guards.
  - `jit dep rm <from> <target>` now validates **both** id arguments identically:
    a too-short or ambiguous prefix in either position is the same argument error
    (exit `2`), where previously a short `<target>` was silently reported as "not
    found" (exit `0`) while a short `<from>` exited `1`.
  - `jit dep add <from> <target>...` now emits the refined `INVALID_ID_PREFIX` /
    `AMBIGUOUS_ID` code (exit `2`) under `--json` for a too-short or ambiguous
    prefix in either the `<from>` or any `<target>` position, instead of the
    generic `DEPENDENCY_ERROR` (exit `1`). The non-`--json` exit code was already
    `2`; this aligns the `--json` code with it.
  - **Startup failures under `--json`** (repository not found, repository format
    too new) now emit a structured error object on stdout (`code`
    `REPOSITORY_NOT_FOUND` / `REPOSITORY_FORMAT_TOO_NEW`) while keeping their
    exit codes (`3` / `10`) and the human line on stderr. Previously `--json`
    produced an empty stdout for these.

### Fixed

- **`jit dep add` with multiple targets is now atomic.** Previously, a variadic
  add (`jit dep add <from> <to1> <to2> ...`) applied edges one at a time, so an
  edge that failed validation (e.g. a redundant edge under the default
  `--reduce`-less policy) left any earlier, already-applied edges persisted —
  the exit code no longer meant "nothing changed." Every requested edge is now
  validated against the would-be-final graph (every edge of the call applied at
  once, so a violation that only emerges from the COMBINATION of two edges in
  the same call is also caught) before anything is written; if any edge fails,
  none of them are added and no event is logged for any of them. The error now
  names every rejected edge, not only the first, and under `--json` carries a
  `details.rejected` array of `{from, to, code, message}` per rejected edge. A
  batch mixing an id-resolution failure with a graph-validation failure exits
  with the resolution failure's code (resolution runs before graph
  validation).
- **Doubled "Error: Error:" prefix on `ActionableError` paths.** Any command
  surfacing an `ActionableError`-derived failure (e.g. an already-claimed
  lease, a missing acting identity, the claims-require-git failure) now prints
  exactly one `Error:` prefix. `ActionableError::to_error_message()` no longer
  embeds its own prefix; the top-level CLI printer is the sole place that adds
  it.
- **`jit issue claim` on an issue already assigned to a different assignee**
  now names the current assignee and states that re-claiming as that same
  assignee succeeds (and promotes it to `in_progress`), instead of the bare
  "Issue is already assigned".
- **`jit claim acquire`/`jit claim release` outside a git repository** now
  distinguishes two causes instead of always suggesting `git init`: no git
  repository at all (still hints `git init`) vs. a git repository with no
  commits yet, so `HEAD` doesn't resolve to a branch (hints making an initial
  commit instead). Same typed error and exit code (`10`) for both; only the
  message differs.
- **Help text cross-references between assignment and lease commands.**
  `jit issue assign`/`claim`/`release`/`unassign` (assignee bookkeeping) and
  `jit claim acquire`/`release` (exclusive, time-boxed leases) share verbs but
  are different mechanisms; each command's `--help` now names its counterpart.

### Migration

- **BREAKING — `jit dep add` with multiple targets no longer partially
  applies on failure.** A variadic add where one target fails validation used
  to leave every edge before the failure persisted; it now leaves the
  dependency set completely unchanged. Scripts that relied on the partial
  application (e.g. retrying only the failed target) must instead retry the
  whole batch. The `--json` success/error response no longer includes an
  `errors` array — a failure is now the command's `Err`/nonzero-exit path,
  carrying every rejected edge under `error.details.rejected` instead.
- **BREAKING — exit codes for prefix and batch-usage errors changed from `1` to
  `2`.** Scripts that branch on the exit code of an ambiguous/too-short id prefix,
  a `jit issue update --filter` usage guard, a misplaced pre-subcommand `jit
  query` filter, or `jit dep rm` with a bad id must
  treat `2` (invalid argument) as the failure code for these cases. A short
  `<target>` to `jit dep rm` that previously succeeded (exit `0`, reported under
  `not_found`) now fails with exit `2`; pass a ≥4-character prefix or the full id.
  `jit dep add` with a too-short/ambiguous prefix already exited `2`, but its
  `--json` `code` changes from `DEPENDENCY_ERROR` to `INVALID_ID_PREFIX` /
  `AMBIGUOUS_ID`. Consumers on `--json` can branch on the new `code` values
  (`AMBIGUOUS_ID`, `INVALID_ID_PREFIX`) instead of the exit code. No
  human-readable messages changed.
- **Additive — startup failures emit JSON on stdout under `--json`.** Callers of
  any command with `--json` in an uninitialized repository, or against a
  repository whose on-disk format is newer than the binary, now receive a parsable
  `{"error": {...}}` object on stdout (previously stdout was empty). Exit codes
  (`3` / `10`) and stderr are unchanged; consumers that only read the exit code
  are unaffected.

- **BREAKING — `jit issue show <id> <id> …` with `--json`.** Passing two or more
  ids previously emitted a bare JSON array (`[ {…}, {…} ]`). It now emits the
  list envelope `{"count": N, "issues": [ {…}, {…} ]}`. Consumers that indexed
  the top-level array must read `.issues` instead (e.g. `jq '.issues[]'` in
  place of `jq '.[]'`). Single-id `issue show --json` is unchanged: it still
  returns a bare issue object.
- **BREAKING — `jit graph deps --json` node collection renamed.** The node
  collection key changed from `tree` to `nodes` (`{"count": N, "nodes": [...]}`).
  Consumers must read `.nodes` instead of `.tree`; each node's inner shape
  (including nested `children`) is unchanged, and `summary` / `issue_id` /
  `depth` remain alongside.
- **Additive — `count` field added.** `gate preset list`, `doc assets list`, and
  `gate status-all` gained a top-level `count` alongside their existing
  collection. `doc assets list` continues to carry its `summary` object; its
  `count` mirrors `summary.total` (the length of `assets`). `gate status-all`
  keeps its `results` / `not_run` / `total` / `passed` keys; its `count` is the
  length of `gates`, not a readiness tally. Existing consumers that
  ignored unknown keys are unaffected.
- **Documentation — `gate status --all`/`--limit`.** The history view already
  emitted `{"count": N, "results": [...]}`; the envelope is now stated in the
  command help and the CLI reference (no shape change).
- **BREAKING — gate identification unified to `key` across all JSON output.**
  Every place a gate previously appeared under `gate_key` now appears under
  `key`; `gate status-all`'s collection also renamed `gate_statuses` to
  `gates`. Affected shapes:
  - `gate status-all` (`check-all`): `{"count": N, "gates": [{"key": ..., "status": ...}], ...}`
    (was `gate_statuses`, entries carried `gate_key`). The `results` entries
    (one per recorded automated run) also switch from `gate_key` to `key`.
  - `gate status`/`check`: the latest-run view, the `--all`/`--limit` history
    view's `results` entries, and the `--stdout`/`--stderr` flat view all carry
    `key` instead of `gate_key`.
  - `gate evaluate`/`pass` and `gate fail`: the success response's `gate_key`
    field is now `key`.
  - `gate evaluate-all`/`pass-all`: each entry in the `gates` array carries
    `key` instead of `gate_key` (the `gates` collection key itself was already
    correct).
  - `gate define` and `gate remove`: the response's `gate_key` field is now
    `key`.
  - A gate-blocked transition error's `error.details.blockers[]` entries
    (`type: "gate"`) carry `key` instead of `gate_key`.
  - A failed `gate evaluate`/`pass` error's `error.details` carries `key`
    instead of `gate_key`, and its nested `checker_result` is now the same lean
    run-summary shape used elsewhere (`key`/`status`/...) instead of the raw
    stored gate-run record (so it also drops `schema_version` and a duplicate
    `issue_id`).
  - The web UI's single gate-run endpoint (`GET
    /issues/:id/gate-runs/:run_id`) now returns the same run-summary shape
    (`key`, no `schema_version`/duplicate `issue_id`) instead of the raw stored
    record.
  Consumers must read `.key` wherever they previously read `.gate_key`, and
  `.gates` wherever they previously read `.gate_statuses` from `gate
  status-all`. The on-disk gate-run storage format
  (`.jit/gate-runs/**`) and `events.jsonl` gate-related event entries are
  unaffected — both keep `gate_key` as an internal/audit field; only the
  `--json` command output surface changed.
