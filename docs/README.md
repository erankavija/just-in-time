# JIT Documentation

**Welcome to the Just-In-Time documentation.** This guide helps you navigate our documentation organization.

## Two Documentation Domains

JIT separates documentation into two clear domains:

### 📘 Product Documentation (`docs/`)
**User-facing, permanent reference** → See [index.md](index.md)

- What JIT is and how to use it
- Domain-agnostic terminology and examples
- Follows [Diátaxis](https://diataxis.fr/) framework (tutorials, how-to, reference, explanation)
- **Never archives** - permanent reference material

**Start here:** [docs/index.md](index.md)

### 🔨 Development Documentation (`dev/`)
**Contributor-facing, lifecycle-managed** → See [dev/index.md](../dev/index.md)

- How we build JIT
- Design decisions, architecture, investigations
- Active work linked to issues
- **Archives on completion** - keeps workspace focused

**For contributors:** [dev/index.md](../dev/index.md)

---

## Directory Structure

```
docs/                      # PRODUCT DOCUMENTATION (permanent)
├── index.md              # Product docs landing page
├── concepts/             # Explanation (what is JIT, core model)
├── tutorials/            # Learning-oriented (getting started)
├── how-to/               # Goal-oriented (specific use cases)
├── reference/            # Information-oriented (CLI, storage, API)
└── diagrams/             # Shared visual assets

dev/                       # DEVELOPMENT DOCUMENTATION (lifecycle-managed)
├── index.md              # Dev docs landing page
├── active/               # Designs for in-progress issues
├── architecture/         # Core system internals (permanent)
├── vision/               # Future planning
├── studies/              # Investigations and reference
├── sessions/             # Development session notes
└── archive/              # Completed work
    ├── features/
    ├── bug-fixes/
    ├── refactorings/
    ├── studies/
    └── sessions/
```

**Key principle:** `docs/` is for users, `dev/` is for contributors. Archives apply only to `dev/`.

---

## Archive Policy (Development Docs Only)

**Important:** Archival applies **only to `dev/`** directory. Product docs in `docs/` never archive.

### What Archives

**Subject to archival:**
- `dev/active/*.md` - When linked issue completes
- `dev/studies/*.md` - When investigation completes  
- `dev/sessions/*.md` - After 1-2 releases

**Retention:** 1-2 releases after completion

### What Never Archives

**Permanent documentation:**
- All of `docs/` (product documentation)
- `dev/architecture/*.md` (foundational reference)
- `dev/vision/*.md` (may archive if abandoned)

### Archive Organization

```
dev/archive/
├── features/       # From dev/active/
├── bug-fixes/      # From dev/active/
├── refactorings/   # From dev/active/
├── studies/        # From dev/studies/
└── sessions/       # From dev/sessions/
```

### Finding Documentation

```bash
# Search all documentation (including archives)
jit search "transitive reduction"

# View doc from issue (works for archived docs too)
jit doc show <issue-id> <path>

# Browse product docs
cat docs/index.md

# Browse development docs  
cat dev/index.md

# List archived documents
ls dev/archive/features/
```

## Authoring Conventions

**For development documentation authors:** See [authoring-conventions.md](authoring-conventions.md) for comprehensive guidelines on writing documentation that is safe to archive and move.

### Asset Management Patterns (Summary)

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
mv dev/active/my-feature-design.md dev/archive/features/
mv dev/active/my-feature-design/ dev/archive/features/
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

- **Feature designs**: `dev/active/short-hash-implementation-plan.md`
- **Architecture decisions**: `dev/architecture/core-system-design.md`
- **Implementation strategies**: `dev/studies/documentation-lifecycle-strategy.md`
- **Session notes**: `dev/sessions/session-2025-12-21-short-hash-progress.md`

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
ls dev/archive/

# View archived document  
cat dev/archive/features/rejection-state.md

# Or use jit if it's linked to an issue
jit doc show <issue-id> dev/archive/features/rejection-state.md
```

## Related Documentation

- [Documentation Organization Strategy](../dev/studies/documentation-organization-strategy.md) - Full reorganization design
- [Documentation Lifecycle Design](../dev/active/documentation-lifecycle-design.md) - Lifecycle management
- [Documentation Lifecycle Phase 2](../dev/active/documentation-lifecycle-phase2-design.md) - Asset management

---

**Questions?** Check [CONTRIBUTOR-QUICKSTART.md](../CONTRIBUTOR-QUICKSTART.md) or [dev/index.md](../dev/index.md).
