# Issue Type Hierarchy Enforcement Proposal

**Date**: 2025-12-14  
**Status**: Proposal  
**Goal**: Flexible, configurable type enforcement with customizable hierarchy levels

---

## Overview

This proposal extends the label enforcement system to include **issue type hierarchy validation** while keeping it fairly loose and optional. The system validates that type hierarchies are logically consistent based on **configurable hierarchy levels** that can be extended in either direction.

---

## Core Principles

1. **Optional but Helpful**: Validation warns rather than blocks (except for critical violations)
2. **Hierarchy-Aware**: Understands configurable hierarchy levels
3. **Agent-Friendly**: Clear error messages guide agents to correct usage
4. **Gradual Adoption**: Can be enabled incrementally per repository
5. **Functionally Pure**: Validation is side-effect-free and testable
6. **Fully Configurable**: Type names, hierarchy levels, and default type are all configurable

---

## Type Hierarchy Model

### Hierarchy Levels (Configurable)

The system supports **arbitrary hierarchy levels** that can be extended upward (broader containers) or downward (more granular items):

```
Level 4: program, portfolio        (strategic planning - optional)
           ↓
Level 3: milestone, release         (time-bounded goals)
           ↓
Level 2: epic, theme, initiative    (feature groupings)
           ↓
Level 1: task, story, bug, feature  (work items - DEFAULT)
           ↓
Level 0: subtask, spike             (granular decomposition - optional)
```

### Default Configuration (3 Levels)

Out of the box, the system uses a simple 3-level hierarchy:

```
milestone               (level 3: top-level container)
  ├── epic             (level 2: mid-level grouping)
  │   └── task         (level 1: work items - DEFAULT)
  ├── epic
  │   ├── task
  │   └── task
  └── task             (can belong directly to milestone)
```

### Hierarchy Rules

**Core Constraint**: An issue at level N can only depend on issues at level N or higher (N+1, N+2, etc.)

| Relationship | Valid? | Example |
|-------------|--------|---------|
| Same level → same level | ✅ Yes | task → task, epic → epic |
| Lower level → higher level | ✅ Yes | task → epic, epic → milestone |
| Higher level → lower level | ❌ No | epic → task, milestone → epic |
| Higher level → same level | ❌ No | milestone → milestone |

### Key Constraints

1. **Hierarchy flows upward**: Lower-level issues can depend on higher-level issues (broader containers)
2. **No reverse dependencies**: Higher-level issues cannot depend on lower-level issues
3. **Peer dependencies allowed**: Issues at the same level can depend on each other
4. **Flexible expansion**: New levels can be added above or below existing levels
5. **Configurable defaults**: Default type (e.g., "task") is configurable per repository

---

## Validation Levels

### LEVEL 1: ERROR (Hard Constraint)

**1.1 Type Label Required**
```rust
// Every issue MUST have exactly ONE type:* label
validate_required_type_label(labels) -> Result<(), Error>
```

**Example violation:**
```bash
$ jit issue create --title "Login API" --label "epic:auth"
Error: Issue must have a type label.
Valid types: type:task, type:epic, type:milestone, type:bug, type:feature, type:research
Example: --label "type:task"
```

**1.2 Hierarchy Violation: Milestone under Epic**
```rust
// A milestone cannot be a dependency of an epic
validate_hierarchy_constraints(issue, dependencies) -> Result<(), Error>
```

**Example violation:**
```bash
$ jit dep add <milestone_id> <epic_id>
Error: Hierarchy violation: Milestone 'v1.0' cannot belong to epic 'auth'.
Reason: Milestones are broader containers than epics.
Hint: Consider making the epic a dependency of the milestone instead:
  jit dep add <epic_id> <milestone_id>
```

### LEVEL 2: WARNING (Soft Constraint)

**2.1 Epic without Epic Label**
```rust
// Issues with type:epic SHOULD have an epic:* label
validate_strategic_consistency(issue) -> Result<(), Warning>
```

**Example warning:**
```bash
$ jit issue create --title "Auth System" --label "type:epic"
Warning: Epic issues should have an epic:* label for group identification.
Suggestion: --label "epic:auth"
Continue without epic label? [y/N]
Bypass: --force flag or --yes for non-interactive
```

**2.2 Milestone without Milestone Label**
```bash
$ jit issue create --title "v1.0 Release" --label "type:milestone"
Warning: Milestone issues should have a milestone:* label.
Suggestion: --label "milestone:v1.0"
Continue? [y/N]
```

**2.3 Orphaned Task**
```rust
// Tasks without epic:* or milestone:* labels are orphaned
validate_task_membership(issue, graph) -> Result<(), Warning>
```

**Example warning:**
```bash
$ jit issue create --title "Fix bug" --label "type:task"
Warning: Task has no epic or milestone association.
Consider adding: --label "epic:xyz" or --label "milestone:v1.0"
Continue? [y/N]
Bypass: --orphan flag to acknowledge intentional orphan
```

**2.4 Epic Nesting Attempt**
```bash
$ jit dep add <epic1_id> <epic2_id>
Warning: Creating dependency epic -> epic may indicate nesting attempt.
Epics should be peers, not nested. Consider splitting into smaller epics.
Continue? [y/N]
```

### LEVEL 3: INFO (Informational)

**3.1 Deep Dependency Chains**
```rust
// Log when dependency chains exceed reasonable depth
check_dependency_depth(issue, graph) -> Option<Info>
```

**Example info:**
```bash
$ jit dep add <task_id> <epic_id>
Info: This creates a 4-level dependency chain: milestone -> epic -> task -> subtask
Consider flattening the hierarchy if this becomes hard to manage.
```

**3.2 Circular Type References**
```bash
$ jit issue update <epic_id> --label "milestone:v1.0"
$ jit issue update <milestone_id> --label "epic:auth"
Info: Issue 01ABC (epic:auth) belongs to milestone:v1.0, which belongs to epic:auth
This creates a circular relationship in the label hierarchy.
```

---

## Configuration Format

### Repository Configuration

