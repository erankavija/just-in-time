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
`version = 1` and `priority = 100`.

The gates a project enforces live in its own `.jit/gates.toml`, its settings in
`.jit/config.toml`; render those with `jit project render` (see
[Rules and Gates](rules-and-gates.md)). A profile package can contribute gate
definitions when it is applied; see [Repository Profiles](profiles.md).

## Portable checker types

Automated gate definitions can use `exec` or one of five in-process checker types.
The in-process checkers do not invoke a shell, a second `jit` binary, or `jq`, and the
configured gate key does not change their behavior:

- `repository_validation` runs the whole-repository `jit validate` computation, so
it never passes where that fails. Finding the repository wrong records `failed`;
being unable to load or read what it needs records `error`. Its verdict is never
reused, and declared inputs do not enable reuse, because it depends on records
the input digest excludes.
- `issue_validation` reports the gated issue's per-issue view.
- `label_target_validation` reads exactly one `<label_namespace>:<target-id>` label
from the gated issue, where the target is a full issue id or unique prefix, and
reports that target's scope view. Its checker table must set `label_namespace`.
- `rule_validation` names one configured graph rule with `rule` and reports its
selected-rule view with the gated issue as the sole firing subject. A missing,
disabled, or non-matching rule is a fault.
- `review_placeholder` passes so a workflow can be installed before an external
reviewer is selected, but records an advisory structured finding and prints
`WARNING: EXTERNAL REVIEW PLACEHOLDER`. Whole-repository validation also warns
while any gate uses it. Replace it with an `exec` checker (for example, `jit gate
update <key> --checker-command <command>`) before treating the gate as review
evidence.

Each validation checker reports what `jit validate` reports for its selection
of the one whole-repository report: `repository_validation` the whole report,
the others their [view](worktree-validate.md#views). Rules therefore evaluate
over the whole repository for every checker, and the view decides which
findings, including configuration errors, the checker sees. A rule pass that
fails records an `error` run, never a clean selection.

An in-process checker that cannot judge the repository records an `error` run:
configuration or declarations that do not load, a broken repository structure, or
a missing or invalid checker setting (`label_namespace`, the target label, `rule`).
A finding about repository content records `failed`. `jit gate evaluate` reports an
`error` run with exit code 10 and JSON verdict `error`, and the next evaluation
retries it.

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

[gates.checker]
type = "review_placeholder"
```
