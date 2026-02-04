import type { GraphNode, GraphEdge } from '../types/models';
import type { HierarchyConfig, NodeCounts } from '../types/hierarchy';
import { getPrimaryTier, getSecondaryTier, getHierarchyLevels } from '../types/hierarchy';
import type {
  VirtualNode,
  VirtualNodeMetadata,
  WindowConfig,
  ExpansionState,
  ExplorationGraph,
} from '../types/explorationGraph';
import { groupNodesByPrimaryTier, extractNodeType } from './hierarchyIndex';
import { orderNodesByRelevance } from './relevanceScoring';

/**
 * Helper to check if a node is virtual
 */
export function isVirtualNode(node: GraphNode | VirtualNode): node is VirtualNode {
  return 'virtualType' in node;
}

/**
 * Helper to create a virtual node ID
 */
function createVirtualNodeId(prefix: string, ...parts: (string | number)[]): string {
  return `${prefix}-${parts.join('-')}`;
}

/**
 * Create a virtual node for a collapsed primary tier bucket
 */
function createPrimaryBucketNode(
  range: [number, number],
  nodes: GraphNode[],
  hierarchyConfig: HierarchyConfig
): VirtualNode {
  const counts = countNodesByType(nodes, hierarchyConfig);
  const primaryTier = getPrimaryTier(hierarchyConfig);
  const tierName = (primaryTier && primaryTier.types.length === 1) 
    ? primaryTier.types[0]
    : 'Items';
  
  return {
    id: createVirtualNodeId('bucket', range[0], range[1]),
    virtualType: {
      type: 'collapsed_primary_bucket',
      range,
      counts,
    },
    label: `${tierName} ${range[0] + 1}-${range[1] + 1}`,
  };
}

/**
 * Create a virtual node for a collapsed secondary group
 */
function createCollapsedSecondaryNode(
  secondaryNode: GraphNode,
  tacticalNodes: GraphNode[],
  hierarchyConfig: HierarchyConfig
): VirtualNode {
  const counts = countNodesByType(tacticalNodes, hierarchyConfig);
  
  return {
    id: createVirtualNodeId('collapsed', secondaryNode.id),
    virtualType: {
      type: 'collapsed_secondary_group',
      secondaryId: secondaryNode.id,
      counts,
    },
    label: secondaryNode.label,
  };
}

/**
 * Create a "more" node for overflow
 */
function createMoreNode(scope: string, count: number): VirtualNode {
  return {
    id: createVirtualNodeId('more', scope, count),
    virtualType: {
      type: 'more_node',
      scope,
      count,
    },
    label: `+${count} more ${scope}`,
  };
}

/**
 * Count nodes by type
 */
function countNodesByType(nodes: GraphNode[], hierarchyConfig: HierarchyConfig): NodeCounts {
  const counts: NodeCounts = { total: nodes.length };
  const primaryTier = getPrimaryTier(hierarchyConfig);
  const secondaryTier = getSecondaryTier(hierarchyConfig);
  const levels = getHierarchyLevels(hierarchyConfig);
  const tacticalLevels = levels.filter(l => 
    l !== primaryTier?.level && l !== secondaryTier?.level
  );

  let primaryCount = 0;
  let secondaryCount = 0;
  const tacticalCounts: Record<string, number> = {};

  for (const node of nodes) {
    const nodeType = extractNodeType(node);
    if (!nodeType) continue;

    const nodeLevel = hierarchyConfig.types[nodeType];
    if (nodeLevel === undefined) continue;

    if (primaryTier && nodeLevel === primaryTier.level) {
      primaryCount++;
    } else if (secondaryTier && nodeLevel === secondaryTier.level) {
      secondaryCount++;
    } else if (tacticalLevels.includes(nodeLevel)) {
      tacticalCounts[nodeType] = (tacticalCounts[nodeType] || 0) + 1;
    }
  }

  if (primaryCount > 0) counts.primary = primaryCount;
  if (secondaryCount > 0) counts.secondary = secondaryCount;
  if (Object.keys(tacticalCounts).length > 0) counts.tactical = tacticalCounts;

  return counts;
}

/**
 * Result of primary tier windowing
 */
interface WindowingResult {
  visibleNodes: GraphNode[];
  virtualBuckets: VirtualNode[];
}

/**
 * Apply primary tier windowing to collapse older tiers
 */
