# Plan: gf2 adopter reliability — safe worktrees, durable gates, actionable profiles (4a559332)

> Planning node: 629c3d73. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

Three independent adopter hazards share one container: a linked checkout can build a
divergent authoritative store that routine cleanup destroys, a gate evaluation can look
successful with nothing recorded, and profile recovery advice reaches only human readers.
The investigation resolved the largest open question in the container's favour — the gate
durability primitive already holds on the automated path — and narrowed the largest design
question to a choice between three worktree authority models, settled below as OD-1.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | The per-invocation mutation classifier already on the command enumeration — exhaustive over its delegating command families, with five wildcard-matched read-only families a new mutating leaf must join deliberately — combined with one converged worktree primitive, becomes a dispatch-time condition evaluated before the repository mutation session is opened. The stance is a typed configuration key defaulting to refusal, with an explicit per-invocation override, and the adopter guides that teach the linked-checkout workflow state the stance. | Classifier: `crates/jit/src/cli.rs:3232-3270`. Layout computed on every dispatch: `crates/jit/src/main.rs:2265-2279`; mutation session opens at `:2286-2303`. Narrow precedent already shipped for one command: `crates/jit/src/main.rs:3546-3550`. Key shape to follow: `crates/jit/src/config.rs:1308-1313`. Guides teaching the governed workflow: `docs/tutorials/parallel-work-worktrees.md:28-35,134-152`, `docs/how-to/multi-agent-coordination.md:193-203`. |
| REQ-02 | A new read-only check in the worktree command family compares issue records and event logs across two checkouts, and a documented procedure preserves both sides before any reconciliation. No merge subsystem (OD-2). | Both plausible names are taken by unrelated concepts (investigation §1 claim 7). Events have no cross-checkout read path: `crates/jit/src/storage/json.rs:1216-1227`. Union merge already declared for the event log: `.gitattributes`. The unguarded cleanup recipe: `docs/how-to/multi-agent-coordination.md:303-312`. |
| REQ-03 | The durability guarantee already holds end to end on the automated path, so the remaining work is the missing regression coverage plus a separately-scoped fix for the postcheck loop that discards checker errors. | Coupled plan applied before success: `crates/jit/src/commands/gate_check.rs:1016-1034`. Discarded result: `:1618`. Injection pattern to follow: `crates/jit/src/commands/gate.rs:1884-1955`. |
| REQ-04 | The exhaustive remedy match already inside the conflict type becomes a serializable value, and the two refusal branches attach it using the details-and-suggestions pattern two neighbouring subcommands already prove. | Typed remedy match: `crates/jit/src/repository_state/profile_apply.rs:673-701`. Branches discarding the typed value: `crates/jit/src/main.rs:1174-1181`, `:1201-1210`. Proven attachment pattern: `:1121-1135`. |
| REQ-05 | A bare positional id is accepted on the show command alone and translated to an identity selector inside that command's own dispatch (OD-4). | Required repeatable selector: `crates/jit/src/cli.rs:2951-2960`. Other lifecycle commands pinned to flag-only: `crates/jit/tests/cli_repo_workflow/integration_schema.rs:144-185`. |
| REQ-06 | No gf2 artefact survives, so the reproduction is built live with real linked checkouts (OD-3), landing as modules of the integration target with the most remaining room. Profile and selector evidence rides in the tasks that change those surfaces, whose suites already run the real binary. | No preserved fixture (investigation §1, REQ-06 assessment). Live-checkout precedent: `crates/jit/tests/cli_repo_workflow/cross_worktree_integration_tests.rs`. Budget: `scripts/rust-build-budget.sh:41-43`, 10 of 12 targets used. |

## Shared architectural contracts

### `worktree-authority` [implementation-produced] — Single worktree detection primitive

The one answer to both halves of "where am I": whether the selected data root sits inside a
linked non-primary checkout, and where the primary checkout's store is. Outside version
control it reports a non-linked checkout rather than failing, preserving Git-optional core
commands (`@/charter/D-4`). When an environment override selects the data root, the answer
follows that selected root's location, not the process working directory. It replaces the
five independent implementations enumerated in investigation §1 claim 4.

### `worktree-write-policy` [implementation-produced] — Declared mutation stance

The typed value declaring whether state-mutating commands may run inside a linked non-primary
checkout, its refusing default when nothing is declared, and the precedence between the
declared stance and a per-invocation override. It follows the shape of the lease-enforcement
key in the same configuration section (`crates/jit/src/config.rs:1308-1313`) while stating its
own resolution explicitly rather than inheriting that key's present-section asymmetry.

