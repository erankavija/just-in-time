# Bounded head lookup experiment

> Diátaxis Type: Explanation

The steward approved the bounded two-process protocol on 2026-11-05 after the experiment distinguished unborn branches from genuine failures without walking unrelated refs, reflog history, ancestors or worktree files (`@/charter/D-19`). The runtime matrix used Git 2.56.0; older-version conclusions below come from versioned source inspection.

## Superseded decision and rejected candidates

The original decision used `rev-parse HEAD --abbrev-ref HEAD` once and treated any nonzero exit as unborn because the checkout had previously been recognized. Review F1 showed that an unchecked or stale checkout can fail for other reasons; constructor visibility does not establish error semantics.

Root-ref listing returned successful empty output for both unborn and malformed symbolic refs or cycles. Formatted branch listing distinguished the tested errors, but read detached HEAD reflog history and enumerated refs. A disposable reflog FIFO blocked that candidate. The steward rejected that history-dependent approach. A commit-peeling discovery candidate lazily fetched missing promisor objects; the final protocol avoids it. No claim is made that every possible one-process approach is impossible.

## Approved protocol


Always start with:

```sh
git -C "$checkout" --no-replace-objects \
  rev-parse --revs-only HEAD --symbolic-full-name HEAD
```

Require successful exit and empty stderr. Spawn failure is `Unavailable`; any nonzero status is an error. **Do not use `HEAD^{commit}` in this first command:** it can lazy-fetch an absent object. This command reads raw ref identity only.

If stdout is exactly two lines, `OID\nREF\n`:

- Parse OID with `CanonicalCommitOid::parse` (40 or 64 hex digits).
- REF must be `HEAD` (detached) or `refs/heads/<nonempty branch>`; strip the exact prefix to obtain the branch name. No shortened/ambiguous ref naming or translated text is used.
- Validate that specific captured OID with a second process:

```sh
git -C "$checkout" --no-replace-objects -c core.commitGraph=false \
  rev-list --no-walk --missing=print "$oid" --
```

Accept only exit 0, empty stderr, and stdout **exactly `OID\n` matching the captured OID**. This proves that the captured object is a commit. Empty output (e.g. blob), a different OID (e.g. a tag peeled to a commit), `?OID` (missing tip on recent Git), any other output, or nonzero exit is an error. Older Git can fail nonzero for a missing tip; the contract treats both representations identically as an error. Never translate a missing commit object to unborn.

If the first process instead succeeds with **zero stdout bytes**, use the second process to validate the unborn case:

```sh
git -C "$checkout" --no-replace-objects symbolic-ref --quiet HEAD
```

Accept `None` only for exit 0, empty stderr, and exactly one nonempty `refs/heads/<branch>` line. Git's recursive resolution rejects malformed referenced refs, invalid symbolic targets, and cycles. Detached/invalid HEAD or symbolic targets outside `refs/heads/` are errors. Any other first-process output is malformed and fails without a second call. Error paths have no retry/fallback beyond this prescribed second command.

Both operations use the configured Git executable. The second validation uses the captured OID, so it does not accidentally validate a later HEAD value. This is not a promise of a transactional snapshot across concurrent Git mutations; the original operation also performs multiple ref reads inside Git. No new locking or checkout-lifetime identity contract is proposed.

## Why the guards are necessary

- `--missing=print` disables lazy fetching before revision arguments are resolved. In contrast, a discovery command containing `HEAD^{commit}` started two fetch subprocesses when the HEAD object was absent in a promisor repository. The discovery fixture used a nonexistent **local** remote, so no network service was contacted; evidence is `head-two-promisor-discovery-trace.json`.
- `--no-walk` selects only the captured commit, without walking ancestors or trees. The `missing-parent` fixture retains a valid tip while deleting its parent object and still succeeds.
- `core.commitGraph=false` is essential. Without it, a commit-graph entry can make a deleted commit object appear valid. This failure is preserved in `head-two-rejected-commitgraph.json`; the final protocol rejects that fixture. The setting is child-local, not a repository configuration mutation.
- `--no-replace-objects` applies to **both** processes. Missing/corrupt original HEAD objects with a valid replacement must still fail, and replacement-ref enumeration must not introduce unrelated reference work. Both replacement fixtures fail correctly.
- The detached `fifo-bounded` fixture puts FIFOs at the HEAD reflog, an unrelated loose branch ref, the index, and an untracked worktree path. Both commands finish successfully; neither attempts to open those paths for reading. An unrelated malformed branch ref also does not affect the lookup. These tests distinguish this protocol from Phase 1 branch listing.
- Every executed lookup in the final matrix reports zero Trace2 `child_start` events, including missing promisor objects. At most two direct processes are attempted; a missing executable makes one failed spawn attempt and starts no process.

## Runtime evidence

