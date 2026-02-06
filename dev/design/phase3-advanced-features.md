# Phase 3: Advanced Features & Polish

**Issue:** 402d1a8f  
**Status:** Ready (Not Started)  
**Priority:** Optional

## Objective

Nice-to-have enhancements that build on Phase 1 (clustering) and Phase 2 (collapse/expand) to provide a production-quality DAG visualization experience.

## Context

Phase 1 and Phase 2 deliver a functional, usable cluster visualization system:
- ✅ Visual cluster containers grouping related work
- ✅ Hybrid grid+rank layout for efficient space usage
- ✅ Interactive collapse/expand for progressive disclosure
- ✅ Shareable view state via URL parameters

Phase 3 adds polish and advanced features based on user feedback and real-world usage patterns.

## Key Features

### 3.1 Edge Styling Improvements

**Goal:** Visual distinction and clarity for different edge types

**Bundled Edge Thickness:**
- Virtual edges (from collapsed containers) show aggregated count
- Edge width proportional to count: 1 edge = 1px, 5+ edges = 3px
- Badge on edge showing count when > 1
- Hover shows list of aggregated source edges

**Color Coding:**
- Cross-cluster edges: Different color (e.g., blue) vs internal (gray)
- Milestone→epic edges: Distinct style (maybe dashed?)
- Virtual edges: Lighter opacity to indicate aggregation
- Blocked/blocking edges: Red tint (if blocking gates visible)

**Path Highlighting:**
- Hover over edge: Highlight full dependency path
- Click node: Highlight all incoming/outgoing edges
- "Show path A→B" feature: Highlight minimal path between two nodes

**Implementation:**
```typescript
// Custom edge component with dynamic styling
const ClusterEdge = ({ data, ...props }) => {
  const { isVirtual, isCrossCluster, aggregatedCount } = data;
  const strokeWidth = Math.min(1 + Math.log2(aggregatedCount), 5);
  const color = isCrossCluster ? '#3b82f6' : '#9ca3af';
  const opacity = isVirtual ? 0.6 : 1.0;
  
  return (
    <BaseEdge
      {...props}
      style={{ strokeWidth, stroke: color, opacity }}
      label={aggregatedCount > 1 ? `${aggregatedCount}` : null}
    />
  );
};
```

**Tests:**
- [ ] Virtual edges show correct aggregated count
- [ ] Cross-cluster edges visually distinct
- [ ] Edge thickness scales with count
- [ ] Hover highlights dependency paths
- [ ] Edge labels render without overlap

### 3.2 Advanced Navigation Features

**Goal:** Help users explore and understand large graphs

**Minimap:**
- ReactFlow built-in minimap showing full graph structure
- Highlight collapsed clusters in minimap
- Click to zoom to region
- Current viewport rectangle shown

**Zoom to Fit:**
- "Zoom to fit cluster" button when cluster selected
- "Zoom to fit all" button in toolbar
- Animate zoom transitions smoothly
- Maintain aspect ratio

**Dependency Tracing:**
- "Follow dependency" tool: Click source, then target
- Highlights minimal path between A and B
- Shows all intermediate nodes and edges
- Option to expand collapsed clusters in path

**Search and Filter:**
- Search box: filter nodes by title/ID
- Matching nodes highlighted/zoomed
- "Show only matches" mode: Hide non-matching clusters
- Keyboard navigation: Tab through matches

**Implementation Priorities:**
1. Minimap (easy, use ReactFlow built-in)
2. Zoom to fit (medium, ReactFlow API)
3. Search/filter (medium, state + UI)
4. Dependency tracing (complex, deferred to 3.2.1)

**Tests:**
- [ ] Minimap shows correct graph overview
- [ ] Zoom to fit works for selected cluster
- [ ] Search highlights matching nodes
- [ ] Filter mode hides non-matching clusters

### 3.3 Performance Optimizations

**Goal:** Smooth rendering for graphs with 500-1000+ nodes

**Current Performance Baseline:**
- Phase 1: ~165 issues render smoothly
- Layout calculation: ~50-100ms
- ReactFlow render: ~100-200ms
- Acceptable for current scale

**Optimization Strategies (if needed):**

1. **Virtual Scrolling / Viewport Culling**
   - Only render nodes within viewport + margin
   - ReactFlow has some built-in culling
   - May need custom implementation for clusters

2. **Lazy Rendering of Collapsed Clusters**
   - Don't render internal nodes until expanded
   - Already handled by collapse/expand logic
   - Verify ReactFlow isn't rendering hidden nodes

3. **Memoization of Layout Calculations**
   - Cache layout results keyed by graph structure hash
   - Recompute only when nodes/edges change
   - Use `useMemo` for expensive calculations

4. **Web Worker for Layout**
   - Move `createClusterAwareLayout` to Web Worker
   - Avoids blocking UI during layout
   - Requires serializable data (no complex objects)
   - Return computed positions to main thread

**Implementation Notes:**
- Start with profiling to identify bottlenecks
- Don't optimize prematurely - wait for real performance issues
- Target: 60fps smooth panning/zooming, < 500ms layout time