### `worktree-refusal-audit` [implementation-produced] — Refusal and override record

A refused invocation performs no repository write and appends no event, and returns a refusal
naming the linked checkout, the stance that refused it, and how to permit the operation — in
both the rendered and the machine-readable form. A permitted override leaves a durable record
a later reader finds in the repository's own history. Refusing before any repository delta is
constructed is what keeps the append-on-state-change guarantee intact rather than excepted.

### `store-divergence-report` [implementation-produced] — Cross-checkout divergence findings

The read-only finding model: issue records present in one checkout's store and absent from the
other in either direction, event records held by one log and missing from the other, and
records present in both whose values conflict — the last reported as its own class. Empty in
the primary checkout, in an agreeing linked checkout, and outside version control. The
machine-readable form carries the same findings, wrapped in the repository's list envelope.

### `gate-durability-boundary` [plan-fixed] — Gate success implies durable record

On the automated evaluation path, the issue update, the gate-run record, and the event are
built into one plan and applied through the recovering two-phase-commit kernel before success
is returned; a persistence failure propagates to the caller
(`crates/jit/src/commands/gate_check.rs:1016-1034`, `crates/jit/src/storage/file_transaction.rs:1004-1018`).
This container does not change that boundary — it covers it, and fixes the separate postcheck
path that discards a checker's result without surfacing it.

### `profile-remedy-data` [implementation-produced] — Serializable remedy vocabulary

The typed value naming each resolution that applies to a profile conflict or divergence,
distinguishing restoring recorded content from capturing repository content and marking
capture applicable only where it is valid. Rendered prose derives from it, so the sentence and
the data never disagree. gf2's own drift is the field case where capture is the valid remedy
and restore is not (investigation §2).

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| worktree-detection-convergence | Converge worktree detection on one primitive | task | One primitive answers whether this checkout is linked and where the primary store lives. | — | REQ-01, OD-5 | touches 5 | worktree-safety | — |
| worktree-write-policy-config | Worktree write policy configuration key | task | A typed worktree policy key resolves the mutation stance for linked checkouts, defaulting to refusal. | — | REQ-01, OD-1 | touches 3 | worktree-safety | — |
| worktree-write-guard | Refuse state-mutating commands in linked worktrees | task | State-mutating dispatch refuses inside a linked checkout unless the stance or an explicit override permits it. | worktree-authority, worktree-write-policy | REQ-01, OD-1, OD-5 | touches 6 | worktree-safety | worktree-detection-convergence, worktree-write-policy-config |
| worktree-divergence-detection | Report divergent checkout stores | task | A read-only check reports the issue records and events one checkout holds without the other. | worktree-authority | REQ-02, OD-2 | touches 5, uncertain | worktree-safety | worktree-detection-convergence |
| worktree-recovery-guidance | Recovery guidance for divergent checkout stores | task | Adopters recover a divergent checkout store losslessly and screen for unmerged state before discarding one. | store-divergence-report | REQ-02, OD-2 | touches 2 | worktree-safety | worktree-divergence-detection |
| gate-durability-regression-test | Regression coverage for gate evaluation durability | task | Injected persistence failure during a gate evaluation returns failure and leaves no passing gate record. | gate-durability-boundary | REQ-03, REQ-06, D-02 | creates 1, touches 1 | gate-durability | — |
| postcheck-error-surfacing | Surface swallowed postcheck failures | task | A postcheck persistence failure reaches the caller instead of being silently discarded. | gate-durability-boundary | REQ-03, D-02 | touches 2 | gate-durability | — |
| profile-remedy-model | Typed remedy data on profile conflicts | task | Profile conflicts and divergences carry a serializable remedy naming which resolutions apply. | — | REQ-04 | touches 2 | profile-guidance | — |
| profile-conflict-json-details | Profile conflict output carries its remedy | task | Profile refusals and divergences reach machine consumers with structured details plus suggested resolutions. | profile-remedy-data | REQ-04, REQ-06 | touches 4 | profile-guidance | profile-remedy-model |
| profile-guidance-reference | Profile recovery guidance reference | task | The profile reference and the bridge tool descriptions state the machine-readable remedy contract. | profile-remedy-data | REQ-04 | touches 3, uncertain | profile-guidance | profile-conflict-json-details |
| profile-show-positional | Positional profile id for the show command | task | A bare positional id selects a recorded profile on the show command without touching the shared grammar. | — | REQ-05, REQ-06, OD-4 | touches 5 | profile-guidance | — |
| worktree-policy-journey | Linked checkout journey coverage | task | A live linked checkout proves the write policy, the divergence check, and the recovery procedure end to end. | worktree-write-policy, worktree-refusal-audit, store-divergence-report | REQ-06, OD-1, OD-3 | creates 1, touches 1 | worktree-safety | worktree-write-guard, worktree-recovery-guidance |

