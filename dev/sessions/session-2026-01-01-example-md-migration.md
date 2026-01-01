# Session Notes: EXAMPLE.md Migration to Diátaxis

**Date:** 2026-01-01  
**Issue:** d6dc4dfa-6784-43a8-8b8a-761bd5a3f38a  
**Goal:** Migrate EXAMPLE.md content to Diátaxis documentation structure

## Progress Summary

### ✅ Completed

**docs/tutorials/quickstart.md**
- Basic CLI usage (jit init, issue create, dep add)
- Short hash examples (min 4 chars, case-insensitive)
- "Labels are optional" message
- Simple dependency graph example
- Quality gate introduction
- 10-minute learning path

**docs/tutorials/first-workflow.md**
- Complete agent orchestration workflow (30 minutes)
- Epic → tasks → gates → completion lifecycle
- Quality gate setup (automated + manual)
- Dependency graph building (epic ← tasks)
- Multi-agent claiming and coordination
- Dynamic issue discovery pattern
- Gate passing and state transitions

### 📋 Remaining EXAMPLE.md Content Analysis

**Lines 211-230: Monitoring and Observability**
- Event log viewing (jit events tail)
- Event queries by type
- Graph queries (show, downstream, roots)
- **Coordinator status** ← DECISION: Remove (dispatch-specific)

**Lines 231-245: Running with Coordinator Daemon**
- jit coordinator start/stop
- Automatic dispatch
- **DECISION: Remove entirely** - Future topic in jit-dispatch crate docs

**Lines 246-279: Bulk Operations**
- Batch state transitions with --filter
- Batch label management
- Batch reassignment
- Complex query filters (AND/OR/NOT)
- Literal vs smart semantics explanation
- Target: `docs/reference/cli-commands.md` (bulk operations section)

**Lines 280-290: Key Concepts**
- High-level concept list
- **DECISION: Avoid duplication** - Already covered in tutorials
- Skip or incorporate minimally into reference

**Lines 291-330: Advanced Patterns**
- Parallel work with dependencies
- Cross-epic dependencies
- Sequential release planning
- Might duplicate tutorial content - review for uniqueness

**Lines 331-397: Troubleshooting**
- Common issues and solutions
- Gate failures
- Dependency cycles
- Target: `docs/how-to/custom-gates.md` (troubleshooting section)

**Lines 398-468: Label Hierarchy Best Practices**
- When to use labels vs dependencies
- Hierarchy design patterns
- Target: `docs/how-to/software-development.md` OR avoid duplication with concepts/core-model.md

**Lines 469-650: Scripting and Automation**
- Shell script examples
- JSON parsing with jq
- Error handling patterns
- CI/CD integration
- Target: `docs/reference/cli-commands.md` (scripting section)

**Lines 651-663: Next Steps**
- Already incorporated into tutorial next steps
- Skip

**Lines 664-end: Understanding Labels and Warnings**
- Label validation
- Hierarchy enforcement
- Warning messages
- Target: `docs/concepts/core-model.md` (labels section) OR `docs/reference/configuration.md`

## Key Decisions

### 1. Coordinator Daemon Documentation
**Decision:** Remove all coordinator/dispatch daemon content from EXAMPLE.md migration.

**Rationale:**
- Coordinator is in `crates/dispatch` (separate concern)
- Future feature, not core JIT functionality
- Should have its own documentation in dispatch crate
- Removes ~50 lines of content

**Action items:**
- Skip lines 145, 227-230 (coordinator agents command)
- Skip lines 231-245 (coordinator daemon section)
- Do not migrate to Diátaxis docs

### 2. Avoid Duplicate Information
**Decision:** Carefully review all remaining content for duplication with already-written tutorials.

**Areas of potential duplication:**
- Key concepts (lines 280-290) - Already in tutorials
- Basic dependency patterns - Already in first-workflow.md
- Gate basics - Already in quickstart.md and first-workflow.md

**Migration principles:**
- **Reference docs:** Commands, options, exact specifications (no duplication)
- **How-to guides:** Specific recipes not covered in tutorials
- **Concepts:** Theory and mental models (complement, don't duplicate tutorials)
- **Skip entirely:** Content fully covered in tutorials

### 3. Content Prioritization

**High priority (must migrate):**
- Bulk operations - New capability, needs reference docs
- Troubleshooting - Practical help, belongs in how-to
- Scripting patterns - Reference material for automation

**Medium priority (review for uniqueness):**
- Advanced patterns - Check if tutorial already covers
- Label best practices - Check against core-model.md duplication

**Low priority (consider skipping):**
- Key concepts list - Fully covered in tutorials
- Next steps - Already in tutorial navigation

## Content Mapping Plan (Revised)

### docs/how-to/custom-gates.md
**Add:**
- Troubleshooting section (lines 331-397)
  - Common gate failures
  - Debug strategies
  - Resolution patterns

**Skip:**
- Basic gate usage (already in first-workflow.md)

### docs/how-to/software-development.md
**Add (if not duplicative):**
- TDD-specific workflow patterns
- CI/CD integration examples (from scripting section)

**Review first:**
- Label hierarchy practices (compare with core-model.md)

### docs/reference/cli-commands.md
**Add:**
- Bulk operations section (lines 246-279)
  - Filter syntax
  - Batch operations
  - Literal vs smart semantics
- Scripting and automation section (lines 469-650)
  - JSON output parsing
  - Error handling
  - Exit codes

### docs/concepts/core-model.md
**Add (if not already there):**
- Label validation and warnings (lines 664-end)
  - But check if already covered in dependencies vs labels section

## Next Actions

1. **Review current tutorials** - Identify what's already covered
2. **Extract unique content** - Only migrate what adds new value
3. **Bulk operations** - High priority, clear reference material
4. **Troubleshooting** - High priority, practical how-to content
5. **Scripting** - Medium priority, useful for automation
6. **Delete EXAMPLE.md** - After migration complete

## Notes

- Tutorials are comprehensive - many concepts already well-covered
- Focus on reference and how-to content (different Diátaxis categories)
- Avoid creating redundancy - readers should find info in one place
- Coordinator content removal saves time and reduces scope creep
