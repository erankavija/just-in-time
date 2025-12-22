# Labels Config Consolidation Design

**Date:** 2025-12-14
**Status:** Planned for Phase C.1
**Estimated Effort:** 1 hour

## Motivation

Currently we have:
- `.jit/label-namespaces.json` - Label namespace configuration
- Proposed: `.jit/type-hierarchy.json` - Type hierarchy configuration

This creates:
- ❌ Duplication: "type" namespace defined in both places
- ❌ Potential inconsistency: namespace exists but no hierarchy, or vice versa
- ❌ Confusing mental model: related configs in different files
- ❌ More complex initialization and validation

## Proposed Solution

**Consolidate into single `.jit/labels.json` file** containing all label-related configuration.

## File Structure

### Schema Version 2 (New)

```json
{
  "schema_version": 2,
  "type_hierarchy": {
    "milestone": 1,
    "epic": 2,
    "story": 3,
    "task": 4
  },
  "namespaces": {
    "type": {
      "description": "Issue type with hierarchical levels",
      "unique": true,
      "strategic": false
    },
    "milestone": {
      "description": "Release milestones and version targets",
      "unique": false,
      "strategic": true
    },
    "epic": {
      "description": "Large features or initiatives",
      "unique": false,
      "strategic": true
    },
    "component": {
      "description": "Technical component or subsystem",
      "unique": false,
      "strategic": false
    },
    "team": {
      "description": "Owning team",
      "unique": true,
      "strategic": false
    }
  }
}
```

### Schema Version 1 (Backward Compatible)

```json
{
  "schema_version": 1,
  "namespaces": { ... }
}
```

## Implementation Plan

### 1. Data Model Updates

**Current:**
```rust
pub struct LabelNamespaces {
    pub schema_version: u32,
    pub namespaces: HashMap<String, NamespaceConfig>,
}
```

**New:**
```rust
pub struct LabelsConfig {
    pub schema_version: u32,
    pub type_hierarchy: Option<HashMap<String, u8>>, // None for schema v1
    pub namespaces: HashMap<String, NamespaceConfig>,
}

impl LabelsConfig {
    /// Get type hierarchy, falling back to default if not configured
    pub fn get_hierarchy(&self) -> HierarchyConfig {
        match &self.type_hierarchy {
            Some(map) => HierarchyConfig::new(map.clone()).unwrap_or_default(),
            None => HierarchyConfig::default(),
        }
    }
}
```

### 2. Storage Layer Updates

**Current:**
```rust
fn load_label_namespaces(&self) -> Result<LabelNamespaces>
fn save_label_namespaces(&self, namespaces: &LabelNamespaces) -> Result<()>
```

**New:**
```rust
fn load_labels_config(&self) -> Result<LabelsConfig> {
    // Try new filename first
    if let Ok(config) = self.load_from(".jit/labels.json") {
        return Ok(config);
    }
    
    // Fall back to old filename for backward compat
    if let Ok(config) = self.load_from(".jit/label-namespaces.json") {
        return Ok(config);
    }
    
    // Return default
    Ok(LabelsConfig::default())
}

fn save_labels_config(&self, config: &LabelsConfig) -> Result<()> {
    // Always save to new filename
    self.save_to(".jit/labels.json", config)
}
```

### 3. Migration Strategy

**Automatic on first write:**
1. Load from old location (`.jit/label-namespaces.json`)
2. Upgrade schema: v1 → v2 (add default type_hierarchy)
3. Save to new location (`.jit/labels.json`)
4. Keep old file for backward compat (don't delete)

**Manual migration command (optional):**
```bash
jit config migrate --labels
```

### 4. Default Configuration

```rust
impl Default for LabelsConfig {
    fn default() -> Self {
        let mut type_hierarchy = HashMap::new();
        type_hierarchy.insert("milestone".to_string(), 1);
        type_hierarchy.insert("epic".to_string(), 2);
        type_hierarchy.insert("story".to_string(), 3);
        type_hierarchy.insert("task".to_string(), 4);
        
        let mut namespaces = HashMap::new();
        namespaces.insert("type".to_string(), NamespaceConfig {
            description: "Issue type with hierarchical levels".to_string(),
            unique: true,
            strategic: false,
        });
        // ... other default namespaces
        
        Self {
            schema_version: 2,
            type_hierarchy: Some(type_hierarchy),
            namespaces,
        }
    }
}
```

## Benefits

✅ **Single source of truth** - All label config in one place
✅ **Consistency** - Type hierarchy and namespace definition together
✅ **Clearer naming** - `labels.json` more accurately describes contents
✅ **Backward compatible** - Reads old filename, auto-migrates on write
✅ **Simpler mental model** - One config file for all label concerns
✅ **Easier validation** - Can cross-check hierarchy types against namespaces

## Risks & Mitigations

**Risk:** Breaking existing tools/scripts that read label-namespaces.json
**Mitigation:** Keep old file, update on write only (lazy migration)

**Risk:** Schema version mismatches
**Mitigation:** Explicit version checks, graceful fallback to v1

**Risk:** Complex migration code
**Mitigation:** Simple fallback chain, auto-upgrade transparent to users

## Testing Strategy

1. **Loading:** Test both old and new filenames
2. **Migration:** Test v1 → v2 upgrade path
3. **Backward compat:** Test reading v1 files
4. **Default fallback:** Test missing file → default config
5. **Validation:** Test hierarchy types exist in namespaces
6. **Integration:** Test all existing label operations still work

## Rollout Plan

**Phase C.1 (This Phase):**
1. Update data model (LabelsConfig)
2. Update storage layer (load/save with fallback)
3. Update all callsites (load_label_namespaces → load_labels_config)
4. Add migration logic (v1 → v2)
5. Update tests (8 new tests for migration/compat)
6. Update documentation

**Phase C.2 (Next Phase):**
- Use consolidated config for hierarchy commands
- Add template system
- No additional migration needed

## Alternative Considered: Keep Separate Files

**Rejected because:**
- Violates DRY (type namespace in two places)
- More complex to maintain consistency
- Confusing for users (which file to edit?)
- No significant benefit to separation

## Files to Modify

- `crates/jit/src/labels.rs` - Update LabelsConfig struct
- `crates/jit/src/storage.rs` - Update load/save methods (trait + impls)
- `crates/jit/src/commands.rs` - Update callsites
- `crates/jit/src/type_hierarchy.rs` - Add HierarchyConfig::from_config()
- Tests in all above files
- Documentation: mention migration path

## Success Criteria

- [ ] All 220+ existing tests pass
- [ ] 8 new tests for migration/compat pass
- [ ] Old repositories automatically migrate on first write
- [ ] New repositories get schema v2 by default
- [ ] Zero breaking changes for users
- [ ] Documentation updated

## Timeline

**Estimated:** 1 hour
- Data model updates: 15 min
- Storage layer updates: 20 min
- Callsite updates: 10 min
- Migration logic: 10 min
- Testing: 5 min (automated)