```toml
# .jit/config.toml

[type_hierarchy]
# Default issue type (automatically added if no type:* label provided)
default_type = "task"

# Hierarchy level definitions (lower numbers = more granular, higher = broader)
# Each level can contain multiple type names
[type_hierarchy.levels]
0 = ["subtask", "spike"]                        # Optional: Granular decomposition
1 = ["task", "story", "bug", "feature"]         # DEFAULT: Work items
2 = ["epic", "theme", "initiative"]             # Feature groupings
3 = ["milestone", "release", "version"]         # Time-bounded goals
4 = ["program", "portfolio"]                    # Optional: Strategic planning

# Strategic types: These types appear in strategic view
[type_hierarchy.strategic]
types = ["epic", "theme", "milestone", "release", "program"]

# Type aliases and labels
[type_hierarchy.label_associations]
# When a type:epic is created, suggest adding epic:* label
epic = "epic"
milestone = "milestone"
release = "milestone"  # Alias: type:release issues also get milestone:* suggestion

[validation]
# Validation strictness level
# - "strict": All validations are errors
# - "loose": Hierarchy checks are warnings, only format errors block
# - "permissive": All validations are warnings
strictness = "loose"

# Require type labels on all issues
require_type_labels = true

# Enforce hierarchy constraints (higher level ↛ lower level)
enforce_hierarchy = true

# Warn on orphaned leaf-level issues (no parent association)
warn_orphaned_leaves = true

# Warn on strategic issues without matching labels
warn_strategic_consistency = true
```

### Default Configuration (3 Levels)

When no config exists, use these defaults:

```rust
pub fn default_hierarchy_config() -> HierarchyConfig {
    HierarchyConfig {
        default_type: "task".into(),
        levels: vec![
            (1, vec!["task".into(), "bug".into(), "feature".into(), "research".into()]),
            (2, vec!["epic".into()]),
            (3, vec!["milestone".into()]),
        ].into_iter().collect(),
        strategic_types: vec!["epic".into(), "milestone".into()],
        label_associations: vec![
            ("epic".into(), "epic".into()),
            ("milestone".into(), "milestone".into()),
        ].into_iter().collect(),
    }
}
```

---

## Implementation Design

### Core Validation Module

```rust
// crates/jit/src/type_hierarchy.rs

use std::collections::HashMap;
use anyhow::{anyhow, Result};

/// Hierarchy configuration loaded from .jit/config.toml
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchyConfig {
    /// Default type name (e.g., "task")
    pub default_type: String,
    
    /// Map of hierarchy level to type names at that level
    /// Level 1 = default work items, higher = broader containers
    pub levels: HashMap<u8, Vec<String>>,
    
    /// Types that appear in strategic view
    pub strategic_types: Vec<String>,
    
    /// Map from type name to suggested label namespace
    /// e.g., "epic" -> "epic" means suggest "epic:*" label for type:epic
    pub label_associations: HashMap<String, String>,
}

impl HierarchyConfig {
    /// Get the hierarchy level for a type name
    pub fn get_level(&self, type_name: &str) -> Option<u8> {
        self.levels.iter()
            .find(|(_, types)| types.contains(&type_name.to_string()))
            .map(|(level, _)| *level)
    }
    
    /// Check if a type is strategic (appears in strategic view)
    pub fn is_strategic(&self, type_name: &str) -> bool {
        self.strategic_types.contains(&type_name.to_string())
    }
    
    /// Get suggested label namespace for a type
    pub fn get_label_association(&self, type_name: &str) -> Option<&str> {
        self.label_associations.get(type_name).map(String::as_str)
    }
    
    /// Check if type1 can depend on type2 (hierarchy constraint)
    /// Rule: Lower-level issues can depend on same or higher-level issues
    pub fn can_depend_on(&self, child_type: &str, parent_type: &str) -> bool {
        let child_level = self.get_level(child_type);
        let parent_level = self.get_level(parent_type);
        
        match (child_level, parent_level) {
            (Some(child), Some(parent)) => {
                // Can depend on same level or higher (parent >= child)
                parent >= child
            }
            // Unknown types: allow by default (permissive)
            _ => true,
        }
    }
    
    /// Get all types at a specific level
    pub fn types_at_level(&self, level: u8) -> Vec<String> {
        self.levels.get(&level).cloned().unwrap_or_default()
    }
    
    /// Get the lowest (most granular) level
    pub fn min_level(&self) -> Option<u8> {
        self.levels.keys().min().copied()
    }
    
    /// Get the highest (broadest) level
    pub fn max_level(&self) -> Option<u8> {
        self.levels.keys().max().copied()
    }
}

/// Extract issue type from labels
pub fn extract_type(labels: &[String]) -> Result<String, ValidationError> {
    let type_labels: Vec<_> = labels.iter()
        .filter(|l| l.starts_with("type:"))
        .collect();
    
    if type_labels.is_empty() {
        return Err(ValidationError::MissingTypeLabel);
    }
    
    if type_labels.len() > 1 {
        return Err(ValidationError::MultipleTypeLabels(
            type_labels.iter().map(|s| s.to_string()).collect()
        ));
    }
    
    let type_name = type_labels[0].strip_prefix("type:").unwrap();
    Ok(type_name.to_string())
}

/// Validate type hierarchy constraints
pub fn validate_hierarchy(
    config: &HierarchyConfig,
    child_type: &str,
    parent_type: &str,
) -> Result<(), HierarchyViolation> {
    if !config.can_depend_on(child_type, parent_type) {
        let child_level = config.get_level(child_type).unwrap_or(0);
        let parent_level = config.get_level(parent_type).unwrap_or(0);
        
        return Err(HierarchyViolation {
            child: child_type.to_string(),
            parent: parent_type.to_string(),
            child_level,
            parent_level,
            reason: format!(
                "{} (level {}) cannot depend on {} (level {}). \
                 Hierarchy flows upward: lower-level issues can only depend on same or higher-level issues.",
                child_type, child_level, parent_type, parent_level
            ),
        });
    }
    Ok(())
}

/// Validate strategic label consistency (warnings)
pub fn validate_strategic_labels(
    config: &HierarchyConfig,
    type_name: &str,
    labels: &[String],
) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();
    
    // Check if this is a strategic type
    if !config.is_strategic(type_name) {
        return warnings;
    }
    
    // Check if it has the associated label
    if let Some(expected_namespace) = config.get_label_association(type_name) {
        let has_label = labels.iter().any(|l| l.starts_with(&format!("{}:", expected_namespace)));
        if !has_label {
            warnings.push(ValidationWarning::MissingStrategicLabel {
                type_name: type_name.to_string(),
                expected_namespace: expected_namespace.to_string(),
            });
        }
    }
    
    warnings
}

/// Check if a type is at the lowest (leaf) level
pub fn is_leaf_type(config: &HierarchyConfig, type_name: &str) -> bool {
    let type_level = config.get_level(type_name);
    let min_level = config.min_level();
    
    match (type_level, min_level) {
        (Some(level), Some(min)) => level == min,
        _ => false,
    }
}

/// Validate orphaned leaf issues (warnings)
pub fn validate_orphans(
    config: &HierarchyConfig,
    type_name: &str,
    labels: &[String],
) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();
    
    // Only check leaf-level types
    if !is_leaf_type(config, type_name) {
        return warnings;
    }
    
    // Check if it has any parent association labels
    let has_parent_label = labels.iter().any(|l| {
        // Check all strategic namespaces
        config.strategic_types.iter().any(|strategic_type| {
            if let Some(namespace) = config.get_label_association(strategic_type) {
                l.starts_with(&format!("{}:", namespace))
            } else {
                false
            }
        })
    });
    
    if !has_parent_label {
        warnings.push(ValidationWarning::OrphanedLeaf {
            type_name: type_name.to_string(),
        });
    }
    
    warnings
}

#[derive(Debug)]
pub enum ValidationError {
    MissingTypeLabel,
    MultipleTypeLabels(Vec<String>),
    UnknownType(String),
}

#[derive(Debug)]
pub struct HierarchyViolation {
    pub child: String,
    pub parent: String,
    pub child_level: u8,
    pub parent_level: u8,
    pub reason: String,
}

#[derive(Debug)]
pub enum ValidationWarning {
    MissingStrategicLabel {
        type_name: String,
        expected_namespace: String,
    },
    OrphanedLeaf {
        type_name: String,
    },
}
```

