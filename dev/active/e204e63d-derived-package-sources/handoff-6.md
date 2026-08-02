# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 7

**Date:** 2026-08-02
**Session number:** 7
**Prior handoffs:** `handoff.md` (1), `handoff-2.md` (2), `handoff-3.md` (3), `handoff-4.md` (5),
`handoff-5.md` (6), same directory. Every trap in every prior handoff remains in force unless a trap
here records its resolution.

## Current state

- Epic `e204e63d` — state: backlog. **78 issues** carry the epic label (one filed this session).
  **51 done** (42 at session start), 21 backlog, 5 ready, 1 in_progress. `jit validate` clean.
- **Wave 2 is closed.** Ten issues closed this session: `fd61b44f`, `0db28190`, `4873b32f`,
  `6c47b99a`, `5d12a79f`, `8371bd46`, `7835fdc0`, `7d038e97`, `d35cc3f2`, `e4ba28c5`.
- `6f8f02ba` is the only issue still `in_progress`; it holds no work of its own and closes behind
  `daddfc0b`.
- Progress file: `progress.json`, same directory.

## What this session did

- **Reclaimed 25 merged worktrees, freeing 68 GB** (/ went 92% → 84%). Do this at every session
  start; each worktree carries a full build tree.
- **Landed wave 2** — six issues dispatched at once, three to Opus agents and three to codex
  `gpt-5.6-luna` at high effort.
- **Delivered the profile-package capability end to end.** It only works as a unit and it all landed
  together: a package is assembled from the repository files it mirrors (`7d038e97`), resolved from a
  supplied location or the repository's own record (`5d12a79f`), applied together with the packages it
  depends on in topological order (`7835fdc0`), and repaired by re-reading every recorded package
  from the location its record names (`8371bd46`).
- **Filed `d35cc3f2` mid-session** on an owner ruling, after establishing that `e4ba28c5`'s reduction
  broke `jit init --profile jit-dogfood` for anyone on a released binary.
- **Found and fixed a latent defect in composed initialization.** `initialize_fresh_repository`
  captured the layout before publishing and then applied the rest of the closure against that stale
  capture. Unreachable on `main` until a package declared a dependency, so every gate passed over it.

## What to do next

- [ ] **Reclaim worktrees first.** This session created seven and closed ten issues; each worktree
      carries a full build tree. Last time this freed 68 GB.
- [ ] Wave 3 is unblocked and mostly independent. `ready` now: `d94f6849`, `ebbb254f`, `2ce8ce1b`,
      `d54f8f83`, `959274b6`. `26f503cc`, `9de22aa7` and `590ff4db` unblock as their predecessors
      close. Nothing is waiting on a decision.
- [ ] Still unfiled after three sessions, both worth filing as dogfooding friction: `jit issue create`
      has no `--description-file` (unlike `jit issue update`, which does), and `jit gate` cannot
      evaluate one gate across several issues in one command.

## Traps — do not repeat these

All prior traps remain in force; read every earlier trap section. New this session:

- **RESOLVED — the dirty-install trap no longer covers tracker data, and carrying it forward is pure
  churn.** Sessions 1, 4 and 5 all recorded "do not leave the tree dirty when a pipeline will
  install"; session 5 lost a pipeline to an uncommitted `progress.json`. `32779829` REQ-11/REQ-12
  fixed the rule and session 6 said to confirm before dropping the habit. **Confirmed:**
  `scripts/install-jit.sh:68` computes the flag as
  `git status --porcelain -- "${build_input_paths[@]}"`, scoped to the nine paths declared in
  `crates/jit/src/domain/binary_build_inputs.txt` (`Cargo.toml`, `Cargo.lock`,
  `crates/jit/{Cargo.toml,Cargo.lock,build.rs,src}`, `profiles/jit-dogfood`, and the two
  `scripts/hooks/` files). `.jit/**` and `dev/**` are not build inputs, so uncommitted gate records,
  issue JSON or `progress.json` cannot set `dirty=true`, and a `.jit`/`dev` commit does not make the
  installed binary stale. **Still true:** a dirty tree *within those nine paths* marks the binary
  stale for its whole life. Commit tracker data when it is worth committing, not to satisfy the
  installer. A trap with an expiry condition needs the condition checked, not the trap obeyed forever.
