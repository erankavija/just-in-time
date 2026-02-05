import type { GraphNode, GraphEdge } from '../types/models';
import type { 
  HierarchyLevelMap, 
  SubgraphCluster, 
  ClusteredGraph,
  ExpansionState,
  VirtualEdge 
} from '../types/subgraphCluster';

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
 * Each Level 1 node (e.g., epic) becomes a cluster containing all lower-level nodes it depends on.
 * 
 * Algorithm:
 * 1. Find all Level 1 nodes (lowest numeric level, e.g., epic = level 2)
 * 2. For each Level 1 node, recursively follow dependencies
 * 3. Include all nodes with HIGHER level numbers (more tactical)
 * 4. Stop when encountering SAME or LOWER level (another strategic node)
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
  
  // Container level is the LOWEST level present (most strategic nodes become containers)
  // e.g., if we have epic=2, story=3, task=4 → containers are at level 2 (epics)
  //       if we have milestone=1, epic=2, ... → containers are at level 1 (milestones)
  const containerLevel = uniqueLevels[0];
  
  // Find all container nodes
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
  
  // FIRST PASS: Mark ALL direct children of ALL containers
  // This prevents transitive dependencies from stealing direct children
  const directChildrenMap = new Map<string, Set<string>>();
  for (const container of containerNodes) {
    const directChildren = new Set<string>();
    const containerEdges = edgesByNode.get(container.id) || [];
    
    for (const edge of containerEdges) {
      const targetNode = nodeMap.get(edge.to);
      if (targetNode) {
        const targetLevel = getNodeLevel(targetNode, hierarchy);
        if (targetLevel > containerLevel) {
          directChildren.add(targetNode.id);
          assignedNodes.add(targetNode.id); // Reserve globally
        }
      }
    }
    
    directChildrenMap.set(container.id, directChildren);
  }
  
  // SECOND PASS: Build clusters with BFS
  for (const container of containerNodes) {
    const clusterNodes: GraphNode[] = [container];
    const visited = new Set<string>([container.id]);
    const queue = [container.id];
    const myDirectChildren = directChildrenMap.get(container.id) || new Set();
    
    // BFS to find all transitive lower-level dependents
    while (queue.length > 0) {
      const currentId = queue.shift()!;
      const outgoingEdges = edgesByNode.get(currentId) || [];
      
      for (const edge of outgoingEdges) {
        const targetNode = nodeMap.get(edge.to);
        if (!targetNode || visited.has(targetNode.id)) continue;
        
        const targetLevel = getNodeLevel(targetNode, hierarchy);
        
        // Include if: higher level AND (my direct child OR not assigned to another cluster)
        const isMyDirectChild = myDirectChildren.has(targetNode.id);
        const isAvailable = !assignedNodes.has(targetNode.id);
        
        if (targetLevel > containerLevel && (isMyDirectChild || isAvailable)) {
          clusterNodes.push(targetNode);
          visited.add(targetNode.id);
          if (!isMyDirectChild) {
            assignedNodes.add(targetNode.id); // Claim if not already claimed
          }
          queue.push(targetNode.id);
        }
        // Note: If targetLevel <= containerLevel, this is a cross-cluster edge (boundary)
      }
    }
    
    // Create cluster
    const clusterNodeIds = new Set(clusterNodes.map(n => n.id));
    const internalEdges = edges.filter(e => clusterNodeIds.has(e.from) && clusterNodeIds.has(e.to));
    const outgoingEdges = edges.filter(e => clusterNodeIds.has(e.from) && !clusterNodeIds.has(e.to));
    const incomingEdges = edges.filter(e => !clusterNodeIds.has(e.from) && clusterNodeIds.has(e.to));
    
    clusters.set(container.id, {
      containerId: container.id,
      containerLevel,
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
    
    // Edge crosses clusters if nodes are in different clusters (or one is not in any cluster)
    return fromCluster !== toCluster && fromCluster && toCluster;
  });
  
  // Find orphan nodes (not assigned to any cluster and not containers)
  const orphanNodes = nodes.filter(n => 
    !assignedNodes.has(n.id) && 
    getNodeLevel(n, hierarchy) > containerLevel
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
  
  // Helper: Find which nodes are hidden by collapsed containers
  const isVisible = (nodeId: string): boolean => {
    // Check if this node itself is collapsed (it's still visible, just its children are hidden)
    // A node is hidden if it's INSIDE a collapsed parent
    let parent = containerMap.get(nodeId);
    
    while (parent) {
      if (expansionState[parent] === false) {
        // This node is inside a collapsed parent
        return false;
      }
      parent = containerMap.get(parent);
    }
    
    return true;  // No collapsed parents, so visible
  };
  
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
