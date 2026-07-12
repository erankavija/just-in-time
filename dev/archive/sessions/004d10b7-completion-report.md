# Epic 004d10b7 — Documentation contract follow-ups: completion report

**Epic:** 004d10b7 — Documentation contract follow-ups
**Outcome:** Complete. All 10 children done, all 3 epic gates passed.
**Milestone:** direct dependency of 9db27a3a (Version 1.0 Production Release).

## Success criteria

| Criterion | Delivered by |
|---|---|
| REQ-01 — each of the ten filed follow-ups is a direct dependency of this epic and retains an independently verifiable completion gate | All 10 are direct dependencies and reached `done` through their own gates (`cargo-ci`, `code-review`, and where applicable `doc-review`, `docs-mechanical`, `mcp-ci`). No gate was waived. |
| REQ-02 — resolve the audit-recorded contract gaps across CLI schema, event and storage semantics, validation, toolchain policy, gates, and runtime defaults | CLI schema: 623f3163. Event semantics: 2c66e9f7. Storage semantics: 562e977c. Validation: afbbf6c3. Toolchain policy: e86b32d4. Gates: 9e0abec5. Runtime defaults: ed049e1d. Plus exit codes (3ac07340), the stale merge-driver entry (7a60f987), and issue-create help (6ad894cb). |
| REQ-03 — this epic is a direct dependency of the v1.0 milestone | Confirmed: `jit graph downstream 004d10b7` reports 9db27a3a. |

## What shipped

Five new generated references, each projected from the code that defines the fact
and each guarded by a golden byte-equal conformance test, so the committed page
cannot drift from the source:

| Reference | Projected from |
|---|---|
| `docs/reference/runtime-defaults.md` | `crates/jit/src/runtime_defaults.rs` |
| `docs/reference/exit-codes.md` | `crates/jit/src/schema.rs` (`command_exit_codes`) |
| `docs/reference/events.md` | `crates/jit/src/domain/event_catalog.rs` |
| `docs/reference/gate-presets.md` | `crates/jit/src/gate_presets/reference.rs` |
| `docs/reference/storage-records.md` | `crates/jit/src/storage/reference.rs` |

Behavioral work: the validation `strictness` key became a real enforcement
modulator (afbbf6c3); the MSRV was declared and enforced in CI (e86b32d4); the
stale claims merge-driver entry was removed (7a60f987).

Hand-maintained duplicates of these facts were removed across the docs and
replaced by citations (`@/inv/single-source-prose`) — including eleven exit-code
tables in `claim.md` and `worktree-validate.md`, a preset list in
`custom-gates.md` that enumerated only five of the eight shipped presets, and the
event and storage restatements in `storage-format.md`.

## Product defects the projections exposed

Binding documentation to runtime — rather than writing it by hand — turned each
projection into a test of the claim it made. That found six real defects:

1. **`jit validate <short-id>` exited 3 for every issue.** `run_rules` and
   `explain_rules` passed the caller's id straight to `load_issue`, which keys on
   the full UUID, so only the full id resolved though every other command accepts
   a prefix. Fixed under 6ad894cb (owner-approved), with regression tests that
   fail without the fix.
2. **The `apply`/4 exit-code row was wrong twice over.** The template cycle guard
   maps to a bare `anyhow!`, so the classifier yields 1, not 4 — and a template
   delta cannot close a cycle by construction (anchor edges only run
   anchor→node; a node's `depends_on` names nodes only). The row was removed.
3. **`snapshot export` exit 6 was undocumented.** `--out <existing>` raises
   `AlreadyExistsError`; no row covered it.
4. **The rule-rejection exit-4 row over-claimed its scope.** It was marked `*`
   ("every command"), but `validate_for_write` is reachable only from commands
   that write an issue.
5. **The event catalog's freshness guard was circular.** It compared
   `EventTag::ALL` against `EventTag`'s own derived schema, so a new `Event`
   variant reusing an existing tag would have serialized an uncataloged `type`
   while still compiling. The guard now reads the variant tags off the schema
   schemars derives from `Event` itself.
6. **Full-id resolution is narrower than it looked.** `resolve_issue_id`
   normalizes the input to *measure* it but takes the full-id fast path with the
   original string, so a hyphenless or uppercased full UUID does not resolve.
   Documented as implemented, and pinned by a test.

## Metrics

- Children completed: 10 of 10 (0 rejected)
- Waves: 4 planned; waves 3 and 4 partially collapsed once no unmet dependencies remained
- Rework cycles: 623f3163 ×1, afbbf6c3 ×1, ed049e1d ×2 (then lead take-over), 3ac07340 ×2 (then lead take-over)
- Lead take-overs: 2 (ed049e1d, 3ac07340) — both after the rework ceiling, both owner-approved
- Escalations: 3 (validation strictness semantics; ed049e1d review non-convergence; the `jit validate` resolver bug)
- Gates evaluated: every child gate plus 3 epic gates. No gate bypassed, removed, or waived.

## Autonomous decisions

- **Wave ordering.** 562e977c was held back from the final parallel dispatch even
  though its dependencies were met: its REQ-02 requires *linking* to the event
  contract that 2c66e9f7 creates, so it could not be written until that page
  existed.
- **Integration method.** Worker branches were integrated by taking their source
  and doc files rather than cherry-picking whole commits, because the workers
  bundled `.jit` state that would have clobbered lead-side issue state. Doc links
  were re-added with `jit doc add` on main.
- **Documenting rather than changing behavior.** Where a projection found the
  product's behavior surprising but not broken (the `apply` cycle guard, the
  reserved `config validate` exit 2, full-id resolution), the reference states
  what the code does, and the underlying oddity is recorded below rather than
  silently "fixed" outside the issue's scope.

## Follow-up work (not filed — needs a home)

The reviewers surfaced four semi-dead code paths. They are engine gaps rather
than documentation-contract work, so they do not belong under this epic:

- `startup_recovery()` has no production caller.
- The auto-heartbeat interval is a config default with no production scheduler;
  `jit claim heartbeat` records a single beat.
- `config validate`'s `exit(2)` warnings branch is unreachable — `result.warnings`
  is never populated. The exit-code reference documents the row as *reserved*.
- `apply`'s `validate_delta_acyclic` guard is unreachable, and would exit 1 rather
  than the 4 its (now removed) documentation claimed.