### Integration Points

**1. Issue Creation with Auto-Default**
```rust
// crates/jit/src/commands.rs

pub fn create_issue(&mut self, params: CreateIssueParams) -> Result<Issue> {
    let config = self.load_hierarchy_config()?;
    
    // Validate labels format
    for label in &params.labels {
        validate_label(label)?;
    }
    
    // Check if type label exists, add default if missing
    let has_type = params.labels.iter().any(|l| l.starts_with("type:"));
    let mut labels = params.labels.clone();
    
    if !has_type {
        let default_type = format!("type:{}", config.default_type);
        labels.push(default_type);
        info!("Auto-added default type label: type:{}", config.default_type);
    }
    
    // Extract and validate type
    let type_name = extract_type(&labels)
        .map_err(|e| format_type_error(e))?;
    
    // Validate type is known in hierarchy
    if config.get_level(&type_name).is_none() {
        return Err(anyhow!(
            "Unknown type '{}'. Valid types: {}",
            type_name,
            format_valid_types(&config)
        ));
    }
    
    // Check strategic consistency (warnings only)
    let warnings = validate_strategic_labels(&config, &type_name, &labels);
    if !warnings.is_empty() && !params.force {
        return Err(anyhow!(
            "Validation warnings: {}\nUse --force to bypass",
            format_warnings(&warnings)
        ));
    }
    
    // Check orphaned leaves (warnings only)
    let orphan_warnings = validate_orphans(&config, &type_name, &labels);
    if !orphan_warnings.is_empty() && !params.force && !params.allow_orphan {
        return Err(anyhow!(
            "Warning: {}\nUse --force or --orphan to proceed",
            format_warnings(&orphan_warnings)
        ));
    }
    
    // Create issue
    let issue = Issue::new(params.title, params.priority, labels);
    self.storage.save_issue(&issue)?;
    
    Ok(issue)
}
```

**2. Dependency Addition with Hierarchy Validation**
```rust
// crates/jit/src/commands.rs

pub fn add_dependency(&mut self, issue_id: &str, dep_id: &str) -> Result<()> {
    let config = self.load_hierarchy_config()?;
    let issue = self.storage.load_issue(issue_id)?;
    let dep = self.storage.load_issue(dep_id)?;
    
    // Extract types
    let issue_type = extract_type(&issue.labels)?;
    let dep_type = extract_type(&dep.labels)?;
    
    // Validate hierarchy (hard constraint)
    if let Err(violation) = validate_hierarchy(&config, &issue_type, &dep_type) {
        let issue_level = config.get_level(&issue_type).unwrap_or(0);
        let dep_level = config.get_level(&dep_type).unwrap_or(0);
        
        return Err(anyhow!(
            "Hierarchy violation: {}\n\
             {} (level {}) cannot depend on {} (level {})\n\
             Hint: Dependencies flow upward. Did you mean: jit dep add {} {}?",
            violation.reason,
            issue_type, issue_level,
            dep_type, dep_level,
            dep_id, issue_id
        ));
    }
    
    // Check cycle
    if self.graph.would_create_cycle(issue_id, dep_id)? {
        return Err(anyhow!("Adding dependency would create a cycle"));
    }
    
    // Add dependency
    self.graph.add_dependency(issue_id, dep_id)?;
    
    Ok(())
}
```

**3. Validation Command with Hierarchy Report**
```rust
// crates/jit/src/commands.rs

pub fn validate_type_hierarchy(&self) -> Result<ValidationReport> {
    let config = self.load_hierarchy_config()?;
    let mut report = ValidationReport::new();
    let issues = self.storage.list_issues()?;
    
    for issue in issues {
        // Check type label presence
        match extract_type(&issue.labels) {
            Ok(type_name) => {
                // Validate type is known in hierarchy
                if config.get_level(&type_name).is_none() {
                    report.add_error(&issue.id, ValidationError::UnknownType(type_name.clone()));
                    continue;
                }
                
                // Check strategic consistency
                let warnings = validate_strategic_labels(&config, &type_name, &issue.labels);
                report.add_warnings(&issue.id, warnings);
                
                // Check orphaned leaves
                let orphan_warnings = validate_orphans(&config, &type_name, &issue.labels);
                report.add_warnings(&issue.id, orphan_warnings);
                
                // Check hierarchy constraints for dependencies
                for dep_id in &issue.dependencies {
                    if let Ok(dep) = self.storage.load_issue(dep_id) {
                        if let Ok(dep_type) = extract_type(&dep.labels) {
                            if let Err(violation) = validate_hierarchy(&config, &type_name, &dep_type) {
                                report.add_hierarchy_violation(&issue.id, dep_id, violation);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                report.add_error(&issue.id, e);
            }
        }
    }
    
    Ok(report)
}

/// Load hierarchy configuration from .jit/config.toml or use defaults
fn load_hierarchy_config(&self) -> Result<HierarchyConfig> {
    let config_path = self.storage.data_dir().join("config.toml");
    
    if config_path.exists() {
        let config_str = std::fs::read_to_string(&config_path)?;
        let config: ConfigFile = toml::from_str(&config_str)?;
        Ok(config.type_hierarchy.unwrap_or_else(default_hierarchy_config))
    } else {
        Ok(default_hierarchy_config())
    }
}
```

