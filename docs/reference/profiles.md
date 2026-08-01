# Repository Profiles

> **Diátaxis Type:** Reference

Repository profiles install a coherent JIT workflow as one explicit operation.
JIT v1.0 exposes one immutable profile embedded in the binary:
`jit-dogfood`. Applying it needs no Git repository, network access, `jq`, or JIT
source checkout.

For a new repository, this is the preferred setup:

```bash
mkdir my-project
cd my-project
jit init --profile jit-dogfood
```

Plain `jit init` remains the methodology-neutral alternative. It creates the
base repository without installing the dogfood workflow.

## Commands

Inspect the profiles carried by the running binary:

```bash
jit profile list
jit profile show jit-dogfood
```

Preview and apply the embedded profile to an existing JIT repository:

```bash
jit profile apply jit-dogfood --dry-run
jit profile apply jit-dogfood
```

All profile commands support `--json`. `profile list` uses the standard
count-wrapped list shape. `profile show` returns the manifest, package identity,
target hashes, size, and the stored provenance record when present. Showing that
record does not re-verify current target bytes. Use
`jit profile apply jit-dogfood --dry-run` for exact current-state verification:
it returns the deterministic plan hash and every target's `create`, `update`, or
`unchanged` action without writing. Successful reapplication of an exact
installation returns `unchanged`.

`jit init --profile jit-dogfood` combines the neutral init scaffold and profile
projection into one validated publication. For an existing repository,
`jit profile apply jit-dogfood` uses the same profile planner and publisher.
The request, result, error, and manifest schemas are available through
`jit --schema`; the same command family is exposed through the generated MCP
tools.

## The `jit-dogfood` package

The package is versioned independently from the JIT binary and declares its
compatible JIT range in a TOML manifest. The manifest is the package inventory:
run `jit profile show jit-dogfood --json` for the exact version, contributions,
assets, managed regions, hashes, and executable declarations carried by the
running binary.

The package installs:

- a repository-neutral issue taxonomy and the namespaces needed by its
  workflow;
- an epic-only plan-before-fan-out template, advisory content standards, and
  enforced epic completion coverage;
- plan, breakdown, code-review, coverage, issue-validation, and
  repository-validation gates;
- portable review prompts and scripts, the JIT agent skill suite, managed
  `AGENTS.md` guidance, and the configured invariant plus rules/gates
  projections.

The deterministic validation checkers run in process. The packaged plan,
breakdown, and code-review checkers are deliberately warning-only placeholders:
they pass with a visible structured warning until the repository replaces them
with real review integrations. A placeholder pass is sequencing evidence, not
review approval. See [Built-in Gate Presets](gate-presets.md) for checker
semantics and [Custom Gates](../how-to/custom-gates.md) for replacement.

The profile changes configuration and installs files, but it never creates Git
commits, rewrites existing issues, or treats the dogfood vocabulary as engine
behavior. A semantic registry entry or one-to-one asset that collides with
differing repository-owned content is a conflict; that content is not silently
overwritten.

## Derived state and managed regions

Declared configuration, registries, and applied-profile provenance are the
authorities for generated schemas, default-rule assertions, configured
projections, and installed profile targets. `jit validate` reports an owned
target that is missing, stale, has the wrong executable mode, or is unexpected.
For provenance-proven derived-state repairs, `jit validate --fix` recomputes
the complete final-state plan and publishes it in one recoverable transaction.
It preserves authored rule comments, order, policy, and all unmanaged document
bytes; it never treats a conventional filename as proof of ownership.

