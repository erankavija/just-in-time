# CLI Command-Grammar Standard

> **Diátaxis Type:** Reference

This document is the canonical grammar every `jit` command follows. It states the
shape of the command line — how nouns and verbs combine, when an argument is
positional versus a flag, which identifier forms a command accepts, and how the
gate surface is partitioned into configuration, execution, and inspection.

It is **prescriptive**. Each rule states the canonical form first, then records
how today's commands conform or where they must be renamed to conform. It is the
named standard the CLI-conformance work cites: the conformance issues (id
normalization, gate-surface grouping, the short-flag sweep, and related grammar
fixes) treat the rules below as the contract and bring divergent commands into
line. No command is renamed here — this document only defines the target.

When an existing command conflicts with a rule below, this document is canonical
and the command is the thing to change.

---

## Noun/verb structure

The command line is `jit <noun> <verb> [arguments] [flags]`.

- A **noun** is a subcommand group naming a domain entity or subsystem: `issue`,
  `gate`, `gate preset`, `dep`, `doc`, `graph`, `query`, `claim`, `config`,
  `label`, `events`, `registry`, `snapshot`, `worktree`, `hooks`, `item`,
  `invariant`, and `apply`'s target. Nouns are singular (`issue`, not `issues`).
- A **verb** is the action on that noun: `create`, `show`, `list`, `update`,
  `add`, `remove`, `pass`, `define`, `acquire`. Verbs are imperative and shared
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

---

## Positional-versus-flag conventions

An argument is **positional** when it is the subject the verb acts on — the thing
without which the verb is meaningless. It is a **flag** when it modifies how the
verb acts.

Canonical rules:

- **The subject is positional.** The issue id, gate key, document path, assignee,
  dependency endpoints, template name, preset name, and qualified item id are
  positional because the verb cannot run without them. Examples:
  `jit issue show <id>`, `jit gate pass <id> <gate-key>`,
  `jit dep add <from> <to>...`, `jit doc add <id> <path>`.
- **Modifiers are flags.** Anything that tunes, filters, scopes, or formats is a
  flag: `--priority`, `--state`, `--label`, `--force`, `--depth`, `--json`,
  `--quiet`.
- **`--json` is always a flag, on every command** (reads and writes alike). It is
  never positional and never implied.
- **A list subject is a positional, space-separated list.** Where a verb acts on
  several subjects of one kind, they are trailing space-separated positionals, not
  a repeated flag: `jit gate add <id> <gate-key>...`,
  `jit dep add <from> <to>...`, `jit issue show <id>...`,
  `jit gate preset apply <name> <id>...`.
- **A list modifier is a repeatable/comma-joined flag.** Where a list tunes the
  verb rather than naming its subjects, it is a flag that accepts both repetition
  and comma-separation: `--label a:b --label c:d` or `--label a:b,c:d`. This
  governs `--label`, `--gate`, `--add-gate`, `--remove-label`, `--remove-gate`,
  `--subtask`, and `--except`. (`--description` on `issue breakdown` is the one
  list flag that is repeatable-only and never comma-split, because prose
  descriptions legitimately contain commas.)

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
| **Issue reference** | full UUID, 8-char `short_id`, or any unique prefix | The canonical form. Every command taking an issue subject resolves all three; none may demand the full UUID. |
| **Gate key** | exact registry key | No prefix or fuzzy match — a gate key is an exact string from the registry. |
| **Lease id** | full lease UUID | `claim renew`, `claim heartbeat`, and `claim force-evict` take the lease's own UUID, distinct from the issue id. |
| **Qualified item id** | `<scope>/<self-id>` | `scope` is `@` for project scope or an issue reference (and so accepts the same three issue forms); `self-id` is exact. |

**Consistency rule.** Any positional that names an issue — `issue show`,
`issue update`, `gate add`, `gate pass`, `dep add`, `doc add`, `claim acquire`,
`claim release`, and the rest — accepts the full UUID, the 8-char short id, and a
unique prefix, identically. A command that resolves only the full UUID, or whose
help omits the accepted forms while a sibling documents them, is nonconforming and
must be aligned. The conformance work uses this rule to make id acceptance uniform
across the gate, claim, and doc surfaces, matching what `jit issue show` already
documents ("full id, short id, or unique prefix").

`claim release` already states "short ids accepted" in its help; the standard
makes that the documented contract for every issue-subject command rather than a
per-command note.

---

## Short-flag rule (the `-t` rule)

A short flag must mean the same thing on every command that exposes it. Where two
options on sibling commands would both want one short letter, the **more primary
option keeps the short flag and the other becomes long-only** — the short letter
is never overloaded to mean different options on different commands.

### `-t` is reserved for `--title`

`-t` binds to `--title` and nothing else. It already means `--title` on
`issue create`, `issue update`, `gate define`, and `registry add`. `doc add`
formerly bound `-t` to `--doc-type`; it has been brought into line (see below) and
now exposes `--title` as the alias of its human label `--label`.

