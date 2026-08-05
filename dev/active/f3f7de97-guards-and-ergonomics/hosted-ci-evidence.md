# Hosted continuous-integration evidence (f3f7de97, REQ-08)

> **Diátaxis Type:** Reference (evidence record)
> Recorded from the GitHub Actions API on 2026-08-05 for the push of `main` at
> `33c89c44580d22ebc7d46987b71f388ee645988d`. Every field below is read from
> `gh run view <id> --json name,jobs`; nothing is restated from memory.

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
| CI | [30979416135](https://github.com/erankavija/just-in-time/actions/runs/30979416135) | success |
| Security Audit | [30979416166](https://github.com/erankavija/just-in-time/actions/runs/30979416166) | success |
| Container Image | [30979416145](https://github.com/erankavija/just-in-time/actions/runs/30979416145) | success |
| Deploy Rustdoc | [30979416133](https://github.com/erankavija/just-in-time/actions/runs/30979416133) | success |

All four were queued at 2026-08-05T05:51:38Z by the push of `33c89c44`.

## Every job, with its duration against the ceiling

| Workflow | Job | Conclusion | Duration | Share of the 360-minute ceiling |
| --- | --- | --- | --- | --- |
| CI | Test Rust Components | success | 12.2 min | 3.4 % |
| CI | Test Rust (html + xml features) | success | 6.8 min | 1.9 % |
| CI | MSRV (build and test on declared rust-version) | success | 6.4 min | 1.8 % |
| CI | Coverage Badges | success | 6.3 min | 1.8 % |
| CI | Test MCP Server | success | 5.7 min | 1.6 % |
| CI | Validate Repository Data | success | 1.7 min | 0.5 % |
| CI | Profile adoption (Linux) | success | 1.6 min | 0.4 % |
| CI | Test Web UI | success | 0.9 min | 0.3 % |
| CI | Workflow Contract | success | 0.2 min | 0.1 % |
| Security Audit | Rust advisories | success | 3.0 min | 0.8 % |
| Security Audit | MCP server advisories | success | 0.2 min | 0.1 % |
| Security Audit | Web UI advisories | success | 0.2 min | 0.1 % |
| Container Image | Build and Smoke the Server Image | success | 8.0 min | 2.2 % |
| Deploy Rustdoc | build-and-deploy | success | 3.1 min | 0.9 % |

Fourteen jobs, fourteen successes, no cancellation and no timeout. The longest
took 12.2 minutes, which is 3.4 % of the ceiling; the whole set of four runs
finished inside 13 minutes of wall clock.

## What changed to make this possible

Three children of this container produced the difference, and each is visible in
the table above.

- `76a4bd21` bounded the foreground-serve case. The last CI run before this
  container's work reached the hosted workflow,
  [30884326921](https://github.com/erankavija/just-in-time/actions/runs/30884326921),
  ran `Test Rust Components`, `Test Rust (html + xml features)`, `MSRV` and
  `Coverage Badges` for 5.36 hours each before GitHub cancelled all four; the
  run's own conclusion is `cancelled`. The rows above are those same four jobs
  at 12.2, 6.8, 6.4 and 6.3 minutes.
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
that it passes on every future one. The claim is bounded to `33c89c44`, and the
three later commits on this container's branch — the progress record, this
document and the completion report — touch no compiled source, no workflow file
and no dependency manifest.
