# Hierarchy Type Icons - Configuration Plan

## Design Principles

1. **Domain-agnostic defaults**: Icons assigned by hierarchy LEVEL, not type name
2. **Full flexibility**: Allow custom type name → icon mapping
3. **Partial overrides**: Only specify what you want to change
4. **Fallback chain**: custom → preset → level default → no icon

## Configuration Format

### `.jit/config.toml`

```toml
[hierarchy]
milestone = 1
epic = 2
story = 3
task = 4

# Default icons by LEVEL (domain-agnostic)
# Level 1: Strategic/highest (⭐)
# Level 2: Container (📦)
# Level 3: Work unit (📝)
# Level 4+: Atomic action (☑️)

[hierarchy.icons]
# Option 1: Use a preset
preset = "simple"  # "simple" | "navigation" | "minimal" | "construction"

# Option 2: Override specific types (partial customization)
[hierarchy.icons.custom]
bug = "🐛"           # Override just bugs, keep rest from preset/defaults
epic = "🚀"          # Override epic, keep rest from preset/defaults

# Option 3: Full custom mapping (overrides preset entirely if any custom defined)
# [hierarchy.icons.custom]
# milestone = "🎯"
# epic = "📦"
# story = "📄"
# task = "✓"
# bug = "🐞"
```

## Default Icon Assignment (by level)

**Level-based defaults** (no SW dev assumptions):

```rust
// Default icons by hierarchy level
const DEFAULT_ICONS_BY_LEVEL: &[(usize, &str)] = &[
    (1, "⭐"),  // Level 1: Strategic/goal
    (2, "📦"),  // Level 2: Container/grouping
    (3, "📝"),  // Level 3: Work unit
    (4, "☑️"),  // Level 4+: Atomic action
];

// Fallback for level >= 4
const LEAF_ICON: &str = "☑️";
```

**Preset definitions** (named collections):

```rust
const ICON_PRESETS: &[(&str, &[(&str, &str)])] = &[
    ("simple", &[
        (1, "⭐"),
        (2, "📦"),
        (3, "📝"),
        (4, "☑️"),
    ]),
    
    ("navigation", &[
        (1, "🏔️"),
        (2, "🗺️"),
        (3, "🧭"),
        (4, "📍"),
    ]),
    
    ("minimal", &[
        (1, "◆"),
        (2, "▣"),
        (3, "▢"),
        (4, "□"),
    ]),
    
    ("construction", &[
        (1, "🏁"),
        (2, "🏗️"),
        (3, "🧱"),
        (4, "🔨"),
    ]),
];
```

## Resolution Algorithm

```rust
fn get_icon_for_type(type_name: &str, level: usize, config: &HierarchyConfig) -> Option<String> {
    // 1. Check custom type mapping (highest priority)
    if let Some(custom_icons) = &config.custom_icons {
        if let Some(icon) = custom_icons.get(type_name) {
            return Some(icon.clone());
        }
    }
    
    // 2. Check preset for this level
    if let Some(preset_name) = &config.icon_preset {
        if let Some(preset) = ICON_PRESETS.iter().find(|(name, _)| name == preset_name) {
            if let Some(icon) = preset.1.iter().find(|(lvl, _)| *lvl == level) {
                return Some(icon.1.to_string());
            }
        }
    }
    
    // 3. Fall back to default level mapping
    if let Some(icon) = DEFAULT_ICONS_BY_LEVEL.iter().find(|(lvl, _)| *lvl == level) {
        return Some(icon.1.to_string());
    }
    
    // 4. Fall back to leaf icon for levels >= 4
    if level >= 4 {
        return Some(LEAF_ICON.to_string());
    }
    
    // 5. No icon
    None
}
```

## Example Scenarios

### Scenario 1: Software Development (defaults)
```toml
[hierarchy]
milestone = 1  # Gets ⭐ (level 1 default)
epic = 2       # Gets 📦 (level 2 default)
story = 3      # Gets 📝 (level 3 default)
task = 4       # Gets ☑️ (level 4 default)
```