export function applyPrimaryTierWindowing(
  nodes: GraphNode[],
  hierarchyConfig: HierarchyConfig,
  windowConfig: WindowConfig,
  expandedBuckets: Set<number>
): WindowingResult {
  const primaryTier = getPrimaryTier(hierarchyConfig);
  const { visiblePrimaryTierCount } = windowConfig;

  // If no primary tier, no windowing
  if (!primaryTier) {
    return { visibleNodes: nodes, virtualBuckets: [] };
  }

  // Group nodes by primary tier index
  const grouped = groupNodesByPrimaryTier(nodes, hierarchyConfig);
  const indices = Array.from(grouped.keys()).sort((a, b) => a - b);

  // Separate unassigned nodes (index -1)
  const unassignedNodes = grouped.get(-1) || [];
  const assignedIndices = indices.filter(i => i >= 0);

  // If total tiers <= window size, show all
  if (assignedIndices.length <= visiblePrimaryTierCount) {
    return { visibleNodes: nodes, virtualBuckets: [] };
  }

  // Determine visible range: last N indices
  const visibleStartIndex = assignedIndices.length - visiblePrimaryTierCount;
  const visibleIndices = assignedIndices.slice(visibleStartIndex);
  const collapsedIndices = assignedIndices.slice(0, visibleStartIndex);

  // Check if collapsed range is expanded
  const bucketStart = collapsedIndices[0];
  const isExpanded = expandedBuckets.has(bucketStart);

  if (isExpanded) {
    // Show all nodes (bucket expanded)
    return { visibleNodes: nodes, virtualBuckets: [] };
  }

  // Create collapsed bucket for older tiers
  const collapsedNodes = collapsedIndices.flatMap(i => grouped.get(i) || []);
  const bucketNode = createPrimaryBucketNode(
    [collapsedIndices[0], collapsedIndices[collapsedIndices.length - 1]],
    collapsedNodes,
    hierarchyConfig
  );

  // Collect visible nodes
  const visibleNodes = [
    ...visibleIndices.flatMap(i => grouped.get(i) || []),
    ...unassignedNodes, // Always show unassigned
  ];

  return {
    visibleNodes,
    virtualBuckets: [bucketNode],
  };
}

/**
 * Result of progressive disclosure
 */
interface DisclosureResult {
  nodes: (GraphNode | VirtualNode)[];
}

/**
 * Apply progressive disclosure within a primary tier
 */
export function applyProgressiveDisclosure(
  nodesInTier: GraphNode[],
  allNodes: GraphNode[],
  edges: GraphEdge[],
  hierarchyConfig: HierarchyConfig,
  windowConfig: WindowConfig,
  expandedSecondaryGroups: Set<string>,
  selectedPath: Set<string>
): DisclosureResult {
  const primaryTier = getPrimaryTier(hierarchyConfig);
  const secondaryTier = getSecondaryTier(hierarchyConfig);
  const { tacticalBudget } = windowConfig;

  const result: (GraphNode | VirtualNode)[] = [];

  // Separate nodes by hierarchy level
  const primaryNodes = nodesInTier.filter(n => {
    const type = extractNodeType(n);
    return type && primaryTier && primaryTier.types.includes(type);
  });
  
  const secondaryNodes = nodesInTier.filter(n => {
    const type = extractNodeType(n);
    return type && secondaryTier && secondaryTier.types.includes(type);
  });
  
  // Tactical nodes = everything that's NOT primary or secondary
  const tacticalNodes = nodesInTier.filter(n => {
    const type = extractNodeType(n);
    if (!type) return false;
    if (primaryTier && primaryTier.types.includes(type)) return false;
    if (secondaryTier && secondaryTier.types.includes(type)) return false;
    return true;
  });

  // Always include primary tier nodes
  result.push(...primaryNodes);

  // If no secondary tier, collapse tactical items directly under primary
  if (!secondaryTier || secondaryNodes.length === 0) {
    // Show top-scored tactical items up to budget
    const ordered = orderNodesByRelevance(tacticalNodes, edges, selectedPath);
    const visible = ordered.slice(0, tacticalBudget);
    const remaining = ordered.length - visible.length;

    result.push(...visible);
    if (remaining > 0) {
      result.push(createMoreNode('items', remaining));
    }

    return { nodes: result };
  }

  // Group tactical nodes by their secondary parent
  const tacticalBySecondary = new Map<string, GraphNode[]>();
  
  for (const tacticalNode of tacticalNodes) {
    // Find which secondary this tactical belongs to by looking for matching label
    let parentSecondary: string | null = null;
    
    for (const secondaryNode of secondaryNodes) {
      // Try each secondary type to find matching label
      // If single type, use label namespace; otherwise use type name
      const secondaryType = extractNodeType(secondaryNode);
      if (!secondaryType) continue;

      const labelNamespace = 
        (secondaryTier?.labelNamespace) || 
        (hierarchyConfig.label_associations?.[secondaryType]) || 
        secondaryType;

      const secondaryLabel = secondaryNode.labels.find(l => l.startsWith(`${labelNamespace}:`));
      if (secondaryLabel && tacticalNode.labels.includes(secondaryLabel)) {
        parentSecondary = secondaryNode.id;
        break;
      }
    }
    
    if (parentSecondary) {
      if (!tacticalBySecondary.has(parentSecondary)) {
        tacticalBySecondary.set(parentSecondary, []);
      }
      tacticalBySecondary.get(parentSecondary)!.push(tacticalNode);
    }
  }

  // Process each secondary node
  for (const secondaryNode of secondaryNodes) {
    const childTacticalNodes = tacticalBySecondary.get(secondaryNode.id) || [];
    const isExpanded = expandedSecondaryGroups.has(secondaryNode.id);

    if (!isExpanded) {
      // Collapsed: show secondary node + collapsed group virtual node if it has children
      result.push(secondaryNode);
      if (childTacticalNodes.length > 0) {
        result.push(createCollapsedSecondaryNode(secondaryNode, childTacticalNodes, hierarchyConfig));
      }
    } else {
      // Expanded: show secondary + top-scored tactical items
      result.push(secondaryNode);
      
      const ordered = orderNodesByRelevance(childTacticalNodes, edges, selectedPath);
      const visible = ordered.slice(0, tacticalBudget);
      const remaining = ordered.length - visible.length;

      result.push(...visible);
      if (remaining > 0) {
        // Use generic scope since we don't track tactical tier names explicitly
        result.push(createMoreNode('items', remaining));
      }
    }
  }

  return { nodes: result };
}

