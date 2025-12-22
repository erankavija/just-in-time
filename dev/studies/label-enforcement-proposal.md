# Label Enforcement Proposal

**Date**: 2025-12-10  
**Status**: Proposal  
**Goal**: Programmatically enforce label conventions so agents cannot create invalid issues

---

## Problem

Agents cannot read and internalize all documentation. They will:
1. Forget to add `type:*` labels
2. Confuse `type:epic` with `epic:auth`
3. Create inconsistent label patterns
4. Break strategic view assumptions

**Current state**: Documentation-only approach fails in practice.

---

## Proposed Solution: Validation at Write Time

### 1. Required Labels Check

**Rule**: Every issue MUST have exactly ONE `type:*` label.

```rust
// In issue creation/update
pub fn validate_required_labels(labels: &[String]) -> Result<(), String> {
    let type_labels: Vec<_> = labels.iter()
        .filter(|l| l.starts_with("type:"))
        .collect();
    
    if type_labels.is_empty() {
        return Err(
            "Issue must have a type label. \
             Valid types: type:task, type:epic, type:milestone, type:bug, type:feature, type:research. \
             Example: --label 'type:task'"
        );
    }
    
    if type_labels.len() > 1 {
        return Err(format!(
            "Issue can only have ONE type label. Found: {}. \
             Use --remove-label to remove existing type before adding new one.",
            type_labels.join(", ")
        ));
    }
    
    Ok(())
}
```

**Enforcement points:**
- `jit issue create` - Fail if no `type:*` label provided
- `jit issue update --remove-label` - Fail if trying to remove last `type:*` label
- API `/api/issues` POST/PUT - Return 400 if validation fails

### 2. Strategic Label Consistency

**Rule**: Issues with `type:epic` SHOULD also have `epic:*` label (self-referential).

```rust
pub fn validate_strategic_consistency(labels: &[String]) -> Result<(), ValidationWarning> {
    let type_label = labels.iter()
        .find(|l| l.starts_with("type:"))
        .map(|s| s.as_str());
    
    let has_epic_membership = labels.iter().any(|l| l.starts_with("epic:"));
    let has_milestone_membership = labels.iter().any(|l| l.starts_with("milestone:"));
    
    match type_label {
        Some("type:epic") if !has_epic_membership => {
            return Err(ValidationWarning::MissingEpicLabel(
                "Epic issues should have an epic:* label to create a group identifier. \
                 Example: --label 'epic:auth' for an authentication epic."
            ));
        },
        Some("type:milestone") if !has_milestone_membership => {
            return Err(ValidationWarning::MissingMilestoneLabel(
                "Milestone issues should have a milestone:* label to create a group identifier. \
                 Example: --label 'milestone:v1.0' for version 1.0 release."
            ));
        },
        _ => Ok(())
    }
}
```

**Behavior**: WARNING (not error) - allow but prompt user to confirm.

### 3. CLI Enforcement

```bash
# Creating issue without type label
$ jit issue create --title "Login API" --label "epic:auth"
Error: Issue must have a type label.
Valid types: type:task, type:epic, type:milestone, type:bug, type:feature, type:research
Example: jit issue create --title "Login API" --label "type:task" --label "epic:auth"

# Creating epic without epic:* label
$ jit issue create --title "Auth System" --label "type:epic"
Warning: Epic issues should have an epic:* label to create a group identifier.
Suggestion: --label "epic:auth"
Continue without epic label? [y/N] n
Aborted.

# Success
$ jit issue create --title "Auth System" --label "type:epic" --label "epic:auth"
Created issue: 01ABC
```

### 4. API Enforcement

```rust
// POST /api/issues
async fn create_issue(
    Json(payload): Json<CreateIssueRequest>
) -> Result<Json<Issue>, ApiError> {
    // Validate required labels
    validate_required_labels(&payload.labels)
        .map_err(|e| ApiError::BadRequest(e))?;
    
    // Validate consistency (warning level)
    if let Err(warning) = validate_strategic_consistency(&payload.labels) {
        // Log warning but allow creation
        warn!("Label consistency warning: {}", warning);
    }
    
    // Proceed with creation
    let issue = storage.create_issue(payload)?;
    Ok(Json(issue))
}
```

**Response on validation failure:**
```json
{
  "error": "ValidationError",
  "message": "Issue must have a type label.",
  "details": {
    "valid_types": ["type:task", "type:epic", "type:milestone", "type:bug", "type:feature", "type:research"],
    "example": "Add --label 'type:task' to your request"
  }
}
```

---

## Proposed Validation Levels

