# Plan: Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e)

> Planning node: 0ba00a6f. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

`main.rs` becomes a thin entry: parse, open the repository through a library bootstrap, dispatch one command group to a library-side handler, render one outcome, and exit once. Handlers return typed responses that output owns and the schema derives from, and failures as values that one library classifier turns into the envelope, the exit code and the server's HTTP status. The executor is reached through per-family facades. Build hygiene closes the work: lints in the manifests, one shared harness, a `main.rs` ceiling, and a measured completion report.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | Each command group moves into a handler whose response type is owned by the output module (`handler-response-ownership`). | Inline bodies: investigation area 2 §1 |
| REQ-02 | `run()` shrinks to group dispatch as groups migrate; a final consolidation leaves `main()` as parse, bootstrap, dispatch, render, exit. | `run()` spans 6,428 lines: area 1 claim 1d |
| REQ-03 | Per-family facades borrowed from the executor (D-1); the shared kernel becomes crate-private. | 195 methods, 28 blocks, 25 files: area 3 §1 |
| REQ-04 | The eight per-query structs are dead and are deleted; every live list response becomes a `list-envelope` instantiation. | area 2 §2 |
| REQ-05 | `generate()` stays the authority; `get_output_schema_for_command` survives as a lookup into the handler registry; `hidden_commands()` and `builtin_global_flags()` stay where they are (D-12). | area 2 §4 |
| REQ-06 | `[workspace.lints]` denies `warnings` and `unsafe_code`; members inherit it (D-14). | area 3 Lints |
| REQ-07 | The harness moves into `jit`'s `test-support` surface (D-15); the completion report re-measures the budgets. | Included in 4 of 10 targets: area 3 Harness |
| REQ-08 | A fourth budget in `rust-build-budget.sh`, calibrated after consolidation (D-20), reported as a field of the existing success line. | area 3 Budget script |
| REQ-09 | Every deliberate change is listed in the decisions below and gets a test that states the new contract; `cli-contract-docs` states the contract once. | Suites: area 1 Prior art, area 2 Consumer sweep |
| REQ-10 | `production-line-count` counts before and after with one committed counter, in the Background's described method. | area 3 REQ-10 counting method |
| REQ-11 | `failure-classification` moves the classifier into output, then absorbs the details and suggestions call sites build today. | 15 emitters: area 1 §A |
| REQ-12 | Handlers return `Outcome` values (`handler-contract`); findings render through `findings-envelope`; `main()` holds the one `std::process::exit`. | 75 exits, 14 findings sites: area 1 claim 3, §D |
| REQ-13 | `type-only-error-codes`: a call site that needs a distinct code gets a typed error; codes nothing produces any more retire. | 50+ fallback sites: area 1 §B, §C |
| REQ-14 | Document path-read failures first get correct codes; the server then derives its status from `failure-classification` through one exhaustive code-to-status projection (D-10). | area 1 §E |
| REQ-15 | `output-mode`: one global `--json`; both argv scans disappear with the stubs and with threading the parsed mode out of `run()`. | area 2 §5 |
| REQ-16 | `issue-filter-args`: one flattened group with one conversion at the CLI boundary (D-8). | about 14 parse sites: area 2 §6 |
| REQ-17 | `handler-contract` registers each leaf's response type, including the pre-bootstrap `version` leaf; the hand table is deleted once every group has migrated. | 74 of 120 leaves untyped: area 2 §4 |
| REQ-18 | `list-envelope` replaces the 33 `count` structs and the inline count bodies; bare prints go through the one success emitter. | area 2 §2, §3 |
| REQ-19 | Mutations return the updated `Issue`; search, claims, foreground serve and repository bootstrap move behind the executor or `commands/` (D-11). | 20 bypass groups: area 3 REQ-19 table |
| REQ-20 | The 13 verb stubs are deleted; schema extraction skips everything clap marks hidden (D-7). | area 2 §7 |
| REQ-21 | Four shared renderers in output: issue list, validation report, gate run status, archive execution. | area 2 §8 |

## Shared architectural contracts

### `failure-classification` [implementation-produced] — One library error classifier

`jit::output::failure::classify(&anyhow::Error) -> JsonError` returns the code, message, details and suggestions for any error; the exit code is `ErrorCode::exit_code` of that code. It lives in `crates/jit/src/output/failure.rs`, a submodule declared from `output.rs`, which stays the module root (D-4). The downcast order of today's `error_to_error_code` is semantic and is preserved. The in-binary classifier test modules move with it and remain its owning tests.

### `type-only-error-codes` [plan-fixed] — An error's code depends only on its type

No call site chooses or substitutes a code. Where a call site needs a code its error type does not carry, the work introduces a typed error and classifies that type. A code the vocabulary still declares but nothing produces is retired from `ErrorCode`, the generated error-code page and the failure-lever registry. Each code change a user can observe gets a test stating the new code and exit status.

### `output-mode` [implementation-produced] — One global output mode

`Cli` declares `--json` once with `global = true`, beside `--quiet`. `OutputMode { json, quiet }` is built once from the parsed `Cli` and is the only source of the flag for handlers and for failure rendering. `run()` hands the mode back to `main()` with its result, so no code reads raw argv.

### `json-mode-streams` [plan-fixed] — One rendering per invocation

Once clap has accepted the arguments, a failing `--json` invocation writes exactly one envelope to stdout and nothing to stderr. In human mode a failure prints one `Error:` diagnostic to stderr. This retires the propagated-failure `Error:` line that is printed today alongside the envelope (D-6). Parse failures follow `parser-diagnostics-outside-envelope`, and success-path warnings stay on stderr as documented.

### `parser-diagnostics-outside-envelope` [plan-fixed] — Parse failures stay clap's

Clap parse failures, including a wrong-verb guess, remain outside the failure envelope, as container a2546471 decided. A wrong verb prints its hint and clap's diagnostic to stderr and exits 2, with or without `--json` (D-7).

### `findings-envelope` [plan-fixed] — Findings are failures with a report

A command whose findings make it exit non-zero returns them as a value. Under `--json` it renders the failure envelope: the registered code is the exit code, and the command's report is `error.details`. In human mode it prints the report and exits with the same code. This is the shape `render_validation_json` already produces for validation. `gate status-all`, `gate evaluate-many` and `doc check-links` move to it from a success envelope with a non-zero exit (D-5).

