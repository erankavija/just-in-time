<!-- Generated from `crate::gate_presets::reference` — do not edit by hand. -->

# Built-in Gate Presets

> **Diátaxis Type:** Reference

The gate presets the `jit` binary ships. A preset is a named bundle of gate
definitions; `jit gate preset apply <preset> <id>...` inserts each bundled gate
into the project's gate registry (`.jit/gates.toml`) under its key — for keys the
registry does not already carry, and, with `--timeout <seconds>`, overwriting the
key with the overridden checker timeout — and then adds those keys to each issue's
required gates. `--no-precheck`, `--no-postcheck`, and `--except <key>` narrow which
of the preset's gates are applied. Each gate materializes into the registry with
`version = 1`, `priority = 100`, and `auto` set from its mode.

This page is generated from the binary's package-derived preset projection. The
embedded `jit-dogfood` profile's plan template selects the built-in names, and its
matching gate contributions provide their definitions. The page lists what the
binary carries — not what any repository has configured. `jit init` writes an empty
gate registry, so nothing below reaches a project until `jit gate preset apply`
runs. The gates a project actually enforces live in its own `.jit/gates.toml`, its
settings in `.jit/config.toml`; render those with `jit reference render` (see
[Rules and Gates](rules-and-gates.md)).

## Portable checker types

Automated gate definitions can use `exec` or one of four in-process checker types.
The in-process checkers do not invoke a shell, a second `jit` binary, or `jq`, and the
configured gate key does not change their behavior:

- `repository_validation` runs structural and declarative validation for the whole
repository.
- `issue_validation` runs declarative validation for the gated issue.
- `label_target_validation` reads exactly one `<label_namespace>:<target-id>` label
from the gated issue and runs scoped validation for that target. Its checker table
must set `label_namespace`.
- `review_placeholder` passes so a workflow can be installed before an external
reviewer is selected, but records an advisory structured finding and prints
`WARNING: EXTERNAL REVIEW PLACEHOLDER`. Whole-repository validation also warns
while any gate uses it. Replace it with an `exec` checker (for example, `jit gate
update <key> --checker-command <command>`) before treating the gate as review
evidence.

Native checker types are selected in the gate registry; `jit gate define` does not
have a checker-type option. These four independent definitions show the canonical
`.jit/gates.toml` syntax. The keys are examples and can be replaced with any
configured gate keys:

```toml
[[gates]]
version = 1
key = "repository-policy"
title = "Repository validation"
description = "Validate the whole repository"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "repository_validation"

[[gates]]
version = 1
key = "work-item-policy"
title = "Issue validation"
description = "Validate the gated issue"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "issue_validation"

[[gates]]
version = 1
key = "container-coverage"
title = "Container coverage"
description = "Validate the container named by the covers label"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "label_target_validation"
label_namespace = "covers"

[[gates]]
version = 1
key = "external-review"
title = "External review"
description = "Passing placeholder until a reviewer is configured"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "review_placeholder"
```

A project can also define its own presets: `jit gate preset create <issue> <name>`
captures an issue's gates into `.jit/config/gate-presets/<name>.json`, and every
JSON file in that directory loads alongside the built-ins. `jit gate preset create`
rejects a built-in name; a hand-authored file that reuses one shadows the built-in
for `jit gate preset show` and `apply`, and `jit gate preset list` then reports that
name as project-local instead of `[builtin]`.

| Preset | Description | Gates |
| --- | --- | --- |
| [`breakdown-review`](#breakdown-review) | Review decomposition quality, issue content, and dependency ordering before implementation. | 1 |
| [`coverage-preview`](#coverage-preview) | Validate the container named by the breakdown issue's brackets label. | 1 |
| [`plan-review`](#plan-review) | Review the linked plan before implementation work fans out. | 1 |

## `breakdown-review`

Review decomposition quality, issue content, and dependency ordering before implementation.

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `breakdown-review` | Breakdown Review | postcheck | auto | Review decomposition quality, issue content, and dependency ordering before implementation. | `review_placeholder` — WARNING: EXTERNAL REVIEW PLACEHOLDER PASSED WITHOUT RUNNING A REVIEWER. Replace this checker with a real external review integration before relying on this gate. |

## `coverage-preview`

Validate the container named by the breakdown issue's brackets label.

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `coverage-preview` | Coverage Preview | postcheck | auto | Validate the container named by the breakdown issue's brackets label. | `label_target_validation` — built-in scoped validation; target label namespace: `brackets` |

## `plan-review`

Review the linked plan before implementation work fans out.

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `plan-review` | Plan Review | postcheck | auto | Review the linked plan before implementation work fans out. | `review_placeholder` — WARNING: EXTERNAL REVIEW PLACEHOLDER PASSED WITHOUT RUNNING A REVIEWER. Replace this checker with a real external review integration before relying on this gate. |
