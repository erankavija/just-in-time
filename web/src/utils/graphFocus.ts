import type { SubgraphCluster } from '../types/subgraphCluster';

/**
 * Find all parent cluster IDs for a given node.
 * Returns cluster IDs from innermost to outermost (child-first order).
 * 
 * @param nodeId - The ID of the node to find parents for
 * @param clusterData - Array of clusters (flat structure with parentClusterId links)
 * @returns Array of parent cluster IDs, from innermost to outermost
 * 
 * @example
 * // For a task in a story cluster in an epic cluster:
 * findParentClusters('task-1', clusters)
 * // => ['story-1', 'epic-1']
 */
export function findParentClusters(
  nodeId: string,
  clusterData: SubgraphCluster[]
): string[] {
  const parents: string[] = [];

  // Find ALL clusters that contain this node
  const matchingClusters = clusterData.filter(cluster => 
    cluster.nodes.some(node => node.id === nodeId)
  );

  if (matchingClusters.length === 0) {
    return [];
  }

  // Find the MOST SPECIFIC cluster (highest level number = deepest nesting)
  // This handles cases where child nodes are included in parent cluster nodes arrays
  const directCluster = matchingClusters.reduce((mostSpecific, current) => 
    current.containerLevel > mostSpecific.containerLevel ? current : mostSpecific
  );

  // Add this cluster to parents list
  parents.push(directCluster.containerId);

  // Walk up the parent chain
  let currentClusterId = directCluster.parentClusterId;
  while (currentClusterId) {
    parents.push(currentClusterId);
    const parentCluster = clusterData.find(c => c.containerId === currentClusterId);
    if (!parentCluster) break;
    currentClusterId = parentCluster.parentClusterId;
  }

  return parents;
}
