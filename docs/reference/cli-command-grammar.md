# CLI Command-Grammar Standard

> **Diátaxis Type:** Reference

This document is the canonical grammar every `jit` command follows. It states the
shape of the command line: how nouns and verbs combine, when an argument is
positional versus a flag, which identifier forms a command accepts, and how the
gate surface is partitioned into configuration, execution, and inspection.

It is **prescriptive**. Each rule states the canonical form, and the commands in
the tree follow it: identifier acceptance, gate-surface grouping, and short-flag
assignment all obey the rules below.

When a command and a rule below disagree, this document is canonical and the
command is the defect.

---

## Noun/verb structure

The command line is `jit <noun> <verb> [arguments] [flags]`. The read-only
`archive` noun uses a target selector in the verb slot, as specified below.

- A **noun** is a subcommand group naming a domain entity or subsystem: `issue`,
  `gate`, `gate preset`, `dep`, `doc`, `graph`, `query`, `claim`, `config`,
  `label`, `events`, `archive`, `snapshot`, `worktree`, `hooks`, `item`,
  `invariant`, `reference`, `migrate`, `profile`, and `apply`'s target. Nouns
  are singular (`issue`, not `issues`).
- A **verb** is the action on that noun: `create`, `show`, `list`, `update`,
  `add`, `remove`, `evaluate`, `define`, `acquire`. Verbs are imperative and shared
  across nouns where the action is the same (`list`, `show`, `add`, `remove`
  recur with consistent meaning).
- A small set of **bare verbs** act on the whole repository and take no noun:
  `init`, `status`, `validate`, `search`, `version`, `recover`, `serve`, and
  `apply`. The rule that selects a bare verb over a noun group: an operation that
  reads or mutates a single domain entity belongs under that entity's noun; an
  operation over the repository as a whole is a bare verb.

**Alias rule.** Aliases exist only as ergonomic spellings of a canonical noun and
behave identically. The canonical noun is the short form; the long form is the
alias: `dep` is canonical with `dependency` as its visible alias, and `doc` is
canonical with `document` as its alias. New nouns do not introduce aliases unless
an established long form is already in agents' muscle memory.

**Top-level convenience aliases.** A small set of top-level spellings flatten a
common noun/verb into a single first-guess command and route to the canonical
form: `jit list` routes to `jit issue list`, and `jit rdeps` to
`jit graph rdeps`. Each behaves identically to the canonical noun/verb it
forwards to, which remains the primary spelling.

**Archive target selectors.** `archive` is a noun group whose second token
selects the kind of target. Its canonical forms are:

- `jit archive document <path>`, where `<path>` is a repository-relative
  document path.
- `jit archive container <id>`, where `<id>` is an issue reference resolved by
  the normal full-id, `short_id`, or unique-prefix rules below.
- `jit archive candidates`, which has no subject because it evaluates the
  complete set of effectively terminal configured non-leaf containers (Done,
  Rejected, or Archived from one of those).

The document and container forms construct and display a read-only archive plan
by default.
`--execute` is the explicit mutation modifier; it recomputes the plan under the
repository write guard rather than consuming preview output. `--json` changes
only the rendering of the selected preview or execution result.
`archive candidates` is always read-only and therefore accepts `--json` but not
`--execute`, a category, or filtering modifiers.

---

## Positional-versus-flag conventions

An argument is **positional** when it is the subject the verb acts on: the thing
without which the verb is meaningless. It is a **flag** when it modifies how the
verb acts.

Canonical rules:

- **The subject is positional.** The issue id, gate key, document path, assignee,
  dependency endpoints, template name, preset name, profile id, and qualified
  item id are positional because the verb cannot run without them. Examples:
  `jit issue show <id>`, `jit gate evaluate <id> <gate-key>`,
  `jit dep add <from> <to>...`, `jit doc add <id> <path>`,
  `jit archive document <path>`, `jit archive container <id>`,
  `jit profile show <profile-id>`.
