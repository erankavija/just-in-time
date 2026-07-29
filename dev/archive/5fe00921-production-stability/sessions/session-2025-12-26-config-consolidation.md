# Configuration Consolidation - Session Notes

**Date**: 2025-12-26  
**Issue**: bac2a42b-160b-45bc-8ba0-357822d0a1ae  
**Status**: In Progress (30% complete)  
**Session Type**: Implementation  

## Goal

Consolidate configuration into a single source of truth (`.jit/config.toml`), eliminating fragmentation between `config.toml` and `labels.json`, and removing misleading namespace `strategic` flags.

**CRITICAL DECISION: NO BACKWARD COMPATIBILITY**
- We will break existing repos that rely on `labels.json`
- Users must migrate to `.jit/config.toml` schema v2
- Clean break is preferred over maintaining legacy code paths

## What We Accomplished

### 1. Removed Strategic Flag Confusion ✅

**Problem**: `LabelNamespace` had a `strategic: bool` field that was misleading because strategic classification is actually type-based (via `strategic_types` config), not namespace-based.

**Solution**:
```rust
// BEFORE
pub struct LabelNamespace {
    pub description: String,
    pub unique: bool,
    pub strategic: bool,  // ← MISLEADING
}

// AFTER
pub struct LabelNamespace {
    pub description: String,
    pub unique: bool,
    // strategic flag REMOVED - classification is type-based only
}
```

**Impact**:
- Updated `domain.rs` to remove field
- Updated CLI: removed `--strategic` flag from `label add-namespace`
- Updated all call sites in tests
- Updated `main.rs` output to not display strategic flag
- All 246 tests still pass

### 2. Added Schema v2 Structure ✅

**Added to `config.rs`**:

```rust
pub struct JitConfig {
    pub version: Option<VersionConfig>,           // NEW: schema versioning
    pub type_hierarchy: Option<HierarchyConfigToml>,
    pub validation: Option<ValidationConfig>,
    pub documentation: Option<DocumentationConfig>,
    pub namespaces: Option<HashMap<String, NamespaceConfig>>,  // NEW: replaces labels.json
}

pub struct VersionConfig {
    pub schema: u32,  // Track schema version for migrations
}

pub struct HierarchyConfigToml {
    pub types: HashMap<String, u8>,
    pub label_associations: Option<HashMap<String, String>>,
    pub strategic_types: Option<Vec<String>>,  // NEW: explicit strategic types list
}

pub struct ValidationConfig {
    pub strictness: Option<String>,
    pub default_type: Option<String>,              // NEW: default when missing
    pub require_type_label: Option<bool>,          // NEW: enforce type presence
    pub label_regex: Option<String>,               // NEW: format validation
    pub reject_malformed_labels: Option<bool>,     // NEW: strict validation
    pub enforce_namespace_registry: Option<bool>,  // NEW: registry enforcement
    pub warn_orphaned_leaves: Option<bool>,
    pub warn_strategic_consistency: Option<bool>,
}

pub struct NamespaceConfig {
    pub description: String,
    pub unique: bool,
    pub examples: Option<Vec<String>>,  // NEW: documentation examples
    // NO strategic flag - that's type-based only
}
```

**Tests Added**: 5 new tests for schema v2 parsing, all passing.

### 3. Updated Repository Config ✅

Updated `.jit/config.toml` to schema v2 format:

```toml
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"
story = "story"

[validation]
strictness = "loose"
default_type = "task"
require_type_label = false
label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'
reject_malformed_labels = false
enforce_namespace_registry = false
warn_orphaned_leaves = true
warn_strategic_consistency = true

# Namespace registry (replaces labels.json)
[namespaces.type]
description = "Issue type (hierarchical). Exactly one per issue."
unique = true
examples = ["type:task", "type:story", "type:epic"]

[namespaces.epic]
description = "Feature or initiative membership"
unique = false
examples = ["epic:auth", "epic:production-stability"]

# ... more namespaces
```

### 4. Namespace Loading from Config ✅ (but architecturally problematic)