---

## CLI Interface

### Commands

**Create with Auto-Default Type**
```bash
# No type label: auto-adds default (type:task)
$ jit issue create --title "Fix login bug" --label "epic:auth"
Info: Auto-added default type label: type:task
Created issue: 01ABC

# Explicit type: no auto-add
$ jit issue create \
    --title "User Authentication Epic" \
    --label "type:epic" \
    --label "epic:auth" \
    --label "milestone:v1.0"
Created issue: 02DEF

# Warning: Strategic type without matching label
$ jit issue create \
    --title "Authentication" \
    --label "type:epic"
Warning: Epic issues should have an epic:* label for group identification.
Suggestion: --label "epic:auth"
Continue without epic label? [y/N] n
Aborted. Retry with: --label "epic:auth" or use --force

# Bypass warning
$ jit issue create \
    --title "Authentication" \
    --label "type:epic" \
    --force
Created issue: 03GHI

# Error: Hierarchy violation (milestone depending on epic)
$ jit dep add <milestone_id> <epic_id>
Error: Hierarchy violation: milestone (level 3) cannot depend on epic (level 2)
Reason: Hierarchy flows upward. Lower-level issues depend on higher-level issues.
Hint: Did you mean: jit dep add <epic_id> <milestone_id>?

# Success: Task depending on epic (upward flow)
$ jit dep add <task_id> <epic_id>
✓ Added dependency: task → epic

# Custom type at extended level
$ jit issue create \
    --title "Q1 2026 Portfolio" \
    --label "type:portfolio"
Created issue: 04JKL (level 4)
```

**Validation Report**
```bash
$ jit validate --type-hierarchy
Validating type hierarchy...

Configuration:
  Hierarchy levels: 3
    Level 1: task, bug, feature, research (DEFAULT: task)
    Level 2: epic
    Level 3: milestone
  Strategic types: epic, milestone

Errors (2):
  Issue 01ABC: Unknown type 'subtask'
    Fix: Update type to one of: task, bug, feature, research, epic, milestone
    Command: jit issue update 01ABC --replace-label "type:task"
  
  Issue 02DEF → 03GHI: Hierarchy violation
    milestone (level 3) cannot depend on epic (level 2)
    Fix: Remove invalid dependency
    Command: jit dep rm 02DEF 03GHI

Warnings (3):
  Issue 04JKL: Epic without epic:* label
    Suggestion: jit issue update 04JKL --label "epic:payments"
  
  Issue 05MNO: Orphaned task (no parent labels)
    Consider: jit issue update 05MNO --label "epic:xyz"
    Or create with: --orphan flag
  
  Issue 06PQR: Milestone without milestone:* label
    Suggestion: jit issue update 06PQR --label "milestone:v1.0"

Summary: 2 errors, 3 warnings
Run with --fix to auto-repair errors
Run with --fix-warnings to apply suggestions
```

**Auto-Fix with Smart Suggestions**
```bash
$ jit validate --type-hierarchy --fix
Fixing type hierarchy issues...

Issue 01ABC: Unknown type 'subtask'
  Suggestion: Change to 'task' (closest match)
  Apply? [Y/n] y
  ✓ Updated type:subtask → type:task

Issue 02DEF → 03GHI: Hierarchy violation
  milestone → epic (reverse flow)
  Suggestion: Reverse dependency (epic → milestone)
  Apply? [Y/n] y
  ✓ Removed dependency 02DEF → 03GHI
  ✓ Added dependency 03GHI → 02DEF

Fixed 2 errors

Warnings remain (use --fix-warnings to address)
```

**Show Hierarchy Configuration**
```bash
$ jit config show-hierarchy
Type Hierarchy Configuration:

Default type: task
Strictness: loose

Hierarchy Levels:
  Level 1 (work items):
    - task, bug, feature, research
  Level 2 (groupings):
    - epic
  Level 3 (containers):
    - milestone

Strategic types (appear in strategic view):
  - epic (suggests epic:* label)
  - milestone (suggests milestone:* label)

Validation rules:
  ✓ Type labels required
  ✓ Hierarchy constraints enforced
  ✓ Orphaned leaves: warning
  ✓ Strategic consistency: warning
```

**Initialize Custom Hierarchy**
```bash
$ jit init --hierarchy-template extended
Created .jit/config.toml with extended hierarchy:
  Level 0: subtask, spike
  Level 1: task, story, bug, feature
  Level 2: epic, theme
  Level 3: milestone, release
  Level 4: program, portfolio
```

---

## Configuration

### Repository Settings

```toml
# .jit/config.toml

[type_hierarchy]
# Default issue type (automatically added if no type:* label provided)
default_type = "task"

# Hierarchy level definitions (lower numbers = more granular, higher = broader)
# Each level can contain multiple type names
[type_hierarchy.levels]
0 = ["subtask", "spike"]                        # Optional: Granular decomposition
1 = ["task", "story", "bug", "feature"]         # DEFAULT: Work items
2 = ["epic", "theme", "initiative"]             # Feature groupings
3 = ["milestone", "release", "version"]         # Time-bounded goals
4 = ["program", "portfolio"]                    # Optional: Strategic planning

# Strategic types: These types appear in strategic view
[type_hierarchy.strategic]
types = ["epic", "theme", "milestone", "release", "program"]

# Type aliases and labels
[type_hierarchy.label_associations]
# When a type:epic is created, suggest adding epic:* label
epic = "epic"
milestone = "milestone"
release = "milestone"  # Alias: type:release issues also get milestone:* suggestion

[validation]
# Validation strictness level
# - "strict": All validations are errors
# - "loose": Hierarchy checks are warnings, only format errors block
# - "permissive": All validations are warnings
strictness = "loose"

# Require type labels on all issues
require_type_labels = true

# Enforce hierarchy constraints (higher level ↛ lower level)
enforce_hierarchy = true

# Warn on orphaned leaf-level issues (no parent association)
warn_orphaned_leaves = true

# Warn on strategic issues without matching labels
warn_strategic_consistency = true
```

