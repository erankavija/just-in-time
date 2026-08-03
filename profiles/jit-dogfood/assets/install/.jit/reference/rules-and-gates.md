## Rules

- **@/rule/jit-content-standards** — Warn when an issue lacks marked, stable success criteria. (warn, advisory)
- **@/rule/hard-criteria-covered** — Before an epic completes, every hard requirement is credited to a completed implementation descendant. (error, enforced)
- **@/rule/coverage-preview** — While breakdown is reviewed, every hard requirement is credited to a drafted implementation descendant. (error, enforced)
- **@/rule/orphan-leaf** — Warn when a leaf-level-typed issue carries no parent-membership label, leaving it unattached to any strategic container. Advisory: never blocks a write. (warn, advisory)
- **@/rule/strategic-consistency** — Warn when a strategic-typed issue lacks its own identifying membership label. Advisory: never blocks a write. (warn, advisory)

## Gates

- **@/gate/breakdown-review** — Breakdown Review: External-review placeholder for decomposition quality, issue content, and dependency ordering before implementation.
- **@/gate/code-review** — Code Review: Review issue-attributable implementation changes against repository policy and success criteria.
- **@/gate/coverage-preview** — Coverage Preview: Validate the container named by the breakdown issue's brackets label.
- **@/gate/jit-validate** — Issue Validation: Run declarative validation for the gated issue.
- **@/gate/plan-review** — Plan Review: External-review placeholder for the linked plan before implementation work fans out.
- **@/gate/repo-validate** — Repository Validation: Run structural and declarative validation for the whole repository.
