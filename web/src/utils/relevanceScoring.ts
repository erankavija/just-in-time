import type { GraphNode, GraphEdge, Priority } from '../types/models';

/**
 * Weight constants for relevance scoring
 * Higher weights = more important in determining visibility
 */
const WEIGHTS = {
  SELECTED_PATH: 10000,     // Item is on selected/highlighted path
  BLOCKED: 1000,            // Item is blocked or blocking
  PRIORITY_CRITICAL: 500,   // Critical priority
  PRIORITY_HIGH: 300,       // High priority
  PRIORITY_NORMAL: 100,     // Normal priority (baseline)
  PRIORITY_LOW: 50,         // Low priority
  DEGREE_MULTIPLIER: 10,    // Multiply by node degree (edge count)
};

/**
 * Compute the degree (edge count) of a node
 * @param nodeId - Node to compute degree for
 * @param edges - All edges in the graph
 * @returns Total number of edges (incoming + outgoing)
 */
export function computeNodeDegree(nodeId: string, edges: GraphEdge[]): number {
  let degree = 0;
  for (const edge of edges) {
    if (edge.from === nodeId || edge.to === nodeId) {
      degree++;
    }
  }
  return degree;
}

/**
 * Get priority weight for a priority level
 */
function getPriorityWeight(priority: Priority): number {
  switch (priority) {
    case 'critical':
      return WEIGHTS.PRIORITY_CRITICAL;
    case 'high':
      return WEIGHTS.PRIORITY_HIGH;
    case 'normal':
      return WEIGHTS.PRIORITY_NORMAL;
    case 'low':
      return WEIGHTS.PRIORITY_LOW;
    default:
      return WEIGHTS.PRIORITY_NORMAL;
  }
}

/**
 * Simple hash function for stable ID tie-breaking
 * Returns a small positive number based on the string
 */
function hashId(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = ((hash << 5) - hash) + id.charCodeAt(i);
    hash = hash & hash; // Convert to 32-bit integer
  }
  return Math.abs(hash) % 100; // Keep it small (0-99)
}

/**
 * Calculate relevance score for a node
 * 
 * Scoring factors (in order of importance):
 * 1. Selected/highlighted path (highest)
 * 2. Blocked or blocking status
 * 3. Priority (critical > high > normal > low)
 * 4. Node degree (number of dependencies)
 * 5. Stable ID hash (tie-breaker)
 * 
 * @param nodeId - Node to score
 * @param allNodes - All nodes (for lookups)
 * @param edges - All edges (for degree calculation)
 * @param selectedPath - Set of node IDs on the selected/highlighted path
 * @returns Relevance score (higher = more relevant)
 */
export function scoreNodeRelevance(
  nodeId: string,
  allNodes: GraphNode[],
  edges: GraphEdge[],
  selectedPath: Set<string>
): number {
  const node = allNodes.find((n) => n.id === nodeId);
  if (!node) return 0;

  let score = 0;

  // 1. Selected/highlighted path (dominates all other factors)
  if (selectedPath.has(nodeId)) {
    score += WEIGHTS.SELECTED_PATH;
  }

  // 2. Blocked or blocking status
  if (node.blocked) {
    score += WEIGHTS.BLOCKED;
  }

  // 3. Priority weight
  score += getPriorityWeight(node.priority);

  // 4. Node degree (structural importance)
  const degree = computeNodeDegree(nodeId, edges);
  score += degree * WEIGHTS.DEGREE_MULTIPLIER;

  // 5. Stable ID hash (tie-breaker for deterministic ordering)
  score += hashId(nodeId);

  return score;
}

/**
 * Order nodes by relevance score (descending)
 * 
 * @param nodes - Nodes to order
 * @param edges - All edges (for degree calculation)
 * @param selectedPath - Set of node IDs on selected/highlighted path
 * @returns Nodes ordered by relevance (most relevant first)
 */
export function orderNodesByRelevance(
  nodes: GraphNode[],
  edges: GraphEdge[],
  selectedPath: Set<string>
): GraphNode[] {
  // Score all nodes
  const scored = nodes.map((node) => ({
    node,
    score: scoreNodeRelevance(node.id, nodes, edges, selectedPath),
  }));

  // Sort by score descending (highest score first)
  scored.sort((a, b) => b.score - a.score);

  return scored.map((s) => s.node);
}
