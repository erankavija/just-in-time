import type { GraphNode, GraphEdge } from '../types/models';
import type { 
  HierarchyLevelMap, 
  SubgraphCluster, 
  ClusteredGraph,
  ExpansionState,
  VirtualEdge 
} from '../types/subgraphCluster';

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
 * Generic clustering algorithm that works for any hierarchy level.
 * 
 * Algorithm:
 * 1. Find all nodes at containerLevel to use as cluster containers
 * 2. For each container, recursively follow dependencies
 * 3. Include all nodes with HIGHER level numbers (more tactical)
 * 4. Stop when encountering SAME or LOWER level (cluster boundaries)
 * 
 * @param nodes - Nodes to cluster
 * @param edges - Edges between these nodes
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
  // Find all nodes at the container level (e.g., stories)
  const containerNodes = nodes.filter(n => getNodeLevel(n, hierarchy) === containerLevel);
  
  if (containerNodes.length === 0) {
    return { clusters: new Map(), crossClusterEdges: [], orphanNodes: nodes };
  }
  
  // Build adjacency map for efficient traversal
  const nodeMap = new Map<string, GraphNode>();
  nodes.forEach(n => nodeMap.set(n.id, n));
  
  const edgesByNode = new Map<string, GraphEdge[]>();
  edges.forEach(edge => {
    if (!edgesByNode.has(edge.from)) {
      edgesByNode.set(edge.from, []);
    }
    edgesByNode.get(edge.from)!.push(edge);
  });
  
  // Assign nodes to clusters
  const clusters = new Map<string, SubgraphCluster>();
  const assignedNodes = new Set<string>();
  
  // Mark direct children of all containers first (prevents stealing)
  const directChildrenMap = new Map<string, Set<string>>();
  for (const container of containerNodes) {
    const directChildren = new Set<string>();
    const containerEdges = edgesByNode.get(container.id) || [];
    
    for (const edge of containerEdges) {
      const childNode = nodeMap.get(edge.to);
      if (childNode && getNodeLevel(childNode, hierarchy) > containerLevel) {
        directChildren.add(edge.to);
      }
    }
    
    directChildrenMap.set(container.id, directChildren);
  }
  
  // Now assign nodes to clusters with traversal
  for (const container of containerNodes) {
    const clusterNodes: GraphNode[] = [container];
    const visited = new Set<string>();
    const queue: string[] = [container.id];
    const directChildren = directChildrenMap.get(container.id) || new Set();
    
    while (queue.length > 0) {
      const current = queue.shift()!;
      
      if (visited.has(current)) {
        continue;
      }
      visited.add(current);
      
      const currentNode = nodeMap.get(current);
      if (!currentNode) continue;
      
      const currentLevel = getNodeLevel(currentNode, hierarchy);
      
      // Add this node to cluster if it's more tactical than container
      if (current !== container.id && currentLevel > containerLevel) {
        // Check if it's not already a direct child of another container
        const isDirectChildOfAnother = [...directChildrenMap.entries()].some(
          ([otherId, children]) => otherId !== container.id && children.has(current)
        );
        
        if (!isDirectChildOfAnother && !assignedNodes.has(current)) {
          clusterNodes.push(currentNode);
          assignedNodes.add(current);
        }
      }
      
      // Traverse dependencies
      const nodeEdges = edgesByNode.get(current) || [];
      for (const edge of nodeEdges) {
        const targetNode = nodeMap.get(edge.to);
        if (!targetNode) continue;
        
        const targetLevel = getNodeLevel(targetNode, hierarchy);
        
        // Continue traversal if target is more tactical
        if (targetLevel > containerLevel && !visited.has(edge.to)) {
          queue.push(edge.to);
        }
        
        // Stop at same or more strategic levels (cluster boundaries)
      }
    }
    
    // Create cluster (even if only contains the container itself)
    const clusterNodeIds = new Set(clusterNodes.map(n => n.id));
    const internalEdges = edges.filter(e => clusterNodeIds.has(e.from) && clusterNodeIds.has(e.to));
    const outgoingEdges = edges.filter(e => clusterNodeIds.has(e.from) && !clusterNodeIds.has(e.to));
    const incomingEdges = edges.filter(e => !clusterNodeIds.has(e.from) && clusterNodeIds.has(e.to));
    
    clusters.set(container.id, {
      containerId: container.id,
      containerLevel,
      parentClusterId: null, // Top-level cluster, no parent
      nodes: clusterNodes,
      internalEdges,
      outgoingEdges,
      incomingEdges,
    });
    
    assignedNodes.add(container.id);
  }
  
  // Find cross-cluster edges
  const crossClusterEdges = edges.filter(edge => {
    const fromCluster = [...clusters.values()].find(c => 
      c.nodes.some(n => n.id === edge.from)
    );
    const toCluster = [...clusters.values()].find(c => 
      c.nodes.some(n => n.id === edge.to)
    );
    
    return fromCluster !== toCluster && fromCluster && toCluster;
  });
  
  // Find orphan nodes (containers without children)
  const orphanNodes = nodes.filter(n => 
    !assignedNodes.has(n.id)
  );
  
  return {
    clusters,
    crossClusterEdges,
    orphanNodes,
  };
}

/**
 * Build child-parent map for efficient lookup of which containers own which nodes.
 * Uses dependency edges where from → to means "from contains/owns to" in the hierarchy.
 * Prefers the most strategic parent (lowest hierarchy level) when multiple parents exist.
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
  const containerMap = new Map<string, string>();
  const nodeMap = new Map(nodes.map(n => [n.id, n]));
  const nodeIds = new Set(nodes.map(n => n.id));
  
  // In a dependency graph, edge from→to means "from depends on to"
  // For containment, we want the REVERSE: if epic→story, then story's parent is epic
  edges.forEach(edge => {
    // Only process edges where BOTH nodes exist (ignore external dependencies)
    if (!nodeIds.has(edge.from) || !nodeIds.has(edge.to)) {
      return;
    }
    
    const currentParent = containerMap.get(edge.to);
    
    if (!currentParent) {
      // No parent yet, use this one
      containerMap.set(edge.to, edge.from);
    } else {
      // Compare hierarchy levels - keep the more strategic parent (lower level number)
      const currentParentNode = nodeMap.get(currentParent);
      const newParentNode = nodeMap.get(edge.from);
      
      if (currentParentNode && newParentNode) {
        const currentLevel = getNodeLevel(currentParentNode, hierarchy);
        const newLevel = getNodeLevel(newParentNode, hierarchy);
        
        if (newLevel < currentLevel) {
          // New parent is more strategic, use it
          containerMap.set(edge.to, edge.from);
        }
      }
    }
  });
  
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
