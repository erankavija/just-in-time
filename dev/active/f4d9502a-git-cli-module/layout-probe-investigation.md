# Investigation: the one-process layout requirement

> Historical investigation of the prior universal one-process REQ-04 at the source
> anchor below. The subsequent steward decision adopts one computation per distinct
> checkout context; the active contract and remediation are in [the plan](plan.md).
> The measurements and rejected alternatives below preserve the evidence for that decision.

## Verdict

**The current implementation violates REQ-04 for an external data root. No complete
one-process replacement preserving the current automatic selection and identity
semantics was established.** There are two distinct gaps:

1. Bootstrap discovers the data-root checkout, and on a miss discovers the invoking
   checkout. These are independent contexts. The boundary carries an explicit path
   for the second context, but no previously resolved Git facts to reuse.
2. Listing worktrees or validating a lease over an external store probes listed
   checkouts again to find each checkout's private Git directory. Removing only
   bootstrap's second probe leaves this hard-criterion violation.

Reuse of an invocation-scoped resolved checkout is the right ownership direction.
It removes repeated discovery only when that checkout has actually been resolved;
passing an unverified path under a new type name does not establish those facts.
A conditional one-probe design is described below, including why it is insufficient
without an owner decision about authoritative inputs. Do not implement it as an
unconditional replacement or close REQ-04 on the strength of the ordinary-checkout
process-budget test.

## Evidence boundary

Source anchor: `b0ece0d5449b42e51018b6acf6b143aa74361d0d`, research branch
`worktree-agent-f4d9502a`. Main inspected at
`f4b9813452665c72b989f209fc2d54e3cee567ce`. The relevant bootstrap, worktree paths,
identity, claim, init, worktree, validation, storage, and server files match main.
Main's Git-module diff adds unrelated operation code; it does not change `probe`.

The measured executable is the existing `target/debug/jit`, dated 2026-11-05 02:14,
SHA-256 `51a77d4755e4ecf39b813d6613dc70a32834d8186b17484d3d7efbaa71270f07`. It was not rebuilt, so exact source correspondence is not
asserted. Its process traces match the cited source branches. Git is 2.56.0.
Experiments clear inherited Git/JIT variables in the child environment and use
isolated Git configuration. The counted wrapper logs arguments and execs the real
Git; fixture creation is excluded from command counts. No process-wide environment
or cwd changes are proposed for committed tests.

Detailed evidence is in `/tmp/f4d9502a-lead/layout-research/`:

- `experiment.py` and `run-ebcd9wn4/results.json`: fixtures, binary hash, command
  outputs, process arguments, and alternative-protocol probes.
- `followup.py` and `run-ebcd9wn4/followup.json`: an active sibling lease, validation,
  checkout move, stable identity, and worktree porcelain output.
- `run-ebcd9wn4/malformed-marker.json`: initialized store below an invalid `.git`
  marker, proving that marker existence is not checkout classification.

All fixtures and their tracker data are synthetic and wholly owned by this
experiment. The dispatch restriction on managing real project worktrees remains
intact; the lead explicitly authorized synthetic linked worktrees in this scratch.
No build or workspace test run was needed for this read-only investigation.

## Existing authority and actual callpaths

`commands/bootstrap.rs:72–180` owns selection. `RepositoryInvocation` supplies
`current_dir` and `data_dir_override`; it contains no `ProbedCheckout` or
`GitCheckout`. `select_data_root` normalizes an override, performs filesystem JIT
repository discovery, and returns `(data_root, non_git_worktree_root)`.
For an external override the second path is normally the explicit invocation cwd.
It can be a subdirectory of a checkout, and it is not yet known to belong to Git.

`dispatch/mod.rs:128–145` and `dispatch/init.rs:19–38` construct this input from the
process boundary. They do not resolve a Git checkout before opening the repository.
The server at this anchor still calls `WorktreePaths::detect()` against its process
cwd (`crates/server/src/main.rs:59–71`) and does not provide a reusable resolved
checkout to shared bootstrap. The planned server migration should consume the same
boundary result, not create an additional probe.

`storage/discovery.rs:56–66` walks for an initialized `.jit` and stops at an existing
`.git` marker. This is data-root discovery, not an authoritative Git classifier.
`RepositoryLayout` establishes filesystem root evidence, not Git membership.
Neither source can supply the missing common directory or private Git directory.

`storage/worktree_paths.rs:70–129` then does:

1. Find the nearest existing directory at/above the selected data root.
2. Call `Git::probe` there.
3. On success, use that checkout's top level and common directory, set
   `git_repository = true`, and retain the selected data root.
