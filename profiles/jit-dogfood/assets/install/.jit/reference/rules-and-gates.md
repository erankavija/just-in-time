## Rules

- **@/rule/jit-content-standards** — Warn when an issue lacks marked, stable success criteria. (warn, advisory)
- **@/rule/hard-criteria-covered** — Before an epic completes, every hard requirement is credited to a completed implementation descendant. (error, enforced)
- **@/rule/coverage-preview** — While breakdown is reviewed, every hard requirement is credited to a drafted implementation descendant. (error, enforced)

## Gates

- **@/gate/breakdown-review** — Breakdown Review: Review decomposition quality, issue content, and dependency ordering before implementation.
- **@/gate/code-review** — Code Review: Review issue-attributable implementation changes against repository policy and success criteria.
- **@/gate/coverage-preview** — Coverage Preview: Validate the container named by the breakdown issue's brackets label.
- **@/gate/jit-validate** — Issue Validation: Run declarative validation for the gated issue.
- **@/gate/plan-review** — Plan Review: Review the linked plan before implementation work fans out.
- **@/gate/repo-validate** — Repository Validation: Run structural and declarative validation for the whole repository.
