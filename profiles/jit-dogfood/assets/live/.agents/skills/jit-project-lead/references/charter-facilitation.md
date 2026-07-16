# Charter facilitation

The steward-owned step that creates and evolves the vision charter with its owner.
It is the body of **mode 3** (steering discussion) and the **charter step of
mode 2** (cold-start goal): the steward facilitates the vision and its
consequential decisions directly, and every outcome lands in the durable charter
(`references/vision-charter.md`). No planning skill and no worker is involved here;
this step owns the charter, nothing below it.

Run it after the charter path is resolved (`references/vision-charter.md`
Location) and read back if it already exists (that reference's Read-back on
resume). It is a referenced section the front door invokes; it does not run the
front door or the dispatch loop.

## Posture selection (from entry state)

Pick one posture from the state at entry, before facilitating:

- **Coached elicitation** — the vision is unstated: no charter exists, or it
  exists with an empty or placeholder vision, and the repository carries little to
  derive from (few or no containers, issues, or code). The owner holds the vision
  in their head; the steward draws it out.
- **Bootstrap from repository state** — project artifacts already exist: a
  populated work graph (containers, issues), committed code, `AGENTS.md`, config,
  and invariants. The vision is implicit in what has been built; the steward
  derives a draft and the owner ratifies it.

Read the entry state to choose: does the charter exist with a real vision, and do
containers/issues/code exist? Thin state routes to coached elicitation; existing
project artifacts route to bootstrap. When the two signals genuinely conflict
(e.g. a rich repo but the owner signals the recorded direction is stale), ask the
owner which posture fits rather than guessing.

Either posture writes through the same Write-back rule below.

## Coached-elicitation posture

The vision is unstated; facilitate it into being with the owner.

1. **Settle the vision first.** Draw out two to five sentences: what the project
   is for, the outcome it must deliver, and the standard by which a sub-strategic
   container's work is judged coherent with it. This is the yardstick the steward
   later cites when accepting a container or resolving an escalation. Land it as
   the charter's Vision.
2. **Walk the consequential decisions one at a time.** For each direction the
   owner is settling, surface the real alternatives, name what is chosen and what
   is rejected and why, and confirm it with the owner. Log each as its own `D-N`
   the moment it is settled (Write-back rule). One decision per entry; a decision
   with no genuine rejected alternative is not consequential and stays out.
3. **Stop when the owner has no further direction to settle.** The charter now
   holds the vision plus every decision reached this session.

## Bootstrap-from-repository-state posture

Project artifacts already exist; derive a draft, then the owner ratifies it.

1. **Derive a draft vision** from repository state: `AGENTS.md`, the strategic
   container's description (`jit issue show <container-id>`), the done containers
   (`jit query strategic --json`), config (`.jit/`), and the domain invariants.
   Keep it to the two-to-five-sentence yardstick shape.
2. **Mine candidate decisions** from the same state: consequential calls the
   project has already made, each with a genuine rejected alternative. Verify each
   against the repo (git history, config, done containers) before proposing it — a
   candidate with no real alternative is dropped.
3. **Present the draft for owner ratification.** The owner ratifies, edits, or
   rejects the vision and each candidate decision. Only ratified content lands.
   The steward never self-ratifies a bootstrapped draft: an owner ratifies the
   vision (see Stop and escalate).
4. **Land additively.** Ratified vision text and each ratified decision land per
   the Write-back rule. Updates to an existing charter are additive: a new `D-N`
   supersedes an earlier one by citing it; a landed entry is never overwritten.

## Write-back rule

Every facilitation outcome lands in the charter, and nowhere else:

- **Vision** outcomes land as the charter's Vision text (two to five sentences).
- **Decision** outcomes land as a new append-only `D-N`: one summary bullet under
  `## Decision Log` (the addressable `- D-N: <one-liner>` row) and the full entry
  (chosen / rejected / reasoning / date) under `## Decision Details`, exactly as
  `references/vision-charter.md` prescribes.
- **Supersession**: to overturn a landed decision, add a new `D-N` that cites and
  supersedes the old one. A landed entry is never edited in place and the log is
  never renumbered.
- Ids are append-only, continuing from the highest existing `D-N`. `Rejected` is
  never empty for a logged decision.

After writing, the charter stays linked to the strategic container per
`references/vision-charter.md` Linkage.

## Stop and escalate

- The charter path cannot be resolved, or an existing charter cannot be parsed
  into a vision plus zero-or-more `D-N` entries — stop rather than overwrite (see
  `references/vision-charter.md` Stop and escalate).
- A bootstrapped draft has no owner available to ratify it. Do not self-ratify and
  do not land an unratified vision; report that ratification is pending.
- The facilitation surfaces a vision-level conflict the owner must resolve (two
  incompatible directions for what the project is for). Record the open question
  and stop; this is the owner's call, not the steward's.

## Red flags

- Handing steering or the charter step to a planning skill. This step is
  steward-owned; the charter is the steward's artifact and mode 3 dispatches no
  worker.
- Reimplementing planning here. This facilitates the vision and its decisions, not
  the container's plan or breakdown. Mode 2 hands the scoped goal to
  `jit-planning-lead` only after the charter step; mode 3 stops at the charter.
- Self-ratifying a bootstrapped draft. A derived vision is a proposal until the
  owner ratifies it.
- Writing an outcome anywhere but the charter. Every settled vision or decision
  lands as Vision text or an append-only `D-N`; a decision left only in the
  conversation is lost.
- Editing or renumbering a landed `D-N`, or logging a decision with an empty
  `Rejected`. Supersede with a new citing entry; drop non-consequential calls.
