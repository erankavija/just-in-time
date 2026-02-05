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

const CLUSTER_SPACING = 200;
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
  ];
  
  allNodeIds.forEach(id => {
    incomingEdges.set(id, new Set());
    outgoingEdges.set(id, new Set());
  });
  
  // Build edge maps from ALL edges (cross-cluster + edges involving orphans)
  const relevantEdges = [...crossClusterEdges];
  
  // Add edges involving orphan nodes
  orphanNodes.forEach(orphan => {
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
    if (allNodeIds.includes(edge.from) && allNodeIds.includes(edge.to)) {
      // from depends on to, so to has an outgoing edge to from
      outgoingEdges.get(edge.to)?.add(edge.from);
      incomingEdges.get(edge.from)?.add(edge.to);
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
  
  while (queue.length > 0) {
    const current = queue.shift()!;
    sorted.push(current);
    
    // Process outgoing edges
    outgoingEdges.get(current)?.forEach(dependent => {
      const newDegree = inDegree.get(dependent)! - 1;
      inDegree.set(dependent, newDegree);
      if (newDegree === 0) {
        queue.push(dependent);
      }
    });
  }
  
  // If not all nodes were sorted, we have a cycle - fall back to original order
  if (sorted.length !== allNodeIds.length) {
    sorted.length = 0;
    sorted.push(...allNodeIds);
  }
  
  // Assign X positions based on sorted order
  const positions = new Map<string, ClusterPosition>();
  let currentX = 0;
  
  sorted.forEach(nodeId => {
    positions.set(nodeId, {
      x: currentX,
      y: 0, // Will be set later if needed
      width: 0, // Will be calculated after node layout
      height: 0,
    });
    currentX += CLUSTER_SPACING; // Placeholder, will adjust after calculating cluster width
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
  
  // Phase 3: Adjust X positions based on actual widths
  const sortedNodeIds = Array.from(clusterPositions.keys());
  let currentX = 0;
  sortedNodeIds.forEach(nodeId => {
    const pos = clusterPositions.get(nodeId)!;
    pos.x = currentX;
    const width = clusterLayouts.has(nodeId) 
      ? clusterLayouts.get(nodeId)!.width 
      : (NODE_WIDTH + 2 * CLUSTER_PADDING);
    currentX += width + CLUSTER_SPACING;
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
