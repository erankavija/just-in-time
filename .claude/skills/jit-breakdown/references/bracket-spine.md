# Bracket spine wiring (Step 6c-bracket)

For a **breakable** parent with an approved plan (Step 1.5 selected bracket
breakdown), splice the source/sink spine `C → impl → B → P` around the
**pre-created** breakdown node `B`. The breakdown node and its gates were
created by `jit apply plan <C>` — breakdown CONSUMES `B`, it does not create it.
Use the in-process engine path when available, or the CLI primitives below.

**1. Locate the pre-created breakdown node `B` and the planning node `P`.** `B`
is the dependency of `C` typed `<breakdown_type>` that carries the
`brackets:<C-short-id>` label the apply engine seeds; it already carries both of
its gates (`coverage-preview` + `breakdown-review`) and already depends on `P`
(`jit apply plan` wired `B → P`). Find `P` through `B`:

```bash
jit issue show <C> --json | jq -r '.depends_on[]'
# B is the dependency typed <breakdown_type> carrying brackets:<C-short-id>
jit issue show <B> --json | jq -r '.depends_on[]'
# P is B's dependency typed <planning_type>
```

If no such `B` exists, the container was never scaffolded — STOP and run
`jit apply plan <C>` first. Do NOT create `B` by hand or re-attach its gates.

**2. Wire sources → `B`.** A *source* child has an empty `depends_on` (no
intra-subgraph predecessor). Every source depends on `B`, gating all impl behind
the approved breakdown:

```bash
# For each source child:
jit dep add <source-child-UUID> <B-UUID>
```

**3. Wire `C` → sinks.** A *sink* child has no sibling listing it in `depends_on`
(no intra-subgraph successor). The container depends on each sink:

```bash
# For each sink child:
jit dep add <C-UUID> <sink-child-UUID>
```

`jit` keeps the DAG transitively reduced, so the scaffold's direct `C → B` anchor
edge and any redundant `C → non-sink` edge are dropped automatically once the spine
connects `C` to the sinks. Verify with
`jit issue show <C> --json | jq .depends_on` (should list sinks only).

**4. Run the coverage-preview gate via the standard runner, then block on it.**

The breakdown spine-splicer (steps 1–3) never runs, stamps, or fabricates a
verdict on `B`'s pre-attached gates (`BracketBreakdownResult` is purely
structural: `container_id`, `breakdown_id`, `planning_id`, `child_ids`,
`coverage_gate_preset`, `breakdown_review_gate_preset`). Running a gate is a
breakdown-workflow step, executed by the standard gate runner exactly like every
gate in this project — never by command code. The deterministic coverage-preview
gate is cheap, so RUN it inline here for immediate feedback (the agent
breakdown-review gate is left for the runner, like `plan-review` on `P` — see the
note at the end of this step):

The concrete gate names in this doc (`coverage-preview`, `breakdown-review`,
`plan-review`) are the DEFAULT ruleset's; substitute the template's recorded preset
names in the commands below when a ruleset differs (the `<coverage-gate>` placeholder
marks where the discovered name goes).

```bash
jit gate pass <B-UUID> <coverage-gate>   # the breakdown node's coverage gate from the template; coverage-preview in the default ruleset
```

This is the standard auto-gate execution path: it runs the checker
(`jit validate --scope <C>`, resolved from `B`'s `brackets:<C-id>` label),
persists a `GateRunResult`, updates `B`'s gate status, and logs the gate event.
For coverage to register, each child must carry the `satisfies:<id>` label(s) for
the `[hard]` criteria it covers (pass them via the child's `labels`, or
`jit issue update <child> --label satisfies:<id>` when wiring by hand).

**GATE the fan-out on the recorded result** (check the recorded gate *status*,
not just the exit code — the project convention):

```bash
jit gate check <B-UUID> <coverage-gate> --json | jq -r .status   # coverage-preview in the default ruleset
```

- **Coverage-preview FAILS** (a `[hard]` criterion uncovered → exit 4, status
  `failed`): a `[hard]` criterion is left uncovered by the drafted children (the
  criterion id appears in the gate-run output). Do NOT release the impl children.
  Surface the findings, add the missing `satisfies:<id>` label(s) (or revise the
  plan/draft), and re-run `jit gate pass <B> <coverage-gate>` until it
  passes.
- **Coverage-preview PASSES** (status `passed`): the deterministic *coverage* half
  is satisfied. This does **not** by itself approve the breakdown or release the impl
  children — `B` still carries the PENDING `breakdown-review` gate, so it stays
  `Gated` (not `Done`) and the children stay blocked. `B` reaches `Done` and releases
  the fan-out only once **both** gates pass.

So the explicit sequence is: **splice the spine around the pre-created `B` → run
coverage-preview inline → block on its fail; the runner separately passes
breakdown-review → impl releases only when `B` is `Done` (both gates passed).**
