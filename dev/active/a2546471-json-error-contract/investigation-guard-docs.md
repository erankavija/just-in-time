# a2546471 — investigation: durable guard (REQ-05/REQ-06) and documentation contract (REQ-07)

Read-only investigation. Scope: how CLI failure behaviour is tested today, what
mechanisms could make REQ-06 fail closed, and where the machine-readable failure
contract belongs. Companion to `investigation.md` (arm census, REQ-01), which is
owned by a sibling investigator; the arm counts below are reported as measured,
not adjudicated against that census.

---

## Question A — how failure behaviour is currently tested (REQ-05)

### A.1 Integration-test topology

Eleven integration-test targets exist workspace-wide, measured from
`cargo metadata --no-deps --format-version=1`:

| Target | Entry point |
|---|---|
| `jit::cli_gate` | `crates/jit/tests/cli_gate/main.rs` |
| `jit::cli_issue` | `crates/jit/tests/cli_issue/main.rs` |
| `jit::cli_item_validate` | `crates/jit/tests/cli_item_validate/main.rs` |
| `jit::cli_query_graph` | `crates/jit/tests/cli_query_graph/main.rs` |
| `jit::cli_repo_workflow` | `crates/jit/tests/cli_repo_workflow/main.rs` |
| `jit::fast_docs_templates` | `crates/jit/tests/fast_docs_templates/main.rs` |
| `jit::fast_issue` | `crates/jit/tests/fast_issue/main.rs` |
| `jit::fast_rules` | `crates/jit/tests/fast_rules/main.rs` |
| `jit::provenance_contract` | `crates/jit/tests/provenance_contract/main.rs` |
| `jit::scratch_build` | `crates/jit/tests/scratch_build/main.rs` |
| `jit-server::document_api_tests` | `crates/server/tests/` |

Each `main.rs` is a module aggregator (`crates/jit/tests/cli_issue/main.rs:1-27`);
individual `*_tests.rs` files are `mod` declarations, not separate Cargo targets.

### A.2 What already asserts on `--json` failure output

| File | Lines | What it asserts | Style |
|---|---|---|---|
| `crates/jit/tests/cli_issue/error_json_tests.rs` | 36-41, 91-96, 131-142 | `json["error"]["code"]` / `["message"]` / `["suggestions"]` for `issue show`, `dep add`, `issue delete` | raw `std::process::Command` + `serde_json::from_str` |
| `crates/jit/tests/cli_issue/exit_code_tests.rs` | 1500-1509, 1512-1525, 1527-1554 | startup-failure envelope **on stdout** with the human line **on stderr**; `stdout.is_empty()` in text mode | raw `Command`, `output.status.code()` |
| `crates/jit/tests/cli_issue/exit_code_tests.rs` | 1567-1584 | `issue delete` refusal exits 2, stderr carries the hint, stdout stays empty | same |
| `crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs` | 61-75, 79-101, 759-812 | every documented exit-code row is reachable and bound to runtime | raw `Command` + `CommandSchema::generate()` |
| `crates/jit/src/main.rs` | 7851-… (`mod exit_code_projection_tests`) | typed errors through `error_to_exit_code` land on the documented row | in-crate unit test |
| `crates/jit/tests/scratch_build/stale_binary_json_exit_tests.rs` | 292 | `STALE_BINARY` envelope with exit 10 matching text mode | raw `Command` |

Eighteen further test files across all five `cli_*` suites assert
`json["error"]["code"]` for one command each (`verb_hint_tests.rs`,
`profile_cli_tests.rs`, `profile_acceptance_tests.rs`, `item_cli_tests.rs`,
`config_get_tests.rs`, `init_tests.rs`, `first_guess_residuals_test.rs`,
`container_rollup_tests.rs`, `dep_add_redundancy_cli_tests.rs`,
`gate_update_test.rs`, `gate_evaluate_exit_code_test.rs`,
`gate_status_history_flat_test.rs`, `gate_evaluate_all_test.rs`,
`gate_findings_test.rs`, `issue_status_projection_tests.rs`,
`project_render_cli_tests.rs`, `derived_state_repair_tests.rs`,
`claim_integration_tests.rs`). They are per-feature, not a contract suite.

**Two existing tests in `error_json_tests.rs` are non-binding and would pass
against the defect this epic repairs:**

- `test_gate_operation_error_json` (`error_json_tests.rs:179-207`) parses stdout
  and discards the result — its only surviving assertion is that stdout is valid
  JSON. The trailing comment reads `// Verify JSON structure - envelope removed,
  just check valid JSON`.
