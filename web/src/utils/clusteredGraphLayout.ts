import type { GraphNode, GraphEdge } from '../types/models';
import type { 
  HierarchyLevelMap, 
  SubgraphCluster, 
  ExpansionState,
  VirtualEdge 
} from '../types/subgraphCluster';
import { assignNodesToSubgraphs, aggregateEdgesForCollapsed } from './subgraphClustering';

/**
 * Result of preparing clustered graph for ReactFlow rendering.
 * Contains clusters, visible nodes/edges, and virtual edges for collapsed containers.
 */
export interface ClusteredGraphForReactFlow {
  /** Map of container ID to subgraph cluster */
  clusters: Map<string, SubgraphCluster>;
  
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
  
  // Build container map for visibility checks
  const containerMap = new Map<string, string>();
  edges.forEach(edge => {
    if (!containerMap.has(edge.to)) {
      containerMap.set(edge.to, edge.from);
    }
  });
  
  // Helper: check if node is visible
  const isNodeVisible = (nodeId: string): boolean => {
    // Node is visible if no parent is collapsed
    let parent = containerMap.get(nodeId);
    
    while (parent) {
      if (expansionState[parent] === false) {
        return false; // Hidden by collapsed parent
      }
      parent = containerMap.get(parent);
    }
    
    return true; // No collapsed parents
  };
  
  // Determine visible nodes
  nodes.forEach(node => {
    if (isNodeVisible(node.id)) {
      visibleNodeIds.add(node.id);
    }
  });
  
  const visibleNodes = nodes.filter(n => visibleNodeIds.has(n.id));
  
  // Step 3: Filter edges to only those between visible nodes
  const visibleEdges = edges.filter(
    e => visibleNodeIds.has(e.from) && visibleNodeIds.has(e.to)
  );
  
  // Step 4: Generate virtual edges for collapsed containers
  const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState);
  
  // Step 5: Filter virtual edges to only those with visible endpoints
  const filteredVirtualEdges = virtualEdges.filter(
    ve => visibleNodeIds.has(ve.from) && visibleNodeIds.has(ve.to)
  );
  
  return {
    clusters: clusteredGraph.clusters,
    visibleNodes,
    visibleEdges,
    virtualEdges: filteredVirtualEdges,
    orphanNodes: clusteredGraph.orphanNodes,
  };
}
