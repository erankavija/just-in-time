# Config Consolidation - Documentation Requirements

**Related Issue**: bac2a42b (Config Consolidation - COMPLETED)  
**Target Issue**: 44d6f247 (API endpoint for label configuration)  
**Status**: Requirements for future documentation work

## Overview

The config consolidation work (bac2a42b) is complete and production-ready:
- Eliminated labels.json entirely (breaking change)
- Created ConfigManager for centralized config loading
- Implemented IssueValidator with 4 validation rules
- All integrated and tested (264 tests passing)

However, several documentation updates are needed to reflect these changes.

## Documentation Requirements

### 1. Update config.toml Reference Documentation

**File**: `docs/reference/example-config.toml` (or create if missing)

**Content Needed**:
```toml
# Full schema v2 example with comprehensive comments

[version]
schema = 2  # Required for new features

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]  # Used by 'query strategic'

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
# Auto-assign type when missing (optional)
default_type = "task"

# Require type:* label on all issues (default: false)
require_type_label = false

# Label format validation regex (optional)
label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'

# Reject malformed labels vs warn (default: false)
reject_malformed_labels = false

# Enforce namespace registry (default: false)
enforce_namespace_registry = false

# Namespace registry - replaces labels.json
[namespaces.type]
description = "Issue type (hierarchical)"
unique = true
examples = ["type:task", "type:story", "type:epic"]

[namespaces.epic]
description = "Epic membership"
unique = false
examples = ["epic:auth", "epic:production-stability"]

# Add more namespaces as needed...
```

### 2. Breaking Changes Notice

**Where**: README.md or CHANGELOG.md

**Content**:
- **BREAKING**: labels.json is no longer supported
- All configuration must be in `.jit/config.toml`
- Namespace definitions moved to `[namespaces.*]` sections
- `strategic` flag removed from namespaces (strategic is type-based only)
- `jit label add-namespace` command removed (edit config.toml directly)

### 3. Update EXAMPLE.md

**Changes Needed**:
- Remove references to labels.json
- Show config.toml examples for:
  - Adding custom namespaces
  - Enabling validation rules
  - Setting default_type
- Update strategic classification examples to emphasize type-based nature

### 4. Update README.md

**Changes Needed**:
- Configuration section: mention config.toml as single source of truth
- Remove any labels.json mentions
- Link to config reference docs

### 5. Migration Guide (Optional)

**File**: `docs/guides/migrating-from-labels-json.md` (if needed for historical reference)

Since we're the only user and already migrated, this is low priority. But for completeness:

```markdown
# Migrating from labels.json to config.toml

**Note**: This guide is for historical reference. All current JIT repositories 
use config.toml schema v2.

## What Changed

- labels.json → config.toml [namespaces.*] sections
- strategic flags removed (now type-based only)
- CLI command removed: jit label add-namespace

## Migration Steps

1. Add [version] schema = 2 to config.toml
2. Copy namespaces from labels.json to [namespaces.*] sections
3. Remove strategic flags
4. Add strategic_types to [type_hierarchy]
5. Delete labels.json
```

## API Endpoint Documentation (Issue 44d6f247)

When implementing the API endpoint, update it to:

**Old (from issue description)**:
```
- Reads from .jit/labels.json
```

**New**:
```
- Reads from .jit/config.toml via ConfigManager
- Returns namespaces, strategic_types, hierarchy from config
- GET /api/config/strategic-types → {strategic_types: [...]}
- GET /api/config/hierarchy → {types: {...}, strategic_types: [...]}
- GET /api/config/namespaces → {namespaces: {...}}
```

## Success Criteria

- [ ] docs/reference/example-config.toml exists with schema v2 examples
- [ ] README.md updated to reference config.toml (no labels.json mentions)
- [ ] EXAMPLE.md shows config.toml usage patterns
- [ ] Breaking changes documented (CHANGELOG or release notes)
- [ ] API endpoint issue (44d6f247) updated to reference ConfigManager

## Priority

**Low-Medium** - The system works without these docs, but they're valuable for:
- Future contributors
- Reference when configuring new features
- Historical context for breaking changes

## Effort Estimate

- Config reference example: 30 minutes
- README/EXAMPLE updates: 30 minutes
- Breaking changes note: 15 minutes
- Total: ~1-2 hours