4. On failure, retain `non_git_worktree_root`. If the data root does not lexically
   start with that path, call `Git::probe(non_git_worktree_root)` and retain its
   common directory, top level, and private Git directory as relocated context.
   Otherwise, or when this second probe also fails, use the unbacked
   `<worktree_root>/.git` fallback.

One `Git::probe` uses one process with
`rev-parse --git-common-dir --show-toplevel --absolute-git-dir`
(`storage/git.rs:97–137`). Non-success and unavailable executable both classify as
no checkout. Malformed successful output or unresolved returned paths are errors.
That classification contract is existing behavior, not a proposal to expand
silent fallback.

Bootstrap configures storage and executor with the same `WorktreePaths` before
recovery and validation. `JsonFileStorage::configured_worktree_paths`
(`storage/json.rs:612`) and `CommandExecutor::require_worktree_paths`
(`commands/mod.rs:2589`) still have lazy detectors for unconfigured handles.
Their planned removal remains necessary, but cannot fix bootstrap's two probes.

### Preserved cases

| Selected data root | Explicit fallback path | Current authority and consequences |
|---|---|---|
| In primary checkout A | A or elsewhere | A owns Git classification, common directory, identity in data root; one probe. |
| In linked checkout A2 | A2 or elsewhere | A2 owns classification; common directory belongs to A; linked-store policy and primary-store mapping apply; one probe. |
| Outside every checkout | A | `git_repository` stays false; A supplies coordination and private identity directory; data has no primary-store counterpart; two probes. |
| Outside every checkout | A/subdir | Worktree root remains A/subdir; relocated top level is A. Identity and primary/linked classification use A; two probes. |
| Outside every checkout | Linked checkout A2 | Shared leases use A's common directory; identity uses A2's private directory; linked-store write refusal does not govern the external store; two probes. |
| In another checkout B | A | B wins. Branch commands run in B and claims use B's common directory; one probe. |
| Non-Git local store | Its containing non-Git root | No coordination; core reads work; one probe. |
| Non-Git external store | Another non-Git path | No coordination; core reads work; two probes. |
| Any of the above, executable absent | Selected path(s) | Core reads work. No Git process starts; the external branch currently makes two failed spawn attempts. |

These are source facts with representative CLI observations, not assumptions that
lexical containment proves membership. Existing external-root obligations are
@/issue/c4e07f6d and @/issue/7dd59ebf; implementation commits are `e3bb042ae` and
`95aae1f12`, respectively. The short issue IDs themselves are not Git revisions.

@/issue/5046408a is still backlog. Its declared worktree root is a path, not a
probed checkout; its default must preserve current derivation. The reviewed plan's
DEC-23 preserves data-derived coordination while replacing the command worktree
root. It cannot be treated as already-supplied Git evidence or silently repurposed
to declare an external data root.

## Measured process counts

Numbers below count layout `rev-parse` processes, followed by all Git processes in
parentheses. Branch and worktree-listing processes are shown separately by that
total; they are not charged as layout probes just because they also use Git.

| Fixture | `query available` | `worktree info` | `worktree list` | `claim list` |
|---|---:|---:|---:|---:|
| Data in primary A | 1 (1) | 1 (2) | 1 (2) | 1 (2) |
| External data, primary A | 2 (2) | 2 (3) | 4 (5) | 2 (3) |
| External data, linked A2 | 2 (2) | 2 (3) | 4 (5) | 2 (3) |
| Data in B, invoked from A | 1 (1) | 1 (2) | 1 (2) | 1 (2) |
| Local data, outside Git | 1 (1) | 1 (2) | 1 (2) | 1 (3) |
| External data, outside Git | 2 (2) | 2 (3) | 2 (3) | 2 (4) |

Core queries succeed in all rows. Worktree info/list fail outside Git, and claim
list reports the typed Git requirement with exit 10 there. With an empty child
PATH, both local and external query cases succeed with no Git executable.
Those successes do not prove the pending migration's missing-executable claim
mapping; that remains its existing owner.

For an external store with no leases, `validate --leases` uses only the two startup
probes. After acquiring one lease from A2, validation from A uses **four layout
probes plus one worktree listing** and succeeds. The trace is:

1. Probe the external data root (no checkout).
2. Probe A (checkout found).
3. List worktrees from A.
4. Probe A again for its identity directory.
5. Probe A2 for its identity directory.

