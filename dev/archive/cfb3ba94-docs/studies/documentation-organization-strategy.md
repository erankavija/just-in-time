# Documentation Organization Strategy

**Status:** Proposed comprehensive reorganization  
**Issue:** 165cf162-1cb1-491d-8c92-b2fb571e7f4c  
**Created:** 2025-12-22  

## Executive Summary

Reorganize JIT documentation into two clear domains:
1. **Product documentation** (`docs/`) - Permanent, user-facing, domain-agnostic using Diátaxis framework
2. **Development documentation** (`dev/`) - Lifecycle-managed, contributor-facing, with JIT archival

This separates "what users need to know" from "how we build it" while maintaining domain-agnostic principles throughout.

## Goals

### Primary Goals
- Clear distinction between product docs and development docs
- Domain-agnostic structure and terminology (works for software, research, knowledge work)
- Diátaxis framework for product documentation (tutorials, how-to, reference, explanation)
- Lifecycle management only for development docs
- Prevent confusion between "how to use JIT" and "how we develop JIT"

### Non-Goals
- Rendering/publishing pipelines (future work)
- Migration of historical archived content (keep in place)
- Automated archival (manual process remains)

## Motivation

**Current Problems:**
1. Mixed user-facing and internal documentation in `docs/`
2. Unclear which docs are permanent vs. temporary
3. Domain-specific bias (appears software-only)
4. No clear framework for content types (tutorial vs. reference vs. how-to)
5. Archival policy applies to all docs, even stable reference material

**User Impact:**
- New users confused by internal design documents
- Researchers unsure if JIT supports their domain
- Unclear where to find specific information types

**Contributor Impact:**
- Unclear where to put new documentation
- Active designs mixed with permanent reference
- No clear lifecycle for development docs

## Proposed Structure

### Top-Level Organization

```
docs/                          # USER-FACING PRODUCT DOCUMENTATION
├── index.md                   # Product docs landing (Diátaxis navigation)
├── concepts/                  # EXPLANATION (Diátaxis)
│   ├── overview.md            # What is JIT?
│   ├── scope.md               # Full domain coverage
│   ├── model.md               # Core concepts
│   └── invariants.md          # System guarantees
├── tutorials/                 # TUTORIALS (Diátaxis)
│   ├── quickstart.md          # Domain-agnostic first experience
│   └── first-workflow.md      # Complete example
├── how-to/                    # HOW-TO GUIDES (Diátaxis)
│   ├── software-development.md
│   ├── research-projects.md
│   ├── knowledge-work.md
│   ├── custom-gates.md
│   └── dependency-management.md
├── reference/                 # REFERENCE (Diátaxis)
│   ├── cli.md                 # Complete CLI reference
│   ├── storage.md             # On-disk format
│   ├── api.md                 # Programmatic API
│   └── configuration.md       # Config options
├── case-studies/              # Real-world examples
│   └── developing-jit-with-jit.md
└── glossary.md                # Term definitions

dev/                           # DEVELOPMENT DOCUMENTATION
├── index.md                   # Dev docs landing
├── active/                    # Work-in-progress (linked to issues)
│   └── *.md                   # Designs for in-progress features
├── architecture/              # System internals
│   ├── core-system-design.md
│   ├── storage-abstraction.md
│   └── graph-algorithms.md
├── vision/                    # Future directions
│   └── *.md                   # Explorations and plans
├── studies/                   # Investigations, analyses
│   └── *.md                   # Research and experiments
├── sessions/                  # Development session notes
│   └── session-YYYY-MM-DD-topic.md
└── archive/                   # JIT-managed archive
    ├── features/
    ├── bug-fixes/
    ├── refactorings/
    ├── studies/
    └── sessions/
```

## Design Principles

### 1. Two-Domain Separation

**Product Documentation (`docs/`):**
- **Audience:** End users, new contributors learning JIT
- **Tone:** Neutral, educational, complete
- **Lifecycle:** Permanent reference material (does not archive)
- **Scope:** Full domain coverage (software, research, knowledge work)
- **Framework:** Diátaxis (tutorials, how-to, reference, explanation)

**Development Documentation (`dev/`):**
- **Audience:** Active contributors, maintainers
- **Tone:** "We" language, opinionated, evolving
- **Lifecycle:** Active → archive as work completes
- **Scope:** How we build JIT, design decisions, investigations
- **Framework:** Lifecycle-based (active/architecture/vision/studies/sessions)

### 2. Diátaxis Framework for Product Docs

Following [Diátaxis](https://diataxis.fr/):

| Type | Purpose | Content |
|------|---------|---------|
| **Tutorials** | Learning-oriented | Step-by-step lessons |
| **How-to Guides** | Goal-oriented | Recipes for use cases |
| **Reference** | Information-oriented | Technical specs |
| **Explanation** | Understanding-oriented | Concepts and design |

### 3. Domain-Agnostic Language

**Product documentation terminology:**
- ✅ Issue, Gate, Dependency, Assignee, State, Event, Coordinator
- ✅ "Work tracking", "quality checkpoints", "task orchestration"
- ❌ Pull request, commit, build (software-specific)
- ❌ Experiment, hypothesis (research-specific)

**Domain-specific terms in how-to guides:**
- Software: Issue → Feature/Bug, Gate → CI check
- Research: Issue → Experiment, Gate → Pre-registration
- Knowledge work: Issue → Project task, Gate → Approval

### 4. Configurable Development Directory

Configuration in `.jit/config.toml`:
```toml
[documentation]
development_root = "dev"  # Configurable per project
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]
```

## Archive Policy

### What Archives (Development Docs Only)

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

## Migration Plan

### Phase 1: Create Structure (Issue 165cf162)

1. Create directory structure
2. Create index files (docs/index.md, dev/index.md)
3. Move existing files to appropriate locations
4. Update `.jit/config.toml` with archival configuration
5. Update all internal links
6. Update issue document references

**File migrations:**
- Active designs → `dev/active/`
- Architecture docs → `dev/architecture/`
- Vision docs → `dev/vision/`
- Studies/analyses → `dev/studies/`
- Session notes → `dev/sessions/`

### Phase 2: Create Product Docs (Follow-up)

Create core product documentation:
- `docs/concepts/scope.md` - Full domain coverage
- `docs/tutorials/quickstart.md` - Domain-agnostic tutorial
- `docs/how-to/*.md` - Domain-specific guides
- `docs/reference/cli.md` - Complete CLI reference

### Phase 3: Extract Dogfooding (Follow-up)

Move team-specific content:
- README → `docs/case-studies/developing-jit-with-jit.md`
- Keep README neutral and concise

## Writing Guidelines

### Product Docs (`docs/`)

- Third person, neutral tone
- Show multiple domain examples
- Generic examples in tutorials
- Domain mapping tables in how-tos

### Development Docs (`dev/`)

- "We" language, opinionated
- Design rationale and tradeoffs
- Implementation details
- Team conventions

## Success Metrics

- ✅ Clear `docs/` vs `dev/` separation
- ✅ Diátaxis structure in place
- ✅ Domain-agnostic terminology
- ✅ No broken links
- ✅ Archive only applies to `dev/`

## References

- [Diátaxis Framework](https://diataxis.fr/)
- Issue 165cf162: Reorganize active docs
- Issue ba3e3884: Documentation navigation

## Next Steps

1. Review and approve this strategy
2. Update issue 165cf162 description
3. Execute Phase 1 migration
4. Create follow-up issues for Phases 2-3
