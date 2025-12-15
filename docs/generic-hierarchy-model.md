# Generic Issue Hierarchy Model

**Date**: 2025-12-11  
**Status**: Proposal  
**Goal**: Generic, extensible hierarchy model that works with existing dependency graph

---

## Problem Statement

Current approach has limitations:
1. **Hardcoded strategic labels** - "epic" and "milestone" assumed in validation
2. **Binary hierarchy** - Only strategic vs tactical (2 levels)
3. **Not extensible** - Cannot model 3+ level hierarchies (portfolio → program → epic → story → task)
4. **Coupled to labels** - Hierarchy logic scattered across label validation code

**Need**: Generic hierarchy model that:
- Works with existing DAG
- Supports arbitrary number of levels (2, 3, 4, ...)
- Fully configurable via registry
- Decoupled from domain-specific assumptions

---

## Core Insight: Hierarchy as Graph Metadata

**Key principle**: Hierarchy is *metadata* on the dependency graph, not separate structure.

The dependency graph already models relationships:
```
Milestone v1.0
  ├── Epic: Auth
  │   ├── Task: JWT implementation
  │   └── Task: OAuth integration
  └── Epic: Billing
      ├── Task: Stripe integration
      └── Task: Invoice generation
```

This IS a hierarchy. We just need to:
1. **Label nodes** with their hierarchy level
2. **Query/filter** by level
3. **Aggregate** metrics bottom-up

---

## Proposed Model

### 1. Hierarchy Level Definition

Add to `.jit/label-namespaces.json`:

```json
{
  "schema_version": 2,
  "hierarchy": {
    "enabled": true,
    "levels": [
      {
        "name": "portfolio",
        "order": 0,
        "description": "Multi-quarter strategic initiatives",
        "type_label": "type:portfolio",
        "membership_namespace": "portfolio"
      },
      {
        "name": "milestone",
        "order": 1,
        "description": "Release goals and time-bounded objectives",
        "type_label": "type:milestone",
        "membership_namespace": "milestone"
      },
      {
        "name": "epic",
        "order": 2,
        "description": "Large features spanning multiple tasks",
        "type_label": "type:epic",
        "membership_namespace": "epic"
      },
      {
        "name": "story",
        "order": 3,
        "description": "User-facing functionality",
        "type_label": "type:story",
        "membership_namespace": null
      },
      {
        "name": "task",
        "order": 4,
        "description": "Atomic implementation work",
        "type_label": "type:task",
        "membership_namespace": null
      }
    ]
  },
  "namespaces": {
    "type": {
      "description": "Issue type defining hierarchy level",
      "unique": true,
      "strategic": false
    },
    "portfolio": {
      "description": "Portfolio membership",
      "unique": false,
      "strategic": true
    },
    "milestone": {
      "description": "Milestone membership",
      "unique": false,
      "strategic": true
    },
    "epic": {
      "description": "Epic membership",
      "unique": false,
      "strategic": false
    }
  }
}
```

### 2. Key Design Elements

#### Order Field
- `order: 0` = highest level (portfolio)
- `order: 4` = lowest level (task)
- Defines parent-child relationships
- Enables validation (child must have higher order than parent)

#### Type Label
- Maps hierarchy level to `type:*` label
- Example: level "epic" → `type:epic`
- Enables querying by level

#### Membership Namespace
- Optional namespace for grouping (e.g., `epic:auth`)
- `null` if level doesn't create groups
- Enables "this task belongs to epic X"

#### Strategic Flag
- Marks namespace as strategic (appears in strategic views)
- Decoupled from hierarchy order
- Example: `portfolio:*` and `milestone:*` are strategic, but `epic:*` is not

---

## API Design

### Domain Model

