import type { GraphNode, GraphEdge } from './models';

/**
 * Maps issue type names to their hierarchy level numbers.
 * Lower numbers = more strategic (e.g., milestone=1, epic=2, story=3, task=4)
 */
export interface HierarchyLevelMap {
  [typeName: string]: number;
}

/**
 * A cluster of nodes organized around a container node (e.g., epic).
 * Contains all lower-level nodes that depend on or are depended by the container.
 */
export interface SubgraphCluster {
  /** The container node (e.g., epic) that defines this cluster */
  containerId: string;
  
  /** Hierarchy level of the container (e.g., 2 for epic) */
  containerLevel: number;
  
  /** All nodes in this cluster (including the container) */
  nodes: GraphNode[];
  
  /** Edges between nodes within this cluster */
  internalEdges: GraphEdge[];
  
  /** Edges from this cluster to other clusters */
  outgoingEdges: GraphEdge[];
  
  /** Edges from other clusters to this cluster */
  incomingEdges: GraphEdge[];
}

/**
 * Tracks which containers are collapsed or expanded.
 * Key: node ID, Value: true = expanded, false = collapsed
 */
export interface ExpansionState {
  [nodeId: string]: boolean;
}

/**
 * A virtual edge created by aggregating child edges when a container is collapsed.
 */
export interface VirtualEdge {
  /** Source node ID (may be container or child) */
  from: string;
  
  /** Target node ID (may be container or child) */
  to: string;
  
  /** Number of actual edges this virtual edge represents */
  count: number;
  
  /** Original edge IDs that were aggregated */
  sourceEdgeIds: string[];
}

/**
 * Result of subgraph assignment - clusters plus edges that cross cluster boundaries
 */
export interface ClusteredGraph {
  /** Map of container ID to its subgraph cluster */
  clusters: Map<string, SubgraphCluster>;
  
  /** Edges that cross between different clusters */
  crossClusterEdges: GraphEdge[];
  
  /** Nodes that don't belong to any cluster (orphans) */
  orphanNodes: GraphNode[];
}
