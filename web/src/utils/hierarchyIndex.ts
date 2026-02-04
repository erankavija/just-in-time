import type { GraphNode } from '../types/models';
import type { HierarchyConfig } from '../types/hierarchy';
import { getPrimaryTier } from '../types/hierarchy';

/**
 * Extract the node type from type:X label
 */
export function extractNodeType(node: GraphNode): string | null {
  const typeLabel = node.labels.find((l) => l.startsWith('type:'));
  if (!typeLabel) return null;
  return typeLabel.substring(5); // Remove 'type:' prefix
}

/**
 * Extract the value from a tierType:value label
 * @param node - The node to extract from
 * @param tierType - The tier type to look for (e.g., 'milestone', 'release', 'epic')
 * @returns The value part of the label, or null if not found
 */
export function extractTierLabel(node: GraphNode, tierType: string): string | null {
  const prefix = `${tierType}:`;
  const label = node.labels.find((l) => l.startsWith(prefix));
  if (!label) return null;
  return label.substring(prefix.length);
}

/**
 * Derive the primary tier index for a node based on hierarchy config
 * @param node - The node to derive index for
 * @param allNodes - All nodes in the graph (for lookups)
 * @param config - Hierarchy configuration
 * @returns Primary tier index (0-based), or -1 for unassigned nodes
 */
export function derivePrimaryTierIndex(
  node: GraphNode,
  allNodes: GraphNode[],
  config: HierarchyConfig
): number {
  const primaryTier = getPrimaryTier(config);
  
  // If no primary tier configured, all nodes go to index 0
  if (!primaryTier) return 0;

  const nodeType = extractNodeType(node);
  
  // If this node IS a primary tier node, derive its index
  if (nodeType && primaryTier.types.includes(nodeType)) {
    return derivePrimaryTierNodeIndex(node, allNodes, primaryTier.types);
  }

  // Otherwise, inherit from parent primary tier label
  // Try each primary tier type to find matching label
  for (const primaryType of primaryTier.types) {
    const primaryLabel = extractTierLabel(node, primaryType);
    if (!primaryLabel) continue;

    // Find the primary tier node with matching label and get its index
    const primaryNode = allNodes.find(
      (n) => extractNodeType(n) === primaryType && extractTierLabel(n, primaryType) === primaryLabel
    );

    if (primaryNode) {
      return derivePrimaryTierNodeIndex(primaryNode, allNodes, primaryTier.types);
    }
  }

  // If we can't find the primary tier node, assign to unassigned bucket
  return -1;
}

/**
 * Derive the index for a primary tier node itself
 * Uses chronological ordering based on appearance in allNodes
 */
function derivePrimaryTierNodeIndex(
  node: GraphNode,
  allNodes: GraphNode[],
  primaryTierTypes: string[]
): number {
  // Find all primary tier nodes (any type at primary level)
  const primaryTierNodes = allNodes.filter((n) => {
    const type = extractNodeType(n);
    return type && primaryTierTypes.includes(type);
  });
  
  // Find index of this node among primary tier nodes
  const index = primaryTierNodes.findIndex((n) => n.id === node.id);
  
  return index >= 0 ? index : -1;
}

/**
 * Group nodes by their primary tier index
 * @param nodes - Nodes to group
 * @param config - Hierarchy configuration
 * @returns Map from primary tier index to nodes in that tier
 */
export function groupNodesByPrimaryTier(
  nodes: GraphNode[],
  config: HierarchyConfig
): Map<number, GraphNode[]> {
  const groups = new Map<number, GraphNode[]>();

  for (const node of nodes) {
    const index = derivePrimaryTierIndex(node, nodes, config);
    
    if (!groups.has(index)) {
      groups.set(index, []);
    }
    groups.get(index)!.push(node);
  }

  return groups;
}