```rust
/// Hierarchy level configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyLevel {
    /// Name of this level (e.g., "epic", "milestone")
    pub name: String,
    
    /// Order in hierarchy (0 = highest, increasing = lower)
    pub order: u32,
    
    /// Human-readable description
    pub description: String,
    
    /// Type label that identifies this level (e.g., "type:epic")
    pub type_label: String,
    
    /// Optional namespace for membership labels (e.g., "epic" for "epic:auth")
    pub membership_namespace: Option<String>,
}

/// Hierarchy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyConfig {
    /// Whether hierarchy is enabled
    pub enabled: bool,
    
    /// Ordered list of hierarchy levels (0 = highest)
    pub levels: Vec<HierarchyLevel>,
}

impl HierarchyConfig {
    /// Get level by type label
    pub fn get_level_by_type(&self, type_label: &str) -> Option<&HierarchyLevel> {
        self.levels.iter().find(|l| l.type_label == type_label)
    }
    
    /// Get level by order
    pub fn get_level_by_order(&self, order: u32) -> Option<&HierarchyLevel> {
        self.levels.iter().find(|l| l.order == order)
    }
    
    /// Get all strategic namespaces (from membership_namespace field)
    pub fn strategic_namespaces(&self) -> Vec<String> {
        self.levels.iter()
            .filter_map(|l| l.membership_namespace.as_ref())
            .cloned()
            .collect()
    }
    
    /// Check if child can depend on parent based on hierarchy
    pub fn validate_dependency(&self, child_type: &str, parent_type: &str) -> Result<()> {
        let child_level = self.get_level_by_type(child_type)
            .ok_or_else(|| anyhow!("Unknown type: {}", child_type))?;
        let parent_level = self.get_level_by_type(parent_type)
            .ok_or_else(|| anyhow!("Unknown type: {}", parent_type))?;
        
        // Parent must have lower or equal order (higher in hierarchy)
        if parent_level.order > child_level.order {
            return Err(anyhow!(
                "Invalid dependency: {} (order {}) cannot depend on {} (order {}). \
                 Dependencies must point to same or higher levels.",
                child_type, child_level.order, parent_type, parent_level.order
            ));
        }
        
        Ok(())
    }
}
```

### Storage Extension

```rust
// In Storage trait
pub trait IssueStore {
    // ... existing methods ...
    
    /// Load hierarchy configuration
    fn load_hierarchy_config(&self) -> Result<HierarchyConfig>;
    
    /// Save hierarchy configuration
    fn save_hierarchy_config(&self, config: &HierarchyConfig) -> Result<()>;
}
```

### Query API

```rust
// In CommandExecutor
impl<S: IssueStore> CommandExecutor<S> {
    /// Query issues at specific hierarchy level
    pub fn query_by_level(&self, level_name: &str) -> Result<Vec<Issue>> {
        let config = self.storage.load_hierarchy_config()?;
        let level = config.levels.iter()
            .find(|l| l.name == level_name)
            .ok_or_else(|| anyhow!("Unknown level: {}", level_name))?;
        
        let issues = self.storage.list_issues()?;
        let filtered = issues.into_iter()
            .filter(|issue| {
                issue.labels.iter().any(|label| label == &level.type_label)
            })
            .collect();
        
        Ok(filtered)
    }
    
    /// Query issues at or above specific level (strategic view)
    pub fn query_above_level(&self, min_order: u32) -> Result<Vec<Issue>> {
        let config = self.storage.load_hierarchy_config()?;
        let issues = self.storage.list_issues()?;
        
        let filtered = issues.into_iter()
            .filter(|issue| {
                issue.labels.iter().any(|label| {
                    if let Some(level) = config.get_level_by_type(label) {
                        level.order <= min_order
                    } else {
                        false
                    }
                })
            })
            .collect();
        
        Ok(filtered)
    }
    
    /// Query by membership namespace (replaces query_strategic)
    pub fn query_by_membership(&self, namespace: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let filtered = issues.into_iter()
            .filter(|issue| {
                issue.labels.iter().any(|label| {
                    label.starts_with(&format!("{}:", namespace))
                })
            })
            .collect();
        
        Ok(filtered)
    }
}
```

---

## CLI Design

### Configuration Commands

```bash
# Show current hierarchy configuration
jit hierarchy show
# Output:
# Hierarchy enabled: true
# Levels (0=highest):
#   0. portfolio  - Multi-quarter strategic initiatives
#   1. milestone  - Release goals (strategic)
#   2. epic       - Large features (strategic)
#   3. story      - User-facing functionality
#   4. task       - Atomic implementation work

# Add new hierarchy level
jit hierarchy add-level program \
  --order 1 \
  --description "Program-level initiatives" \
  --type-label "type:program" \
  --membership-namespace "program" \
  --strategic

# Remove hierarchy level (only if no issues use it)
jit hierarchy remove-level portfolio

# Reorder levels (shifts order numbers)
jit hierarchy reorder milestone --order 2

# Disable hierarchy (fall back to flat label system)
jit hierarchy disable
```

### Query Commands

```bash
# Query by level name
jit query level milestone
# Shows all issues with type:milestone

# Query strategic view (at or above order N)
jit query strategic --min-order 2
# Shows portfolio, milestone, epic (order 0, 1, 2)

# Query by membership
jit query membership epic:auth
# Shows all issues with epic:auth label

# Combined query
jit query level epic --membership milestone:v1.0
# Shows all epics in milestone v1.0
```