- `test_invalid_state_error_json` (`error_json_tests.rs:154-176`) asserts a
  three-way disjunction across stdout **or** stderr
  (`stdout.contains("INVALID") || stderr.contains("invalid") || stderr.contains("state")`),
  so it passes whether or not the envelope is emitted.

Both are in the epic's blast radius and should be tightened or replaced by REQ-05
rather than left as apparent coverage.

### A.3 Shared fixtures available (`@/inv/shared-test-contracts`)

- **In-process harness:** `crates/jit/tests/common/harness.rs` — `TestHarness`
  (`:14-42`) wrapping `CommandExecutor<InMemoryStorage>`, with fluent helpers
  (`create_issue` `:102`, `create_ready_issue` `:159`, `add_gate` `:200`) and
  aggregate seeders (`seed_memory_issue` `:228`, `seed_memory_event` `:233`,
  `seed_memory_gate_registry` `:248`). Included via `#[path = "../common/harness.rs"]`
  by exactly four suites: `fast_docs_templates/main.rs:6`, `fast_issue/main.rs:6`,
  `fast_rules/main.rs:5`, `cli_query_graph/main.rs:5`. **Not** by `cli_issue`,
  `cli_gate`, `cli_repo_workflow`, or `cli_item_validate`.
- **No shared subprocess CLI runner exists.** This is a finding, not an omission
  in my search. 60 test files under `crates/jit/tests/` define their own binary
  resolution and temp-repo setup. Two idioms coexist:
  - `env!("CARGO_BIN_EXE_jit")` — e.g. `error_json_tests.rs:4-6`,
    `list_envelope_tests.rs:16`, `exit_code_tests.rs:9`,
    `command_exit_code_projection_tests.rs:30`
  - `assert_cmd::cargo::cargo_bin!("jit")` — e.g. `issue_show_shape_test.rs:14`,
    `gate_field_contract_test.rs:32`, `issue_create_json_contract_test.rs:14`

  37 files use `assert_cmd`. Both crates are declared dev-dependencies:
  `crates/jit/Cargo.toml` `assert_cmd = { workspace = true }` /
  `predicates = { workspace = true }`, pinned in `Cargo.toml:` `assert_cmd = "2.0"`,
  `predicates = "3.1"`, `tempfile = "3.10"`.

  REQ-05 driving ~60-99 arms through subprocess invocation is the first thing in
  the tree large enough to justify a shared runner, and `@/inv/shared-test-contracts`
  points that way. Whether to extract one (and retrofit callers) or add a
  suite-local one is a plan decision with a real churn cost.

### A.4 Is there a canonical JSON-contract suite? (`@/inv/semantic-test-assertions`)

**Success side — yes, two.**

- `crates/jit/tests/cli_issue/list_envelope_tests.rs` is the canonical list-envelope
  contract. Its module doc (`:1-9`) enumerates every collection key
  (`issues`, `gates`, `events`, `documents`, `leases`, `namespaces`, `values`,
  `presets`, `assets`, `items`, `worktrees`, `roots`, `dependents`, `findings`,
  `templates`, `results`) and its `assert_envelope` helper (`:36-…`) is the shared
  assertion.
- `crates/jit/tests/cli_issue/gate_field_contract_test.rs` is the canonical
  cross-surface contract for gate-list field naming (`:1-24`), explicitly paired
  with the `crate::schema` unit tests so a rename fails on both sides.

**Error side — no.** `error_json_tests.rs` is the nearest thing: 207 lines,
5 tests, 4 command arms, no module doc claiming canonical status, and two of the
five tests non-binding (A.2). Nothing enumerates the error envelope's fields or
the error-code vocabulary in one place.

Consequence for the plan: REQ-05 **creates** the canonical error-envelope suite
rather than extending one. The natural home is `cli_issue`, beside
`list_envelope_tests.rs` — that suite already owns every existing machine-contract
assertion (`list_envelope`, `error_json`, `exit_code`, `command_exit_code_projection`,
`issue_create_json_contract`, `gate_field_contract`).

### A.5 Build-footprint headroom (`@/inv/bounded-rust-build-footprint`)

Budgets are declared once at `scripts/rust-build-budget.sh:39-40`:
`MAX_INTEGRATION_TARGETS=12`, `MAX_EXECUTABLE_BYTES=2 GiB`.

Measured live (`CARGO_INCREMENTAL=0 ./scripts/rust-build-budget.sh`, exit 0):

```
rust-build-budget: integration-targets=11/12 active-executables=15 \
  bytes=1274215192/2147483648 profile=line-tables-only incremental=disabled \
  dep-features=pruned
```