**Current Implementation** (in `storage/json.rs`):
```rust
fn load_label_namespaces(&self) -> Result<LabelNamespaces> {
    // Load config.toml
    let config = crate::config::JitConfig::load(&self.root)?;
    
    // Build namespaces from config
    if let Some(namespaces_config) = config.namespaces {
        // Convert NamespaceConfig -> LabelNamespace
        // Return consolidated LabelNamespaces
    }
    
    // Fallback to labels.json if exists (with deprecation warning)
    // Then fallback to defaults
}
```

**Why This Is Problematic**:
- TOML parsing happening in `json.rs` is confusing
- Storage layer shouldn't be responsible for config management
- Violates separation of concerns
- Labels.json still supported (despite "no backward compatibility" decision)

## What's Missing (Phase 1 Incomplete)

### Critical: Architecture Issues

**Problem**: No clear separation between:
1. **Configuration management** - loading/parsing config.toml
2. **Storage layer** - persisting/loading runtime state
3. **Business logic** - using configuration to enforce rules

**Questions to Answer**:
- Where should config be loaded? At application startup? Per-command?
- Should there be a `ConfigManager` or `ConfigService`?
- How should config be passed to storage/executor layers?
- Should config be cached or loaded fresh each time?

### Missing Validation Logic

New validation fields defined but **not implemented**:

1. **`default_type`** - Should auto-assign when creating issue without type label
   - Where: `issue create` command
   - Logic: Check for type:* label, add default if missing
   
2. **`require_type_label`** - Should reject issues without type label
   - Where: `issue create`, `issue update` validation
   - Logic: Error if no type:* label present and require_type_label=true

3. **`label_regex`** - Should validate label format
   - Where: Label parsing/validation
   - Logic: Reject labels not matching regex pattern

4. **`reject_malformed_labels`** - Should error vs. warn on bad format
   - Where: Label validation
   - Logic: Hard error if true, warning if false

5. **`enforce_namespace_registry`** - Should reject unknown namespaces
   - Where: Label parsing
   - Logic: Check namespace exists in registry, error if not found

### Incomplete Consolidation

- **labels.json still exists** - Moved to `labels.json.deprecated` but still loaded as fallback
- **Should be removed entirely** - Per "no backward compatibility" decision
- **Migration path undefined** - How do existing repos update?

### Missing Documentation Updates

- `docs/reference/example-config.toml` - Needs full schema v2 example
- `README.md` - References to labels.json need updating
- `EXAMPLE.md` - Configuration examples need updating
- CLI help text - May reference old flags

## Quality Gates Status

- ✅ **tdd-reminder**: Passed (wrote tests first for new schema)
- ✅ **tests**: All 246 tests pass
- ✅ **clippy**: Zero warnings
- ✅ **fmt**: Code formatted
- ⏳ **code-review**: Pending (architectural concerns)

## Architectural Decisions Needed

### 1. Configuration Loading Strategy

**Option A: Load at Startup (Singleton Pattern)**
```rust
pub struct App {
    config: Arc<JitConfig>,
    storage: Box<dyn IssueStore>,
    executor: CommandExecutor,
}
```
- **Pros**: Single load, consistent view, easy to cache
- **Cons**: Doesn't reflect config changes during long-running sessions

**Option B: Load Per-Command (Fresh Every Time)**
```rust
impl CommandExecutor {
    pub fn create_issue(&self, ...) -> Result<String> {
        let config = JitConfig::load(self.storage.root())?;
        // use config for this command only
    }
}
```
- **Pros**: Always fresh, simpler lifecycle
- **Cons**: Repeated I/O, performance overhead

**Option C: Config Manager Service**
```rust
pub struct ConfigManager {
    root: PathBuf,
    cached: Arc<RwLock<Option<JitConfig>>>,
}

impl ConfigManager {
    pub fn load(&self) -> Result<JitConfig> { /* load with caching */ }
    pub fn reload(&self) -> Result<()> { /* force refresh */ }
}
```
- **Pros**: Separation of concerns, controllable caching
- **Cons**: More complexity, need to thread through dependencies

