# Repository Profiles

> **Diátaxis Type:** Reference

Repository profiles install a coherent JIT workflow as one explicit operation.
A profile is a package: a directory holding a manifest, the assets that
manifest declares, and the managed-region sources it carries. Applying a
package renders it into the repository once, and from then on the repository
holds every value. This is the canonical adopter reference for authoring,
offline exchange, composition, and variables. Use [`jit profile show --json`](cli-commands.md#jit-profile-show)
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
name.

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
jit init --profile packages/jit-dogfood
```

When working from a source checkout rather than a release extraction, first
make portable archives from the checkout. This matters for `jit-dogfood`, whose
`assets/live/` files are drawn from the checkout's declared repository roots
and are not stored as duplicate authored files:

```bash
(cd <jit-checkout> && jit profile pack --source profiles/jit-default --output /tmp/jit-default.jitprof)
(cd <jit-checkout> && jit profile pack --source profiles/jit-dogfood --output /tmp/jit-dogfood.jitprof)

mkdir my-project
cd my-project
jit init
jit profile add --archive /tmp/jit-default.jitprof --destination packages/jit-default
jit profile add --archive /tmp/jit-dogfood.jitprof --destination packages/jit-dogfood
jit init --profile packages/jit-dogfood
```

The initial plain `jit init` only creates the structural repository needed by
`profile add`; the final profiled initialization applies the package and its
dependency from the directories the archives placed in the worktree.

Plain `jit init` remains the methodology-neutral alternative. It creates the
structural minimum — the schema version and the project name, beside the empty
registries, the event log and the index — and declares no workflow vocabulary.

## Author a manifest

Create a package as a directory with a root `manifest.toml`. The minimal
package needs no sources or declarations; this copyable manifest is accepted by
the package decoder as-is:

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
are all required. `manifest-version` is `2`, the only manifest version; the
decoder rejects any other value. Package identity hashes the authored manifest
bytes.

The rest of the top-level declarations are optional arrays of tables. Omit a
family to declare none of it; repeat `[[...]]` to declare several entries in
the order written. Manifests reject unknown fields. Every package path is a
non-empty, safe relative path: it cannot be absolute, traverse with `..`,
contain an empty path segment, or use a platform path prefix.

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
incompatibilities make a selection containing a matching selected package fail
before publication.

A `[[variable]]` requires `name`; `default` and `env` are independently
optional. Both the profile variable name and the optional environment name use
`[A-Z][A-Z0-9_]*`, and a name is declared at most once. Variables are
non-secret public configuration: do not place credentials in them or in a
values file. Their resolution precedence is declaration default, values file,
the declared environment variable, then repeated `--set NAME=VALUE` (the last
`--set` wins).

Use `{{jit:var:NAME}}` only in a templated asset or region body, or in the
following contribution fields: a namespace entry's `description`; a gate's
`title`, `description`, or `checker.prompt`; or an invariant, rule, or template
entry's `description`. A templated body must be UTF-8; an untemplated body may
be binary but cannot contain that token. All other contribution strings, plus
paths, IDs, versions, modes, and targets, are literal and reject variable
references. A reference with no resolved value refuses the operation.

### Files, regions, and live roots

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

For a live asset that `jit profile pack` draws from the repository, write its
asset source below `assets/live/` and make its `target` the repository file that
supplies the bytes. The package does not need a checked-in copy of that live
source: packing assembles the tree in memory before writing its archive. Assets
outside that prefix, and every region source, are package-authored files read
from the package directory.

Each `[[live-source]]` declares one required repository-relative directory
`root`; its optional `exclude` list defaults to `[]`. Use shell-style,
repository-relative patterns in `exclude` (`*` stays within a path segment and
`**` spans segments). Roots cannot repeat or overlap. A root states the live
repository area the package intends to account for. When a package is intended
to account for a complete area, declare each live asset under that root or
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
| `keyed-array` | `target`, `identity`, complete-table `value` | `target` is `gates`, `invariants`, `rules`, or `templates`. The value must carry the same identity: `key` for a [gate](storage-format.md#gate-registry), `id` for an [invariant](storage-format.md#invariants-registry), and `name` for a [rule](storage-format.md#validation-rules-registry) or [template](storage-format.md#graph-template-registry). The linked registry references define the rest of each table; [Author Validation Rules](../how-to/validation-rules.md#anatomy-of-a-rule) gives rule anatomy. |
| `projection` | `name`, table `value` | `value` requires `kind`, `mode`, `target`, and `style`, in the same shape as a [`[projection.<name>]` declaration](configuration.md#projectionname). Its `target` is a safe repository-relative path. |

Contributions have unique semantic identities within a package. In particular,
two entries cannot publish the same target and identity. Use the
[Configuration reference](configuration.md) for namespaces, item kinds,
hierarchy, documentation, and projections; use the linked registry references
above for gates, invariants, rules, and templates. The manifest carries each
declared value whole so it can be composed, validated, and packed without
treating repository-local vocabulary as an engine default.

## Commands

Inspect one or more packages at their locations:

```bash
jit profile show --profile packages/jit-dogfood
```

Report what a selection would change here, including the targets and registry
declarations it could not publish, before publishing any of it:

```bash
jit profile diff --profile packages/jit-dogfood
```

Preview and apply a profile to an existing JIT repository:

```bash
jit profile apply --profile packages/jit-dogfood --dry-run
jit profile apply --profile packages/jit-dogfood
```

A selector is a bare path to a package directory in the worktree. `--profile`
repeats to select several packages.

Author a package by creating and editing its manifest and declared sources in
a worktree directory. A package author edits the manifest directly, and
`jit profile pack` draws each live asset from the repository file its
declaration targets.

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

For a declared profile variable, supply a TOML values file containing
`[variables]` or repeat `--set NAME=VALUE`. See [Package identity,
compatibility, and variables](#package-identity-compatibility-and-variables)
for declaration, reference, and precedence semantics.

The package-resolution contract is defined in [Profile
Commands](cli-commands.md#profile-commands). This page describes the package
and its composition; the command reference covers where a declared dependency
is looked for. A directory of obtained
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

When a package and a package it depends on, directly or transitively, declare
one registry entry, asset target, or managed region, the dependent package's value
is composed, so a package depending on two others settles what they disagree on
by stating a value. Two packages with no dependency path between them that state
one differently, with no dependent stating a value over them, are refused:
neither holds authority over the other's declarations. The refusal is taken
from the same decision [`jit profile diff`](cli-commands.md#jit-profile-diff)
reports, so it names the package being applied, the declaration or target it
carries, and every package whose differing definition remains. Only the packages
being applied are parties: the repository's current content never conflicts with
a package. Identical restatements merge. The result does not depend on the order
packages are selected in.

Applying writes the composed value over an existing entry under the same
identity, replacing the whole entry; fields the package does not state are
dropped. A committed local edit to a declared entry or target is replaced by the
next application, and git keeps the prior version.

In a git repository an application refuses while a path it would write differs
from HEAD — modified, staged, or untracked — with
[`PROFILE_UNCOMMITTED_TARGETS`](error-codes.md) naming each such path. Each
written path is judged by the checkout that holds it, a path in no checkout is
never refused, and dirty paths the application does not write do not matter.
Commit or discard the named changes, then apply again; a refusal leaves the
repository byte-identical. Without git, an application proceeds, so read
[`jit profile diff`](cli-commands.md#jit-profile-diff) first. Profiled
`jit init` over an absent data root is exempt, as it has no earlier content to
protect.

## Inspect and rehearse

Use `jit profile diff` before a change to see every target and semantic
declaration a selection would create, update, leave unchanged, or conflict on. An
existing declaration whose value differs reports as `update`, because
publication replaces the whole entry under its identity with the package's value.
`diff` and `apply` reports list every file the publication writes, with the
exception of event-log appends and created directories.

Start with a rehearsal for any mutation. `--dry-run` runs the same planning and
validation as its corresponding `apply`, `pack`, or `add` command, but writes no target or lifecycle
event, so it fails where the command would, including on final validation of
the proposed repository. Unlike a rehearsal, `jit profile diff` reports all
conflicts so an operator can identify each conflicting target or declaration and
the packages defining it, and states a final-validation refusal, such as a
configured projection left stale, as status `would_conflict` carrying a
`refusal`.

A refused publication's `PROFILE_CONFLICT` error carries
`error.details.conflicts[]` in the [standard `--json` error
envelope](cli-commands.md#cli-json-contracts), one entry per refused target or
declaration as `{owner, subject, conflict}`; `conflict.owners` names the
packages defining it differently. `jit --schema` publishes this shape as
`types.ProfileTargetConflictsError`.

Profile list responses use the shared shape described in the
[CLI JSON contracts](cli-commands.md#cli-json-contracts), with `profiles` as the
collection key. A show response contains
one package entry per selector occurrence in selector order, including repeated
selectors. Lifecycle rehearsals and runs report the same ordered observations:
dependency-only packages once before the selected roots, then every selected
root occurrence in selector order. A repeated root's later observations are
`unchanged` and carry no decisions. Each show entry carries the manifest, package
identity, target paths, and size. Showing a package does not re-verify current
target bytes. Use
`jit profile apply --profile packages/jit-dogfood --dry-run` for exact current-state
verification of the named package and its dependency closure: it returns every
package's target and declaration decisions without writing. Applying content the repository already
holds publishes nothing and returns `unchanged` for every package of the set.

Profiled `jit init` combines the neutral init scaffold and profile projection
into one validated, recoverable publication. The scaffold and the complete
dependency-ordered profile selection are planned together and reach the one
commit point [Publication and recovery](#publication-and-recovery) describes;
no member of that selection is published separately. For an existing repository,
`jit profile apply` uses the same profile planner and publisher.
The request, result, error, and manifest schemas are available through
`jit --schema`.

## The `jit-dogfood` package

The package is versioned independently from the JIT binary and declares its
compatible JIT range in a TOML manifest. The manifest is the package inventory:
run `jit profile show --profile packages/jit-dogfood --json` for the
exact version, dependencies, contributions, assets, managed regions, package hash, and
executable declarations the resolved package carries. It also declares the repository roots from which
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
behavior.

## Derived state and managed regions

Declared configuration and registries are the authorities for generated
schemas, default-rule assertions, and configured projections. `jit validate`
reports a derived target that is missing, stale, or unexpected, and
`jit validate --fix` recomputes the complete final-state plan and publishes it
through one redo record, as every other mutation publishes.
It preserves authored rule comments, order, policy, and all unmanaged document
bytes.

A value a package contributed is repository content: editing it is an ordinary repository change
that git shows and restores, and `jit validate` neither reports nor repairs it.

Managed regions replace only their marked region.
Distinct regions may nest, but delimiters must form one unambiguous containment
tree. Duplicate, partial, reversed, crossing, or repeated regions fail
before publication. Ambiguous structure is likewise non-repairable and leaves
every target unchanged. Human output and the standard JSON `VALIDATION_FAILED`
error envelope retain the underlying target or delimiter cause so an operator
can repair the authority rather than guess;
see the [machine-readable failure contract](cli-commands.md#cli-json-contracts).

## Publication and recovery

Before writing, JIT captures the relevant repository state, builds a
deterministic final-state plan, and validates the complete overlay. Package,
path, conflict, and validation failures discovered before publication leave
every repository target unchanged.

A profile application publishes through one redo record, the record every JIT
mutation publishes through; [Publication and
Recovery](storage-format.md#publication-and-recovery) in the storage reference
defines what the record carries, its commit point, and how a session open
applies a record an interruption left. A profile adds no publication path of its
own. In a repository whose data root exists, an application's worktree targets
— assets and managed regions — and its data-root targets — the registries its
contributions reach and its `profile_lifecycle` event
— are operations of that one record, so an application never commits its
worktree half apart from its data-root half.

Profiled `jit init` publishes a data root that does not exist yet. When the
application has worktree targets, its record is committed inside the new root
before that root is published, so one rename commits the data root and the
record carrying those worktree targets together; an application with no
worktree targets publishes the new root alone. An occupied destination is never
overwritten; the losing operation fails, and a retry proceeds through the
ordinary existing-root path.

One repository-scoped `profile_lifecycle` audit event records the operation
and its per-profile outcomes for the complete selection, published by the same
redo record as the targets. A package's outcome is decided by what its
application publishes, so applying identical content again publishes nothing
and reports the package unchanged. The event records each profile's id, version
and the variable values applied. The repository keeps no other account of which
packages were applied: once applied, every value is repository content.

These repository-state guarantees do not change Git requirements. Core commands,
including init, profile application, project rendering, and validation, work
without Git. Profiles do not add or alter the documented Git requirements; see
the canonical [Git Optional](../concepts/guarantees.md#git-optional) guarantee.

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
