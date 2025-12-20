import type { GraphNode, GraphEdge } from '../types/models';

// Default strategic types - should be fetched from API/config in the future
const DEFAULT_STRATEGIC_TYPES = ['milestone', 'epic'];

/**
 * Filter configuration for graph visualization
 * Supports multiple independent filter types that can be combined
 */
export interface GraphFilter {
  type: 'strategic' | 'label';
  config: StrategicFilterConfig | LabelFilterConfig;
}

export interface StrategicFilterConfig {
  enabled: boolean;
  strategicTypes?: string[]; // Optional: defaults to DEFAULT_STRATEGIC_TYPES
}

export interface LabelFilterConfig {
  patterns: string[]; // e.g., ["milestone:v1.0", "epic:*"]
}

/**
 * Result of applying filters to a node
 */
export interface NodeFilterResult {
  visible: boolean;    // Should the node be shown at all
  dimmed: boolean;     // Should the node be grayed out
  reason?: string;     // Why the node is filtered (for debugging)
}

/**
 * Result of applying filters to an edge
 */
export interface EdgeFilterResult {
  visible: boolean;
  dimmed: boolean;
}

/**
 * Apply a list of filters to a node
 * Returns aggregated result based on all active filters
 */
export function applyFiltersToNode(
  node: GraphNode,
  filters: GraphFilter[]
): NodeFilterResult {
  if (filters.length === 0) {
    return { visible: true, dimmed: false };
  }

  let shouldDim = false;
  let shouldHide = false;

  for (const filter of filters) {
    if (filter.type === 'strategic') {
      const config = filter.config as StrategicFilterConfig;
      if (config.enabled) {
        const strategicTypes = config.strategicTypes || DEFAULT_STRATEGIC_TYPES;
        // Strategic filter HIDES non-strategic nodes
        const isStrategic = node.labels.some(label => {
          if (label.startsWith('type:')) {
            const typeValue = label.substring(5);
            return strategicTypes.includes(typeValue);
          }
          return false;
        });
        if (!isStrategic) {
          shouldHide = true;
        }
      }
    } else if (filter.type === 'label') {
      const config = filter.config as LabelFilterConfig;
      if (config.patterns.length > 0) {
        // Label filter DIMS non-matching nodes
        const matches = matchesAnyPattern(node.labels, config.patterns);
        if (!matches) {
          shouldDim = true;
        }
      }
    }
  }

  return {
    visible: !shouldHide,
    dimmed: shouldDim,
    reason: shouldHide ? 'hidden by strategic filter' : shouldDim ? 'dimmed by label filter' : undefined,
  };
}

/**
 * Apply filters to an edge based on its source and target node states
 */
export function applyFiltersToEdge(
  _edge: GraphEdge,
  sourceResult: NodeFilterResult,
  targetResult: NodeFilterResult
): EdgeFilterResult {
  // Hide edge if either node is hidden
  if (!sourceResult.visible || !targetResult.visible) {
    return { visible: false, dimmed: false };
  }

  // Dim edge if either node is dimmed
  const shouldDim = sourceResult.dimmed || targetResult.dimmed;

  return { visible: true, dimmed: shouldDim };
}

/**
 * Check if any of the node's labels match any of the filter patterns
 * Supports exact match and wildcard patterns (e.g., "milestone:*")
 */
export function matchesAnyPattern(labels: string[], patterns: string[]): boolean {
  if (patterns.length === 0) {
    return true; // No patterns means no filtering
  }

  return patterns.some(pattern => 
    labels.some(label => matchesPattern(label, pattern))
  );
}

/**
 * Check if a label matches a pattern
 * Supports exact match and wildcard (e.g., "milestone:*" matches "milestone:v1.0")
 */
export function matchesPattern(label: string, pattern: string): boolean {
  if (pattern.endsWith('*')) {
    const prefix = pattern.slice(0, -1);
    return label.startsWith(prefix);
  }
  return label === pattern;
}

/**
 * Create a strategic filter configuration
 */
export function createStrategicFilter(enabled: boolean): GraphFilter {
  return {
    type: 'strategic',
    config: { enabled },
  };
}

/**
 * Create a label filter configuration
 */
export function createLabelFilter(patterns: string[]): GraphFilter {
  return {
    type: 'label',
    config: { patterns },
  };
}
