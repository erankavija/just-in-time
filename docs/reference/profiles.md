# Repository Profiles

> **Diátaxis Type:** Reference

Repository profiles install a coherent JIT workflow as one explicit operation.
A profile is a package: a directory holding a manifest, the assets that
manifest declares, and the managed-region sources it owns. This is the
canonical adopter reference for the profile lifecycle: authoring and capture,
offline exchange, composition, variables and ownership, drift recovery,
reconfiguration, and upgrade. Use [`jit profile show --json`](cli-commands.md#jit-profile-show)
to inspect the exact packages a release or a repository currently carries.

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
registries, the event log and the index — and declares no workflow vocabulary.

## Author a version-2 manifest

Create a package as a directory with a root `manifest.toml`. The minimal
version-2 package needs no sources or declarations; this copyable manifest is
accepted by the package decoder as-is:

```toml
[profile]
manifest-version = 2
id = "my-workflow"
version = "1.0.0"
compatible-jit = "*"
```

`id` is a lowercase-kebab package identifier, `version` is a semantic version,
and `compatible-jit` is a semantic-version requirement for the JIT binary. The
version discriminator, identity, package version, and compatibility requirement
are all required. New packages use version 2: it has the `compatible-jit`
spelling above and hashes the authored manifest bytes as part of package
identity. The decoder also recognizes the older version-1 wire used by released
packages, but its `jit` compatibility spelling, bare dependency IDs, and lack
of variables, incompatibilities, and template opt-in are not this authoring
contract.

The rest of the top-level declarations are optional arrays of tables. Omit a
family to declare none of it; repeat `[[...]]` to declare several entries in
the order written. Version-2 manifests reject unknown fields. Every package
path is a non-empty, safe relative path: it cannot be absolute, traverse with
`..`, contain an empty path segment, or use a platform path prefix.

```toml
# Each block below is optional and repeatable.
[[dependency]]
id = "base-workflow"
version = ">=1.0.0, <2.0.0"

[[incompatibility]]
id = "legacy-workflow"
version = ">=1.0.0"

[[variable]]
name = "PROJECT_NAME"
default = "my-project"
env = "JIT_PROFILE_PROJECT_NAME"

[[asset]]
source = "assets/install/guide.md"
target = "docs/workflow.md"
executable = false
template = false

[[region]]
source = "regions/agents.md"
target = "AGENTS.md"
region-id = "my-workflow-guidance"
placement = "append"
template = false

[[live-source]]
root = "docs"
exclude = ["docs/drafts/**"]
```

### Package identity, compatibility, and variables

`[[dependency]]` and `[[incompatibility]]` each require a lowercase-kebab
`id` and a semantic-version `version` requirement. A package cannot depend on
itself. Dependencies are resolved before the package that declares them;
incompatibilities make a selection containing a matching installed or selected
package fail before publication.

A `[[variable]]` requires `name`; `default` and `env` are independently
optional. Both the profile variable name and the optional environment name use
`[A-Z][A-Z0-9_]*`, and a name is declared at most once. Variables are
non-secret public configuration: do not place credentials in them or in a
values file. Their resolution precedence is declaration default, values file,
the declared environment variable, then repeated `--set NAME=VALUE` (the last
`--set` wins).

Use `{{jit:var:NAME}}` only in a templated asset or region body, or in the
free-form prose fields of a supported semantic contribution. A templated body
must be UTF-8; an untemplated body may be binary but cannot contain that token.
Paths, IDs, versions, modes, targets, and constrained contribution fields are
literal and reject variable references. A reference with no resolved value
refuses the operation.

### Files, regions, and capture roots

An `[[asset]]` copies exactly one package file at required `source` to the
required repository `target`. `executable` and `template` are optional and
default to `false`. Set `executable = true` for a source that must be published
with executable mode; a source that is executable on disk but omits that
declaration is rejected.

An `[[region]]` inserts the required package `source` into a managed region in
the required repository `target`. `region-id` is a required lowercase-kebab
marker identity, and `placement = "append"` is currently the required placement
policy: it appends the delimited region if absent, then later refreshes only
that region. `template` is optional and defaults to `false`. An asset and a
region cannot share a source or a target, and package files other than
`manifest.toml` must be declared as one of those sources.

For a live asset that `jit profile capture` refreshes from the repository,
write its asset source below `assets/live/` and make its `target` the repository
file that supplies the bytes. The package does not need a checked-in copy of
that live source when capture creates its destination. Assets outside that
prefix, and every region source, are package-authored files read from the
package directory.

Each `[[live-source]]` declares one required repository-relative directory
`root`; its optional `exclude` list defaults to `[]`. Use shell-style,
repository-relative patterns in `exclude` (`*` stays within a path segment and
`**` spans segments). Roots cannot repeat or overlap. A root states the live
repository area the package intends to account for. When a package is intended
to capture a complete area, declare each live asset under that root or
explicitly exclude it, rather than silently omitting material.

### Semantic contributions

`[[contribution]]` publishes one semantic registry declaration instead of a
file. Every contribution requires `kind` and the fields in the matching row.
Its `value` is TOML data that represents the complete registry value, not a
partial patch. The target names below are stable profile-model vocabulary; the
names, tables, and policy values your repository declares belong to its
[Configuration reference](configuration.md), rather than to this repository's
dogfood configuration.

| `kind` | Required fields | Target and value contract |
|---|---|---|
| `scalar` | `target`, non-empty string `value` | `target` is one of `documentation-development-root`, `documentation-archive-root`, `validation-strictness`, or `validation-default-type`; it selects that scalar configuration setting. |
| `map-entry` | `target`, non-empty `identity`, `value` | `target` is `type-hierarchy-types`, `label-associations`, `namespaces`, or `item-kinds`; `value` must be the complete value for that configuration entry (a positive integer for a type-hierarchy entry, a string for a label association, and a complete table for a namespace or item kind). |
| `set-string` | `target`, non-empty string `value` | `target` is `strategic-types`, `documentation-managed-paths`, `documentation-permanent-paths`, or `documentation-issue-scoped-areas`; it adds one member to that configured string collection. |
| `keyed-array` | `target`, `identity`, complete-table `value` | `target` is `gates`, `invariants`, `rules`, or `templates`. The value must carry the same identity: `key` for a gate, `id` for an invariant, and `name` for a rule or template. The corresponding registry reference defines the rest of that table. |
| `projection` | `name`, table `value` | `value` requires `kind`, `mode`, `target`, and `style`, in the same shape as a [`[projection.<name>]` declaration](configuration.md#projectionname). Its `target` is a safe repository-relative path. |

Contributions have unique semantic identities within a package. In particular,
two entries cannot publish the same target and identity. Use the configuration
reference to author the complete table for a namespace, item kind, gate,
invariant, rule, template, or projection; the manifest carries that declared
value whole so it can be composed, validated, and captured without treating
repository-local vocabulary as an engine default.

## Commands

List the profiles this repository's own records name, and inspect one or more —
either profiles those records name, or packages at locations:

```bash
jit profile list
jit profile show --profile id:jit-dogfood
jit profile show --profile path:packages/jit-dogfood
```

Check every recorded profile against the package it came from and the content it
owns, which is what says whether a repository needs reconfiguring, upgrading, or
capturing:

```bash
jit profile validate
```

Report what a selection would change here, including the targets and registry
declarations it could not publish and who claims each of them, before publishing
any of it:

```bash
jit profile diff --profile path:packages/jit-dogfood
jit profile diff --profile id:jit-dogfood
```

Preview and apply a profile to an existing JIT repository:

```bash
# A first application names where the package was placed
jit profile apply --profile path:packages/jit-dogfood --dry-run
jit profile apply --profile path:packages/jit-dogfood

# Later runs need no location: the applied-profile record names it
jit profile apply --profile id:jit-dogfood --dry-run
```

Author a package by creating its manifest and declared sources in a worktree
directory. Capture refreshes a package tree from the repository targets its
manifest declares, which is how an edit made in place reaches the package that
owns it:

```bash
jit profile capture --source profiles/my-workflow --destination build/my-workflow --dry-run
jit profile capture --source profiles/my-workflow --destination build/my-workflow
```

That covers both classes of content a package owns and the repository authors:
the bytes of a live asset, and the value of a declared contribution. An adopter
who changes a contributed value in its registry captures the package that
published it and applies the result; the registry keeps the value they chose,
and the package and its applied-profile record agree with it again. Until they
do, [`jit profile diff`](cli-commands.md#jit-profile-diff),
[`jit profile validate`](cli-commands.md#jit-profile-validate), and `jit
validate` all name that declaration, and an application is refused rather than
reporting nothing to do.

Pack a package into one portable file to hand to somebody, and place one
somebody handed you:

```bash
jit profile pack --source profiles/my-workflow --output my-workflow.tar --dry-run
jit profile pack --source profiles/my-workflow --output my-workflow.tar
jit profile add --archive my-workflow.tar --destination packages/my-workflow --dry-run
jit profile add --archive my-workflow.tar --destination packages/my-workflow
```

The archive carries the package's identity digest, and adding one recomputes
that identity from the extracted content and refuses an archive that disagrees.
This detects an archive damaged or truncated in transit; it establishes
integrity rather than origin, so the channel the archive arrived over is still
what says who produced it.

`--output` for `jit profile pack` can be outside the repository. When it is
inside the repository, the output path may be at most 128 path components below
the repository root; choose an external path if a deeper output location is
needed. An existing output or add destination is never overwritten.

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
declarations. The refusal is taken from the same decision
[`jit profile diff`](cli-commands.md#jit-profile-diff) reports, so it names the
package being applied, the declaration or target it carries, and who already
holds it — the repository, or the package whose applied-profile record claims
it. Identical restatements merge, so a package and its dependency may declare
the same thing.

Profile enumeration and its handling of applied-profile records are defined in
[Profile Commands](cli-commands.md#profile-commands).

## Inspect, recover, and evolve

Use `jit profile diff` before a change to see every target and semantic
declaration a selection would create, update, retain, remove, or conflict on.
Each decision identifies every package that owns it; an empty owner list is
repository-authored content, while several owners identify shared content.
`jit profile validate` checks every applied-profile record against its package
and the content it owns. `jit validate` includes those findings in its
whole-repository validation. Both checks report drift without changing files.

Start with a rehearsal for any mutation. `--dry-run` runs the same planning and
validation as its corresponding `apply`, `reconfigure`, `upgrade`, `capture`,
`pack`, or `add` command, but writes no target, provenance record, or lifecycle
event. Unlike a rehearsal, `jit profile diff` reports all conflicts so an
operator can identify the conflicting target or declaration and its owner.

Choose the recovery action from the reported ownership and intended authority:

- Reconfigure an unchanged installed package when a declared variable needs a
  different value: `jit profile reconfigure --profile id:<PROFILE_ID> --set NAME=VALUE --dry-run`,
  then repeat without `--dry-run`.
- Capture a package when its owner intentionally adopts a changed declared live
  asset or semantic contribution from the repository: `jit profile capture --source <DIR> --destination <DIR>`.
  Capture refreshes every declared contribution it owns from the registry and
  leaves repository-authored declarations out of the package.
- Upgrade when the package directory contains a newer version: `jit profile upgrade --profile path:<DIR> --dry-run`,
  then repeat without `--dry-run`. Upgrade preserves shared content another
  installed package still owns and removes unchanged content the replaced
  package solely owned but no longer contributes.

An owned target that has drifted is a conflict for reconfigure or upgrade; the
conflict publishes nothing. Restore the intended owner’s content, or capture
the intended package change before applying or upgrading it. Do not use a
profile operation to overwrite repository-authored or another profile’s
conflicting content.

All profile commands support `--json` and use the standard count-wrapped
`{"count": N, "profiles": [...]}` collection shape. A show response contains
one package entry per selector occurrence in selector order, including repeated
selectors. Lifecycle rehearsals and runs report the same ordered observations:
dependency-only packages once before the selected roots, then every selected
root occurrence in selector order. A repeated root's later observations are
`unchanged` and carry no decisions. Each show entry carries the manifest, package
identity, target hashes, size, and stored provenance record when present.
Showing that record does not re-verify current target bytes. Use
`jit profile apply --profile id:jit-dogfood --dry-run` for exact current-state
verification of the named package and its dependency closure: it returns the
deterministic plan hash and every package's target and declaration decisions
without writing. Successful reapplication of an exact installation returns
`unchanged` for every package of the set.

Profiled `jit init` combines the neutral init scaffold and profile projection
into one validated, recoverable transaction. The scaffold and the complete
dependency-ordered profile selection are planned and published together; no
member of that selection is published separately. For an existing repository,
`jit profile apply` uses the same profile planner and publisher.
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
matching filename). Every changed or applied package has its own provenance
record. One repository-scoped `profile_lifecycle` audit event records the
operation and its per-profile outcomes for the complete selection. Each record
stores the profile ID, version, origin, package hash, resolved public variable
values with their source kinds, and sorted ownership claims. Each claim names
one semantic declaration, file target, or managed region together with its
published-base fingerprint and retention intent. That fingerprint is what
[`jit profile validate`](cli-commands.md#jit-profile-validate) holds the
repository's current content against, so it names the profile that diverged and
the contribution that did. A managed region whose published body encloses a
region another owner manages — a package's guidance region around a configured
projection — is checked for presence rather than content: that nested body is
rendered from what its own owner declares, so the enclosing profile's recorded
base is not the whole of what sits between its delimiters. Validation and repair
reuse the stored resolved variable values but load effective configuration from
the declared registries; they do not read the current process environment. The
audit event carries the resolved target hashes but never the resolved values.
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
