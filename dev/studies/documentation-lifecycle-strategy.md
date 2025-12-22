# Documentation Lifecycle Strategy

## Problem Statement

Currently:
- All design docs created in `docs/`
- Linked to active issues (good!)
- But what happens when issues complete?
- docs/ will accumulate 100s of historical design docs over time

**Need:** Clear lifecycle from active → completed → archived

## Proposed Documentation Lifecycle

### Stage 1: Active Development (`docs/design/`)
**When:** Issue in backlog/ready/in_progress/gated states
**Location:** `docs/design/<feature-name>.md`
**Linked:** Via `jit doc add <issue-id> docs/design/<feature>.md`

Example:
```bash
jit issue create --title "Epic: Webhooks" --gate code-review
jit doc add $EPIC docs/design/webhooks.md --doc-type design
# Doc stays in docs/design/ while issue is active
```

### Stage 2: Recently Completed (`docs/design/` - short retention)
**When:** Issue moves to done/rejected
**Location:** Still in `docs/design/` for 1-2 releases
**Why:** Easy reference for recent changes, troubleshooting

### Stage 3: Archived (`.jit/docs/archive/`)
**When:** After 1-2 releases OR manually archived
**Location:** `.jit/docs/archive/<category>/<feature>.md`
**Linked:** Link still works in issue metadata

Example:
```bash
# After v1.1 release, archive v1.0 implementation docs
mv docs/design/rejection-state.md .jit/docs/archive/features/
mv docs/design/gate-enforcement-bug-analysis.md .jit/docs/archive/bug-fixes/

# jit doc references still work (path tracked in issue)
jit doc show <issue-id> rejection-state.md  # Shows archived doc
```

## Archive Organization

```
.jit/
└── docs/
    └── archive/
        ├── features/          (completed feature designs)
        │   ├── rejection-state-design.md
        │   ├── label-hierarchy-implementation.md
        │   └── gate-system-v1.md
        │
        ├── bug-fixes/         (bug analysis docs)
        │   ├── gate-enforcement-bug.md
        │   └── precheck-bypass-fix.md
        │
        ├── refactorings/      (refactoring plans)
        │   ├── binary-to-library.md
        │   └── generic-dag.md
        │
        ├── sessions/          (dev session notes)
        │   └── 2025-12-*.md
        │
        └── pre-jit/           (before we started dogfooding)
            ├── type-hierarchy-enforcement-proposal.md
            └── ...
```

## Active docs/ Structure

```
docs/
├── README.md                  (navigation + archive policy)
│
├── guides/                    (PERMANENT user guides - never archive)
│   ├── labels.md
│   ├── gates.md
│   ├── file-locking.md
│   └── research-workflows.md
│
├── design/                    (ACTIVE feature designs - archive when done)
│   ├── observability.md       (active epic)
│   ├── production-stability.md (active epic)
│   ├── agent-validation.md    (active epic)
│   └── webhooks.md            (future: archive after completion)
│
├── architecture/              (PERMANENT architecture docs - rarely archive)
│   ├── overview.md
│   ├── cli-and-mcp-strategy.md
│   ├── web-ui.md
│   ├── search.md
│   └── ...
│
├── development/               (PERMANENT developer docs)
│   ├── contributing.md
│   ├── architecture-pitfalls.md
│   └── clippy-suppressions.md
│
└── vision/                    (TEMPORARY - archive when implemented OR rejected)
    ├── knowledge-management.md
    ├── document-graph.md
    └── webhooks-vision.md
```

## Document Categories & Lifecycle

