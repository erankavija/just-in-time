# Intake: redo-record mutations (f1dc843a)

Input for planning `f1dc843a` in `plan-from-existing` mode. It records what the
session that filed the epic measured and decided. Governing decision:
`@/charter/D-17`.

## Owner decisions

These were settled with the owner before the epic was filed. Planning takes them
as given.

- **Mechanism.** The redo record replaces the journaled rollback kernel. It
  supersedes decisions D11 and D14 of the archived plan
  `dev/archive/cdc840ad-repository-materialization/cdc840ad-plan.md`, which are
  not the charter's own D-11 and D-14.
- **Capture goes too.** Phased capture, listing fingerprints, mutation document
  closure, apply-time whole-read-set revalidation, and the plan hash over
  captured entries are removed in the same epic, not just the kernel.
- **Record location.** The redo record lives under the data root's gitignored
  `tmp/`. That keeps core commands free of Git (`@/charter/D-4`).
- **Guiding constraints.**
  - Simplicity rules.
  - No mutation cost may scale with project history.
  - The epic must remove more code than it adds (REQ-06).
- **Release gate.** The epic gates the v1.0 tag: `bb03df0a` depends on it, and
  `@/charter/D-14` names it.
- **Moved criterion.** 9750b151's 100 ms criterion moved here as REQ-01.
  9750b151 completes on its other criteria.

## Measurements

Method:
- Two copies on tmpfs: `git clone --shared` with the live `.jit/` synced in.
  - The adopter repository at `~/Projects/gf2` commit `9102aef73` has 876
    issues, 1469 unpinned document references (116 MB), 42 pinned revisions,
    and a 3.0 MB event log.
  - This repository has 1104 issues and a 3.2 MB event log, and enforces
    leases.
- Each row is the median of three runs of one `jit issue update`.
- The harness is [bench.sh](bench.sh). Run it as `bench.sh <jit-binary> <label>`.

| Build | gf2 changing | gf2 idempotent | jit changing | jit idempotent | jit lease-refused |
| --- | ---: | ---: | ---: | ---: | ---: |
| `8107fc280`, before 9750b151 | 1777 ms | 1260 ms | 445 ms | 335 ms | 323 ms |
| `f69b98455`, 9750b151 complete | 110 ms | 69 ms | 119 ms | 75 ms | 73 ms |

For the 9750b151 row:

- **What those commits changed:**
  - Captured bytes carry their identity from construction, so closing an image
    no longer re-hashes them.
  - Mutations capture only plan documents.
  - Each attempt captures once, after a light preflight where one is needed.
  - The plan hash digests written bytes instead of serializing them.
  - Listed children reuse the identity their exact path already recorded.
  - Captured issues are parsed once.
- **What remains in a changing update** (frame-pointer profile of `f69b98455`
  on this repository):
  - SHA-256 is 36 % of wall time. About 20 % is kernel publication re-hashing
    the event log roughly eight times: backup, initial journal, preimage
    inspections, move-aside, final identity. About 9 % is base capture, 4 % is
    the apply-time revalidation capture, and 2.5 % is the plan hash.
  - Base capture takes 48 %, in three phases. Phases two and three each read
    every issue file and the event log.
  - Parsing every issue takes 11 %.
  - Kernel publication takes 20 %.
  - `RepositoryImage::close` takes 9 %; half of that canonicalizes every
    listed child's path.
  - Image clones into the session and the plan take about 5 %.
- **Scaling:** the whole-file event-log rewrite scales with history. Reading
  and parsing every issue scales with issue count, about 10 ms at 1000 issues.

## Code inventory

Line counts at `f69b98455`, the starting point for REQ-06:

| File | Lines |
| --- | ---: |
| `crates/jit/src/storage/file_transaction.rs` | 3525 |
| `crates/jit/src/storage/transaction_journal.rs` | 179 |
| `crates/jit/src/storage/transaction_recovery.rs` | 172 |
| `crates/jit/src/storage/transaction_staging.rs` | 16 |
| `crates/jit/src/storage/guard_order.rs` | 96 |
| `crates/jit/src/storage/repository_state_store.rs` | 1973 |
| `crates/jit/src/storage/repository_state_store_tests.rs` | 5051 |
| `crates/jit/src/storage/memory.rs` | 1080 |
| `crates/jit/src/repository_state/image.rs` | 2597 |
| `crates/jit/src/repository_state/mutation.rs` | 2539 |

Consumers of `open_mutation_session`, `capture_proposed_base`, or
`capture_mutation_base` under `crates/jit/src/`:

- **Commands:** `commands/`, namely archive, batch_create, bulk_update, claim,
  config, dependency, document, gate, gate_check, init, issue, mod, profile,
  template, and validate.
- **Storage:** `storage/`, namely gate_runs, json, repository_state_store,
  file_transaction, and the contention and conformance test suites.
- **CLI:** `main.rs`.

The investigator produces the full consumer inventory.

## Open questions for planning

1. **Scope of the redo mechanism.** Validation, `validate --fix` repair,
   profile apply, config, archive, gate runs, and project render also publish
   through the current session and kernel. Decide which of them switch to the
   redo record and what each still needs to read. D-17 governs issue
   mutations, and REQ-05 allows no second publication path.
2. **Read set per mutation.** D-17 allows a mutation to read the issues its
   derivation consults. Decide whether the first cut keeps reading every
   issue, which scales with issue count rather than history and sits within
   REQ-01, or narrows to the target and its dependency neighbourhood.
3. **The memory backend.** Decide how it models the redo record and its
   interruption points, so REQ-03 runs the same contract on both backends
   (`@/inv/shared-test-contracts`).
4. **The event-log tail window.** Decide its size for the preimage check, and
   how appends interact with the existing torn-tail finalization.
5. **Invariant wording.** `@/inv/atomic-writes` names temp-file and rename for
   replacements. Decide the statement for positioned appends and redo replay.
   `@/inv/derived-state-coherence` says "publishes recoverably", which the
   redo record satisfies.
6. **Test consolidation.** The kernel's failure-point conformance suites,
   including `profile_interruption_tests` and the new
   `RepositoryBeforeReadSetRevalidation` point, shrink to the redo record's
   interruption points. That shrinkage is where most of REQ-06 comes from.
7. **Regression protection.** Decide whether REQ-01 and REQ-02 get an automated
   timing check or stay benchmark-evidenced. The benchmark alone doesn't
   prevent regressions.

## Known unrelated flake

Right after a rebuild, the first full `cargo nextest run -p jit` can time out
six `test_utils` and `cli_warnings_integration` tests at 20 s. A rerun passes.
The flake predates this work.
