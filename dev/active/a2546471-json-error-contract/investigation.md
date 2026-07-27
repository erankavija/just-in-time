# Investigation — a2546471 structured failure reporting across the machine-readable CLI surface

Read-only fact-finding for the container's plan. No code, `.jit/` state, or issues were changed.

## Finding index

Each id below names the heading that owns its evidence, so a plan or manifest
reference resolves to one section of this report.

| Id | Section |
|---|---|
| `F-MECHANISMS` | 1. Every mechanism that emits the envelope on failure |
| `F-ARM-COUNT` | 2. Recounted numbers |
| `F-EXIT-DIVERGENCE` | 5. `error_to_exit_code` vs `JsonError::exit_code()` |
| `F-SEARCH-EXIT10` | 5 → `search` exits on a literal its reported code does not map to |
| `F-PRESET-APPLY-ZERO` | 5 → `gate preset apply` returns zero with its failures nested in a success payload |
| `F-EXIT-DOC-DEFECT` | 5 → the generated exit-status reference is bound through the plain invocation alone |
| `F-STDOUT-CLEAN` | 6. The stderr/stdout split |
| `F-MCP` | 7. MCP bridge |
| `F-SCHEMA-DERIVED` | The census question → the `--schema` machinery |
| `F-NO-STATIC` | The census question → dispatch structure in `main.rs` |
| `F-FORCED-FAILURE` | The census question → is a forced failure reachable for every arm? |
| `F-CLAP-PARSE` | Consumer sweep → argument-parse failures |

## Method and provenance

- Tree: `main` at `8a9963a6`, clean working tree.
- The `jit` on `PATH` (`/home/vkaskivuo/.cargo/bin/jit`) reports commit `758a44d3`, which is **behind** HEAD, but `git diff --stat 758a44d3..HEAD -- crates/` is empty — no Rust source changed between them. All measurements nonetheless used a binary built from this tree: `cargo build -p jit --bin jit` → `target/debug/jit`. The one exception is the MCP probe (§7), which necessarily resolves `jit` from `PATH`; that binary is source-identical for `crates/`.
- Runtime probes ran in a throwaway repo at `<scratchpad>/repo` (`git init` + `jit init` + one seed issue). Nothing ran against this repository's `.jit/`.
- Census derivation (reproducible): `jit --schema` → recursive walk of `commands` → a **leaf** is a command with no `subcommands`; an arm "accepts the flag" iff its `flags` array contains a member named `json`. Static mechanism attribution: `crates/jit/src/main.rs` dispatch arms delimited by variant-pattern start lines, each body scanned for `handle_json_error!`, `JsonError::`, `std::process::exit(`, `?;`.

---

## 1. F-MECHANISMS — Every mechanism that emits the envelope on failure: claim invalid as stated

`handle_json_error!` is **not** the only route. It is one of nine distinct mechanisms.

