# Hosted continuous-integration evidence (f3f7de97, REQ-08)

> **Diátaxis Type:** Reference (evidence record)
> Recorded from the GitHub Actions API on 2026-08-05 for the push of `main` at
> `7b6e0160`, this container's last commit carrying anything the workflow runs
> on. Every field below is read from `gh run view <id> --json name,jobs`;
> nothing is restated from memory.

## What REQ-08 asks

`@/issue/f3f7de97/requirement/REQ-08` asks that the hosted continuous-integration
workflow complete with every job passing, and that no job reach its execution
ceiling. Two facts are needed: a conclusion per job, and a duration per job
against the ceiling that applies to it.

## The ceiling that applies

Every job in this repository's workflows declares `runs-on: ubuntu-latest` and,
except for `docker.yml`, no `timeout-minutes`. A GitHub-hosted job with no
declared timeout runs under the platform's own six-hour job ceiling, so 360
minutes is the bound each duration below is measured against. `76a4bd21` was
filed because four jobs of one workflow had previously run to exactly that
ceiling.

## The runs

| Workflow | Run | Conclusion |
| --- | --- | --- |
| CI | [30987011841](https://github.com/erankavija/just-in-time/actions/runs/30987011841) | success |
| Container Image | [30987011696](https://github.com/erankavija/just-in-time/actions/runs/30987011696) | success |
| Deploy Rustdoc | [30987012456](https://github.com/erankavija/just-in-time/actions/runs/30987012456) | success |

All three were queued at 2026-08-05T07:57:03Z by the push of `7b6e0160`. The
push before it, `b94bf7e2`, carried this container's last change to a compiled
source and was green across the same three workflows
([30985528861](https://github.com/erankavija/just-in-time/actions/runs/30985528861),
[30985528840](https://github.com/erankavija/just-in-time/actions/runs/30985528840),
[30985528852](https://github.com/erankavija/just-in-time/actions/runs/30985528852)).

Security Audit is absent by design rather than by omission. Its `push` trigger is
filtered to `**/Cargo.lock` and `**/package-lock.json`, and this push changed
neither, so no run was created. It last ran on this container's work at
[30979416166](https://github.com/erankavija/just-in-time/actions/runs/30979416166)
and passed all three of its jobs.

## Every job, with its duration against the ceiling

| Workflow | Job | Conclusion | Duration | Share of the 360-minute ceiling |
| --- | --- | --- | --- | --- |
| CI | Test Rust Components | success | 11.5 min | 3.2 % |
| CI | MSRV (build and test on declared rust-version) | success | 7.9 min | 2.2 % |
| CI | Test Rust (html + xml features) | success | 7.1 min | 2.0 % |
| CI | Coverage Badges | success | 6.2 min | 1.7 % |
| CI | Test MCP Server | success | 4.0 min | 1.1 % |
| CI | Validate Repository Data | success | 1.8 min | 0.5 % |
| CI | Profile adoption (Linux) | success | 1.6 min | 0.5 % |
| CI | Test Web UI | success | 0.9 min | 0.3 % |
| CI | Workflow Contract | success | 0.2 min | 0.1 % |
| Container Image | Build and Smoke the Server Image | success | 8.4 min | 2.3 % |
| Deploy Rustdoc | build-and-deploy | success | 2.7 min | 0.8 % |

Eleven jobs, eleven successes, no cancellation and no timeout. The longest took
11.5 minutes, which is 3.2 % of the ceiling; the whole set of three runs finished
inside 13 minutes of wall clock.

`Validate Repository Data` is the job that reads `.jit/`. It ran on this tree and
passed, so the container's tracker state is covered by the hosted workflow and
not only by the local gate that runs the same command.

## What changed to make this possible

Three children of this container produced the difference, and each is visible in
the table above.

- `76a4bd21` bounded the foreground-serve case. The last CI run before this
  container's work reached the hosted workflow,
  [30884326921](https://github.com/erankavija/just-in-time/actions/runs/30884326921),
  ran `Test Rust Components`, `Test Rust (html + xml features)`, `MSRV` and
  `Coverage Badges` for 5.36 hours each before GitHub cancelled all four; the
  run's own conclusion is `cancelled`. The rows above are those same four jobs
  at 11.5, 7.1, 7.9 and 6.2 minutes.
- `ee02e514` removed the Windows leg after the owner ruled Windows out of scope.
  In that same run, `Profile adoption (windows-latest)` failed while
  `Profile adoption (ubuntu-latest)` passed. Its two defects (`25d25f2f`,
  `03566554`) were rejected rather than fixed, and the workflow no longer runs a
  platform whose failures nobody acts on. The surviving `Profile adoption
  (Linux)` row is what remained.
- `3019eacd` replaced the merge-integrity guard that reported a cold workspace
  build as passing implausibly fast, so a merge whose result does not build and
  test can no longer reach this workflow claiming it does.

## What this record does not establish

One green set of runs is evidence that the workflow passes on this commit, not
that it passes on every future one. The claim is bounded to `7b6e0160`.

This record was anchored twice before and wrong both times, each caught by the
container's own holistic review. It first named `33c89c44` and claimed the
commits after it touched no compiled source; `36fc326c` and `84bb42f3` —
`91cc038c`'s fix and its rework — had changed
`crates/jit/src/storage/contention_probe.rs`, `claim_coordinator.rs` and
`lock.rs`. Re-anchored to `b94bf7e2`, it then argued that the `.jit/` commits
after that were covered by the local `repo-validate` gate running the same
command as the hosted job. The same command is not the same run, and the review
declined it. Hence a hosted run on the tree that carries them.

## The commits after the run, and what covers them

A container cannot get a hosted run for the commit that closes it: that commit
records the closure, so it exists only after every gate has already answered.
What can be done is to say exactly what the residue is and what checks it.

Commits after `7b6e0160` fall in two classes, and the workflow itself treats
them differently.

**Markdown.** `ci.yml`'s push trigger carries `paths-ignore: '**.md'`, so a
commit touching only markdown creates no run by the workflow's own definition.
This record and the completion report are in that class.

**Tracker state under `.jit/`.** These do trigger the workflow, and the run
above is on a tree that carries every one of them except the last: the
container's own transition to `done`, which cannot precede the gate that
authorises it.

That transition landed as `7c7e3f56` and was pushed, and the workflow is green on
it as well —
[30988318879](https://github.com/erankavija/just-in-time/actions/runs/30988318879)
(CI),
[30988318819](https://github.com/erankavija/just-in-time/actions/runs/30988318819)
(Container Image) and
[30988318744](https://github.com/erankavija/just-in-time/actions/runs/30988318744)
(Deploy Rustdoc), queued at 2026-08-05T08:16:02Z. So the residue this section
exists to bound turns out to be empty: every commit of this container that the
workflow runs on has a green run, the closing one included. The reasoning above
is kept because it is what a reader needs when the closing run has not landed
yet, which is the state every container is in at the moment its gate answers.

Both claims are checkable rather than asserted:

```bash
# Nothing compiled changed after the run this record names.
git diff --stat 7b6e0160..HEAD -- crates/ Cargo.toml Cargo.lock .github/

# What did change, and therefore which of the two classes covers it.
git diff --stat 7b6e0160..HEAD
```

An empty first result is the evidence for the compiled surface; anything in it
means this record needs a newer run.
