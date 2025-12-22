# Graph Filtering Architecture

## Overview

The graph visualization supports a flexible, composable filtering system that allows multiple independent filter types to work together. This architecture was designed to accommodate future filter requirements without requiring significant refactoring.

## Design Principles

### 1. Filter Composition

Multiple filters can be active simultaneously. Each filter type has its own semantics:

- **Strategic Filter**: HIDES non-strategic nodes completely (milestone/epic labels only)
- **Label Filter**: DIMS nodes that don't match patterns (preserves graph context)

This distinction is intentional:
- Strategic view fundamentally changes the graph structure (different abstraction level)
- Label filtering is a search/focus mechanism (keeps context visible but de-emphasized)

### 2. Separation of Concerns

The filtering system is split into three layers:

```
┌─────────────────────────────────────┐
│  UI Layer (GraphView.tsx)           │  ← React component
│  - Manages filter state             │
│  - Renders nodes/edges              │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  Filter Application                 │  ← Pure functions
│  (graphFilter.ts)                   │
│  - applyFiltersToNode()             │
│  - applyFiltersToEdge()             │
│  - Filter composition logic         │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  Filter Configuration               │  ← Data structures
│  (GraphFilter types)                │
│  - StrategicFilterConfig            │
│  - LabelFilterConfig                │
└─────────────────────────────────────┘
```

### 3. Extensibility

Adding a new filter type requires:

1. Define config type (e.g., `StatusFilterConfig`)
2. Add to `GraphFilter` union type
3. Implement logic in `applyFiltersToNode()`
4. Create factory function (e.g., `createStatusFilter()`)

**No changes needed to:**
- GraphView component (already generic)
- Edge filtering logic (automatically uses node results)
- Existing filter types

## Implementation Details

### Filter Results

Each node evaluation returns:

```typescript
interface NodeFilterResult {
  visible: boolean;    // Should node be in the graph at all?
  dimmed: boolean;     // Should node be grayed out?
  reason?: string;     // Why filtered (for debugging)
}
```

**Rules:**
- If ANY filter hides a node → `visible = false`
- If ANY filter dims a node → `dimmed = true`
- Hidden takes precedence over dimmed

### Edge Filtering

Edges automatically inherit filter state from their endpoints:

```typescript
interface EdgeFilterResult {
  visible: boolean;    // Hide if either node is hidden
  dimmed: boolean;     // Dim if either node is dimmed
}
```

This ensures visual consistency without manual edge configuration.

### Visual Styling

**Dimmed nodes:**
- `opacity: 0.5` on entire node container
- `opacity: 0.4` on node content
- Smooth `transition: opacity 0.2s ease` for UX

**Dimmed edges:**
- `opacity: 0.3` on edge line
- Different color: `var(--border)` vs `var(--border-hover)`
- Matching transition timing

## Usage Examples

### Example 1: Strategic View Only

```typescript
const filters = [createStrategicFilter(true)];
// Result: Only milestone/epic nodes visible, others hidden
```

### Example 2: Label Filtering Only

```typescript
const filters = [createLabelFilter(['milestone:v1.0', 'epic:*'])];
// Result: All nodes visible, non-matching nodes dimmed
```

### Example 3: Combined Filters

```typescript
const filters = [
  createStrategicFilter(true),
  createLabelFilter(['milestone:v1.0'])
];
// Result: 
// - Non-strategic nodes hidden (strategic filter)
// - Strategic nodes not matching label dimmed (label filter)
```

### Example 4: Wildcard Patterns

```typescript
const filters = [createLabelFilter(['milestone:*', 'epic:*'])];
// Result: Dims nodes without any milestone or epic labels
```

## Pattern Matching

Label patterns support:

- **Exact match**: `milestone:v1.0` → matches only that label
- **Wildcard**: `milestone:*` → matches any milestone label
- **Multiple patterns** (OR logic): `['milestone:*', 'epic:*']` → matches either

Implementation in `matchesPattern()`:

```typescript
if (pattern.endsWith('*')) {
  const prefix = pattern.slice(0, -1);
  return label.startsWith(prefix);
}
return label === pattern;
```

## Testing Strategy

The filter system has comprehensive test coverage:

### Unit Tests (28 tests in `graphFilter.test.ts`)

- Pattern matching (exact, wildcard, edge cases)
- Single filter types (strategic, label)
- Filter composition (strategic + label)
- Edge filtering logic

### Integration Tests (9 tests in `LabelFilter.test.tsx`)

- UI component rendering
- User interactions (add, remove, clear)
- Autocomplete and suggestions
- Wildcard input

### Component Tests (6 tests in `GraphView.test.tsx`)

- Props acceptance (viewMode, labelFilters)
- Filter combination rendering

**Total: 43 new tests, all passing**

## Performance Considerations

### Filter Evaluation

- O(n) for node filtering (unavoidable)
- O(m) for edge filtering (unavoidable)
- Pattern matching is O(1) per label (string prefix check)

### Optimization Opportunities

Current implementation re-evaluates all filters on every render. Future optimizations:

1. **Memoization**: Cache filter results per node ID
2. **Incremental updates**: Only re-filter changed nodes
3. **Index-based filtering**: Pre-compute label indices

**Current performance is acceptable** for typical graph sizes (<1000 nodes).

## Future Extensions

### Potential Filter Types

1. **State Filter**: Show only specific states (ready, in_progress, etc.)
2. **Priority Filter**: Highlight critical/high priority work
3. **Assignee Filter**: Focus on specific agent's work
4. **Date Filter**: Issues created/updated in date range
5. **Dependency Depth**: Show only nodes within N hops of selected node

### Implementation Guidance

For any new filter type:

1. **Consider semantics**: Should it hide or dim?
   - Hide: Changes graph structure (like strategic)
   - Dim: Focuses attention (like label filter)

2. **Define config type**:
   ```typescript
   interface MyFilterConfig {
     enabled: boolean;
     threshold?: number;
   }
   ```

3. **Add to union**:
   ```typescript
   export interface GraphFilter {
     type: 'strategic' | 'label' | 'my-filter';
     config: StrategicFilterConfig | LabelFilterConfig | MyFilterConfig;
   }
   ```

4. **Implement logic**:
   ```typescript
   if (filter.type === 'my-filter') {
     const config = filter.config as MyFilterConfig;
     if (config.enabled && !meetsThreshold(node, config.threshold)) {
       shouldDim = true;
     }
   }
   ```

5. **Write tests**: 
   - Pattern matching logic
   - Interaction with other filters
   - UI integration

## Related Files

- `web/src/utils/graphFilter.ts` - Core filter logic (150 lines)
- `web/src/utils/graphFilter.test.ts` - Unit tests (28 tests)
- `web/src/components/Labels/LabelFilter.tsx` - UI component (260 lines)
- `web/src/components/Graph/GraphView.tsx` - Integration point
- `docs/label-hierarchy-implementation-plan.md` - Context for label system

## References

- Functional programming approach: Pure functions, no side effects
- Type-driven design: TypeScript ensures correct filter composition
- TDD: All code written test-first with full coverage