### Default Configuration (3 Levels)

When no config exists, use these defaults:

```rust
pub fn default_hierarchy_config() -> HierarchyConfig {
    HierarchyConfig {
        default_type: "task".into(),
        levels: vec![
            (1, vec!["task".into(), "bug".into(), "feature".into(), "research".into()]),
            (2, vec!["epic".into()]),
            (3, vec!["milestone".into()]),
        ].into_iter().collect(),
        strategic_types: vec!["epic".into(), "milestone".into()],
        label_associations: vec![
            ("epic".into(), "epic".into()),
            ("milestone".into(), "milestone".into()),
        ].into_iter().collect(),
    }
}
```

### Hierarchy Templates

**Template: default (3 levels)**
```toml
[type_hierarchy]
default_type = "task"
[type_hierarchy.levels]
1 = ["task", "bug", "feature", "research"]
2 = ["epic"]
3 = ["milestone"]
```

**Template: extended (5 levels)**
```toml
[type_hierarchy]
default_type = "task"
[type_hierarchy.levels]
0 = ["subtask", "spike"]
1 = ["task", "story", "bug", "feature"]
2 = ["epic", "theme"]
3 = ["milestone", "release"]
4 = ["program", "portfolio"]
```

**Template: agile (4 levels, story-centric)**
```toml
[type_hierarchy]
default_type = "story"
[type_hierarchy.levels]
0 = ["subtask"]
1 = ["story", "bug", "spike"]
2 = ["epic"]
3 = ["release"]
```

**Template: minimal (2 levels)**
```toml
[type_hierarchy]
default_type = "task"
[type_hierarchy.levels]
1 = ["task", "bug"]
2 = ["milestone"]
```

---

## Migration Strategy

### Phase 1: Add Validation Module (2-3 hours)
1. Create `type_hierarchy.rs` with validation logic
2. Add `IssueType` enum and hierarchy rules
3. Implement pure validation functions
4. Write comprehensive unit tests (property-based for hierarchy rules)

### Phase 2: Integrate with Commands (1-2 hours)
1. Update `create_issue` to validate types
2. Update `add_dependency` to check hierarchy
3. Add `--force` flag to bypass warnings
4. Update error messages with suggestions

### Phase 3: Validation Command (1 hour)
1. Implement `validate --type-hierarchy` command
2. Add `ValidationReport` structure
3. Implement auto-fix suggestions
4. Add JSON output support

### Phase 4: Configuration Support (30 min)
1. Add `.jit/config.toml` parsing
2. Implement strictness levels
3. Add configuration validation
4. Document configuration options

### Phase 5: Web UI Integration (1 hour)
1. Add type badges to issue cards
2. Show hierarchy violations in validation panel
3. Add quick-fix buttons for common issues
4. Display warning icons on strategic issues

### Phase 6: Documentation (30 min)
1. Update agent guidelines with type hierarchy rules
2. Add examples to CLI help text
3. Create migration guide for existing repositories
4. Document configuration options

**Total Estimated Effort**: 6-8 hours

