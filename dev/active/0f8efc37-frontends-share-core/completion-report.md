## Epic Complete: Run the server and web UI on the core's shared mechanisms (0f8efc37)

The server and web UI now construct storage, search issues and documents, and read document versions through the core's shared mechanisms; refreshes share one issue capture, and the browser consumes graph results instead of re-downloading issues.

### Success Criteria

- [x] REQ-01 shared core constructor — delivered by a236e81e (foundations checkpoint)
- [x] REQ-02 layout binding fails at construction — delivered by 9c3eb2bf
- [x] REQ-03 storage-owned watcher allowlist — delivered by 1d2235b1
- [x] REQ-04 configuration change events reload hierarchy — delivered by 1d2235b1, cd0ac0f7
- [x] REQ-05 no `/issues` request on change; graph supplies labels and titles — delivered by 83b561ed
- [x] REQ-06 one captured snapshot per tracker version — delivered by 6326532e
- [x] REQ-07 one executor helper off async workers — delivered by 0f302914
- [x] REQ-08 one semantic issue search; ripgrep only for linked documents — delivered by 4d8791fb, b067c2db, 1471c12a
- [x] REQ-09 one core document content/history/diff family — delivered by d4172016
- [x] REQ-10 unused routes and client methods removed — delivered by 4538b071
- [x] REQ-11 server binary compiles library modules once — delivered by abf68973
- [x] REQ-12 one raw-document response builder — delivered by ca44bb7c
- [x] REQ-15 indexed layout and refresh passes — delivered by 1cdf5965

Stories: c8e79465 (server) and 3a61ff16 (browser) passed repo-validate and code-review. Epic repo-validate and holistic-review passed.

### Key Decisions

- Released the stale d4172016 claim (its claimant's epic f4d9502a was complete), preserved the prior worker's uncommitted diff as a salvage commit, and resumed from it rather than redoing it.
- Failed d4172016 lead review when Git-less HTTP history regressed from an empty list to HTTP 500; the steward ruled the Git-less answer is an empty list decided once in core `document_history` (`@/charter/D-4`, `@/charter/D-20`).
- Treated the d4172016 code-review finding (combined route test) as in scope under `@/inv/orthogonal-tests`.
- Reconciled the abf68973 advisory: `satisfies` denotes contribution (`.jit/config.toml` `[namespaces.satisfies]`); server REQ-01 is delivered jointly by the route-level children.

### Escalations

- 38df8a6c installer policy: the user authorized the exact `allowScripts` entry `esbuild@0.25.12`; the verification install was denied by session auto-mode until the user ran `npm ci --offline` directly. Landed as 872f3def0.
- Reviewer model: changing gate reviewers outside `.jit/gates.toml` was declined; the user approved moving all review gates to gpt-6.1-sol (be302dc9c).
- Holistic F1 fix shape, search path contract and rg stdin handling: steward rulings applied in 1471c12a.

### Issues Discovered During Execution

- 1471c12a — Return repository-relative paths from linked-document search (wave 7, holistic-review run ea3ab36e F1: absolute linked-document paths rejected by document routes).

### Holistic Quality Notes

- Holistic re-run passed with two low advisories: a stale overview sentence in `docs/reference/cli-commands.md` claiming document history requires Git (forwarded to the steward), and the absence of a retained transcript in `nonblocking-install-verification.md` (steward ruled no action).
- `jit search` result paths changed from absolute to search-root-relative, matching `SearchResult.path`'s contract; recorded in CHANGELOG.
