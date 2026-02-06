import dagre from 'dagre';
import type { SubgraphCluster } from '../types/subgraphCluster';
import type { GraphNode, GraphEdge } from '../types/graph';

interface ClusterPosition {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface NodeWithPosition extends GraphNode {
  position: { x: number; y: number };
}

interface ClusterLayout {
  nodes: NodeWithPosition[];
  width: number;
  height: number;
}

interface ClusterAwareLayoutResult {
  nodes: NodeWithPosition[];
  clusters: Map<string, ClusterPosition>;
}

const CLUSTER_SPACING = 100; // Spacing between clusters (horizontal and vertical)
const NODE_WIDTH = 180;
const NODE_HEIGHT = 60;
const NODE_SPACING_Y = 20;
const CLUSTER_PADDING = 40;

/**
 * Compute horizontal positions for clusters AND orphan nodes based on topological sort.
 * Uses Kahn's algorithm for topological ordering of cross-cluster dependencies.
 * 
 * In jit: A→B means "A depends on B" (B must complete first)
 * So B should be positioned LEFT of A (dependencies on left, dependents on right)
 */
export function computeClusterPositions(
  clusters: SubgraphCluster[],
  crossClusterEdges: GraphEdge[],
  orphanNodes: GraphNode[] = [],
  allEdges: GraphEdge[] = []
): Map<string, ClusterPosition> {
  // Build adjacency map: who depends on whom
  // edge from→to means "from depends on to", so to must come before from
  const incomingEdges = new Map<string, Set<string>>();
  const outgoingEdges = new Map<string, Set<string>>();
  
  // Include both clusters and orphan nodes
  const allNodeIds = [
    ...clusters.map(c => c.containerId),
    ...orphanNodes.map(n => n.id)
  ].sort(); // Sort for deterministic order
  
  allNodeIds.forEach(id => {
    incomingEdges.set(id, new Set());
    outgoingEdges.set(id, new Set());
  });
  
  // Build a map from node ID to cluster ID
  const nodeToCluster = new Map<string, string>();
  clusters.forEach(cluster => {
    nodeToCluster.set(cluster.containerId, cluster.containerId); // Container maps to itself
    cluster.nodes.forEach(node => {
      nodeToCluster.set(node.id, cluster.containerId); // Tasks map to their cluster
    });
  });
  orphanNodes.forEach(orphan => {
    nodeToCluster.set(orphan.id, orphan.id); // Orphans map to themselves
  });
  
  // Build edge maps from ALL edges (cross-cluster + edges involving orphans + cluster boundary edges)
  const relevantEdges = [...crossClusterEdges];
  
  // Add cluster incoming/outgoing edges for proper positioning (sorted by cluster ID for stability)
  [...clusters].sort((a, b) => a.containerId.localeCompare(b.containerId)).forEach(cluster => {
    if (cluster.incomingEdges) {
      relevantEdges.push(...cluster.incomingEdges);
    }
    if (cluster.outgoingEdges) {
      relevantEdges.push(...cluster.outgoingEdges);
    }
  });
  
  // Add edges involving orphan nodes (sorted for stability)
  [...orphanNodes].sort((a, b) => a.id.localeCompare(b.id)).forEach(orphan => {
    allEdges.forEach(edge => {
      if (edge.from === orphan.id && allNodeIds.includes(edge.to)) {
        relevantEdges.push(edge);
      }
      if (edge.to === orphan.id && allNodeIds.includes(edge.from)) {
        relevantEdges.push(edge);
      }
    });
  });
  
  relevantEdges.forEach(edge => {
    // Map task IDs to cluster IDs
    const fromCluster = nodeToCluster.get(edge.from);
    const toCluster = nodeToCluster.get(edge.to);
    
    if (fromCluster && toCluster && allNodeIds.includes(fromCluster) && allNodeIds.includes(toCluster)) {
      // from depends on to, so to has an outgoing edge to from
      outgoingEdges.get(toCluster)?.add(fromCluster);
      incomingEdges.get(fromCluster)?.add(toCluster);
    }
  });
  
  // Kahn's algorithm: start with nodes that have no incoming edges (roots)
  const queue: string[] = [];
  const sorted: string[] = [];
  const inDegree = new Map<string, number>();
  
  allNodeIds.forEach(id => {
    const degree = incomingEdges.get(id)!.size;
    inDegree.set(id, degree);
    if (degree === 0) {
      queue.push(id);
    }
  });
  
  // Assign ranks (columns) using longest path from root
  // Nodes with no dependencies get rank 0
  // Each node's rank = max(dependency ranks) + 1
  const ranks = new Map<string, number>();
  const visiting = new Set<string>(); // Track nodes being visited (cycle detection)
  
  const calculateRank = (nodeId: string): number => {
    if (ranks.has(nodeId)) {
      return ranks.get(nodeId)!;
    }
    
    // Cycle detection: if we're already visiting this node, there's a cycle
    if (visiting.has(nodeId)) {
      ranks.set(nodeId, 0); // Treat as root to break cycle
      return 0;
    }
    
    const deps = incomingEdges.get(nodeId)!;
    if (deps.size === 0) {
      ranks.set(nodeId, 0);
      return 0;
    }
    
    visiting.add(nodeId);
    const depRanks = Array.from(deps).sort().map(d => calculateRank(d)).filter(r => !isNaN(r));
    visiting.delete(nodeId);
    
    const maxDepRank = depRanks.length > 0 ? Math.max(...depRanks) : -1;
    const rank = maxDepRank + 1;
    ranks.set(nodeId, rank);
    return rank;
  };
  
  // Calculate rank for all nodes (in sorted order for determinism)
  [...allNodeIds].sort().forEach(id => calculateRank(id));
  
  // Group nodes by rank
  const nodesByRank = new Map<number, string[]>();
  ranks.forEach((rank, nodeId) => {
    if (!nodesByRank.has(rank)) {
      nodesByRank.set(rank, []);
    }
    nodesByRank.get(rank)!.push(nodeId);
  });
  
  // Sort nodes within each rank
  nodesByRank.forEach((nodes, rank) => {
    if (rank === 0) {
      // Rank 0: sort by number of dependents (most dependents = leftmost in grid)
      nodes.sort((a, b) => {
        const aDeps = outgoingEdges.get(a)!.size;
        const bDeps = outgoingEdges.get(b)!.size;
        if (aDeps !== bDeps) return bDeps - aDeps; // More dependents first
        return a.localeCompare(b); // Stable sort by ID
      });
    } else {
      // Other ranks: sort by ID for stability
      nodes.sort();
    }
  });
  
  // Assign positions based on ranks
  // Rank 0 (independent nodes): use grid layout to spread horizontally
  // Other ranks: position to the right, ensuring dependency flow
  const positions = new Map<string, ClusterPosition>();
  
  const rank0Nodes = nodesByRank.get(0) || [];
  let gridWidth = 0;
  
  if (rank0Nodes.length > 0) {
    // Simple initial grid layout - will be adjusted in Phase 3 with actual widths
    const cols = Math.min(5, Math.ceil(Math.sqrt(rank0Nodes.length)));
    
    rank0Nodes.forEach((nodeId, index) => {
      const col = index % cols;
      const row = Math.floor(index / cols);
      positions.set(nodeId, {
        x: col * 1000, // Initial spacing - will be adjusted
        y: row * 500,  // Initial spacing - will be adjusted
        width: 0,
        height: 0,
      });
    });
    
    gridWidth = cols * 1000; // Estimate - will be refined in Phase 3
  }
  
  // Position other ranks AFTER the grid (initial positions, refined in Phase 3)
  nodesByRank.forEach((nodesInRank, rank) => {
    if (rank === 0) return; // Already handled
    
    const x = gridWidth + rank * 1000;
    
    nodesInRank.forEach((nodeId, index) => {
      positions.set(nodeId, {
        x,
        y: index * 500,
        width: 0,
        height: 0,
      });
    });
  });
  
  return positions;
}

/**
 * Layout nodes within a cluster using dagre with tight constraints.
 * Uses dependency-aware layout but keeps nodes compact.
 */
export function layoutNodesWithinCluster(
  nodes: GraphNode[],
  internalEdges: GraphEdge[]
): ClusterLayout {
  const dagreGraph = new dagre.graphlib.Graph();
  dagreGraph.setDefaultEdgeLabel(() => ({}));
  dagreGraph.setGraph({ 
    rankdir: 'LR',
    nodesep: 60,  // Vertical spacing between nodes in same rank
    ranksep: 150, // Horizontal spacing between ranks (columns)
    ranker: 'longest-path', // Use longest path for better topological ordering
    marginx: CLUSTER_PADDING,
    marginy: CLUSTER_PADDING,
  });
  
  // Add nodes
  nodes.forEach(node => {
    dagreGraph.setNode(node.id, { 
      width: NODE_WIDTH, 
      height: NODE_HEIGHT 
    });
  });
  
  // Add edges (reversed: in jit A→B means A depends on B, so B comes first)
  internalEdges.forEach(edge => {
    dagreGraph.setEdge(edge.to, edge.from);
  });
  
  dagre.layout(dagreGraph);
  
  // Extract positioned nodes and calculate bounds
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  
  const positionedNodes: NodeWithPosition[] = nodes.map(node => {
    const pos = dagreGraph.node(node.id);
    const x = pos.x - NODE_WIDTH / 2;
    const y = pos.y - NODE_HEIGHT / 2;
    
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x + NODE_WIDTH);
    maxY = Math.max(maxY, y + NODE_HEIGHT);
    
    return {
      ...node,
      position: { x, y },
    };
  });
  
