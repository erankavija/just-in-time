import type { GraphNode, GraphEdge } from '../types/models';
import type { 
  HierarchyLevelMap, 
  SubgraphCluster, 
  ExpansionState,
  VirtualEdge 
} from '../types/subgraphCluster';
import { assignNodesToSubgraphs, assignNodesToClusters, getHierarchyLevels, getNodeLevel } from './subgraphClustering';

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
 * Prepare a clustered graph for ReactFlow rendering with multi-level clustering support.
 * 
 * This function:
 * 1. Applies clustering at primary hierarchy level (N)
 * 2. Within expanded clusters, applies sub-clustering at next level (N+1)
 * 3. Determines visible nodes based on hierarchical expansion state
 * 4. Creates virtual edges for collapsed containers at any level
 * 5. Returns only the data needed for rendering
 * 
 * @param nodes - All graph nodes
 * @param edges - All graph edges
 * @param hierarchy - Hierarchy level mapping from config
 * @param expansionState - Which containers are expanded/collapsed (at any level)
 * @returns Data ready for ReactFlow rendering
 */
export function prepareClusteredGraphForReactFlow(
  nodes: GraphNode[],
  edges: GraphEdge[],
  hierarchy: HierarchyLevelMap,
  expansionState: ExpansionState
): ClusteredGraphForReactFlow {
  // Step 1: Apply primary clustering (container level selection happens in assignNodesToSubgraphs)
  const primaryClustered = assignNodesToSubgraphs(nodes, edges, hierarchy);
  
  // Step 2: Apply nested clustering within expanded primary clusters
  // Find the primary container level and next level for sub-clustering
  const levels = getHierarchyLevels(nodes, hierarchy);
  const primaryContainerLevel = primaryClustered.clusters.size > 0
    ? Array.from(primaryClustered.clusters.values())[0].containerLevel
    : levels[0];
  const subContainerLevel = levels.find(l => l > primaryContainerLevel);
  
  // Build comprehensive cluster map (includes both primary and sub-clusters)
  const allClusters = new Map<string, SubgraphCluster>();
  const nodeToClusterMap = new Map<string, string>(); // node -> immediate parent cluster
  const nodeToAncestorClusters = new Map<string, string[]>(); // node -> all ancestor clusters (hierarchical)
  
  // Add primary clusters
  primaryClustered.clusters.forEach((cluster, clusterId) => {
    allClusters.set(clusterId, cluster);
    cluster.nodes.forEach(node => {
      nodeToClusterMap.set(node.id, clusterId);
      nodeToAncestorClusters.set(node.id, [clusterId]);
    });
  });
  
  // Apply sub-clustering within expanded primary clusters
  if (subContainerLevel) {
    primaryClustered.clusters.forEach((primaryCluster) => {
      const isPrimaryExpanded = expansionState[primaryCluster.containerId] ?? false; // Default to collapsed
      
      if (isPrimaryExpanded) {
        // Check if there are any nodes at the sub-container level
        const hasSubContainers = primaryCluster.nodes.some(n => 
          getNodeLevel(n, hierarchy) === subContainerLevel
        );
        
        if (!hasSubContainers) return; // Skip if no sub-containers exist
        
        // Apply sub-clustering to nodes within this primary cluster
        const subClustered = assignNodesToClusters(
          primaryCluster.nodes,
          primaryCluster.internalEdges,
          hierarchy,
          subContainerLevel
        );
        
        // Add sub-clusters to the comprehensive map
        subClustered.clusters.forEach((subCluster, subClusterId) => {
          // Mark sub-cluster with parent cluster ID
          const enrichedSubCluster: SubgraphCluster = {
            ...subCluster,
            parentClusterId: primaryCluster.containerId,
          };
          allClusters.set(subClusterId, enrichedSubCluster);
          
          // Update node mappings for sub-cluster members
          subCluster.nodes.forEach(node => {
            // Update immediate parent (overrides primary cluster for children)
            if (node.id !== subCluster.containerId) {
              nodeToClusterMap.set(node.id, subClusterId);
            }
            
            // Build ancestor chain: [primaryCluster, subCluster]
            const ancestors = [primaryCluster.containerId];
            if (node.id !== subCluster.containerId) {
              ancestors.push(subClusterId);
            }
            nodeToAncestorClusters.set(node.id, ancestors);
          });
        });
      }
    });
  }
  
  // Step 3: Determine visible nodes based on hierarchical expansion state
  const allClusterContainerIds = new Set(
    Array.from(allClusters.values()).map(c => c.containerId)
  );
  
  const isNodeVisible = (nodeId: string): boolean => {
    // Cluster containers are visible if their parent cluster is expanded (or no parent)
    if (allClusterContainerIds.has(nodeId)) {
      const ancestors = nodeToAncestorClusters.get(nodeId) || [];
      // Check all ancestors are expanded (skip the node itself if it's in the list)
      const parentAncestors = ancestors.filter(a => a !== nodeId);
      return parentAncestors.every(ancestorId => expansionState[ancestorId] ?? false); // Default to collapsed
    }
    
    // Regular nodes: visible only if ALL ancestor clusters are expanded
    const ancestors = nodeToAncestorClusters.get(nodeId) || [];
    return ancestors.every(ancestorId => expansionState[ancestorId] ?? false); // Default to collapsed
  };
  
  const visibleNodeIds = new Set<string>();
  nodes.forEach(node => {
    if (isNodeVisible(node.id)) {
      visibleNodeIds.add(node.id);
    }
  });
  
  const visibleNodes = nodes.filter(n => visibleNodeIds.has(n.id));
  
  // Step 4: Filter edges and create virtual edges for collapsed containers
  const visibleEdges: GraphEdge[] = [];
  const virtualEdgeMap = new Map<string, VirtualEdge>();
  
  edges.forEach(edge => {
    const fromVisible = visibleNodeIds.has(edge.from);
    const toVisible = visibleNodeIds.has(edge.to);
    
    if (fromVisible && toVisible) {
      // Both endpoints visible - show edge as-is
      visibleEdges.push(edge);
    } else {
      // At least one endpoint hidden - create virtual edge to closest visible ancestor
      const fromAncestors = nodeToAncestorClusters.get(edge.from) || [];
      const toAncestors = nodeToAncestorClusters.get(edge.to) || [];
      
      // Find the first visible ancestor (or the node itself if visible)
      const getVisibleRep = (nodeId: string, ancestors: string[]): string => {
        if (visibleNodeIds.has(nodeId)) return nodeId;
        // Walk up ancestor chain to find first visible ancestor
        for (const ancestorId of ancestors) {
          if (visibleNodeIds.has(ancestorId)) return ancestorId;
        }
        return nodeId; // Fallback (shouldn't happen)
      };
      
      const virtualFrom = getVisibleRep(edge.from, fromAncestors);
      const virtualTo = getVisibleRep(edge.to, toAncestors);
      
      // Skip if both endpoints are the same (internal edge within collapsed cluster)
      if (virtualFrom === virtualTo) return;
      
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
  
  // Step 5: Package results
  return {
    clusters: Array.from(allClusters.values()),
    visibleNodes,
    visibleEdges,
    virtualEdges,
    crossClusterEdges: primaryClustered.crossClusterEdges,
    orphanNodes: primaryClustered.orphanNodes,
  };
}
