# Configuration Consolidation Plan

**Status**: Active  
**Epic**: Production Stability  
**Category**: Design  
**Created**: 2025-12-26

## Problem Statement

The current configuration system suffers from fragmentation and duplication across multiple files, creating maintenance burden, inconsistency risks, and confusion about the source of truth for strategic vs. tactical issue classification.

### Current Issues

1. **Duplicate Hierarchy Data**
   - `.jit/labels.json` contains: `type_hierarchy`, `label_associations`, `strategic_types`
   - `.jit/config.toml` contains: `type_hierarchy.types`, `type_hierarchy.label_associations`, `strategic_types`
   - Risk: Two sources of truth can diverge

2. **Strategic Flag Confusion**
   - `labels.json` has per-namespace `"strategic": true/false` flags
   - But `query strategic` uses `strategic_types` from type labels, **not** namespace flags
   - Namespace strategic flags are misleading and unused in core logic

3. **Web UI Hardcoding**
   ```typescript
   // src/utils/strategicView.ts
   const DEFAULT_STRATEGIC_TYPES = ['milestone', 'epic']; // HARDCODED!
   // TODO: Fetch strategic_types from API instead of hardcoding
   ```

4. **Missing Validation Settings**
   - No `default_type` (when type label is missing)
   - No `require_type_label` toggle
   - No label format validation (`label_regex`, `reject_malformed_labels`)
   - No namespace registry enforcement

## Design Goals

1. **Single Source of Truth**: All human-authored configuration in `.jit/config.toml`
2. **Clear Separation**: TOML for preferences, JSON for runtime state
3. **Strategic Clarity**: Type-based only, no namespace-level strategic flags
4. **Validation Control**: Configurable strictness and format enforcement
5. **API-Driven UI**: Web UI fetches config from API, no hardcoding
6. **Backward Compatibility**: Migration path for existing repos

## Proposed Architecture

### File Roles

**`.jit/config.toml`** - User-editable preferences (version controlled)
- Type hierarchy definitions
- Label namespace registry
- Validation policies
- Documentation lifecycle settings
- Strategic type configuration

**`.jit/*.json`** - Machine-generated runtime state (auto-managed)
- `issues/*.json` - Mutable workflow state
- `index.json` - Performance cache
- `events.jsonl` - Append-only audit log

### Consolidated Schema (v2)

```toml
[version]
schema = 2

[type_hierarchy]
# Applied if no type:* label exists
default_type = "task"

# Levels: 1 = strategic, 3+ = tactical
[type_hierarchy.levels]
1 = ["milestone", "release"]
2 = ["epic", "theme"]
3 = ["story", "task", "bug", "spike"]

# Strategic view is driven EXCLUSIVELY by type labels
[type_hierarchy.strategic_types]
types = ["milestone", "release", "epic", "theme"]

# Type → membership label namespace mapping
[type_hierarchy.label_associations]
milestone = "milestone"
release   = "milestone"
epic      = "epic"
theme     = "epic"
story     = "story"

[validation]
# Strongly recommended for consistency
require_type_label = true

# Leaves should belong under some membership label
warn_orphaned_leaves = true

# Strategic types should have corresponding membership label
warn_missing_membership_for_strategic = true

# Label format enforcement
enforce_namespace_registry = true
reject_malformed_labels = true
label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'

# Namespace registry (replaces labels.json)
# NO "strategic" flags - strategic is derived from type labels
[namespaces.type]
description = "Issue type (hierarchical). Exactly one per issue."
unique = true
examples = ["type:task", "type:story", "type:epic"]

[namespaces.epic]
description = "Feature or initiative membership"
unique = false
examples = ["epic:auth", "epic:billing", "epic:api"]

[namespaces.milestone]
description = "Release or time-bounded goal membership"
unique = false
examples = ["milestone:v1.0", "milestone:q1-2026"]

[namespaces.story]
description = "Story membership (optional)"
unique = false
examples = ["story:login", "story:checkout"]

[namespaces.component]
description = "Technical area or subsystem"
unique = false
examples = ["component:backend", "component:frontend"]

[namespaces.team]
description = "Owning team"
unique = true
examples = ["team:core", "team:platform"]

[namespaces.resolution]
description = "Reason for issue closure"
unique = true
examples = ["resolution:wont-fix", "resolution:duplicate"]

[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

# Archive categories
categories.design   = "features"
categories.analysis = "bug-fixes"
categories.refactor = "refactorings"
categories.session  = "sessions"
categories.study    = "studies"

# Optional Phase 2 automation
mode = "manual"              # manual | release | done
retention_releases = 2
retention_days = 30
```

## Implementation Plan

### Phase 1: Core Refactoring (High Priority)

**Goal**: Eliminate duplication and strategic flag confusion

Tasks:
1. Update `config.rs` schema
   - Add `version.schema` field
   - Add `validation.{default_type, require_type_label, label_regex}`
   - Add `namespaces.*` tables (without `strategic` field)
   - Add `type_hierarchy.levels` and `strategic_types.types`

2. Migrate `storage/json.rs`
   - Load namespaces from `config.toml` instead of `labels.json`
   - Keep backward compatibility: fall back to `labels.json` if config.namespaces is missing
   - Log deprecation warning if `labels.json` is used

