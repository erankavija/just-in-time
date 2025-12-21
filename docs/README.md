# JIT Documentation

**Welcome to the Just-In-Time documentation.** This guide helps you navigate our documentation, understand archival policies, and follow authoring conventions.

## Organization Overview

Our documentation is **domain-agnostic** — the structure and conventions work equally well for software development, research projects, knowledge work, or any workflow where you track issues and their context.

### Directory Structure

```
docs/
├── README.md              # This file - navigation and conventions
├── design.md              # Core architecture reference
├── *.md                   # Active documentation (guides, designs, strategies)
├── diagrams/              # Shared visual assets
└── (see .jit/docs/archive/ for completed work)

.jit/docs/archive/         # Archived documentation
├── features/              # Completed feature designs
├── bug-fixes/             # Bug analysis and fixes
├── refactorings/          # Architecture changes
└── sessions/              # Implementation session notes
```

**Key principle:** `docs/` contains **active and recently completed** work. Older documentation moves to `.jit/docs/archive/` to keep the workspace focused.

### Domain-Agnostic Approach

This structure adapts to different project types:

- **Software development** (our use): Feature designs in active/, architecture in reference/
- **Research projects**: Hypotheses and experiments in active/, published papers in archive/
- **Knowledge work**: Project plans in active/, completed projects in archive/

The mechanics are the same: link docs to issues, track lifecycle, archive when done.

## Archive Policy

### When to Archive

Documents move to archive when:

1. **Issue completes** - Design docs for Done/Rejected issues (retain 1-2 releases)
2. **Feature ships** - Implementation guides after stabilization
3. **Manual archival** - You decide a document is historical

### Archive Categories

- **features/** - Completed feature designs and implementation plans
- **bug-fixes/** - Bug analyses and resolution documentation
- **refactorings/** - Architecture changes and migration guides
- **sessions/** - Development session notes and progress logs

### Retention Periods

- **Active docs**: Current work, linked to in-progress issues
- **Recently completed**: 1-2 releases after issue completion
- **Archived**: Permanent, searchable via `jit search`

### Finding Archived Docs

```bash
# Search all documentation (including archives)
jit search "transitive reduction"

# View doc from issue (works for archived docs too)
jit doc show <issue-id> <path>

# List archived documents
ls .jit/docs/archive/features/
```

## Authoring Conventions

### Asset Management Patterns

**Critical for documentation lifecycle:** Choose the right asset pattern so documents remain portable and links don't break during archival.

#### Pattern 1: Per-Document Assets

For assets specific to one document:

```
docs/
├── my-feature-design.md
└── my-feature-design/          # Assets directory named after doc
    ├── architecture.png
    └── flow-diagram.svg
```

**Link relatively:**
```markdown
![Architecture](my-feature-design/architecture.png)
```

**On archival:** Move both doc and assets directory together:
```bash
mv docs/my-feature-design.md .jit/docs/archive/features/
mv docs/my-feature-design/ .jit/docs/archive/features/
# Links still work - relative paths preserved
```

#### Pattern 2: Shared Assets

For assets used by multiple documents:

```
docs/
├── diagrams/                   # Shared assets directory
│   ├── system-overview.png
│   └── data-flow.svg
├── feature-a.md
└── feature-b.md
```

**Link with root-relative paths:**
```markdown
![System Overview](/docs/diagrams/system-overview.png)
```

**On archival:** Shared assets remain when docs archive. Use for:
- Architecture diagrams referenced by multiple features
- Logo, branding assets
- Common workflow diagrams

#### Pattern 3: External Assets

For external resources:

```markdown
![Rust Book](https://doc.rust-lang.org/book/cover.png)
[GitHub Issue](https://github.com/org/repo/issues/123)
```

**Note:** External URLs are preserved in documentation but handled specially in snapshots (marked as external, not bundled).

## Project-Specific Conventions

**This is a software development project.** Here's how we use the structure:

### Active Documentation

- **Feature designs**: `docs/short-hash-implementation-plan.md`
- **Architecture decisions**: `docs/design.md`
- **Implementation strategies**: `docs/documentation-lifecycle-strategy.md`
- **Session notes**: `docs/session-2025-12-21-short-hash-progress.md`

### Archive Organization

Our categories map to software development workflows:

- **features/** - New capabilities (e.g., short hash support, gate system)
- **bug-fixes/** - Bug analyses and resolutions
- **refactorings/** - Code quality improvements (e.g., transitive reduction)
- **sessions/** - Development session logs (moved after completion)

## Multi-Domain Examples

### Research Project Conventions

```
docs/
├── README.md
├── hypothesis-dark-matter-detection.md    # Active hypothesis
├── experiment-protocol-v2.md              # Current methodology
└── hypothesis-dark-matter-detection/
    └── detector-calibration.png

.jit/docs/archive/
└── publications/
    ├── nature-paper-2024.md               # Published research
    └── rejected-hypotheses/
        └── luminosity-correlation.md
```

### Knowledge Work Example

```
docs/
├── README.md
├── project-website-redesign.md            # Active project
├── meeting-notes-2025-q1.md              # Current quarter
└── project-website-redesign/
    └── mockups.png

.jit/docs/archive/
└── completed-projects/
    ├── brand-refresh-2024.md
    └── office-relocation/
        └── floor-plan.pdf
```

## Search and Discovery

### Search Documentation

```bash
# Find any mention of a term
jit search "gate"

# Search in specific paths
jit search "lifecycle" --glob "docs/**/*.md"
```

### Find Docs from Issues

```bash
# List documents linked to an issue
jit doc list <issue-id>

# View specific document
jit doc show <issue-id> <path>

# See document history
jit doc history <issue-id> <path>
```

### Browse Archives

```bash
# List archived categories
ls .jit/docs/archive/

# View archived document
cat .jit/docs/archive/features/rejection-state.md

# Or use jit if it's linked to an issue
jit doc show <issue-id> .jit/docs/archive/features/rejection-state.md
```

## Related Documentation

- [Documentation Lifecycle Design](documentation-lifecycle-design.md) - Full design specification
- [Documentation Lifecycle Strategy](documentation-lifecycle-strategy.md) - Implementation approach
- [Phase 2 Design](documentation-lifecycle-phase2-design.md) - Asset management and snapshots

---

**Questions?** Check [CONTRIBUTOR-QUICKSTART.md](../CONTRIBUTOR-QUICKSTART.md) or create an issue.
