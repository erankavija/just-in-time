# Implementation Plan: Hierarchy-Aware Subgraph Clustering Layout

## Problem Statement

Current DAG visualization renders all 165 issues in a single flat graph, making it unreadable. We need a layout that:
- Uses configurable hierarchy levels to create visual structure
- Groups lower-level work into collapsible subgraphs under higher-level containers
- Preserves ALL dependency edges (within and across subgraphs)
- Shows temporal/dependency flow left-to-right

## Proposed Solution

**Two-tier clustering layout with collapsible containers:**

1. **Level 0 nodes** (e.g., milestones): Horizontal timeline, left-to-right
2. **Level 1 nodes** (e.g., epics): Subgraph clusters positioned LEFT of their parent Level 0 node
3. **Level 2+ nodes** (e.g., stories, tasks): Inside Level 1 subgraphs, collapsible
4. **Edge aggregation**: When containers collapse, edges "bubble up" to show dependencies

### Visual Example
```
┌─ Epic A ──────┐              ┌─ Epic B ──────┐
│ ⊞ Story A1 ───┼──────────────┼─▶ ⊟ Story B1  │    ┌─ Milestone v1.0 ─┐
│   (collapsed) │              │   ├─ Task B1  │───▶│                   │
│               │              │   └─ Task B2  │    └───────────────────┘
│ ⊟ Story A2    │              │               │             │
│  ├─ Task A3   │              │               │             ▼
│  └─ Task A4 ──┼──────────────┼──▶ Task B1    │    ┌─ Epic C ──────────┐
└───────────────┘              └───────────────┘    │ ⊟ Story C1         │
                                                    │  ├─ Task C1         │
                                                    │  └─ Task C2         │
                                                    └────────────────────┘
```

## Key Design Principles

### 1. Hierarchy Agnostic
- Read hierarchy from `config.toml` → `type_hierarchy.types`
- Example: `{ milestone = 1, epic = 2, story = 3, task = 4, bug = 4 }`
- Level numbers define structure (lower = more strategic)
- Type names are configurable, not hardcoded

### 2. Subgraph Assignment Algorithm
```
For each Level 1 node (e.g., epic):
  1. Start with Level 1 node as root
  2. Follow dependencies downward (to higher level numbers)
  3. Add ALL nodes with level > 1 to this subgraph
  4. STOP when encountering level ≤ 1 (another epic/milestone)
  5. These cross-level edges remain but don't pull nodes into subgraph
```

### 3. Edge Preservation
- **Within subgraph**: Task→Task, Story→Task edges drawn normally
- **Cross subgraph**: Epic A's Task → Epic B's Task preserves edge
- **Collapsed containers**: Story (collapsed) inherits ALL child edges

### 4. Collapsible State Management
- Stories (Level 2) can collapse/expand
- Epics (Level 1) can collapse/expand (hides all children)
- Expansion state stored in React state (later: URL/localStorage)

### 5. Edge Aggregation on Collapse
When Story S is collapsed:
```typescript
virtualEdges = []
for each child C of S:
  for each edge C→X where X is visible:
    add edge S→X to virtualEdges
  for each edge Y→C where Y is visible:
    add edge Y→S to virtualEdges
```

## Implementation Workplan

### Phase 1: Core Data Structures & Algorithms ✅ COMPLETE
**Status:** All steps complete, issue 783a8086 marked DONE (2026-02-06)

- [x] **1.1** Define TypeScript types for hierarchy-aware graph ✅
  - `HierarchyLevelMap` - maps type names to numeric levels
  - `SubgraphCluster` - contains nodes, edges, level info
  - `ExpansionState` - tracks collapsed/expanded containers
  - `VirtualEdge` - aggregated edges from collapsed nodes
  - `ClusteredGraph` - result type with clusters, cross-cluster edges, orphans
  - **Files:** `web/src/types/subgraphCluster.ts`

- [x] **1.2** Implement hierarchy derivation from config ✅
  - `extractNodeType(node)` - extracts type from type:X label
  - `getNodeLevel(node, hierarchy)` - determines node's level from type label
  - Handle multiple types per level (task=4, bug=4)
  - Returns Infinity for unknown/missing types
  - **Tests:** 9 tests covering various configs, edge cases
  - **Files:** `web/src/utils/subgraphClustering.ts`