- **Modifiers are flags.** Anything that tunes, filters, scopes, or formats is a
  flag: `--priority`, `--state`, `--label`, `--force`, `--depth`, `--json`,
  `--quiet`.
- **`--json` is always a flag, on every command** (reads and writes alike). It is
  never positional and never implied.
- **A list subject is a positional, space-separated list.** Where a verb acts on
  several subjects of one kind, they are trailing space-separated positionals, not
  a repeated flag: `jit gate add <id> <gate-key>...`,
  `jit dep add <from> <to>...`, `jit issue show <id>...`,
  `jit gate preset apply <name> [<id>...]` (its id list is optional; zero ids
  applies the preset to nothing).
- **A list modifier is a repeatable/comma-joined flag.** Where a list tunes the
  verb rather than naming its subjects, it is a flag that accepts both repetition
  and comma-separation: `--label a:b --label c:d` or `--label a:b,c:d`. This
  governs `--label`, `--gate`, `--add-gate`, `--remove-label`, `--remove-gate`,
  and `--except`.

The positional-list-versus-flag-list split is the canonical reason
`jit gate add abc tests clippy` (subjects) reads differently from
`jit issue create --gate tests,clippy` (modifier): the first names the gates the
verb attaches; the second tunes a create.

---

## Identifier-acceptance semantics

Sibling commands must accept the same identifier forms for the same kind of
subject. The forms are fixed per identifier kind:

| Identifier kind | Accepted forms | Notes |
|-----------------|----------------|-------|
| **Issue reference** | full UUID, 8-char `short_id`, or any unique prefix of at least 4 characters | The canonical form. Every command taking an issue subject resolves all three; none may demand the full UUID. A prefix shorter than 4 characters is rejected. |
| **Gate key** | exact registry key | No prefix or fuzzy match. A gate key is an exact string from the registry. |
| **Lease id** | full lease UUID | `claim renew`, `claim heartbeat`, and `claim force-evict` take the lease's own UUID, distinct from the issue id. |
| **Qualified item id** | `@/<kind>/<self-id>` (project) or `@/issue/<issue-ref>/<kind>/<self-id>` (issue) | Every explicit address includes its kind segment. `<issue-ref>/<self-id>` is accepted as input sugar only when the configured item kinds can infer the kind unambiguously. |