**Tests:**
- [ ] Performance benchmarks: 500, 1000 node graphs
- [ ] Layout calculation < 500ms for 1000 nodes
- [ ] Smooth panning/zooming (60fps)
- [ ] No memory leaks on expand/collapse cycles

### 3.4 Export and Sharing

**Goal:** Share visualizations outside the web UI

**Export as Image:**
- "Export as PNG" button in toolbar
- "Export as SVG" for vector graphics
- Use ReactFlow's built-in `toPng()` / `toSvg()` utilities
- Include current viewport or full graph (user choice)
- Filename: `jit-graph-{date}.png`

**Shareable Links:**
- Already implemented in Phase 2 (URL params)
- Add "Copy link" button with confirmation
- QR code generation for mobile sharing (optional)

**Embed Mode:**
- `?embed=true` URL parameter
- Hides toolbar, simplifies UI
- For embedding in docs/wiki pages
- Read-only mode (no interactions)

**Export Metadata:**
- "Export graph data" as JSON
- Includes nodes, edges, cluster structure
- Can be imported later (future: import feature)

**Implementation:**
```typescript
// Export as PNG
const exportAsPng = useCallback(() => {
  const viewportNode = document.querySelector('.react-flow');
  toPng(viewportNode, {
    filter: (node) => !node.classList?.contains('react-flow__minimap'),
  }).then((dataUrl) => {
    const link = document.createElement('a');
    link.download = `jit-graph-${Date.now()}.png`;
    link.href = dataUrl;
    link.click();
  });
}, []);
```

**Tests:**
- [ ] Export PNG works, image contains graph
- [ ] Export SVG works, scalable
- [ ] Copy link button copies URL to clipboard
- [ ] Embed mode hides toolbar, simplifies UI

## Implementation Priority

Phase 3 features are **OPTIONAL** - implement based on user needs:

**High Priority (if needed):**
- 3.1.1: Basic edge styling (color coding, virtual edge count)
- 3.2.1: Minimap
- 3.4.1: Export as PNG

**Medium Priority:**
- 3.1.2: Path highlighting on hover
- 3.2.2: Search and filter
- 3.4.2: Shareable links enhancement (already 90% done in Phase 2)

**Low Priority / Future:**
- 3.1.3: Advanced path tracing (A→B minimal path)
- 3.3: Performance optimizations (wait for real bottlenecks)
- 3.4.3: Embed mode
- 3.4.4: JSON export/import

## Success Criteria (Optional)

Since Phase 3 is optional, success criteria depend on which features are implemented:

**If implementing 3.1 (Edge Styling):**
- [ ] Aggregated edges show count badges
- [ ] Cross-cluster edges visually distinct from internal edges
- [ ] Hover highlights dependency paths

**If implementing 3.2 (Navigation):**
- [ ] Minimap provides overview navigation
- [ ] Search highlights matching nodes across clusters
- [ ] Zoom to fit works smoothly

**If implementing 3.3 (Performance):**
- [ ] Graph renders smoothly with 500+ nodes
- [ ] Layout calculation < 500ms for large graphs
- [ ] No memory leaks or performance degradation

**If implementing 3.4 (Export):**
- [ ] Can export current view as image
- [ ] Shareable links include full view state
- [ ] Embed mode works for documentation

**All features:**
- [ ] All tests passing (TDD)
- [ ] Code review passed
- [ ] No regression in existing functionality

## Implementation Approach

**Incremental delivery:**
- Pick highest-value features based on user feedback
- Implement one sub-feature at a time
- Ship partial Phase 3 (e.g., just edge styling)
- Gather feedback, iterate

**Alternative approach:**
- Skip Phase 3 entirely if Phase 1+2 meet user needs
- Revisit based on production usage patterns
- Focus engineering effort on other priorities

## File Changes (Estimated)

**If implementing edge styling:**
- `web/src/components/Graph/edges/ClusterEdge.tsx` - Custom edge component
- `web/src/components/Graph/GraphView.tsx` - Wire up custom edges

**If implementing navigation:**
- `web/src/components/Graph/Minimap.tsx` - Minimap wrapper
- `web/src/components/Graph/SearchBar.tsx` - Search UI
- `web/src/components/Graph/GraphView.tsx` - Integration

**If implementing performance:**
- `web/src/workers/layoutWorker.ts` - Web Worker for layout
- `web/src/utils/clusterAwareLayout.ts` - Memoization

**If implementing export:**
- `web/src/utils/export.ts` - Export utilities
- `web/src/components/Graph/ExportButton.tsx` - UI component

## Risks & Mitigations

**Risk:** Feature creep - Phase 3 balloons in scope  
**Mitigation:** Strict prioritization, implement incrementally, ship partial features

**Risk:** Performance optimizations add complexity without benefit  
**Mitigation:** Profile first, optimize only proven bottlenecks

**Risk:** Advanced features confuse users  
**Mitigation:** Progressive disclosure, hide advanced features in menus

## Related Documentation

- `dev/design/subgraph-clustering-layout.md` - Overall design
- `dev/design/phase2-collapse-expand.md` - Phase 2 plan (prerequisite)