```mermaid
flowchart LR
    N0["worktree-detection-convergence: Converge worktree detection on one primitive"]
    N1["worktree-write-policy-config: Worktree write policy configuration key"]
    N2["worktree-write-guard: Refuse state-mutating commands in linked worktrees"]
    N3["worktree-divergence-detection: Report divergent checkout stores"]
    N4["worktree-recovery-guidance: Recovery guidance for divergent checkout stores"]
    N5["gate-durability-regression-test: Regression coverage for gate evaluation durability"]
    N6["postcheck-error-surfacing: Surface swallowed postcheck failures"]
    N7["profile-remedy-model: Typed remedy data on profile conflicts"]
    N8["profile-conflict-json-details: Profile conflict output carries its remedy"]
    N9["profile-guidance-reference: Profile recovery guidance reference"]
    N10["profile-show-positional: Positional profile id for the show command"]
    N11["worktree-policy-journey: Linked checkout journey coverage"]
    N0 --> N2
    N1 --> N2
    N0 --> N3
    N3 --> N4
    N7 --> N8
    N8 --> N9
    N2 --> N11
    N4 --> N11
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| OD-1 — worktree authority model (resolves epic D-01) | Chosen: refuse by default, with a declared configuration stance and an explicit audited override. Rejected: a repository-shared canonical store and a write-through redirect to the primary, because both contradict the documented per-checkout design that four independent sources confirm is deliberate (investigation §1 claim 5); rejected initialization-in-the-checkout as the opt-in signal, because the newer dispatch protocol instructs agents to skip it (investigation §2). |
| OD-2 — depth of REQ-02 | Chosen: detect, report, preserve. A read-only divergence surface plus a documented lossless recovery path that reports semantic conflicts. Rejected: automatic store reconciliation or a merge subsystem, a permanent maintenance burden guarding a state that OD-1 makes rare. |
| OD-3 — REQ-06 evidence base | Chosen: an equivalent linked-checkout reproduction built live in tests, which the criterion explicitly permits. Rejected: a committed gf2 fixture, because no backup survives and the live checkout is untouched field evidence rather than a test artefact. |
| OD-4 — REQ-05 shape | Chosen: accept an unambiguous positional id on the show command only, translated inside that command's dispatch. Rejected: loosening the shared selector parser, whose tag-required contract the other lifecycle commands depend on. |
| OD-5 — detection convergence | Chosen: the five detection implementations converge onto the layout primitive as part of this container, before the guard is written. Rejected: adding the guard as a sixth implementation, which `@/inv/convention-convergence` names a defect absent a cited exception. |
| D-02 — reproduce before remediating | Held. The investigation reproduced each defect at the planning tier and found REQ-03's durability property already satisfied on the automated path, which is why that criterion's work is coverage plus a separately-scoped postcheck fix rather than new durability machinery. |
| Risk — a refusing default breaks this repository's own agent fleet | The dogfood opt-in is declared in this repository's tracker configuration by the task that introduces the key, which the guard task depends on, so no ordering exists in which the fleet runs against a refusing default. |
| Risk — convergence narrows documented read-side fallback behaviour | The layered issue read fallback is adopter-facing documented behaviour with a known test and documentation blast radius (investigation §6). Its observable behaviour is held fixed; only the detection question underneath converges. |
| Risk — the postcheck swallow's reachability from today's CLI is unestablished | Enumerating the invocations that reach that loop is the first criterion of the task that fixes it, so the surfacing shape is chosen from evidence rather than assumed. |
| Risk — profile prose is pinned by unit assertions, including capture-only-where-applicable | The prose derives from the typed remedy rather than being replaced by it, so those assertions stay meaningful; the pinned command-boundary assertions are extended by the same tasks that change the surfaces (investigation §3.d). |

## Investigation sources

- [Investigation](investigation.md) — claim-by-claim verification, the exhaustive consumer
  sweeps for worktree detection, direct `.jit` writers, linked-checkout tests, and profile
  JSON consumers, plus the prior-art sweep behind OD-1 and OD-2.
