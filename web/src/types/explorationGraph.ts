import type { GraphNode, GraphEdge } from '../types/models';
import type { VirtualNodeType } from '../types/hierarchy';

/**
 * Virtual node in the exploration graph
 */
export interface VirtualNode {
  id: string;  // Generated ID (e.g., "bucket-0-4", "collapsed-epic-auth")
  virtualType: VirtualNodeType;
  label: string;  // Display label (e.g., "Milestones 1-5", "+12 more stories")
}

/**
 * Metadata for virtual nodes (edge bundling info)
 */
export interface VirtualNodeMetadata {
  virtualNodeId: string;
  inboundEdgeCount: number;   // Edges coming into this virtual node
  outboundEdgeCount: number;  // Edges going out from this virtual node
  inboundFrom: Set<string>;   // Node IDs that point to this virtual node
  outboundTo: Set<string>;    // Node IDs this virtual node points to
}

/**
 * Configuration for windowing and budgets
 */
export interface WindowConfig {
  /** How many primary tier items to show (e.g., last 10 milestones) */
  visiblePrimaryTierCount: number;
  /** Max secondary tier items per primary tier (e.g., max 20 epics per milestone) */
  secondaryTierBudget: number;
  /** Max tactical items per secondary tier (e.g., max 8 stories per epic) */
  tacticalBudget: number;
}

/**
 * User expansion state
 */
export interface ExpansionState {
  /** Which primary tier buckets user has expanded (by start index) */
  expandedPrimaryBuckets: Set<number>;
  /** Which secondary tier groups user has expanded (by node ID) */
  expandedSecondaryGroups: Set<string>;
}

/**
 * Result of building exploration graph
 */
export interface ExplorationGraph {
  /** Mix of real nodes and virtual nodes */
  nodes: (GraphNode | VirtualNode)[];
  /** Edges between visible nodes */
  edges: GraphEdge[];
  /** Metadata for virtual nodes (edge bundling info) */
  virtualNodeMetadata: Map<string, VirtualNodeMetadata>;
}

/**
 * Helper to check if a node is virtual
 */
export function isVirtualNode(node: GraphNode | VirtualNode): node is VirtualNode {
  return 'virtualType' in node;
}

/**
 * Helper to create a virtual node ID
 */
export function createVirtualNodeId(prefix: string, ...parts: (string | number)[]): string {
  return `${prefix}-${parts.join('-')}`;
}
