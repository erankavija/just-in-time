# Scope and Domain Coverage

> **Diátaxis Type:** Explanation

JIT is a domain-agnostic issue tracker. While examples often use software development terminology, JIT works equally well for research, knowledge work, operations, and creative projects.

## Domain Coverage

### Software Development

| JIT Concept | Maps To |
|-------------|---------|
| Issue | Feature, bug, technical debt |
| Gate | Tests, linting, code review |
| Dependency | Build order, feature prerequisites |
| Priority | Severity, business impact |

**Typical workflow:** Plan → Implement → Test → Review → Deploy

**Example setup:**

```bash
# Define quality gates
jit gate define tests --title "Unit Tests" --mode auto \
  --checker-command "cargo test" --stage postcheck

jit gate define clippy --title "Lint Check" --mode auto \
  --checker-command "cargo clippy -- -D warnings" --stage postcheck

jit gate define code-review --title "Code Review" --mode manual \
  --stage postcheck

# Create feature with dependencies
jit issue create --title "Add user authentication" \
  --label type:feature --label component:auth

jit issue create --title "Add password hashing" \
  --label type:task --label component:auth

jit dep add <auth-issue> <hashing-issue>  # Auth depends on hashing
jit gate add <auth-issue> tests clippy code-review
```

### Research Projects

| JIT Concept | Maps To |
|-------------|---------|
| Issue | Experiment, analysis, paper section |
| Gate | Peer review, data validation, IRB approval |
| Dependency | Prior studies, sequential experiments |
| Priority | Publication deadline, grant timeline |

**Typical workflow:** Hypothesis → Literature → Data → Analysis → Publication

**Example setup:**

```bash
# Research-specific gates
jit gate define lit-review --title "Literature Review" --mode manual \
  --description "Verify relevant prior work is cited"

jit gate define data-validation --title "Data Validation" --mode manual \
  --description "Confirm data quality and methodology"

jit gate define peer-review --title "Peer Review" --mode manual \
  --description "Internal review before submission"

# Create experiment pipeline
jit issue create --title "Literature review: ML in genomics" \
  --label type:task --label phase:research

jit issue create --title "Experiment: Model comparison" \
  --label type:task --label phase:experiment

jit issue create --title "Analysis: Statistical validation" \
  --label type:task --label phase:analysis

# Chain dependencies
jit dep add <experiment> <lit-review>
jit dep add <analysis> <experiment>
```

### Knowledge Work

| JIT Concept | Maps To |
|-------------|---------|
| Issue | Document, report, presentation |
| Gate | Fact-checking, editing, stakeholder approval |
| Dependency | Chapter order, prerequisite research |
| Priority | Deadline, audience importance |

**Typical workflow:** Outline → Draft → Review → Revise → Publish

**Example setup:**

```bash
# Knowledge work gates
jit gate define fact-check --title "Fact Check" --mode manual \
  --description "Verify all claims and citations"

jit gate define editing --title "Editorial Review" --mode manual \
  --description "Grammar, style, and clarity review"

jit gate define stakeholder --title "Stakeholder Approval" --mode manual \
  --description "Sign-off from relevant stakeholders"

# Create document structure
jit issue create --title "Chapter 1: Introduction" \
  --label type:task --label doc:annual-report

jit issue create --title "Chapter 2: Financial Summary" \
  --label type:task --label doc:annual-report

jit issue create --title "Chapter 3: Future Outlook" \
  --label type:task --label doc:annual-report

# Chapters depend on previous chapters
jit dep add <chapter-2> <chapter-1>
jit dep add <chapter-3> <chapter-2>
```

### Operations / DevOps

| JIT Concept | Maps To |
|-------------|---------|
| Issue | Runbook, migration, incident response |
| Gate | Change approval, rollback plan, testing |
| Dependency | Infrastructure prerequisites |
| Priority | Urgency, blast radius |