- **Do NOT run `code-review` before an issue's mechanical gates.** It reads the issue's RECORDED GATE
  EVIDENCE, not only the diff. Running it first cost two review rounds: `0db28190`'s REQ-04 ("the
  mechanical documentation checks pass") was failed while `docs-mechanical` sat queued behind the
  reviewer and passed twelve seconds later; `fd61b44f`'s REQ-03 was failed for having one `cargo-ci`
  execution on record. Both then passed **unchanged**. The session copy of `merge-and-gate.sh` now
  sorts `code-review` last.
- **Do NOT expect a re-run to produce a second gate execution.** `32779829`'s verdict reuse means
  `jit gate evaluate` over an unchanged digest reuses instead of executing, so a criterion demanding
  *consecutive runs* cannot be evidenced by running the gate again. Pass `--force`. The efficiency win
  and the evidence requirement pull against each other.
- **Do NOT assume evidence you gathered counts.** Two whole-workspace runs in the lead's terminal did
  not satisfy `fd61b44f`'s reviewer, correctly: it reads `.jit/gate-runs`, not the lead's scrollback.
- **Do NOT relay a reviewer's finding site-by-site — sweep the whole surface first.** `7d038e97` spent
  three rounds on one false claim living in four files (module doc, script header, CHANGELOG entry,
  test doc comment, plus the test's own name). The reviewer names one instance per round and has no
  memory across rounds; supplying that memory is the lead's job under Tier 2.5. This hit both
  `MAX_REWORK_ATTEMPTS` and `MAX_SAME_FINDING_REPEATS` and forced an escalation one sweep would have
  avoided.
- **Do NOT name only the invariant in a brief when a primitive exists.** `7d038e97`'s brief cited
  `@/invariant/atomic-writes`; the worker satisfied its own reading with check-then-rename, a TOCTOU
  the reviewer caught. `crates/jit/src/storage/external_publish.rs` already exposed
  `publish_external_directory_noreplace` over `renameat2`/`RENAME_NOREPLACE`. **Name the function.**
- **A worker citing CODE PATHS is not evidence for a criterion about an OUTCOME.** `d35cc3f2` cited
  `init.rs:93-97` as satisfying "initialization succeeds"; building the binary and running
  `jit init --profile jit-dogfood` showed it failing and applying only half the configuration.
  When a criterion names an outcome, require the command and its observed result.
- **A test edited to supply what the default path lacks is masking a regression, and the tell is in
  the diff.** `e4ba28c5` changed four `location: None` call sites to a supplied location, which made
  its tests pass while `jit init --profile jit-dogfood` stayed broken for adopters. The lead read
  those edits as scope creep, checked they were inside `mod tests`, and moved on — the regression was
  caught separately by running the real command. `-location: None` / `+location: Some(...)` in a diff
  is a question, not a detail.
- **Do NOT let a decision amendment orphan the leaf criteria written against it.** Session 6 amended
  D-20 and delivered it through `c7058cac`, but `e4ba28c5`'s REQ-04 still named the item kind the
  amendment had removed — unsatisfiable, caught at dispatch only because the two manifests were
  measured. After any decision amendment, re-read every undispatched leaf citing that decision.
- **Do NOT trust a worker's "outside this issue's scope" list without reading it.** `e4ba28c5`
  reported 14 suite failures as out of scope; **two were the tests it had just written for this
  issue**, and seven more were its own change's consequence.
- **A fact repeated in N places goes stale in N places.** Three separate instances this session:
  `7d038e97`'s destination claims across four files, `5d12a79f`'s resolution order across two
  reference pages, and its enumeration rule and failure sentence on top. Brief documentation-heavy
  issues to pick a canonical home up front.
- **`docs-mechanical`'s M6 check is named "canonical homes" and does NOT catch this.** It passed
  before and after `5d12a79f`'s duplication existed. Only the adversarial reviewer caught it.
- **RULING — `@/invariant/single-source-prose` does not reach an API doc comment on the implementing
  symbol.** `resolve_profile_package`'s doc comment states the resolution order and stays: it sits on
  the function implementing it, so it is bound by proximity, not maintained across a distance;
  AGENTS.md separately requires public APIs to document their contracts; and making a Rust doc comment
  cite an adopter page would invert the dependency direction. The reviewer's own scope agreed.
- **`setsid nohup` codex workers notify NOTHING.** Agent-tool workers send idle pings and backgrounded
  Bash notifies on exit, but a detached codex dispatch is silent. Attach a watcher at dispatch time:
  `until [ -f "$SP/logs/<id>.last.md" ] && [ "$SP/logs/<id>.last.md" -nt "$SP/briefs/<id>.md" ]; do
  sleep 15; done`, run with `run_in_background`. Two codex workers ran unwatched this session.
- **Do NOT pipe `dispatch-worker-worktree.sh` through a filtering grep.** It correctly refused to
  create a worktree over a dirty tree, the grep discarded the error, and the codex dispatch that
  followed died instantly against a nonexistent directory. Read its output.
- **To kill a codex worker,** match `/proc/*/cmdline` on both `codex` and the worktree name and send
  SIGTERM; confirm by watching the log stop growing rather than by process listing (the processes
  linger briefly as they exit). Then discard the worktree's uncommitted partial edit — a killed
  worker's half-applied change is not a starting point — and stop any watcher waiting on its
  `last.md`, which will never be written.
- **`7835fdc0`'s own issue Notes are wrong** and would have cost a round: they say "the graph layer
  already owns that shape" for topological sorting. It does not — `find_keyed_cycle` exists,
  `validate_dag` checks acyclicity over issue-shaped nodes, and no topological sort existed. The brief
  corrected it and the worker added `keyed_topological_order` beside `find_keyed_cycle`.

## Owner rulings and preferences recorded this session

Full text in `progress.json` under `owner_rulings`. In brief:

1. `e4ba28c5` REQ-03 reads as a disjointness test, not as composed application (later superseded in
   practice: once `7835fdc0` landed, the round-2 test compares both routes, which is strictly more).
2. `e4ba28c5` REQ-04 restated as a zero-kind guard, the old wording having been orphaned by D-20.
3. `7835fdc0` pulled forward into wave 2 so `e4ba28c5` could merge.
4. `7d038e97` given a fourth attempt with a fully swept site list after hitting both rework thresholds.
5. The embed gap covered by embedding `jit-default` too, filed as `d35cc3f2`.

**Working preferences the invoker stated, which are not in the skill:**

- **Gate one issue at a time** and report per issue. This costs nothing: verdict reuse means the
  expensive part is *merging* several issues before any gating, not gating them separately. Merging
  a batch is still fine — say so explicitly and still report per issue.
- **Choose the worker model case by case, not by rule.** Codex `gpt-5.6-luna` for first attempts on
  clearly-specified work and for reworks whose sites and correct end state are both named. Opus when
  the work needs diagnosis, generalizing past the named instance, or establishing an outcome. State
  which and why at each dispatch.

## Reference artefacts

- Epic: `jit issue show e204e63d` — nine live criteria (REQ-06 retired).
- Plan: `e204e63d-plan.md`; manifest: `e204e63d-breakdown.json` — authoritative, but note `32779829`
  (session 6) and `d35cc3f2` (session 7) were added to the graph on owner rulings and are not in it.
- Boundary audit: `dev/active/7cbefe7c/findings.md` — **S4's reachability claim is disproven.**
- Session tooling (scratchpad, NOT committed — copy forward from the newest session directory):
  `merge-and-gate.sh` (now sorting `code-review` last), `dispatch-codex.sh`, `dispatch-wave.sh`,
  `mkbrief.py`, `ws-runs.sh`, `briefs/`, `logs/`.
