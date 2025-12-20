import type { GraphNode, GraphEdge } from '../types/models';

// Default strategic types - should be fetched from API in the future
// Currently matches the default configuration in labels.json
const DEFAULT_STRATEGIC_TYPES = ['milestone', 'epic'];

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
 * TODO: Fetch strategic_types from API instead of hardcoding
 */
export function filterStrategicNodes(nodes: GraphNode[], strategicTypes: string[] = DEFAULT_STRATEGIC_TYPES): GraphNode[] {
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
 */
export function calculateDownstreamStats(
  nodeId: string,
  nodes: GraphNode[],
  edges: GraphEdge[]
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

    // Don't count the root node itself
    if (currentId !== nodeId) {
      const node = nodeMap.get(currentId);
      if (node) {
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

    // Add dependencies to stack
    const dependencies = adjacencyList.get(currentId) || [];
    for (const depId of dependencies) {
      if (!visited.has(depId)) {
        stack.push(depId);
      }
    }
  }

  return stats;
}