**RECOMMENDATION**: Option C - Create `ConfigManager` for clear responsibility separation

### 2. Storage Layer Refactoring

**Current Problem**: `IssueStore::load_label_namespaces()` now loads config.

**Proposed Solution**:
```rust
// Storage trait should NOT load config
pub trait IssueStore {
    // Remove these:
    // fn load_label_namespaces() -> ...
    // fn save_label_namespaces() -> ...
    
    // Storage is for runtime state only
    fn load_issue(&self, id: &str) -> Result<Issue>;
    fn save_issue(&self, issue: &Issue) -> Result<()>;
    // ... etc
}

// Namespaces come from config, not storage
pub struct ConfigManager {
    pub fn get_namespaces(&self) -> Result<HashMap<String, LabelNamespace>> {
        let config = self.load()?;
        // Build from config.namespaces
    }
}
```

**Migration Path**:
- Phase 1: Add ConfigManager, make storage use it
- Phase 2: Remove load/save_label_namespaces from IssueStore trait
- Phase 3: Delete labels.json support entirely

### 3. Validation Implementation Strategy

**Where should validation live?**

**Option A: In CommandExecutor**
```rust
impl CommandExecutor {
    fn validate_issue(&self, issue: &Issue, config: &JitConfig) -> Result<()> {
        // Check require_type_label
        // Check label_regex
        // Check namespace_registry
    }
}
```

**Option B: Dedicated Validator**
```rust
pub struct IssueValidator {
    config: Arc<JitConfig>,
}

impl IssueValidator {
    pub fn validate(&self, issue: &Issue) -> Result<Vec<ValidationError>> {
        // All validation logic here
    }
}
```

**RECOMMENDATION**: Option B - Clean separation, easier to test

## Next Steps

### Immediate (Fix Architecture)

1. **Create ConfigManager** (`src/config_manager.rs`)
   - Load/cache config.toml
   - Provide access to namespaces, hierarchy, validation settings
   - Replace config loading in json.rs

2. **Refactor Storage Layer**
   - Remove TOML parsing from `storage/json.rs`
   - Make storage depend on ConfigManager for namespace info
   - Clean up responsibility boundaries

3. **Remove labels.json Support**
   - Delete fallback logic entirely
   - Remove `labels.json.deprecated` file
   - Update init to create namespace section in config.toml

### Follow-up (Implement Validation)

4. **Create IssueValidator**
   - Implement `default_type` logic
   - Implement `require_type_label` check
   - Implement `label_regex` validation
   - Implement `namespace_registry` enforcement

5. **Integrate Validation**
   - Call validator in `issue create`
   - Call validator in `issue update`
   - Add validation to label operations

6. **Update Documentation**
   - Full schema v2 reference
   - Migration guide (breaking change notice)
   - Updated examples

## Breaking Changes Notice

⚠️ **This refactoring will break existing repositories** ⚠️

**What Will Break**:
- Repositories using `labels.json` will no longer work
- Must migrate to `.jit/config.toml` with `[namespaces.*]` sections
- Schema v1 configs will need updating to schema v2

**Migration Required**:
```bash
# Manual migration steps:
# 1. Add [version] schema = 2 to config.toml
# 2. Add [namespaces.*] sections from labels.json
# 3. Remove strategic flags from namespace definitions
# 4. Add strategic_types to [type_hierarchy]
# 5. Delete labels.json
```

**Future**: We may add `jit config migrate` command, but NOT for backward compatibility - only to help one-time migration.

## Lessons Learned

1. **Architecture First** - Should have designed config layer before implementing
2. **Scope Creep** - Adding fields without implementing validation was premature
3. **Test Coverage** - Tests pass but don't validate architectural soundness
4. **Clear Decisions** - "No backward compatibility" should have been enforced from start

## References

- Issue: bac2a42b-160b-45bc-8ba0-357822d0a1ae
- Plan: `dev/active/config-consolidation-plan.md`
- Config module: `crates/jit/src/config.rs`
- Storage implementation: `crates/jit/src/storage/json.rs`
- Domain models: `crates/jit/src/domain.rs`