### LEVEL 1: ERROR (Blocks operation)

1. **Missing `type:*` label**
   - Message: "Issue must have a type label"
   - Fix: Add `--label "type:task"` (or appropriate type)

2. **Multiple `type:*` labels**
   - Message: "Issue can only have ONE type label"
   - Fix: Remove one with `--remove-label`

3. **Invalid label format**
   - Message: "Invalid label format: 'auth'. Expected 'namespace:value'"
   - Fix: Use `--label "epic:auth"` instead

4. **Unknown namespace** (if registry validation enabled)
   - Message: "Unknown namespace 'foo'. Valid: type, epic, milestone, component, team"
   - Fix: Use standard namespace or register custom one

### LEVEL 2: WARNING (Allows but prompts)

1. **Epic without `epic:*` label**
   - Message: "Epic issues should have an epic:* label"
   - Prompt: "Continue? [y/N]"
   - Bypass: `--force` flag

2. **Milestone without `milestone:*` label**
   - Message: "Milestone issues should have a milestone:* label"
   - Prompt: "Continue? [y/N]"
   - Bypass: `--force` flag

3. **Task without epic/milestone membership**
   - Message: "Task is not associated with any epic or milestone"
   - Prompt: "Continue? [y/N]"
   - Bypass: `--force` flag

### LEVEL 3: INFO (Logs only)

1. **No component label**
   - Log: "Issue has no component label for technical area tracking"

2. **No team label**
   - Log: "Issue has no team label for ownership tracking"

---

## Implementation Plan

### Phase 1: Core Validation (1-2 hours)

1. Add `validate_required_labels()` function to domain model
2. Add validation to `Issue::new()` and `Issue::update_labels()`
3. Add tests for validation logic
4. Update CLI to call validation before storage
5. Return clear error messages

**Files to modify:**
- `crates/jit/src/domain/issue.rs` - Add validation
- `crates/jit/src/cli/issue.rs` - Call validation before create/update
- `crates/jit/src/domain/issue.rs` - Add tests

### Phase 2: Strategic Consistency Warnings (1 hour)

1. Add `validate_strategic_consistency()` with warning level
2. Add `--force` flag to bypass warnings
3. Add interactive prompt in CLI
4. Update tests

### Phase 3: API Validation (30 min)

1. Add validation to API endpoints
2. Return structured error responses
3. Update OpenAPI schema with error responses
4. Test with curl/Postman

### Phase 4: Migration Helper (1 hour)

1. Add `jit validate --fix` command to auto-fix existing issues
2. Scan all issues for missing `type:*` labels
3. Suggest type based on other labels:
   - Has `epic:*` and no other tasks depend on it → suggest `type:epic`
   - Has `milestone:*` and no other tasks depend on it → suggest `type:milestone`
   - Otherwise → suggest `type:task`
4. Allow interactive or automatic fixes

---

## Auto-Fix Heuristics

```rust
pub fn suggest_type_label(issue: &Issue, graph: &DependencyGraph) -> String {
    // If has epic:* label and other issues depend on it, likely an epic
    if issue.labels.iter().any(|l| l.starts_with("epic:")) {
        let dependents = graph.get_dependents(&issue.id);
        if dependents.len() > 2 {
            return "type:epic".to_string();
        }
    }
    
    // If has milestone:* label and many issues depend on it
    if issue.labels.iter().any(|l| l.starts_with("milestone:")) {
        let dependents = graph.get_dependents(&issue.id);
        if dependents.len() > 5 {
            return "type:milestone".to_string();
        }
    }
    
    // If title contains keywords
    if issue.title.to_lowercase().contains("epic") {
        return "type:epic".to_string();
    }
    if issue.title.to_lowercase().contains("release") || 
       issue.title.to_lowercase().contains("milestone") {
        return "type:milestone".to_string();
    }
    
    // Default to task
    "type:task".to_string()
}
```

---

## Validation Command

```bash
# Check for issues without type labels
$ jit validate
Validating repository...
Found 3 issues with missing type labels:
  - 01ABC "Login API" - has epic:auth but no type
  - 02DEF "Payment flow" - has epic:billing but no type
  - 03GHI "Release v1.0" - has milestone:v1.0 but no type

Run 'jit validate --fix' to auto-fix these issues.

# Auto-fix with suggestions
$ jit validate --fix
Issue 01ABC "Login API"
  Suggestion: type:task (has epic:auth, no dependents)
  Apply? [Y/n] y
  ✓ Added type:task

Issue 02DEF "Payment flow"
  Suggestion: type:task (has epic:billing, no dependents)
  Apply? [Y/n] y
  ✓ Added type:task

Issue 03GHI "Release v1.0"
  Suggestion: type:milestone (has milestone:v1.0, 12 dependents)
  Apply? [Y/n] y
  ✓ Added type:milestone

✓ Fixed 3 issues

# Non-interactive fix
$ jit validate --fix --auto
Applying automatic fixes...
✓ Fixed 3 issues
```

