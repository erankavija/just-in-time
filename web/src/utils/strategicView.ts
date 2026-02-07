import type { GraphNode, GraphEdge } from '../types/models';

/**
 * Stats for downstream dependencies of a node
 */
export interface DownstreamStats {
  total: number;
  done: number;
  inProgress: number;
  blocked: number;
  ready: number;
}

/**
 * Filter nodes to only strategic ones (type:milestone or type:epic labels)
 */
export function filterStrategicNodes(nodes: GraphNode[], strategicTypes: string[] = ['milestone', 'epic']): GraphNode[] {
  return nodes.filter(node => 
    node.labels.some(label => {
      // Check if label is type:X where X is in strategicTypes
      if (label.startsWith('type:')) {
        const typeValue = label.substring(5); // Remove 'type:' prefix
        return strategicTypes.includes(typeValue);
      }
      return false;
    })
  );
}

/**
 * Filter edges to only include those between strategic nodes
 */
export function filterStrategicEdges(
  edges: GraphEdge[], 
  strategicNodeIds: Set<string>
): GraphEdge[] {
  return edges.filter(edge => 
    strategicNodeIds.has(edge.from) && strategicNodeIds.has(edge.to)
  );
}

/**
 * Calculate downstream dependency statistics for a node
 * Uses DFS to traverse all transitive dependencies
 * Only counts nodes that are LOWER in the hierarchy (higher level number)
 */
export function calculateDownstreamStats(
  nodeId: string,
  nodes: GraphNode[],
  edges: GraphEdge[],
  hierarchyConfig?: { [typeName: string]: number }
): DownstreamStats {
  const stats: DownstreamStats = {
    total: 0,
    done: 0,
    inProgress: 0,
    blocked: 0,
    ready: 0,
  };

  // Build adjacency list for efficient traversal
  const adjacencyList = new Map<string, string[]>();
  for (const edge of edges) {
    if (!adjacencyList.has(edge.from)) {
      adjacencyList.set(edge.from, []);
    }
    adjacencyList.get(edge.from)!.push(edge.to);
  }

  // Build node lookup map
  const nodeMap = new Map<string, GraphNode>();
  for (const node of nodes) {
    nodeMap.set(node.id, node);
  }
  
  // Determine hierarchy level of the source node
  const sourceNode = nodeMap.get(nodeId);
  let sourceLevel: number | null = null;
  
  if (hierarchyConfig && sourceNode) {
    // Find the type label for the source node
    for (const label of sourceNode.labels) {
      if (label.startsWith('type:')) {
        const typeName = label.substring(5);
        if (hierarchyConfig[typeName] !== undefined) {
          sourceLevel = hierarchyConfig[typeName];
          break;
        }
      }
    }
  }

  // DFS to find all downstream nodes
  const visited = new Set<string>();
  const stack = [nodeId];

  while (stack.length > 0) {
    const currentId = stack.pop()!;
    
    // Skip if already visited
    if (visited.has(currentId)) {
      continue;
    }
    visited.add(currentId);

    // Determine if we should count and traverse this node
    let shouldCountAndTraverse = true;
    
    // Don't count the root node itself, but DO traverse it
    if (currentId !== nodeId) {
      const node = nodeMap.get(currentId);
      if (node) {
        // Check hierarchy level
        if (hierarchyConfig && sourceLevel !== null) {
          let nodeLevel: number | null = null;
          
          // Find the type label for this node
          for (const label of node.labels) {
            if (label.startsWith('type:')) {
              const typeName = label.substring(5);
              if (hierarchyConfig[typeName] !== undefined) {
                nodeLevel = hierarchyConfig[typeName];
                break;
              }
            }
          }
          
          // Only count/traverse if this node is LOWER in hierarchy (higher level number)
          // If node has no type label, count it (might be an orphan/bug/etc)
          if (nodeLevel !== null && nodeLevel <= sourceLevel) {
            shouldCountAndTraverse = false;
          }
        }
        
        if (shouldCountAndTraverse) {
          stats.total++;
          
          // Count by state
          if (node.state === 'done') {
            stats.done++;
          } else if (node.state === 'in_progress') {
            stats.inProgress++;
          } else if (node.state === 'ready') {
            stats.ready++;
          }
          
          // Count blocked
          if (node.blocked) {
            stats.blocked++;
          }
        }
      }
    }

    // Only add dependencies to stack if we're traversing this node
    // (either it's the root, or it's lower in hierarchy)
    if (currentId === nodeId || shouldCountAndTraverse) {
      const dependencies = adjacencyList.get(currentId) || [];
      for (const depId of dependencies) {
        if (!visited.has(depId)) {
          stack.push(depId);
        }
      }
    }
  }

  return stats;
}
