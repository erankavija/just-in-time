import type { GraphNode, GraphEdge } from '../types/models';
import type {
  HierarchyLevelMap,
  SubgraphCluster,
  ClusteredGraph,
  ExpansionState,
  VirtualEdge
} from '../types/subgraphCluster';
import { resolveHierarchy, type HierarchyInputNode } from './hierarchyResolution';

/**
 * Adapt the web graph (nodes + a separate edge list) into the input shape the
 * canonical resolver consumes: each node carries its `type:` value and its
 * outgoing dependency ids (edge `from → to` means "from depends on to").
 * `resolveHierarchy` ignores dep ids outside the node set, so passing every
 * edge target is safe.
 */
function toHierarchyInput(nodes: GraphNode[], edges: GraphEdge[]): HierarchyInputNode[] {
  const depsByNode = new Map<string, string[]>();
  for (const edge of edges) {
    const deps = depsByNode.get(edge.from);
    if (deps) deps.push(edge.to);
    else depsByNode.set(edge.from, [edge.to]);
  }
  return nodes.map((node) => ({
    id: node.id,
    type: extractNodeType(node),
    dependencies: depsByNode.get(node.id) ?? [],
  }));
}

/**
 * Get all unique hierarchy levels present in the graph, sorted from strategic to tactical.
 * @param nodes - Graph nodes
 * @param hierarchy - Hierarchy level mapping
 * @returns Array of levels sorted ascending (e.g., [1, 2, 3, 4])
 */
export function getHierarchyLevels(nodes: GraphNode[], hierarchy: HierarchyLevelMap): number[] {
  const levels = nodes
    .map(n => getNodeLevel(n, hierarchy))
    .filter(level => level !== Infinity);
  
  return [...new Set(levels)].sort((a, b) => a - b);
}

/**
 * Extract the node type from type:X label.
 * @param node - The node to extract type from
 * @returns The type name (e.g., 'task', 'epic') or null if no type label
 */
export function extractNodeType(node: GraphNode): string | null {
  const typeLabel = node.labels.find((l) => l.startsWith('type:'));
  if (!typeLabel) return null;
  return typeLabel.substring(5); // Remove 'type:' prefix
}

/**
 * Get the hierarchy level for a node based on its type.
 * @param node - The node to get level for
 * @param hierarchy - Hierarchy level mapping from config
 * @returns Numeric level (1 = most strategic), or Infinity if no type/unknown type
 */
export function getNodeLevel(node: GraphNode, hierarchy: HierarchyLevelMap): number {
  const nodeType = extractNodeType(node);
  if (!nodeType) return Infinity;
  
  const level = hierarchy[nodeType];
  return level !== undefined ? level : Infinity;
}

/**
 * Assign nodes to subgraph clusters based on hierarchy boundaries.
 * This is a convenience wrapper that automatically determines the container level
 * using the strategy: prefer level 2 (epic) if it exists, otherwise use lowest level.
 * 
 * For explicit control over container level, use assignNodesToClusters() directly.
 * 
 * @param nodes - All nodes in the graph
 * @param edges - All edges in the graph
 * @param hierarchy - Hierarchy level mapping from config
 * @returns Clustered graph with nodes assigned to clusters
 */
export function assignNodesToSubgraphs(
  nodes: GraphNode[],
  edges: GraphEdge[],
  hierarchy: HierarchyLevelMap
): ClusteredGraph {
  // Find all unique levels present in the graph
  const nodeLevels = nodes.map(n => ({ node: n, level: getNodeLevel(n, hierarchy) }))
    .filter(({ level }) => level !== Infinity);
  
  if (nodeLevels.length === 0) {
    return { clusters: new Map(), crossClusterEdges: [], orphanNodes: nodes };
  }
  
  const uniqueLevels = [...new Set(nodeLevels.map(nl => nl.level))].sort((a, b) => a - b);
  
  // Container level selection strategy:
  // - Prefer level 2 (typically "epic") if it exists
  // - Fall back to lowest level if level 2 doesn't exist
  // - This allows milestones (level 1) to be visible nodes, not containers
  const containerLevel = uniqueLevels.includes(2) ? 2 : uniqueLevels[0];
  
  // Delegate to generic clustering function
  return assignNodesToClusters(nodes, edges, hierarchy, containerLevel);
}

