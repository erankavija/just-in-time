# Guards and ergonomics — execution plan

> **Diátaxis Type:** Explanation

Execution plan for `f3f7de97`. The container arrived already decomposed into nine
leaf issues, so this document records what execution had to settle before the
fan-out could start: the design questions the container's own Notes deferred, the
scope the owner added once execution began, the state of the tree each issue was
filed against, and the wave order.

## Owner decisions

- D-1: The merge-integrity guard is removed rather than made to build and test.
  Both endings that keep a per-merge verification cost five to ten minutes of
  cold build behind the host build lock, and the owner ruled that cost out. The
  work `scripts/verify-commit-builds.sh` claims is already performed by the
  authoritative Rust gate, which compiles every target and runs the workspace
  suite against the tree a merge produces.
- D-2: The two carrier-pair issues converge on one home and one failure-report
  shape rather than one mechanism or two independent ones. The contribution
  check is built beside the region check that already exists, and both report
  drift through the same shape, so one defect class has one report form.
- D-3: The region issue's Background is amended to state the tree as it is. Its
  criteria are untouched.
- D-4: The hosted continuous-integration workflow is part of this container. The
  container carries a criterion for it, and the defects behind its failure are
  filed as children.
- D-5: Windows is out of scope. The workflow's Windows leg is removed rather
  than repaired, and the two Windows defects are rejected with their diagnosis
  retained. The repository keeps no Windows signal until someone restores it.

## Tree state the issues were filed against

Two of the descriptions describe a repository that has since moved, and one names
a root cause that inspection refines. Recorded here so a reader does not
re-derive it.

- The region pair is already bound. `test_managed_regions_match_every_declared_source_tree_consumer`
  in `crates/jit/src/profile/repository_package.rs` walks the manifest's region
  declarations and compares each packaged source against the rendered body.
  Inserting a line into the rendered region in `AGENTS.md` fails it. The gap the
  issue closes is that the failure names neither carrier.
- The contribution pair is not bound. Nothing compares a `[[contribution]]` value
  against the corresponding entry in `.jit/invariants.toml`, `.jit/rules.toml`,
  `.jit/gates.toml`, or the namespace and type-hierarchy tables in
  `.jit/config.toml`. The template registry is the one exception, bound by its
  own drift assertion.
- The merge guard's blind spot is its build command. `cargo build --workspace`
  compiles neither test targets nor dev-dependencies, which is both why a merge
  breaking only tests passes it and why a cold run finishes in about half a
  minute.

## What the workflow failure is

Two independent causes, established from the logs of the last completed run and
the run cancelled during this session.

- One test holds four jobs open to the six-hour execution ceiling. Its own waits
  are bounded; the reap and the two pipe-drain joins that follow them are not,
  and job cleanup reports the processes it should have killed as orphans. It
  passes in isolation on a developer machine in under a second.
- The Windows leg of the profile-adoption job fails every test it runs, on three
  distinct defects: initialization overflows the main thread's stack, staged
  package publication reports its atomic no-replace rename unsupported, and a
  test helper writes a host path separator into a TOML basic string.

## Waves

Wave membership is bounded by file footprint, not by the DAG: the only real
dependency edge among the leaves is the region check's on the contribution check.

1. `5367fcba` (package-assembly argument parsing), `89a7e34e` (staleness refusal
   message), `6d496b8c` (multi-issue gate evaluation), `3019eacd` (merge guard),
   `ee02e514` (workflow platform matrix), `76a4bd21` (unbounded serve test).
2. `896b5b5d` (issue description from file or standard input), `a2d7d212`
   (contribution-versus-registry binding), `06f1fa95` (load-independent drain
   assertion).
3. `176d14d3` (region-check reporting), `e516b4f8` (foreign requirement-tag
   sweep, run last so its sweep observes the finished tree).

The two command-surface issues are separated across waves one and two because
both add a subcommand surface and would otherwise contend for the same argument
parser and generated schema.

The two workflow issues sit in wave one because the criterion covering the
workflow can only be evidenced by a completed hosted run, and a run costs
wall-clock this container does not control.