| Category | Location | Archive? | When to Archive |
|----------|----------|----------|----------------|
| **User Guides** | docs/guides/ | NEVER | Permanent, update in place |
| **Architecture** | docs/architecture/ | RARELY | Only if major redesign obsoletes |
| **Development** | docs/development/ | NEVER | Permanent reference |
| **Active Designs** | docs/design/ | YES | 1-2 releases after completion |
| **Visions** | docs/vision/ | YES | When implemented or rejected |
| **Bug Analyses** | docs/design/ | YES | Immediately after fix merged |
| **Session Notes** | docs/design/ | YES | Immediately (or don't create) |

## Workflow Examples

### Example 1: Feature Development
```bash
# Create epic with design doc
EPIC=$(jit issue create --title "Epic: Webhooks" --priority high)
echo "# Webhooks Design..." > docs/design/webhooks.md
jit doc add $EPIC docs/design/webhooks.md --doc-type design

# Work happens...
jit issue update $EPIC --state done

# After v1.1 release, archive
mkdir -p .jit/docs/archive/features
mv docs/design/webhooks.md .jit/docs/archive/features/
# Link still works - jit tracks the path in issue metadata
```

### Example 2: Bug Fix
```bash
# Bug found, create analysis doc
BUG=$(jit issue create --title "Fix: Race condition in claim" --priority critical)
echo "# Race Condition Analysis..." > docs/design/claim-race-analysis.md
jit doc add $BUG docs/design/claim-race-analysis.md --doc-type analysis

# Fix merged
jit issue update $BUG --state done

# Archive immediately (no need to keep in active docs)
mv docs/design/claim-race-analysis.md .jit/docs/archive/bug-fixes/
```

### Example 3: Session Notes
```bash
# Option A: Don't create separate file, use issue description/context
jit issue update $ISSUE --description "$(cat session-notes.txt)"

# Option B: Create doc but archive immediately
echo "# Session 2025-12-20..." > docs/design/session-2025-12-20.md
jit doc add $ISSUE docs/design/session-2025-12-20.md
mv docs/design/session-2025-12-20.md .jit/docs/archive/sessions/
```

## Archive Policy (docs/README.md)

```markdown
## Archive Policy

### What Gets Archived
- Feature design docs after 1-2 releases
- Bug analysis docs immediately after fix
- Refactoring plans after completion
- Session notes immediately
- Vision docs when implemented/rejected

### What Stays Active
- User guides (docs/guides/) - permanent
- Architecture docs (docs/architecture/) - long-lived
- Development docs (docs/development/) - permanent
- Active feature designs (docs/design/) - until completed

### When to Archive
Run quarterly (or per release):
```bash
# Archive completed designs from last release
for doc in docs/design/*-design.md; do
  # Check if linked issue is done and old enough
  # Move to .jit/docs/archive/features/
done
```

### Searching Archived Docs
```bash
# jit search works across all files
jit search "webhook" --glob "**/*.md"

# Find doc in issue
jit issue show <issue-id>
jit doc list <issue-id>
jit doc show <issue-id> <path>  # Works even if archived
```
```

## Implementation for Current Restructuring

### Phase 1: Archive Pre-Jit Historical Docs
```bash
mkdir -p .jit/docs/archive/pre-jit
mv docs/type-hierarchy-*.md .jit/docs/archive/pre-jit/
mv docs/label-*-plan.md .jit/docs/archive/pre-jit/
mv docs/session-2025-*.md .jit/docs/archive/pre-jit/
mv docs/*-refactoring-plan.md .jit/docs/archive/pre-jit/
# etc - all the ~20 historical docs
```

### Phase 2: Archive Recently Completed (Linked) Docs
```bash
# These are linked to done issues but can be archived
mkdir -p .jit/docs/archive/{bug-fixes,analyses}

# Bug fixes already done
mv docs/gate-enforcement-bug-analysis.md .jit/docs/archive/bug-fixes/
mv docs/state-transition-feedback-design.md .jit/docs/archive/analyses/

# Issues still have doc references, but files are archived
```

### Phase 3: Keep Active Designs in docs/design/
```bash
# These stay - linked to active/recent epics
docs/design/
├── ci-gate-integration-design.md    (completed but recent - keep)
├── gate-examples.md                 (completed but recent - keep)
├── rejection-state-design.md        (completed but recent - keep)
├── observability-design.md          (active epic)
├── production-stability-design.md   (active epic)
├── production-polish-design.md      (active epic)
└── agent-validation-design.md       (active epic)

# After v1.1 release, archive the completed ones
```

### Phase 4: Reorganize Permanent Docs
```bash
mkdir -p docs/{guides,architecture,development,vision}

# User guides (permanent)
mv docs/label-conventions.md docs/guides/labels.md  # consolidate
mv docs/file-locking-usage.md docs/guides/file-locking.md
# etc

# Architecture (permanent)
mv docs/cli-and-mcp-strategy.md docs/architecture/
mv docs/web-ui-architecture.md docs/architecture/
# etc
```

## Benefits of This Approach

1. **Clean docs/**: Only active/recent/permanent docs visible
2. **Clear lifecycle**: Design doc → Archive when done
3. **Searchable history**: `jit search` works in archives
4. **Issue linkage preserved**: Archived docs still linked via jit
5. **Scalable**: Won't accumulate 100s of files in docs/
6. **Dogfooding**: Uses jit's own document tracking system
7. **Git-friendly**: Archives still versioned, just organized

## Decision Points

1. **Retention period**: Keep completed designs for 1 release or 2?
   - Recommendation: 1-2 releases (2-6 months)

2. **Auto-archive or manual?**
   - Recommendation: Manual initially, could add `jit doc archive` command later

3. **Session notes - create at all?**
   - Recommendation: Use issue descriptions/context instead, or archive immediately

4. **Archive trigger - per release or ad-hoc?**
   - Recommendation: Ad-hoc initially, add to release checklist

## Proposed `docs/README.md`

```markdown
# Documentation Organization

## Active Documentation

### User Guides (docs/guides/)
Permanent user-facing documentation. Updated in place.
- `labels.md` - Label system and hierarchy
- `gates.md` - Quality gate system
- `file-locking.md` - Multi-agent concurrency
- `research-workflows.md` - Research use cases

### Architecture (docs/architecture/)
Long-lived architectural documentation. Rarely archived.
- `overview.md` - System architecture
- `cli-and-mcp-strategy.md` - CLI design principles
- `web-ui.md` - Frontend architecture
- `search.md` - Search implementation

### Development (docs/development/)
Permanent developer reference. Never archived.
- `contributing.md` - Contribution guidelines
- `architecture-pitfalls.md` - Common issues

### Active Designs (docs/design/)
Feature designs linked to active or recently completed issues.
**Archived 1-2 releases after completion.**

Current active designs:
- `observability-design.md` - Epic: Production Observability
- `production-stability-design.md` - Epic: Production Stability
- (See linked issues via `jit issue show <id>`)

### Vision (docs/vision/)
Future feature plans. Archived when implemented or rejected.

## Archived Documentation

Located in `.jit/docs/archive/`:
- `features/` - Completed feature designs
- `bug-fixes/` - Bug analysis docs
- `pre-jit/` - Historical docs before jit tracking
- `sessions/` - Development session notes

**Search archived docs:** `jit search "term" --glob "**/*.md"`
**View archived doc:** `jit doc show <issue-id> <filename>`

## Archive Policy

Documents are archived to `.jit/docs/archive/` when:
- Feature designs: 1-2 releases after issue completion
- Bug analyses: Immediately after fix merged
- Session notes: Immediately (or don't create as separate files)
- Vision docs: When implemented or rejected

Archived documents remain:
- In git history
- Searchable via `jit search`
- Linked from issues via `jit doc`
```