The owning source is `recorded_worktree_ids` in `commands/validate.rs:1370–1402`,
which calls `WorktreePaths::for_worktree_root`; the latter probes each listed
checkout when relocated context is present (`storage/worktree_paths.rs:204–216`).
`execute_worktree_list_at` in `commands/worktree.rs:240–290` takes the same path.
Even one listing per run, as required by REQ-08, does not remove those probes.

## Why the tempting one-process substitutions are insufficient

### Probe the already-selected worktree path

This works for an explicitly known external store, but automatic mode has not
established that fact. With cwd A and data B/.jit, current behavior selects B.
With cwd A and an outside store, current behavior selects A for coordination.
Probing A alone returns identical facts in those two cases and supplies no evidence
about B. Probing the data root alone gets B right but supplies no A facts on a miss.
A fresh, invocation-scoped `ProbedCheckout` supplied by a genuine earlier resolver
can be reused; moving its acquisition earlier still counts its Git process.

### Use `.git` existence to choose the one probe

An existing marker is not a valid checkout. The experiment creates an empty `.git`
directory above an initialized external store. Git rejects it; today's invocation
falls back to A and succeeds with two probes. A marker-based selector spends its
only probe on the invalid marker and then must either change behavior, guess A's
facts, or run another process. No direct `.git` parser is proposed.

A carefully constrained filesystem negative proof could avoid the first probe for
some ordinary outside directories. That is an optimization hypothesis, not a
complete solution: it would have to account for bare repositories, Git discovery
environment, symlinks, mount boundaries, invalid markers, and separately located
administrative directories. It must not turn the JIT discovery walk into a second
Git implementation. A fast path does not establish the universal process bound.

### Put both directories into one `rev-parse`

Measured on Git 2.56.0:

- Two global `-C` options select the last directory; they do not return two layouts.
- `-C` inserted among `rev-parse` options does not switch repositories and fails.
- `--resolve-git-dir <other>/.git` returns the other administrative directory, but
  following `--show-toplevel` still reports the original checkout. It does not
  return the other's common directory or validate worktree membership there.
- An invalid `--resolve-git-dir` target fails the command. Partial stdout is not
  a documented independent per-root classification result.

