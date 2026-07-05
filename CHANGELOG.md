# Changelog

All notable changes to Just-In-Time (JIT) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
  list`), `roots` (`graph roots`), `dependents` (`graph rdeps`, `rdeps`), and
  `findings` (`invariant check`). Inner record shapes are unchanged.

### Migration

- **BREAKING — `jit issue show <id> <id> …` with `--json`.** Passing two or more
  ids previously emitted a bare JSON array (`[ {…}, {…} ]`). It now emits the
  list envelope `{"count": N, "issues": [ {…}, {…} ]}`. Consumers that indexed
  the top-level array must read `.issues` instead (e.g. `jq '.issues[]'` in
  place of `jq '.[]'`). Single-id `issue show --json` is unchanged: it still
  returns a bare issue object.
- **Additive — `count` field added.** `gate preset list` and `doc assets list`
  gained a top-level `count` alongside their existing collection. `doc assets
  list` continues to carry its `summary` object; `count` mirrors `summary.total`
  (the length of `assets`). Existing consumers that ignored unknown keys are
  unaffected.
