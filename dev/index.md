# JIT Development Documentation

Welcome to the **development documentation** for Just-In-Time (JIT). This documentation is for contributors working on JIT itself.

## Documentation Domains

### 📘 Product Documentation
**User-facing, permanent reference** → See [docs/index.md](../docs/index.md)

Product docs explain *what JIT is* and *how to use it* - for end users, not contributors.

### 🔨 Development Documentation (You Are Here)
**Contributor-facing, lifecycle-managed** → This directory (`dev/`)

Development docs explain *how we build JIT* - design decisions, architecture, investigations, and active work.

---

## Directory Structure

### 🚧 [active/](active/) - Active Development
Design documents for **features currently in progress**, linked to open issues in `.jit/`.

**Lifecycle:** Moved here when work starts → Archived to `archive/` when issue completes

**Examples:** Feature designs, implementation plans for open issues

### 🎨 [design/](design/) - Design Explorations
Feature and interface design proposals: web UI interaction models and CLI quality explorations.

### 🗂️ [plans/](plans/) - Implementation Plans
Implementation plans for scoped work items, each keyed to its issue id.

### 🏗️ [architecture/](architecture/) - System Architecture
**Permanent internal reference** for core system design and architecture.

**Lifecycle:** Permanent (does not archive)

**Contents:**
- [core-system-design.md](architecture/core-system-design.md) - Foundational system design
- [web-ui-architecture.md](architecture/web-ui-architecture.md) - Web UI design
- [graph-filtering-architecture.md](architecture/graph-filtering-architecture.md) - Graph query design
- [cli-and-mcp-strategy.md](architecture/cli-and-mcp-strategy.md) - CLI and MCP integration

### 🔮 [vision/](vision/) - Vision & Charter
**Forward-looking explorations** and vision documents for future features, plus the v1.0 [charter](vision/9db27a3a-charter.md).

**Lifecycle:** May archive if abandoned, otherwise permanent

**Examples:** Future feature proposals, strategic planning documents, the product charter

### 🔬 [studies/](studies/) - Investigations & Reference
**Completed investigations, analyses**, and active reference guides.

**Lifecycle:** Active reference stays; completed studies may archive after 1-2 releases

**Examples:** Performance analyses, design explorations, coding conventions, quick references

### 🧪 [eval/](eval/) - Skill Evaluations
Evaluation records and harnesses for the project's agent skills: baselines, adjudications, and the trigger and steering eval runners.

### ⚗️ [experiments/](experiments/) - Coordination Experiments
Records of process and coordination experiments, such as manual worktree parallel-work trials.

### 📝 [sessions/](sessions/) - Session Notes
**Development session notes** documenting work-in-progress.

**Lifecycle:** Archived to `archive/sessions/` after 1-2 releases

**Naming:** `session-YYYY-MM-DD-topic.md`

### 🎞️ [presentations/](presentations/) - Talk Decks
Reveal.js presentation decks and their assets.

### 📦 [archive/](archive/) - Completed Work
**Archived documentation** from completed work, organized by category.

**Structure:**
- `archive/features/` - Completed features (from `active/`)
- `archive/bug-fixes/` - Completed bug fixes (from `active/`)
- `archive/refactorings/` - Completed refactorings (from `active/`)
- `archive/studies/` - Completed investigations (from `studies/`)
- `archive/sessions/` - Old session notes (from `sessions/`)

**Retention:** 1-2 releases after completion

---

## Documentation Lifecycle

```
active/         → Work starts (linked to issue)
                ↓
                Work completes (issue Done)
                ↓
archive/        → After 1-2 releases
```

**What never archives:**
- `architecture/` - Permanent internal reference
- `vision/` - Unless abandoned
- `docs/` - Product documentation (different domain)

---

## Key Documents

### Architecture & Design
- [core-system-design.md](architecture/core-system-design.md) - Start here for system overview
- [web-ui-architecture.md](architecture/web-ui-architecture.md) - Web UI design

### Development Guides
- See [docs/tutorials/quickstart.md](../docs/tutorials/quickstart.md) - Getting started (10 min)
- See [docs/reference/cli-commands.md](../docs/reference/cli-commands.md#mcp-tools-reference) - MCP tools for agents
- See [../AGENTS.md](../AGENTS.md) - Contributor and agent guidance for this repository

### Reference
- [studies/architecture-pitfalls.md](studies/architecture-pitfalls.md) - Common pitfalls
- [studies/clippy-suppressions.md](studies/clippy-suppressions.md) - Documented suppressions

### Strategy
- [studies/documentation-organization-strategy.md](studies/documentation-organization-strategy.md) - This reorganization
- [vision/knowledge-management-vision.md](vision/knowledge-management-vision.md) - Future vision

---

## For Contributors

**Getting Started:**
1. Read [../AGENTS.md](../AGENTS.md)
2. Review [architecture/core-system-design.md](architecture/core-system-design.md)
3. Use `jit query available` to find tasks

**Adding Documentation:**
- **Active work?** → Add design doc to `active/` and link to issue
- **Architectural decision?** → Add to `architecture/`
- **Investigation/analysis?** → Add to `studies/`
- **Session notes?** → Add to `sessions/` (use `session-YYYY-MM-DD-topic.md`)
- **User guide?** → Add to `docs/` (product documentation)

**Authoring guidelines:** See [authoring-conventions.md](authoring-conventions.md) for asset management and link patterns that ensure documents remain portable during archival.

**Review policy ownership:** Treat applicable `AGENTS.md` files as canonical
engineering prose and configured addressable-item sources as canonical for
registry or issue policy. Repository-specific review prompts describe how to
discover and judge that policy; they do not duplicate it. The shared AI-review
wrapper stays tool-agnostic and only transports context plus the structured
verdict contract. When a qualified item governs a finding, record its resolved
ID in the finding's optional `references` array so the review remains
traceable. See [Custom Gates](../docs/how-to/custom-gates.md#ground-a-repository-review-in-canonical-policy)
for the integration pattern.

**See also:**
- [docs/index.md](../docs/index.md) - Product documentation
- [TESTING.md](TESTING.md) - Testing strategy

---

*This structure was established in Issue 165cf162-1cb1-491d-8c92-b2fb571e7f4c*