### `handler-response-ownership` [plan-fixed] — Output owns every success payload

Every success response a handler returns is a type defined in the output module (`crates/jit/src/output.rs`, with one submodule per command group under `crates/jit/src/output/` as needed) or is built by an output constructor that wraps a command-layer result. Command-layer result types stay in `commands/` so that dependencies keep pointing inward (D-3). Every response derives or implements `JsonSchema` and a human rendering.

### `list-envelope` [implementation-produced] — One generic list envelope

`output::ListEnvelope<K: CollectionKey, T, E = ()>` serializes `count` (derived from the collection), the collection under `K::KEY`, and the flattened fields of `E`. A matching manual `JsonSchema` implementation names the same keys. `CollectionKey` is a marker trait carrying `const KEY: &'static str`. A key belongs to the response rather than the item type, since `MinimalIssue` appears under several keys. `ProfileCollection<T>` converges onto it. A "count struct" is a struct whose `count` states the length of a sibling collection. Tallies such as `StateCount` and `ProjectionRenderReport.count` are not list envelopes (D-9).

### `issue-filter-args` [implementation-produced] — One issue-filter argument group

`cli::IssueFilterArgs` is a `#[derive(Args)]` group of `--state`, `--assignee`, `--priority` and repeatable `--label`, flattened into each issue-filtering command. One conversion method turns it into the executor's existing query filter at the CLI boundary. A bad state or priority reports `INVALID_ARGUMENT`, and a bad label pattern reports `INVALID_LABEL_PATTERN`. Label values are patterns, and an exact label is a pattern that matches only itself (D-8).

### `handler-contract` [implementation-produced] — Library-side command handlers

`crates/jit/src/dispatch/` holds one module per command group. A handler takes its parsed arguments and a `CommandContext` carrying the executor and the `output-mode`, and returns `anyhow::Result<Outcome<R>>`, where `R` is its registered response type. `Outcome` is either a success or a findings result carrying an `ErrorCode` (`findings-envelope`). One registration per group generates both the group's dispatch and the schema's per-leaf response lookup, so they cannot drift. `main()` renders an `Outcome` through one output emitter and a failure through `failure-classification`. Until every group has migrated, schema generation consults the registry first and the legacy table second (D-2).

### `executor-family-facades` [implementation-produced] — Narrow executor capabilities

`CommandExecutor` exposes one accessor per command family, such as `issues()` returning `IssueOps<'_, S>`. Each family's methods live on its facade, and their storage bounds move with them. A consumer that needs one family accepts that facade. The kernel (sessions, rule and config caches, the validation `capture_*` helpers) stays on `CommandExecutor` (D-1).

### `repository-bootstrap` [implementation-produced] — One library repository opener

One library function performs today's `run()` prelude: discover the data root, construct `JsonFileStorage`, configure worktree paths and layout, open or retain the mutation session, validate the store, and return the executor with the retained-session guard. `main.rs` calls it and makes no storage call of its own. 0f8efc37 REQ-01 adopts the same function for the server (D-11).

### `production-line-count` [plan-fixed] — Production line counting

