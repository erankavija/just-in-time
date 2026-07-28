# Configuration Reference

> **Diátaxis Type:** Reference

Complete reference for JIT configuration options.

For the portable recommended workflow, initialize with
`jit init --profile jit-dogfood`; the
[Repository Profiles reference](profiles.md) owns that package's exact contract.
This page is the advanced manual surface for repositories that want to inspect
or customize individual settings.

**Quick links:**
- [Example config.toml](example-config.toml) - Full annotated example with all options
- [Schema Configuration](#schema-configuration) - Issue types, validation, namespaces
- [`[documentation]`](#documentation) - Development-area classification and issue artifact directories
- [Runtime Configuration](#runtime-configuration) - Lease enforcement and compatibility fields

## Configuration Files

| File | Purpose |
|------|---------|
| `.jit/config.toml` | Repository config (schema + runtime) |
| `.jit/templates.toml` | Graph templates and their [role/anchor bindings](#template-bindings-jittemplatestoml) |
| `~/.config/jit/config.toml` | User defaults |
| `~/.config/jit/agent.toml` | Agent identity |
| `/etc/jit/config.toml` | System defaults |

Priority (how the merged effective configuration is resolved, e.g. for `jit config`): environment variables > repository > user > system > hardcoded defaults. Some execution paths (structural lease enforcement, claim coordination limits) read the repository `.jit/config.toml` directly rather than through this merge.

---

## Schema Configuration

These settings define how issues are organized and validated. See [example-config.toml](example-config.toml) for full annotated examples.

### `[version]`

```toml
[version]
schema = 2
```

Schema version. Required for newer features like namespace registry and documentation lifecycle.

### `[documentation]`

```toml
[documentation]
development_root = "dev"
archive_root = "dev/archive"
managed_paths = [
  "dev/active",
  "dev/studies",
  "dev/sessions",
  "dev/plans",
  "dev/presentations",
  "dev/design",
  "dev/benchmarks",
  "dev/experiments",
]
permanent_paths = [
  "dev/architecture",
  "dev/eval",
  "dev/vision",
  "dev/index.md",
  "dev/TESTING.md",
  "dev/authoring-conventions.md",
]
issue_scoped_areas = [
  "dev/active",
  "dev/studies",
  "dev/plans",
  "dev/presentations",
]
```

Controls document lifecycle management. The table above is the one `jit init`
scaffolds into a new repository, rendered from the `SHIPPED_DOCUMENTATION_POLICY`
declaration in `crates/jit/src/config.rs`, the single source of that
classification; `jit config get documentation` reports the table a repository is
running under.

Path vocabulary is repository policy: an adopter reclassifies any area, adds
areas of their own, or drops a convention entirely, and every command reads the
table in front of it. This repository's `.jit/config.toml` is the dogfood policy
under which jit itself is developed and must not be read as the shipped
declaration.

#### Development-area classification

An area's class states what archival does to the artifacts inside it, and it
follows from what the area holds. A managed area holds revision-specific work
products of one issue, so archiving that issue takes them with it: selected
documents in `managed_paths` move to the archive mirror. A permanent area holds
living documentation, a configured item-kind source, or a documented invocation
path, so its source stays where readers and configuration already point:
selected documents in `permanent_paths` are copied to the mirror while their
source remains in place; “permanent” prevents source deletion, not mirror
publication. The table above assigns each shipped area to one of the two
classes.

`development_root` is the outer boundary of both. A document it does not
contain is retained exactly where it is — the plan schedules no destination for
it, and artifact discovery stops following its links rather than drawing what it
cites into the plan — so declare managed and permanent areas inside the
development root. This is what keeps an archive from relocating source files,
scripts, agent assets, and repository-root documents that a development
document happens to link, and it applies whichever area claims the path.

An entry in either path list names a directory or an individual file. A
directory entry classifies every artifact beneath it; a file entry classifies
exactly that one path. Development-root documents that belong to no area are
therefore listed one file at a time, since an entry for the root itself would
classify every area under it and collapse the managed/permanent split.

#### Archival policy completeness

Dependency-aware `jit archive document` and `jit archive container` planning
and `--execute` classify this table by authored completeness. The read-only
`jit archive candidates` report consumes the same policy: for every candidate
it derives and reports `configured`, `incomplete`, or `unconfigured`, without
authorizing execution or substituting accessor defaults. For target planning
and execution, mutation-authorizing policy is `configured` only when all three
of these keys are explicitly present:

| Required key | Archive-planner meaning |
|--------------|-------------------------|
| `managed_paths` | Repository-relative component roots whose selected artifacts may relocate |
| `permanent_paths` | Repository-relative component roots whose source artifacts must remain |
| `archive_root` | Repository-relative mirror root for proposed destinations |

If the table is absent, plans report `unconfigured`. If it exists but any of
the three keys is absent, previews report `incomplete`. Both statuses make a
plan ineligible and make `--execute` refuse mutation. Explicit empty
arrays still count as authored fields; their policy meaning is deliberately
different from an omitted key.

The `DocumentationConfig` accessors retain fallback values for display callers,
but archive planning and execution never use those fallbacks to claim
eligibility. This prevents a partial policy from silently authorizing mutation.

#### Issue artifact directories

`issue_scoped_areas` declares which areas organize their artifacts one directory
per issue; every other area keeps its artifacts flat. An area is named whole:
membership is exact-area equality under lexical path normalization, so `<area>`,
`./<area>`, and `<area>/` name the same area while a path *inside* a declared
area is not itself one. An absent key resolves to the shipped declaration shown
above; an authored list replaces that declaration whole, so an empty list opts
every area out of the convention. Adoption is independent of the archival
classification — an area may be managed and issue-scoped, managed and flat,
permanent and issue-scoped, or neither — and an issue-scoped area archives its
artifacts exactly as a flat one does. The key is not part of the three-key
completeness that authorizes archival mutation.

The directory an issue owns inside a declared area is `<area>/<short-id>-<slug>`
when the issue resolves a single membership value, and `<area>/<short-id>`
otherwise. The membership value resolves in three steps: the issue's single
`type:` label, that type's membership namespace from
[`[type_hierarchy].label_associations`](#type_hierarchy), then a single value of
that namespace on the issue; the slug is that value normalized, the same suffix
form an archive destination directory carries. Normalizing lowercases the
value's alphanumeric characters, collapses each run of everything else into a
single `-`, bounds the result at 48 characters, and drops a trailing separator.
Every other shape — no type label, several of them, a type the mapping does not
name, no membership value, several of them, or a value that normalizes to
nothing — names the bare short-id directory. The name comes from labels and the
short id alone, so renaming an issue leaves its directory where it is. Filenames carry no short-id prefix:
the directory already names the issue, so each file names its own artifact
(`plan.md`, `breakdown.json`).

`jit doc dir <id> <area>` prints that directory, which is how a caller obtains
the path instead of composing the name itself; it substitutes straight into a
command, as in `mkdir -p "$(jit doc dir <id> <area>)"`. The answer is a name
rather than a reading of the tree, so it resolves the same before anything is
written there, and an area the repository does not declare is rejected rather
than resolved.

The convention governs artifacts as they are created. Artifacts already sitting
flat in a declared area stay resolvable and are never restructured in place;
`jit doc conformance` lists them as advice, mutating nothing and blocking no
state transition.

A template node names its document inside its container's directory by declaring
an area — see [Node document fields](#node-document-fields).

#### Citation scan roots

`citation_scan_roots` names the repository-relative roots an in-content
citation scan reads: it matches a moving artifact's path anywhere in a scanned
file's text — a shell-script line, a doc comment, an inline code span, or a
markdown link target alike. Each entry is a directory, reaching every file
beneath it, or an individual file, reaching exactly that path, matched the
same way as `managed_paths` and `permanent_paths` above. The key is optional;
`jit init` does not scaffold it, so a fresh repository's `[documentation]`
table omits it, and an absent key resolves to the development root together
with `permanent_paths` (both shown above). An authored list replaces that
default outright rather than extending it, and its entries need not lie under
the development root, since a citation a move can break may live wherever the
repository writes it. Like `issue_scoped_areas`, the key is not part of the
three-key completeness that authorizes archival mutation, so omitting it
leaves archive eligibility unchanged.

### `[type_hierarchy]`

```toml
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
story = "story"
```

| Field | Description |
|-------|-------------|
| `types` | Type name → hierarchy level (lower = more strategic) |
| `strategic_types` | Types shown in `jit query strategic` |
| `label_associations` | Type → membership label namespace mapping |

The hierarchy is repository configuration: its type names are not a fixed JIT
vocabulary. The `jit init` template uses the four types shown above. This
repository's dogfood configuration additionally declares `bug` and
`enhancement`; those are local choices, not shipped defaults.

### `[validation]`

```toml
[validation]
strictness = "loose"
default_type = "task"
content_format = "markdown"
```

| Field | Description |
|-------|-------------|
| `strictness` | Repo-wide enforcement modulator: `"strict"`, `"loose"` (default), `"permissive"` |
| `default_type` | Auto-assign when no type:* label |
| `content_format` | Default body parser: `"markdown"` (default), `"html"`, `"xml"` |

**Strictness** globally modulates which rule violations block a write or state
transition, layered on top of each rule's per-rule `enforce` flag and `severity`
(defined in `rules.toml`). It never changes a rule's severity or `enforce` flag —
only the block/allow decision:

- `strict` — any violation blocks, whether a warning or an error, enforced or not.
- `loose` (default) — only an enforced error blocks; every other finding is an
  advisory warning.
- `permissive` — nothing blocks; every violation is reported as an advisory
  warning.

An unrecognized value is rejected. Under `--force`, a blocked write or transition
proceeds and the bypass is logged, at every level.

> **Validation enforcement lives in `.jit/rules.toml`.** The ruleset `jit
> validate` and write-validation enforce is declared there, scaffolded by `jit
> init`: label/type format, the namespace registry, per-namespace uniqueness, the
> orphan-leaf / strategic-consistency warnings, and any custom rules you author.
> The built-in rules marked `origin = "default"` derive their assertion — and the
> membership of the `namespace-unique-*` family — from the `[namespaces]` /
> `[type_hierarchy]` registry in `config.toml`, in memory at load; the
> `schemas/default-*.json` files are regenerated projections, not the validation
> authority. So you change what a default rule checks by editing that registry
> (a hand-declared namespace takes effect on the next command, no regeneration;
> the next jit-driven config write — `jit config set` or re-init — also writes
> the matching `namespace-unique-*` row through to `rules.toml` so its
> `@/rule/<name>` address resolves), and author new conventions as custom rules
> in `rules.toml`. `strictness` tunes how all of these gate operations globally.

### `[namespaces.*]`

```toml
[namespaces.epic]
description = "Epic membership"
unique = false
examples = ["epic:auth", "epic:docs"]
```

Declare label namespaces (taxonomy: `description`, `unique`, `examples`). The
registry drives the `namespace-registry` and `namespace-unique-<ns>`
rules. Allowed-value enums, value patterns, and required-ness are NOT configured
here — author them as rules in `.jit/rules.toml`.

Rule names are colon-free slugs (`namespace-registry`, `namespace-unique-team`,
`label-format`, etc.). A rule's origin (`default` for the built-in rules,
`bracket` for those a bracket criterion installs) is a separate `origin` field
on its `.jit/rules.toml` entry, not part of its name. Every rule is addressable
at `@/rule/<self-id>` (`self-id` being its `name`), e.g. `@/rule/label-format`.

### `[projection.<name>]`

```toml
[projection.invariants]
kind = "invariant"
mode = "region"
target = "AGENTS.md"
style = "id-anchor"

[projection.rules-and-gates]
kind = ["rule", "gate"]
mode = "separate-file"
target = "docs/reference/rules-and-gates.md"
style = "full"
```

Declare a documentation projection: `jit project render` writes one or more
addressable item kinds into a documentation file, keeping the rendered copy in
sync with the kind's declared source of truth — a TOML registry for
registry-first kinds, a markdown document for markdown-first kinds — so a
hand-maintained duplicate never drifts. The mechanism is generic — any
project-scoped kind projects this way. Fields:

| Field | Values | Meaning |
|-------|--------|---------|
| `kind` | a kind name, or an array of names | **Required.** The addressable item kind(s) to render (e.g. `"invariant"`, or `["rule", "gate"]`); an array must name at least one kind — an empty list is rejected at parse. Every kind must be project-scoped; issue-scoped kinds are not renderable. |
| `mode` | `region` \| `separate-file` | `region` rewrites only the delimited block inside an existing `target`, byte-preserving everything outside it; `separate-file` writes the whole `target` file. Defaults to `separate-file`. |
| `target` | repo-relative path | **Required.** The documentation file written. There is no default — a projection with no `target` is an error. |
| `style` | `id-anchor` \| `full` | `id-anchor` renders generic `- **{self-id}** — {text}` bullets and works for any project-scoped kind; `full` renders the built-in rich views (severity/enforcement metadata) and is limited to the registry kinds whose declared `source` is exactly the invariant store (`.jit/invariants.toml`) or exactly the rule + gate stores together (`.jit/rules.toml` + `.jit/gates.toml`) — the only sources with a whole-file renderer. A markdown-first kind, or a registry kind whose source points elsewhere, must use `id-anchor`. Defaults to `full`. |
| `region-begin` / `region-end` | marker strings | `region`-mode delimiters. Default to `<!-- jit:<name>:begin -->` / `<!-- jit:<name>:end -->`, derived from the projection name. |

Run `jit project render` (optionally `--name <name>` for one) after editing a
projected kind's source of truth — its registry, or its markdown document for a
markdown-first kind; `jit validate` reports a stale target. This repository's own
three projections (`invariants`, `charter`, `rules-and-gates`) are repo-local
configuration, not a shipped default.

### Template bindings (`.jit/templates.toml`)

A `[[template]]` names its own nodes with arbitrary `role`s and its own anchor
slots with arbitrary `name`s. Three of those names carry MEANING for the bracket
tooling, and the top-level `[roles]` and `[anchors]` tables tell jit which ones:

```toml
# .jit/templates.toml
[roles]
planning  = "spec"     # the role of the node that holds the plan
breakdown = "split"    # the role of the node that holds the fan-out

[anchors]
container = "target"   # the anchor `jit apply <template> <container>` auto-binds

[[template]]
name       = "plan"
applies_to = ["epic"]
  [[template.anchors]]
  name = "target"
  [[template.nodes]]
  role = "spec"
  type = "planning"
  # ...
```

| Key | Default | What it binds |
|-----|---------|---------------|
| `roles.planning` | `planning` | The node whose `doc` gives the plan document's location, whose first gate is the plan-quality gate bracket breakdown requires, and whose `type` locates the applied bracket's planning node |
| `roles.breakdown` | `breakdown` | The node bracket breakdown consumes, whose type bounds `jit validate --scope`, and which `jit apply --force` locates the applied bracket by |
| `anchors.container` | `container` | The anchor `jit apply <template> <container>` binds to its positional `<container>` argument, so no `--anchor` flag is needed |

Both tables are optional, and so is each key within them. A repository that names
its roles and anchor the default way declares neither table. The defaults are the
`DEFAULT_PLANNING_ROLE`, `DEFAULT_BREAKDOWN_ROLE`, and `DEFAULT_CONTAINER_ANCHOR`
constants in `crates/jit/src/templates.rs`, which are their single source of
truth.

A binding names a role or anchor; it never invents one. If `roles.breakdown` names
a role no template node declares, that template simply has no breakdown node, and
the commands that need one say so.

These bindings are the only names the bracket tooling reads from configuration.
Everything else a template refers to by role names it in place: a
`[[template.transforms]]` entry carries its own `role` field naming the node it
targets, so `move-upstream-to-role` moves the container's pre-apply upstream
dependencies onto whichever declared role that entry names.

#### Node document fields

A `[[template.nodes]]` entry's `doc` names the document the created node carries,
and `doc_area` names the issue-scoped area that document belongs in. With an area
declared, the `{container.dir}` token inside `doc` interpolates to the canonical
artifact directory the container owns in that area
([Issue artifact directories](#issue-artifact-directories)), so
`doc = "{container.dir}/plan.md"` writes the plan into the container's own
directory with the filename naming the artifact alone. The declared area is
matched against `issue_scoped_areas`, and an area the registry does not declare
fails the apply rather than producing a path outside the convention. A node that
declares no `doc_area` leaves `{container.dir}` out of scope: the token stays
verbatim in the interpolated path, so such a node's `doc` names its own location.
`jit doc dir <container> <area>` prints the directory the token resolves to.

### Rule selectors (`.jit/rules.toml` `when`)

Each rule's `when` table selects which issues it applies to. All present
dimensions are AND-combined; an empty `when` matches every issue. The full
authoring guide is in
[How to define validation rules](../how-to/validation-rules.md); the selector
grammar is:

| Key            | Type            | Matches issues…                          |
|----------------|-----------------|------------------------------------------|
| `type`         | string          | whose `type:<value>` label equals this   |
| `label`        | string          | carrying this label; supports `ns:*`      |
| `state`        | string or list  | in one of these lifecycle states         |
| `has_doc_type` | string          | with a document of this `doc_type`        |

The `state` predicate accepts a single state or a list of states and matches
when the issue is in any of them:

```toml
when = { type = "epic", state = "in_progress" }              # single state
when = { state = ["ready", "in_progress", "gated"] }         # any of several
```

Valid state tokens: `backlog`, `ready`, `in_progress`, `gated`, `done`,
`rejected`, `archived`. An unknown state name is rejected at load with an error
naming the rule and listing the valid tokens.

### Graph rule scope semantics (`.jit/rules.toml` `scope`)

Several graph rule kinds accept a `scope` key in their `assert` table that
controls which issues are consulted when resolving labels or checking uniqueness.
The three scope values and the kinds that accept them are:

| Scope value | Meaning                                 | Accepted by         |
|-------------|-----------------------------------------|---------------------|
| `"linked"`  | Only issues linked by a dependency edge | `label-reference`   |
| `"global"`  | Any issue in the repository             | `label-reference`   |
| `"all"`     | Any issue in the repository (repo-wide) | `label-uniqueness`  |

**`"linked"` vs `"global"` (label-reference):** `scope = "linked"` constrains
reference resolution to issues connected by a dependency edge in either
direction. `scope = "global"` (the default) resolves against any issue in the
repository. Use `"linked"` when a reference should only resolve within the same
epic's dependency graph; use `"global"` when the reference must resolve globally
regardless of graph structure.

**`"all"` (label-uniqueness only):** `"all"` is reserved exclusively for
`label-uniqueness`. It means "across the entire repository" and is the only
permitted scope for that kind. `"all"` is a distinct token from `"global"` to
keep the two semantics explicit: `label-reference`'s `global` resolves
references; `label-uniqueness`'s `all` detects ownership collisions.

**Transition-time limitation:** rules that use `scope = "all"` (i.e.
`label-uniqueness`) run ONLY in `jit validate`. They are skipped at transition
time because transition enforcement evaluates only the issue's dependency
neighborhood, not the whole repository, and repo-wide uniqueness cannot be
determined from a neighborhood slice. `label-reference` with `scope = "global"`
is similarly skipped at transition time for the same reason. Run `jit validate`
after adding or changing labels that are subject to uniqueness rules.

---

## Runtime Configuration

### `[worktree]` Section

The active setting is lease enforcement for structural issue operations.

```toml
[worktree]
enforce_leases = "strict"  # Lease enforcement level
```

#### `enforce_leases`

| Value | Description |
|-------|-------------|
| `"strict"` | Require an active lease for structural issue operations; fail without one |
| `"warn"` | Warn if no lease but allow operation |
| `"off"` | No lease enforcement |

With no `[worktree]` section, command execution uses `off`. If the section is
present but `enforce_leases` is omitted, it resolves to `strict`.

### `[coordination]` Section

The two limits below are enforced when `jit claim acquire` creates an
indefinite (`--ttl 0`) lease.

```toml
[coordination]
max_indefinite_leases_per_agent = 2
max_indefinite_leases_per_repo = 10
```

#### `max_indefinite_leases_per_agent`

Maximum indefinite (TTL=0) leases per agent. Default: `2`.

#### `max_indefinite_leases_per_repo`

Maximum indefinite leases across entire repository. Default: `10`.

`default_ttl_secs`, `lease_renewal_threshold_pct`, and `stale_threshold_secs`
are accepted and shown by config commands, but currently do not alter claim
timing. Timed claims therefore use `jit claim acquire --ttl` (defaulting to the
built-in claim lease TTL — see
[Runtime coordination defaults](#runtime-coordination-defaults)), and an
indefinite lease needs explicit `jit claim heartbeat` calls.

### Runtime coordination defaults

The built-in lock acquisition timeout and poll interval, orphaned temp-file
cleanup threshold, and claim lease TTL — each with its unit and operational
scope — are listed in
[Runtime Coordination Defaults](runtime-defaults.md). That reference is
projected from `crates/jit/src/runtime_defaults.rs`, the single source those
defaults are read from, so the values there never drift from the code.

### Parsed compatibility fields

`worktree.mode`, `[global_operations]`, `[locks]`, and `[events]` values are
accepted and available to `jit config get` / `jit config show`, but no current
command applies them as runtime controls. Do not use them to change worktree
handling, branch policy, lock recovery, or event format.

## Agent Config (`~/.config/jit/agent.toml`)

Agent-specific configuration for persistent identity.

```toml
[agent]
id = "agent:my-agent"
created_at = "2026-01-01T00:00:00Z"
description = "My development agent"
```

### `[agent]` Section

#### `id`

Agent identity in `type:identifier` format. Examples:
- `agent:copilot-1`
- `human:alice`
- `ci:github-actions`

**Environment override:** `JIT_AGENT_ID`

#### `created_at`

ISO 8601 timestamp when this agent config was created.

#### `description`

Human-readable description of this agent.

`[agent].id` is the active persistent identity source (after an explicit CLI
identity and `JIT_AGENT_ID`). The optional `default_ttl_secs` field is parsed
metadata only: it does not select a claim TTL.

## Environment Variables

| Variable | Description | Valid Values |
|----------|-------------|--------------|
| `JIT_AGENT_ID` | Agent identity | `type:identifier` |

## Config Commands

### Show Effective Config

```bash
jit config show
```

Displays the merged configuration from all sources.

### Get Single Value

```bash
jit config get <dotted.key> [--json]
```

Resolves a dotted key against the WHOLE configuration surface — type
hierarchy, label namespaces, item kinds, documentation paths, validation
settings, project identity, schema version, and the system/user/repo-layered
worktree/coordination/lock/event settings below — via a generic path walk,
not a hand-maintained list of recognised keys. A dotted path mirrors
`config.toml`'s own structure, including a section's kebab-case keys (e.g.
`item_kinds.<name>.id-pattern`) and a user-declared map's own entries (e.g.
`namespaces.type.unique`). An intermediate key returns the whole subtree at
that point (`jit config get documentation` prints the entire section) rather
than erroring.

```bash
jit config get worktree.enforce_leases
jit config get coordination.max_indefinite_leases_per_agent
jit config get type_hierarchy.strategic_types
jit config get documentation.development_root
jit config get namespaces.type.unique
```

`worktree`, `coordination`, `global_operations`, `locks`, and `events` resolve
the merged, default-filled view that `jit config show` displays. Every other
section reads the repository's `config.toml` only, with no system/user merge or
built-in defaults; an absent section resolves to `{}`. This visibility does
not make the parsed compatibility fields above active runtime controls.
`templates` and `invariants` are not part of this surface (they load from
sibling files, not `config.toml`); see `jit config list-templates` / `jit item
list --kind invariant`.

An unknown key exits `2` (`INVALID_ARGUMENT`): an unknown top-level key
names the valid sections, an unknown nested key names the missing segment
and its resolved parent; see the [machine-readable failure
contract](cli-commands.md#cli-json-contracts).

```bash
jit config get bogus_section
# Error: unknown config key 'bogus_section'; valid top-level sections: ...
```

### Set Value

```bash
# Set in repository config
jit config set worktree.enforce_leases warn

# Set in user config
jit config set worktree.enforce_leases warn --global
```

### Validate Config

```bash
jit config validate
```

Exit codes are listed in [Exit Codes](exit-codes.md), the generated reference:
`0` when the configuration is valid and `1` when errors are found.

## Example Configurations

See [example-config.toml](example-config.toml) for a complete annotated template.

**Common patterns:**

| Use Case | Key Settings |
|----------|--------------|
| Single agent | Defaults work, no config needed |
| Multi-agent team | `enforce_leases = "strict"`; acquire a lease before structural changes |
| CI/CD | Set `JIT_AGENT_ID` when a command needs an agent identity |
| Solo dev | `enforce_leases = "warn"` for flexibility |

## See Also

- [Example config.toml](example-config.toml) - Full annotated configuration
- [Tutorial: Parallel Work](../tutorials/parallel-work-worktrees.md)
- [How-to: Multi-Agent Coordination](../how-to/multi-agent-coordination.md)