The documented operation resolves an explicit repository/gitfile, while
`--show-toplevel` reports the current repository's worktree. This matches the
experiment; it is not a multi-context layout protocol.
[Git rev-parse reference](https://git-scm.com/docs/git-rev-parse#Documentation/git-rev-parse.txt---resolve-git-dirltpathgt).

`git for-each-repo` explicitly starts a Git subprocess for every listed repository.
A shell alias or wrapper around multiple probes likewise exceeds a process bound;
counting only the wrapper would hide the work.
[Git for-each-repo reference](https://git-scm.com/docs/git-for-each-repo).

### Derive sibling identity locations from `worktree list`

Porcelain lists worktree path, HEAD, branch/detached/bare and status annotations;
it does not supply the private administrative directory. Git documents that its
name can have a disambiguating numeric suffix.
[Git worktree reference](https://git-scm.com/docs/git-worktree#_details).

In the experiment, moving `linked` to `moved` keeps the identity ID and changes the
reported root with one relocation warning. Its administrative directory remains
`A/.git/worktrees/linked`. The listing reports `moved` without that directory.
Thus deriving `<common>/worktrees/<basename>` fails even without a name collision.
Reading Git's pointer files directly is prohibited by this assignment. A global
cache could become stale and would not handle an unvisited sibling's identity.
Replacing the identity location or declining to materialize an unvisited sibling's
identity changes existing semantics and is not an authorized shortcut.

## Conditional candidate and ownership

A useful candidate carrier has one invocation-owned resolved checkout and explicit
provenance for the data root's relationship to that checkout. It must distinguish
**data-owned checkout**, **external store with a coordination checkout**, and
**no checkout**. This is a design sketch, not a request to add a helper or competing
representation before the owner decides the input contract.

1. Bootstrap resolves path inputs and their provenance once. A caller that already
   owns current-invocation Git facts passes them; otherwise the Git module performs
   the single allowed probe against the one explicitly authoritative directory.
2. `WorktreePaths` derives coordination, local-data, primary-data and identity paths
   from that result and provenance. It preserves separate effective worktree and
   physical checkout top-level paths for the subdirectory case.
3. Bootstrap passes the same result to storage, executor, startup recovery and
   server. Remove both lazy fallback detectors; absent injected context performs
   no detection. Consumers choose the appropriate capability below.
4. Worktree-list consumers must also receive authoritative private-directory facts
   for siblings. Present porcelain output does not provide them, and the selected
   checkout's probe cannot supply them. That part of this candidate has no proven
   compliant producer under the current constraints.

With a trusted explicit external-store classification, steps 1–3 take one probe
and preserve external coordination and identity for the selected checkout. With a
trusted data-owned classification they take one probe of B and preserve the
other-checkout case. **Current CLI inputs supply neither classification.** Adding
an explicit selection/declaration would be a product contract decision, with
migration/default semantics to resolve, not a mechanical refactor. Current
automatic behavior and the unvisited-sibling requirement remain unsolved by this
candidate.

### Caller guidance for the ongoing migrations

| Consumer | Correct current authority | Migration consequence |
|---|---|---|
| `initialize_worktree_identity` | It returns early when `!paths.is_git_repository()` (`commands/init.rs:300`). | Keep the guard. `paths.git()` is correct inside it. Do not broaden init to create external-store identities. |
| Worktree info branch | Previously runs at `paths.worktree_root`. External-store mode must still see its relocated selected checkout. | `paths.git()` alone is wrong. `write_checkout()` currently supplies that capability; use it or a single shared accessor with exactly that authority. Preserve reported worktree root separately. |
| Claim branch and user-name operations | Same selected checkout as the command's previous explicit root, including relocated context. | Do not map `git_repository == false` directly to `NoRepository`; an external store can coordinate. Use the available selected/relocated checkout and retain typed missing-checkout/no-commit handling. |
| Claim coordinator and recovery | `has_claims_coordination()` and cached `common_dir/shared_jit`. | Keep relocated coordination; reuse the branch fact when already obtained rather than reprobe. |
| Worktree list and lease validation | Selected/relocated checkout for listing; listed checkout's own private directory for identity. | `write_checkout()` can fix routing of the listing, but does not solve per-sibling layout probes. Retain the concern explicitly. |
| Profile dirty-target refusal | Existing `write_checkout()` handles relocated worktree writes (`commands/profile.rs:1175`). | Preserve it; `git()` would skip the refusal for an external store. |
| Linked-store refusal and issue aggregation | `holds_linked_store()` / `primary_data_root()`, which intentionally distinguish the data plane. | Do not infer data ownership from the existence of a command checkout; external shared data must not gain linked-store policy or a primary-data counterpart. |
| Gate evidence / future configured worktree | The effective worktree selected by bootstrap, with the existing plan's data-common-dir distinction. | Preserve selected-root provenance. Review DEC-23 before generalizing an accessor; a later configured worktree may differ from the data-root checkout. |

`git()` currently constructs operations at `worktree_root` only when
`git_repository` is true; `write_checkout()` adds relocated context. Neither
accessor probes. The table is deliberately per caller: blanket substitution would
break either external claims or init identity behavior. The unguarded command
consumers and the guarded init consumer are different contracts.

## Required proof for an implementation decision

Keep the existing external-store behavioral owners in
`crates/jit/tests/cli_repo_workflow/external_data_root_tests.rs`, including stable
IDs across alternating checkouts, subdirectory identity, actual moves, concurrency,
listing an unvisited sibling, lease recognition and stale shared identity immunity.
Extend the process-count owner with the missing layout cases rather than copying
the entire behavioral matrix into consumer tests (`@/inv/orthogonal-tests`).

The decisive process assertions must cover an ordinary checkout, an external store
from primary and linked checkouts, a data root in another checkout, outside Git,
missing executable, and list/lease validation with a sibling lease. Include a
moved checkout so private-directory derivation cannot pass by basename coincidence.
For every case assert the selected common directory and identity behavior as well
as the count; a one-process answer for the wrong repository is not success.

## Minimal owner decisions

1. **Input authority:** determine whether a new explicit data/checkout authority
   contract is permissible. If yes, define automatic-mode behavior and proof of
   current-invocation provenance before implementing the conditional design. If
   automatic data-first fallback must remain exactly as today, retain this as an
   unresolved architectural blocker; the current callers do not contain reusable
   facts that make the second probe disappear.
2. **Sibling identity evidence:** identify an allowed producer of each listed
   checkout's private Git directory that fits the hard process budget and preserves
   unvisited/moved-checkout behavior. No such producer was established here. This
   requires a coordinated design decision, not a local migration that drops the
   probes or relocates identity files.

Recommendation: preserve the landed external-root semantics and keep REQ-04 open
until both evidence gaps have a reviewed solution. Do not amend the criterion,
waive the external layout, introduce direct Git-file parsing or a stale cache, or
claim that reusing a path is reusing a detected layout. The process-count findings
are observed; infeasibility for every conceivable Git protocol is not formally
proved. The tested and documented stock-CLI candidates above are insufficient.
