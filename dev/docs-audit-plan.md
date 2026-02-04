# Documentation Audit Plan

**Epic:** cfb3ba94 (User Documentation)  
**Story:** 5326b331 (How-To Guides)  
**Date:** 2026-02-04

## Objective

Ensure documentation is consistent, accurate, and without redundancy before closing story 5326b331.

## Issues Found

### Critical (Must Fix)

1. **Broken link in labels.md**
   - Line 7 references `../concepts/dependencies.md` which doesn't exist
   - Fix: Change to `../concepts/core-model.md#dependencies-vs-labels-understanding-the-difference`

2. **Claim command documentation inconsistency**
   - Some docs use `jit issue claim <id> <assignee>` 
   - Others use `jit claim acquire <id> --agent-id <assignee>`
   - Resolution: Both are valid for different use cases:
     - `jit issue claim` - Simple claiming, no TTL management
     - `jit claim acquire` - Full lease control with TTL, renewal, release
   - Action: Document both clearly, use `jit issue claim` for simple examples, `jit claim acquire` for multi-agent coordination

3. **Draft status markers**
   - 14+ files have "Status: Draft" headers
   - Action: Remove all draft markers from completed docs

### Medium (Should Fix)

4. **Gate add syntax inconsistency**
   - Some use `jit gate add <issue> <gate-key>`
   - Others use `jit issue update <id> --add-gate <gate-key>`
   - Resolution: Use `jit gate add` when only adding gates

5. **TDD content duplication**
   - software-development.md and custom-gates.md both explain TDD workflow
   - Action: Keep detailed TDD in software-development.md, reference from custom-gates.md

6. **cli-commands.md incomplete**
   - Has placeholder comments for undocumented commands
   - Action: Complete or remove placeholders

### Low Priority

7. **Cross-reference gaps**
   - Some See Also sections missing back-references
   - Action: Add where beneficial

## File-by-File Changes

### docs/reference/labels.md
- [ ] Fix broken link line 7

### docs/how-to/software-development.md
- [ ] Remove Draft marker (line 3)
- [ ] Verify claim command usage is appropriate (simple cases)
- [ ] Use `jit gate add` instead of `jit issue update --add-gate`

### docs/how-to/custom-gates.md
- [ ] Remove Draft marker
- [ ] Replace TDD duplication with cross-reference to software-development.md
- [ ] Use `jit gate add` consistently
- [ ] Update claim examples to `jit issue claim` for simple cases

### docs/how-to/multi-agent-coordination.md
- [ ] Keep `jit claim acquire` usage (appropriate for multi-agent)
- [ ] Add note explaining when to use `jit issue claim` vs `jit claim acquire`

### docs/concepts/core-model.md
- [ ] Remove Draft marker
- [ ] Verify claim explanation is accurate

### docs/concepts/overview.md
- [ ] Remove Draft marker

### docs/reference/cli-commands.md
- [ ] Remove Draft marker
- [ ] Complete or remove placeholder comments
- [ ] Ensure claim commands documented correctly

### docs/reference/glossary.md
- [ ] Remove Draft marker if present

### docs/reference/storage-format.md
- [ ] Remove Draft marker if present

### docs/index.md
- [ ] Remove "(draft)" annotations from file listings

## Claim Command Guidance

Document this distinction clearly:

**Use `jit issue claim`** when:
- Single developer workflow
- Simple task assignment
- No need for TTL or lease management

**Use `jit claim acquire`** when:
- Multiple agents working in parallel
- Need TTL-based automatic lease expiry
- Need lease renewal for long-running work
- Need explicit lease release

## Success Criteria

- [ ] No broken cross-reference links
- [ ] No "Draft" or "Status: Draft" markers in production docs
- [ ] Consistent command syntax throughout
- [ ] No redundant content (single source of truth)
- [ ] All claim/gate examples use appropriate commands
