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

### Phase 1: Core Data Structures & Algorithms
- [ ] **1.1** Define TypeScript types for hierarchy-aware graph
  - `HierarchyLevel` - maps type names to numeric levels
  - `SubgraphCluster` - contains nodes, edges, level info
  - `ExpansionState` - tracks collapsed/expanded containers
  - `VirtualEdge` - aggregated edges from collapsed nodes

- [ ] **1.2** Implement hierarchy derivation from config
  - `deriveHierarchyLevels(config)` - extracts level mapping
  - `getNodeLevel(node, hierarchy)` - determines node's level from type label
  - Handle multiple types per level (task=4, bug=4)
  - Tests: various config.toml structures

- [ ] **1.3** Implement subgraph assignment algorithm
  - `assignNodesToSubgraphs(nodes, edges, hierarchy)` 
  - For each Level 1 node, collect its lower-level dependents
  - Stop at hierarchy boundaries (don't cross to same/lower level)
  - Tests: verify correct clustering, cross-cluster edges preserved

- [ ] **1.4** Implement edge aggregation for collapsed nodes
  - `aggregateEdgesForCollapsed(subgraph, expansionState)`
  - Bubble up edges from hidden children to visible parent
  - Handle multi-level collapse (Story collapsed inside Epic collapsed)
  - Tests: various collapse scenarios

### Phase 2: Layout Engine
- [ ] **2.1** Implement Level 0 horizontal positioning
  - Sort Level 0 nodes by temporal order (milestone labels, dependency order)
  - Position left-to-right with fixed spacing
  - Tests: milestone ordering

- [ ] **2.2** Implement Level 1 subgraph clustering
  - Position each Level 1 cluster to LEFT of its parent Level 0 node
  - Vertical packing of multiple Level 1 nodes per Level 0
  - Allocate space based on subgraph size (collapsed vs expanded)
  - Tests: cluster positioning, space allocation

- [ ] **2.3** Implement intra-subgraph layout
  - Use dagre for positioning nodes WITHIN each subgraph
  - Handle collapsed containers (render as single node)
  - Tests: verify internal structure is readable

- [ ] **2.4** Implement cross-subgraph edge routing
  - Draw edges that span subgraphs
  - Edge bundling for collapsed nodes (multiple edges → one thick edge)
  - Tests: edge routing correctness

### Phase 3: ReactFlow Integration
- [ ] **3.1** Create subgraph container components
  - `SubgraphCluster` component - visual boundary for Level 1 nodes
  - Render title, collapse/expand button
  - Handle click to toggle expansion state

- [ ] **3.2** Implement expansion state management
  - React state for tracking collapsed/expanded nodes
  - `toggleExpansion(nodeId)` - collapse/expand handler
  - Re-layout on state change

- [ ] **3.3** Wire up layout to ReactFlow
  - Transform subgraph data → ReactFlow nodes/edges
  - Custom node types for containers vs leaf nodes
  - Edge rendering with proper markers

- [ ] **3.4** Add GraphView toggle for new layout
  - Add "clustered" option to layout algorithm selector
  - Apply new layout when selected
  - Fallback to dagre when hierarchy unavailable

### Phase 4: Polish & UX
- [ ] **4.1** Visual hierarchy indicators
  - Different node styles for Level 0/1/2/3+
  - Collapse/expand icons (⊞/⊟)
  - Badge showing hidden node count when collapsed

- [ ] **4.2** Edge styling improvements
  - Bundled edge thickness based on aggregated count
  - Color coding for cross-cluster edges
  - Hover to highlight dependency paths

- [ ] **4.3** Persistence & Navigation
  - Save expansion state to URL params
  - "Expand all" / "Collapse all" buttons
  - "Focus on node" - expand path to selected node

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
