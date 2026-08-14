# Investigation: gf2 adopter reliability (worktree safety, gate durability, profile guidance)

Container: `4a559332` — "gf2 adopter reliability: safe worktrees, durable gates, actionable profiles"

## Scope and method

Read-only source investigation. No file was edited, no `.jit/` state was mutated, and no
mutating `jit` command was run. Verification was split across four bounded, independent
read-only passes (worktree/storage, gates, profiles, testing topology), each re-deriving
file:line citations from the current tree rather than trusting the input hypotheses, plus
direct spot-checks performed in this session (CLI invocations of `jit profile show`, `jit
query divergence`; direct reads of cited source lines; a live inspection of this
repository's own `.agents/worktrees/` fleet and `.jit/config.toml`). Every claim below is
CORRECTED where the original hypothesis was imprecise, and the corrected file:line is what
is cited.

Every numbered claim from the input brief is addressed; none is dropped silently.

## 1. Claim classification

### Worktree / storage claims

**1 — CORRECTED.** `.jit` discovery walks ancestors and stops after the first ancestor whose
`.git` path exists (file or directory, undifferentiated) — but a candidate directory only
*qualifies* as a repository root when it also contains `.jit/index.json`, not merely `.jit`
(`crates/jit/src/storage/discovery.rs:43-53`). `JIT_DATA_DIR` overrides the selected data
root, but `jit` still runs discovery to derive a worktree root even with the override set
(`crates/jit/src/main.rs:2219-2262`, specifically `:2231-2238`). Without the override, `jit
init` targets only the current directory and never searches upward (`main.rs:2239-2240`).

**2 — VERIFIED, with scope clarification.** Semantic repository mutations flow through
`RepositoryMutationSession::apply`, opened via `RepositoryStateStore::open_mutation_session`
(`crates/jit/src/storage/repository_state_store.rs:186-208`, `:315-345`, `:486-567`).
`IssueStore`'s doc comment states it has no publication API
(`crates/jit/src/storage/mod.rs:96-101`). The two claimed bypasses are real, but both are
non-semantic, machine-local/runtime state, not issue/event data:

- `crates/jit/src/storage/worktree_identity.rs:157`, `:169-185`, `:208-212` writes/removes
  `.jit/worktree.json` directly.
- `crates/jit/src/commands/serve.rs:84-92` writes `.jit/server.pid.json` directly.

`docs/reference/storage-format.md:44-55` explicitly classifies worktree identity, server
files, locks, and temp state as machine-local, outside the semantic store.

**3 — CORRECTED.** `main.rs` computes `WorktreePaths` on every normal dispatch and uses
`worktree_paths.worktree_root` to build the canonical repository layout
(`crates/jit/src/main.rs:2265-2279`, esp. `:2273-2279`); `worktree_paths.rs:50-121` performs
the underlying `git rev-parse` calls and `:125-136` defines `is_worktree()`. That boolean is
**not** used at this general dispatch point to gate ordinary writes — but it is not entirely
unused either: `issue delete` already has an independent, unconditional secondary-worktree
guard (`main.rs:3546-3550`, see "Architecture fit" below). So the correction to the original
claim is that "currently unused for write policy" is true only for the *general* case; one
specific command already enforces a primary-worktree-only write policy today.

**4 — CORRECTED.** At least **five** distinct worktree-detection/main-fallback mechanisms
exist, not three:

- `WorktreePaths`, via `git` subprocess calls (`crates/jit/src/storage/worktree_paths.rs:50-121`).
- `JsonFileStorage::is_secondary_worktree`, via `.git.is_file()` (`crates/jit/src/storage/json.rs:814-831`).
- `load_index_from_main_worktree`, via `git2::Repository::path()` vs. `commondir()` (`json.rs:667-702`).
- `load_issue_from_main_worktree`, via inline `git rev-parse` shell-outs (`json.rs:754-811`).
- A repository-state linked-worktree fallback using `GitRevisionResolver` and
  `discover_main_worktree` (`crates/jit/src/storage/repository_state_store.rs:1394-1457`).

This is a real convention-convergence concern: five independent implementations of
"which directory is the main worktree" is more surface than one abstraction should have.

**5 — VERIFIED, and confirmed deliberate, not accidental.** Issue reads try local storage,
then Git `HEAD`, then the main worktree, in that order (`crates/jit/src/storage/json.rs:547-598`,
`:616-702`, `:705-811`, `:1053-1090`). Aggregated index membership unions all three sources,
first-source-wins on duplicate IDs (`json.rs:550-588`). This is read-only backfill:
`read_events()` reads only the locally-selected `events.jsonl`, with no fallback
(`json.rs:1216-1227`). A local issue/index copy shadows the lower-priority Git/main copies.

This design is **confirmed intentional**, not an accident of implementation, by four
independent lines of evidence: (a) the original design document specifies exactly this
layered resolution order and states "worktrees contain write copies of claimed issues, but
all issues remain readable from any worktree via git"
(`dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md:451-472`); (b) the
current adopter-facing tutorial documents the same 3-tier fallback verbatim
(`docs/tutorials/parallel-work-worktrees.md:81-95`, mermaid diagram at `:190-201`); (c) the
current adopter-facing how-to guide repeats it with its own diagram
(`docs/how-to/multi-agent-coordination.md:177-183`); (d) `docs/concepts/guarantees.md:368-370`
and `docs/reference/storage-format.md:44-55` both state `.jit` is per-worktree data while
`.git/jit` is shared. Any redesign that removes or narrows this fallback is a documented,
adopter-facing behavior change, not an internal cleanup.

**6 — VERIFIED.** `WorktreePaths` resolves Git's common directory and maps it to
`<common_dir>/jit` (`crates/jit/src/storage/worktree_paths.rs:71-121`);
`control_plane.rs:7-24` creates `git_dir/jit/{locks,events}` there. For linked worktrees,
Git's common directory is the primary repository's `.git`, so the control plane is
physically the same directory for every worktree.

**7 — VERIFIED, precisely as "no reconcile tooling for divergent stores."** No JIT command
merges, imports, or reconciles two divergent `.jit` stores.
`crates/jit/src/commands/snapshot.rs:431-435` explicitly documents snapshot import/diff as
future work. The currently-documented recovery path for divergent `.jit` state is ordinary
Git merge plus `jit validate --fix`
(`docs/how-to/multi-agent-coordination.md:160-173`). Separately verified in this session:
`jit query divergence` and `jit validate --branch-drift` are **not** this tooling — `query
divergence` reports membership-label-vs-DAG disagreement (an unrelated concept that happens
to share the word "divergence"), and `--branch-drift` checks `origin/main` ancestry, not
cross-worktree `.jit` state. `docs/reference/worktree-validate.md:279-281` documents a hidden
`--divergence` stub on `jit validate` that errors and redirects to `--branch-drift` or `query
divergence` — confirming, from the CLI's own error routing, that no store-divergence detector
exists under any name today.

### REQ-01 / REQ-02 assessment

Both are **valid-and-open**, but the framing needs one important correction surfaced by the
prior-art sweep (§2): a blanket "refuse all mutation from a linked worktree" policy is not a
free-standing addition — it must be reconciled with an existing, heavily-used, sanctioned
workflow that depends on linked-worktree mutation succeeding. See §2 and §5.

### Gates claims

**8 — CORRECTED, and this resolves the REQ-03 question.** The current automated `gate
evaluate` flow is: `main.rs:4870-4924` dispatches to `executor.pass_gate`, printing success
only in the `Ok(outcome)` arm at `main.rs:4881`; `commands/gate.rs:878-906` routes automated
gates to `check_gate_with`; `commands/gate_check.rs:1016-1034` builds `UpdateIssue`,
`RecordGateRun`, and `RecordEvent` intents into one plan and calls `session.apply` before
returning success; `RecordGateRun` becomes a `gate-runs/<id>/result.json` write
(`crates/jit/src/repository_state/mutation.rs:1002-1050`); the file-backed store invokes the
transaction kernel and returns only after it succeeds
(`crates/jit/src/storage/repository_state_store.rs:502-565`); the kernel writes the terminal
`Committed` journal before cleanup and `Ok`
(`crates/jit/src/storage/file_transaction.rs:1004-1018`); recovery is dispatched on the next
mutation-session open (`repository_state_store.rs:404-425`,
`file_transaction.rs:1824-1865`).

**Explicit answer: yes — for automated `gate evaluate`, current main already satisfies
REQ-03's durability guarantee end to end.** A persistence error propagates through
`session.apply` → `check_gate_with` → `pass_gate` → the CLI error arm; success prints only
after the coupled issue update, gate-run record, and event have all passed the recovered
2-phase-commit kernel. The claim needed correction because `pass_gate` also has a *manual*-gate
branch (`commands/gate.rs:909-935`) that records an attestation through a different mutation
path, not yet independently traced here. **REQ-03's open work is exactly what the epic text
says: a regression test reproducing the reported failure class — the underlying durability
primitive already exists and does not need new production code for the automated path.**

**9 — VERIFIED, with an important scope caveat.** `run_postchecks` discards checker errors
with `let _ = self.check_gate(...)` (`crates/jit/src/commands/gate_check.rs:1606-1626`,
exact swallow at `:1618`), called after a transition to `Gated`
(`crates/jit/src/commands/issue.rs:628-643`). No warning or error is surfaced for the
swallowed failure; `auto_transition_to_done` subsequently observes the still-unpassed gate
and correctly leaves the issue in `Gated`
(`crates/jit/src/commands/issue.rs:769-781`) — so the *state machine* stays consistent
(gate-semantics invariant holds), but **the command itself returns ordinary success and looks
identical to a clean postcheck run**, even though the automated gate's persistence attempt
failed silently and left no recorded evidence. The doc comment above the swallow claims
"errors are logged" but the visible code has no logging call at that site — worth confirming
whether `check_gate` logs internally before this change assumes the comment is accurate.

Scope caveat found in verification: the primary CLI path most agents use,
`issue update --state ...`, calls `update_issue` — a **different** function from
`update_issue_state` (`main.rs:3413-3444` vs. the function containing this swallow). The
current direct CLI caller of `update_issue_state` is `issue reject`
(`main.rs:3719-3728`), which transitions to `Rejected`, not `Gated`. This means the exact
reachability of this swallow from today's primary CLI surface needs to be nailed down before
scoping a fix — it is real code with a real gap, but confirm which caller(s) actually drive a
transition through `run_postchecks` before treating it as the dominant cause of gf2's report.

**10 — VERIFIED, gap confirmed.** `TransactionFailureInjector` and constructors exist
(`crates/jit/src/storage/transaction_recovery.rs:15-89`; constructors at
`storage/json.rs:321-350` and `storage/memory.rs:138-145`). The only gate-specific
injected-failure tests are `test_gate_definition_apply_failure_recovers_registry_and_event_together`
and `test_gate_preset_apply_failure_recovers_registry_issue_and_events_together`
(`crates/jit/src/commands/gate.rs:1884-1919`, `:1921-1955`), both injecting failure while
defining/presetting a gate — **not** while running one. No
`TransactionFailureInjector`/`with_repository_state_failures` usage exists anywhere in
`gate_check.rs`. Three generic tests exercise `RecordGateRun` materialization/loading
directly (`repository_state_store_tests.rs:4389-4523`, `:4594-4665`;
`storage/gate_runs.rs:25-90`) but none of them drive failure through the `gate
evaluate` command path. **Confirmed: no existing test injects a persistence failure through
`pass_gate`/`check_gate_with`.** This is exactly REQ-03's stated remaining gap.

### REQ-03 assessment

**Already-done for the durability property on the automated path (claim 8); open only for
regression-test coverage (claim 10) and for characterizing/fixing the separate
`run_postchecks` swallow (claim 9), which is a different code path (issue-state-transition
postchecks) from `gate evaluate` proper.** Treat these as two related but distinct pieces of
work: (a) add a failure-injection regression test on `pass_gate`/`check_gate_with` proving the
existing guarantee, satisfying REQ-03's "regression test reproduces the class" wording; (b)
decide whether the `run_postchecks` swallow needs a warning/error surfaced, and confirm its
actual current CLI reachability first.

### Profile claims

**11 — CORRECTED.** `ProfileDivergence` is a typed enum, `Serialize`/`JsonSchema`-derived,
serde-tagged (`crates/jit/src/profile/agreement.rs:24-28`); `.message()` at `:73-105` is only
its human-rendered prose. `ProfileTargetConflict` is typed but **not** serializable
(`crates/jit/src/repository_state/profile_apply.rs:628-662`); its restore-vs-capture
distinction is encoded entirely by which prose constant `.message()` selects (`:664-727`,
constants `SET_ASIDE_OR_CAPTURE`/`RESTORE_OR_CAPTURE`/`ALIGN_OR_SELECT_ONE` at
`:712/:717/:726`) — there is no typed remedy field a machine consumer can match on.

**12 — VERIFIED.** `JsonError` has `with_details`, `with_suggestion`, `with_suggestions`
(`crates/jit/src/output.rs:498-514`). Mutation-conflict error branches construct only
`JsonError::new(..., error.to_string())`, discarding the typed enum before it reaches JSON
(`crates/jit/src/main.rs:1174-1181`, `:1201-1210`). Two exceptions already exist and are
useful precedent: `profile validate` and `profile diff` already attach structured detail via
`render_validation_json` (`main.rs:1121-1135`, `:1053-1067`) — i.e., the pattern REQ-04 wants
is already proven on two of the profile subcommands and just needs extending to the
conflict/divergence paths.

**13 — CORRECTED.** `jit profile capture --source --destination` exists
(`crates/jit/src/cli.rs:3100-3134`, `commands/profile.rs:733-750`,
`profile/package_capture.rs:323-347`, contribution refresh at `:433-444`). No explicit typed
`Restore`/`Capture` remedy *value* exists as a standalone type, but `ProfileTargetConflict::message()`
is itself an exhaustive typed match that already selects the correct prose remedy per variant
(`profile_apply.rs:673-701`) — so the domain logic distinguishing restore-valid from
capture-only cases already exists as code, it is just not exposed as data. Turning that match
into a typed field is a small, well-scoped change, not new domain design.

**14 — VERIFIED.** `profile show` requires repeatable `--profile <SELECTOR>` values
(`crates/jit/src/cli.rs:2951-2960`); parsing accepts only `id:`/`path:` tags
(`crates/jit/src/commands/profile.rs:154-189`). Directly reproduced in this session: `jit
profile show somebadid` → `error: unexpected argument 'somebadid' found` with `Usage: jit
profile show [OPTIONS] --profile <SELECTOR>`, exit code 2 — Clap's generic message, with no
hint about the `id:`/`path:` tag syntax. Covered by
`crates/jit/tests/cli_repo_workflow/profile_cli_tests.rs:401-405`. Shared parsing
(`parse_profile_selectors`, `main.rs:837-846`, `:916-930`) is used by other profile
lifecycle commands, so any loosening must be scoped to `profile show`'s own argument
definition/dispatch, not the shared parser — confirmed correct in the original hypothesis.

**15 — VERIFIED**, full consumer inventory in §3.d below.

### REQ-04 / REQ-05 assessment

**Valid-and-open, well-scoped.** REQ-04 has two existing precedents to extend
(`profile validate`/`profile diff`'s structured JSON) rather than a new mechanism to invent.
REQ-05 is a small, already-isolated fix at `profile show`'s own arg parsing; the shared
selector parser must not be touched.

### Testing claims

**16 — VERIFIED.** 10 of `MAX_INTEGRATION_TARGETS=12` targets currently in use
(`scripts/rust-build-budget.sh:41-43`): `cli_gate`, `cli_issue`, `cli_item_validate`,
`cli_query_graph`, `cli_repo_workflow`, `fast_docs_templates`, `fast_issue`, `fast_rules`,
`scratch_build`, `server_integration`. `cli_repo_workflow` alone aggregates 33 test modules
via `mod` declarations in `crates/jit/tests/cli_repo_workflow/main.rs:5-37`, including
`cross_worktree_integration_tests`, `worktree_cli_tests`, `profile_cli_tests`,
`profile_edit_refresh_cli_tests`, `profile_acceptance_tests`, and `gate_preset_tests` — i.e.,
exactly the suites this epic's e2e work needs to extend already live in the one target with
the most remaining room to add modules without spending a scarce target slot.

**17 — VERIFIED, exhaustive enumeration below (§3.c).** Every existing linked-worktree test
that performs a structural mutation does so only *after* first running `jit init` inside that
linked worktree — this is a load-bearing pattern for §2 and §5.

**18 — CORRECTED.** The crashed-fixture-then-subprocess pattern
(`leave_fresh_prepared_journal`, `crates/jit/tests/cli_issue/recover_command_tests.rs:80-114`)
injects `RepositoryBeforeDataRootPublication`, calls `session.apply(&plan).unwrap_err()` to
leave a prepared-but-uncommitted journal, then launches a `jit recover`/`jit init` subprocess
and asserts recovery. **The pattern is reusable at the transaction/recovery boundary in
general, but the existing helper is initialization-specific and the CLI subprocess exposes no
failure injector for other command paths.** A gate-run durability test built on this pattern
would need to execute the gate evaluation in-process with an injected `JsonFileStorage`, then
launch a separate subprocess for recovery and assert on the gate journal/state afterward — not
a direct reuse of the existing helper.

**19 — VERIFIED.** `MAX_INTEGRATION_TARGETS=12`, `MAX_EXECUTABLE_BYTES=2,147,483,648` (2 GiB),
`MAX_TEST_SUITE_SECONDS=30` (30,000 ms), all at `scripts/rust-build-budget.sh:41-43`,
byte-budget enforcement at `:184-204`, invoked by CI at `scripts/cargo-ci.sh:568-594`. The
repository's last recorded benchmark: 1,596,001,608 executable bytes across 14 executables,
22.56–22.58s suite clock (`dev/benchmarks/rust-build-budgets/README.md:43-48`) — both
comfortably under budget today, giving real headroom for new e2e coverage, but the budget
script could not be re-run in this read-only session to reconfirm current numbers (Cargo
could not open `target/debug/.cargo-build-lock`).

### REQ-06 assessment

**Valid-and-open, with one concrete gap the plan must account for.** No backed-up gf2 fixture
currently exists anywhere in this repository as a committed or durable artifact. The prior
epic's handoff notes a session-local backup at `/tmp` scratchpad
(`dev/active/c639cfb5-jit-profiles-complete/handoff-9.md:67`, "gf2 record backups: `/tmp`
scratchpad `gf2-records-backup/` (session-local; gf2 itself is untouched)") — that directory
was session-scoped and should not be assumed to still exist. The live gf2 checkout exists at
`/home/vkaskivuo/Projects/gf2` (confirmed present on disk in this session) and is, per that
same handoff, untouched. REQ-06 execution will need to either capture a fresh backup of gf2's
current `.jit/` state as a committed fixture, or build the "equivalent linked-worktree
reproduction" the epic text offers as an alternative — the plan should decide which and not
assume a ready-made fixture is waiting.

## 2. Prior-art sweep

This is the section with the most material relevant to D-01's premises, because the
repository's own operational history already ran into this exact failure class.

**The gf2 incident is documented field evidence, not hypothetical.** The epic's own
description states it directly: "On 2026-08-13, gf2 recovered an issue and disjoint event
history from a dangling commit after mutating JIT commands ran from a linked worker
worktree." (`.jit/issues/4a559332-e795-4320-9eb9-193e248d628c.json`, `description` field).
`dev/active/c639cfb5-jit-profiles-complete/handoff-9.md:24,28,34,56,67` records related gf2
field observations from the immediately preceding epic: a shared cargo-ci build-lock defect
fixed in both repos (`4ce92063` here, `5c244cad` in gf2); gf2's profile drift already
diagnosed (three known divergences from `jit-default`) with **capture already decided as
gf2's correct recovery path**, not restore, "since the migration that would have fixed it is
being deleted rather than widened" — this is directly load-bearing prior art for REQ-04: for
at least one real case, restore is not merely less-preferred than capture, it is **invalid**,
and the plan should treat "which remedies are valid, not merely which is preferred" as the
shape REQ-04's structured guidance needs to encode.

**This repository's own sanctioned multi-agent workflow depends on linked-worktree mutation
succeeding, and any REQ-01 policy must be reconciled with it, not simply layered on top.**
Live inspection in this session: `git worktree list` shows 26+ active linked worktrees under
`.agents/worktrees/agent-*` (e.g. `agent-0708d692` on branch `worktree-agent-0708d692`), each
with its own full `.jit/` — its own `issues/` (1038 entries vs. 1064 in the primary at time of
inspection), its own `events.jsonl` (3.1 MB), its own `gate-runs/` (4167 entries), and (for
this particular one) **no** `worktree.json` of its own. This is the `jit-parallel`/
`jit-execution-lead` skill mechanism (also independently found by the worktree-area pass:
`.agents/skills/jit-parallel/references/worktree-mode.md:15-23,33-36,60-64` states sanctioned
worker worktrees live under `.agents/worktrees`, share `.git/jit`, and carry `.jit` per
worktree; `.agents/skills/jit-execution-lead/references/worktree-dispatch-protocol.md:32-47`
states the sanctioned workflow creates SHA-anchored linked worktrees there and dispatches
agents into them).

**There is a real, documented tension between two generations of the sanctioned protocol on
exactly the signal a refuse-by-default policy would most naturally key on.** The *original*
design (`dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md`, 2026-01-03/04)
and every current worktree test (§3.c, claim 17) have agents run `jit init` explicitly inside
a new linked worktree before mutating anything there — `jit init` is the closest thing that
exists today to an "explicit, auditable opt-in" signal. But
`dev/studies/multi-agent-parallelism-analysis.md:248-251,335-337` (found by the worktree-area
pass) records that the *newer* dispatch protocol instructs agents **not** to run `jit init`
when `.jit` is already Git-tracked (because a normal `git worktree add` already checks out
whatever `.jit/` content is tracked at that commit). If REQ-01's opt-in is implemented as "was
`jit init` explicitly run in this worktree," it is compatible with every existing test and
with the *original* protocol, but is exactly the signal the *newer* protocol tells agents to
skip. This is a genuine design fork the plan needs to resolve deliberately, not discover
during implementation.

**The repository already lived through, and partly fixed, this exact class of problem once
before, via a different mechanism than a worktree write-refusal.**
`dev/archive/4a00b2b0-agent-validation/studies/worktree-merge-analysis.md` (2026-02-17) is a
first-hand friction report from an early two-agent worktree session: "Issue state changes
(.jit/) are conflict magnets" — both agents wrote the same issue JSON, one claiming it, one
completing it; "Forgotten `.jit/` commits create a second merge round" — issue state updates
made via CLI in the agent worktree were not committed alongside the code commit that
completed them; "Worktree isolation doesn't cover `.jit/` writes." Its recommendations
included moving coordination state into `.git/jit/` (the shared control plane) — **this
recommendation was implemented**: `control_plane.rs:7-24` places locks/events under
`.git/jit`, matching claim 6, and `.gitattributes` in this repository sets
`merge=union` for `.jit/events.jsonl` and `.jit/claims.jsonl` specifically, matching this
document's other recommendation. What was **not** implemented is the "custom merge driver for
issue JSON" idea or any guard on `git worktree remove` — the second failure mode
("forgotten `.jit/` commits") is mechanically identical to gf2's incident (a linked worktree's
uncommitted-and-unmerged `.jit/` state being lost), and it was already observed once, in this
repository, before this epic.

**The adopter-facing multi-agent-coordination guide's own "clean up an orphaned worktree"
recipe has no safeguard against exactly this loss.** `docs/how-to/multi-agent-coordination.md:303-312`,
under "Recovery Scenarios → Orphaned Worktree," instructs: `git worktree remove
../old-worktree`, followed only by a note about *lease* expiry — it says nothing about
checking whether that worktree holds committed-nowhere `.jit/` issue or event state before
removal. This is the single most plausible mechanical explanation for how gf2's incident
happened by following documented, sanctioned guidance, and it is a concrete, low-risk fix
target regardless of which broader worktree-authority design D-01 lands on: `jit worktree
remove`-adjacent tooling, or the doc's own recipe, should check for unmerged local `.jit/`
history before a worktree is discarded.

**Lease enforcement — a mechanism that could plausibly have prevented part of gf2's
incident — is off by default, and is off in this repository's own dogfood config.**
`docs/reference/configuration.md:460-478`: with no `[worktree]` section in `.jit/config.toml`,
`enforce_leases` resolves to `"off"`; only a *present* section with an *omitted* key resolves
to `"strict"` (matching the code default at `crates/jit/src/config.rs:1308-1313`,
`unwrap_or(EnforcementMode::Strict)`, which only fires past that first branch). Directly
verified in this session: this repository's own `.jit/config.toml` has **no** `[worktree]`
section, so its own dozens of live agent worktrees currently run with lease enforcement off.
`require_active_lease` (`crates/jit/src/commands/mod.rs:3194-3238`) is a "legacy preflight"
per its own doc comment, which states authoritative enforcement is "rechecked from captured
config by mutation coordinators that enforce leases during publication" — i.e., there may be
a second, newer enforcement point inside the captured-mutation pipeline this investigation did
not fully trace; the plan should not assume the six `require_active_lease` call sites
(`commands/issue.rs:657,698`; `commands/gate.rs:746,790,836,1053`) are the only place lease
state is consulted. Regardless, leases are a *concurrency* control (one writer per issue at a
time), not a *durability* control (nothing about a lease prevents a worktree's committed-but-
unmerged history from being discarded on cleanup) — so even fully-strict lease enforcement
would not by itself have prevented gf2's incident.

**Direct architecture precedent for REQ-01 already exists in the codebase, narrowly scoped to
one command.** `crates/jit/src/main.rs:3546-3550`, `issue delete`: `if
storage.is_secondary_worktree() { anyhow::bail!("Deletion is not allowed in secondary
worktrees. Deletions must be performed from the main worktree to maintain consistency across
all worktrees."); }`. Comment: "Phase 3 safety check." `git log` confirms this landed in
commit `d1ab0a44b`, "jit:5dbc3548 Phase 3: Block deletion in secondary worktrees" — a
previously-completed jit issue. This guard is unconditional (no opt-in of any kind, not even
an environment variable) and covers only `issue delete`. It is the closest existing precedent
for where and how a REQ-01 guard should be implemented — see §5.

**No prior design note prohibits restore-vs-capture guidance in JSON errors (REQ-04); the
prior profile-lifecycle epic's own documents anticipate exactly this kind of follow-on
work.** `dev/archive/c639cfb5-jit-profiles-complete/active/plan.md:20-35,48-53,94-105,217-231`
defines the delivered typed three-way conflict/decision model and requires coordinated
updates across schema/MCP/tests/docs for any structural change — i.e., the plan for REQ-04
should expect to touch the generated schema, the MCP bridge, and the profile test suite
together, not just `output.rs`, matching what that epic's own retrospective states about its
scope discipline.

## 3. Consumer sweeps

### 3.a Worktree detection / main-worktree fallback read sites

- `crates/jit/src/storage/worktree_paths.rs:50-136` — canonical Git/non-Git detection, `is_worktree()`.
- `crates/jit/src/main.rs:2273-2279` — startup layout construction on every dispatch.
- `crates/jit/src/storage/worktree_identity.rs:123-125` — secondary-status check before handling copied identity files.
- `crates/jit/src/storage/json.rs:814-831` — independent `.git`-is-a-file test used by deletion policy (`is_secondary_worktree`).
- `crates/jit/src/commands/claim.rs:93-104,243-254,360-371,453-464,510-521,567-578,617-635,685-696,743-754` — claim acquire/heartbeat/release/renew/status/list/force-evict/recover, each independently resolving worktree context.
- `crates/jit/src/commands/mod.rs:574-600,3088-3128,3142-3155` — mutation claim coordination, identity initialization, lease checks.
- `crates/jit/src/commands/template.rs:284-299` — template lease-warning detection.
- `crates/jit/src/commands/validate.rs:2645-2654,2897-2910` — claim-index validation.
- `crates/jit/src/commands/worktree.rs:167-193,212-218,238-300` — `worktree info`/`list` and main-worktree classification.
- `crates/jit/src/storage/json.rs:616-702` — Git and main-worktree index fallback.
- `crates/jit/src/storage/json.rs:705-742` — Git `HEAD` issue fallback.
- `crates/jit/src/storage/json.rs:744-811` — uncommitted main-worktree issue fallback.
- `crates/jit/src/storage/json.rs:1053-1090,1093-1131` — issue fallback chains.
- `crates/jit/src/storage/repository_state_store.rs:927-934,1359-1435` — linked-worktree evidence capture from local/Git-HEAD/main.
- `crates/jit/src/storage/repository_state_store.rs:1438-1457` — main-worktree discovery via Git common-dir.
- `crates/jit/src/storage/json.rs:1216-1227` — events are local-only; **no fallback exists for events**, unlike issues/index.

### 3.b Direct `.jit` filesystem writes bypassing `RepositoryMutationSession`

All are non-semantic/machine-local, or test-only. No production issue/config/event publisher
outside the transaction machinery was found.

- `crates/jit/src/storage/worktree_identity.rs:157,169-185,208-212,243-274` — worktree identity: atomic (`NamedTempFile`, `write_all`, fsync, `persist` rename, parent-dir fsync). Intentional machine-local exception.
- `crates/jit/src/commands/serve.rs:84-92` — `.jit/server.pid.json`: atomic rename, but **not fsynced** (weaker durability than worktree identity). Intentional runtime exception.
- `crates/jit/src/commands/serve.rs:557-572` — default `.jit/server.log`: ordinary non-atomic append/create. Intentional runtime exception.
- `crates/jit/src/storage/lock.rs:280-305,331-339` — lock files and lock metadata under `.jit`. Housekeeping.
- `crates/jit/src/storage/repo_lock.rs:282-289` — repository lock file creation. Housekeeping.
- `crates/jit/src/storage/temp_cleanup.rs:31-47` — stale `.tmp` file removal below `.jit`. Housekeeping.
- `crates/jit/src/commands/snapshot.rs:446-471` — writes a caller-selected snapshot *export destination's* `.jit` contents; not the live repository's semantic store.
- Test-only fixture writes (not production bypasses): `commands/claim.rs:1136-1148,1816-1819`; `commands/init.rs:751-754`; `storage/json.rs:168-181,2320-2322,2603-2619`; `storage/repository_state_store_tests.rs:4056-4057`.

### 3.c Tests/docs pinning current linked-worktree mutation behavior

**Every existing test that mutates from a linked worktree does so only after first running
`jit init` there.** This is the single most consequential fact for D-01's design: a
refuse-by-default policy keyed on "has `jit init` been explicitly run in this worktree" would
not break a single one of these tests as written.

`cross_worktree_integration_tests.rs` (`crates/jit/tests/cli_repo_workflow/`):

- `test_issue_show_reads_from_git_in_secondary_worktree` (:73-126) — `jit init` at :118-119 (mutating success), then reads committed issue.
- `test_issue_show_reads_from_main_worktree_uncommitted` (:128-169) — `jit init` at :161-162, reads uncommitted main issue.
- `test_query_all_shows_issues_from_all_sources` (:171-255) — `jit init` at :225-226, creates issue C at :228-235 (mutating success).
- `test_graph_show_works_across_worktrees` (:257-329) — `jit init` at :312-313.
- `test_partial_id_resolution_across_worktrees` (:331-386) — `jit init` at :377-378.
- `test_local_overrides_git_and_main` (:388-460) — `jit init` at :433-434, `issue update` at :436-447 (both mutating successes); secondary sees the modified title, primary still sees the original.
- `test_deletion_blocked_in_secondary_worktree` (:462-510) — `jit init` at :488 (mutating success), but `issue delete` from that worktree is asserted to **fail** at :497-509 — this is the existing `main.rs:3546-3550` guard from §2, already pinned by a test.
- `test_deletion_requires_env_var` (:512-556) — primary-worktree only, not in scope for this sweep.

`worktree_cli_tests.rs` (same directory):

- `test_worktree_list_multiple_worktrees` (:334-371) — `jit worktree info` at :341-347 (can create the identity file; not an issue mutation).
- `test_init_in_new_worktree_generates_unique_id` (:562-597) — `jit init` at :569-574, distinct identity asserted.
- `test_init_is_idempotent_in_worktree` (:599-631) — `jit init` twice at :606-611/:618-623, stable ID asserted.
- `test_worktree_list_shows_distinct_ids` (:633-679) — `jit init` in two linked worktrees at :641-652, three distinct IDs asserted from primary.
- `test_git_worktree_move_preserves_id` (:681-784) — `jit init` at :739-744, then `git worktree move` preserves identity.

Documented (not just tested) sanctioned linked-worktree mutation:
`docs/how-to/multi-agent-coordination.md:193-203` ("Writes go to LOCAL `.jit/` only"),
`docs/tutorials/parallel-work-worktrees.md:134-152` (issue state updates and completion from a
secondary worktree). Historical: `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md:83-109`
documents `jit issue update` run from a linked worktree in the earliest manual experiment.

### 3.d Consumers of profile conflict/divergence JSON and selector error text

**Rust CLI/integration tests** (all in `crates/jit/tests/cli_repo_workflow/` unless noted):

| File:line | Asserts |
|---|---|
| `profile_cli_tests.rs:309-346` | Conflicting repeatable selectors → `PROFILE_CONFLICT`, exit 4, nothing published. |
| `profile_cli_tests.rs:401-405` | Bare positional `profile show` → Clap's `unexpected argument`. |
| `profile_cli_tests.rs:407-419` | Removed `--from` flag → `unexpected argument`. |
| `profile_cli_tests.rs:422-435` | Malformed `profile:planner` → `expected id:ID or path:DIR`. |
| `profile_cli_tests.rs:1685-1711`, `:1714-1742` | Profiled-init target conflict → `PROFILE_CONFLICT`, occupant/authored state preserved. |
| `profile_cli_tests.rs:1981-2025` | Conflict JSON code, target/profile names, `recorded`/`now`/`would publish` wording, mentions `jit profile capture`, no Rust debug type names. |
| `profile_cli_tests.rs:2062-2121`, `:2271-2304` | Capture's count-wrapped JSON shape and failure-mode JSON. |
| `profile_edit_refresh_cli_tests.rs:97-107` | Helper consumes `error.details.profiles[*].divergences[*].kind/target`. |
| `profile_edit_refresh_cli_tests.rs:114-174`, `:181-252` | `changed_target` divergence JSON, `PROFILE_CONFLICT` on reapplication, capture-refresh JSON. |
| `profile_acceptance_tests.rs:797-1029` | Generated command/success schemas, envelopes, `reason` fields. |
| `profile_acceptance_tests.rs:1069-1111`, `:1141-1422` | Validation JSON envelope; divergence `kind`/`target`; `changed_target`, `absent_target`, `unreadable_package`, `unowned_target`, `changed_package_identity`. |
| `profile_acceptance_tests.rs:1435-1490`, `:1515-1575` | Diff JSON consumer; create/conflict decisions. |
| `profile_acceptance_tests.rs:1652-1713` | Semantic-declaration create/conflict decisions and owner JSON. |
| `profile_no_git_lifecycle_tests.rs:228-238` | Clean validation → empty `divergences` array. |
| `integration_schema.rs:59-142`, `:144-185` | All profile commands expose typed success schemas; `show`/`apply`/`reconfigure`/`upgrade` expose only required repeatable `--profile`, no positional/`--from`. |
| `help_cross_reference_tests.rs:105-120` | Help text mentions repository package locations/worktree directories. |

**Rust unit/harness layers:**

| File:line | Asserts |
|---|---|
| `crates/jit/src/repository_state/profile_apply.rs:4078-4174` | Conflict prose contains remedies, no Rust debug formatting, names profile/subject, mentions capture only where applicable. |
| `crates/jit/src/commands/profile.rs:3831-3844,5127-5169,5842-5975,6721-6770,7014-7072,7263-7301,7452-7497,8354-8436` | Typed conflict values; rendered refusal contains `.message()`; reconfigure/upgrade report typed divergence without publishing; diff/semantic-declaration conflict reasons equal typed refusal messages. |
| `crates/jit/src/main.rs:8973-8996` | Repository-state conflicts map to `PROFILE_CONFLICT`/exit 4, but does **not** assert structured details — i.e., no existing test would break if structured details were added here, only if the flat message changed. |
| `crates/jit/tests/fast_rules/derived_state_repair_tests.rs:450-475,517-545,547-575,620-647` | Profile-owned drift diagnosis/repair, package/record disagreement prose, unresolved-location prose, clean-repo no-findings. |

**Schema and MCP:**

- `crates/jit/src/schema.rs:477-501,510-528` — exposes `ProfileAgreementResult`/`ProfilePlanResult` success schemas; **no error schema exposed here**.
- `mcp-server/lib/schema-loader.js:1-26` — CLI schema is the MCP source of truth.
- `mcp-server/lib/tool-generator.js:27-98` — generates input/success `outputSchema`; no profile-specific error schema path.
- `mcp-server/curated-tools.json:56-63` — describes profile commands; validate already mentions divergence and capture as a recovery choice.
- `mcp-server/test-unit.js:465-491`, `test-integration.js:396-446,449-465,468-495,597-641,646-682,684-706` — asserts tool inventory, input/required-field parity, output shapes, selector order — **no MCP test asserts a structured profile conflict error, `ProfileTargetConflict` shape, divergence `kind` shape, or selector parse error.**

**Web:** no consumer found under `web/src` — searches for `profile`, `ProfileSelector`,
`divergence`, `conflict`, and profile-specific error handling returned no matches. Existing
web error handling is generic network/API handling. **REQ-04/REQ-05 changes have no web-layer
blast radius.**

**Documentation:** `docs/reference/cli-command-grammar.md:86-91`;
`docs/reference/cli-commands.md:536-551,631-650,673-690,692-727,729-780,782-822,824-866,868-900,902-925`;
`docs/reference/profiles.md:315-347` (already gives restore-vs-capture prose guidance —
REQ-04 needs to structure this, not invent it), `:349-362,420-449`;
`docs/reference/error-codes.md:32-42` (`PROFILE_CONFLICT` definition);
`docs/reference/jit-content-standards.md:11-12`; `docs/tutorials/quickstart.md:76-96`.

## 4. Primitive verification

**2-phase-commit + journal recovery for `execute_repository_delta` — confirmed.** The
function is literally named `execute_repository_delta`
(`dev/architecture/repository-state-materialization.md:313,327`, matching
`crates/jit/src/storage/file_transaction.rs`). Durable sequence, each step independently
verified against source:

1. Initial journal written atomically and synced (`file_transaction.rs:909-918`; staged files
   `sync_all` at `transaction_staging.rs:6-16`).
2. A complete `Prepared` journal is written before any live mutation (`file_transaction.rs:1180-1184`).
3. Live parents are synced before the terminal decision (`file_transaction.rs:1396-1420`).
4. Journal flips to `Committed` and syncs before cleanup and `Ok` return (`file_transaction.rs:1004-1018`).
5. On reopen, `Prepared` journals roll back and `Committed` journals verify/converge through cleanup (`file_transaction.rs:1845-1865`).

Failure-injection evidence backs this: `repository_state_store_tests.rs:541-580` (failures
across prepare/publish/commit/cleanup, reopen verifies complete old/new state), `:2604-2632`
(forward crash recovery, both root layouts), `:2975-3043` (interrupted rollback recovery),
`:1474-1504` (post-commit failure recovers forward without rollback).

**`worktree.json` write — atomic**, confirmed: same-directory `NamedTempFile`, content
fsync, `persist` rename, parent-directory fsync (`worktree_identity.rs:243-274`).

**Server PID-file write — atomic rename, but not fsynced** (`serve.rs:84-92`): weaker
durability than the worktree-identity write; a crash between rename and the next disk flush
could still lose the PID record, though this is machine-local, non-semantic state so the
consequence is bounded to stale PID detection, not repository data.

**Gitfile worktree behavior — correct.** For a linked worktree whose `.git` is a file, Git
transparently follows it; `--git-common-dir` returns the primary's shared `.git`,
`--show-toplevel` returns the linked worktree's own root, so `common_dir != worktree_root/.git`
and `is_worktree()` correctly reports `true` (`worktree_paths.rs:52-136`). Directly confirmed
against the live repository: `.agents/worktrees/agent-0708d692/.git` is a plain gitfile
pointing at `/home/vkaskivuo/Projects/just-in-time/.git/worktrees/agent-0708d692`, and `git
rev-parse` from inside it returns the primary's common-dir and the linked directory as
top-level, exactly as the code expects.

**Git absent / non-Git directory — degrades gracefully, Git-optional invariant holds.** If
`git rev-parse --is-inside-work-tree` fails or Git is absent, the result is converted to
`false` via `.unwrap_or(false)`, and `WorktreePaths` falls back to a non-Git layout using the
supplied root (`worktree_paths.rs:50-68`). Discovery itself never invokes Git at all —
it is a pure filesystem ancestor walk (`discovery.rs:43-68`). Other Git-dependent features
(e.g. claim coordination) still require Git and fail/disable appropriately
(`commands/init.rs:523-554`; `commands/mod.rs:3091-3099` treats absent Git evidence as
`NotApplicable`), but core repository storage remains Git-optional as required by
`@/charter/D-4`.

## 5. Architecture fit

**REQ-01's guard belongs at CLI dispatch, following existing precedent, not inside storage
discovery or `CommandExecutor` construction.** `main.rs:3546-3550`'s `issue delete` guard is
the concrete, already-shipped example of exactly this shape:
`storage.is_secondary_worktree()` checked at the dispatch site, before the command proceeds.
The disqualifying property of that existing guard is that it is unconditional — it has *no*
opt-in — while REQ-01 explicitly asks for "an explicit, auditable opt-in path." Generalizing
this pattern (rather than inventing a second, parallel one — see convention-convergence,
below) means: (a) consolidating the five worktree-detection mechanisms from claim 4 onto one
(`WorktreePaths` is the most complete and the one already computed on every dispatch,
`main.rs:2265-2279`); (b) deciding what the "explicit, auditable opt-in" signal is, given the
live tension in §2 between the older protocol's "run `jit init` here" and the newer
protocol's "don't"; (c) scoping which commands the guard applies to — today it is one
(`delete`); REQ-01 implies "any state-mutating command."

**`WorktreePaths` should become the single detection primitive**, not a sixth
implementation. `JsonFileStorage::is_secondary_worktree` (`json.rs:814-831`),
`load_index_from_main_worktree`'s `git2` check (`json.rs:667-702`), and
`load_issue_from_main_worktree`'s inline `git rev-parse` (`json.rs:754-811`) are three more
call sites doing conceptually the same "am I in a linked worktree, and where is the main one"
work independently. This is squarely the `convention-convergence` invariant: "a local parallel
variant... is a defect unless it is a named, cited exception with a tracked convergence
condition." None of these five are cited as an exception anywhere found in this sweep.

**REQ-02's divergence-detection surface has no natural home yet** — `jit query divergence`
and `jit validate --branch-drift` are both already-used names for unrelated concepts (§1,
claim 7), so a new command or subcommand name is needed rather than an extension of either.
`jit worktree info`/`jit worktree list` (`docs/reference/worktree-validate.md:10-155`) are the
existing read-only worktree-introspection surface and are a plausible location to extend
(e.g., a `jit worktree list --check-divergence` or a dedicated verb), but this sweep found no
existing partial implementation to build on — REQ-02 is new surface, not a gap in existing
surface.

**REQ-03's failure-injection gap should reuse `TransactionFailureInjector` +
`with_repository_state_failures` exactly as the two existing gate-definition tests do**
(`commands/gate.rs:1884-1919,1921-1955`), just targeting a failure point reached during
`check_gate_with`'s `session.apply` instead of `define_gate`'s. This needs no new test
infrastructure.

**REQ-04's JSON-detail wiring should reuse `JsonError::with_details`/`with_suggestions`
exactly as `profile validate`/`profile diff` already do** (`main.rs:1121-1135,1053-1067`) —
this is not new mechanism, it is extending an existing, already-proven pattern to two more
error branches (`main.rs:1174-1181,1201-1210`).

**REQ-05 is entirely local to `profile show`'s own clap definition and dispatch** — the
shared `parse_profile_selectors` (`main.rs:837-846,916-930`) must not change, since other
lifecycle commands depend on its current tag-required contract (confirmed by
`integration_schema.rs:144-185`, which explicitly asserts other lifecycle commands expose
*only* the repeatable `--profile` flag, no positional).

**Domain-agnostic invariant check for a worktree-policy flag:** a boolean like "refuse
mutation outside the primary worktree unless opted in" is mechanism, not instance — it is
engine safety behavior parameterized by which directory Git reports as primary, not by any
repository-declared vocabulary (label namespace, gate key, item kind, template). It does not
need to come from `.jit/config.toml` to satisfy `domain-agnostic`, but *should* still be
configurable there (consistent with `enforce_leases`'s existing `[worktree]` section pattern)
so a repository can choose its own default, matching D-01's "explicit, auditable opt-in"
requirement without hardcoding one policy for every adopter.

## 6. Invariant check

**Git-optional core commands (`@/charter/D-4`).** Confirmed in §4: `WorktreePaths` and
discovery degrade to a non-Git, single-directory model when Git is absent, and this remains
true regardless of how REQ-01's guard is implemented, *provided* the guard's condition is
itself expressed in terms of `WorktreePaths`/`is_worktree()`, which already returns `false`
(not an error) outside Git. A REQ-01 implementation must confirm the non-Git path is `false`
(never blocked) rather than erroring, or D-4 breaks for every non-Git repository.

**Primary worktree, always exempt.** `is_worktree()` is `false` in the primary worktree by
construction (`common_dir == worktree_root/.git`), so any guard gated on it is naturally a
no-op for the common case, matching D-01's intent that ordinary single-checkout usage is
unaffected.

**`JIT_DATA_DIR` override.** Discovery still runs to derive a worktree root even when
`JIT_DATA_DIR` is set (`main.rs:2231-2238`, claim 1) — a REQ-01 guard should confirm what
"primary worktree" means when the data directory has been explicitly overridden to a location
that may not correspond to any worktree at all; this combination was not exercised by any
existing test found in this sweep and needs explicit coverage.

**Event-log invariant vs. refusal-before-write.** `event-log` requires every state change to
append an event; a REQ-01 refusal must occur *before* any `RepositoryDelta` is constructed (at
CLI dispatch, per §5), so that a refused command produces zero writes and zero events, not a
partial state change followed by a refusal. The existing `issue delete` precedent already
satisfies this shape (the `bail!` in `main.rs:3546-3550` occurs before `executor` is invoked
for that command).

**`canonical-cutover` implications if the read-side union fallback (claim 5) is narrowed or
removed.** This fallback is adopter-facing, documented behavior (§1, claim 5), not incidental
implementation detail — removing or narrowing it is a breaking behavior change requiring
updates to `docs/tutorials/parallel-work-worktrees.md`, `docs/how-to/multi-agent-coordination.md`,
and the design doc that specifies it, plus the tests in §3.c that currently assert on it
(`test_query_all_shows_issues_from_all_sources`, `test_local_overrides_git_and_main`,
`test_graph_show_works_across_worktrees`, `test_partial_id_resolution_across_worktrees`,
`test_issue_show_reads_from_git_in_secondary_worktree`,
`test_issue_show_reads_from_main_worktree_uncommitted`). Since this is a greenfield project
(no shipped-release back-compat obligation), a deliberate narrowing is permitted, but it is
not a no-op cleanup — it is a documented contract change with a concrete test and doc blast
radius, all enumerated in §3.c/§3.d.

## Claim-count summary

19 numbered claims + 6 REQs = 25 items assessed.

- **Already-done:** REQ-03's core durability primitive (claim 8's explicit finding).
- **Valid-and-open:** claims 1 (partially — the qualifier is a correction, not a refutation),
  2 (as scope clarification), 3, 4, 5 (confirmed intentional, narrows the redesign question
  rather than opening it), 6, 7, 9, 10, 11, 12, 13, 14, 16, 17, 18 (with a correction on direct
  reusability), 19; REQ-01, REQ-02, REQ-04, REQ-05, REQ-06.
- **Invalid-as-stated (required correction to the file:line or scope, not to the underlying
  conclusion):** claims 1, 3, 4, 8, 13, 15 (confirmed but needed the full sweep), 18 — none of
  these were refuted outright; every "CORRECTED" verdict above still supports the same
  direction as the original hypothesis once the citation or scope is fixed.
- **Refuted:** none. No claim in the input brief was found to be substantively wrong; the
  corrections were all precision fixes (line numbers, exact scope, an undercount of mechanisms
  in claim 4, an undercount of gap-severity in claim 8 which turned out to be *better* news
  than the hypothesis — REQ-03's core guarantee already holds).
