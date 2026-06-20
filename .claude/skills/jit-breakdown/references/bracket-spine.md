# Bracket spine wiring (Step 6c-bracket)

For a **breakable** parent with an approved plan (Step 1.5 selected bracket
breakdown), create `B` and splice the source/sink spine `C → impl → B → P`. Use
the in-process engine path when available, or the CLI primitives below.

**1. Create the breakdown node `B`.** It carries the configured breakdown type, a
`brackets:<C-id>` label, and the container's membership label (NOT its type):

```bash
jit issue create \
  --title "Breakdown: <container title>" \
  --label "type:<breakdown_type>" \
  --label "brackets:<C-UUID>" \
  --label "<membership-label>"
# capture B-UUID
```

**2. Attach BOTH of `B`'s gates and wire `B → P`:**

```bash
jit gate preset apply <coverage_gate_preset> <B-UUID>          # deterministic coverage-preview gate
jit gate preset apply <breakdown_review_gate_preset> <B-UUID>  # agent breakdown-review gate
jit dep add <B-UUID> <P-UUID>                                  # B depends on approved plan
```

(The in-process engine path attaches both automatically; apply both here only when
wiring `B` by hand with CLI primitives.)

**3. Wire sources → `B`.** A *source* child has an empty `depends_on` (no
intra-subgraph predecessor). Every source depends on `B`, gating all impl behind
the approved breakdown:

```bash
# For each source child:
jit dep add <source-child-UUID> <B-UUID>
```

**4. Wire `C` → sinks.** A *sink* child has no sibling listing it in `depends_on`
(no intra-subgraph successor). The container depends on each sink:

```bash
# For each sink child:
jit dep add <C-UUID> <sink-child-UUID>
```

`jit` keeps the DAG transitively reduced, so the scaffold's direct `C → P` edge
and any redundant `C → non-sink` edge are dropped automatically. Verify with
`jit issue show <C> --json | jq .depends_on` (should list sinks only, not `P`).

**5. Run the coverage-preview gate via the standard runner, then block on it.**

The bracket-builder (step 1–4) only ATTACHES `B`'s gates — it leaves both PENDING
and never runs, stamps, or fabricates a verdict (`BracketBreakdownResult` is purely
structural: `container_id`, `breakdown_id`, `planning_id`, `child_ids`,
`coverage_gate_preset`, `breakdown_review_gate_preset`). Running a gate is a
breakdown-workflow step, executed by the standard gate runner exactly like every
gate in this project — never by command code. The deterministic coverage-preview
gate is cheap, so RUN it inline here for immediate feedback (the agent
breakdown-review gate is left for the runner, like `plan-review` on `P` — see the
note at the end of this step):

```bash
jit gate pass <B-UUID> <coverage_gate_preset>   # e.g. coverage-preview
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
jit gate check <B-UUID> <coverage_gate_preset> --json | jq -r .status
```

- **Coverage-preview FAILS** (a `[hard]` criterion uncovered → exit 4, status
  `failed`): a `[hard]` criterion is left uncovered by the drafted children (the
  criterion id appears in the gate-run output). Do NOT release the impl children.
  Surface the findings, add the missing `satisfies:<id>` label(s) (or revise the
  plan/draft), and re-run `jit gate pass <B> <coverage_gate_preset>` until it
  passes.
- **Coverage-preview PASSES** (status `passed`): the deterministic *coverage* half
  is satisfied. This does **not** by itself approve the breakdown or release the impl
  children — `B` still carries the PENDING `breakdown-review` gate, so it stays
  `Gated` (not `Done`) and the children stay blocked. `B` reaches `Done` and releases
  the fan-out only once **both** gates pass.

So the explicit sequence is: **create bracket → run coverage-preview inline → block
on its fail; the runner separately passes breakdown-review → impl releases only when
`B` is `Done` (both gates passed).**