**Typical workflow:** Plan → Approve → Execute → Verify → Document

**Example setup:**

```bash
# Operations gates
jit gate define change-approval --title "Change Approval" --mode manual \
  --description "CAB approval for production changes"

jit gate define rollback-plan --title "Rollback Plan" --mode manual \
  --description "Documented rollback procedure"

jit gate define staging-test --title "Staging Test" --mode auto \
  --checker-command "./scripts/test-staging.sh"

# Create migration with prerequisites
jit issue create --title "Backup production database" \
  --label type:task --label change:db-migration --priority high

jit issue create --title "Run migration scripts" \
  --label type:task --label change:db-migration --priority high

jit issue create --title "Verify data integrity" \
  --label type:task --label change:db-migration --priority high

# Strict ordering for operations
jit dep add <run-migration> <backup-db>
jit dep add <verify-data> <run-migration>
jit gate add <run-migration> change-approval rollback-plan
```

## Domain Comparison

| JIT Concept | Software | Research | Knowledge Work | Operations |
|-------------|----------|----------|----------------|------------|
| Issue | Feature/Bug | Experiment | Document | Runbook |
| Gate | CI Check | Peer Review | Approval | Change Control |
| Dependency | Prerequisite | Prior Study | Chapter Order | Infra Prereq |
| Priority | Severity | Impact | Deadline | Urgency |
| Assignee | Developer | Researcher | Writer | Engineer |
| Epic | Feature Set | Paper | Report | Migration |
| Milestone | Release | Submission | Publication | Quarter |

## What JIT Is

**Dependency-driven coordination:**
- Express "B needs A done first" with automatic blocking
- Visualize work order with `jit graph show`
- Query what's ready with `jit query available`

**Quality gating:**
- Enforce process checkpoints (tests, reviews, approvals)
- Automated gates run scripts; manual gates require sign-off
- Block completion until gates pass

**Multi-agent orchestration:**
- Atomic claiming prevents conflicts
- Lease-based coordination with TTL
- Event log for observability

**Local-first, git-friendly:**
- All state in `.jit/` directory (JSON files)
- Version, diff, merge with git
- No external database or cloud service

## What JIT Is Not

**Not a project management tool:**
- No Gantt charts or timeline views
- No resource allocation or budgeting
- No time estimates (use labels if needed)

**Not a communication platform:**
- No built-in comments or chat
- No notifications (integrate via scripts)
- No @mentions

**Not a CI/CD system:**
- Gates can trigger CI, but JIT doesn't run builds
- No artifact storage or deployment
- Integrates with CI/CD, doesn't replace it

**Not a knowledge base:**
- Tracks work items, not documentation content
- Can link to docs, but not a wiki
- No full-text document search

**Not a distributed system:**
- Repository-local coordination only
- No multi-repository dependencies
- File-based, not client-server

## When to Use JIT

**Good fit:**
- Work has clear dependencies between tasks
- Quality gates are important to your process
- AI agents or automation involved
- You prefer CLI/JSON over web UI
- Local-first tracking without cloud dependencies

**Consider alternatives when:**
- Non-technical users need rich web UI
- You need cloud-hosted collaboration
- Project management features required (Gantt, budgets)
- Built-in communication essential

## Integration Points

JIT focuses on coordination. Integrate with:

| Need | Integration |
|------|-------------|
| Communication | Slack, email, chat tools |
| Time tracking | External tools + labels |
| Document authoring | Editors, IDEs |
| Build/deployment | CI/CD systems (GitHub Actions, etc.) |
| Visualization | Web UI (`jit-server`) |

## See Also

- [Core Model](core-model.md) - Issues, dependencies, gates, states
- [Design Philosophy](design-philosophy.md) - Why JIT works this way
- [How-To: Software Development](../how-to/software-development.md)
- [How-To: Research Projects](../how-to/research-projects.md)
- [How-To: Knowledge Work](../how-to/knowledge-work.md)
