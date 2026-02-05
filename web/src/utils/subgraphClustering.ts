import type { GraphNode, GraphEdge } from '../types/models';
import type { HierarchyLevelMap, SubgraphCluster, ClusteredGraph } from '../types/subgraphCluster';

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
  
  for (const container of containerNodes) {
    const clusterNodes: GraphNode[] = [container];
    const visited = new Set<string>([container.id]);
    const queue = [container.id];
    
    // BFS to find all lower-level dependents
    while (queue.length > 0) {
      const currentId = queue.shift()!;
      const outgoingEdges = edgesByNode.get(currentId) || [];
      
      for (const edge of outgoingEdges) {
        const targetNode = nodeMap.get(edge.to);
        if (!targetNode || visited.has(targetNode.id)) continue;
        
        const targetLevel = getNodeLevel(targetNode, hierarchy);
        
        // Only include nodes at HIGHER level numbers (more tactical)
        if (targetLevel > containerLevel) {
          clusterNodes.push(targetNode);
          visited.add(targetNode.id);
          assignedNodes.add(targetNode.id);
          queue.push(targetNode.id);
        }
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