**Canonical resolution.** `doc add` drops the `-t` short flag; its type selector
stays as `--doc-type`, long-only. The model already exists in the tree:
`doc archive` exposes its type selector as `--type` with no short flag. Every
type-selecting flag follows that model — `--doc-type` / `--type` are long-only —
so `-t` is unambiguously `--title` across the whole CLI.

Where `issue create` grows an explicit type selector, it is spelled `--type`,
long-only, never `-t`. Keeping the selector long-only is what preserves
`-t` = `--title` on the command the background calls out (cli.rs:437 today binds
`issue create`'s `-t` to `--title`; that binding must remain unambiguous).

### Other short flags governed by the same rule

The same "primary keeps the short, the other goes long-only" principle resolves
the secondary collisions the conformance sweep encounters:

| Short | Canonical meaning | Long-only exception | Conformance note |
|-------|-------------------|---------------------|------------------|
| `-t` | `--title` | `--doc-type`, `--type` | `doc add` drops `-t`. |
| `-d` | `--description` | — | Already consistent across `issue create`, `issue update`, `gate define`, `registry add`. |
| `-l` | `--label` | — | Already consistent. |
| `-p` | `--priority` | — | Already consistent across `issue create`, `issue update`, `issue list`, and `query *`. |
| `-s` | `--state` | `--stage` | `gate define` and `registry add` bind `-s` to `--stage`; the standard makes `--stage` long-only so `-s` is `--state` everywhere. |

Short flags `-d`, `-l`, and `-p` are already uniform and the standard records them
as fixed. `-t` and `-s` are the two letters the conformance work must
disambiguate.

---

## Gate command grouping: configuration vs execution vs inspection

The gate surface is one flat `gate` enum today that interleaves three distinct
responsibilities. The standard separates them, because the downstream gate rename
sweep regroups commands along exactly this boundary. Every gate verb belongs to
exactly one of three groups:

**1. Configuration** — shapes *what gates exist and which issues require them*.
Mutating the registry, attaching gates to issues, and managing presets.

- Registry definition: `gate define`, `gate remove`.
- Issue attachment: `gate add` (attach registered gates to an issue), and the
  attachment aliases `issue update --add-gate` / `--remove-gate`.
- Presets (reusable attachment bundles): `gate preset list`, `gate preset show`,
  `gate preset apply`, `gate preset create`.

**2. Execution** — *produces a verdict and may advance issue state*. These are the
only gate verbs that mutate gate-run state.

- `gate pass`, `gate pass-all`, `gate fail`.

**3. Inspection** — *reports definitions or run results with no side effects*.
Strictly read-only.

- Registry reads: `gate list`, `gate show`.
- Run-result reads: `gate check`, `gate check-all`.

### The load-bearing invariant

**Inspection never mutates; execution never merely reports.** `gate check` and
`gate check-all` show the last recorded run and must stay non-mutating (their help
already says "inspection only, non-mutating"). `gate pass` / `gate fail` /
`gate pass-all` run checkers, record verdicts, and can transition the issue. The
two must never be conflated: an inspection verb that quietly re-runs a checker, or
an execution verb dressed as a "check", is nonconforming. This configuration ÷
execution ÷ inspection partition is the contract the gate rename sweep applies to
regroup the flat enum.

### Conformance notes for the gate surface

- **`gate add` is attachment, not definition.** It attaches an already-registered
  gate to an issue; it does not create a registry entry. Its name must not be
  confused with `gate define` (registry creation). The standard keeps attachment
  (configuration) and definition (configuration) as distinct verbs within the
  configuration group.
- **`jit registry` duplicates gate-registry configuration.** `registry list`,
  `registry add`, `registry remove`, and `registry show` operate on the same gate
  registry as `gate define` / `gate remove` / `gate list` / `gate show`. `jit
  gate` is the canonical home for gate-registry configuration; the duplicate
  `registry` noun is reconciled to a single surface by the conformance work rather
  than left as two spellings of one operation.

---

## Conformance summary

| Rule | Conforming today | Must change |
|------|------------------|-------------|
| Noun/verb shape | All noun groups and bare verbs | — |
| Positional subject vs flag modifier | Issue/gate/path/endpoint positionals; `--json`/filters as flags | — |
| Positional list vs flag list | `gate add`, `dep add`, `issue show` (positional); `--label`/`--gate` (flag) | — |
| Issue-id acceptance | `issue show` (documents all three forms) | Gate, claim, and doc issue-subject commands: accept and document full id / short id / prefix uniformly |
| `-t` = `--title` | `issue create`, `issue update`, `gate define`, `registry add`, `doc add` (`--title` aliases `--label`; `--doc-type` long-only) | — |
| `-s` = `--state` | `issue list`, `issue search`, `query *` | `gate define`, `registry add`: make `--stage` long-only |
| Gate config ÷ execution ÷ inspection | Group boundaries are well-defined by behavior | Flat `gate` enum regrouped; `jit registry` reconciled into the `gate` configuration surface |

---

## See also

- [CLI Commands](cli-commands.md) — full per-command reference and JSON contracts.
- [Glossary](glossary.md) — term definitions.
- [Core Model](../concepts/core-model.md) — issues, gates, dependencies, states.