Production lines are counted by `python3 scripts/production-lines.py <rev>`, committed by the baseline task from the investigation's [prodlines.py](prodlines.py). Its default mode is the method the epic's Background describes: it excludes `#[cfg(test)]` items, test-only sibling modules and test-named files, doc comments, and blank lines. That mode is the REQ-10 evidence, and the budget script counts `main.rs` with it through a working-tree single-file mode. The survey's figure of about 63,100 is reproduced only by `--strip-line-comments --test-support-as-test` (63,118 at 21f9e563, against 66,372 in the default mode), so the reports give the flagged figures beside the authoritative ones (D-13).

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| workspace-lints | Encode the zero-warning bar in workspace lints | task | [workspace.lints] denies warnings and unsafe_code, both crates inherit it, and the per-root #![deny(unsafe_code)] attributes are gone | — | D-14, F-LINTS, REQ-06 | touches 9 | — | — |
| shared-test-harness | Compile the integration harness once in jit's test-support surface | task | The in-process test harness lives in jit's test-support surface and no integration target #[path]-includes it | — | D-15, F-HARNESS, REQ-07 | touches 8 | — | — |
| production-line-baseline | Record the production-line baseline for crates/jit/src | task | A committed counter and a recorded per-module baseline of crates/jit/src production lines, pinned to a named commit | production-line-count | D-13, F-PRODLINES, REQ-10 | creates 2 | — | — |
| mcp-global-json-injection | MCP bridge injects --json from the schema's global options | task | The MCP bridge adds --json when the schema declares it per leaf or globally, without exposing a global json tool property | — | A24, F-MCP-JSON, REQ-15 | touches 3 | — | — |
| verb-hint-stub-removal | Wrong-verb hints come only from the parser-level path | task | The 13 hidden verb stubs are gone and COMMAND_HINTS is served by unknown_subcommand_hint on stderr with exit 2 | parser-diagnostics-outside-envelope | D-7, F-HIDDEN, REQ-09, REQ-15, REQ-20, S13 | touches 4 | — | — |
| global-json-flag | One global --json flag carried as OutputMode | task | --json is declared once on Cli and threaded as OutputMode from parse to failure rendering, with no per-command json field or argv scan | — | A24, F-JSON-FIELDS, REQ-09, REQ-15 | touches 6 | — | mcp-global-json-injection, verb-hint-stub-removal |
| failure-classifier-library | Move the error classifier into output::failure | task | jit::output::failure::classify replaces main.rs's error_to_error_code and error_to_exit_code with unchanged behaviour | — | A3, D-4, F-CLASSIFIER, REQ-11 | creates 1, touches 2 | — | — |
| classifier-derives-details | Derive failure details and suggestions from the error type | task | classify returns the details and suggestions call sites build today, derived only from the error value | failure-classification, type-only-error-codes | A3, F-CALLSITE-DETAILS, F-FALLBACK-SITES, REQ-11, REQ-13 | touches 5 | — | failure-classifier-library |
| list-envelope-type | Introduce the generic ListEnvelope type | task | ListEnvelope<K, T, E> and CollectionKey exist with serialization and schema tests, ProfileCollection uses it, dead query structs are gone | — | A25, D-17, D-9, F-COUNT-STRUCTS, REQ-04, REQ-18 | touches 3 | — | — |
| list-envelope-adoption | Express output-module count structs as ListEnvelope instantiations | task | The live count structs in the output module are ListEnvelope instantiations with unchanged wire shapes | list-envelope | A25, D-9, F-COUNT-STRUCTS, REQ-04, REQ-18 | touches 3 | — | list-envelope-type |
| handler-contract | Library-side command handler contract | task | dispatch/ provides CommandContext, Outcome, per-group registration feeding dispatch and schema, and one Outcome emitter in output | failure-classification, output-mode, list-envelope, handler-response-ownership, findings-envelope | D-2, D-3, D-5, F-HANDLER-SHAPE, REQ-02, REQ-05, REQ-12, REQ-17 | creates 1, touches 4 | — | global-json-flag, classifier-derives-details, list-envelope-adoption |
| issue-filter-args | Shared IssueFilterArgs group for issue filters | task | The issue-filtering commands flatten one IssueFilterArgs group whose single conversion parses state, priority, assignee and label patterns | — | A24, D-8, F-FILTERS, REQ-09, REQ-16 | touches 6 | — | global-json-flag |
| issue-list-renderer | Render issue lists through one output renderer | task | One output-module renderer prints the human issue list for each listing command, with deliberate format changes tested | — | D-18, F-RENDER-DUP, REQ-09, REQ-21, S14 | touches 2 | — | global-json-flag |
| executor-facade-issues | Issue methods behind an issues facade | task | `CommandExecutor::issues()` returns `IssueOps<'_, S>` carrying the issue-lifecycle methods, establishing the facade shape other families reuse. | — | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 21 | — | global-json-flag |
| executor-facade-gates | Gate methods behind a gates facade | task | The gate methods live on `GateOps<'_, S>` reached through `CommandExecutor::gates()`, with behaviour unchanged. | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 10 | — | executor-facade-issues |
| executor-facade-admin | Archive, snapshot, config, item, invariant and project methods behind an admin facade | task | The repository administration methods live on `AdminOps<'_, S>` reached through `CommandExecutor::admin()`, with behaviour unchanged. | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 13 | — | executor-facade-issues |
| executor-facade-queries | Query, search, event, graph and dependency methods behind a queries facade | task | The query and graph methods live on `QueryOps<'_, S>` reached through `CommandExecutor::queries()`, with behaviour unchanged. | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 16 | — | executor-facade-issues |
| handlers-issue-reads | Issue read commands move to library handlers | task | Issue read leaves and the list alias run as dispatch handlers returning typed output-owned responses, with no fallback error codes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode, issue-filter-args | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 4 | — | handler-contract, issue-filter-args, issue-list-renderer, executor-facade-gates, executor-facade-admin, executor-facade-queries |
| create-returns-issue | Issue creation returns the created issue | task | create_issue returns the new Issue with its warnings, so the CLI reports it without a storage reload | executor-family-facades | A26, F-BYPASS, REQ-19 | touches 9 | — | executor-facade-issues |
| mutations-return-issue | Issue mutations return the updated issue | task | Issue update, assignment, claim, reject and release methods return the updated Issue, so those CLI paths make no storage call | executor-family-facades | A26, F-BYPASS, REQ-19 | touches 4 | — | create-returns-issue |
| executor-facade-validation | Validation methods behind a validation facade | task | The validation methods live on `ValidationOps<'_, S>` reached through `CommandExecutor::validation()`, with behaviour unchanged. | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 10 | — | executor-facade-issues |
| handlers-issue-writes | Issue write commands move to library handlers | task | Issue mutation leaves run as dispatch handlers that report the Issue the executor returns, with no storage call in main.rs | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18, REQ-19 | touches 6 | — | handlers-issue-reads, mutations-return-issue, executor-facade-validation |
| handlers-dep | Dependency commands move to library handlers | task | dep add and dep rm run as dispatch handlers with typed responses and classifier-only error codes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 4 | — | handler-contract, executor-facade-queries |
| handlers-graph | Graph commands move to library handlers | task | graph leaves and the rdeps alias run as dispatch handlers with typed responses and list envelopes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 4 | — | handler-contract, issue-list-renderer, executor-facade-queries |
| handlers-gate-definitions | Gate definition and preset commands move to library handlers | task | Gate definition and preset leaves run as dispatch handlers with typed responses, classifier-only codes and a findings envelope for preset apply | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 5 | — | handler-contract, executor-facade-gates |
| gate-run-status-renderer | Render gate run status lines through one output renderer | task | One output-module renderer prints the not-run and passed gate run lines for gate status and the evaluate commands | — | D-18, F-RENDER-DUP, REQ-09, REQ-21, S14 | touches 2 | — | global-json-flag |
| handlers-gate-status | Gate status commands move to library handlers | task | gate status and status-all run as dispatch handlers with typed responses, a findings envelope for status-all and no storage call | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18, REQ-19 | touches 6 | — | handlers-gate-definitions, gate-run-status-renderer |
| handlers-gate-evaluation | Gate evaluation commands move to library handlers | task | The gate evaluation leaves run as dispatch handlers with typed responses and evaluate-many findings as the failure envelope | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | touches 6 | — | handlers-gate-status |
| executor-facade-documents | Document methods behind a documents facade | task | The document methods live on DocumentOps reached through CommandExecutor::documents(), with behaviour unchanged | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 7 | — | executor-facade-issues |
| handlers-doc | Document commands move to library handlers | task | doc, doc assets and check-links run as dispatch handlers with typed responses and check-links findings as the failure envelope | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 6 | — | handler-contract, executor-facade-documents |
| handlers-query-status | Query and status commands move to library handlers | task | query subcommands, bare query and jit status run as dispatch handlers returning typed list envelopes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode, issue-filter-args | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 5 | — | handlers-issue-reads, executor-facade-validation |
| validation-report-renderer | One output renderer for validation findings | task | One output-module function prints the findings block of each validation command | — | D-18, F-RENDER-DUP, REQ-09, REQ-21, S14 | touches 2 | — | global-json-flag |
| handlers-validate | Validate command moves to a library handler | task | The jit validate modes run through one dispatch handler that returns findings as VALIDATION_FAILED outcomes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 5 | — | handler-contract, validation-report-renderer, executor-facade-validation |
| handlers-config | Config commands move to library handlers | task | config leaves run as dispatch handlers with typed responses and the config check in the commands layer | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 4 | — | handler-contract, validation-report-renderer, executor-facade-admin |
| handlers-hooks | Hooks command moves to a library handler | task | hooks install runs as a dispatch handler with a typed response and its classified failure code | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 4 | — | handler-contract |
| claims-executor-family | Claim and recover operations behind a claims facade | task | Claim and recover operations are methods of a `claims()` facade, and the CLI passes no storage handle to them. | executor-family-facades | A26, D-11, F-BYPASS, REQ-03, REQ-19 | touches 5 | — | executor-facade-issues |
| handlers-claim-recover | Claim and recover commands move to library handlers | task | claim and recover leaves run as dispatch handlers through executor claim methods with typed responses and classifier-only codes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18, REQ-19 | creates 2, touches 4 | — | handler-contract, claims-executor-family |
| serve-foreground-command | Foreground serve as a commands-layer operation | task | `commands::serve::run_foreground` owns the foreground server lifecycle and returns its outcome; the CLI no longer exits from the serve arm. | — | A26, F-BYPASS, REQ-19 | touches 4 | — | global-json-flag |
| handlers-serve | Serve command moves to a library handler | task | jit serve runs as a dispatch handler over commands/serve with typed responses and no process calls in main.rs | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18, REQ-19 | creates 2, touches 4 | — | handler-contract, serve-foreground-command |
| executor-facade-profiles | Profile, init, template and breakdown methods behind a profiles facade | task | The profile and template methods live on `ProfileOps<'_, S>` reached through `CommandExecutor::profiles()`, with behaviour unchanged. | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | touches 10 | — | executor-facade-issues |
| handlers-profile | Profile commands move to library handlers | task | profile leaves run as dispatch handlers with typed responses, findings outcomes and classified codes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 5 | — | handler-contract, executor-facade-profiles |
| apply-returns-created-issues | Template application returns the created issues | task | `apply_template` returns the created issues by role, and `jit apply` reporting makes no storage call. | executor-family-facades | A26, F-BYPASS, REQ-19 | touches 4 | — | executor-facade-profiles |
| handlers-init-apply | Init and apply commands move to library handlers | task | init and apply run as dispatch handlers with typed responses, and apply reports created issues without a storage reload | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18, REQ-19 | creates 2, touches 4 | — | handler-contract, apply-returns-created-issues |
| handlers-label | Label commands move to library handlers | task | label namespaces and label values run as dispatch handlers returning list envelopes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 4 | — | handler-contract, executor-facade-issues |
| handlers-events | Events commands move to library handlers | task | events tail and events query run as dispatch handlers returning list envelopes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 4 | — | handler-contract, executor-facade-queries |
| handlers-snapshot-migrate | Snapshot and migrate commands move to library handlers | task | snapshot export and any remaining migrate leaf run as dispatch handlers with typed responses | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 5 | — | handler-contract, executor-facade-admin, executor-facade-validation |
| archive-execution-renderer | One output renderer for archive execution summaries | task | The archive execution summary prints through one output-module function for both archive commands | — | F-RENDER-DUP, REQ-21, S14 | touches 2 | — | global-json-flag |
| handlers-archive | Archive commands move to library handlers | task | archive leaves run as dispatch handlers whose payloads go through the success envelope as typed responses | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 4 | — | handler-contract, archive-execution-renderer, executor-facade-admin |
| handlers-worktree | Worktree commands move to library handlers | task | worktree leaves run as dispatch handlers with list envelopes and classified failure codes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 4 | — | handler-contract |
| handlers-item-invariant-project | Item, invariant and project commands move to library handlers | task | item, invariant and project leaves run as dispatch handlers with typed list envelopes and classifier-only codes | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 2, touches 7 | — | handler-contract, executor-facade-admin |
| search-through-executor | CLI search runs through the executor | task | One executor method serves `jit search` and returns matches with issue titles, so the search path calls neither `jit::search` nor storage. | executor-family-facades | A26, D-11, F-BYPASS, REQ-19 | touches 5 | — | executor-facade-queries |
| handlers-search | Top-level search command moves to a library handler | task | jit search runs as a dispatch handler over the executor search with a typed list envelope and one classification on both paths | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18, REQ-19 | creates 1, touches 5 | — | handler-contract, search-through-executor |
| handlers-version | Version command moves to a library handler | task | jit version runs as a registered pre-bootstrap handler with a typed response | handler-contract, handler-response-ownership, failure-classification, type-only-error-codes, findings-envelope, list-envelope, output-mode | A25, D-3, F-GROUP-ARMS, REQ-01, REQ-02, REQ-12, REQ-13, REQ-17, REQ-18 | creates 1, touches 3 | — | handler-contract |
| repository-bootstrap-constructor | Library repository bootstrap for the CLI | task | One library function opens the repository and returns the executor with its retained session; `main.rs` calls only it. | — | A26, D-11, F-BYPASS, REQ-19 | touches 5 | — | global-json-flag |
| json-silent-stderr | Keep stderr empty for --json failures | task | A failing --json invocation writes one envelope to stdout and nothing to stderr; human mode prints one Error: line | json-mode-streams, output-mode | D-6, F-STDERR-DUP, REQ-09, REQ-12 | touches 3 | — | global-json-flag, failure-classifier-library |
| main-entry-consolidation | Reduce main.rs to parse, bootstrap, dispatch, render and exit | task | main.rs only parses, bootstraps, dispatches, renders and exits, with the crate's single std::process::exit call | handler-contract, failure-classification, repository-bootstrap, json-mode-streams | A26, A3, F-DROP-SKIPPED, F-EXIT-SITES, REQ-02, REQ-11, REQ-12, REQ-19 | touches 4 | — | handlers-issue-writes, handlers-dep, handlers-graph, handlers-gate-evaluation, handlers-doc, handlers-query-status, handlers-validate, handlers-config, handlers-hooks, handlers-claim-recover, handlers-serve, handlers-profile, handlers-init-apply, handlers-label, handlers-events, handlers-snapshot-migrate, handlers-archive, handlers-worktree, handlers-item-invariant-project, handlers-search, handlers-version, repository-bootstrap-constructor, json-silent-stderr |
| main-rs-line-ceiling | Enforce a main.rs production-line ceiling in the budget script | task | rust-build-budget.sh fails when main.rs exceeds MAX_MAIN_RS_CODE_LINES and reports the value on its success line | production-line-count | D-20, F-BUDGET-SCRIPT, REQ-08 | touches 4 | — | main-entry-consolidation, production-line-baseline |
| path-read-error-codes | Classify document path-read failures by variant | task | PathReadError variants classify to INVALID_ARGUMENT or a document-not-found code instead of generic or repository-not-found codes | failure-classification, type-only-error-codes | D-10, F-SERVER-STATUS, REQ-09, REQ-13, REQ-14 | touches 4 | — | classifier-derives-details |
| server-status-from-classification | Derive server HTTP statuses from the library classification | task | Server routes map failures through classify and one exhaustive ErrorCode-to-StatusCode match | failure-classification | A3, D-10, F-SERVER-STATUS, REQ-14 | touches 1 | — | path-read-error-codes |
| executor-kernel-closure | Crate-private executor kernel | task | `CommandExecutor`'s public surface is its constructors and family accessors; the kernel and `config_manager` are crate-private. | executor-family-facades | D-1, F-EXECUTOR-SURFACE, REQ-03 | creates 1, touches 6 | — | handlers-issue-writes, handlers-dep, handlers-graph, handlers-gate-evaluation, handlers-doc, handlers-query-status, handlers-validate, handlers-config, handlers-hooks, handlers-claim-recover, handlers-serve, handlers-profile, handlers-init-apply, handlers-label, handlers-events, handlers-snapshot-migrate, handlers-archive, handlers-worktree, handlers-item-invariant-project, handlers-search, handlers-version, server-status-from-classification |
| retire-unproduced-error-codes | Retire error codes that nothing produces | task | ErrorCode, the generated error-code page and the failure-lever registry list only codes some error type classifies to | type-only-error-codes | A3, F-FALLBACK-SITES, REQ-09, REQ-13 | touches 5 | — | handlers-issue-writes, handlers-dep, handlers-graph, handlers-gate-evaluation, handlers-doc, handlers-query-status, handlers-validate, handlers-config, handlers-hooks, handlers-claim-recover, handlers-serve, handlers-profile, handlers-init-apply, handlers-label, handlers-events, handlers-snapshot-migrate, handlers-archive, handlers-worktree, handlers-item-invariant-project, handlers-search, handlers-version |
| schema-omits-hidden | Omit clap-hidden commands and flags from the generated schema | task | CommandSchema::generate() skips the commands and flags clap marks hidden | — | F-HIDDEN, REQ-05, REQ-20, S13 | touches 1 | — | verb-hint-stub-removal |
| schema-registry-derivation | Derive output schemas from the handler registry | task | get_output_schema_for_command looks up the handler registry, the hand table and CommandResponse leaf fallback are gone | handler-contract | A25, D-2, F-SCHEMA-TABLE, REQ-05, REQ-17 | touches 2 | — | handlers-issue-writes, handlers-dep, handlers-graph, handlers-gate-evaluation, handlers-doc, handlers-query-status, handlers-validate, handlers-config, handlers-hooks, handlers-claim-recover, handlers-serve, handlers-profile, handlers-init-apply, handlers-label, handlers-events, handlers-snapshot-migrate, handlers-archive, handlers-worktree, handlers-item-invariant-project, handlers-search, handlers-version, schema-omits-hidden |
| cli-contract-docs | State the CLI output contract once in the command reference | task | cli-commands.md states the list envelope, global --json, stream rule, findings envelope and wrong-verb hints once | list-envelope, json-mode-streams, findings-envelope, parser-diagnostics-outside-envelope, output-mode | D-5, D-6, D-7, D-8, F-CONTRACT-DOCS, REQ-09 | touches 3 | — | main-entry-consolidation, retire-unproduced-error-codes, schema-registry-derivation |
| completion-report | Write the df129d9e completion report | task | completion-report.md records per-module production lines before and after plus the re-measured build-footprint budget line | production-line-count | D-13, F-BUDGET-SCRIPT, F-PRODLINES, REQ-07, REQ-10 | creates 1 | — | main-rs-line-ceiling, shared-test-harness, workspace-lints, executor-kernel-closure, cli-contract-docs |
| build-hygiene | Build hygiene and measured completion for crates/jit | story | Lints live in the manifests, the harness compiles once, main.rs has a ceiling, and line counts are reported before and after | — | REQ-06, REQ-07, REQ-08, REQ-10 | — | — | completion-report |
| error-boundary | One library error boundary for the CLI and the server | story | One library classifier owns every failure's code, exit code, details and HTTP status, and main.rs holds the only exit | — | REQ-02, REQ-09, REQ-11, REQ-12, REQ-13, REQ-14, REQ-19 | — | — | server-status-from-classification, retire-unproduced-error-codes, main-entry-consolidation |
| output-contract | One stated output and schema contract for the CLI | story | List payloads share one envelope type, shared human views have one renderer, and the schema derives every leaf's type | — | REQ-04, REQ-05, REQ-09, REQ-17, REQ-18, REQ-20, REQ-21 | — | — | cli-contract-docs |
| command-dispatch | Library-side command dispatch for crates/jit | story | Every command group dispatches from a thin run() to a typed library handler over one global --json and shared filter args | — | REQ-01, REQ-02, REQ-05, REQ-09, REQ-12, REQ-13, REQ-15, REQ-16, REQ-17, REQ-18, REQ-19, REQ-20 | — | — | handlers-issue-writes, handlers-dep, handlers-graph, handlers-gate-evaluation, handlers-doc, handlers-query-status, handlers-validate, handlers-config, handlers-hooks, handlers-claim-recover, handlers-serve, handlers-profile, handlers-init-apply, handlers-label, handlers-events, handlers-snapshot-migrate, handlers-archive, handlers-worktree, handlers-item-invariant-project, handlers-search, handlers-version |
| executor-surface | Narrow executor surface and storage-free CLI entry | story | CommandExecutor is reached through per-family facades, and the CLI path makes no direct storage or library-bypass call. | — | REQ-03, REQ-19 | — | — | repository-bootstrap-constructor, executor-kernel-closure |