- **Integration targets:** 11 of 12 — **one slot free.**
- **Executable bytes:** 1,274,215,192 of 2,147,483,648 — 59.3% used,
  **873,268,456 bytes (~833 MiB) free** across 15 active executables
  (mean ≈ 85 MB each).

**A per-arm failure suite must go inside an existing target.** Adding a 12th
consumes the last slot and links a fresh executable of roughly mean size,
spending ~10% of the byte budget for one file. The right home is **`cli_issue`**
(`crates/jit/tests/cli_issue/main.rs`), on three grounds: it already aggregates
every machine-contract suite (A.4); a new module there adds compiled code to an
executable that is already linked, not a new one; and the completeness-assert
pattern REQ-06 would reuse (B.1) already lives in that target
(`command_exit_code_projection_tests.rs:759`).

---

## Question B — candidate mechanisms for the durable check (REQ-06)

Prerequisite fact used by every option below: **`--json` is declared per arm, not
globally.** `Cli` (`crates/jit/src/cli.rs:26-37`) declares only `quiet` as
`global = true`; `schema` is top-level-only. Each command variant carries its own
`#[arg(long)] json: bool` (e.g. `cli.rs:52-53`). `jit --schema`'s
`global_options` are exactly `quiet`, `schema`, `help`, `version`.

**Measured arm counts** (`jit --schema`, recursive jq walk of `.commands`,
leaf = a command with no `subcommands`):

| | count |
|---|---|
| leaf arms total | 112 |
| accept `--json`, **visible** | **64** |
| accept `--json`, **hidden** | **35** |
| accept no `--json` | 13 |

The 64 visible arms reconcile with the epic's "roughly 63". Hidden status comes
from `CommandSchema::hidden_commands()` (`crates/jit/src/schema.rs:228-268`) — an
MCP tool-listing filter, not an executability filter: hidden arms still run and
still take `--json`. **Guard scope (64 vs 99) is an explicit plan decision, not a
detail.**

The 13 no-`--json` leaves are all wrong-verb stubs (`dep delete`, `dep remove`,
`doc delete`, `doc rm`, `issue rm`, `issue remove`, `issue complete`, `issue edit`,
`gate rm`, `gate delete`, `label rm`, `label remove`, `label add`). They
nonetheless honour `--json`: `verb_hint_error` (`main.rs:893-905`) scans its
captured trailing argv for a literal `--json` at `:894` and routes through
`invalid_argument`, which prints the envelope and exits. Verified:

```
$ jit dep remove aaaa bbbb --json
{ "error": { "code": "INVALID_ARGUMENT", "message": "'jit dep remove' is not a jit command. Use 'jit dep rm' instead." } }
# stdout only; stderr empty; exit 2
```

So **any reflection keyed on the declared flag misses 13 arms that do have a
`--json` failure contract.** That is a concrete false-negative for option 1.

### Existing failure-emission mechanisms (the ground truth all options sit on)

1. `handle_json_error!` (`crates/jit/src/output_macros.rs:70-83`) — 18 call sites
   in `main.rs` (1303, 1377, 1458, 2566, 2652, 2681, 2730, 2763, 2959, 3419,
   3473, 5142, 6074, …). Refines id errors via `jit::output::refine_id_error`
   (`output.rs:717-731`), prints the envelope, `process::exit`s.
2. `invalid_argument(message, json)` (`main.rs:784-794`) — 18 call sites. Prints
   the envelope and exits when `json`, otherwise returns a typed
   `InvalidArgumentError`.
3. `emit_startup_json_error` (`main.rs:1766-1791`) — called from `main()` at
   `:1729`. Reads `--json` from argv (`:1767`) and renders an envelope for exactly
   three classes: `REPOSITORY_NOT_FOUND`, `REPOSITORY_FORMAT_TOO_NEW`,
   `STALE_BINARY` (`:1772-1786`).

The top-level plain-text printer is `main.rs:1728`: `eprintln!("Error: {}", e)`,
on **stderr**, followed by `error_to_exit_code(&e)` (`:1730`). Confirmed live:

```
$ jit doc list nosuchid --json
# stdout: (empty)
# stderr: Error: Issue not found: nosuchid
# exit 3
```

