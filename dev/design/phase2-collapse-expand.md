# Phase 2: Collapse/Expand Interactivity

**Issue:** 6f678db0  
**Status:** In Progress  
**Started:** 2026-02-06

## Objective

Add interactive collapse/expand functionality to cluster containers and individual nodes, enabling users to progressively disclose graph details and manage visual complexity for large DAGs (165+ issues).

## Foundation from Phase 1

Phase 1 already provides the infrastructure we'll build on:

- ✅ `ExpansionState` type definition (`web/src/types/subgraphCluster.ts`)
- ✅ `aggregateEdgesForCollapsed()` algorithm (`web/src/utils/subgraphClustering.ts`)
- ✅ Virtual edge rendering in GraphView
- ✅ Cluster boundary edge handling
- ✅ `ClusteredGraph` includes expansion state parameter

**What's missing:** UI interactions, state management, persistence

## Implementation Approach

### 2.1 Cluster Container Collapse/Expand

**Goal:** Click epic cluster header to hide/show all internal nodes (stories/tasks)

**UI Component Changes:**
- Modify cluster container node type to have clickable header
- Add collapse/expand icon (⊞ collapsed, ⊟ expanded) to header
- Show badge with hidden node count when collapsed
- Visual distinction for collapsed vs expanded state

**State Management:**
- React state: `expansionState: Record<string, boolean>`
- Initialize all clusters as expanded
- Toggle on header click
- Re-layout on state change (triggers `prepareClusteredGraphForReactFlow`)

**Edge Aggregation:**
- When cluster collapses, use existing `aggregateEdgesForCollapsed()`
- Virtual edges already handled by Phase 1 infrastructure
- Ensure cluster boundary edges show aggregated dependency count

**Tests:**
- [ ] Clicking cluster header toggles expansion state
- [ ] Collapsed cluster renders only epic node (no stories/tasks)
- [ ] Badge shows correct count of hidden nodes
- [ ] Virtual edges generated for collapsed cluster dependencies
- [ ] Re-expanding cluster shows original layout

### 2.2 Story-Level Collapse Within Clusters

**Goal:** Click story nodes to collapse/expand their child tasks

**UI Component Changes:**
- Story nodes get collapse/expand icon when they have tasks
- Similar visual treatment to cluster headers (but smaller scale)
- Show task count badge when collapsed

**Implementation Strategy:**
- Reuse same `expansionState` mechanism
- Story collapse works independently within expanded clusters
- Use `aggregateEdgesForCollapsed()` for story→task edge aggregation
- Nested collapse handled by existing algorithm (tested in Phase 1)

**Tests:**
- [ ] Clicking story node toggles its expansion
- [ ] Collapsed story hides all child tasks
- [ ] Collapsed story inherits task dependencies (virtual edges)
- [ ] Story collapse works when cluster is expanded
- [ ] Nested collapse: collapsing cluster with collapsed stories works

### 2.3 Expansion State Persistence

**Goal:** Share links with specific view state, persist across sessions

**URL Parameter Strategy:**
```typescript
// URL format: ?expanded=epic1,epic2&collapsed=story1,story2
// Or compressed: ?exp=epic1,epic2&col=story1,story2
// Or bitmap encoding for many nodes (future optimization)
```