```mermaid
flowchart LR
    N0["workspace-lints: Encode the zero-warning bar in workspace lints"]
    N1["shared-test-harness: Compile the integration harness once in jit's test-support surface"]
    N2["production-line-baseline: Record the production-line baseline for crates/jit/src"]
    N3["mcp-global-json-injection: MCP bridge injects --json from the schema's global options"]
    N4["verb-hint-stub-removal: Wrong-verb hints come only from the parser-level path"]
    N5["global-json-flag: One global --json flag carried as OutputMode"]
    N6["failure-classifier-library: Move the error classifier into output::failure"]
    N7["classifier-derives-details: Derive failure details and suggestions from the error type"]
    N8["list-envelope-type: Introduce the generic ListEnvelope type"]
    N9["list-envelope-adoption: Express output-module count structs as ListEnvelope instantiations"]
    N10["handler-contract: Library-side command handler contract"]
    N11["issue-filter-args: Shared IssueFilterArgs group for issue filters"]
    N12["issue-list-renderer: Render issue lists through one output renderer"]
    N13["executor-facade-issues: Issue methods behind an issues facade"]
    N14["executor-facade-gates: Gate methods behind a gates facade"]
    N15["executor-facade-admin: Archive, snapshot, config, item, invariant and project methods behind an admin facade"]
    N16["executor-facade-queries: Query, search, event, graph and dependency methods behind a queries facade"]
    N17["handlers-issue-reads: Issue read commands move to library handlers"]
    N18["create-returns-issue: Issue creation returns the created issue"]
    N19["mutations-return-issue: Issue mutations return the updated issue"]
    N20["executor-facade-validation: Validation methods behind a validation facade"]
    N21["handlers-issue-writes: Issue write commands move to library handlers"]
    N22["handlers-dep: Dependency commands move to library handlers"]
    N23["handlers-graph: Graph commands move to library handlers"]
    N24["handlers-gate-definitions: Gate definition and preset commands move to library handlers"]
    N25["gate-run-status-renderer: Render gate run status lines through one output renderer"]
    N26["handlers-gate-status: Gate status commands move to library handlers"]
    N27["handlers-gate-evaluation: Gate evaluation commands move to library handlers"]
    N28["executor-facade-documents: Document methods behind a documents facade"]
    N29["handlers-doc: Document commands move to library handlers"]
    N30["handlers-query-status: Query and status commands move to library handlers"]
    N31["validation-report-renderer: One output renderer for validation findings"]
    N32["handlers-validate: Validate command moves to a library handler"]
    N33["handlers-config: Config commands move to library handlers"]
    N34["handlers-hooks: Hooks command moves to a library handler"]
    N35["claims-executor-family: Claim and recover operations behind a claims facade"]
    N36["handlers-claim-recover: Claim and recover commands move to library handlers"]
    N37["serve-foreground-command: Foreground serve as a commands-layer operation"]
    N38["handlers-serve: Serve command moves to a library handler"]
    N39["executor-facade-profiles: Profile, init, template and breakdown methods behind a profiles facade"]
    N40["handlers-profile: Profile commands move to library handlers"]
    N41["apply-returns-created-issues: Template application returns the created issues"]
    N42["handlers-init-apply: Init and apply commands move to library handlers"]
    N43["handlers-label: Label commands move to library handlers"]
    N44["handlers-events: Events commands move to library handlers"]
    N45["handlers-snapshot-migrate: Snapshot and migrate commands move to library handlers"]
    N46["archive-execution-renderer: One output renderer for archive execution summaries"]
    N47["handlers-archive: Archive commands move to library handlers"]
    N48["handlers-worktree: Worktree commands move to library handlers"]
    N49["handlers-item-invariant-project: Item, invariant and project commands move to library handlers"]
    N50["search-through-executor: CLI search runs through the executor"]
    N51["handlers-search: Top-level search command moves to a library handler"]
    N52["handlers-version: Version command moves to a library handler"]
    N53["repository-bootstrap-constructor: Library repository bootstrap for the CLI"]
    N54["json-silent-stderr: Keep stderr empty for --json failures"]
    N55["main-entry-consolidation: Reduce main.rs to parse, bootstrap, dispatch, render and exit"]
    N56["main-rs-line-ceiling: Enforce a main.rs production-line ceiling in the budget script"]
    N57["path-read-error-codes: Classify document path-read failures by variant"]
    N58["server-status-from-classification: Derive server HTTP statuses from the library classification"]
    N59["executor-kernel-closure: Crate-private executor kernel"]
    N60["retire-unproduced-error-codes: Retire error codes that nothing produces"]
    N61["schema-omits-hidden: Omit clap-hidden commands and flags from the generated schema"]
    N62["schema-registry-derivation: Derive output schemas from the handler registry"]
    N63["cli-contract-docs: State the CLI output contract once in the command reference"]
    N64["completion-report: Write the df129d9e completion report"]
    N65["build-hygiene: Build hygiene and measured completion for crates/jit"]
    N66["error-boundary: One library error boundary for the CLI and the server"]
    N67["output-contract: One stated output and schema contract for the CLI"]
    N68["command-dispatch: Library-side command dispatch for crates/jit"]
    N69["executor-surface: Narrow executor surface and storage-free CLI entry"]
    N3 --> N5
    N4 --> N5
    N6 --> N7
    N8 --> N9
    N5 --> N10
    N7 --> N10
    N9 --> N10
    N5 --> N11
    N5 --> N12
    N5 --> N13
    N13 --> N14
    N13 --> N15
    N13 --> N16
    N10 --> N17
    N11 --> N17
    N12 --> N17
    N14 --> N17
    N15 --> N17
    N16 --> N17
    N13 --> N18
    N18 --> N19
    N13 --> N20
    N17 --> N21
    N19 --> N21
    N20 --> N21
    N10 --> N22
    N16 --> N22
    N10 --> N23
    N12 --> N23
    N16 --> N23
    N10 --> N24
    N14 --> N24
    N5 --> N25
    N24 --> N26
    N25 --> N26
    N26 --> N27
    N13 --> N28
    N10 --> N29
    N28 --> N29
    N17 --> N30
    N20 --> N30
    N5 --> N31
    N10 --> N32
    N31 --> N32
    N20 --> N32
    N10 --> N33
    N31 --> N33
    N15 --> N33
    N10 --> N34
    N13 --> N35
    N10 --> N36
    N35 --> N36
    N5 --> N37
    N10 --> N38
    N37 --> N38
    N13 --> N39
    N10 --> N40
    N39 --> N40
    N39 --> N41
    N10 --> N42
    N41 --> N42
    N10 --> N43
    N13 --> N43
    N10 --> N44
    N16 --> N44
    N10 --> N45
    N15 --> N45
    N20 --> N45
    N5 --> N46
    N10 --> N47
    N46 --> N47
    N15 --> N47
    N10 --> N48
    N10 --> N49
    N15 --> N49
    N16 --> N50
    N10 --> N51
    N50 --> N51
    N10 --> N52
    N5 --> N53
    N5 --> N54
    N6 --> N54
    N21 --> N55
    N22 --> N55
    N23 --> N55
    N27 --> N55
    N29 --> N55
    N30 --> N55
    N32 --> N55
    N33 --> N55
    N34 --> N55
    N36 --> N55
    N38 --> N55
    N40 --> N55
    N42 --> N55
    N43 --> N55
    N44 --> N55
    N45 --> N55
    N47 --> N55
    N48 --> N55
    N49 --> N55
    N51 --> N55
    N52 --> N55
    N53 --> N55
    N54 --> N55
    N55 --> N56
    N2 --> N56
    N7 --> N57
    N57 --> N58
    N21 --> N59
    N22 --> N59
    N23 --> N59
    N27 --> N59
    N29 --> N59
    N30 --> N59
    N32 --> N59
    N33 --> N59
    N34 --> N59
    N36 --> N59
    N38 --> N59
    N40 --> N59
    N42 --> N59
    N43 --> N59
    N44 --> N59
    N45 --> N59
    N47 --> N59
    N48 --> N59
    N49 --> N59
    N51 --> N59
    N52 --> N59
    N58 --> N59
    N21 --> N60
    N22 --> N60
    N23 --> N60
    N27 --> N60
    N29 --> N60
    N30 --> N60
    N32 --> N60
    N33 --> N60
    N34 --> N60
    N36 --> N60
    N38 --> N60
    N40 --> N60
    N42 --> N60
    N43 --> N60
    N44 --> N60
    N45 --> N60
    N47 --> N60
    N48 --> N60
    N49 --> N60
    N51 --> N60
    N52 --> N60
    N4 --> N61
    N21 --> N62
    N22 --> N62
    N23 --> N62
    N27 --> N62
    N29 --> N62
    N30 --> N62
    N32 --> N62
    N33 --> N62
    N34 --> N62
    N36 --> N62
    N38 --> N62
    N40 --> N62
    N42 --> N62
    N43 --> N62
    N44 --> N62
    N45 --> N62
    N47 --> N62
    N48 --> N62
    N49 --> N62
    N51 --> N62
    N52 --> N62
    N61 --> N62
    N55 --> N63
    N60 --> N63
    N62 --> N63
    N56 --> N64
    N1 --> N64
    N0 --> N64
    N59 --> N64
    N63 --> N64
    N64 --> N65
    N58 --> N66
    N60 --> N66
    N55 --> N66
    N63 --> N67
    N21 --> N68
    N22 --> N68
    N23 --> N68
    N27 --> N68
    N29 --> N68
    N30 --> N68
    N32 --> N68
    N33 --> N68
    N34 --> N68
    N36 --> N68
    N38 --> N68
    N40 --> N68
    N42 --> N68
    N43 --> N68
    N44 --> N68
    N45 --> N68
    N47 --> N68
    N48 --> N68
    N49 --> N68
    N51 --> N68
    N52 --> N68
    N53 --> N69
    N59 --> N69
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

All decisions were taken autonomously under the session's no-question rule. Each records the rejected option.

| Risk / decision | Resolution and rationale |
|---|---|
| D-1 REQ-03 shape | Chosen: per-family facade structs. Rejected: extension traits, which spread storage bounds into trait where-clauses, need separate traits for the `JsonFileStorage`-only blocks, and add a parallel abstraction. The measured coupling (families call the kernel and rarely each other) fits facades: area 3 §1. |
| D-2 Handler location | Chosen: handlers in a library `dispatch/` module, because schema generation lives in the library and must reach every handler's response type. Rejected: binary-only modules, which the schema cannot see. |
| D-3 Response ownership | Chosen: output owns response types and wraps command results through constructors. Rejected: moving `ItemListResult` and similar types into output, which would make `commands/` depend on output. |
| D-4 Classifier home | Chosen: `output/failure.rs`, with `output.rs` kept as the module root in the crate's non-`mod.rs` layout. That keeps the epic's literal `output.rs` criteria true and avoids a rename racing the other `output.rs` editors. Rejected: `errors.rs`, which `domain/` imports, so a classifier there would make a pure layer depend outward. |
| D-5 Findings commands | Chosen: `findings-envelope` for every findings command. Rejected: keeping a success envelope with a non-zero exit, where the exit code equals no envelope code. Deliberate change for `gate status-all`, `gate evaluate-many` and `doc check-links`. |
| D-6 stderr under `--json` | Chosen: no stderr diagnostic. This rewrites the assertion at `top_level_failure_envelope_tests.rs:58` and the statement at `cli-commands.md:506`. Rejected: keeping the `Error:` line, which renders one failure twice and contradicts the handler-owned paths' empty stderr. |
| D-7 Wrong-verb hints | Chosen: the parser-level hint only, on stderr with exit 2, which is a deliberate change from the JSON hint envelope that `verb_hint_tests.rs` asserts. Rejected: an argv scan to recover `--json` after a failed parse, which REQ-15 forbids. clap cannot yield partial matches: area 2 Primitive verification. |
| D-8 Filter parsing | Chosen: `IssueFilterArgs` holds the raw values and exposes one conversion at the CLI boundary over the existing `FromStr` types. A bad value keeps its `INVALID_ARGUMENT` envelope. Label filters are patterns everywhere: `issue search` gains pattern matching and reports `INVALID_LABEL_PATTERN`. Repeated `--label` stays the only multi-value form. Rejected: clap `value_parser`s, which semantic-types epic 2195c98f owns and which would turn a bad value into a clap diagnostic. |
| D-9 Count structs | Chosen: every struct whose `count` is the length of a sibling collection becomes a `list-envelope` instantiation. That includes `GateCheckAllResponse` (gates, with `results` as an extra) and `StateRollup` (by_state). Inline count bodies are replaced by their group's handler. Command-layer list results (`DocumentListResult`, `AssetListResult`, `ItemListResult`, `InvariantCheckResult`, `ProjectRenderResult`) lose their `count` field and are wrapped by their group's handler. `ProfileAgreement` and the snapshot manifest's `IssuesInfo` and `DocumentsInfo` derive their counts through the envelope. |
| D-10 Server status | Chosen: `crates/server` keeps one exhaustive `ErrorCode` → `StatusCode` match applied to `classify`'s code; `http` types stay out of `jit`. Rejected: putting HTTP statuses in the library. The current disagreements between the CLI and the server resolve to the classification. Before the server switches, a separate deliberate change classifies `PathReadError` by variant: `OutsideRepoRoot` joins `InvalidPath` as `INVALID_ARGUMENT`, and `NotFound` and `CommitNotFound` become a new document-not-found code. Without this, the server's 400 and 404 statuses would regress to 500 and to repository-not-found. |
| D-11 REQ-19 scope | Chosen: storage construction, session bootstrap, the claims free functions, search and foreground serve all count as calls `main.rs` must not make. This epic produces `repository-bootstrap` and the executor search entry point. 0f8efc37 then adopts the bootstrap for the server (its REQ-01) and replaces what the search entry point computes (its REQ-08). Rejected: waiting on 0f8efc37, which is unplanned. |
| D-12 `hidden_commands()` | Chosen: keep it, since REQ-05 pins it. It encodes MCP tool-listing policy that duplicates `mcp-server/curated-tools.json`, a D-18 candidate reported to the steward. |
| D-13 Line counting | Chosen: the Background's described method is authoritative, as REQ-10 requires, and the flagged figures are reported beside it. Rejected: making the flagged mode authoritative because it reproduces 63,100, which would contradict REQ-10's wording. The "before" figure is taken before this epic first changes `crates/jit/src`, because concurrent epics move it. |
| D-14 Lint encoding | Chosen: `[workspace.lints.rust] warnings = "deny"`, `unsafe_code = "deny"`, with `lints.workspace = true` in both members, replacing the four `#![deny(unsafe_code)]` roots. Plain builds then fail on warnings, which is the bar. GitHub CI's `--all-features` clippy differs from the gate; aligning them is reported, not in scope. |
| D-15 Harness home | Chosen: `jit`'s existing `test-support` surface, compiled once in the library build every target links. Rejected: a new `jit-test-support` crate, which adds a unit without adding a capability. |
| D-16 External sequencing | Implementation-consumed edges re-homed to manifest keys. `global-json-flag`, the one sweep across every `main.rs` arm and the ancestor of every arm-moving task, waits for every in-flight bug that edits those arms: 2d10f1dd, 34999aad, 3754095f, 16d63038, 8673dc5e and f37d228e. The tasks that consume a specific fix also wait on it directly: `classifier-derives-details` and `search-through-executor` on 34999aad; `mutations-return-issue` on 2d10f1dd and 16d63038; `gate-run-status-renderer` on 8673dc5e; `validation-report-renderer` on 3754095f; `handlers-doc` on f37d228e. Edges already implied through `global-json-flag` are dropped as redundant when wired. The planning node keeps only its archived predecessor a2546471. |
| D-17 Dead output API | Chosen: `list-envelope-type` deletes the eight dead per-query structs (REQ-04). The other dead macros, constructors and `OutputContext` methods belong to 842b0ac6 REQ-15. Its plan records the shared files and adds no edge; whichever lands second resolves `output_macros.rs`. |
| D-18 Renderer differences | Chosen: each shared renderer adopts the form most of its current sites print, and each command whose human output changes gets a test stating the new output. The copies differ in indent, footer, stream and exit: area 2 §8. |
| D-19 Concurrent `main.rs` edits | Risk: 842b0ac6 removes `main.rs` regions (`migrate`, inert config display) while this epic moves them. Mitigation: each handler task migrates whatever its group holds when it starts; the two plans share files without edges. |
| D-20 Ceiling calibration | Chosen: after `main-entry-consolidation`, set the ceiling to that file's measured code lines plus at most 10% headroom. The derivation goes in `dev/benchmarks/rust-build-budgets/README.md`. |
| D-21 Overlap with 2195c98f | Risk: semantic types retypes `IssueStore`, executor signatures and CLI id parsing, and adds closed-vocabulary types, while this epic moves the same arms and methods. Chosen boundary: 2195c98f owns every new type and value parser; this epic owns where code lives. Recommended order, escalated to the steward: 2195c98f's storage and domain stages proceed independently, and its CLI-dispatch and command-id stages land after this epic's handler migrations, so ids are typed once in `dispatch/`. Its error-code vocabulary stage lands before `retire-unproduced-error-codes`. |
| D-22 Overlap with 842b0ac6 claims | Risk: 842b0ac6's removal of the unused claim storage type parameter rewrites the `commands/claim.rs` functions that `claims-executor-family` turns into facade methods. Chosen: that 842b0ac6 task lands first, and the steward adds the edge to `claims-executor-family` once 842b0ac6's breakdown creates it. |
| Wide `main.rs` fan-out | Risk: about forty tasks edit `main.rs`. Mitigation: `global-json-flag` lands first as the only sweep across every arm. A facade task rewrites every arm that calls its family, so a handler task waits for every facade its arms call, not only its own family's. Handler migrations then delete arms rather than editing them. |

## Investigation sources

- [Investigation](investigation.md) owns the inventories. Finding ids used as manifest sources:
  - F-CLASSIFIER, F-CALLSITE-DETAILS, F-FALLBACK-SITES, F-EXIT-SITES, F-DROP-SKIPPED, F-STDERR-DUP and F-SERVER-STATUS: area 1, from the classifier through the server mappings.
  - F-JSON-FIELDS, F-MCP-JSON, F-FILTERS, F-HIDDEN, F-COUNT-STRUCTS, F-SCHEMA-TABLE, F-RENDER-DUP, F-HANDLER-SHAPE, F-GROUP-ARMS and F-CONTRACT-DOCS: area 2 §1 to §8 and its Consumer sweep.
  - F-EXECUTOR-SURFACE, F-BYPASS, F-LINTS, F-HARNESS, F-BUDGET-SCRIPT and F-PRODLINES: area 3.
- Audit rows A3, A24, A25, A26, S13 and S14 are the findings this epic folds in.
- [prodlines.py](prodlines.py) is the investigation's production-line counter, which becomes `scripts/production-lines.py`.
