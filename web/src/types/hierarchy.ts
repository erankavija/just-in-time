/**
 * Hierarchy configuration derived from strategic types
 */
export interface HierarchyConfig {
  /** Ordered list of strategic types from API */
  strategicTypes: string[];
  /** First strategic type (tier 0) - e.g., 'milestone', 'release', 'program' */
  primaryTier: string | null;
  /** Second strategic type (tier 1) - e.g., 'epic', 'milestone' */
  secondaryTier: string | null;
  /** All remaining types (tier 2+) - e.g., ['story', 'task', 'bug', 'enhancement'] */
  tacticalTiers: string[];
}

/**
 * Create hierarchy config from strategic types list
 */
export function createHierarchyConfig(strategicTypes: string[]): HierarchyConfig {
  return {
    strategicTypes,
    primaryTier: strategicTypes[0] || null,
    secondaryTier: strategicTypes[1] || null,
    tacticalTiers: strategicTypes.slice(2),
  };
}

/**
 * Virtual node types for exploration graph
 */
export type VirtualNodeType =
  | {
      type: 'collapsed_primary_bucket';
      range: [number, number]; // [startIndex, endIndex]
      counts: NodeCounts;
    }
  | {
      type: 'collapsed_secondary_group';
      secondaryId: string;
      counts: NodeCounts;
    }
  | {
      type: 'more_node';
      scope: string; // e.g., 'stories', 'tasks'
      count: number;
    };

export interface NodeCounts {
  primary?: number;
  secondary?: number;
  tactical?: Record<string, number>; // e.g., { story: 5, task: 12, bug: 3 }
  total: number;
}