/**
 * Assign nodes to clusters at a specific hierarchy level.
 *
 * Presentation as a pure projection of the canonical resolution: cluster
 * membership is DERIVED from {@link resolveHierarchy} (the single source of
 * containment facts, itself verified against the jit core), not from a parallel
 * traversal. A node belongs to the container it reaches by walking up its
 * canonical parent chain to the nearest ancestor at exactly `containerLevel`
 * (e.g. the nearest epic-level ancestor). Nodes with no such ancestor are
 * orphans; the container node itself owns its own cluster.
 *
 * @param nodes - Nodes to cluster
 * @param edges - Edges between these nodes (`from → to` = "from depends on to")
 * @param hierarchy - Hierarchy level mapping
 * @param containerLevel - The hierarchy level to use as containers (e.g., 2 for epic, 3 for story)
 * @returns Clustered graph with containers and their children
 */
export function assignNodesToClusters(
  nodes: GraphNode[],
  edges: GraphEdge[],
  hierarchy: HierarchyLevelMap,
  containerLevel: number
): ClusteredGraph {
  // Find all nodes at the container level (e.g., stories/epics).
  const containerNodes = nodes.filter(n => getNodeLevel(n, hierarchy) === containerLevel);

  if (containerNodes.length === 0) {
    return { clusters: new Map(), crossClusterEdges: [], orphanNodes: nodes };
  }

  // Canonical containment facts for the whole set.
  const resolution = resolveHierarchy(toHierarchyInput(nodes, edges), hierarchy);
  const levelById = new Map(nodes.map(n => [n.id, getNodeLevel(n, hierarchy)]));

  // The container that owns `id` at this level: walk up the canonical parent
  // chain to the nearest ancestor whose level equals `containerLevel`.
  const ownerAtLevel = (id: string): string | null => {
    let cur: string | null = id;
    const seen = new Set<string>();
    while (cur !== null && !seen.has(cur)) {
      if (levelById.get(cur) === containerLevel) return cur;
      seen.add(cur);
      cur = resolution.get(cur)?.parent ?? null;
    }
    return null;
  };

  // Group nodes by owning container. Seed every container (container first) so a
  // childless container still forms a cluster.
  const members = new Map<string, GraphNode[]>();
  for (const container of containerNodes) members.set(container.id, [container]);
  const assignment = new Map<string, string>(); // node id -> cluster id
  for (const node of nodes) {
    const owner = ownerAtLevel(node.id);
    if (owner !== null && members.has(owner)) {
      if (node.id !== owner) members.get(owner)!.push(node);
      assignment.set(node.id, owner);
    }
  }

  const clusters = new Map<string, SubgraphCluster>();
  for (const container of containerNodes) {
    const clusterNodes = members.get(container.id)!;
    const clusterNodeIds = new Set(clusterNodes.map(n => n.id));
    clusters.set(container.id, {
      containerId: container.id,
      containerLevel,
      parentClusterId: null, // Top-level cluster, no parent
      nodes: clusterNodes,
      internalEdges: edges.filter(e => clusterNodeIds.has(e.from) && clusterNodeIds.has(e.to)),
      outgoingEdges: edges.filter(e => clusterNodeIds.has(e.from) && !clusterNodeIds.has(e.to)),
      incomingEdges: edges.filter(e => !clusterNodeIds.has(e.from) && clusterNodeIds.has(e.to)),
    });
  }

  // An edge crosses clusters when both endpoints are assigned to different ones.
  const crossClusterEdges = edges.filter(edge => {
    const from = assignment.get(edge.from);
    const to = assignment.get(edge.to);
    return from !== undefined && to !== undefined && from !== to;
  });

  const orphanNodes = nodes.filter(n => !assignment.has(n.id));

  return {
    clusters,
    crossClusterEdges,
    orphanNodes,
  };
}