---

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_hierarchy_constraint_upward_flow() {
        let config = default_hierarchy_config();
        
        // Task (level 1) can depend on epic (level 2) ✓
        assert!(config.can_depend_on("task", "epic"));
        
        // Task (level 1) can depend on milestone (level 3) ✓
        assert!(config.can_depend_on("task", "milestone"));
        
        // Epic (level 2) cannot depend on task (level 1) ✗
        assert!(!config.can_depend_on("epic", "task"));
        
        // Milestone (level 3) cannot depend on epic (level 2) ✗
        assert!(!config.can_depend_on("milestone", "epic"));
        
        // Task (level 1) can depend on task (level 1) - peer ✓
        assert!(config.can_depend_on("task", "task"));
        
        // Epic (level 2) can depend on epic (level 2) - peer ✓
        assert!(config.can_depend_on("epic", "epic"));
    }
    
    #[test]
    fn test_get_level_for_types() {
        let config = default_hierarchy_config();
        
        assert_eq!(config.get_level("task"), Some(1));
        assert_eq!(config.get_level("epic"), Some(2));
        assert_eq!(config.get_level("milestone"), Some(3));
        assert_eq!(config.get_level("unknown"), None);
    }
    
    #[test]
    fn test_strategic_type_detection() {
        let config = default_hierarchy_config();
        
        assert!(config.is_strategic("epic"));
        assert!(config.is_strategic("milestone"));
        assert!(!config.is_strategic("task"));
        assert!(!config.is_strategic("bug"));
    }
    
    #[test]
    fn test_label_associations() {
        let config = default_hierarchy_config();
        
        assert_eq!(config.get_label_association("epic"), Some("epic"));
        assert_eq!(config.get_label_association("milestone"), Some("milestone"));
        assert_eq!(config.get_label_association("task"), None);
    }
    
    #[test]
    fn test_custom_hierarchy_levels() {
        let mut levels = HashMap::new();
        levels.insert(0, vec!["subtask".into()]);
        levels.insert(1, vec!["task".into(), "bug".into()]);
        levels.insert(2, vec!["epic".into()]);
        levels.insert(3, vec!["milestone".into()]);
        levels.insert(4, vec!["program".into()]);
        
        let config = HierarchyConfig {
            default_type: "task".into(),
            levels,
            strategic_types: vec!["epic".into(), "milestone".into(), "program".into()],
            label_associations: HashMap::new(),
        };
        
        // Subtask (0) can depend on task (1)
        assert!(config.can_depend_on("subtask", "task"));
        
        // Program (4) cannot depend on anything lower
        assert!(!config.can_depend_on("program", "milestone"));
        assert!(!config.can_depend_on("program", "epic"));
        
        // Task (1) can depend on all higher levels
        assert!(config.can_depend_on("task", "epic"));
        assert!(config.can_depend_on("task", "milestone"));
        assert!(config.can_depend_on("task", "program"));
    }
}
```

### Property-Based Tests
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_hierarchy_is_transitive(
        types in prop::collection::vec(
            prop_oneof![
                Just("task"),
                Just("epic"),
                Just("milestone"),
            ],
            3
        )
    ) {
        let config = default_hierarchy_config();
        
        // If A can depend on B, and B can depend on C,
        // then A can depend on C (transitivity)
        if types.len() >= 3 {
            let (a, b, c) = (&types[0], &types[1], &types[2]);
            if config.can_depend_on(a, b) && config.can_depend_on(b, c) {
                assert!(config.can_depend_on(a, c));
            }
        }
    }
    
    #[test]
    fn test_hierarchy_levels_monotonic(
        type1 in "[a-z]{1,10}",
        type2 in "[a-z]{1,10}",
        level1 in 1u8..5,
        level2 in 1u8..5,
    ) {
        let mut levels = HashMap::new();
        levels.insert(level1, vec![type1.clone()]);
        levels.insert(level2, vec![type2.clone()]);
        
        let config = HierarchyConfig {
            default_type: type1.clone(),
            levels,
            strategic_types: vec![],
            label_associations: HashMap::new(),
        };
        
        // If type1 can depend on type2, level2 >= level1
        if config.can_depend_on(&type1, &type2) {
            assert!(level2 >= level1);
        }
    }
    
    #[test]
    fn test_no_cycles_in_valid_hierarchies(
        num_types in 2usize..10,
    ) {
        // Generate types at different levels
        let mut levels = HashMap::new();
        for i in 0..num_types {
            levels.insert(i as u8, vec![format!("type{}", i)]);
        }
        
        let config = HierarchyConfig {
            default_type: "type0".into(),
            levels,
            strategic_types: vec![],
            label_associations: HashMap::new(),
        };
        
        // Build dependency graph following can_depend_on rules
        let mut edges = vec![];
        for i in 0..num_types {
            for j in 0..num_types {
                let type_i = format!("type{}", i);
                let type_j = format!("type{}", j);
                if config.can_depend_on(&type_i, &type_j) && i != j {
                    edges.push((i, j));
                }
            }
        }
        
        // Verify no cycles exist (using topological sort)
        assert!(!has_cycle(num_types, &edges));
    }
}

fn has_cycle(n: usize, edges: &[(usize, usize)]) -> bool {
    let mut graph = vec![vec![]; n];
    for &(u, v) in edges {
        graph[u].push(v);
    }
    
    let mut visited = vec![false; n];
    let mut rec_stack = vec![false; n];
    
    fn dfs(node: usize, graph: &[Vec<usize>], visited: &mut [bool], rec_stack: &mut [bool]) -> bool {
        visited[node] = true;
        rec_stack[node] = true;
        
        for &neighbor in &graph[node] {
            if !visited[neighbor] {
                if dfs(neighbor, graph, visited, rec_stack) {
                    return true;
                }
            } else if rec_stack[neighbor] {
                return true;
            }
        }
        
        rec_stack[node] = false;
        false
    }
    
    for i in 0..n {
        if !visited[i] && dfs(i, &graph, &mut visited, &mut rec_stack) {
            return true;
        }
    }
    
    false
}
```

### Integration Tests
```rust
#[test]
fn test_create_with_auto_default_type() {
    let mut cmd = setup_test_env();
    
    // Create issue without type label
    let issue = cmd.create_issue(CreateIssueParams {
        title: "Fix login bug".into(),
        labels: vec!["epic:auth".into()],
        ..Default::default()
    }).unwrap();
    
    // Should auto-add default type
    assert!(issue.labels.contains(&"type:task".into()));
}

#[test]
fn test_create_valid_hierarchy() {
    let mut cmd = setup_test_env();
    
    // Create milestone (level 3)
    let milestone = cmd.create_issue(CreateIssueParams {
        title: "v1.0".into(),
        labels: vec!["type:milestone".into(), "milestone:v1.0".into()],
        ..Default::default()
    }).unwrap();
    
    // Create epic (level 2) under milestone
    let epic = cmd.create_issue(CreateIssueParams {
        title: "Auth".into(),
        labels: vec!["type:epic".into(), "epic:auth".into(), "milestone:v1.0".into()],
        ..Default::default()
    }).unwrap();
    
    cmd.add_dependency(&epic.id, &milestone.id).unwrap();
    
    // Create task (level 1) under epic
    let task = cmd.create_issue(CreateIssueParams {
        title: "Login".into(),
        labels: vec!["type:task".into(), "epic:auth".into()],
        ..Default::default()
    }).unwrap();
    
    cmd.add_dependency(&task.id, &epic.id).unwrap();
    
    // Validation should pass
    let report = cmd.validate_type_hierarchy().unwrap();
    assert!(report.errors.is_empty());
}

#[test]
fn test_reject_reverse_hierarchy() {
    let mut cmd = setup_test_env();
    
    // Create milestone and epic
    let milestone = cmd.create_issue(CreateIssueParams {
        title: "v1.0".into(),
        labels: vec!["type:milestone".into()],
        ..Default::default()
    }).unwrap();
    
    let epic = cmd.create_issue(CreateIssueParams {
        title: "Auth".into(),
        labels: vec!["type:epic".into()],
        ..Default::default()
    }).unwrap();
    
    // Attempt to make milestone depend on epic (reverse flow - should fail)
    let result = cmd.add_dependency(&milestone.id, &epic.id);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Hierarchy violation"));
    assert!(result.unwrap_err().to_string().contains("cannot depend on"));
}

#[test]
fn test_custom_hierarchy_config() {
    let mut cmd = setup_test_env_with_config(extended_hierarchy_config());
    
    // Create subtask (level 0)
    let subtask = cmd.create_issue(CreateIssueParams {
        title: "Update schema".into(),
        labels: vec!["type:subtask".into()],
        ..Default::default()
    }).unwrap();
    
    // Create task (level 1)
    let task = cmd.create_issue(CreateIssueParams {
        title: "Database migration".into(),
        labels: vec!["type:task".into()],
        ..Default::default()
    }).unwrap();
    
    // Subtask can depend on task (upward flow)
    cmd.add_dependency(&subtask.id, &task.id).unwrap();
    
    // Task cannot depend on subtask (reverse flow)
    let result = cmd.add_dependency(&task.id, &subtask.id);
    assert!(result.is_err());
}
```

