# Handoff — Version 1.0 release (`b80e3c70`) — session 5

**Date:** 2026-07-30

**Session number:** 5

**Prior handoffs:** `handoff-4.md`, `handoff-3.md`, `handoff.md` in this directory

**Main baseline before this session:** `426fd7f6`

**Progress authority:** `dev/active/b80e3c70-v1-release/progress.json`

## Current state

- Waves 2 and 3 are complete. Children stand at 19 Done, 1 Rejected, 4 open.
- Open: `f9af9788` (Ready), `e90b9a3b`, `080e54c6`, `bb03df0a` (Backlog), plus the epic itself.
- Main is clean, `jit validate` passes, and every merge commit this session was verified to build in isolation.
- The epic remains assigned to `agent:jit-execution-lead`; its release sink remains intentionally unmet.

## Invoker decisions that bind later sessions

- **No remote interaction was authorized.** Main is now 400+ commits ahead of the public `origin`. Nothing was pushed. Every workflow this epic builds has still never executed on GitHub.
- **Release authority is the maintainer.** The invoker creates and pushes the annotated `v1.0.0` tag. The lead prepares the verified commit and verifies the published release afterwards.
- **Release verification runs in a clean rootless Podman container** with a fresh `HOME` and no repository access, so a globally installed `jit` cannot mask a missing artifact. `bb03df0a` REQ-06 also uses Podman; Docker is unavailable on this host.
- **Copyright holder is `erankavija`**, declared once in `[workspace.package]` and in both npm manifests, with `LICENSE-MIT` derived from that declaration rather than a hand-written literal.
- **Standing rule on successor-collapse findings:** when `code-review` attributes a finding to a successor issue that already carries the criterion, record the scope boundary and route the finding through `surfaced_pitfalls` without escalating each instance.

## Wave outcomes

- `e0fed1bb` made `ci.yml` callable through `workflow_call` and added the `callers.require_needs` contract grammar: a reusable workflow names the jobs a caller inherits, and the verifier requires each to exist, to carry no `if:`, and every caller job to reach the call through `needs`. A sibling local call is exempt. Review found `ci.yml` had **no repository-validation job at all** — `git grep 'jit validate' -- .github/` returned nothing across all five workflows — so rework added a `repo-validate` job at `fetch-depth: 0`. Depth 1 fails with exit 4 because document-reference commit pins have no object to resolve against.
  - Merge `2f22825d`; scope boundary `da35e723`; completion `d3e23cda`.
- `3a3b5704` deleted the five obsolete Docker definitions after committing smoke evidence for the replacement image, reduced `docker-compose.yml` to one `/repo`-mounted service, and cut `docker.yml` to a single `build-and-smoke` job with no registry path.
  - Merge `de4cbc86`; REQ-05 amendment `8fbc739a`.
- `22d374bd` added both complete license texts, `docs/release-notes/v1.0.0.md` written entirely by citation, and extended `release-version-contract.py` to derive the copyright holder from the manifest. Self-test is 28 cases.
  - Merge `b903ff2f`; boundary `b64a05c5`.
- `f7f80d53` split the audits into three named jobs — `cargo-audit`, `npm-audit-mcp-server`, `npm-audit-web` — so each boundary is individually nameable in `callers.require_needs`. `cargo-audit` is pinned to `0.22.2 --locked`, verified current against the crates.io index. The `npm audit --production` laxity routed from an earlier wave is closed.
  - Merge `fc4c1475`; boundary `dbb84fa9`; REQ-03 amendment `b14015cc`.
- `cdee58b3` added `scripts/rust-version-policy.py`, deriving the declaration from `cargo metadata` and current stable from the release-channel manifest at run time, with no stable literal anywhere. 22 unit tests. The `msrv` job now builds **and** tests `--locked` with the toolchain read from the script.
  - Merge `72861505`; rework merge `ba4a32df`.

## Verified preconditions still holding at session end

