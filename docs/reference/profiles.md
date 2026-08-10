# Repository Profiles

> **Diátaxis Type:** Reference

Repository profiles install a coherent JIT workflow as one explicit operation.
A profile is a package: a directory holding a manifest, the assets that
manifest declares, and the managed-region sources it owns. JIT v1.0 publishes
two, `jit-dogfood`, the workflow package an adopter names, and `jit-default`,
the generic vocabulary it declares a dependency on. Applying `jit-dogfood`
applies both, so the dependency is never named.

## Obtaining a package

A release carries the package directories. The [asset
table](release-policy.md#what-a-release-publishes) is the complete list of what
a release publishes, and the [Installation
Guide](../../INSTALL.md#pre-built-binaries) covers downloading an archive,
verifying it, and where its contents land.

A package is applied from a location inside the repository worktree, so an
obtained directory is placed in the repository before it is applied. A package
read from outside the worktree — including from under `.jit/` — is refused by
name. Leave the directory where it was applied from: the applied-profile record
names that location, and `jit profile list`, `jit validate`, and
`jit validate --fix` read the package there again.

Place a package and the packages it depends on as siblings under one parent,
each directory named by its own profile ID — that is where [Profile
Commands](cli-commands.md#profile-commands) looks for a declared dependency:

```text
packages/
  jit-default/
  jit-dogfood/
```

Applying a package needs no network access, no Git repository, and no JIT
source checkout: the application reads the placed directory and writes the
repository. What it does need is bytes that were obtained beforehand.

For a new repository, this is the preferred setup:

```bash
mkdir my-project
cd my-project
cp -R <extracted-archive>/packages .
jit init --profile path:packages/jit-dogfood
```

Plain `jit init` remains the methodology-neutral alternative. It creates the
structural minimum — the schema version and the project name, beside the empty
registries, the event log and the index — and declares no vocabulary at all.
`jit-default` carries the generic vocabulary a repository needs to be usable
(the type hierarchy, the namespace registry, the item kinds, the validation
defaults, and the development-area classification), and `jit-dogfood` declares
this project's workflow on top of it.

## Commands

List the profiles this repository's own records name, and inspect one or more —
either profiles those records name, or packages at locations:

```bash
jit profile list
jit profile show --profile id:jit-dogfood
jit profile show --profile path:packages/jit-dogfood
```

Preview and apply a profile to an existing JIT repository:

```bash
# A first application names where the package was placed
jit profile apply --profile path:packages/jit-dogfood --dry-run
jit profile apply --profile path:packages/jit-dogfood

# Later runs need no location: the applied-profile record names it
jit profile apply --profile id:jit-dogfood --dry-run
```

Packages may declare non-secret variables. Supply a TOML values file containing
`[variables]` or repeat `--set NAME=VALUE`; precedence is declaration default,
values file, declared environment variable, then `--set`, with the last
`--set` winning. Variable references are available in templated asset and
region bodies and declared free-form prose fields; package paths, identities,
modes, and other constrained fields reject them.

The selector syntax and package-resolution contract are defined in [Profile
Commands](cli-commands.md#profile-commands). This page
describes the package and its lifecycle; the command reference covers how a
package obtained from a directory is selected and found again on later runs,
including where a declared dependency is looked for. A directory of obtained
packages applies as a set: naming one of them reaches its siblings without
naming them.

Applying a package applies the packages it declares a dependency on first,
transitively, so a package carrying a delta over another produces the same
repository as one carrying both. A package two others depend on is applied once.
Declared dependencies that close a cycle are rejected, naming the cycle, and a
dependency that resolves through no route fails the application naming the
package that declared it beside the one that could not be found — so an adopter
is never left diagnosing a package they did not name. Both refusals are raised
over the whole set before any of it is applied.

A registry declaration or asset target that two packages state differently is
refused for the same reason: neither package holds authority over the other's
declarations. The failure names the package being applied, the declaration or
target it carries, and who already holds it — the repository, or the package
whose applied-profile record claims that target. Identical restatements merge,
so a package and its dependency may declare the same thing.

Profile enumeration and its handling of applied-profile records are defined in
[Profile Commands](cli-commands.md#profile-commands).

All profile commands support `--json`. Profile commands that emit collections
use the standard count-wrapped collection shape. A show response
contains one package entry per selector occurrence in selector order, including
repeated selectors; a dry-run reports one plan entry per selected occurrence;
normal application reports one result per applied package, dependencies before
the package that declares them. Each show entry carries the manifest, package
identity, target hashes, size, and stored provenance record when present.
Showing that record does not re-verify current target bytes. Use
`jit profile apply --profile id:jit-dogfood --dry-run` for exact current-state
verification of the named package: it returns a one-entry collection with the
deterministic plan hash and every target's `create`, `update`, or `unchanged`
action without writing. Successful reapplication of an exact installation
returns `unchanged` for every package of the set.

Profiled `jit init` combines the neutral init scaffold and profile projection
into one validated publication; where the profile declares dependencies, the
first package of the resolved set is published with the scaffold and the rest
follow it in order. For an existing repository, `jit profile apply` uses the
same profile planner and publisher.
The request, result, error, and manifest schemas are available through
`jit --schema`; the same command family is exposed through the generated MCP
tools.

## The `jit-dogfood` package

The package is versioned independently from the JIT binary and declares its
compatible JIT range in a TOML manifest. The manifest is the package inventory:
run `jit profile show --profile path:packages/jit-dogfood --json` for the
exact version, dependencies, contributions, assets, managed regions, hashes, and
executable declarations the resolved package carries. Once the repository has
applied it, its own record can be selected with `id:jit-dogfood`. It also declares the repository roots from which
packaged live assets are drawn, so any repository file under a declared root
that is neither claimed by a packaged asset nor matched by a declared exclusion
is caught rather than silently left out of the package.

Applying it installs:

- the repository-neutral vocabulary its `jit-default` dependency declares, plus
  the types and namespaces its own workflow adds;
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
review approval. See [Gate Presets](gate-presets.md) for checker
semantics and [Custom Gates](../how-to/custom-gates.md) for replacement.

The profile changes configuration and installs files, but it never creates Git
commits, rewrites existing issues, or treats the dogfood vocabulary as engine
behavior. A semantic registry entry or one-to-one asset that collides with
differing content is a conflict; that content is not silently overwritten.

## Derived state and managed regions

Declared configuration, registries, and applied-profile provenance are the
authorities for generated schemas, default-rule assertions, configured
projections, and installed profile targets. `jit validate` reports an owned
target that is missing, stale, has the wrong executable mode, or is unexpected.
For provenance-proven derived-state repairs, `jit validate --fix` recomputes
the complete final-state plan and publishes it in one recoverable transaction.
It preserves authored rule comments, order, policy, and all unmanaged document
bytes; it never treats a conventional filename as proof of ownership.

What a recorded profile owns is recomputed from the package its own record
resolves to, read again from the location that record names. A stored record
that disagrees with the package read there fails validation naming the record,
so repair restores every profile-owned target or none. The command-level
package lookup and its failure behavior are defined in
[Profile Commands](cli-commands.md#profile-commands).

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

Successful application writes a canonical provenance record at
`.jit/profiles/<profile-id>.json` (therefore the `jit-dogfood` ID selects the
matching filename) and appends the repository-scoped `profile_applied` audit
event. Every applied package writes its own record and appends its own event,
so applying a package that declares a dependency leaves one record and one
event per package of the set. The record stores the profile ID, version, origin,
package hash, resolved public variable values with their source kinds, and
sorted ownership claims. Each claim names one semantic declaration, file target,
or managed region together with its published-base fingerprint and retention
intent. Validation and repair reuse the stored resolved variable values but load
effective configuration from the declared registries; they do not read the
current process environment. The audit event carries the resolved target hashes
but never the resolved values.
The record does not state why a package was applied, so it reads the same
whether the adopter named that package or received it as another's dependency.

The origin says where the applied bytes were read from: the repository
directory holding the package, carried as a worktree-relative location, so
re-reading a repository's package never depends on machine state. The command
reference defines how later commands use that recorded location.

These repository-state guarantees do not change Git requirements. Core commands,
including init, profile application, project rendering, and validation, work
without Git. Claim leases remain the documented exception: their shared
coordination state lives under `.git/jit/`, so claim acquire/renew/release require
a Git repository. Profiles do not add or alter that lease surface.

## V1.0 lifecycle boundary

The following capabilities are deferred to the post-1.0 profile epic and do not
exist in this release:

- sensitive-value handling;
- semantic shared ownership;
- detailed diff;
- safe removal.

There is no profile-removal command hidden behind the v1.0 interface. Package
lookup follows the command contract in
[Profile Commands](cli-commands.md#profile-commands);
no configured search path discovers a package. Edit repository configuration
directly for advanced customization, or start from the manual guides below.

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
