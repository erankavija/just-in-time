## Rules

- Develop test-first. Follow CLAUDE.md: layering, `@/inv/orthogonal-tests`, `@/inv/shared-test-contracts` (but no production wrapper that only tests use), terse comments, no backward compatibility, and no local variant of a shared mechanism (`@/inv/convention-convergence`).
- Read every success criterion literally. If one is impossible, stale against today's code, or conflicts with a project invariant, stop and report it under Needs decision; don't reinterpret it.
- Never change issue state, pass gates, write `.jit/`, or run any jit command that writes (including `jit recover` and `jit doc add`), and never act under another agent's identity. Report a doc path for the lead to link.
- Every cargo invocation (build, test, clippy, doc, run) runs with `CARGO_INCREMENTAL=0` in its own environment, for example `CARGO_INCREMENTAL=0 cargo doc ...`. Do not rely on an exported variable surviving a sandbox or subshell. Incremental state fills the shared disk (hundreds of GB per worktree).
- No test may sleep or wait on wall-clock time to observe a property; use explicit signals or injected clocks.
- Checks: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (zero warnings; the jit-server `web/dist` build-script notice is environmental), and targeted tests. Whole-suite runs take the host lock (`flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock <cmd>`). The lead's branch CI is authoritative.
- Report the production-line delta: commit, then run `python3 scripts/production-lines.py HEAD` and `python3 scripts/production-lines.py $(git merge-base main HEAD)`. The script reads each revision through `git show`, so never check out another revision to measure. Sum the modules.
- Commit on your branch with `jit:8fbf8c43` in the message, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Return

Your final message is the dispatcher's only input; write it for the next action.
Use these sections in order and write `None.` for an empty one:

1. **Outcome** — `done`, `partial`, or `blocked`, then the deliverables your
   assignment names (artifact paths, commit SHAs, counts, verdict).
2. **Needs decision** — each choice, escalation, or conflict the dispatcher must
   resolve, with the options and your recommendation.
3. **Deviations** — each departure from the assignment, each unmet criterion, and
   each failed or skipped check with its command and first failing line.

Omit passing checks, step narration, restated instructions, and anything the
written artifact or commit already records. Stay within 200 words unless a
deviation needs more evidence.

Deliver the message through the reporting channel the dispatch names. When the
harness hides a background agent's final output from its dispatcher, send it
with the harness messaging tool.