Every case below produced its expected classification, used at most two direct Git attempts, and recorded zero Trace2 child starts. Fixtures used an owned short `/dev/shm` directory removed on exit. The promisor remote was a nonexistent local path; no network service was contacted. Missing-executable means a failed spawn attempt, not a started process.

| Case | Exit / stdout, process 1 → process 2 | Classification |
|---|---|---|
| branch | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `OID\n` | branch |
| unborn | 0 / `empty` → 0 / `refs/heads/feature/slash\n` | unborn |
| detached | 0 / `OID\nHEAD\n` → 0 / `OID\n` | detached |
| linked | 0 / `OID\nrefs/heads/linked/slash\n` → 0 / `OID\n` | branch |
| sha256 | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `OID\n` | branch |
| sha256-unborn | 0 / `empty` → 0 / `refs/heads/feature/slash\n` | unborn |
| nonrepo | 128 / `empty` + stderr | error |
| nonexistent | 128 / `empty` + stderr | error |
| stale | 128 / `empty` + stderr | error |
| malformed-head | 128 / `empty` + stderr | error |
| empty-head | 128 / `empty` + stderr | error |
| bad-symbolic-head | 128 / `empty` + stderr | error |
| head-invalid-ref | 0 / `empty` → 128 / `empty` + stderr | error |
| head-tag-missing | 0 / `empty` → 0 / `refs/tags/missing\n` | error |
| head-zero | 0 / `OID\nHEAD\n` → 0 / `?OID\n` | error |
| head-missing-detached | 0 / `OID\nHEAD\n` → 0 / `?OID\n` | error |
| head-cycle | 0 / `empty` → 128 / `empty` + stderr | error |
| missing-commit | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `?OID\n` | error |
| corrupt-commit | 0 / `OID\nrefs/heads/feature/slash\n` → 128 / `empty` + stderr | error |
| bad-branch-ref | 0 / `empty` → 128 / `empty` + stderr | error |
| branch-blob | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `empty` | error |
| detached-blob | 0 / `OID\nHEAD\n` → 0 / `empty` | error |
| promisor-missing | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `?OID\n` | error |
| unrelated-corrupt-ref | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `OID\n` | branch |
| unborn-with-other-branch | 0 / `empty` → 0 / `refs/heads/new/unborn\n` | unborn |
| fifo-bounded | 0 / `OID\nHEAD\n` → 0 / `OID\n` | detached |
| missing-parent | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `OID\n` | branch |
| head-target-dir | 0 / `empty` → 0 / `refs/heads/directory\n` | unborn |
| packed-refs | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `OID\n` | branch |
| commit-graph-missing | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `?OID\n` | error |
| replacement-missing | 0 / `OID\nrefs/heads/feature/slash\n` → 0 / `?OID\n` | error |
| replacement-corrupt | 0 / `OID\nrefs/heads/feature/slash\n` → 128 / `empty` + stderr | error |
| sha256-detached | 0 / `OID\nHEAD\n` → 0 / `OID\n` | detached |
| missing-executable | spawn failed | unavailable |

## Version evidence


The latest required CLI feature is `rev-list --missing=print`, present in [Git 2.16.0 source](https://raw.githubusercontent.com/git/git/v2.16.0/builtin/rev-list.c) and absent from [2.15.0 source](https://raw.githubusercontent.com/git/git/v2.15.0/builtin/rev-list.c). Thus **2.16 is the command-feature floor**, not a newly adopted project support policy. Partial-clone support's subsequent implementation disables fetching for this option before revision setup in [Git 2.17.0](https://raw.githubusercontent.com/git/git/v2.17.0/builtin/rev-list.c); the same ordering and `fetch_if_missing = 0` behavior are explicit in [Git 2.39.5](https://raw.githubusercontent.com/git/git/v2.39.5/builtin/rev-list.c). `core.commitGraph=false` disables a later optimization; Git versions predating that optimization do not need it. SHA-256 repositories naturally require a Git version that understands their object format; parsing remains algorithm-independent.

The assigned Dockerfile's final stage is `debian:bookworm-slim` and installs `git` with apt. Debian's authoritative [bookworm package page](https://packages.debian.org/bookworm/git) identifies Git `1:2.39.5-0+deb12u3`. The proposed protocol fits that existing package's command surface; **no Docker/package or Git >=2.45 change is proposed**. Runtime experiments here used 2.56.0; 2.16/2.17/2.39 conclusions are versioned-source evidence, not claims that those binaries were executed. A supported older-binary runtime check can strengthen implementation validation without altering the design.

## Criterion reconciliation

The approved change replaces only the head operation's explicit one-process budget and its invalid nonzero-means-unborn rule. The branch-label contract stays unchanged. Downstream consumers request one head operation and test their own wiring; the issue-read process budget is unchanged. No WorktreePaths redesign, project-wide Git minimum, or Docker change is part of this decision.

The owning behavior suite covers the failure boundaries and regressions established here. This document records experimental evidence, not a claim that research-only work passed Rust gates.
