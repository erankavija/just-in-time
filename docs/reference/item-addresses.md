# Item Addresses

> **Diátaxis Type:** Reference

**Canonical specification of the addressable-item address grammar.** Normative
implementation: `crates/jit/src/domain/item.rs`.

An **addressable item** is a structured entry that carries a *self-id*: a line in
a declared section of an issue description, or an entry in a declared
project-level source file. An **address** names one such item. Addresses appear
in issue text, in documentation, and in link labels (`satisfies:@/issue/…`);
`jit item show` resolves them, and `jit validate` reports addresses that resolve
to nothing (rule `dangling-item-link`).

Nothing about an address is stored. It is a projection over three inputs: the
item's scope, its kind, and its parsed self-id.

## Kinds are repository configuration

The kind segment of an address is a name declared in the `[item_kinds]` table of
`.jit/config.toml`. Each kind declares the section or source file it reads, the
regular expression its self-ids match (`id-pattern`), its scope (`issue` or
`project`), and optional `aliases`. The engine interprets no kind name.

To enumerate the kinds a repository declares, read that table or run:

```bash
jit item list                     # every item, every kind
jit item list --kind invariant    # one kind, by registry name or alias
```

## Grammar

```
address           := project-address | issue-address | sugar-address

project-address   := "@" [ project-name ] "/" kind "/" self-id
issue-address     := "@" "/" "issue" "/" issue-ref "/" kind "/" self-id
sugar-address     := issue-ref "/" self-id

kind              := registry name of a declared kind, or one of its aliases
self-id           := the item's authored id
issue-ref         := a full issue id, a short id, or a unique id prefix
project-name      := the name declared as `[project] name` in .jit/config.toml
```

The two `@`-prefixed forms are **explicit**: they name their kind. `jit item
list` mints only these two forms, and every minted id round-trips through `jit
item show`.

| Form | Example | Addresses |
|------|---------|-----------|
| Project | `@/invariant/dag-acyclic` | An item of a project-scoped kind |
| Project, aliased kind | `@/inv/dag-acyclic` | The same item, via the kind's alias |
| Project, named | `@acme/invariant/dag-acyclic` | The same item, when `acme` is this repository's declared project name |
| Issue | `@/issue/<short-id>/requirement/REQ-01` | An item of an issue-scoped kind, inside the issue named by `<short-id>` |
| Sugar | `<short-id>/REQ-01` | The same item, kind inferred |

The project rows resolve as written; `<short-id>` in the issue and sugar rows is
a placeholder for a real issue's short id, and `REQ-01` for one of its
requirement self-ids.

### The `@` sentinel and the reserved `issue` segment

`@` opens every explicit address, for both scopes. What distinguishes the two is
the reserved segment `issue` in second position, not the sentinel.

`issue` is a built-in segment, never a kind name. An address of the shape
`@/issue/<self-id>` is rejected with a message saying so, rather than being read
as a kind named `issue`.

### Scope and the project name

Bare `@` is the local project. `@<name>` is a named-project reference. It
resolves exactly when `<name>` equals the repository's declared `[project] name`,
in which case it means the same item as bare `@`. Any other name is an error, as
is any named-project reference in a repository that declares no project name.
Resolution is local: no federation or remote lookup is attempted.

The named-project prefix applies to the project form only. The issue form
requires the bare `@` sentinel.

### Segment rules

- Every segment is non-empty.
- A colon anywhere in an address is rejected. The colon is reserved for the label
  `namespace:value` separator.
- The project form has exactly two segments after the scope token; the issue form
  has exactly four, the first being `issue`. Any other segment count is a parse
  error naming the offending address.
- An address carries at least one `/`. A bare self-id (`REQ-01`) is therefore not
  an address, which is how link labels distinguish a qualified reference from an
  unqualified one.

### Uniqueness

A self-id is unique per `(scope, kind)`. Two items of different kinds may share a
scope and a self-id (a rule and a gate both named `coverage-preview`); because
the kind is a segment of the address, they still address distinctly
(`@/rule/coverage-preview` and `@/gate/coverage-preview`). A self-id repeated
within one scope under one kind is an error at indexing time.

## The sugar form

`<short-id>/<self-id>` is an input form that omits the kind segment. The address
splits on its first `/`; the left side is the issue reference, the right side the
self-id.

### Kind inference

The self-id is matched against the `id-pattern` of every **issue-scoped** kind.
The match is a search, so a pattern that occurs anywhere within the self-id
counts. Project-scoped kinds are excluded from candidacy: their self-ids are
free-form slugs with no distinguishing shape, so **project items have no sugar
form** and always require the explicit `@/<kind>/<self-id>` address.

`jit item show` then resolves as follows:

| Candidate kinds matching the self-id | Behavior |
|--------------------------------------|----------|
| Exactly one | The kind is fixed; resolution filters by that kind and the self-id. |
| Zero, or more than one | The kind is left open; resolution filters by self-id alone among the issue's indexed items. |

With the kind left open, an issue holding exactly one item with that self-id
still resolves. It is the *items*, not the patterns, that decide. Given a
repository whose `requirement` and `risk` patterns both match the string
`RISK-01`, and an issue holding one `risk` item with that self-id:

```bash
$ jit item show <short-id>/RISK-01 --json | jq -r .item.qualified_id
@/issue/<short-id>/risk/RISK-01
```

An ambiguity is reported only when the issue really holds several items sharing
the self-id under different kinds:

```bash
$ jit item show <short-id>/RISK-01
Error: self-id 'RISK-01' in issue <short-id> is ambiguous across kinds (requirement, risk);
use the explicit '@/issue/<short-id>/<kind>/<self-id>' address
```

An issue holding no item with that self-id is a not-found error naming the issue
and the self-id.

The domain function `expand_sugar_address` is stricter than the CLI path: it
requires exactly one candidate kind and returns `SugarKindNotFound` or
`SugarKindAmbiguous` otherwise. The CLI treats both of those as "kind open" and
defers to item matching.

## Aliases

A kind may declare aliases in `[item_kinds].<kind>.aliases`. An alias is accepted
wherever the kind segment is, and in `jit item list --kind`. Output renders the
registry name:

```bash
$ jit item show @/inv/dag-acyclic --json | jq -r .item.qualified_id
@/invariant/dag-acyclic
```

## Resolution errors

| Input | Result |
|-------|--------|
| `@/dag-acyclic` | Parse error: the kind segment is missing; no self-id-only fallback exists |
| `@/issue/REQ-01` | Parse error: `issue` is a reserved segment |
| `@other/inv/dag-acyclic` | Not resolvable: `other` is not this repository's project name |
| `@/issue/<id>/<kind>/<self-id>` naming a kind that does not own that self-id | Not found, naming the kind and self-id; never a fallback to another kind's item |
| An unresolvable issue reference, or an ambiguous id prefix | The standard issue-id resolution error |

## See also

- [Configuration](configuration.md) - `config.toml` sections and `jit config get`
- [CLI Commands](cli-commands.md) - `jit item list`, `jit item search`
- [Labels](labels.md) - link labels whose value half may be an address
- [Containment and Completion](../concepts/containment-and-completion.md) - the
  work-graph model these items annotate