- `cargo audit -D warnings`: no advisories. `npm audit --omit=dev`: zero in `web/` and `mcp-server/`.
- Declared `rust-version = "1.97"` equals current stable `1.97.1`, so `bb03df0a` REQ-02 needs no refresh today. Re-derive it before tagging rather than trusting this line.

## Findings routed to `f9af9788` — close these before that issue completes

Three `code-review` findings were deferred to their owning issue. Recording them did **not** make them out of scope.

1. `release.yml` never calls `ci.yml`, so a tagged build can reach publication without the normal suites. Owner: REQ-02, REQ-09.
2. `release.yml` never calls `security-audit.yml`, so the three audit boundaries precede nothing. Owner: REQ-02, REQ-09.
3. The native archive carries only the two binaries, and the release body is hard-coded rather than derived from `docs/release-notes/v1.0.0.md`. Owner: REQ-03, REQ-05.

The contract grammar to bind all of these already exists. `f9af9788` declares the caller edges; it does not need new verifier machinery.

## What to do next

1. Dispatch `f9af9788` against current main. It owns `.github/workflows/release.yml` outright and deletes the superseded packaging workflow.
2. Verify all three routed findings are closed by its implementation before running its gates.
3. Then `e90b9a3b` → `080e54c6`, both documentation issues gated on `docs-mechanical` and the adversarial `doc-review`. Fan out disjoint-footprint auditors over the whole surface rather than batch-fixing reviewer findings.
4. Then `bb03df0a`, which needs the invoker: the push, the tag, and the clean-container verification.

## Traps — do not repeat these

- **Run every `jit` command from the main checkout.** A git worktree carries its own `.jit/` copy. Running `jit gate status` or `jit issue show` after a `cd` into a worker worktree reported every gate as "not run" while main had them all passed. This produced a false conclusion that amending a description clears gate evidence — it does not. Check `pwd` before any gate or status call.
- **A scope-boundary note does not always clear a successor-collapse finding.** It worked for `e0fed1bb`, whose criteria never named `release.yml`. It failed for `f7f80d53`, whose REQ-03 named the release workflow and publication directly; that needed a criteria amendment. Read the criterion's own wording before choosing the remedy.
- **Do not trust an exit-0 background notification as evidence a chained command ran.** A `python3 <<'PY' … PY` heredoc followed by `&& git commit && jit gate evaluate` reported success while the gate never executed; the gate's recorded run was still the previous one. Verify the gate's `last_run_at` moved.
- **Amend the issue before evaluating gates, not after.** Even though amending does not clear evidence, `code-review` reads the current description, so an amendment after a passing review leaves a review that never saw the amended contract.
- **Keep the incremental preflight in mind.** `cargo-ci` failed once for `3a3b5704` with every check green except `incremental-state`. Remove `target/debug/incremental` only after confirming it is the sole failure.
- **Do not let issue-impact review collapse successor work into its prerequisite.** Three instances this session, all on `release.yml`. Pre-empt it in the dispatch prompt by naming the owning issue.
- **Verify a version pin before accepting it.** `cargo-audit 0.22.2` was checked against `https://index.crates.io/ca/rg/cargo-audit` rather than assumed.

## Housekeeping the next session inherits

Eight git worktrees exist under `.agents/worktrees/`, each carrying a full build tree: `agent-e0fed1bb`, `agent-22d374bd`, `agent-3a3b5704`, `agent-f7f80d53`, `agent-cdee58b3`, `agent-a122b9b3`, `lead-install-clean`, and `steward-v1-readiness`. The five wave-2/3 branches are merged and their worktrees are reclaimable; `agent-a122b9b3` is preserved deliberately for forensics and must not be removed. `git worktree remove` was denied by this session's permission classifier, so the reclamation is outstanding.

## Reference artefacts

- Epic: `jit issue show b80e3c70`
- Next issue: `jit issue show f9af9788`
- Progress: `dev/active/b80e3c70-v1-release/progress.json`
- Prior handoff: `dev/active/b80e3c70-v1-release/handoff-4.md`
- Container cutover smoke evidence: `dev/active/8b05a612-production-readiness/3a3b5704-container-cutover-smoke.md`