---

## MCP Integration

### Updated Tool Schemas

```typescript
// MCP tool: issue_create
{
  name: "issue_create",
  description: `Create a new issue with configurable type hierarchy validation.

TYPE HIERARCHY (configurable per repository):
  Default 3-level hierarchy:
    milestone (level 3) - top-level container
      ↓
    epic (level 2) - mid-level grouping
      ↓
    task/bug/feature/research (level 1) - work items

  Dependencies flow upward: Lower-level issues can depend on same or higher-level issues.

AUTO-DEFAULT TYPE:
  - If no type:* label provided, auto-adds the default type (usually "task")
  - Configure default in .jit/config.toml: type_hierarchy.default_type

REQUIRED:
  - Exactly ONE type:* label (or omit to use default)

OPTIONAL BUT RECOMMENDED:
  - epic:* label for type:epic issues
  - milestone:* label for type:milestone issues
  - Parent labels (epic:* or milestone:*) for leaf-level issues to avoid orphans

HIERARCHY CONSTRAINTS:
  - Higher-level issues cannot depend on lower-level issues
  - Peer dependencies (same level) are allowed
  - Hierarchy is configurable and can be extended up or down

Examples:
  // Default type (task) auto-added
  {
    "title": "Fix login bug",
    "labels": ["epic:auth"]
  }
  
  // Explicit epic
  {
    "title": "User Authentication",
    "labels": ["type:epic", "epic:auth", "milestone:v1.0"]
  }
  
  // Leaf task with parent
  {
    "title": "Implement JWT",
    "labels": ["type:task", "epic:auth"]
  }
`,
  inputSchema: {
    type: "object",
    properties: {
      title: { type: "string" },
      labels: { 
        type: "array",
        items: { type: "string" },
        description: "Labels. Omit type:* to use default type (task)"
      },
      force: {
        type: "boolean",
        description: "Bypass validation warnings (strategic consistency, orphan checks)"
      },
      orphan: {
        type: "boolean",
        description: "Allow leaf-level issues without parent labels (no epic/milestone)"
      }
    },
    required: ["title"]
  }
}

// MCP tool: dep_add
{
  name: "dep_add",
  description: `Add a dependency with configurable hierarchy validation.

DEPENDENCY FLOW:
  Dependencies flow UPWARD in the hierarchy:
  - Lower-level → same level (peer dependency) ✓
  - Lower-level → higher level (belongs to) ✓
  - Higher-level → lower level (reverse flow) ✗

DEFAULT 3-LEVEL HIERARCHY:
  Valid:
    - task → epic (task belongs to epic)
    - task → milestone (task belongs to milestone)
    - epic → milestone (epic belongs to milestone)
    - task → task (peer dependency)
    - epic → epic (peer dependency)
  
  Invalid:
    - milestone → epic (reverse: higher → lower)
    - epic → task (reverse: higher → lower)

CUSTOM HIERARCHIES:
  The hierarchy is configurable in .jit/config.toml
  Extended hierarchies may have more levels (e.g., 0-4)
  Same rule applies: dependencies flow upward

The system will reject reverse hierarchy relationships with clear error messages.
`,
  inputSchema: {
    type: "object",
    properties: {
      issue_id: { 
        type: "string", 
        description: "Child issue (lower or same level)" 
      },
      dependency_id: { 
        type: "string", 
        description: "Parent issue (higher or same level)" 
      }
    },
    required: ["issue_id", "dependency_id"]
  }
}

// MCP tool: config_show_hierarchy
{
  name: "config_show_hierarchy",
  description: `Show the configured type hierarchy for the repository.

Returns:
  - Hierarchy levels and types at each level
  - Default type
  - Strategic types
  - Label associations
  - Validation rules

Useful for agents to understand the hierarchy before creating issues.
`,
  inputSchema: {
    type: "object",
    properties: {}
  }
}
```

### Agent Guidance in System Prompt

```markdown
## Type Hierarchy System

The repository uses a **configurable type hierarchy** for issue organization:

### Default Hierarchy (3 levels)
```
Level 3: milestone          (top-level time-bounded goals)
Level 2: epic               (feature groupings)
Level 1: task, bug, feature (work items - DEFAULT)
```

### Key Rules

1. **Auto-Default Type**: If you don't specify a `type:*` label, the system auto-adds `type:task`
   - Explicit: `--label "type:bug"` for bugs
   - Implicit: Omit `--label "type:*"` for tasks

2. **Hierarchy Flow**: Dependencies flow UPWARD
   - ✅ task → epic (task belongs to epic)
   - ✅ epic → milestone (epic belongs to milestone)
   - ❌ milestone → epic (reverse flow, blocked)
   - ❌ epic → task (reverse flow, blocked)

3. **Strategic Labels**: Strategic types should have matching labels
   - `type:epic` → add `epic:auth` (or similar)
   - `type:milestone` → add `milestone:v1.0` (or similar)
   - System warns if missing (use `force: true` to bypass)

4. **Orphan Prevention**: Leaf types (task, bug) should belong to something
   - Add `epic:*` or `milestone:*` labels
   - System warns if isolated (use `orphan: true` to bypass)

### Best Practices

**Creating an epic:**
```json
{
  "title": "User Authentication System",
  "labels": ["type:epic", "epic:auth", "milestone:v1.0"]
}
```

**Creating a task (with auto-default):**
```json
{
  "title": "Implement JWT validation",
  "labels": ["epic:auth"]
}
// System auto-adds type:task
```

**Creating a task (explicit type):**
```json
{
  "title": "Fix login bug",
  "labels": ["type:bug", "epic:auth"]
}
```

**Check hierarchy before operations:**
```typescript
// Query the current hierarchy configuration
const config = await mcp.call("config_show_hierarchy", {});
console.log(config.levels);  // See levels and types
console.log(config.default_type);  // See auto-default
```

### Custom Hierarchies

Some repositories may use **extended hierarchies** (4-5 levels) or **custom types**:
- Always check `config_show_hierarchy` first
- Respect the configured levels
- Use the repository's default type when omitting `type:*`

### Error Handling

If you get a hierarchy violation error:
```
Error: Hierarchy violation: milestone (level 3) cannot depend on epic (level 2)
Hint: Did you mean: jit dep add <epic_id> <milestone_id>?
```

You likely reversed the dependency direction. **Swap the arguments** to fix.
```