### Validation

```bash
# Validate hierarchy relationships
jit validate --hierarchy
# Checks:
# - All issues have valid type:* label
# - Type labels match configured levels
# - Dependencies respect hierarchy order
# - Membership namespaces exist in config

# Example error:
# Issue 01ABC (type:task) depends on 02DEF (type:epic)
#   ✓ Valid: task (order 4) can depend on epic (order 2)
# Issue 03GHI (type:milestone) depends on 04JKL (type:task)
#   ✗ Invalid: milestone (order 1) cannot depend on task (order 4)
#   Milestones can only depend on: portfolio, milestone
```

---

## Migration Strategy

### Phase 1: Add Hierarchy Config (Backward Compatible)

1. Extend schema to include `hierarchy` field
2. Default to disabled if not present
3. Existing repos work unchanged

```rust
// In load_hierarchy_config()
pub fn load_hierarchy_config(&self) -> Result<HierarchyConfig> {
    match self.read_namespaces_file() {
        Ok(data) => {
            if let Some(hierarchy) = data.get("hierarchy") {
                serde_json::from_value(hierarchy.clone())
            } else {
                // Legacy: no hierarchy config, return disabled
                Ok(HierarchyConfig::disabled())
            }
        }
        Err(_) => Ok(HierarchyConfig::disabled())
    }
}
```

### Phase 2: Migrate `query_strategic()`

Replace hardcoded logic with registry-driven:

```rust
// OLD: Hardcoded
pub fn query_strategic(&self) -> Result<Vec<Issue>> {
    let namespaces = self.storage.load_label_namespaces()?;
    let strategic_namespaces: Vec<String> = namespaces
        .namespaces.iter()
        .filter(|(_, ns)| ns.strategic)
        .map(|(name, _)| name.clone())
        .collect();
    // ... filter by strategic_namespaces
}

// NEW: Registry-driven
pub fn query_strategic(&self) -> Result<Vec<Issue>> {
    let config = self.storage.load_hierarchy_config()?;
    
    if !config.enabled {
        // Fall back to legacy strategic namespace approach
        return self.query_strategic_legacy();
    }
    
    // Use hierarchy config to determine strategic levels
    let namespaces = self.storage.load_label_namespaces()?;
    let strategic_namespaces: Vec<String> = namespaces
        .namespaces.iter()
        .filter(|(_, ns)| ns.strategic)
        .map(|(name, _)| name.clone())
        .collect();
    
    let issues = self.storage.list_issues()?;
    let filtered = issues.into_iter()
        .filter(|issue| {
            issue.labels.iter().any(|label| {
                strategic_namespaces.iter().any(|ns| {
                    label.starts_with(&format!("{}:", ns))
                })
            })
        })
        .collect();
    
    Ok(filtered)
}
```

### Phase 3: Update Web UI

```typescript
// API: GET /api/hierarchy
{
  "enabled": true,
  "levels": [
    { "name": "milestone", "order": 1, "type_label": "type:milestone" },
    { "name": "epic", "order": 2, "type_label": "type:epic" },
    { "name": "task", "order": 4, "type_label": "type:task" }
  ]
}

// Frontend: Dynamic view selector
<select onChange={handleLevelChange}>
  {hierarchy.levels.map(level => (
    <option value={level.order}>{level.name}</option>
  ))}
  <option value="all">All Levels</option>
</select>
```

---

## Benefits

### 1. Fully Generic
- No hardcoded level names
- Supports 2, 3, 4+ level hierarchies
- Users define their own taxonomy

### 2. Backward Compatible
- Disabled by default
- Existing repos work unchanged
- Gradual migration path

### 3. Validation Power
- Enforce hierarchy order in dependencies
- Prevent nonsensical relationships (milestone depends on task)
- Clear error messages

### 4. Query Flexibility
```bash
# Traditional 3-level
jit query level milestone
jit query level epic
jit query level task

# Custom 5-level
jit query level portfolio
jit query level program
jit query level epic
jit query level story
jit query level task

# Strategic filtering
jit query strategic --min-order 2  # Show top 3 levels
jit query strategic --max-order 3  # Show bottom 2 levels
```

### 5. Decoupled Design
- Hierarchy config separate from label validation
- No assumptions in validation code
- Easy to extend with new features

---

## Implementation Plan

### Phase 1: Domain Model (2 hours)
- [ ] Add `HierarchyLevel` and `HierarchyConfig` structs
- [ ] Add hierarchy field to label-namespaces schema
- [ ] Implement helper methods (get_level_by_type, validate_dependency)
- [ ] Add unit tests