| # | Mechanism | Definition | Emission site | Call sites |
|---|-----------|-----------|---------------|-----------|
| 1 | `handle_json_error!` macro | `crates/jit/src/output_macros.rs:70` | `println!` at `output_macros.rs:77`, `exit` at `:78` | 13 (main.rs:1303, 1377, 1458, 2566, 2652, 2681, 2730, 2763, 2959, 3419, 3473, 5142, 6074) |
| 2 | `profile_result` | `main.rs:836` | `println!` `:841`, `exit` `:842` | 2 (`main.rs:2041`, `:2052`) |
| 3 | `emit_startup_json_error` | `main.rs:1766` | `println!` `:1789` | 1, from `main()` at `main.rs:1729`; reads `--json` from `std::env::args()` at `:1767` |
| 4 | `render_gate_pass_error` | `main.rs:537` | `println!` `:627`, `exit` `:628` | 2 (`main.rs:4394`, `:4460`) |
| 5 | `resolve_gate_key_for` | `main.rs:755` | `println!` `:769`, `exit` `:770` | 4 (`main.rs:3925`, `:4032`, `:4092`, `:4350`) |
| 6 | `invalid_argument(message, json)` | `main.rs:784` | `println!` `:789`, `exit` `:791` | 17 call sites (main.rs:900, 937, 956, 1677, 2026, 2801, 2807, 2815, 2821, 2837, 3519, 3642, 3680, 3703, 3906, 3915, 6456); `:900` is inside `verb_hint_error` and `:937`/`:956` inside `resolve_optional_gate_key` |
| 7 | Inline ad-hoc `println!(json_error) + std::process::exit` | — | 35 dispatch-body sites | main.rs:2172, 2203, 2233, 2259, 3061, 3165, 3278, 3411, 3591, 3733, 3773, 3837, 3864, 3972, 3992, 4085, 4121, 4162, 4334, 4499, 4549, 4614, 4718, 6298, 6412, 6743, 7349, 7394, 7436, 7469, 7557, 7630, 7669, 7722, 7775 |
| 8 | `Commands::Recover` stderr variant | — | `eprintln!` `main.rs:6990`, `exit(1)` `:6991` | 1. **The only envelope written to stderr**, the only one serialized compact (`serde_json::to_string`, not `to_json_string()`'s pretty form), and the only one carrying a lowercase code (`"recovery_failed"`, `main.rs:6989`) outside the `ErrorCode` vocabulary |
| 9 | `gate preset apply` success-payload errors | — | `main.rs:4667-4672` | 1. Per-issue failures are serialized into an `errors` array inside a **success** payload; the arm exits `0` |

Envelope-shaped helpers that build but do not print (`JsonError` factories): `dep_add_batch_json_error` (`main.rs:361`), `claim_json_error` (`:492`), `validate_fix_json_error` (`:507`), `stale_binary_json_error` (`:640`), `profile_json_error` (`:796`), and the `JsonError::issue_not_found` / `gate_not_found` / `cycle_detected` / `invalid_state` / `invalid_priority` / `gate_validation_failed` / `transition_blocked` constructors (`crates/jit/src/output.rs:736-822`).

The team lead's cited `main.rs:765` and `:790` are inside mechanisms 5 and 6 respectively; `:628` is mechanism 4. Confirmed.

## 2. F-ARM-COUNT — Recounted numbers: both figures in the container body are wrong

- **`handle_json_error!` call sites: 13, not 18.** `grep -rn "handle_json_error" crates/ --include="*.rs"` returns 21 lines: 1 macro definition (`output_macros.rs:70`), 13 real invocations, 5 doc/code comments in `main.rs` (`:2564`, `:2669`, `:3416`, `:3469`, `:5119`), 1 doc comment in `crates/jit/src/commands/issue.rs:334`, and 1 in `crates/jit/tests/cli_issue/issue_status_projection_tests.rs:333`.
- **Arms accepting `--json`: 99, not ~63.** Definition counted: a *leaf* command in `jit --schema` (no `subcommands`) whose `flags` include `json`. The schema has 112 leaf commands; 99 accept `--json`, 13 do not. The 13 without: `dep delete`, `dep remove`, `doc delete`, `doc rm`, `gate delete`, `gate rm`, `issue complete`, `issue edit`, `issue remove`, `issue rm`, `label add`, `label remove`, `label rm` — all hidden verb-hint stubs (`cli.rs`, `#[command(hide = true)]`), which nevertheless render the envelope by sniffing a literal `--json` out of their captured argv (`verb_hint_error`, `main.rs:894`).
- One non-leaf group also carries a `json` flag: `query` (`main.rs:5530`); it is a dispatch shell, not an arm.
- **Arms with no envelope route on any failure path: 51 of 99** (§ inventory). Two of those are arguably not defects: `version` is handled pre-dispatch and infallible (`main.rs:1895-1913`), and `serve`'s start/stop/status failures go through `eprintln!` + `exit(1)` by design (`main.rs:7075`, `:7123`, `:7299`).

## 3. Spot-check by invocation — claim **confirmed, and the split is wider than stated**

All in the scratch repo, `--json`, stdout/stderr separated:

| Arm | stdout | stderr | exit |
|---|---|---|---|
| `issue show deadbeef` | envelope, `ISSUE_NOT_FOUND` | *(empty)* | 3 |
| `gate status deadbeef` | envelope, `INVALID_ARGUMENT` | *(empty)* | 2 |
| `item show @/nope/nope` | envelope, `ITEM_COMMAND_FAILED` | *(empty)* | 1 |
| `dep add deadbeef deadbeef` | envelope, `DEPENDENCY_ERROR` | *(empty)* | 1 |
| `doc list deadbeef` | *(empty)* | `Error: Issue not found: deadbeef` | 3 |
| `doc show deadbeef README.md` | *(empty)* | `Error: Issue not found: deadbeef` | 3 |
| `graph deps deadbeef` | *(empty)* | `Error: Issue not found: deadbeef` | 3 |

The plain-text form always lands on **stderr** and stdout is left **empty** — not "a plain-text line on stdout". This matters for REQ-03 (§6).

## 4. The `doc` arms — claim **confirmed but incomplete**

Every listed arm propagates with a bare `?`; `doc dir` is repaired. One arm was missed.

| Arm | Dispatch | Propagation site |
|---|---|---|
| `doc add` | `main.rs:4970` | `?` at `main.rs:4987` |
| `doc list` | `:5018` | `?` at `:5022` |
| `doc remove` | `:5056` | `?` at `:5057` |
| `doc show` | `:5076` | `?` at `:5077` |
| `doc history` | `:5204` | `?` at `:5208` |
| `doc diff` | `:5226` | `?` at `:5233` |
| `doc assets list` | `:5245` | `?` at `:5255` |
| `doc check-links` | `:5383` | `?` at `:5389`, `:5390` |
| **`doc conformance`** | `:5146` | `?` at `:5155` — **not named in the container body**; it is a ninth `doc` arm with a bare `?`, and the code comment at `main.rs:5148-5154` argues the omission is deliberate |
| `doc dir` | `:5097` | repaired: `handle_json_error!` at `:5142`, downcast classification at `:5126-5138` |

## 5. F-EXIT-DIVERGENCE — `error_to_exit_code` vs `JsonError::exit_code()`: "Both forms carry the same exit code" is invalid as stated

They disagree on 9 measured arms. Measured by running the identical forced failure twice, once without `--json` and once with:

| Arm | forced failure | plain exit | `--json` exit |
|---|---|---|---|
| `dep add deadbeef deadbeef` | unresolvable id | 3 | **1** |
| `dep rm deadbeef deadbeef` | unresolvable id | 3 | **1** |
| `claim acquire deadbeef` | unresolvable id | 3 | **1** |
| `claim release deadbeef` | unresolvable id | 3 | **1** |
| `gate fail deadbeef no-such-gate` | unresolvable id | 3 | **1** |
| `gate preset create deadbeef p2` | unresolvable id | 3 | **1** |
| `gate preset show no-such-preset` | missing preset | 3 | **1** |
| `gate define <existing-key> …` | duplicate key | 6 | **1** |
| `gate preset apply no-such-preset <id>` | missing preset | 1 | **0** |

Root cause: the plain path classifies by typed downcast in `error_to_exit_code` (`main.rs:49`), while these arms hand a **command-scoped fallback code** to `JsonError::new` without downcasting first — `"DEPENDENCY_ERROR"` (`main.rs:3422`, `:3476`), `"GATE_ERROR"` (`:3590`, `:4498`), `"PRESET_ERROR"` (`:4613`, `:4717`), `claim_json_error`'s `fallback_code` (`:500`). None of those strings is in `ErrorCode`, so `ErrorCode::to_exit_code` (`output.rs:681`) falls through its `_ =>` arm to `ExitCode::GenericError` (`output.rs:702`). `refine_id_error` (`output.rs:717`) only rescues `InvalidIdPrefixError`/`AmbiguousIdError` — it does not rescue `IssueNotFoundError`, which is why an unresolvable-but-well-formed id lands on 1.

Two further disagreements, structural rather than measured:

### F-SEARCH-EXIT10 — `search` exits on a literal its reported code does not map to

`main.rs:6413` is a hardcoded `std::process::exit(10)` after printing an envelope whose code is `SEARCH_FAILED` or `RIPGREP_NOT_FOUND` (`:6389`, `:6392`). Neither string is in `ErrorCode`, so `json_error.exit_code()` would be `1`. The envelope's own reported code and the process exit status disagree by construction — the exact condition REQ-04 forbids.

### F-PRESET-APPLY-ZERO — `gate preset apply` returns zero with its failures nested in a success payload

The human path exits `1` on any per-issue error (`main.rs:4692`); the `--json` path has no such exit and returns `0` with the errors nested in the success payload.

### F-EXIT-DOC-DEFECT — The generated exit-status reference is bound through the plain invocation alone

**This contradicts the generated exit-code reference.** `docs/reference/exit-codes.md` states `gate define | 6 | The gate key is already registered.` and `dep add | 4 | …`, and the binding test `test_command_exit_codes_gate_define_duplicate_emits_6` (`crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs:575`) exercises the **non-`--json`** invocation only. The projection is therefore accurate for plain invocations and silently wrong for `--json` ones — a live `@/inv/single-source-prose` defect the plan should name.

## 6. F-STDOUT-CLEAN — REQ-03: the stderr/stdout split already holds, and what remains is narrower than the requirement implies

- `main()` writes `eprintln!("Error: {}", e)` at `main.rs:1728` — **stderr**. Every envelope except `recover`'s is `println!` — **stdout**.
- Across all 61 arms I could drive to a real in-arm failure under `--json`, stdout was either completely empty or exactly one well-formed JSON document. **No arm wrote a non-JSON line to stdout on failure.** (Probe classifier: `FAIL-NONJSON-STDOUT` count = 0.)
- `emit_startup_json_error` does not double-print: the human line goes to stderr from `main()`, the envelope to stdout, and it returns early (`main.rs:1785`) for anything that is not one of its three startup classes. Verified: `jit status --json` in a directory with no `.jit` prints the `REPOSITORY_NOT_FOUND` envelope on stdout and the multi-line human message on stderr, exit 3.
- Human/progress output is systematically guarded. `OutputContext::print_data`/`print_info`/`print_success` are no-ops under `json` (`output.rs:331`, `:340`, `:349`); `print_warning` writes to stderr and is suppressed under `json` (`output.rs:357-363`). `serve`'s many `println!` sites are inside `if !json` branches — verified empirically (`serve --status --json` emits only `{"status":"not_running"}`).

**What REQ-03 still requires, concretely:**
1. Fix `recover` (`main.rs:6989-6991`): its envelope goes to stderr, the same stream that carries plain diagnostics everywhere else, and it is compact-serialized rather than pretty like every other envelope.
2. Pin the property. Today it holds by construction and convention, not by any test. Nothing fails if a future arm adds an unguarded `println!` before a fallible call.
3. Decide the requirement's reading: as written ("no plain-text diagnostic on the stream that carries the envelope") it is already satisfied for 98 of 99 arms. If the intent is "the envelope must exist at all", that is REQ-02, not REQ-03.

## 7. F-MCP — MCP bridge: claim confirmed, with a concrete defect

- Generation: `mcp-server/lib/schema-loader.js:22` runs `execFile('jit', ['--schema'])` at server start and parses stdout. No schema file is bundled (`schema-loader.js:5`). `mcp-server/lib/tool-generator.js` turns each schema command into a `jit_<path>` tool; `mcp-server/curated-tools.json` decides which are advertised by default.
- `--json` is appended automatically for any arm whose schema `flags` contain `json` (`mcp-server/lib/cli-executor.js:118`, `:139-141`).
- Failure handling: a non-zero exit makes `execFile` throw. The catch block at `cli-executor.js:72-82` tries `JSON.parse(error.stdout)`; if that yields an object with `.error`, it returns `{success:false, error:{code, message}}`. Otherwise it falls to `cli-executor.js:85` and returns `EXECUTION_ERROR` with Node's raw `error.message`.

Measured end-to-end through the real `executeCommand`:

```
issue show  -> {"success":false,"error":{"code":"ISSUE_NOT_FOUND","message":"Issue not found: deadbeef"}}
doc list    -> {"success":false,"error":{"code":"EXECUTION_ERROR",
                "message":"Command failed: jit doc list deadbeef --json\nError: Issue not found: deadbeef\n"}}
graph deps  -> {"success":false,"error":{"code":"EXECUTION_ERROR",
                "message":"Command failed: jit graph deps deadbeef --json\nError: Issue not found: deadbeef\n"}}
```

An agent calling `jit_doc_list` receives `EXECUTION_ERROR` with the plain-text diagnostic embedded in a shell-command string. The failure reason is undecodable without parsing prose.

Separate latent bug worth flagging (not in scope, but adjacent): `cli-executor.js:56` and `:36` test `result.success === false`. The CLI envelope has **no** `success` field, so that branch is dead — an envelope printed on a **zero**-exit invocation would be reported as a *success*. The catch-path branch at `:75` uses `!result.success` (truthy test) and therefore works, which is why the measured behavior above is correct.

---

## The census question (REQ-01 / REQ-06)

### How arms are declared

`crates/jit/src/cli.rs` is clap-derive. `Cli` (`cli.rs:26`) carries exactly two root options: `quiet` with `#[arg(short, long, global = true)]` (`cli.rs:28`) and `schema` with `#[arg(long)]` (`cli.rs:32`).

**`--json` is per-arm, not global.** It appears as a plain `json: bool` field with `#[arg(long)]` on individual `Subcommand` enum variants — 100 occurrences of `json: bool` across `cli.rs`. No `global = true` appears anywhere except `quiet`. There is therefore **no derive-level property that distinguishes "accepts `--json`"** beyond "this variant has a field named `json`". At the clap-runtime level the distinguishing property is exactly `Command::get_arguments()` containing an `Arg` whose long is `json` — which is what the schema already computes.

### F-SCHEMA-DERIVED — The `--schema` machinery is derived from clap

`crates/jit/src/schema.rs`, entry point `CommandSchema::generate()` (`schema.rs:162`). It is **derived, not hand-maintained**: `crate::cli::Cli::command()` (`schema.rs:163`) via `clap::CommandFactory`, then a recursive walk (`extract_command_with_path_hidden`, `schema.rs:275`) that records every subcommand, its positional `args` (`extract_argument`, `:351`) and its `flags` (`extract_flag`, `:375`) straight off the clap `Arg`s. It qualifies as a derived source under `@/inv/single-source-prose`, and the derivation binds to clap's own runtime — not to a hand-written mirror, so it does not trip the circular-projection-guard concern.

What it **does** record: every arm, and whether that arm has a `json` flag. That is exactly REQ-01's input.

What it does **not** record:
- Whether an arm's failure path renders the envelope. Nothing in the schema models this.
- Per-arm error codes. `OutputSchema.error` is the hardcoded string `"ErrorResponse"` for every command (`schema.rs:710`), pointing at a type registered in `types` (`schema.rs:774`) — a single shape, not per-arm classification.
- Per-arm exit codes only at *family* granularity: `command_exit_codes` (`schema.rs:847`) is a hand-authored `Vec<CommandExitCode>` of 30 rows keyed by prose strings like `"gate evaluate, gate evaluate-all"` and `"any command that writes an issue"`. It is hand-authored but *bound by test* — see below.

Three hand-maintained lists inside `schema.rs` are worth naming because a plan touching this file will meet them: `hidden_commands()` (`schema.rs:228`, 35 literal paths), `builtin_global_flags()` (`:209`), and `get_output_schema_for_command` (`:456`, a `match` over ~100 literal command paths).

### F-NO-STATIC — Dispatch structure in `main.rs`: no static reading decides the property

`run()` (`main.rs:1870`) is one 6000-line `match command` over `Commands`, with nested `match` blocks per namespace. `Commands::Item`, `Commands::Invariant`, `Commands::Project` delegate to wrapper functions (`run_item` `:1285`, `run_invariant` `:1368`, `run_project` `:1434`) that catch the inner result and classify centrally — an existing precedent for group-level central conversion.

An arm's failure path **cannot be identified structurally with confidence**. Bodies mix `?` propagation, `match … Err(e) =>` with inline classification, and helper calls that never return (`invalid_argument`, `resolve_gate_key_for`, `render_gate_pass_error`, `profile_result` all `std::process::exit` internally). Worse, an arm having an envelope route does not mean all of its failures take it — verified partial coverage:

- `issue claim` (`main.rs:3120`): envelope only for `TransitionBlockedError` (`:3160-3167`); every other failure returns `Err(e)` at `:3169`. Measured: `issue claim deadbeef agent:x --json` → empty stdout, `Error:` on stderr, exit 3.
- `issue update` (`main.rs:2771`): envelope for `TransitionBlockedError`/`IssueNotFoundError`/generic at `:2946-2959`, but id resolution happens earlier and propagates. Measured: `issue update deadbeef --state done --json` → empty stdout, exit 3.
- `gate status` (`main.rs:3874`): five separate envelope sites plus `resolve_gate_key_for`; other paths propagate.

**Consequence for REQ-01/REQ-06: a purely static source-scanning check cannot decide this property.** Any credible guard is either a runtime sweep or a refactor that makes the property structural (see Architecture fit).

### Existing repository-level structural-check mechanisms REQ-06 could reuse

| Mechanism | Where | Can it detect "this arm can reach the plain-text printer on failure"? | Cost |
|---|---|---|---|
| Schema-walking Rust test with a completeness assertion | `crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs`, esp. `test_command_exit_codes_every_row_is_verified` (`:759`) | **Yes, if paired with a runtime probe.** This is the closest precedent: it enumerates rows from `CommandSchema::generate()` and fails when a row lacks a binding. The same shape — enumerate all `--json` arms from the schema, fail if one lacks a registered failure probe — makes a *newly added arm* fail the build without needing to know its failure mode. | Low. Reuses `CommandSchema::generate()` and an existing test target. |
| Subprocess-driving integration tests | same file; `crates/jit/tests/cli_repo_workflow/test_cli_consistency.rs` (whose header already claims "All commands support --json") | **Yes**, this is the only way to observe the actual property. | Moderate–high: one forced-failure invocation per arm × 99. Each `Command::new(jit_binary())` spawn is ~10-50 ms, so ~5 s serial for the sweep — acceptable. Authoring the 99 forced-failure fixtures is the real cost. |
| `scripts/*.sh` gate checkers (`docs-mechanical.sh`, `docs-check-projections.sh`, `rust-build-budget.sh`) wired in `.jit/gates.toml` | `scripts/`, `.jit/gates.toml` | **Only for a source-text heuristic** ("no `?;` in a dispatch arm body"), which the partial-coverage evidence above shows is unsound in both directions. A shell checker *could* drive the binary, but it would duplicate the Rust harness for no gain. | Low to write, high false-positive rate. Not recommended as the primary guard. |
| Compile-time policy tests | `crates/jit/tests/scratch_build/build_profile_policy_tests.rs`, `dependency_feature_policy_tests.rs` | **No.** These assert manifest/profile facts, not control flow. | — |

### F-FORCED-FAILURE — Is a forced failure reachable for every arm?

No. Of 99 arms I drove 61 into a real in-arm failure; 7 more were rejected by clap before dispatch, and for 31 I could construct no failing argument or flag value at all. Those 31 take no argument that can be made invalid and have no invalid flag value:

`archive candidates`, `archive document`, `config list-templates`, `config show`, `config show-hierarchy`, `config validate`, `doc conformance`, `events query`, `gate define`, `gate list`, `gate preset apply`, `gate preset list`, `graph roots`, `init`, `invariant check`, `item list`, `item search`, `label namespaces`, `label values`, `profile list`, `project render`, `query all`, `query blocked`, `query closed`, `query count`, `query divergence`, `query strategic`, `serve`, `status`, `version`, `worktree list`.

(`gate define` and `gate preset apply` are in this list only because my probe arguments happened to succeed; both have constructible failures — a duplicate key and a real preset against a bad id — which I confirmed separately in §5.)

Two universal forced failures do exist for that set:
1. **Run outside a repository.** Every arm past `run()`'s discovery step fails with `RepositoryNotFoundError`. Verified: `jit status --json` in an empty directory emits the `REPOSITORY_NOT_FOUND` envelope on stdout, exit 3 — via `emit_startup_json_error`, i.e. *already* satisfying the contract for all of them.
2. **Corrupt the store.** Making `.jit/issues/*.json` unparseable or unreadable forces an I/O/parse failure inside the arm body, which is the path REQ-02 actually cares about for these arms.

`version` genuinely cannot fail: it is served before repository discovery (`main.rs:1895-1913`) and its only fallible call is `to_json_string()` on a struct of `&'static str`s. Any REQ-06 guard needs an explicit, justified exemption for it — and probably for `serve`, whose failure model is a daemon-control exit, not an error envelope.

---

## Consumer sweep — all 99 arms accepting `--json`

Derivation: schema walk (§Method) crossed with the `main.rs` dispatch site and, where a forced failure was constructible, the observed runtime verdict. `VERDICT` legend — `ENVELOPE`: forced failure produced an envelope on stdout; `PLAIN`: forced failure produced empty stdout + `Error:` on stderr; `CLAP`: my probe argument was rejected by clap before dispatch, so the arm was not reached (mechanism column is from source only); `n/f`: no failure could be forced with an argument or flag value.

Probe outcome totals across the 99 arms: **31 ENVELOPE, 30 PLAIN, 7 CLAP, 31 n/f**.

**Mechanism legend** — `hje` = `handle_json_error!`; `adhoc` = inline `println!(json_error)+exit`; `helper` = one of `profile_result`/`claim_json_error`/`render_gate_pass_error`/`resolve_gate_key_for`/`invalid_argument`; `none` = no envelope route on any failure path.

### F-CLAP-PARSE — Argument-parse failures are a distinct class the plan must scope explicitly

A malformed invocation (`jit query count --by bogus --json`, `jit hooks install --hook x --json`) is rejected inside `Cli::parse()` at `main.rs:1878`, before any dispatch. Clap prints its own `error: invalid value …` to stderr and exits 2 on its own. No `--json` arm can render an envelope for that class without replacing clap's error handling wholesale (`try_parse` + a jit-owned renderer). REQ-02 as written — "every arm accepting the flag renders its failures as the structured error envelope" — is unachievable for argument-parse failures unless the plan either carves them out or takes on clap error handling.

### `issue` (16 arms)

| Arm | Dispatch | Mechanism | Verdict | Code seen |
|---|---|---|---|---|
| `issue show` | main.rs:2503 | hje (2566, 2652) | ENVELOPE | `ISSUE_NOT_FOUND` |
| `issue status` | main.rs:2660 | hje (2681) | ENVELOPE | `ISSUE_NOT_FOUND` |
| `issue children` | main.rs:2710 | hje (2730) | ENVELOPE | `ISSUE_NOT_FOUND` |
| `issue progress` | main.rs:2738 | hje (2763) | ENVELOPE | `ISSUE_NOT_FOUND` |
| `issue update` | main.rs:2771 | hje (2959) — **partial**, id resolution propagates | PLAIN | — |
| `issue delete` | main.rs:3037 | adhoc (3052-3062) | ENVELOPE | `DELETION_NOT_CONFIRMED` |
| `issue claim` | main.rs:3120 | adhoc (3164-3166) — **partial**, blocked-transition only | PLAIN | — |
| `issue claim-next` | main.rs:3263 | adhoc (3277-3279) — **partial** | PLAIN | — |
| `issue create` | main.rs:2273 | none (+`invalid_argument` for flag conflicts) | PLAIN | — |
| `issue batch-create` | main.rs:2359 | none | PLAIN | — |
| `issue search` | main.rs:2412 | none | PLAIN | — |
| `issue assign` | main.rs:3096 | none | PLAIN | — |
| `issue unassign` | main.rs:3193 | none | PLAIN | — |
| `issue reject` | main.rs:3210 | none | PLAIN | — |
| `issue release` | main.rs:3246 | none | PLAIN | — |
| `issue list` | main.rs:3303 | none | PLAIN | — |

### `doc` (10 arms)

| Arm | Dispatch | Mechanism | Verdict |
|---|---|---|---|
| `doc dir` | main.rs:5097 | hje (5142) | ENVELOPE `ISSUE_NOT_FOUND` |
| `doc add` | main.rs:4970 | none | PLAIN |
| `doc list` | main.rs:5018 | none | PLAIN |
| `doc remove` | main.rs:5056 | none | PLAIN |
| `doc show` | main.rs:5076 | none | PLAIN |
| `doc history` | main.rs:5204 | none | PLAIN |
| `doc diff` | main.rs:5226 | none | PLAIN |
| `doc assets list` | main.rs:5245 | none | PLAIN |
| `doc check-links` | main.rs:5383 | none (findings exit at 5449) | PLAIN |
| `doc conformance` | main.rs:5146 | none | n/f |

### `gate` (15 arms)

| Arm | Dispatch | Mechanism | Verdict |
|---|---|---|---|
| `gate define` | main.rs:3485 | adhoc (3590-3592) | n/f (static: `GATE_ERROR`, exit 1 vs plain 6) |
| `gate update` | main.rs:3599 | adhoc (3733) | ENVELOPE `GATE_NOT_FOUND` |
| `gate list` | main.rs:3741 | adhoc (3773) | n/f |
| `gate show` | main.rs:3781 | adhoc (3837) | ENVELOPE `GATE_NOT_FOUND` |
| `gate remove` | main.rs:3844 | adhoc (3864) | ENVELOPE `GATE_NOT_FOUND` |
| `gate status` | main.rs:3874 | adhoc (3972, 3992, 4085, 4121, 4162) + `resolve_gate_key_for` | ENVELOPE `INVALID_ARGUMENT` |
| `gate status-all` | main.rs:4171 | none (findings exit at 4269) | PLAIN |
| `gate add` | main.rs:4272 | adhoc (4334) | ENVELOPE `ISSUE_NOT_FOUND` |
| `gate evaluate` | main.rs:4342 | `render_gate_pass_error` (4394) | ENVELOPE `ISSUE_NOT_FOUND` |
| `gate evaluate-all` | main.rs:4398 | `render_gate_pass_error` (4460) | ENVELOPE `ISSUE_NOT_FOUND` |
| `gate fail` | main.rs:4464 | adhoc (4499) | ENVELOPE `GATE_ERROR` |
| `gate preset list` | main.rs:4508 | adhoc (4549) | n/f |
| `gate preset show` | main.rs:4557 | adhoc (4614) | ENVELOPE `PRESET_ERROR` |
| `gate preset apply` | main.rs:4622 | none — errors folded into the **success** payload (4667-4672), exit 0 | n/f |
| `gate preset create` | main.rs:4696 | adhoc (4718) | ENVELOPE `PRESET_ERROR` |

### `claim` (7 arms) — all via `claim_json_error`

`claim acquire` main.rs:7308 (7348-7350) · `claim release` :7357 (7393-7395) · `claim renew` :7402 (7435-7437) · `claim heartbeat` :7444 (7468-7470) · `claim status` :7477 (7556-7558) · `claim list` :7565 (7629-7631) · `claim force-evict` :7638 (7668-7670). All ENVELOPE. Codes observed: `CLAIM_ACQUIRE_ERROR`, `CLAIM_RELEASE_ERROR`, `CLAIM_REQUIRES_GIT`.

### `query` (7 arms) — all `none`

`query all` main.rs:5564 · `query available` :5576 · `query blocked` :5622 · `query strategic` :5710 · `query closed` :5755 · `query count` :5800 · `query divergence` :5824. `query available --label bogus` measured PLAIN, exit 2; the rest n/f.

### `graph` (5 arms) — all `none`

`graph deps` main.rs:4729 (PLAIN) · `graph rdeps` :4777 (PLAIN) · `graph roots` :4801 (n/f) · `graph tree` :4824 (PLAIN) · `graph export` :4861 (CLAP).

### `config` (6 arms)

`config get` main.rs:6043 — hje (6074), ENVELOPE `INVALID_ARGUMENT`. `config show` :5907, `config set` :6085, `config validate` :6116 (findings exit at 6194), `config show-hierarchy` :6197, `config list-templates` :6221 — all `none`.

### `item` / `invariant` / `project` (6 arms) — wrapper-level `handle_json_error!`

`item list` main.rs:1342 · `item search` :1346 · `item show` / `item resolve` :1350 — all via `run_item` (`:1303`), code `ITEM_COMMAND_FAILED`. `invariant check` :1394 via `run_invariant` (`:1377`), `INVARIANT_COMMAND_FAILED`. `project render` :1471 via `run_project` (`:1458`), `VALIDATION_FAILED` or `PROJECT_COMMAND_FAILED`.

### `profile` (3 arms) — `profile_json_error`

`profile list` main.rs:2154 (2171-2173, n/f) · `profile show` :2177 (2202-2204, ENVELOPE `PROFILE_NOT_FOUND`) · `profile apply` :2208 (2232-2234, 2258-2260, ENVELOPE `PROFILE_NOT_FOUND`).

### Remaining 24 arms

| Arm | Dispatch | Mechanism | Verdict |
|---|---|---|---|
| `init` | main.rs:2011 | `profile_result` (2041, 2052) + `invalid_argument` (2026) | n/f |
| `version` | main.rs:1895 | pre-dispatch; infallible | n/f |
| `apply` | main.rs:3317 | none | PLAIN |
| `dep add` | main.rs:3350 | adhoc `dep_add_batch_json_error` (3409-3412) + hje (3419) | ENVELOPE `DEPENDENCY_ERROR` |
| `dep rm` | main.rs:3427 | hje (3473) | ENVELOPE `DEPENDENCY_ERROR` |
| `list` | main.rs:2131 | rewritten to `issue list`; none | PLAIN |
| `rdeps` | main.rs:2128 | rewritten to `graph rdeps`; none | PLAIN |
| `events tail` | main.rs:4931 | none | CLAP |
| `events query` | main.rs:4947 | none | n/f |
| `archive candidates` | main.rs:5453 | none | n/f |
| `archive document` | main.rs:5461 | none | n/f |
| `archive container` | main.rs:5497 | none | PLAIN |
| `label namespaces` | main.rs:5848 | none | n/f |
| `label values` | main.rs:5874 | none | n/f |
| `hooks install` | main.rs:6258 | adhoc (6294-6299), covers every `install_hooks` error | CLAP |
| `search` | main.rs:6316 | adhoc (6408-6413), **hardcoded exit 10** | CLAP |
| `status` | main.rs:6420 | none | n/f |
| `validate` | main.rs:6441 | `validate_fix_json_error` (6742-6744) — `--fix` path only | PLAIN |
| `recover` | main.rs:6924 | adhoc to **stderr** (6989-6991) | CLAP |
| `migrate lifecycle-timestamps` | main.rs:7000 | none | CLAP |
| `serve` | main.rs:7022 | none (eprintln + exit 1) | n/f |
| `worktree info` | main.rs:7679 | adhoc (7721-7723) | ENVELOPE `WORKTREE_INFO_ERROR` |
| `worktree list` | main.rs:7730 | adhoc (7774-7776) | n/f |
| `snapshot export` | main.rs:7785 | none | CLAP |

**Totals: 48 arms have at least one envelope route (several only partial); 51 have none.**

---

## Primitive verification

| Property the plan may assert | Verdict | Evidence |
|---|---|---|
| "The shared handler classifies by downcast" | **Contradicted as stated.** `handle_json_error!` (`output_macros.rs:70`) does **not** classify. It takes an already-built `JsonError` from the call site and only *refines* it via `refine_id_error` (`output.rs:717`), which downcasts for exactly two types (`InvalidIdPrefixError`, `AmbiguousIdError`). All other classification is per-call-site. | `output_macros.rs:71-82`, `output.rs:717-731` |
| "Exit code is derived from the error code" | **Confirmed for the macro and most helpers**, contradicted at two sites. `JsonError::exit_code()` = `ErrorCode::to_exit_code(&self.error.code)` (`output.rs:507`), and the macro exits on `json_error.exit_code().code()` (`output_macros.rs:78`). Contradicted at `main.rs:6413` (hardcoded `exit(10)`) and `main.rs:6991` (hardcoded `exit(1)`). | as cited |
| "`--json` is a global flag" | **Contradicted.** Only `--quiet` is `global = true` (`cli.rs:28`). `--json` is a per-variant `#[arg(long)] json: bool`, 100 occurrences in `cli.rs`. | `cli.rs:26-37` |
| "The schema is derived from clap" | **Confirmed.** `CommandSchema::generate()` uses `Cli::command()` via `CommandFactory` and walks clap's own `get_subcommands()`/`get_arguments()`. | `schema.rs:8`, `:162-203`, `:275-348` |
| "`ErrorCode::to_exit_code` covers every code the CLI emits" | **Contradicted.** At least 21 code strings emitted by dispatch are absent from the `ErrorCode` constants and fall to the `_ => GenericError` arm, so every failure carrying one exits `1` regardless of its real class: `DEPENDENCY_ERROR`, `GATE_ERROR`, `GATE_CHECK_ERROR`, `PRESET_ERROR`, `ITEM_COMMAND_FAILED`, `INVARIANT_COMMAND_FAILED`, `PROJECT_COMMAND_FAILED`, `PROFILE_ERROR`, `SEARCH_FAILED`, `RIPGREP_NOT_FOUND`, `WORKTREE_INFO_ERROR`, `WORKTREE_LIST_ERROR`, `HOOKS_INSTALL_ERROR`, `GENERIC_ERROR`, `recovery_failed`, and the seven `CLAIM_*_ERROR` fallbacks. (`"GATE_NOT_FOUND"` at `main.rs:4330` is a string literal whose value matches the `ErrorCode::GATE_NOT_FOUND` constant, so it maps correctly by coincidence of spelling.) | `output.rs:644-704` vs the dispatch sites in §1 |
| "`error_to_exit_code` classifies by typed downcast, never message text" | **Confirmed.** 20+ downcast branches, no string matching; final fallback at `main.rs:331`. Pinned by `test_error_to_exit_code_produces_documented_codes` (`main.rs:8040`). | `main.rs:49-332` |

## Architecture fit

**Primitives a conversion should reuse:** `JsonError` and its factories (`output.rs:465`, `:736-822`), `ErrorCode` constants and `ErrorCode::to_exit_code` (`output.rs:640`, `:681`), `ExitCode` (`output.rs:539`), `refine_id_error` (`output.rs:717`), `error_to_exit_code` (`main.rs:49`), and the per-namespace `*_json_error` builders (`dep_add_batch_json_error` `main.rs:361`, `claim_json_error` `:492`, `validate_fix_json_error` `:507`, `stale_binary_json_error` `:640`, `profile_json_error` `:796`).

**Layer boundaries.** `JsonError`, `ErrorCode`, `ExitCode`, and `refine_id_error` all live in `crates/jit/src/output.rs`, which the project's stated boundary assigns to CLI/output. `error_to_exit_code` lives in `main.rs`. Both are on the correct side: rendering and exit-status classification belong to CLI/output, not to `commands/` or `domain/`. The typed errors being downcast (`jit::errors::*`, `jit::storage::*`, `jit::commands::*`) are already public domain/storage types, so a central classifier consumes them without inverting any dependency.

**A central conversion would not violate a boundary.** Classifying once in `run()`/`main()` — a single `anyhow::Error → JsonError` function next to `error_to_exit_code` — stays entirely inside the CLI/output layer, and there is existing precedent for it at the namespace level (`run_item` `main.rs:1285`, `run_invariant` `:1368`, `run_project` `:1434` each catch their inner result and classify centrally). The design tension is different from a boundary violation:

- *Central classifier.* One `anyhow::Error → JsonError` mapping, structurally parallel to `error_to_exit_code`, applied wherever `run()` returns `Err` and `--json` was requested. Makes REQ-02 and REQ-04 hold by construction (one classifier feeds both the envelope code and the exit code) and makes REQ-06 trivially satisfiable, since a new arm inherits the behavior with no per-arm work. Costs: `--json` is per-arm, so `main()` cannot see it without either threading the flag out of the parsed command (an exhaustive match over ~100 variants, or a derive/helper) or re-reading argv the way `emit_startup_json_error` already does (`main.rs:1767`); and per-arm `details` enrichment (`issue_id`, `key`, `prefix`, blockers, `checker_result`) is lost unless the central classifier reads it off the typed error, which several current envelopes get from local dispatch context rather than from the error itself.
- *Per-arm downcast (the established pattern).* Preserves rich `details` and command-specific suggestions. Costs: 51 arms to convert, ~48 already-converted arms to audit for partial coverage and for the fallback-code exit-class bug in §5, and REQ-06 needs an independent guard because nothing structural stops the next arm from omitting it.
- *Hybrid.* Central classifier as the floor (guarantees an envelope and a code-consistent exit for every arm), per-arm enrichment retained where it exists. The `run_item`/`run_invariant`/`run_project` wrappers are exactly this shape at namespace granularity.

Reported as trade-offs; no recommendation.

## Architectural-invariant check

`.jit/invariants.toml` declares 14 invariants; the four cited plus `domain-agnostic` bear on this work.

- **`@/inv/single-source-prose`** — Live defect found (§5): `docs/reference/exit-codes.md`, generated by `render_exit_code_reference` (`schema.rs:1068`) from `generate_command_exit_codes` (`schema.rs:847`), documents `gate define → 6` and `dep add → 4` while the `--json` invocations exit 1. The projection is bound to runtime only through the non-`--json` path. Any REQ-07 doc work must not create a second hand-maintained home; the existing canonical homes are `docs/reference/cli-commands.md` §"CLI JSON contracts" (lines 5-45, envelope shape) and `docs/reference/exit-codes.md` (generated, exit codes). REQ-01's census is itself a volatile enumeration — under this invariant it should be derived (from `jit --schema`), not committed as a hand-maintained table.
- **`@/inv/semantic-test-assertions`** — REQ-05 says "envelope structure and classification rather than a copied message string", which matches. The field-name inventory of `ErrorDetail` belongs in one canonical suite; `crates/jit/tests/fast_docs_templates/schema_tests.rs:161` and `crates/jit/tests/cli_repo_workflow/integration_schema.rs:211` already assert the `ErrorResponse` type's presence.
- **`@/inv/shared-test-contracts`** — A 99-arm regression suite must use a shared fixture + a single conformance body parameterized over arms, not 99 hand-written tests. The precedent is `command_exit_code_projection_tests.rs`, whose `documented_row` helper (`:60`) and `test_command_exit_codes_every_row_is_verified` (`:759`) implement exactly this shape.
- **`@/inv/bounded-rust-build-footprint`** — **Headroom is 1.** `scripts/rust-build-budget.sh:37` sets `MAX_INTEGRATION_TARGETS=12`; `cargo metadata` currently reports **11** test targets (`cli_gate`, `cli_issue`, `cli_item_validate`, `cli_query_graph`, `cli_repo_workflow`, `document_api_tests`, `fast_docs_templates`, `fast_issue`, `fast_rules`, `provenance_contract`, `scratch_build`). A new suite must therefore be a **module inside an existing target** (`cli_repo_workflow/` or `cli_issue/` are the natural homes), or it consumes the last slot. The executable-bytes budget is `MAX_EXECUTABLE_BYTES = 2 GiB` (`rust-build-budget.sh:38`); I did not measure current bytes (it requires a full `cargo test --workspace --no-run`), so that headroom is **unverified** here.
- **`@/inv/domain-agnostic`** — No conflict. Error codes and exit codes are engine-level machine contracts, not domain vocabulary. A plan should avoid introducing per-command error codes that encode adopter workflow names.

## Prior art

The sweep of `dev/` and `docs/` found little; nothing supersedes this report.

- `dev/architecture/cli-and-mcp-strategy.md:651` and `:662` record only that `JsonOutput`/`JsonError` exist and the "JSON output foundation" is done. No error-contract facts, no census.
- `docs/reference/cli-commands.md:5-45` is the existing canonical statement of the envelope shape (`error.code`, `error.message`, `error.details`, `error.suggestions`) plus the blocked-transition variants at `:43-110`. `:643`, `:1130`, `:1191`, `:2293` document envelope behavior for specific commands.
- `docs/reference/exit-codes.md` is generated from `schema.rs` and is the canonical exit-code home.
- `crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs` is the exit-code binding suite and the strongest structural precedent for REQ-06.
- `crates/jit/tests/scratch_build/stale_binary_json_exit_tests.rs` is the precedent for testing an envelope + exit-code pair end to end.
- `crates/jit/tests/cli_repo_workflow/test_cli_consistency.rs:1-6` opens with the claim "All commands support --json flag for machine-readable output" but only exercises seven `issue` arms on the success path. That header is a stale over-claim worth correcting alongside this work.
- No `dev/plans/`, `dev/studies/`, or `dev/archive/` document addresses `handle_json_error`, exit-code policy, or an arm census.

## Unverifiable / open

- **Current test-executable bytes vs the 2 GiB budget** — not measured; requires a full `cargo test --workspace --no-run`.
- **`recover`'s failure envelope in practice** — I could not force `executor.recover_transactions()` to fail, so the stderr/compact/`recovery_failed` behavior at `main.rs:6989-6991` is established from source only, not by invocation.
- **The claim-namespace fallback codes** — the scratch repo had no commits, so five `claim` arms short-circuited on `CLAIM_REQUIRES_GIT` (exit 10, agreeing on both paths). Their `CLAIM_*_ERROR` fallbacks are structurally in the same exit-1-vs-typed-class bucket as `claim acquire`/`claim release` (which I did measure disagreeing), but that was not confirmed by invocation for `renew`, `heartbeat`, `status`, `list`, `force-evict`.
- **Whether any arm can print partial success to stdout before failing** — I found none among the 61 arms I drove to failure, and the human-output helpers are all `json`-guarded, but I could not exhaustively drive multi-step arms (`gate evaluate-all` with a mid-list checker failure, `project render` with one failing projection, `validate --fix` with a mid-run write failure) into a partial-output state.
