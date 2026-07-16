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
overwritten. Managed regions replace only the package-owned marked region and
preserve all prose outside its markers.

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
event. The record stores the profile ID, version, embedded origin, package hash,
and per-target hashes used to recognize an exact reapplication.

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