---

## Benefits

### For Agents
1. **Auto-Default**: Don't need to remember default type (usually task) - just omit it
2. **Clear Rules**: Simple upward flow model (lower → higher)
3. **Immediate Feedback**: Errors caught at creation/dependency time
4. **Helpful Suggestions**: Error messages guide to correct usage
5. **Flexible Enforcement**: Warnings don't block, only guide
6. **Discoverable**: Can query hierarchy config via MCP tool

### For Humans
1. **Consistent Structure**: Hierarchy rules prevent logical errors
2. **Self-Documenting**: Type labels make relationships explicit
3. **Easy Navigation**: Strategic views work reliably
4. **Low Maintenance**: Validation catches mistakes early
5. **Customizable**: Can configure hierarchy per repository needs
6. **Extensible**: Can add levels above (strategic) or below (granular)

### For System
1. **Reliable Queries**: Strategic views always return correct results
2. **Graph Integrity**: Hierarchy constraints complement DAG validation
3. **Type Safety**: Issue types are explicit and validated
4. **Extensible**: Easy to add new types or levels via configuration
5. **Backward Compatible**: Default config matches current 3-level model
6. **No Breaking Changes**: Existing issues work as-is

---

## Comparison with Alternatives

### Alternative 1: Strict Enforcement (Rejected)
**Pros**: Guarantees correctness
**Cons**: Too rigid, blocks valid use cases, frustrates users

### Alternative 2: No Enforcement (Current State)
**Pros**: Maximum flexibility
**Cons**: Agents make mistakes, inconsistent data, broken views

### Alternative 3: Hard-Coded Types (Rejected)
**Pros**: Simple implementation
**Cons**: Not extensible, forces specific terminology (task/epic/milestone)

### Alternative 4: Configurable Hierarchy (This Proposal) ✅
**Pros**: 
- Balances flexibility and correctness
- Agent-friendly with auto-defaults
- Extensible in both directions (add levels 0, 4, 5...)
- Terminology can be customized (story/theme/release...)
- Supports different workflows (Agile, Waterfall, custom)

**Cons**: 
- Requires configuration parsing (TOML)
- Slightly more complex validation logic
- Need to document configuration options

---

## Recommendation

**Implement Phase 1-3 immediately** (4-5 hours):
1. Core validation module with configurable hierarchy
2. Integration with create/dep commands (with auto-default)
3. Validation command with auto-fix

**Defer Phase 4-6** until we gather real-world usage patterns:
4. Hierarchy templates (default, extended, agile, minimal)
5. Web UI integration (type badges, hierarchy visualization)
6. Comprehensive documentation with examples

This provides:
- ✅ Configurable hierarchy (extend up or down)
- ✅ Flexible terminology (task/story, epic/theme, milestone/release)
- ✅ Auto-default type (no manual type:* for most issues)
- ✅ Hierarchy enforcement (upward flow only)
- ✅ Type label validation (required but auto-added)
- ✅ Strategic consistency (warnings for epics/milestones)
- ✅ Clear error messages with suggestions
- ✅ Auto-fix capabilities
- ✅ Backward compatible (default config matches current usage)
- ✅ Functional, testable design with property-based tests

**Total Initial Effort**: ~5 hours for core functionality
**Total Full Implementation**: ~8 hours including templates and UI

---

## Migration Path

### Existing Repositories

**Option 1: No config (default behavior)**
- Uses default 3-level hierarchy (task, epic, milestone)
- All existing issues work as-is
- New issues get `type:task` auto-added if missing
- No breaking changes

**Option 2: Explicit config**
- Add `.jit/config.toml` with desired hierarchy
- Run `jit validate --type-hierarchy --fix` to repair unknown types
- Adjust type labels to match new configuration
- Can extend hierarchy without breaking existing issues

### Example Migration Script

```bash
#!/bin/bash
# Migrate existing repository to extended hierarchy

# 1. Initialize config with extended template
jit init --hierarchy-template extended

# 2. Validate current issues
jit validate --type-hierarchy > validation-report.txt

# 3. Auto-fix issues with missing type labels
jit validate --type-hierarchy --fix --yes

# 4. Review and fix hierarchy violations manually
# (Check validation-report.txt for specific issues)

# 5. Verify all issues are valid
jit validate --type-hierarchy
echo "Migration complete!"
```

---

## Future Enhancements

### Phase 7: Advanced Features (Deferred)

1. **Hierarchy Visualization**
   - Generate hierarchy diagram: `jit config show-hierarchy --diagram`
   - Show actual issue distribution across levels
   - Identify most/least used types

2. **Smart Type Suggestions**
   - Analyze issue title/description to suggest type
   - Machine learning model for type classification
   - Historical type usage patterns

3. **Conditional Validation Rules**
   - Allow custom validation logic in config
   - Example: "type:spike must have time-box label"
   - Lua or similar for custom validators

4. **Hierarchy Analytics**
   - Report on hierarchy depth (how deep is our tree?)
   - Identify issues at wrong level (task with 10+ dependents → should be epic?)
   - Suggest refactoring opportunities

5. **Multi-Tenant Hierarchies**
   - Different teams use different hierarchies
   - Team-specific config overlays
   - Validation scoped to team labels

---

## Appendix: Configuration Examples

### Example 1: Jira-Style Hierarchy
```toml
[type_hierarchy]
default_type = "task"
[type_hierarchy.levels]
1 = ["subtask"]
2 = ["task", "bug"]
3 = ["story"]
4 = ["epic"]
5 = ["initiative"]
[type_hierarchy.strategic]
types = ["epic", "initiative"]
```

### Example 2: GitHub-Style (Flat)
```toml
[type_hierarchy]
default_type = "issue"
[type_hierarchy.levels]
1 = ["issue", "pull-request", "discussion"]
2 = ["milestone"]
# No epics, very flat
```

### Example 3: SAFe-Style (5 Levels)
```toml
[type_hierarchy]
default_type = "story"
[type_hierarchy.levels]
0 = ["task"]
1 = ["story", "spike", "bug"]
2 = ["feature"]
3 = ["epic"]
4 = ["capability"]
5 = ["solution"]
[type_hierarchy.strategic]
types = ["epic", "capability", "solution"]
```