**Implementation:**
1. **Serialize expansion state to URL params**
   - On state change, debounce and update URL
   - Use `useSearchParams` or similar
   - Default: all expanded (don't encode), only encode collapsed nodes
   - Consider compression for large state (50+ nodes)

2. **Restore state from URL on load**
   - Parse URL params in GraphView mount
   - Initialize `expansionState` from URL
   - Gracefully handle invalid/missing node IDs

3. **localStorage fallback (optional Phase 2.3.1)**
   - Save last view state per repository
   - Restore on next visit if no URL params
   - Key: `jit.graph.expansionState.${repoId}`

**Toolbar Actions:**
- "Expand All" button: set all expansionState values to true
- "Collapse All" button: collapse all clusters (maybe keep milestones expanded?)
- "Focus on Node" (future): expand path from root to selected node

**Tests:**
- [ ] URL updates when expansion state changes
- [ ] URL params restore expansion state on page load
- [ ] "Expand All" button expands everything
- [ ] "Collapse All" button collapses all clusters
- [ ] Shareable links work (copy/paste to new tab)
- [ ] Invalid node IDs in URL handled gracefully

### 2.4 Visual Polish for Collapse/Expand

**Goal:** Smooth, polished UX for expand/collapse interactions

**Animations:**
- Fade in/out for nodes appearing/disappearing
- Smooth position transitions (ReactFlow built-in)
- Edge fade transitions
- Duration: 200-300ms (fast enough, not jarring)

**Visual Indicators:**
- Collapse/expand icons: ⊞ (collapsed), ⊟ (expanded)
- Icon position: Top-right of cluster header
- Hover state: Highlight header, show tooltip "Click to expand/collapse"
- Collapsed clusters: Distinct background color/opacity
- Badge: Circular, shows count, positioned near icon

**Interaction Polish:**
- Click targets: Large enough (min 24x24px for icons)
- Cursor changes to pointer on hover
- Click doesn't propagate to ReactFlow drag
- Double-click protection (debounce)

**Tests:**
- [ ] Icons render correctly (collapsed vs expanded)
- [ ] Hover states work on cluster headers
- [ ] Animations don't cause layout flicker
- [ ] Click targets are accessible (not too small)
- [ ] Visual regression tests for cluster appearance

## Implementation Plan (TDD)

### Step 1: Basic Cluster Collapse (2.1)
1. **Test:** Click cluster header toggles expansion state
2. **Implement:** 
   - Add `expansionState` to GraphView state
   - Create custom cluster node component with clickable header
   - Wire onClick to toggle state
3. **Test:** Collapsed cluster hides internal nodes
4. **Implement:**
   - Pass `expansionState` to `prepareClusteredGraphForReactFlow`
   - Filter nodes based on expansion state
5. **Test:** Virtual edges generated for collapsed clusters
6. **Verify:** Existing `aggregateEdgesForCollapsed()` works correctly

### Step 2: Story Collapse (2.2)
1. **Test:** Story nodes can collapse independently
2. **Implement:**
   - Extend story node type with collapse icon
   - Add story IDs to `expansionState`
3. **Test:** Nested collapse (collapsed story in collapsed cluster)
4. **Verify:** Existing algorithm handles this (already tested in Phase 1)

### Step 3: URL Persistence (2.3)
1. **Test:** Expansion state serializes to URL
2. **Implement:**
   - Create `serializeExpansionState()` utility
   - Use `useSearchParams` to update URL on state change
3. **Test:** URL params restore state on load
4. **Implement:**
   - Parse URL in useEffect, initialize state
5. **Test:** Toolbar actions work
6. **Implement:**
   - "Expand All" / "Collapse All" buttons

### Step 4: Visual Polish (2.4)
1. **Implement:** Custom cluster node component with styled header
2. **Implement:** Collapse/expand icons (⊞/⊟)
3. **Implement:** Badge showing hidden node count
4. **Implement:** CSS animations for expand/collapse
5. **Test:** Visual regression tests (optional)

## File Changes

**New files:**
- `web/src/components/Graph/nodes/ClusterNode.tsx` - Custom cluster container node
- `web/src/components/Graph/nodes/StoryNode.tsx` - Story node with collapse capability (optional, might extend default)
- `web/src/utils/expansionState.ts` - URL serialization utilities
- `web/src/utils/expansionState.test.ts` - Tests for serialization

**Modified files:**
- `web/src/components/Graph/GraphView.tsx` - Add expansion state management, toolbar
- `web/src/utils/clusteredGraphLayout.ts` - May need minor updates for collapse handling
- `web/src/components/Graph/GraphView.css` - Styles for cluster headers, badges, animations

## Success Criteria

- [ ] Clicking cluster header toggles collapse/expand
- [ ] Collapsed clusters show badge with hidden node count
- [ ] Virtual edges correctly aggregate when cluster collapses
- [ ] Story-level collapse works within expanded clusters
- [ ] Expansion state persists in URL (shareable)
- [ ] "Expand all" / "Collapse all" buttons work
- [ ] All tests passing (TDD)
- [ ] Code review passed
- [ ] No performance regression (collapse/expand < 100ms)

## Out of Scope (Phase 3)

- Advanced animations (spring physics, stagger)
- "Focus on node" feature
- Keyboard shortcuts for expand/collapse
- Minimap showing collapsed/expanded state
- localStorage persistence (may include if easy)

## Risks & Mitigations

**Risk:** ReactFlow re-layout on collapse causes jarring jumps  
**Mitigation:** Use ReactFlow's built-in animation system, ensure position changes are smooth

**Risk:** URL params get too long with many collapsed nodes  
**Mitigation:** Default to "all expanded", only encode collapsed nodes; consider compression

**Risk:** Virtual edges don't render correctly after collapse  
**Mitigation:** Extensive testing of `aggregateEdgesForCollapsed()` (already done in Phase 1)

## Timeline Estimate

This is a reference only, not a commitment:
- Step 1 (Basic cluster collapse): ~2-3 hours
- Step 2 (Story collapse): ~1-2 hours
- Step 3 (URL persistence): ~2-3 hours
- Step 4 (Visual polish): ~2-3 hours
- **Total:** ~7-11 hours of focused work

## Related Documentation

- `dev/design/subgraph-clustering-layout.md` - Overall clustering design
- Phase 1 implementation (completed) - Foundation this builds on