  const width = maxX - minX + 2 * CLUSTER_PADDING;
  const height = maxY - minY + 2 * CLUSTER_PADDING;
  
  // Normalize positions (shift to start at CLUSTER_PADDING)
  const normalizedNodes = positionedNodes.map(node => ({
    ...node,
    position: {
      x: node.position.x - minX + CLUSTER_PADDING,
      y: node.position.y - minY + CLUSTER_PADDING,
    },
  }));
  
  return {
    nodes: normalizedNodes,
    width,
    height,
  };
}

/**
 * Create a cluster-aware layout for the entire graph.
 * Phase 1: Position clusters AND orphan nodes based on dependencies
 * Phase 2: Layout nodes within each cluster
 * Phase 3: Combine into final layout
 */
export function createClusterAwareLayout(
  clusters: SubgraphCluster[],
  crossClusterEdges: GraphEdge[],
  orphanNodes: GraphNode[] = [],
  allEdges: GraphEdge[] = []
): ClusterAwareLayoutResult {
  // Phase 1: Compute cluster and orphan positions
  const clusterPositions = computeClusterPositions(clusters, crossClusterEdges, orphanNodes, allEdges);
  
  // Phase 2: Layout nodes within clusters
  const clusterLayouts = new Map<string, ClusterLayout>();
  clusters.forEach(cluster => {
    const layout = layoutNodesWithinCluster(cluster.nodes, cluster.internalEdges);
    clusterLayouts.set(cluster.containerId, layout);
    
    // Update cluster position with actual dimensions
    const pos = clusterPositions.get(cluster.containerId)!;
    pos.width = layout.width;
    pos.height = layout.height;
  });
  
  // Set dimensions for orphan nodes
  orphanNodes.forEach(orphan => {
    const pos = clusterPositions.get(orphan.id);
    if (pos) {
      pos.width = NODE_WIDTH + 2 * CLUSTER_PADDING;
      pos.height = NODE_HEIGHT + 2 * CLUSTER_PADDING;
    }
  });
  
  // Phase 3: Adjust positions based on actual widths and heights
  // The grid layout for rank 0 needs column widths adjusted based on actual cluster sizes
  const positionsByRank = new Map<number, Array<{id: string, pos: ClusterPosition}>>();
  
  clusterPositions.forEach((pos, id) => {
    // Nodes in grid (x < 5000) stay in grid, others are grouped by rank
    if (pos.x < 5000) { // Grid area (5 cols * 1000 max)
      const gridRank = -1; // Special marker for grid nodes
      if (!positionsByRank.has(gridRank)) {
        positionsByRank.set(gridRank, []);
      }
      positionsByRank.get(gridRank)!.push({id, pos});
    } else {
      // Ranked nodes beyond the grid
      const rank = Math.round(pos.x / 1000);
      if (!positionsByRank.has(rank)) {
        positionsByRank.set(rank, []);
      }
      positionsByRank.get(rank)!.push({id, pos});
    }
  });
  
  // For each rank, calculate max width and adjust X position
  const rankWidths = new Map<number, number>();
  positionsByRank.forEach((items, rank) => {
    if (rank === -1) {
      // Grid: width spans multiple columns, calculate overall grid width
      const maxX = Math.max(...items.map(({pos}) => pos.x));
      const maxWidth = Math.max(...items.map(({id}) => {
        return clusterLayouts.has(id) 
          ? clusterLayouts.get(id)!.width 
          : (NODE_WIDTH + 2 * CLUSTER_PADDING);
      }));
      rankWidths.set(rank, maxX + maxWidth); // Total grid width
    } else {
      const maxWidth = Math.max(...items.map(({id}) => {
        return clusterLayouts.has(id) 
          ? clusterLayouts.get(id)!.width 
          : (NODE_WIDTH + 2 * CLUSTER_PADDING);
      }));
      rankWidths.set(rank, maxWidth);
    }
  });
  
  // Adjust X positions: grid stays in place, ranked nodes positioned after grid
  let cumulativeX = 0;
  const sortedRanks = Array.from(rankWidths.keys()).sort((a, b) => a - b);
  
  sortedRanks.forEach(rank => {
    const items = positionsByRank.get(rank)!;
    
    if (rank === -1) {
      // Grid nodes: adjust column positions based on actual widths
      const byColumn = new Map<number, Array<{id: string, pos: ClusterPosition}>>();
      items.forEach(item => {
        const col = Math.round(item.pos.x / 1000);
        if (!byColumn.has(col)) {
          byColumn.set(col, []);
        }
        byColumn.get(col)!.push(item);
      });
      
      // Calculate max width per column
      const columnWidths = new Map<number, number>();
      byColumn.forEach((colItems, col) => {
        const maxWidth = Math.max(...colItems.map(({id, pos}) => {
          // Use cluster layout width if available, otherwise use pos.width (for orphans)
          return clusterLayouts.has(id) 
            ? clusterLayouts.get(id)!.width 
            : (pos.width || NODE_WIDTH + 2 * CLUSTER_PADDING);
        }));
        columnWidths.set(col, maxWidth);
      });
      
      // Position columns with proper spacing
      let colX = 0;
      const sortedCols = Array.from(columnWidths.keys()).sort((a, b) => a - b);
      const colPositions = new Map<number, number>();
      
      sortedCols.forEach(col => {
        colPositions.set(col, colX);
        colX += columnWidths.get(col)! + CLUSTER_SPACING;
      });
      
      // Apply column positions and adjust Y spacing
      byColumn.forEach((colItems, col) => {
        // Sort by ID for stable ordering
        colItems.sort((a, b) => a.id.localeCompare(b.id));
        const x = colPositions.get(col)!;
        let currentY = 0;
        
        colItems.forEach(({id, pos}) => {
          pos.x = x;
          pos.y = currentY;
          const height = clusterLayouts.has(id)
            ? clusterLayouts.get(id)!.height
            : (NODE_HEIGHT + 2 * CLUSTER_PADDING);
          currentY += height + CLUSTER_SPACING;
        });
      });
      
      cumulativeX = colX; // Start ranked nodes after grid
    } else {
      // Ranked nodes: position horizontally after grid
      items.forEach(({pos}) => {
        pos.x = cumulativeX;
      });
      cumulativeX += rankWidths.get(rank)! + CLUSTER_SPACING;
      
      // Adjust Y positions within rank
      items.sort((a, b) => a.id.localeCompare(b.id)); // Sort by ID for stability
      let currentY = 0;
      items.forEach(({id, pos}) => {
        pos.y = currentY;
        const height = clusterLayouts.has(id)
          ? clusterLayouts.get(id)!.height
          : (NODE_HEIGHT + 2 * CLUSTER_PADDING);
        currentY += height + CLUSTER_SPACING;
      });
    }
  });
  
  // Phase 4: Combine all nodes with absolute positions
  const allNodes: NodeWithPosition[] = [];
  
  // Add cluster nodes
  clusters.forEach(cluster => {
    const clusterPos = clusterPositions.get(cluster.containerId)!;
    const layout = clusterLayouts.get(cluster.containerId)!;
    
    layout.nodes.forEach(node => {
      allNodes.push({
        ...node,
        position: {
          x: clusterPos.x + node.position.x,
          y: clusterPos.y + node.position.y,
        },
      });
    });
  });
  
  // Add orphan nodes
  orphanNodes.forEach(orphan => {
    const pos = clusterPositions.get(orphan.id);
    if (pos) {
      allNodes.push({
        ...orphan,
        position: {
          x: pos.x + CLUSTER_PADDING,
          y: pos.y + CLUSTER_PADDING,
        },
      });
    }
  });
  
  return {
    nodes: allNodes,
    clusters: clusterPositions,
  };
}
