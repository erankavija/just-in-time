# Search Result Focus Navigation - Implementation Plan

## Issue
f527996e - Add search result focus navigation to DAG

## Objective
When a search result is clicked, automatically navigate to the issue in the DAG by:
1. Centering the viewport on the target node
2. Expanding all collapsed clusters in the path to the issue
3. Highlighting the focused node for visual feedback

## Current State Analysis

### Search Implementation
- **Location**: `SearchBar` in App.tsx with `useSearch` hook
- **Search types**: Client-side (instant) + server-side (debounced, 300ms)
- **Result types**: Issues and Documents
- **Current click behavior**: Sets `selectedIssueId` → shows detail panel (no graph navigation)

### Graph Implementation  
- **Component**: `GraphView.tsx` (~980 lines)
- **Props**: No way to trigger focus from parent currently
- **Viewport control**: Has `reactFlowInstanceRef.current.fitView()` already used for layout changes
- **Expansion state**: Managed internally with `useState<ExpansionState>`
- **Node highlighting**: No current implementation

## Implementation Plan

### Phase 1: Add Focus API to GraphView
**Goal**: Enable parent components to trigger focus on specific nodes

**Tasks**:
- Add `focusNodeId` prop to GraphView interface
- Add `onFocusComplete` callback prop (optional)
- Create `focusOnNode()` function that:
  - Finds all parent clusters containing the target node
  - Updates expansion state to expand all parent clusters
  - Centers viewport on the target node using `fitView({ nodes: [node] })`
  - Returns boolean success/failure
- Add useEffect to watch `focusNodeId` prop changes
- Write tests for focus logic

**Files to modify**:
- `web/src/components/Graph/GraphView.tsx`

### Phase 2: Add Node Highlighting
**Goal**: Provide visual feedback when a node is focused

**Tasks**:
- Add `highlightedNodeId` state to GraphView
- Add highlight styling to node rendering (bright border, subtle glow)
- Auto-clear highlight after 2-3 seconds
- Ensure highlight works for both cluster and regular nodes
- Add CSS for highlight effect (no animation, just static highlight)

**Files to modify**:
- `web/src/components/Graph/GraphView.tsx` - state and rendering
- `web/src/components/Graph/nodes/ClusterNode.tsx` - accept highlight prop
- `web/src/components/Graph/nodes/ClusterNode.css` - highlight styles

### Phase 3: Integrate Focus with Search
**Goal**: Wire up search result clicks to trigger graph focus

**Tasks**:
- Add `focusIssueId` state to App.tsx
- Pass `focusNodeId={focusIssueId}` prop to GraphView
- Update search result click handler:
  - Set `selectedIssueId` (existing behavior)
  - Set `focusIssueId` (new behavior)
  - Clear search query (existing behavior)
- Clear `focusIssueId` after focus completes (via callback)
- Test with various graph states (collapsed/expanded)

**Files to modify**:
- `web/src/App.tsx`

### Phase 4: Testing & Edge Cases
**Goal**: Ensure robust behavior in all scenarios

**Test cases**:
- Focus on visible node (no expansion needed)
- Focus on node in collapsed epic cluster
- Focus on node in collapsed story within expanded epic
- Focus on node not in current graph (filtered out)
- Focus on document search result with `issue_id`
- Multiple rapid focus requests (debouncing)
- Focus while graph is still loading
- Visual test: Highlight visible and clear

**Test strategy**: Add integration tests to GraphView.test.tsx

## Technical Details

### Finding Parent Clusters
```typescript
function findParentClusters(
  nodeId: string, 
  clusterData: SubgraphCluster[]
): string[] {
  const parents: string[] = [];
  
  function findInCluster(cluster: SubgraphCluster): boolean {
    // Check direct children
    if (cluster.children.some(c => c.id === nodeId)) {
      parents.push(cluster.containerId);
      return true;
    }
    
    // Check nested clusters
    for (const child of cluster.children) {
      if (child.type === 'cluster' && findInCluster(child)) {
        parents.push(cluster.containerId);
        return true;
      }
    }
    
    return false;
  }
  
  clusterData.forEach(findInCluster);
  return parents;
}
```

### Viewport Centering
```typescript
// ReactFlow provides multiple options:
reactFlowInstance.fitView({ 
  nodes: [{ id: nodeId }],  // Focus on specific node
  duration: 300,            // Smooth animation
  padding: 0.2,             // Add padding around node
});
```

### Highlight Styling
```css
.react-flow__node.highlighted {
  box-shadow: 0 0 0 3px var(--accent),
              0 0 12px var(--accent);
  z-index: 1000;
}
```

**Note**: No pulse animation - just a static highlight that appears and disappears after timeout.

## Success Criteria

- Clicking issue search result centers graph on that issue
- All parent clusters auto-expand to reveal the issue
- Focused node highlights briefly for visual feedback
- Works for both epic-level and story-level collapsed clusters
- Handles edge cases gracefully (node not found, already visible)
- No performance degradation (focus completes in <500ms)
- All existing tests still pass
- New integration tests cover focus scenarios

## Future Enhancements (Out of Scope)

- Keyboard shortcut to focus selected issue (e.g., 'f' key)
- "Pan to issue" option in issue context menu
- Breadcrumb trail showing expansion path
- Animate expansion sequence (parents expand one by one)
- Zoom level adjustment based on node depth

## Estimated Effort

- Phase 1: 2-3 hours (focus API + parent cluster finding)
- Phase 2: 1-2 hours (highlighting)
- Phase 3: 1 hour (integration)
- Phase 4: 2 hours (testing)

**Total: ~6-8 hours**
