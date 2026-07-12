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

This page is generated from the preset definitions in
`crates/jit/src/gate_presets/` and lists what the binary carries — not what any
repository has configured. `jit init` writes an empty gate registry, so nothing
below reaches a project until `jit gate preset apply` runs. The gates a project
actually enforces live in its own `.jit/gates.toml`, its settings in
`.jit/config.toml`; render those with `jit reference render` (see
[Rules and Gates](rules-and-gates.md)).

A project can also define its own presets: `jit gate preset create <issue> <name>`
captures an issue's gates into `.jit/config/gate-presets/<name>.json`, and every
JSON file in that directory loads alongside the built-ins. `jit gate preset create`
rejects a built-in name; a hand-authored file that reuses one shadows the built-in
for `jit gate preset show` and `apply`, and `jit gate preset list` then reports that
name as project-local instead of `[builtin]`.

| Preset | Description | Gates |
| --- | --- | --- |
| [`breakdown-review`](#breakdown-review) | Agent quality review of the decomposition on the breakdown node before fan-out | 1 |
| [`coverage-preview`](#coverage-preview) | Deterministic coverage preview on the breakdown node (scoped validate) | 1 |
| [`js-tdd`](#js-tdd) | Test-driven development workflow for JavaScript/TypeScript | 4 |
| [`minimal`](#minimal) | Minimal workflow with just code review | 1 |
| [`plan-review`](#plan-review) | Agent plan-quality review on the planning node before fan-out | 1 |
| [`python-tdd`](#python-tdd) | Test-driven development workflow for Python projects | 5 |
| [`rust-tdd`](#rust-tdd) | Test-driven development workflow for Rust projects | 5 |
| [`security-audit`](#security-audit) | Security review workflow | 3 |

## `breakdown-review`

Agent quality review of the decomposition on the breakdown node before fan-out

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `breakdown-review` | AI Breakdown Review | postcheck | auto | AI-powered adversarial review of a breakdown against the design doc and content standards: per-child content standards, dependency-DAG coherence, and right-sized decomposition (coverage of [hard] criteria is the separate coverage-preview gate) | `exec` — command `./scripts/ai-review.sh`; timeout 1800s; working dir: unset; env: `REVIEWER_AGENT=codex exec`; context passed: yes; prompt: unset; prompt file: `./scripts/breakdown-review-prompt.md` |

## `coverage-preview`

Deterministic coverage preview on the breakdown node (scoped validate)

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `coverage-preview` | Coverage Preview | postcheck | auto | Run scoped validation for the container resolved from the breakdown node's brackets: label; blocks when a [hard] criterion is uncovered at plan time | `exec` — command `./scripts/coverage-preview.sh`; timeout 300s; working dir: unset; env: empty; context passed: yes; prompt: unset; prompt file: unset |

## `js-tdd`

Test-driven development workflow for JavaScript/TypeScript

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `tdd-reminder` | Write tests first (TDD) | precheck | manual | Reminder to write failing tests before implementation | none — manual attestation |
| `jest` | All tests pass | postcheck | auto | npm test must pass | `exec` — command `npm test`; timeout 300s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `eslint` | ESLint passes | postcheck | auto | ESLint must pass with no errors | `exec` — command `npm run lint`; timeout 120s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `code-review` | Code review completed | postcheck | manual | Another developer reviewed the code | none — manual attestation |

## `minimal`

Minimal workflow with just code review

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `code-review` | Code review completed | postcheck | manual | Code has been reviewed | none — manual attestation |

## `plan-review`

Agent plan-quality review on the planning node before fan-out

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `plan-review` | AI Plan Review | postcheck | auto | AI-powered plan/design review before fan-out, against the planning issue's success criteria and linked design document | `exec` — command `./scripts/ai-review.sh`; timeout 1800s; working dir: unset; env: `REVIEWER_AGENT=codex exec`; context passed: yes; prompt: unset; prompt file: `./scripts/plan-review-prompt.md` |

## `python-tdd`

Test-driven development workflow for Python projects

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `tdd-reminder` | Write tests first (TDD) | precheck | manual | Reminder to write failing tests before implementation | none — manual attestation |
| `pytest` | All tests pass | postcheck | auto | pytest must pass | `exec` — command `pytest`; timeout 300s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `black` | Code formatted (Black) | postcheck | auto | Code must be formatted with Black | `exec` — command `black --check .`; timeout 30s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `mypy` | Type checking passes | postcheck | auto | mypy type checking must pass | `exec` — command `mypy .`; timeout 120s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `code-review` | Code review completed | postcheck | manual | Another developer reviewed the code | none — manual attestation |

## `rust-tdd`

Test-driven development workflow for Rust projects

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `tdd-reminder` | Write tests first (TDD) | precheck | manual | Reminder to write failing tests before implementation | none — manual attestation |
| `tests` | All tests pass | postcheck | auto | cargo test must pass | `exec` — command `cargo test`; timeout 300s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `clippy` | Clippy lints pass | postcheck | auto | No clippy warnings allowed | `exec` — command `cargo clippy --all-targets -- -D warnings`; timeout 120s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `fmt` | Code formatted | postcheck | auto | Code must be formatted with cargo fmt | `exec` — command `cargo fmt --check`; timeout 30s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `code-review` | Code review completed | postcheck | manual | Another developer reviewed the code | none — manual attestation |

## `security-audit`

Security review workflow

| Gate key | Title | Stage | Mode | Description | Checker |
| --- | --- | --- | --- | --- | --- |
| `security-review` | Security review completed | precheck | manual | Review code for security vulnerabilities: injection, auth, crypto, secrets | none — manual attestation |
| `secret-detection` | No secrets in code | postcheck | auto | Detect hardcoded secrets and credentials | `exec` — command `gitleaks detect --no-git`; timeout 20s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
| `dependency-audit` | Dependency vulnerabilities checked | postcheck | auto | Audit dependencies for known vulnerabilities | `exec` — command `cargo audit`; timeout 60s; working dir: unset; env: empty; context passed: no; prompt: unset; prompt file: unset |