---

## MCP Server Integration

Update MCP tool schemas to include validation:

```typescript
{
  name: "issue_create",
  inputSchema: {
    type: "object",
    properties: {
      title: { type: "string" },
      labels: {
        type: "array",
        items: { type: "string" },
        description: "Labels in format 'namespace:value'. REQUIRED: Must include ONE 'type:*' label"
      }
    },
    required: ["title", "labels"]
  },
  // Add validation rules to description
  description: `Create a new issue.

LABEL REQUIREMENTS:
1. MUST include exactly ONE type:* label:
   - type:task (implementation work)
   - type:epic (large feature)
   - type:milestone (release goal)
   - type:bug (defect)
   - type:feature (enhancement)
   
2. SHOULD include epic:* if part of an epic
3. SHOULD include milestone:* if part of a release

Example:
{
  "title": "Implement JWT validation",
  "labels": ["type:task", "epic:auth", "milestone:v1.0"]
}`
}
```

---

## Benefits

### For Agents
1. **Clear error messages** when they make mistakes
2. **Cannot create invalid issues** - fail fast
3. **Examples in error messages** show correct usage
4. **Auto-fix** can correct existing mistakes

### For Humans
1. **Consistent issue structure** guaranteed
2. **Strategic view** always works correctly
3. **No manual cleanup** needed
4. **Self-documenting** via error messages

### For System
1. **Type label always present** - can rely on it
2. **Strategic filtering** always consistent
3. **Validation in one place** - easy to maintain
4. **Migration path** for existing data

---

## Rollout Strategy

### Step 1: Add Validation (Non-Breaking)
- Add validation functions
- Make warnings only (not errors)
- Collect metrics on violations

### Step 2: Fix Existing Data
- Run `jit validate --fix --auto` on existing repository
- Verify all issues have type labels

### Step 3: Enforce (Breaking Change)
- Change warnings to errors
- Update documentation
- Release as minor version bump (0.3.0)

### Step 4: Monitor
- Check logs for validation failures
- Collect common mistakes
- Improve error messages based on data

---

## Open Questions

1. **Should we enforce `type:*` on existing issues during read?**
   - Pro: Catches all issues immediately
   - Con: Breaks existing repositories without migration
   - **Recommendation**: No, only on write operations

2. **Should `type:epic` automatically add `epic:*` label?**
   - Pro: Reduces agent mistakes
   - Con: Need to generate epic namespace value
   - **Recommendation**: Prompt but don't auto-add (ambiguous which value)

3. **Should we require epic/milestone membership for tasks?**
   - Pro: Better organization
   - Con: Too restrictive for small tasks
   - **Recommendation**: Warning only, not error

4. **Should validation be configurable per-repository?**
   - Pro: Flexibility for different workflows
   - Con: Inconsistency across repositories
   - **Recommendation**: Start strict, add config later if needed

---

## Alternative: Auto-Add Default Type

**Proposal**: If no type label provided, auto-add `type:task` as default.

```rust
pub fn normalize_labels(mut labels: Vec<String>) -> Vec<String> {
    let has_type = labels.iter().any(|l| l.starts_with("type:"));
    
    if !has_type {
        labels.push("type:task".to_string());
        info!("Auto-added type:task label (no type specified)");
    }
    
    labels
}
```

**Pros:**
- Never fails validation
- Sensible default (most issues are tasks)
- Smooth agent experience

**Cons:**
- Silent behavior (may surprise users)
- Wrong default for epics/milestones
- Hides the mistake instead of teaching

**Recommendation**: Don't auto-add - explicit is better than implicit. Force agents to think about issue type.

---

## Recommendation

**Implement Phase 1-3 immediately** (2-3 hours total):
1. Required `type:*` validation (ERROR level)
2. Strategic consistency warnings (WARNING level)
3. API validation with structured errors

**Defer Phase 4** (migration helper) until we have real data to validate heuristics.

This gives us:
- ✅ Programmatic enforcement
- ✅ Clear error messages for agents
- ✅ Can't create invalid issues
- ✅ Backward compatible (existing issues still readable)
- ✅ Migration path via `jit validate --fix`

**Total effort**: ~3-4 hours for full implementation.
