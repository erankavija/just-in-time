import type { GraphNode, GraphEdge } from '../types/models';
import type { 
  HierarchyLevelMap, 
  SubgraphCluster, 
  ExpansionState,
  VirtualEdge 
} from '../types/subgraphCluster';
import { assignNodesToSubgraphs } from './subgraphClustering';

/**
 * Result of preparing clustered graph for ReactFlow rendering.
 * Contains clusters, visible nodes/edges, and virtual edges for collapsed containers.
 */
export interface ClusteredGraphForReactFlow {
  /** Subgraph clusters (array for easy iteration) */
  clusters: SubgraphCluster[];
  
  /** Edges that cross cluster boundaries */
  crossClusterEdges: GraphEdge[];
  
  /** Nodes that should be rendered (accounting for collapsed containers) */
  visibleNodes: GraphNode[];
  
  /** Edges between visible nodes */
  visibleEdges: GraphEdge[];
  
  /** Virtual edges representing aggregated edges from collapsed containers */
  virtualEdges: VirtualEdge[];
  
  /** Nodes not assigned to any cluster */
  orphanNodes: GraphNode[];
}

/**
 * Prepare a clustered graph for ReactFlow rendering.
 * 
 * This function:
 * 1. Assigns nodes to subgraph clusters based on hierarchy
 * 2. Determines which nodes are visible based on expansion state
 * 3. Creates virtual edges for collapsed containers
 * 4. Returns only the data needed for rendering
 * 
 * @param nodes - All graph nodes
 * @param edges - All graph edges
 * @param hierarchy - Hierarchy level mapping from config
 * @param expansionState - Which containers are expanded/collapsed
 * @returns Data ready for ReactFlow rendering
 */
export function prepareClusteredGraphForReactFlow(
  nodes: GraphNode[],
  edges: GraphEdge[],
  hierarchy: HierarchyLevelMap,
  expansionState: ExpansionState
): ClusteredGraphForReactFlow {
  // Step 1: Assign nodes to clusters
  const clusteredGraph = assignNodesToSubgraphs(nodes, edges, hierarchy);
  
  // Step 2: Determine visible nodes based on expansion state
  const visibleNodeIds = new Set<string>();
  
  // Identify cluster container nodes (nodes that have clusters)
  const clusterContainerIds = new Set(
    Array.from(clusteredGraph.clusters.values()).map(c => c.containerId)
  );
  
  // Build map: nodeId -> clusterId (which cluster is this node a member of?)
  const nodeToClusterMap = new Map<string, string>();
  clusteredGraph.clusters.forEach((cluster, clusterId) => {
    cluster.nodes.forEach(node => {
      nodeToClusterMap.set(node.id, clusterId);
    });
  });
  
  // Helper: check if node is visible
  const isNodeVisible = (nodeId: string): boolean => {
    // Cluster containers are ALWAYS visible (they are the top-level collapsible units)
    if (clusterContainerIds.has(nodeId)) {
      return true;
    }
    
    // Check if this node is a member of a cluster
    const clusterId = nodeToClusterMap.get(nodeId);
    
    if (clusterId) {
      // Node is inside a cluster - visible only if cluster is expanded
      const isExpanded = expansionState[clusterId] ?? true; // Default to expanded
      return isExpanded;
    }
    
    // Not in any cluster (orphan node) - always visible
    return true;
  };
  
  // Determine visible nodes
  nodes.forEach(node => {
    if (isNodeVisible(node.id)) {
      visibleNodeIds.add(node.id);
    }
  });
  
  const visibleNodes = nodes.filter(n => visibleNodeIds.has(n.id));
  
  // Step 3: Filter edges and create virtual edges for collapsed clusters
  const visibleEdges: GraphEdge[] = [];
  const virtualEdgeMap = new Map<string, VirtualEdge>();
  
  edges.forEach(edge => {
    const fromVisible = visibleNodeIds.has(edge.from);
    const toVisible = visibleNodeIds.has(edge.to);
    
    if (fromVisible && toVisible) {
      // Both endpoints visible - show edge as-is
      visibleEdges.push(edge);
    } else {
      // At least one endpoint is hidden - create virtual edge to/from cluster container
      const fromClusterId = nodeToClusterMap.get(edge.from);
      const toClusterId = nodeToClusterMap.get(edge.to);
      
      // Determine virtual edge endpoints
      const virtualFrom = fromVisible ? edge.from : fromClusterId || edge.from;
      const virtualTo = toVisible ? edge.to : toClusterId || edge.to;
      
      // Skip if both endpoints end up being the same (internal edge within collapsed cluster)
      if (virtualFrom === virtualTo) return;
      
      // Create/update virtual edge
      const key = `${virtualFrom}→${virtualTo}`;
      if (!virtualEdgeMap.has(key)) {
        virtualEdgeMap.set(key, {
          from: virtualFrom,
          to: virtualTo,
          count: 0,
          sourceEdgeIds: [],
        });
      }
      const virtualEdge = virtualEdgeMap.get(key)!;
      virtualEdge.count++;
      virtualEdge.sourceEdgeIds.push(`${edge.from}→${edge.to}`);
    }
  });
  
  const virtualEdges = Array.from(virtualEdgeMap.values());
  
  return {
    clusters: Array.from(clusteredGraph.clusters.values()),
    crossClusterEdges: clusteredGraph.crossClusterEdges,
    visibleNodes,
    visibleEdges,
    virtualEdges,
    orphanNodes: clusteredGraph.orphanNodes,
  };
}