/**
 * Build exploration graph from base graph
 */
export function buildExplorationGraph(
  baseGraph: { nodes: GraphNode[]; edges: GraphEdge[] },
  hierarchyConfig: HierarchyConfig,
  windowConfig: WindowConfig,
  expansionState: ExpansionState,
  selectedPath: Set<string>
): ExplorationGraph {
  const { nodes: baseNodes, edges: baseEdges } = baseGraph;

  // Step 1: Apply primary tier windowing
  const windowingResult = applyPrimaryTierWindowing(
    baseNodes,
    hierarchyConfig,
    windowConfig,
    expansionState.expandedPrimaryBuckets
  );

  const explorationNodes: (GraphNode | VirtualNode)[] = [];
  
  // Add virtual bucket nodes first
  explorationNodes.push(...windowingResult.virtualBuckets);

  // Step 2: Group visible nodes by primary tier and apply progressive disclosure
  const grouped = groupNodesByPrimaryTier(windowingResult.visibleNodes, hierarchyConfig);
  
  for (const nodesInTier of grouped.values()) {
    const disclosureResult = applyProgressiveDisclosure(
      nodesInTier,
      baseNodes,
      baseEdges,
      hierarchyConfig,
      windowConfig,
      expansionState.expandedSecondaryGroups,
      selectedPath
    );
    
    explorationNodes.push(...disclosureResult.nodes);
  }

  // Step 3: Filter edges (only between visible real nodes for now)
  const visibleNodeIds = new Set(
    explorationNodes
      .filter(n => !isVirtualNode(n))
      .map(n => n.id)
  );

  const filteredEdges = baseEdges.filter(
    edge => visibleNodeIds.has(edge.from) && visibleNodeIds.has(edge.to)
  );

  // Step 4: Create metadata for virtual nodes (edge bundling - simplified for Phase 1)
  const virtualNodeMetadata = new Map<string, VirtualNodeMetadata>();
  for (const node of explorationNodes) {
    if (isVirtualNode(node)) {
      virtualNodeMetadata.set(node.id, {
        virtualNodeId: node.id,
        inboundEdgeCount: 0,
        outboundEdgeCount: 0,
        inboundFrom: new Set(),
        outboundTo: new Set(),
      });
    }
  }

  return {
    nodes: explorationNodes,
    edges: filteredEdges,
    virtualNodeMetadata,
  };
}