### Scenario 2: Research Project
```toml
[hierarchy]
program = 1      # Gets ⭐ (level 1 default)
project = 2      # Gets 📦 (level 2 default)
workpackage = 3  # Gets 📝 (level 3 default)
deliverable = 4  # Gets ☑️ (level 4 default)

[hierarchy.icons]
preset = "navigation"  # Use navigation theme
```

### Scenario 3: Custom with Partial Override
```toml
[hierarchy]
objective = 1
initiative = 2
feature = 3
task = 4

[hierarchy.icons]
preset = "simple"

[hierarchy.icons.custom]
objective = "🎯"   # Override just level 1
bug = "🐛"         # Add bug type (level 4, but special icon)
# initiative, feature, task get preset/default icons
```

### Scenario 4: Full Custom
```toml
[hierarchy]
goal = 1
theme = 2
capability = 3
activity = 4

[hierarchy.icons.custom]
goal = "🎯"
theme = "🎨"
capability = "⚙️"
activity = "▶️"
bug = "🔥"
```

## API Response Format

```json
{
  "hierarchy": {
    "levels": {
      "milestone": 1,
      "epic": 2,
      "story": 3,
      "task": 4
    },
    "icons": {
      "milestone": "⭐",
      "epic": "📦",
      "story": "📝",
      "task": "☑️",
      "bug": "🐛"
    }
  }
}
```

Frontend receives **resolved** icons (algorithm already applied).

## Implementation Files

### Backend

**New/Modified:**
- `crates/jit/src/config.rs`
  - Add `IconConfig` struct with `preset: Option<String>` and `custom: HashMap<String, String>`
  - Add `get_icon_for_type()` resolver function
  - Add preset definitions

- `crates/jit/src/hierarchy.rs`
  - Extend to include icon resolution
  - Provide `get_type_icon(type_name: &str) -> Option<String>`

- `crates/jit-server/src/handlers.rs`
  - Extend `/api/hierarchy` endpoint to include resolved icons map

### Frontend

**New:**
- `web/src/types/hierarchyConfig.ts`
  ```typescript
  export interface HierarchyConfig {
    levels: HierarchyLevelMap;
    icons: Record<string, string>;  // Resolved: type_name -> icon
  }
  ```

**Modified:**
- `web/src/components/Graph/nodes/ClusterNode.tsx`
  - Add icon prop to `ClusterNodeData`
  - Render icon in header: `{icon} #{nodeId}`

- `web/src/components/Graph/GraphView.tsx`
  - Fetch hierarchy config with icons
  - Pass icon to cluster node data

- `web/src/utils/clusteredGraphLayout.ts`
  - Accept hierarchy config with icons
  - Include icon in node data preparation

## Migration Strategy

1. **Phase 1**: Backend config parsing (handle missing icons gracefully)
2. **Phase 2**: Add icon resolution logic (level-based defaults + presets)
3. **Phase 3**: Expose in API (add icons to `/api/hierarchy`)
4. **Phase 4**: Frontend rendering (show icons in cluster nodes)
5. **Phase 5**: Documentation (add examples to config.toml comments)

## Default Preset Choice

**Recommendation: "simple"** (⭐📦📝☑️)
- Most universally recognized symbols
- Good rendering across platforms
- Professional yet friendly
- Works for any domain

## Future Enhancements

- UI preset selector (settings panel)
- Custom emoji picker in web UI
- Icon animation/effects on expand/collapse
- Different icons for different states (e.g., ✓ vs ☑️ for done vs in_progress)
- Accessibility: ARIA labels mapping icon to type name

## Notes

- Icons are purely visual enhancement (optional)
- System works without icons (shows just `#nodeId`)
- Icons don't affect functionality, only display
- Server-side resolution ensures consistency across UI and CLI
