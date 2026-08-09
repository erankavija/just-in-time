<!-- Generated from `crate::gate_presets::reference` — do not edit by hand. -->

# Gate Presets

> **Diátaxis Type:** Reference

A gate preset is a named bundle of gate definitions a project declares for
itself. `jit gate preset create <issue> <name>` captures an issue's gates into
`.jit/config/gate-presets/<name>.json`, and every JSON file in that directory
loads as a preset whose name must equal its filename stem. `jit gate preset list`
reports them, `jit gate preset show <preset>` prints one, and
`jit gate preset apply <preset> <id>...` inserts each bundled gate into the
project's gate registry (`.jit/gates.toml`) under its key — for keys the registry
does not already carry, and, with `--timeout <seconds>`, overwriting the key with
the overridden checker timeout — and then adds those keys to each issue's required
gates. `--no-precheck`, `--no-postcheck`, and `--except <key>` narrow which of the
preset's gates are applied. Each gate materializes into the registry with
`version = 1`, `priority = 100`, and `auto` set from its mode.

The gates a project enforces live in its own `.jit/gates.toml`, its settings in
`.jit/config.toml`; render those with `jit project render` (see
[Rules and Gates](rules-and-gates.md)). A profile package can contribute gate
definitions when it is applied; see [Repository Profiles](profiles.md).

## Portable checker types

Automated gate definitions can use `exec` or one of five in-process checker types.
The in-process checkers do not invoke a shell, a second `jit` binary, or `jq`, and the
configured gate key does not change their behavior:

- `repository_validation` runs structural and declarative validation for the whole
repository.
- `issue_validation` runs declarative validation for the gated issue.
- `label_target_validation` reads exactly one `<label_namespace>:<target-id>` label
from the gated issue and runs scoped validation for that target. Its checker table
must set `label_namespace`.
- `rule_validation` names one configured graph rule with `rule` and evaluates it
with the gated issue as its sole firing subject. The complete repository may be
used for graph and identifier-resolution context, but unrelated issues cannot
contribute findings. A missing, disabled, or non-matching rule is an error.
- `review_placeholder` passes so a workflow can be installed before an external
reviewer is selected, but records an advisory structured finding and prints
`WARNING: EXTERNAL REVIEW PLACEHOLDER`. Whole-repository validation also warns
while any gate uses it. Replace it with an `exec` checker (for example, `jit gate
update <key> --checker-command <command>`) before treating the gate as review
evidence.

Native checker types are selected in the gate registry; `jit gate define` does not
have a checker-type option. These five independent definitions show the canonical
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
key = "selected-rule"
title = "Selected rule validation"
description = "Evaluate one configured graph rule for the gated issue"
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "rule_validation"
rule = "coverage-preview"

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
