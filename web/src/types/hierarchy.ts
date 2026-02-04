/**
 * Hierarchy configuration matching JIT's actual config.toml structure
 */
export interface HierarchyConfig {
  /** Type name to hierarchy level mapping (lower = more strategic) */
  types: Record<string, number>;
  /** Types considered strategic (used for filtering) */
  strategic_types: string[];
  /** Type to label namespace mapping (e.g., epic → "epic") */
  label_associations?: Record<string, string>;
}

/**
 * Derived tier information for a specific hierarchy level
 */
export interface TierInfo {
  /** Hierarchy level (1 = most strategic) */
  level: number;
  /** All type names at this level (e.g., ['task', 'bug'] at level 4) */
  types: string[];
  /** Label namespace for this tier (if single type and has association) */
  labelNamespace?: string;
}

/**
 * Create hierarchy config for testing
 * @param typesByLevel - Map of level → type names (e.g., { 1: ['milestone'], 2: ['epic'] })
 */
export function createHierarchyConfig(
  typesByLevel: Record<number, string[]>,
  strategicTypes?: string[]
): HierarchyConfig {
  const types: Record<string, number> = {};
  
  for (const [levelStr, typeNames] of Object.entries(typesByLevel)) {
    const level = parseInt(levelStr, 10);
    for (const typeName of typeNames) {
      types[typeName] = level;
    }
  }

  // Auto-derive strategic types from levels 1-2 if not provided
  const derived = strategicTypes || Object.entries(types)
    .filter(([, level]) => level <= 2)
    .map(([type]) => type);

  return {
    types,
    strategic_types: derived,
  };
}

/**
 * Get sorted list of hierarchy levels (lowest first)
 */
export function getHierarchyLevels(config: HierarchyConfig): number[] {
  const levels = new Set(Object.values(config.types));
  return Array.from(levels).sort((a, b) => a - b);
}

/**
 * Get all types at a specific hierarchy level
 */
export function getTypesAtLevel(config: HierarchyConfig, level: number): string[] {
  return Object.entries(config.types)
    .filter(([, l]) => l === level)
    .map(([type]) => type);
}

/**
 * Get tier info for a specific level
 */
export function getTierInfo(config: HierarchyConfig, level: number): TierInfo | null {
  const types = getTypesAtLevel(config, level);
  if (types.length === 0) return null;

  const tierInfo: TierInfo = { level, types };

  // If single type with label association, include namespace
  if (types.length === 1 && config.label_associations) {
    const namespace = config.label_associations[types[0]];
    if (namespace) {
      tierInfo.labelNamespace = namespace;
    }
  }

  return tierInfo;
}

/**
 * Get the primary tier (lowest level)
 */
export function getPrimaryTier(config: HierarchyConfig): TierInfo | null {
  const levels = getHierarchyLevels(config);
  if (levels.length === 0) return null;
  return getTierInfo(config, levels[0]);
}

/**
 * Get the secondary tier (second lowest level)
 */
export function getSecondaryTier(config: HierarchyConfig): TierInfo | null {
  const levels = getHierarchyLevels(config);
  if (levels.length < 2) return null;
  return getTierInfo(config, levels[1]);
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
