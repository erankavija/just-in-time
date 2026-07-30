# Workflow Contract

Every committed GitHub workflow is verified by one harness. A workflow declares
what it guarantees in [`.github/workflow-contract.yml`](../.github/workflow-contract.yml);
it carries no verifier of its own.

Two checkers run over the same tree:

| Checker | Covers |
| --- | --- |
| `actionlint`, pinned and checksum-verified | workflow schema, expression syntax, context and `needs` reference validity, shell quoting inside `run:` |
| [`scripts/workflow-contract.py`](../scripts/workflow-contract.py) | this repository's structural declarations — triggers, `workflow_call` interfaces, `needs` edges, caller obligations, permissions, failure escapes, `uses:` pinning |

The linter runs with its shellcheck and pyflakes integrations off, so a run
depends on the pinned binary alone rather than on which tools the host happens
to have installed.

`.github/` is repository policy. It is not part of the shipped jit surface, and
nothing here describes adopter-facing behaviour.

## Running it

```bash
./scripts/workflow-contract.sh            # both checkers over this repository
./scripts/workflow-contract-selftest.sh   # prove the harness still rejects each defect class
```

`workflow-contract.sh` takes an optional repository root, so it can be pointed
at a scratch copy. It needs `python3` with PyYAML, and `curl` the first time it
runs: the pinned `actionlint` release is downloaded into
`${XDG_CACHE_HOME:-~/.cache}/jit/actionlint/<version>` after its SHA-256 is
verified, and reused offline afterwards. `ACTIONLINT_BIN` points the harness at
an already-installed binary, whose `-version` must still match the pin.

Exit codes are `0` clean, `1` findings, `2` environment or contract error. An
environment error is never reported as a pass.

The same two commands run in the `workflow-contract` job of
[`ci.yml`](../.github/workflows/ci.yml), which runs on every pull request,
regardless of its base branch or changed paths. The contract forbids `branches`,
`branches-ignore`, `paths`, and `paths-ignore` filters on that pull-request
trigger, so no edit to a workflow, to the contract, or to the harness itself
reaches `main` without them: a change that violates a declared assertion fails
its pull request.

## Declaring assertions

The contract file is a single mapping of `version`, `global` rules, and one
entry per workflow file name.

### Global rules

| Rule | Effect |
| --- | --- |
| `require_declaration` | every file under `.github/workflows/` must appear in the contract, and every contract entry must name a committed file |
| `require_workflow_permissions` | every workflow states a workflow-level `permissions` block rather than inheriting the repository default |
| `forbid_failure_escapes` | no `continue-on-error`; no `if:` that survives a failed predecessor (`always()`, `cancelled()`, or `failure()` in a disjunction); no `\|\| true`, `\|\| :`, `\|\| exit 0`, or `set +e` in a `run:` script |
| `require_sha_pinned_uses` | every external `uses:` is a 40-character lowercase commit SHA carrying an upstream-version comment; every `./…` reference resolves in the tree and is scanned recursively; every `docker://` reference carries an image digest |

Matrix `fail-fast: false` is not a failure escape: it governs whether sibling
matrix legs are cancelled, not whether a failure fails the run.

### Per-workflow declarations

```yaml
workflows:
  release.yml:
    triggers:
      required:                  # the event must exist, and must carry every listed filter value
        push:
          tags: ['v*']
      allowed: [push]            # optional closed set: no other event may be declared
      forbidden: [pull_request_target]
    permissions:                 # exact match against the workflow-level block
      contents: read
    workflow_call:               # only for reusable workflows
      inputs:
        required:
          target: {type: string, required: true}
        allowed: [target]        # closed set; `[]` forbids every input
      outputs:
        required: [digest]
        allowed: [digest]
      secrets:
        allowed: []
    callers:                     # only for reusable workflows
      require_needs: [validate]  # own jobs every caller inherits
    jobs:
      create-release:            # declaring a job asserts the workflow defines it
        permissions:             # exact match against that job's block
          contents: write
        needs: [build-binaries]  # transitive predecessors in the `needs` graph
```

Within a required event mapping, `forbidden_filters` names filter keys that may
not appear on that event. The CI contract uses
`forbidden_filters: [branches, branches-ignore, paths, paths-ignore]` for
`pull_request`, ensuring its workflow-contract job is scheduled for every base
branch and changed path.

`needs` is transitive on purpose. Declaring that publication needs validation
states that no path reaches publication while validation fails, whatever
intermediate jobs are added between them.

Two structural checks run on every workflow regardless of its declaration:
`needs` may only name jobs the workflow defines, and the job graph must be
acyclic.

### Caller obligations

A reusable workflow states once, in its own entry, what a caller gets by
calling it. `callers.require_needs` names its own jobs, and binds every
workflow in the tree that calls it by file name (`uses:` a
`./.github/workflows/…` path, the form that runs the called workflow on the
caller's own commit).

Each job a caller runs itself has to reach that call through `needs`, so
nothing of the caller's starts before every inherited job has succeeded. A
caller holds one node for the whole called graph, so the edge it can declare
names the call; the job list here is what gives that edge its content. One
edge at the head of the caller's graph carries the whole graph, since `needs`
is transitive. A caller job that calls another workflow of this repository is
exempt: it is a verified boundary of the same kind, and making it wait would
only serialize two sets of suites that can run beside each other.

The promise is checked at its source too. A named job has to exist, and has to
carry no `if:` condition: a skipped job leaves the call green, so a condition
would turn an inherited result into an inherited nothing.

## Case vectors

Each directory under `test-vectors/workflow-contract/` is a miniature
repository root — a contract plus the `.github/` tree it declares assertions
over — and an `expected.txt` stating the exit code the verifier must produce
and the substrings its report must contain. The self-test runs every directory
it finds, so covering a new defect class means adding a directory, not editing
the harness.

The self-test also seeds a schema defect that only `actionlint` detects into a
scratch copy of `.github/`, which keeps the linter provably in the path rather
than declared and skipped.

## Action pins

Every external action and reusable workflow is referenced by an exact
40-character lowercase commit SHA with a trailing comment naming the upstream
version that SHA was resolved from:

```yaml
uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0
```

A tag is a movable pointer: whoever controls the upstream repository can repoint
it at different code without a change landing here. The SHA is the reviewed
artifact; the comment is what makes it legible to a reviewer.

### Updating a pin

Resolve the tag against the upstream repository rather than copying a SHA from
anywhere else. `git ls-remote` prints the peeled commit for an annotated tag,
and the tags sharing that commit give the most specific version for the comment:

```bash
git ls-remote --tags https://github.com/actions/checkout | grep -E 'refs/tags/v4(\^\{\})?$'
```

A pin-update pull request states, for each reference it moves:

1. the upstream release the new SHA belongs to;
2. the tag-to-SHA resolution the author ran;
3. the upstream changelog delta between the old and new release;
4. the permission delta — any scope, secret, or token the action newly reads or
   writes.

The pull request passes `./scripts/workflow-contract.sh` and is reviewed by a
maintainer before it merges. An updater bot may open such a pull request; it
never merges one, because the changelog and permission deltas are what the
review is for and no bot has assessed them.

### Updating the pinned `actionlint`

The version and the per-platform SHA-256 checksums live at the top of
[`scripts/workflow-contract.sh`](../scripts/workflow-contract.sh). Update them
together, taking the checksums from the release's own
`actionlint_<version>_checksums.txt` asset. A stale checksum fails the download
closed rather than running an unverified binary.