Managed regions replace only the package- or projection-owned marked region.
Distinct regions may nest, but delimiters must form one unambiguous containment
tree. Duplicate, partial, reversed, crossing, or multiply owned regions fail
before publication. Unresolvable provenance and ambiguous ownership are likewise
non-repairable and leave every target unchanged. Human output and the standard
JSON `VALIDATION_FAILED` error envelope retain the underlying target, delimiter,
or provenance cause so an operator can repair the authority rather than guess;
see the [machine-readable failure contract](cli-commands.md#cli-json-contracts).

## Publication, rollback, and recovery

Before writing, JIT captures the relevant repository state, builds a
deterministic final-state plan, and validates the complete overlay. It rebuilds
that plan while holding the repository-wide mutation lock before publication.
Package, path, conflict, and validation failures discovered before transaction
preparation leave repository targets unchanged.

Publication is a recoverable multi-file transaction, not a claim that the host
filesystem can atomically rename an entire directory tree. An ordinary handled
failure before the commit point is returned only after JIT restores and verifies
the exact old state. If interruption or rollback uncertainty prevents that
proof, JIT retains a typed durable journal in one of two machine-local control
areas:

- repository-sibling `.jit-bootstrap/`, marked by
  `transaction-protocol-v1`, for initialization when `.jit/` did not yet exist;
- `.jit/tmp/transactions/` for an existing repository.

When the selected data root does not exist, JIT assembles the complete root in a
verified sibling staging directory while the external bootstrap journal remains
outside it. One atomic no-replace rename publishes that complete root. An
occupied destination is never overwritten; the losing operation fails and a
retry treats the now-existing root through the ordinary existing-root path.

Every later mutating JIT command runs mandatory recovery before normal
repository services start. A prepared journal is rolled back to the old state; a
committed journal verifies the new state and completes cleanup. Recovery
therefore converges to all-old or all-new JIT-managed targets. It cannot protect
against unrelated programs modifying those paths outside JIT's locks, storage
checks, and recovery protocol.

An applied result may include a `transaction_cleanup_pending` warning after the
commit point. In that case the new repository state is authoritative and the
retained committed journal is cleanup work for mandatory recovery, not a failed
application.

Successful application writes a minimal provenance record at
`.jit/profiles/<profile-id>.json` (therefore the `jit-dogfood` ID selects the
matching filename) and appends the repository-scoped `profile_applied` audit
event. The record stores the profile ID, version, origin, package hash, and
per-target hashes used to recognize an exact reapplication.

The origin says where the applied bytes came from: either compiled into the
binary, or read from a repository directory, in which case it carries that
directory as a worktree-relative location. The record is the one place a
repository states where its package is, so a later run reads the same package
from the same location. That location is confined to the worktree — a package
read from outside it, including from under `.jit/`, is refused by name rather
than recorded — so re-reading a repository's package never depends on machine
state.

These repository-state guarantees do not change Git requirements. Core commands,
including init, profile application, project rendering, and validation, work
without Git. Claim leases remain the documented exception: their shared
coordination state lives under `.git/jit/`, so claim acquire/renew/release require
a Git repository. Profiles do not add or alter that lease surface.

## V1.0 lifecycle boundary

The v1.0 surface is intentionally apply-only. The following capabilities are
deferred to the post-1.0 profile epic and do not exist in this release:

- multiple profile composition;
- explicit local profile directories;
- profile dependencies and incompatibilities;
- variables and sensitive-value handling;
- semantic shared ownership;
- reconfiguration;
- detailed diff;
- three-way upgrade;
- safe removal.

There is no local-package discovery, composition flag, variable input,
profile-upgrade command, or profile-removal command hidden behind the v1.0
interface. Edit repository configuration directly for advanced customization,
or start from the manual guides below.

## Advanced customization

Use the profile when you want JIT's portable recommended workflow. Use the
manual references when you are designing a different methodology or need to
replace individual pieces:

- [Configuration](configuration.md) — hierarchy, namespaces, templates, and
  projection settings.
- [Adopt the Planning Bracket](../how-to/adopt-planning-bracket.md) — manually
  construct a different plan-before-fan-out ruleset.
- [Custom Gates](../how-to/custom-gates.md) — define or replace gate
  integrations.
- [Validation Rules](../how-to/validation-rules.md) — author repository policy.
- [Examples](../examples/README.md) — complete alternative configurations and
  focused rule examples.