**Consistency rule.** Every positional that names an issue (`issue show`,
`issue update`, `gate add`, `gate evaluate`, `dep add`, `doc add`, `claim acquire`,
`claim release`, `archive container`, and the rest) accepts the full UUID, the
short id, and a unique prefix, identically — the three forms and their resolution
are specified in
[Storage Record Layout → Issue Identifiers](storage-records.md#issue-identifiers).
Id acceptance is uniform across the issue, gate,
claim, doc, and archive surfaces; `jit issue show` documents the three forms as
"full id, short id, or unique prefix", and every issue-subject command resolves
them the same way. A command that resolved only the full UUID, or whose help
omitted the accepted forms while a sibling documented them, would violate this
rule.

`claim release` states "short ids accepted" in its help, and that contract holds
for every issue-subject command.

---

## Short-flag rule (the `-t` rule)

A short flag must mean the same thing on every command that exposes it. Where two
options on sibling commands would both want one short letter, the **more primary
option keeps the short flag and the other becomes long-only**. The short letter
is never overloaded to mean different options on different commands.

### `-t` is reserved for `--title`

`-t` binds to `--title` and nothing else. It means `--title` on `issue create`,
`issue update`, and `gate define`. `doc add` exposes `--title` as a visible alias
of its human label `--label` (`-l`), and its type selector is `--doc-type`,
long-only, so `-t` stays free of it.

**Type selectors are long-only.** Every type-selecting flag is spelled without a
short flag: `issue create` and `issue update` take `--type`, while `doc add`
takes `--doc-type`. Keeping type selectors long-only is what makes `-t`
unambiguously `--title` across the whole CLI.

### Other short flags governed by the same rule

The same "primary keeps the short, the other goes long-only" principle fixes each
short letter to one meaning:

| Short | Canonical meaning | Long-only exception | Note |
|-------|-------------------|---------------------|------|
| `-t` | `--title` | `--doc-type`, `--type` | `doc add` has no `-t`; its type selector is `--doc-type`. |
| `-d` | `--description` |  | Uniform across `issue create`, `issue update`, and `gate define`. |
| `-l` | `--label` |  | Uniform across the commands that take labels. |
| `-p` | `--priority` |  | Uniform across `issue create`, `issue update`, `issue list`, and `query *`. |
| `-s` | `--state` | `--stage` | `gate define`'s `--stage` is long-only, so `-s` is `--state` everywhere. |

Each short flag above binds one meaning across the whole CLI.

---

## Gate command grouping: configuration vs execution vs inspection

The `gate` verbs divide by responsibility along one boundary. Every gate verb
belongs to exactly one of three groups:

**1. Configuration** shapes *what gates exist and which issues require them*.
Mutating the registry, attaching gates to issues, and managing presets.

- Registry definition: `gate define`, `gate remove`.
- Issue attachment: `gate add` (attach registered gates to an issue), and the
  attachment aliases `issue update --add-gate` / `--remove-gate`.
- Presets (reusable attachment bundles): `gate preset list`, `gate preset show`,
  `gate preset apply`, `gate preset create`.

**2. Execution** *produces a verdict and may advance issue state*. These are the
only gate verbs that mutate gate-run state.

- `gate evaluate` (alias `eval`), `gate evaluate-all`, `gate fail`.

**3. Inspection** *reports definitions or run results with no side effects*.
Strictly read-only.

- Registry reads: `gate list`, `gate show`.
- Run-result reads: `gate status`, `gate status-all`. `gate status-all` is
  read-only but exits nonzero (4) unless every required gate has passed: a
  readiness signal, not a mutation.

### The load-bearing invariant

**Inspection never mutates; execution never merely reports.** `gate status` and
`gate status-all` show recorded state and must stay non-mutating (their help
already says "inspection only, non-mutating"). `gate evaluate` / `gate fail` /
`gate evaluate-all` run checkers, record verdicts, and can transition the issue. The
two must never be conflated: an inspection verb that quietly re-runs a checker, or
an execution verb dressed as a "check", is nonconforming. This configuration ÷
execution ÷ inspection partition is the contract the gate surface holds to.

### Notes for the gate surface

- **`gate add` is attachment, not definition.** It attaches an already-registered
  gate to an issue; it does not create a registry entry. Its name is distinct from
  `gate define` (registry creation). Attachment (configuration) and definition
  (configuration) are distinct verbs within the configuration group.
- **`jit gate` is the single gate-registry surface.** Gate-registry
  configuration lives solely under `jit gate` (`gate define` / `gate remove` /
  `gate list` / `gate show`), so one operation has exactly one spelling.

---

## Grammar summary

| Rule | Where it holds |
|------|----------------|
| Noun/verb shape | All noun groups and bare verbs |
| Positional subject vs flag modifier | Issue/gate/path/endpoint positionals; `--json`/filters as flags |
| Positional list vs flag list | `gate add`, `dep add`, `issue show` (positional); `--label`/`--gate` (flag) |
| Issue-id acceptance | `issue show`, `archive container`, and every gate, claim, and doc issue-subject command accept and document full id / short id / prefix uniformly |
| `-t` = `--title` | `issue create`, `issue update`, `gate define`; `doc add` aliases `--title` to `--label`, with `--doc-type` long-only |
| `-s` = `--state` | `issue list`, `issue search`, `query *`; `gate define`'s `--stage` is long-only |
| Gate config ÷ execution ÷ inspection | Group boundaries follow command behavior |

---

## See also

- [CLI Commands](cli-commands.md): full per-command reference and JSON contracts.
- [Glossary](glossary.md): term definitions.
- [Core Model](../concepts/core-model.md): issues, gates, dependencies, states.