### Phase 2: Storage Layer (1 hour)
- [ ] Add load/save methods to IssueStore trait
- [ ] Implement in JsonFileStorage and InMemoryStorage
- [ ] Handle missing config gracefully (disabled by default)
- [ ] Add storage tests

### Phase 3: Query API (2 hours)
- [ ] Implement query_by_level()
- [ ] Implement query_above_level()
- [ ] Migrate query_strategic() to use hierarchy config
- [ ] Add comprehensive query tests

### Phase 4: CLI Commands (2 hours)
- [ ] Add `jit hierarchy` command group
- [ ] Implement show, add-level, remove-level subcommands
- [ ] Update `jit query` to support level/strategic flags
- [ ] Add CLI integration tests

### Phase 5: Validation (1 hour)
- [ ] Add hierarchy validation to `jit validate`
- [ ] Check dependency order constraints
- [ ] Verify type labels match configured levels
- [ ] Add validation tests

### Phase 6: Documentation (1 hour)
- [ ] Update label-conventions.md
- [ ] Add hierarchy configuration guide
- [ ] Provide migration examples
- [ ] Document query patterns

**Total Estimated Time**: 9 hours

---

## Example Configurations

### Minimal (2 levels)
```json
{
  "hierarchy": {
    "enabled": true,
    "levels": [
      { "name": "epic", "order": 0, "type_label": "type:epic", "membership_namespace": "epic" },
      { "name": "task", "order": 1, "type_label": "type:task", "membership_namespace": null }
    ]
  }
}
```

### Standard (3 levels - Scrum)
```json
{
  "hierarchy": {
    "enabled": true,
    "levels": [
      { "name": "milestone", "order": 0, "type_label": "type:milestone", "membership_namespace": "milestone" },
      { "name": "epic", "order": 1, "type_label": "type:epic", "membership_namespace": "epic" },
      { "name": "story", "order": 2, "type_label": "type:story", "membership_namespace": null }
    ]
  }
}
```

### Enterprise (5 levels - SAFe)
```json
{
  "hierarchy": {
    "enabled": true,
    "levels": [
      { "name": "portfolio", "order": 0, "type_label": "type:portfolio", "membership_namespace": "portfolio" },
      { "name": "program", "order": 1, "type_label": "type:program", "membership_namespace": "program" },
      { "name": "epic", "order": 2, "type_label": "type:epic", "membership_namespace": "epic" },
      { "name": "feature", "order": 3, "type_label": "type:feature", "membership_namespace": null },
      { "name": "story", "order": 4, "type_label": "type:story", "membership_namespace": null }
    ]
  }
}
```

### Research Team (Custom)
```json
{
  "hierarchy": {
    "enabled": true,
    "levels": [
      { "name": "initiative", "order": 0, "type_label": "type:initiative", "membership_namespace": "initiative" },
      { "name": "experiment", "order": 1, "type_label": "type:experiment", "membership_namespace": "experiment" },
      { "name": "analysis", "order": 2, "type_label": "type:analysis", "membership_namespace": null }
    ]
  }
}
```

---

## Open Questions

1. **Should hierarchy order be enforced in dependencies?**
   - Option A: Hard error (milestone cannot depend on task)
   - Option B: Warning only
   - **Recommendation**: Hard error - hierarchy violations are almost always mistakes

2. **Can an issue belong to multiple membership groups?**
   - Example: `epic:auth` + `epic:billing`
   - Current: Yes (labels are not unique)
   - **Recommendation**: Keep flexible, validate per use case

3. **Should breakdown inherit hierarchy level?**
   - Example: Breaking down epic creates epics or tasks?
   - **Recommendation**: Inherit parent's level-1, configurable via flag

4. **Migration path for existing issues?**
   - Command: `jit hierarchy migrate --auto-infer`
   - Use graph structure to infer levels (root = milestone, leaf = task)
   - **Recommendation**: Manual migration with validation

---

## Summary

This proposal provides:
1. ✅ **Generic model** - No hardcoded assumptions about level names
2. ✅ **Extensible** - Supports 2-10+ levels easily
3. ✅ **Backward compatible** - Disabled by default, gradual adoption
4. ✅ **Validation** - Enforces sensible hierarchy constraints
5. ✅ **Query power** - Flexible filtering by level, order, membership
6. ✅ **Decoupled** - Hierarchy separate from label validation logic

The key insight: **hierarchy is graph metadata, not separate structure**. We already have the DAG - just need to annotate nodes with their level and provide tools to query/filter/validate based on hierarchy rules.