Note for REQ-03 as worded ("no plain-text diagnostic on the stream that carries
the envelope"): the envelope goes to stdout and the plain line to stderr, so the
split is already the intended design — `exit_code_tests.rs:1500-1509` pins it
deliberately ("Human line stays on stderr"). REQ-03 is satisfied by that split
without suppressing stderr. Whether the stderr line *should* also be suppressed
under `--json` is a question REQ-03's wording does not settle; the plan should say
which reading it takes.

### Option 1 — runtime exhaustive test

**Enumeration: feasible, with strong precedent.** `CommandSchema::generate()`
(`crates/jit/src/schema.rs:162-203`) already reflects `crate::cli::Cli::command()`
through `clap::CommandFactory` (`schema.rs:8`) and walks the tree with
`get_subcommands()` (`:182`, `:297`) and `get_arguments()` (`:168`, `:326`),
reading `Arg::get_long()` (`:377`), `Arg::get_id()` (`:328`, `:352`), and
`is_positional()` (`:332`).

clap version: `Cargo.toml:27` declares `clap = { version = "4.5", features = ["derive"] }`;
`Cargo.lock:433-434` resolves **clap 4.5.53**. The introspection API is not a
guess — it is compiled and shipping in this tree today.

Because `CommandSchema` is a public type (`crates/jit/src/lib.rs:42`), a test can
enumerate arms with **zero new machinery**: walk `CommandSchema::generate().commands`
and select leaves whose `flags` contain `json`. No hand-maintained list, so
`@/inv/single-source-prose` is satisfied on the enumeration side.

**Forced failure: this is where it gets hard, and it is not uniformly solvable.**
The universal lever is a bad issue id, but arms taking no rejectable argument have
no such lever. `doc conformance` says so in its own code comment
(`main.rs:5148-5155`): "This command takes no argument it could reject and states
no verdict in its exit status". `init`, `serve`, `profile list`,
`gate preset list`, `config list-templates`, and the `query` family are in the
same position. A per-arm forcing recipe is therefore unavoidable — and a recipe
table is precisely the hand-maintained mirror the epic wants to avoid.

**The tree already solves that exact shape.**
`test_command_exit_codes_every_row_is_verified`
(`crates/jit/tests/cli_issue/command_exit_code_projection_tests.rs:759-812`)
builds a `verified` set by hand, derives a `projected` set from
`CommandSchema::generate()`, and asserts **set equality**. Adding a projection row
without a binding fails the build. Applied here: the recipe table is *declared*
but not *authoritative* — the clap reflection is the authority, and an
unregistered new arm fails.

- **Fails closed?** Yes, for arms clap declares a `json` flag on.
- **False negatives:** (a) the 13 verb-hint stubs, invisible to flag reflection;
  (b) an arm whose forcing recipe drives it into a *different* failure than the
  one that would propagate — the test passes on the envelope it reached, not the
  one the untested path would produce; (c) arms whose only realistic failure is
  infrastructure (unreadable `config.toml`) and which by design return to the
  top-level handler (`main.rs:5139-5141`, `5148-5155`) — the plan must decide
  whether those count as violations.
- **Cost:** 64-99 subprocess spawns plus `jit init` per temp repo. Contained by
  putting it in `cli_issue` (A.5). No new target, negligible byte growth.

### Option 2 — static/source check over `main.rs`

**Precedent shape exists** (`scripts/rust-build-budget.sh`, `scripts/docs-check-*.sh`),
but the target text is hostile.

`run()` spans `main.rs:1870` to roughly `:7849` — one function of ~6000 lines
containing 480 `?` occurrences (442 lines matching `?;`/`?)`/`?.`/`?$`). A grep
cannot separate them, and the counterexamples are in the converted code:

- Converted `doc dir` arm (`main.rs:5097-5144`) contains `output_ctx.print_data(&response.directory)?`
  (`:5108`) — a `?` on a *printing* call, not the command.
- `handle_json_error!` itself contains `json_error.to_json_string()?`
  (`output_macros.rs:77`).
- The same converted arm contains a deliberate `return Err(e)` escape (`:5140`)
  for failures it refuses to relabel.
- Unconverted `doc list` (`main.rs:5022`) is `executor.list_document_references(&id)?`
  — textually indistinguishable from a converted arm's helper `?`.

**Decisive false-negative class:** the wrapper/inner split already used three
times in the tree. `run_item` / `run_item_inner` (`main.rs:1285` / `:1314`),
`run_invariant` / `run_invariant_inner` (`:1368` / `:1388`), and `run_project` /
`run_project_inner` (`:1434` / `:1465`) put the classification in the wrapper and
leave the inner full of bare `?`. A dispatch-arm scan sees only `run_item(...)?`
and cannot tell classified from unclassified.

A syntax-aware version would need `syn`. **`syn` is not a dependency of
`crates/jit`** (verified against `crates/jit/Cargo.toml`), so this is new
machinery, not reuse.

- **Fails closed?** Only against the pattern it recognises, and only at a
  false-positive rate that forces a suppression whitelist — which is itself the
  hand-maintained mirror `@/inv/single-source-prose` rejects.

### Option 3 — type-system / API-shape change

What would have to change:

- `run() -> Result<()>` (`main.rs:1870`) currently uses `anyhow::Result` with
  `Context` imported crate-wide (`main.rs:19`).
- `error_to_exit_code` (`main.rs:47-332`) is a ~285-line downcast cascade over
  roughly 30 typed error types, ending `ExitCode::GenericError` (`:332`).
- Introducing a classified error type (`{ code: ErrorCode, source: anyhow::Error }`)
  **must not** carry a blanket `From<anyhow::Error>` — that would re-open the hole
  by auto-classifying every `?` to a default. Without the blanket impl, **every
  one of the 480 `?` sites in `main.rs` needs an explicit classification call.**

Honest assessment: this is the only option where the **compiler** enforces REQ-06
with no separate check, and it genuinely collapses REQ-02/03/04/06 into one
mechanism — the top-level printer would render the envelope by construction and
derive the exit code from the same value. It is also a rewrite of `main.rs`'s
error plumbing: 8182 lines in the file, ~6000 in `run()`, 480 `?` sites, 36
existing direct-exit sites (18 `handle_json_error!` + 18 `invalid_argument`) to
fold in. That is not a sweep, and it is very hard to wave-parallelise — one
worktree, one writer, on the single largest file in the crate.

- **Fails closed?** Yes, maximally: an unclassified error would not compile.

### Option 3b — generalise the top-level printer (not in the brief; it is real)

`main()` already contains three quarters of this. `emit_startup_json_error`
(`main.rs:1766-1791`) reads `--json` from argv and renders an envelope for three
error classes. Broadening it to render for **every** error — with the code drawn
from a new `error_to_error_code` inverse of the existing `error_to_exit_code`
cascade — makes the envelope structurally unmissable: the top-level printer *is*
the envelope renderer, so "reaching the top-level plain-text printer" stops being
a failure mode.

- **Fails closed?** Yes, on envelope **presence**, and it is the only option that
  does so without enumerating anything.
- **Does not fail closed on classification quality.** An error no downcast arm
  matches yields the generic code — mirroring `ErrorCode::to_exit_code`'s
  `_ => ExitCode::GenericError` fallback (`crates/jit/src/output.rs:702`). If
  REQ-02's "carrying a classified error code" is read strictly, this needs
  option 1 as a companion: 3b guarantees the envelope, 1 guarantees the code.
- **Cost:** one generalised function plus an `error_to_error_code` cascade
  paralleling `error_to_exit_code`. Two cascades that must stay consistent is the
  maintenance liability; a shared table returning `(ErrorCode, ExitCode)` would
  avoid it but touches the existing classifier.
- **Interaction with REQ-03:** the existing stderr line at `main.rs:1728` would
  print alongside. Under the stdout/stderr reading above that is compliant and
  already test-pinned (`exit_code_tests.rs:1500-1509`).

### Option 4 — how a new check gets wired

Three patterns exist in the repo, in descending order of precedent strength:

1. **A `run_step` inside `scripts/cargo-ci.sh` calling a new `scripts/<name>.sh`.**
   Precedent: `run_step budget … rust-build-budget.sh` and
   `run_step incremental-state check_no_incremental_state`, both near the end of
   `scripts/cargo-ci.sh`. Needs no `.jit/gates.toml` change; every issue already
   carrying `cargo-ci` inherits it. `summarize_pass` (`cargo-ci.sh`) has a
   per-step case arm for folding a one-line result into the persisted gate
   summary.
2. **A Rust test in an existing suite, mirroring a policy.** Precedent: the whole
   `crates/jit/tests/scratch_build/` suite mirrors `rust-build-budget.sh`'s
   policies (`build_profile_policy_tests.rs`, `dependency_feature_policy_tests.rs`,
   `rust_build_budget_checker_tests.rs`), and the shell script's comments cite
   them by name.
3. **A new gate key in `.jit/gates.toml`.** 19 gates are declared there. **No
   repo-structural check is its own gate** — they all live inside `cargo-ci` or
   `docs-mechanical`. A new key only protects issues that carry it, so this is the
   fail-**open** wiring: forget the key on a future issue and the guard is absent.

**Established pattern for a repo-structural check: 1 or 2, not 3.** For REQ-06
specifically, (2) is stronger than (1): `cargo test --workspace` and
`cargo clippy --workspace --all-targets` are both already in `cargo-ci`, so a Rust
test needs no new step at all and cannot be skipped by a gate-list omission.

### B — summary table

| Option | Fails closed for an unregistered new arm? | Main weakness | Rough cost |
|---|---|---|---|
| 1 runtime exhaustive test | Yes, for arms with a declared `json` flag | Misses the 13 verb-hint stubs; needs a per-arm forcing recipe (mitigated by a set-equality assert) | 1 new module in `cli_issue`; 64-99 spawns |
| 2 static source scan | Only against recognised patterns | Wrapper/inner split defeats it; needs `syn` (not a dependency) or a whitelist | new script + new dependency |
| 3 type-system | Yes, at compile time | Rewrite of `main.rs` error plumbing: 480 `?` sites, ~6000-line `run()` | very large, poorly parallelisable |
| 3b generalised top-level printer | Yes, on envelope presence | Does not enforce classification quality; second classifier cascade to keep consistent | one function + one cascade |
| 4 wiring | n/a — how, not what | A new gates.toml key is the fail-open choice | n/a |

Options 1 and 3b are complementary rather than alternatives: 3b makes the
envelope unmissable, 1 makes the code correct per arm.

---

## Question C — the canonical documentation home (REQ-07)

### C.1 Where the machine-readable OUTPUT contract lives today

**Canonical home: `docs/reference/cli-commands.md`, section `## CLI JSON contracts`
(`:5-117`).** It states, in one place:

- Success responses are the payload itself, not a `{success, data}` wrapper (`:7-10`)
- The error envelope shape, with a worked `ISSUE_NOT_FOUND` example carrying
  `code` / `message` / `details` / `suggestions` (`:25-41`)
- Blocked-transition envelopes with `error.details.blockers` and `remediation`
  (`:43-112`), applied to `issue update`, `issue claim`, `issue claim-next`
- Wrong-verb hints under `--json` are `INVALID_ARGUMENT`, exit 2 (`:185-186`)

**Sites that reference rather than duplicate it** (all `@/charter/D-13`-clean):

- `docs/concepts/design-philosophy.md:78` — one line, "JSON errors include error
  codes and context", no shape
- `docs/reference/profiles.md:102`, `docs/reference/worktree-validate.md:254` —
  cite specific codes in their own context
- `docs/reference/storage-records.md:31`, `docs/reference/configuration.md:628`,
  `docs/reference/claim.md` (7 sites), `docs/reference/worktree-validate.md`
  (4 sites), `docs/reference/cli-commands.md` (11 sites) — all link to
  `exit-codes.md`

**Gap found, adjacent to but outside REQ-07's scope:** the `{"count", "<collection>"}`
list envelope is **not** stated as a convention in `cli-commands.md`. It appears
only in per-command examples (`:974`, `:1027`, `:1075`, `:1111`, …) and is stated
for one family in `docs/reference/labels.md:363,610`. Its only complete
enumeration anywhere is the module doc of a **test file**,
`crates/jit/tests/cli_issue/list_envelope_tests.rs:1-9`. That is a pre-existing
`@/charter/D-13` gap on the success side; REQ-07's subject is the failure
contract, so this is reported, not proposed as scope.

### C.2 Exit codes: documented, generated, and runtime-bound

`docs/reference/exit-codes.md` is a **full-file projection**, not hand-maintained:

- Rendered by `jit::schema::render_exit_code_reference()`
  (`crates/jit/src/schema.rs:1068-1126`); the committed file carries the generated
  banner at `docs/reference/exit-codes.md:3`.
- Freshness enforced by `test_exit_code_reference_doc_is_current`
  (`crates/jit/src/schema.rs:1585-1602`) — byte equality against the committed
  file, regenerated with `UPDATE_EXIT_CODE_DOC=1`.
- Data sources: `CommandSchema::generate_exit_codes()` (`schema.rs:794-836`) and
  `generate_command_exit_codes()` (`schema.rs:847-1059`).
- Every per-command row is bound to runtime by test
  (`command_exit_code_projection_tests.rs` + `main.rs`'s
  `exit_code_projection_tests`), and completeness is enforced by set equality at
  `command_exit_code_projection_tests.rs:759-812`.
- A second guard checks every projected code is a member of the global taxonomy
  (`schema.rs:1556-1580`).

**Caveat the plan should know:** `generate_exit_codes()` (`schema.rs:794-836`) is
nine hand-written `ExitCodeDoc` literals, **not** derived from the `ExitCode` enum.
A tenth enum variant would not appear there and nothing would fail.

### C.3 Error codes: not documented, and not currently enumerable

- **`ErrorCode` is a unit struct, not an enum.** `crates/jit/src/output.rs:640`
  declares `pub struct ErrorCode;`, carrying **20 associated
  `const &'static str` items** at `output.rs:644-676`: `ISSUE_NOT_FOUND`,
  `GATE_NOT_FOUND`, `CYCLE_DETECTED`, `INVALID_ARGUMENT`, `VALIDATION_FAILED`,
  `ALREADY_EXISTS`, `INVALID_STATE`, `BLOCKED`, `GATE_FAILED`, `IO_ERROR`,
  `PARSE_ERROR`, `CLAIM_REQUIRES_GIT`, `AMBIGUOUS_ID`, `INVALID_ID_PREFIX`,
  `REPOSITORY_NOT_FOUND`, `REPOSITORY_FORMAT_TOO_NEW`, `STALE_BINARY`,
  `DELETION_NOT_CONFIRMED`, `PROFILE_NOT_FOUND`, `PROFILE_CONFLICT`.
- **Not exhaustively enumerable at compile time.** No derive, no `strum`, no
  `ALL` array, no iterator. A twenty-first const is invisible to any reflection.
- **The forward map fails open.** `ErrorCode::to_exit_code` (`output.rs:681-704`)
  ends `_ => ExitCode::GenericError` (`:702`), so a new const silently classifies
  as exit 1.
- **No adopter documentation enumerates the vocabulary.** Individual codes appear
  in examples (`cli-commands.md:30,52,85`, `profiles.md:102`,
  `worktree-validate.md:254`). There is no table.

By contrast **`ExitCode` *is* an enum** (`output.rs:539-572`), 9 variants,
`#[repr(i32)]`. `ExitCode::description` (`:582-598`) is an exhaustive `match` with
no wildcard, so a new variant is a compile error there — but
`ExitCode::all_codes_documentation()` (`:601-632`) hand-lists all nine in a
`format!`, and a new variant would **not** break it. That function is a live
`@/inv/single-source-prose` hazard independent of this epic; worth naming in the
plan even if out of scope.

### C.4 Is there projection machinery that fits an error-code vocabulary?

**`.jit/config.toml`'s `[projection.*]` does not fit.** The three declared
projections (`:255` invariants → `AGENTS.md`; `:264` rules-and-gates →
`docs/reference/rules-and-gates.md`; `:273` charter → `AGENTS.md`) each name a
`kind` from `[item_kinds]` with a markdown or TOML source of truth
(`@/charter/D-6`), a `mode = "region"`, a `target`, and a `style`. An error-code
vocabulary is a set of Rust constants, not a declared item kind. Forcing it into
`[projection.*]` would require inventing a kind whose TOML registry duplicates the
Rust consts — a second source of truth, which is the defect rather than the fix.

**The fitting machinery is the other, larger family already in the tree:** a
`render_*() -> String` function beside the definitions, a committed generated
page, and an in-crate freshness test.

| Generated page | Renderer | Path const |
|---|---|---|
| `docs/reference/exit-codes.md` | `schema.rs:1068` `render_exit_code_reference` | (inline, `schema.rs:1590`) |
| `docs/reference/events.md` | `domain/event_catalog.rs:494` `render_event_reference` | `event_catalog.rs:36` |
| `docs/reference/storage-records.md` | `storage/reference.rs:392` `render_reference_markdown` | `storage/reference.rs:44` |
| `docs/reference/runtime-defaults.md` | `runtime_defaults.rs:46` `render_reference_markdown` | `runtime_defaults.rs:38` |
| `docs/reference/gate-presets.md` | `gate_presets/reference.rs:214` `render_reference_markdown` | `gate_presets/reference.rs:23` |

Each has a stale-file test (`runtime_defaults.rs:116`, `gate_presets/reference.rs:334`,
`event_catalog.rs:776`, `storage/reference.rs:776`, `schema.rs:1599`).

**`events.md` is the exact template for a non-circular error-code projection.**
`EventTag::ALL` (`domain/event_catalog.rs:122-144`) is a `const [EventTag; 21]`,
and its doc comment (`:118-121`) records that "a conformance test compares this
list against the variants schemars derives from the enum, so a tag left out of it
fails the suite" — the freshness guard is bound to the type's own derive, not to a
hand-written mirror. `event_catalog()` (`:477-487`) maps `ALL` through
`tag.scope()` / `tag.description()`; `render_event_reference()` (`:494`) projects
that into markdown.

**Therefore REQ-07 has a shaped, precedented path, but it has a prerequisite:**
`ErrorCode` must become enumerable first (an enum with `as_str`, a derive-checked
`ALL`, and — to close the fail-open at `output.rs:702` — an exhaustive
code → exit-code match instead of a wildcard). Without that, a "generated" table
is the same hand-maintained list relocated into Rust, which
`@/inv/single-source-prose` treats as the defect. This conversion is not in the
epic's non-goals: the non-goal is "revising the error-code vocabulary or exit-code
assignment", and turning a const set into an enum with the same members and the
same mappings revises neither. But it is real work the plan must place somewhere.

**Note on coverage overlap:** `scripts/docs-check-projections.sh` covers **only**
`[projection.*]` targets — it reads `jit project render --json`'s
`.projections[].target` and `git diff`s those paths. A generated error-code page
built the `exit-codes.md` way is therefore covered by **`cargo-ci`**, not by
`docs-mechanical`.

### C.5 `docs-mechanical` footprint for a new or edited page

Gate: `./scripts/docs-mechanical.sh` with `DOCS_FOOTPRINT = "docs/"`
(`.jit/gates.toml`, `docs-mechanical` block). The orchestrator fans out to three
checkers and aggregates status, with exit 2 (environment error) dominating exit 1
(finding) (`scripts/docs-mechanical.sh:88-101`).

- **M2 `docs-check-links.sh`** — every intra-repo markdown link and
  intra-document heading anchor must resolve, in inline and reference forms
  (`:12-24`). GitHub anchor slugification is reproduced but **without** the
  numeric `-1`/`-2` disambiguation for duplicate headings (`:26-33`), so
  duplicated headings on a new page produce human-adjudicated findings. Practical
  consequence: a new reference page needs its `docs/index.md` entry (the reference
  list runs `docs/index.md:44-60`) **and** every anchor other pages link into it
  must exist.
- **M3 `docs-check-citations.sh`** — (1) backtick-span path tokens whose first
  segment is a tracked top-level repo entry must exist on disk, extension or not
  (`:14-21`); (2) `@/<kind>/<self-id>` citations must resolve through
  `jit item show` for a live registered kind (`:22-27`). Requires `jit` and `jq`
  on PATH (`:65-72`) — a stale or absent install makes this exit 2.
- **M5 `docs-check-projections.sh`** — re-runs `jit project render` **in place**
  and `git diff`s the configured targets; note the documented side effect
  (`:18-21`): a stale tree is left holding the freshly rendered region.

### C.6 Memory-flagged pitfall — verified, with a correction

**The underlying claim holds.** Measured on `main` at this checkout:

| Directory | files present | files tracked |
|---|---|---|
| `dev/plans` | 0 | 0 |
| `dev/sessions` | 0 | 0 |
| `dev/design` | 0 | 0 |
| `dev/experiments` | 0 | 0 |
| `dev/active` | 47 entries | 58 |

The four are empty **and** untracked, so they exist on this working copy and would
be **absent** in a fresh `git worktree`. A backtick citation `dev/plans/…` has a
tracked first segment (`dev`) and would be reported MISSING in a worktree while
passing on main.

**Correction to the operational implication: it does not currently bite the
`docs/` footprint.** `docs/` contains exactly four `dev/` references, none of them
to the four empty directories:

- `docs/index.md:100` — a markdown link `[dev/index.md](../dev/index.md)`, not a
  backtick path token; `dev/index.md` is tracked
- `docs/how-to/multi-agent-coordination.md:472` and
  `docs/tutorials/parallel-work-worktrees.md:257` —
  `dev/archive/ad601a15-parallel-work/dev/design/worktree-parallel-work.md`,
  tracked under `dev/archive/`
- `docs/how-to/adopt-planning-bracket.md:109` — `dev/active`, tracked

**The live risk for this epic is different and sharper:**
`dev/active/a2546471-json-error-contract/` does not exist in git — verified absent
from `dev/active` before this file was written. Any adopter page under `docs/`
citing a path inside it fails M3 in a worktree, and on main until the file is both
created and present. Rule for the plan: adopter documentation cites the shipped
surface (`docs/`, `crates/`), never the planning artifact.

---

## Load-bearing claims I could not verify

- **Forced-failure reachability per arm.** I established that several arms have no
  argument-shaped rejection (`main.rs:5148-5155` says so for `doc conformance`),
  but I did not attempt to force a failure in all 64/99 arms. Whether every arm
  has *some* reachable forced failure is unresolved, and it is the single fact
  option 1's feasibility rests on. If the plan takes option 1, this needs an
  explicit spike before the manifest fixes the coverage requirement.
- **Wall-clock cost of a 64-99-spawn suite.** Not measured. `cargo-ci` runs with
  a 900 s timeout (`.jit/gates.toml`) and the suite is already parallel at
  `RUST_TEST_THREADS=20` (`scripts/cargo-ci.sh`), so headroom is likely, but I did
  not benchmark it.
- **The 64-vs-99 reconciliation against the epic's census.** The epic body states
  "roughly 63 command arms"; my visible-leaf count is 64 and my all-leaf count is
  99. I report both with the method; reconciling them against REQ-01's census is
  `investigator-census`'s call, not mine.
