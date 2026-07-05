# Changelog

All notable changes to Just-In-Time (JIT) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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

### Migration

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