- [x] **1.3** Implement subgraph assignment algorithm ✅
  - `assignNodesToSubgraphs(nodes, edges, hierarchy)` 
  - For each lowest-level node, collect its lower-level dependents via BFS
  - Stop at hierarchy boundaries (don't cross to same/lower level)
  - Returns clusters + cross-cluster edges + orphan nodes
  - **Tests:** 5 tests (single epic, multiple epics, cross-cluster edges, hierarchy boundaries, stories)
  - **Files:** `web/src/utils/subgraphClustering.ts`

- [x] **1.4** Implement edge aggregation for collapsed nodes ✅
  - `buildContainerMap()` - builds parent-child relationships from deps
  - `aggregateEdgesForCollapsed(nodes, edges, expansionState)`
  - Bubble up edges from hidden children to visible parent
  - Handle multi-level collapse (Story collapsed inside Epic collapsed)
  - Aggregate multiple edges to same target (count, sourceEdgeIds)
  - **Tests:** 5 tests (single collapse, multiple aggregation, incoming edges, nested collapse)
  - **Files:** `web/src/utils/subgraphClustering.ts`

- [x] **1.5** Cluster-aware layout algorithm ✅
  - `createClusterAwareLayout()` - main layout orchestrator
  - `computeClusterPositions()` - hybrid grid+rank positioning
  - `layoutNodesWithinCluster()` - dagre layout within each cluster
  - **Layout features:**
    - Epic-level visual container boxes
    - Hybrid grid (rank 0) + ranked columns (rank > 0) for efficient 2D space
    - Grid: up to 5 columns, sqrt-based arrangement for independent clusters
    - Ranked: positioned RIGHT of grid based on dependency depth
    - Proper left→right dependency flow (B on left, A depends on B on right)
    - Deterministic, stable positioning (no shuffling across re-renders)
    - Cluster boundary edges (task→cluster mapping) for accurate positioning
    - Milestone positioning (most dependents = leftmost)
  - **Code quality:**
    - Modular design: extracted `buildEdgeMaps`, `calculateRanks`, `adjustGridColumns`
    - 508 lines, well-factored functions (largest: 141 lines)
    - All 133 tests passing
    - Zero TypeScript errors, zero compiler warnings
  - **Files:** `web/src/utils/clusterAwareLayout.ts`, `web/src/components/Graph/GraphView.tsx`
  - **Commits:** bb3b566, 82470cc, refactoring 2026-02-06

- [x] **1.6** Integration with GraphView ✅
  - "cluster-aware" layout option in GraphView
  - Works with existing ReactFlow infrastructure
  - All edge types rendering: internal, cross-cluster, virtual, cluster boundary
  - Handles expansion state (foundation for Phase 2 collapse/expand)
  - **Result:** 22 clusters rendered (18 in grid, 4 ranked)
  
- [x] **1.7** Integration test with real repo data ✅
  - Created `clusteredGraphLayout.integration.test.ts` with 6 comprehensive tests
  - Tests simulate Epic ad601a15 structure (55 deps, multiple stories/tasks)
  - Tests story collapse, nested collapse, milestone hierarchies, cross-cluster edges, orphans
  - **Status:** All integration tests passing
  - **Files:** `web/src/utils/clusteredGraphLayout.integration.test.ts`

**Phase 1 Deliverables (COMPLETE):**
- ✅ Visual cluster containers (epic-level boxes)
- ✅ All edge types rendering correctly (internal, cross-cluster, virtual, boundary)
- ✅ Hybrid grid+rank layout for efficient 2D space usage
- ✅ Proper dependency flow (left-to-right temporal)
- ✅ Stable, deterministic layout (no shuffling)
- ✅ Works with real repository data (165+ issues)
- ✅ Clean, modular, well-tested code
- ✅ All quality gates passed (TDD, code review)

### Phase 2: Collapse/Expand Interactivity 📋 PLANNED
**Status:** Phase 1 provides foundation - expansion state handling already integrated

**Scope:** Add interactive collapse/expand functionality to cluster containers and individual nodes within clusters.

- [ ] **2.1** Implement cluster container collapse/expand
  - Click epic container header to toggle expanded/collapsed state
  - Collapsed: Show only epic node, hide all internal nodes (stories/tasks)
  - Virtual edges: Aggregate all child dependencies to cluster boundary
  - Visual indicator: Badge showing hidden node count when collapsed
  - Tests: cluster collapse, virtual edge generation

- [ ] **2.2** Implement story-level collapse within clusters
  - Click story nodes to collapse/expand their tasks
  - Collapsed story inherits all task dependencies
  - Expanded shows full task breakdown
  - Tests: nested collapse (story in epic), edge aggregation

- [ ] **2.3** Expansion state persistence
  - Save expansion state to URL params (shareable links)
  - Consider localStorage fallback for persistence across sessions
  - "Expand all" / "Collapse all" toolbar buttons
  - "Focus on node" - expand path to selected node
  - Tests: URL param serialization, restore state

- [ ] **2.4** Visual polish for collapse/expand
  - Smooth animations for expand/collapse transitions
  - Collapse/expand icons (⊞/⊟) on container headers
  - Hover states for interactive elements
  - Different node styles for collapsed vs expanded containers
  - Tests: visual regression tests

### Phase 3: Advanced Features & Polish 📋 FUTURE
**Status:** Nice-to-have features for enhanced UX

- [ ] **3.1** Edge styling improvements
  - Bundled edge thickness based on aggregated count (virtual edges)
  - Color coding for cross-cluster vs internal edges
  - Hover to highlight full dependency paths
  - Edge labels showing dependency count when aggregated
  - Tests: edge rendering, aggregation counts

- [ ] **3.2** Advanced navigation features
  - Minimap showing full graph structure
  - Zoom to fit selected cluster
  - "Follow dependency" - trace path from A to B
  - Search/filter to highlight matching nodes
  - Tests: navigation actions

- [ ] **3.3** Performance optimizations
  - Virtual scrolling for large graphs (1000+ nodes)
  - Lazy rendering of collapsed cluster contents
  - Memoization of layout calculations
  - Web worker for heavy layout computation
  - Tests: performance benchmarks

- [ ] **3.4** Export and sharing
  - Export graph as SVG/PNG
  - Share link with specific view state (zoom, expansion, filters)
  - Embed mode for documentation
  - Tests: export functionality

## Technical Notes

### Hierarchy Configuration
From `.jit/config.toml`:
```toml
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4, bug = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"
story = "story"
```

**Key insights:**
- Numeric levels are authoritative (not type names)
- Multiple types can share same level (task=4, bug=4)
- Label associations are optional (used for grouping, not structure)

### API Endpoints Needed
- `GET /api/config/hierarchy` - fetch hierarchy config
  - Returns: `{ types: {...}, strategic_types: [...], label_associations: {...} }`
  - Already implemented in jit-server (verify endpoint)

### Edge Cases to Handle
1. **Nodes without type labels** - assign to highest level (treat as leaf)
2. **Orphaned nodes** - no Level 0/1 parent - render in separate "Ungrouped" cluster
3. **Cycles in hierarchy** - shouldn't happen (DAG invariant) but validate
4. **Empty subgraphs** - Level 1 node with no children - render as simple node
5. **Deeply nested collapse** - Epic collapsed → all stories/tasks hidden

### Testing Strategy
- **Unit tests**: Each algorithm in isolation (TDD)
- **Integration tests**: Full layout pipeline with realistic data
- **Visual regression**: Screenshot tests for layout correctness
- **Performance**: Benchmark with 165-node graph from real repo

## Success Criteria
- [ ] All 165 issues render in structured, readable layout
- [ ] Collapsing stories/epics reduces visible nodes significantly
- [ ] ALL dependency edges remain visible (no information loss)
- [ ] Temporal flow (left→right) is preserved
- [ ] Works with ANY hierarchy config (not hardcoded to milestone/epic)
- [ ] All tests passing (unit + integration)
- [ ] No performance degradation vs. current dagre layout

## Non-Goals (Future Work)
- Custom edge bundling visualization (Phase 3 from original plan)
- Fisheye/focus+context techniques
- Animated transitions between collapse states
- Server-side layout computation