3. Remove strategic flag from domain
   - Update `domain::LabelNamespace` - remove `strategic: bool` field
   - Update all tests that reference namespace strategic flags
   - Update CLI `label add-namespace` - remove `--strategic` flag

4. Update validation logic
   - Implement `default_type` assignment in `issue create`
   - Implement `require_type_label` validation
   - Implement `label_regex` validation
   - Implement `enforce_namespace_registry` check

**Tests Required**:
- Config parsing with new schema
- Backward compatibility with old `labels.json`
- Validation with new settings enabled/disabled
- Strategic query unchanged behavior (uses type labels only)

### Phase 2: API & Web UI (Medium Priority)

**Goal**: Eliminate hardcoded strategic types in web UI

Tasks:
1. Add API endpoint
   - `GET /api/config/strategic-types` → `{"strategic_types": ["milestone", "epic"]}`
   - `GET /api/config/hierarchy` → full hierarchy config
   - `GET /api/config/namespaces` → namespace registry

2. Update web UI
   - Fetch strategic types from API on startup
   - Remove `DEFAULT_STRATEGIC_TYPES` hardcoding
   - Add loading state for config fetch
   - Cache config in React context

**Tests Required**:
- API endpoint returns correct config
- Web UI handles config loading states
- Strategic view filters correctly with custom types

### Phase 3: Migration Tooling (Lower Priority)

**Goal**: Help users migrate existing repos

Tasks:
1. Add `jit config migrate` command
   - Read `labels.json`
   - Generate equivalent TOML sections
   - Append to `.jit/config.toml` or create if missing
   - Create backup of original `labels.json`
   - Report success and changes made

2. Add `jit config validate` command
   - Check for conflicts between `labels.json` and `config.toml`
   - Warn if using deprecated `labels.json`
   - Validate TOML schema version
   - Check for required fields

**Tests Required**:
- Migration from various `labels.json` formats
- Conflict detection
- Idempotency (running migrate twice)

### Phase 4: Documentation (Critical)

**Goal**: Clear migration guide and updated examples

Tasks:
1. Update `docs/reference/example-config.toml`
   - Add full schema v2 example with comments
   - Document all new fields
   - Explain strategic classification (type-based only)

2. Create migration guide
   - Document breaking changes
   - Provide migration steps
   - Explain backward compatibility period
   - FAQ for common issues

3. Update README and EXAMPLE.md
   - Reflect new configuration structure
   - Update CLI examples
   - Document strategic query behavior

4. Update CONTRIBUTOR-QUICKSTART.md
   - Explain configuration architecture
   - Document file roles (TOML vs JSON)
   - Add guidelines for config changes

## Migration Strategy

### Backward Compatibility

**Grace Period**: 2-3 releases

During grace period:
- Support both `labels.json` and `config.toml` for namespace registry
- Prefer `config.toml` if both exist
- Log deprecation warning when `labels.json` is used
- CLI migration command available

After grace period:
- Remove `labels.json` loading code
- Require `config.toml` for namespace registry
- Breaking change: schema v3

### Deprecation Timeline

**v1.1** (Current + 1):
- Add schema v2 support
- Add deprecation warnings for `labels.json`
- Add `jit config migrate` command

**v1.2**:
- Make `labels.json` opt-in with explicit flag
- Require manual acknowledgment to use legacy format

**v2.0**:
- Remove `labels.json` support entirely
- Schema v3 becomes standard

## Risk Assessment

### High Risk
- Breaking existing repos without migration path
- **Mitigation**: Backward compatibility, migration tooling, clear warnings

### Medium Risk
- Test coverage gaps causing regressions
- **Mitigation**: Comprehensive test suite before refactoring

### Low Risk
- Web UI config fetch adds latency
- **Mitigation**: Cache config, load asynchronously, use sensible defaults

## Success Metrics

1. **Zero configuration conflicts** - No duplication across files
2. **Single source of truth** - Strategic classification unambiguous
3. **Web UI config-driven** - No hardcoded strategic types
4. **Validation enabled** - Label format enforced by default
5. **Migration success** - Existing repos can migrate without manual edits
6. **Documentation complete** - Users understand config architecture

## Open Questions

1. Should `gates.json` also migrate to TOML?
   - **Consideration**: Gates are typically configured once, benefit from comments
   - **Decision**: Future refactoring, not in this phase

2. Should we version the config schema explicitly?
   - **Decision**: Yes, `[version] schema = 2` enables future migrations

3. How aggressive should default validation be?
   - **Recommendation**: Start with `strictness = "loose"`, allow opt-in to strict
   - **Rationale**: Avoid blocking existing workflows

4. Should documentation lifecycle settings move to separate file?
   - **Decision**: No, keep in single config.toml for simplicity
   - **Rationale**: Related to validation and lifecycle management

## References

- Current implementation: `crates/jit/src/config.rs`
- Namespace loading: `crates/jit/src/storage/json.rs`
- Strategic query: `crates/jit/src/commands/query.rs::query_strategic()`
- Web UI hardcoding: `web/src/utils/strategicView.ts`
- Example config: `docs/reference/example-config.toml`