/**
 * Build a node → parent-container map for collapse-time edge aggregation.
 *
 * Derived from {@link resolveHierarchy}: each node's parent is its nearest
 * dominating container (the canonical containment chain). Only containers can be
 * collapsed, so this chain is exactly what {@link aggregateEdgesForCollapsed}
 * walks up to find a hidden node's visible representative.
 *
 * @param nodes - All nodes in the graph
 * @param edges - All edges in the graph
 * @param hierarchy - Hierarchy level mapping
 * @returns Map of node ID → parent container ID
 */
function buildContainerMap(
  nodes: GraphNode[],
  edges: GraphEdge[],
  hierarchy: HierarchyLevelMap
): Map<string, string> {
  const resolution = resolveHierarchy(toHierarchyInput(nodes, edges), hierarchy);
  const containerMap = new Map<string, string>();
  for (const node of nodes) {
    const parent = resolution.get(node.id)?.parent;
    if (parent) containerMap.set(node.id, parent);
  }
  return containerMap;
}

/**
 * Aggregate edges for collapsed containers.
 * When a container is collapsed, all edges to/from its children are "bubbled up"
 * to the container itself, creating virtual edges.
 * 
 * @param nodes - All nodes in the cluster/graph
 * @param edges - All edges in the cluster/graph  
 * @param expansionState - Which containers are expanded/collapsed
 * @returns Array of virtual edges representing aggregated child edges
 */
export function aggregateEdgesForCollapsed(
  nodes: GraphNode[],
  edges: GraphEdge[],
  expansionState: ExpansionState,
  hierarchy: HierarchyLevelMap
): VirtualEdge[] {
  // Build map of which nodes are children of which containers
  const containerMap = buildContainerMap(nodes, edges, hierarchy);
  
  // Helper: Get the visible representative for a node
  const getVisibleRepresentative = (nodeId: string): string => {
    // Traverse up the container hierarchy to find the topmost collapsed ancestor
    let current = nodeId;
    let parent = containerMap.get(current);
    
    // Keep going up until we find no more parents
    while (parent) {
      if (expansionState[parent] === false) {
        // Parent is collapsed, so everything inside it is hidden
        // The collapsed parent becomes the representative
        current = parent;
      }
      // Continue up to the next level
      parent = containerMap.get(parent);
    }
    
    return current;
  };
  
  // Collect edges that need aggregation
  const edgeAggregation = new Map<string, { from: string; to: string; sourceIds: string[] }>();
  
  edges.forEach(edge => {
    const fromRep = getVisibleRepresentative(edge.from);
    const toRep = getVisibleRepresentative(edge.to);
    
    // Only create virtual edges if at least one endpoint changed (got aggregated)
    if (fromRep === edge.from && toRep === edge.to) {
      // Both endpoints visible as-is, no aggregation needed
      return;
    }
    
    // Skip internal edges within same collapsed container
    if (fromRep === toRep) return;
    
    // Create virtual edge
    const key = `${fromRep}→${toRep}`;
    const edgeId = `${edge.from}→${edge.to}`;
    
    if (!edgeAggregation.has(key)) {
      edgeAggregation.set(key, {
        from: fromRep,
        to: toRep,
        sourceIds: [],
      });
    }
    
    edgeAggregation.get(key)!.sourceIds.push(edgeId);
  });
  
  // Convert to VirtualEdge array
  return Array.from(edgeAggregation.values()).map(agg => ({
    from: agg.from,
    to: agg.to,
    count: agg.sourceIds.length,
    sourceEdgeIds: agg.sourceIds,
  }));
}
