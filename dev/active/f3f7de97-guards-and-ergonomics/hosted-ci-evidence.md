# Hosted continuous-integration evidence (f3f7de97, REQ-08)

> **Diátaxis Type:** Reference (evidence record)
> Recorded from the GitHub Actions API on 2026-08-05 for the push of `main` at
> `b94bf7e2`, the commit at which this container's last change to a compiled
> source landed. Every field below is read from `gh run view <id> --json
> name,jobs`; nothing is restated from memory.

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
| CI | [30985528861](https://github.com/erankavija/just-in-time/actions/runs/30985528861) | success |
| Container Image | [30985528840](https://github.com/erankavija/just-in-time/actions/runs/30985528840) | success |
| Deploy Rustdoc | [30985528852](https://github.com/erankavija/just-in-time/actions/runs/30985528852) | success |

All three were queued at 2026-08-05T07:34:04Z by the push of `b94bf7e2`.

Security Audit is absent by design rather than by omission. Its `push` trigger is
filtered to `**/Cargo.lock` and `**/package-lock.json`, and this push changed
neither, so no run was created. It last ran on this container's work at
[30979416166](https://github.com/erankavija/just-in-time/actions/runs/30979416166)
and passed all three of its jobs.

## Every job, with its duration against the ceiling

| Workflow | Job | Conclusion | Duration | Share of the 360-minute ceiling |
| --- | --- | --- | --- | --- |
| CI | Test Rust Components | success | 12.1 min | 3.4 % |
| CI | MSRV (build and test on declared rust-version) | success | 8.2 min | 2.3 % |
| CI | Test Rust (html + xml features) | success | 7.0 min | 1.9 % |
| CI | Coverage Badges | success | 6.3 min | 1.7 % |
| CI | Test MCP Server | success | 4.8 min | 1.3 % |
| CI | Validate Repository Data | success | 1.7 min | 0.5 % |
| CI | Profile adoption (Linux) | success | 1.5 min | 0.4 % |
| CI | Test Web UI | success | 0.9 min | 0.2 % |
| CI | Workflow Contract | success | 0.3 min | 0.1 % |
| Container Image | Build and Smoke the Server Image | success | 6.7 min | 1.9 % |
| Deploy Rustdoc | build-and-deploy | success | 2.6 min | 0.7 % |

Eleven jobs, eleven successes, no cancellation and no timeout. The longest took
12.1 minutes, which is 3.4 % of the ceiling; the whole set of three runs finished
inside 13 minutes of wall clock.

## What changed to make this possible

Three children of this container produced the difference, and each is visible in
the table above.

- `76a4bd21` bounded the foreground-serve case. The last CI run before this
  container's work reached the hosted workflow,
  [30884326921](https://github.com/erankavija/just-in-time/actions/runs/30884326921),
  ran `Test Rust Components`, `Test Rust (html + xml features)`, `MSRV` and
  `Coverage Badges` for 5.36 hours each before GitHub cancelled all four; the
  run's own conclusion is `cancelled`. The rows above are those same four jobs
  at 12.1, 7.0, 8.2 and 6.3 minutes.
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
that it passes on every future one. The claim is bounded to `b94bf7e2`.

An earlier version of this record made the same claim for `33c89c44` and added
that the commits after it touched no compiled source. That was false: `36fc326c`
and `84bb42f3` — `91cc038c`'s fix and its rework — changed
`crates/jit/src/storage/contention_probe.rs`, `claim_coordinator.rs` and
`lock.rs`. The container's own holistic review caught it, which is why this
record is anchored at `b94bf7e2` instead.

## The commits after the run, and what covers them

A container cannot get a hosted run for the commit that closes it: that commit
records the closure, so it exists only after every gate has already answered.
What can be done is to say exactly what the residue is and what checks it.

Commits after `b94bf7e2` fall in two classes, and the workflow itself treats
them differently.

**Markdown.** `ci.yml`'s push trigger carries `paths-ignore: '**.md'`, so a
commit touching only markdown creates no run by the workflow's own definition.
This record and the completion report are in that class.

**Tracker state under `.jit/`.** These do trigger the workflow. One job reads
them: `Validate Repository Data`, which runs `jit validate` with no issue id —
whole-repository rules plus the integrity checks. That is the same command this
container's `repo-validate` gate runs, against the same tree, and its result is
recorded on the container. For a `.jit`-only delta the hosted job and the local
gate are one check, not two similar ones.

Both claims are checkable rather than asserted:

```bash
# Nothing compiled changed after the run this record names.
git diff --stat b94bf7e2..HEAD -- crates/ Cargo.toml Cargo.lock .github/

# What did change, and therefore which of the two classes covers it.
git diff --stat b94bf7e2..HEAD
```

An empty first result is the evidence for the compiled surface; anything in it
means this record needs a newer run.
