import { useEffect, useState, useCallback, useRef } from 'react';
import ReactFlow, {
  Controls,
  Background,
  useNodesState,
  useEdgesState,
  MarkerType,
  Position,
  type Node,
  type Edge,
  type NodeTypes,
} from 'reactflow';
import dagre from 'dagre';
import 'reactflow/dist/style.css';
import { apiClient } from '../../api/client';
import type { State, Priority, GraphNode as ApiGraphNode, GraphEdge } from '../../types/models';
import type { SubgraphCluster } from '../../types/subgraphCluster';
import { calculateDownstreamStats, type DownstreamStats } from '../../utils/strategicView';
import { applyFiltersToNode, applyFiltersToEdge, createStrategicFilter, createLabelFilter, type GraphFilter } from '../../utils/graphFilter';
import type { HierarchyConfig, ExpansionState } from '../../types/subgraphCluster';
import { prepareClusteredGraphForReactFlow } from '../../utils/clusteredGraphLayout';
import { createClusterAwareLayout } from '../../utils/clusterAwareLayout';
import { findParentClusters } from '../../utils/graphFocus';
import ClusterNode from './nodes/ClusterNode';


// State colors using CSS variables
const stateColors: Record<State, string> = {
  backlog: 'var(--state-backlog)',
  ready: 'var(--state-ready)',
  in_progress: 'var(--state-in-progress)',
  gated: 'var(--state-gated)',
  done: 'var(--state-done)',
  rejected: 'var(--state-rejected)',
  archived: 'var(--state-archived)',
};

const priorityIcons: Record<Priority, string> = {
  critical: '●',
  high: '●',
  normal: '●',
  low: '●',
};

const priorityColors: Record<Priority, string> = {
  critical: 'var(--error)',
  high: 'var(--warning)',
  normal: 'var(--info)',
  low: 'var(--success)',
};

// Layout algorithm types
export type LayoutAlgorithm = 'dagre' | 'compact';

// Custom node types for ReactFlow
const nodeTypes: NodeTypes = {
  cluster: ClusterNode,
};

// ReactFlow options (defined outside component to avoid recreating on each render)
const proOptions = { hideAttribution: true };
const backgroundStyle = { backgroundColor: 'var(--bg-primary)' };

// Layout configuration
const LAYOUT_CONFIG = {
  nodeWidth: 220,
  nodeHeight: 100,  // Slightly taller to account for labels
  rankSpacing: 280, // Horizontal space between ranks (column width)
  nodeSpacing: 20,  // Vertical space between nodes
  maxNodesPerColumn: 6, // Max nodes per column before starting new column in same rank
  columnSpacing: 240, // Horizontal space between columns within same rank
  margin: 40,
};

// Dagre layout algorithm
const getDagreLayout = (nodes: Node[], edges: Edge[]) => {
  const dagreGraph = new dagre.graphlib.Graph();
  dagreGraph.setDefaultEdgeLabel(() => ({}));
  dagreGraph.setGraph({ 
    rankdir: 'LR',
    nodesep: 80,
    ranksep: 120,
    marginx: 40,
    marginy: 40,
  });

  nodes.forEach((node) => {
    dagreGraph.setNode(node.id, { width: LAYOUT_CONFIG.nodeWidth, height: LAYOUT_CONFIG.nodeHeight });
  });

  edges.forEach((edge) => {
    dagreGraph.setEdge(edge.source, edge.target);
  });

  dagre.layout(dagreGraph);

  const layoutedNodes = nodes.map((node) => {
    const nodeWithPosition = dagreGraph.node(node.id);
    return {
      ...node,
      position: {
        x: nodeWithPosition.x - LAYOUT_CONFIG.nodeWidth / 2,
        y: nodeWithPosition.y - LAYOUT_CONFIG.nodeHeight / 2,
      },
    };
  });

  return { nodes: layoutedNodes, edges };
};

// Cluster-aware layout - groups nodes by cluster with visual separation
const getClusterAwareLayout = (
  nodes: Node[],
  edges: Edge[],
  clusterData: ReturnType<typeof prepareClusteredGraphForReactFlow>,
  allOriginalEdges: GraphEdge[],
  expansionState: ExpansionState
) => {
  // Separate top-level clusters only (parentClusterId === null)
  // Nested sub-clusters are handled within their parent's layout
  const expandedClusters: SubgraphCluster[] = [];
  const collapsedClusters: SubgraphCluster[] = [];
  const collapsedAsOrphans: ApiGraphNode[] = [];
  
  clusterData.clusters
    .filter(cluster => cluster.parentClusterId === null) // Only top-level clusters
    .forEach(cluster => {
      const isExpanded = expansionState[cluster.containerId] ?? false; // Default to collapsed
      if (isExpanded) {
        expandedClusters.push(cluster);
      } else {
        // Keep cluster info for child→container mapping
        collapsedClusters.push(cluster);
        
        // Treat collapsed cluster container as an orphan node
        const containerNode = nodes.find(n => n.id === cluster.containerId);
        if (containerNode) {
          collapsedAsOrphans.push({
            id: cluster.containerId,
            label: containerNode.data?.label || cluster.containerId,
            state: containerNode.data?.state || 'backlog',
            priority: containerNode.data?.priority || 'normal',
            labels: containerNode.data?.labels || [],
            blocked: false, // Collapsed clusters are never blocked (the container itself is shown)
          });
        }
      }
    });
  
  // Combine with actual orphan nodes
  const allOrphans = [...clusterData.orphanNodes, ...collapsedAsOrphans];
  
  // Filter cluster nodes to only include visible nodes (respects sub-cluster expansion)
  // Include both visible edges AND virtual edges for correct intra-cluster layout
  const visibleNodeIds = new Set(clusterData.visibleNodes.map(n => n.id));
  
  // Build map of virtual edges for each cluster
  const virtualEdgesByCluster = new Map<string, GraphEdge[]>();
  clusterData.virtualEdges.forEach(ve => {
    // Find which top-level cluster(s) these nodes belong to
    const fromTopCluster = clusterData.clusters.find(c => 
      c.parentClusterId === null && c.nodes.some(n => n.id === ve.from)
    );
    const toTopCluster = clusterData.clusters.find(c => 
      c.parentClusterId === null && c.nodes.some(n => n.id === ve.to)
    );
    
    // If both endpoints are in the same TOP-LEVEL cluster, it's an internal virtual edge
    if (fromTopCluster && toTopCluster && fromTopCluster.containerId === toTopCluster.containerId) {
      const clusterId = fromTopCluster.containerId;
      if (!virtualEdgesByCluster.has(clusterId)) {
        virtualEdgesByCluster.set(clusterId, []);
      }
      virtualEdgesByCluster.get(clusterId)!.push({ from: ve.from, to: ve.to });
    }
  });
  
  const expandedClustersWithVisibleNodes = expandedClusters.map(cluster => {
    const visibleNodes = cluster.nodes.filter(n => visibleNodeIds.has(n.id));
    const visibleInternalEdges = cluster.internalEdges.filter(e => 
      visibleNodeIds.has(e.from) && visibleNodeIds.has(e.to)
    );
    const virtualInternalEdges = virtualEdgesByCluster.get(cluster.containerId) || [];
    
    return {
      ...cluster,
      nodes: visibleNodes,
      internalEdges: [...visibleInternalEdges, ...virtualInternalEdges],
    };
  });
  
  // Use the new cluster-aware layout algorithm
  const layoutResult = createClusterAwareLayout(
    expandedClustersWithVisibleNodes,
    clusterData.crossClusterEdges,
    allOrphans,
    allOriginalEdges,
    collapsedClusters
  );
  
  const finalNodes: Node[] = [];
  
  // Add cluster container boxes ONLY for expanded clusters
  layoutResult.clusters.forEach((clusterPos, clusterId) => {
    const cluster = clusterData.clusters.find(c => c.containerId === clusterId);
    if (!cluster) return;
    
    const isExpanded = expansionState[clusterId] ?? false; // Default to collapsed
    
    // Only render the visual container box if expanded
    if (isExpanded) {
      const containerNode = nodes.find(n => n.id === clusterId);
      const clusterTitle = containerNode?.data?.label || clusterId.substring(0, 8);
      
      const CLUSTER_PADDING = 40;
      const NODE_PADDING = 10; // Vertical padding from ClusterNode style
      const offset = (CLUSTER_PADDING - NODE_PADDING) / 2;
      
      finalNodes.push({
        id: `cluster-${clusterId}`,
        type: 'group', // ReactFlow group node type
        position: {
          x: clusterPos.x + offset,
          y: clusterPos.y + offset,
        },
        style: {
          width: clusterPos.width,
          height: clusterPos.height,
          backgroundColor: 'rgba(200, 200, 200, 0.1)',
          border: '2px solid rgba(150, 150, 150, 0.3)',
          borderRadius: '8px',
          zIndex: -1,
        },
        data: {
          label: clusterTitle,
        },
        draggable: false,
        selectable: false,
      });
    }
  });
  
  // Convert layout result to ReactFlow nodes
  layoutResult.nodes.forEach(layoutNode => {
    const originalNode = nodes.find(n => n.id === layoutNode.id);
    if (!originalNode) {
      throw new Error(`Node ${layoutNode.id} not found in original nodes`);
    }
    
    finalNodes.push({
      ...originalNode,
      position: layoutNode.position,
      zIndex: 10, // Ensure nodes appear above cluster containers
    });
  });
  
  // Orphan nodes are now positioned by the layout algorithm (in layoutResult.nodes)
  // No need for separate orphan handling
  
  return { nodes: finalNodes, edges };
};

// Compact layered layout - vertical stacking within ranks
const getCompactLayout = (
  nodes: Node[], 
  edges: Edge[],
  clusterData?: ReturnType<typeof prepareClusteredGraphForReactFlow> | null,
  allOriginalEdges?: GraphEdge[],
  expansionState?: ExpansionState
) => {
  if (nodes.length === 0) {
    return { nodes: [], edges };
  }

  // If we have cluster data, use cluster-aware layout
  if (clusterData && clusterData.clusters.length > 0) {
    return getClusterAwareLayout(nodes, edges, clusterData, allOriginalEdges || [], expansionState || {});
  }

  // Otherwise fall back to basic rank-based layout
  return getBasicRankLayout(nodes, edges);
};

// Extract the existing compact layout logic to a separate function
const getBasicRankLayout = (nodes: Node[], edges: Edge[]) => {

  // Build adjacency list (source -> targets, where source depends on targets)
  const dependsOn = new Map<string, Set<string>>();
  const dependedBy = new Map<string, Set<string>>();
  
  nodes.forEach(node => {
    dependsOn.set(node.id, new Set());
    dependedBy.set(node.id, new Set());
  });

  edges.forEach(edge => {
    // edge.source is the dependency, edge.target is the dependent
    // so target depends on source
    dependsOn.get(edge.target)?.add(edge.source);
    dependedBy.get(edge.source)?.add(edge.target);
  });

  // Compute ranks via topological sort (BFS from roots)
  const ranks = new Map<string, number>();
  const nodeSet = new Set(nodes.map(n => n.id));
  
  // Find roots (nodes with no dependencies within our visible set)
  const roots: string[] = [];
  nodes.forEach(node => {
    const deps = dependsOn.get(node.id);
    const visibleDeps = deps ? [...deps].filter(d => nodeSet.has(d)) : [];
    if (visibleDeps.length === 0) {
      roots.push(node.id);
    }
  });

  // BFS to assign ranks
  const queue = [...roots];
  roots.forEach(id => ranks.set(id, 0));

  while (queue.length > 0) {
    const current = queue.shift()!;
    const currentRank = ranks.get(current)!;
    
    const dependents = dependedBy.get(current) || new Set();
    dependents.forEach(dep => {
      if (!nodeSet.has(dep)) return;
      
      // Rank is max of all dependency ranks + 1
      const existingRank = ranks.get(dep) ?? -1;
      const newRank = currentRank + 1;
      
      if (newRank > existingRank) {
        ranks.set(dep, newRank);
        // Re-process this node's dependents
        if (!queue.includes(dep)) {
          queue.push(dep);
        }
      }
    });
  }

  // Handle any unranked nodes (disconnected or cycles)
  nodes.forEach(node => {
    if (!ranks.has(node.id)) {
      ranks.set(node.id, 0);
    }
  });

  // Group nodes by rank
  const nodesByRank = new Map<number, string[]>();
  ranks.forEach((rank, nodeId) => {
    if (!nodesByRank.has(rank)) {
      nodesByRank.set(rank, []);
    }
    nodesByRank.get(rank)!.push(nodeId);
  });

  // Sort ranks
  const sortedRanks = [...nodesByRank.keys()].sort((a, b) => a - b);
  
  // Calculate how many columns each rank needs
  const columnsPerRank = new Map<number, number>();
  sortedRanks.forEach(rank => {
    const count = nodesByRank.get(rank)!.length;
    columnsPerRank.set(rank, Math.ceil(count / LAYOUT_CONFIG.maxNodesPerColumn));
  });
  
  // Calculate cumulative X offset for each rank (accounting for multi-column ranks)
  const rankStartX = new Map<number, number>();
  let currentX = LAYOUT_CONFIG.margin;
  sortedRanks.forEach(rank => {
    rankStartX.set(rank, currentX);
    const cols = columnsPerRank.get(rank)!;
    currentX += cols * LAYOUT_CONFIG.columnSpacing + LAYOUT_CONFIG.rankSpacing;
  });
  
  // Calculate positions
  const positions = new Map<string, { x: number; y: number }>();
  
  // Max height based on maxNodesPerColumn
  const totalHeightMax = LAYOUT_CONFIG.maxNodesPerColumn * (LAYOUT_CONFIG.nodeHeight + LAYOUT_CONFIG.nodeSpacing);

  sortedRanks.forEach(rank => {
    const nodesInRank = nodesByRank.get(rank)!;
    const baseX = rankStartX.get(rank)!;
    
    nodesInRank.forEach((nodeId, index) => {
      const col = Math.floor(index / LAYOUT_CONFIG.maxNodesPerColumn);
      const row = index % LAYOUT_CONFIG.maxNodesPerColumn;
      
      // Calculate how many nodes in this column for vertical centering
      const nodesInThisColumn = Math.min(
        LAYOUT_CONFIG.maxNodesPerColumn,
        nodesInRank.length - col * LAYOUT_CONFIG.maxNodesPerColumn
      );
      const columnHeight = nodesInThisColumn * (LAYOUT_CONFIG.nodeHeight + LAYOUT_CONFIG.nodeSpacing);
      const startY = (totalHeightMax - columnHeight) / 2;
      
      positions.set(nodeId, {
        x: baseX + col * LAYOUT_CONFIG.columnSpacing,
        y: startY + row * (LAYOUT_CONFIG.nodeHeight + LAYOUT_CONFIG.nodeSpacing) + LAYOUT_CONFIG.margin,
      });
    });
  });

  // Apply positions to nodes
  const layoutedNodes = nodes.map(node => ({
    ...node,
    position: positions.get(node.id) || { x: 0, y: 0 },
  }));

  return { nodes: layoutedNodes, edges };
};

// Main layout function
const getLayoutedElements = (
  nodes: Node[], 
  edges: Edge[], 
  algorithm: LayoutAlgorithm = 'dagre',
  clusterData?: ReturnType<typeof prepareClusteredGraphForReactFlow> | null,
  allOriginalEdges?: GraphEdge[],
  expansionState?: ExpansionState
) => {
  switch (algorithm) {
    case 'compact':
      return getCompactLayout(nodes, edges, clusterData, allOriginalEdges, expansionState);
    case 'dagre':
    default:
      return getDagreLayout(nodes, edges);
  }
};

export type ViewMode = 'tactical' | 'strategic';

interface GraphViewProps {
  onNodeClick?: (issueId: string) => void;
  viewMode?: ViewMode;
  labelFilters?: string[]; // e.g., ["milestone:v1.0", "epic:*"]
  layoutAlgorithm?: LayoutAlgorithm;
  onLayoutChange?: (algorithm: LayoutAlgorithm) => void;
  /** Node ID to focus on (centers viewport and expands parent clusters) */
  focusNodeId?: string | null;
  /** Callback when focus operation completes (or fails) */
  onFocusComplete?: (success: boolean) => void;
}

export function GraphView({ 
  onNodeClick, 
  viewMode = 'tactical', 
  labelFilters = [],
  layoutAlgorithm = 'compact',
  onLayoutChange: _onLayoutChange,
  focusNodeId,
  onFocusComplete,
}: GraphViewProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [_nodeStats, setNodeStats] = useState<Map<string, DownstreamStats>>(new Map());
  const [strategicTypes, setStrategicTypes] = useState<string[]>(['milestone', 'epic']); // Default fallback
  const [hierarchyConfig, setHierarchyConfig] = useState<HierarchyConfig | null>(null);
  const [expansionState, setExpansionState] = useState<ExpansionState>(() => {
    // Load initial state from localStorage
    try {
      const saved = localStorage.getItem('jit.graph.expansionState');
      return saved ? JSON.parse(saved) : {};
    } catch (e) {
      console.warn('Failed to load expansion state from localStorage:', e);
      return {};
    }
  });
  const [isInteractive, setIsInteractive] = useState(true); // Start unlocked (will be toggled to locked on init)
  
  // Listen to interactive button clicks to update our state
  useEffect(() => {
    const button = document.querySelector('.react-flow__controls-interactive');
    if (!button) return;
    
    const observer = new MutationObserver(() => {
      // Check if the button's aria-label indicates locked/unlocked state
      const isLocked = button.getAttribute('title')?.includes('unlock');
      setIsInteractive(!isLocked);
    });
    
    observer.observe(button, { attributes: true });
    return () => observer.disconnect();
  }, []);
  
  const [defaultViewport] = useState(() => {
    // Load viewport from localStorage on mount
    try {
      const saved = localStorage.getItem('jit.graph.viewport');
      return saved ? JSON.parse(saved) : { x: 0, y: 0, zoom: 1 };
    } catch (e) {
      console.warn('Failed to load viewport from localStorage:', e);
      return { x: 0, y: 0, zoom: 1 };
    }
  });
  
  const [hasInitialFit, setHasInitialFit] = useState(false);
  const reactFlowInstanceRef = useRef<any>(null);
  const savedViewportRef = useRef<any>(null);
  const clusterDataRef = useRef<ReturnType<typeof prepareClusteredGraphForReactFlow> | null>(null);
  const [isRenderable, setIsRenderable] = useState(true); // Control rendering during viewport restoration

  // Save expansion state to localStorage whenever it changes
  useEffect(() => {
    try {
      localStorage.setItem('jit.graph.expansionState', JSON.stringify(expansionState));
    } catch (e) {
      console.warn('Failed to save expansion state to localStorage:', e);
    }
  }, [expansionState]);

  // Save viewport to localStorage whenever it changes (debounced)
  const saveViewportTimeoutRef = useRef<number | null>(null);
  
  const handleViewportChange = useCallback(() => {
    // Debounce viewport saves to avoid excessive localStorage writes
    if (saveViewportTimeoutRef.current) {
      clearTimeout(saveViewportTimeoutRef.current);
    }
    
    saveViewportTimeoutRef.current = window.setTimeout(() => {
      if (reactFlowInstanceRef.current) {
        try {
          const viewport = reactFlowInstanceRef.current.getViewport();
          localStorage.setItem('jit.graph.viewport', JSON.stringify(viewport));
        } catch (e) {
          console.warn('Failed to save viewport to localStorage:', e);
        }
      }
    }, 300);
  }, []);

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (saveViewportTimeoutRef.current) {
        clearTimeout(saveViewportTimeoutRef.current);
      }
    };
  }, []);

  /**
   * Toggle expansion state for a container node (cluster or hierarchical parent).
   * Hierarchy-agnostic - works with any configured node types.
   */
  const toggleExpansion = useCallback((nodeId: string) => {
    if (!reactFlowInstanceRef.current) return;
    
    const rfInstance = reactFlowInstanceRef.current;
    const viewport = rfInstance.getViewport();
    
    // Find the node being toggled (could be cluster container or regular node)
    const node = rfInstance.getNode(nodeId);
    if (!node) return;
    
    // Calculate screen position of node
    const nodeScreenX = node.position.x * viewport.zoom + viewport.x;
    const nodeScreenY = node.position.y * viewport.zoom + viewport.y;
    
    // Store for adjustment after layout
    savedViewportRef.current = {
      nodeId,
      zoom: viewport.zoom,
      screenX: nodeScreenX,
      screenY: nodeScreenY,
    };
    
    setIsRenderable(false); // Hide during transition
    
    setExpansionState(prev => ({
      ...prev,
      [nodeId]: !(prev[nodeId] ?? false), // Default to collapsed (false)
    }));
  }, []);

  // Restore viewport after nodes change (when expanding/collapsing)
  useEffect(() => {
    if (savedViewportRef.current && reactFlowInstanceRef.current && !loading && nodes.length > 0) {
      const savedData = savedViewportRef.current;
      
      // Use setTimeout to ensure nodes state has fully propagated
      setTimeout(() => {
        if (!reactFlowInstanceRef.current) return;
        
        // Find the node in the new nodes array directly (not via rfInstance which might be stale)
        const node = nodes.find(n => n.id === savedData.nodeId);
        
        if (node) {
          // Calculate viewport offset to keep node at same screen position
          const newViewportX = savedData.screenX - node.position.x * savedData.zoom;
          const newViewportY = savedData.screenY - node.position.y * savedData.zoom;
          
          reactFlowInstanceRef.current.setViewport(
            { 
              x: newViewportX, 
              y: newViewportY, 
              zoom: savedData.zoom 
            }, 
            { duration: 0 }
          );
        }
        
        savedViewportRef.current = null;
        setIsRenderable(true);
      }, 50);
    }
  }, [nodes, loading]);

  // Fetch strategic types from API on mount
  useEffect(() => {
    const fetchStrategicTypes = async () => {
      try {
        const types = await apiClient.getStrategicTypes();
        setStrategicTypes(types);
      } catch (err) {
        console.warn('Failed to fetch strategic types, using defaults:', err);
        // Keep default values
      }
    };
    fetchStrategicTypes();
  }, []);

  // Fetch hierarchy config from API on mount
  useEffect(() => {
    const fetchHierarchyConfig = async () => {
      try {
        const config = await apiClient.getHierarchy();
        // Store full config (types + icons)
        setHierarchyConfig({
          levels: config.types,
          icons: config.icons || {},
        });
      } catch (err) {
        console.warn('Failed to fetch hierarchy config:', err);
        // Fallback config - just use levels, icons optional
        const fallback: HierarchyConfig = {
          levels: {
            milestone: 1,
            epic: 2,
            story: 3,
            task: 4,
            bug: 4,
          },
          icons: {}, // No icons in fallback - will just show #nodeId
        };
        setHierarchyConfig(fallback);
      }
    };
    fetchHierarchyConfig();
  }, []);

  const loadGraph = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await apiClient.getGraph();
      
      // Build filter configuration
      const filters: GraphFilter[] = [];
      if (viewMode === 'strategic') {
        filters.push(createStrategicFilter(true, strategicTypes));
      }
      if (labelFilters.length > 0) {
        filters.push(createLabelFilter(labelFilters));
      }
      
      // Apply filters to all nodes
      const nodeFilterResults = new Map(
        data.nodes.map(node => [node.id, applyFiltersToNode(node, filters)])
      );
      
      // Filter out hidden nodes
      const visibleNodes = data.nodes.filter(node => nodeFilterResults.get(node.id)?.visible);
      
      // Calculate downstream stats for visible nodes (using full graph)
      const stats = new Map<string, DownstreamStats>();
      for (const node of visibleNodes) {
        stats.set(node.id, calculateDownstreamStats(node.id, data.nodes, data.edges, hierarchyConfig?.levels));
      }
      
      // Apply clustering if using compact layout and hierarchy config is available
      let nodesToRender = visibleNodes;
      let edgesToRender = data.edges;
      let clusterData = null;
      
      if (layoutAlgorithm === 'compact' && hierarchyConfig) {
        const clustered = prepareClusteredGraphForReactFlow(
          visibleNodes,
          data.edges,
          hierarchyConfig.levels,
          expansionState
        );
        
        // Keep nodesToRender as visibleNodes for flowNode creation
        // The expansion filtering will happen during rendering
        nodesToRender = visibleNodes; // Use ALL visible nodes, not filtered ones
        clusterData = clustered; // Pass to layout function
        clusterDataRef.current = clustered; // Store in ref for focus effect
        
        // Recalculate stats for cluster containers based on their actual members
        // (not dependency traversal which may include cross-cluster nodes)
        for (const cluster of clustered.clusters) {
          const clusterStats: DownstreamStats = {
            total: 0,
            done: 0,
            inProgress: 0,
            blocked: 0,
            ready: 0,
          };
          
          // Count cluster members (excluding the container itself)
          for (const member of cluster.nodes) {
            if (member.id !== cluster.containerId) {
              clusterStats.total++;
              if (member.state === 'done') clusterStats.done++;
              else if (member.state === 'in_progress') clusterStats.inProgress++;
              else if (member.state === 'ready') clusterStats.ready++;
              if (member.blocked) clusterStats.blocked++;
            }
          }
          
          // Override the dependency-based stats with cluster-based stats
          stats.set(cluster.containerId, clusterStats);
        }
        
        // Collect all edges that should be visible:
        // 1. visibleEdges (edges between visible nodes)
        // 2. Internal cluster edges (edges within each cluster)
        // 3. Cross-cluster edges (edges between different clusters)
        // 4. Virtual edges (for collapsed containers)
        const allInternalEdges = clustered.clusters.flatMap(c => c.internalEdges);
        
        edgesToRender = [
          ...clustered.visibleEdges,
          ...allInternalEdges,
          ...clustered.crossClusterEdges, // Add cross-cluster edges!
          // Add virtual edges as regular edges with metadata
          ...clustered.virtualEdges.map(ve => ({
            from: ve.from,
            to: ve.to,
            // Could add metadata here for rendering (e.g., thickness based on count)
          })),
        ];
        
        // Deduplicate edges by ID
        const edgeMap = new Map();
        edgesToRender.forEach(e => {
          const key = `${e.from}-${e.to}`;
          if (!edgeMap.has(key)) {
            edgeMap.set(key, e);
          }
        });
        edgesToRender = Array.from(edgeMap.values());
      }
      
      // Set node stats AFTER clustering has updated them
      setNodeStats(stats);
      
      const flowNodes: Node[] = nodesToRender.map((node: ApiGraphNode) => {
        const nodeStats = stats.get(node.id); // Use local stats, not state
        const hasDownstream = nodeStats && nodeStats.total > 0;
        const filterResult = nodeFilterResults.get(node.id)!;
        const isDimmed = filterResult.dimmed;
        
        // Check if this is a strategic node (based on configured strategic types)
        const isStrategic = node.labels.some(label => {
          if (label.startsWith('type:')) {
            const typeValue = label.substring(5);
            return strategicTypes.includes(typeValue);
          }
          return false;
        });
        
        // Get icon for this node's type
        const nodeType = node.labels.find(l => l.startsWith('type:'))?.substring(5);
        const typeIcon = nodeType && hierarchyConfig?.icons?.[nodeType];
        
        // Check if this node is a cluster container
        const cluster = clusterData?.clusters.find(c => c.containerId === node.id);
        const isClusterContainer = !!cluster;
        
        // For cluster containers when using compact layout, add collapse/expand functionality
        if (isClusterContainer && cluster && layoutAlgorithm === 'compact') {
          const isExpanded = expansionState[node.id] ?? false; // Default to collapsed
          const hiddenNodeCount = isExpanded ? 0 : cluster.nodes.length - 1; // -1 for container itself
          
          return {
            id: node.id,
            type: 'cluster',
            position: { x: 0, y: 0 },
            sourcePosition: Position.Right,
            targetPosition: Position.Left,
            data: {
              label: node.label,
              icon: typeIcon, // Pass icon to ClusterNode
              typeName: nodeType, // Pass type name
              isExpanded,
              hiddenNodeCount,
              onToggleExpansion: () => toggleExpansion(node.id),
              state: node.state,
              priority: node.priority,
              labels: node.labels,
              nodeId: node.id, // For reference
              isStrategic, // Pass strategic flag
              downstreamStats: hasDownstream ? nodeStats : undefined, // Pass stats if available
            },
            style: {
              opacity: isDimmed ? 0.4 : 1,
            },
          };
        }
        
        // Regular node (not a cluster container)
        return {
          id: node.id,
          type: 'default',
          position: { x: 0, y: 0 }, // Will be set by dagre
          sourcePosition: Position.Right,
          targetPosition: Position.Left,
          data: {
            label: (
              <div style={{ 
                padding: '10px 12px',
                fontFamily: 'var(--font-mono)',
                fontSize: '12px',
                opacity: isDimmed ? 0.4 : 1,
                transition: 'opacity 0.2s ease',
              }}>
                <div style={{ 
                  fontSize: '10px', 
                  color: 'var(--text-muted)',
                  marginBottom: '4px',
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                }}>
                  <span>
                    {typeIcon && `${typeIcon} `}
                    {nodeType && `${nodeType.charAt(0).toUpperCase() + nodeType.slice(1)} `}
                    #{node.id.substring(0, 8)}
                  </span>
                </div>
                <div style={{ 
                  fontWeight: 600,
                  marginBottom: '6px',
                  color: 'var(--text-primary)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  maxWidth: '180px',
                }}>
                  {node.label}
                </div>
                <div style={{ 
                  fontSize: '11px',
                  display: 'flex',
                  gap: '8px',
                  alignItems: 'center',
                }}>
                  <span style={{ color: priorityColors[node.priority] }}>
                    {priorityIcons[node.priority]} {node.priority}
                  </span>
                  <span style={{ color: 'var(--text-secondary)' }}>
                    | {node.state}
                  </span>
                </div>
                {hasDownstream && isStrategic && (
                  <div 
                    style={{
                      fontSize: '10px',
                      color: 'var(--text-secondary)',
                      fontFamily: 'var(--font-mono)',
                      borderTop: '1px solid var(--border)',
                      paddingTop: '4px',
                      marginTop: '6px',
                    }}
                    title={`Downstream: ${nodeStats.total} tasks (${nodeStats.done} done, ${nodeStats.inProgress} in progress, ${nodeStats.blocked} blocked)`}
                  >
                    ↓ {nodeStats.total} task{nodeStats.total !== 1 ? 's' : ''}
                    {nodeStats.done > 0 && ` • ✓ ${nodeStats.done}`}
                    {nodeStats.blocked > 0 && ` • ⚠ ${nodeStats.blocked}`}
                  </div>
                )}
              </div>
            ),
          },
          style: {
            border: `2px solid ${stateColors[node.state]}`,
            borderRadius: '12px',
            backgroundColor: 'var(--bg-tertiary)',
            padding: 0,
            width: 220,
            boxShadow: '0 2px 8px rgba(0, 0, 0, 0.3)',
            opacity: isDimmed ? 0.5 : 1,
            transition: 'opacity 0.2s ease',
          },
        };
      });

      // Note: edge.from depends on edge.to, so for L->R layout, 
      // the dependency (to) should be on the left, and dependent (from) on the right
      const flowEdges: Edge[] = edgesToRender
        .map((edge) => {
          const sourceResult = nodeFilterResults.get(edge.to);
          const targetResult = nodeFilterResults.get(edge.from);
          
          if (!sourceResult || !targetResult) {
            return null;
          }
          
          const edgeFilterResult = applyFiltersToEdge(edge, sourceResult, targetResult);
          
          if (!edgeFilterResult.visible) {
            return null;
          }
          
          const isDimmed = edgeFilterResult.dimmed;
          const edgeColor = isDimmed ? 'var(--border)' : 'var(--border-hover)';
          
          return {
            id: `${edge.from}-${edge.to}`,
            source: edge.to,   // Swap: dependency goes on the left
            target: edge.from, // Swap: dependent goes on the right
            sourceHandle: 'right' as const,
            targetHandle: 'left' as const,
            type: 'simplebezier' as const,
            animated: false,
            style: {
              stroke: edgeColor,
              strokeWidth: 2,
              opacity: isDimmed ? 0.3 : 1,
              transition: 'opacity 0.2s ease',
            },
            markerEnd: {
              type: MarkerType.ArrowClosed,
              color: edgeColor,
            },
          } as Edge;
        })
        .filter((edge): edge is Edge => edge !== null);

      // Apply layout algorithm
      const layouted = getLayoutedElements(flowNodes, flowEdges, layoutAlgorithm, clusterData, data.edges, expansionState);
      
      // Filter nodes to only show visible ones (respecting expansion state)
      const visibleNodeIds = clusterData 
        ? new Set(clusterData.visibleNodes.map(n => n.id))
        : new Set(nodesToRender.map(n => n.id));
      
      const finalNodes = layouted.nodes.filter(node => {
        // Always show visual cluster boxes (for expanded clusters only, created in layout)
        if (node.id.startsWith('cluster-')) return true;
        
        // Show nodes that are visible according to expansion state
        return visibleNodeIds.has(node.id);
      });
      
      setNodes(finalNodes);
      setEdges(layouted.edges);
      
      // Only fit view on first load if no saved viewport exists
      if (!hasInitialFit && finalNodes.length > 0) {
        setHasInitialFit(true);
        const savedViewport = localStorage.getItem('jit.graph.viewport');
        if (!savedViewport) {
          // No saved viewport, fit to view
          setTimeout(() => {
            reactFlowInstanceRef.current?.fitView({ duration: 200 });
          }, 50);
        }
        // Otherwise defaultViewport prop handles restoration
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load graph');
      console.error('Failed to load graph:', err);
    } finally {
      setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [setNodes, setEdges, viewMode, labelFilters, strategicTypes, layoutAlgorithm, hierarchyConfig, expansionState]); // nodeStats is setState, not a dependency

  useEffect(() => {
    loadGraph();
  }, [loadGraph]);

  // Handle focus requests
  useEffect(() => {
    if (!focusNodeId || loading) {
      return;
    }

    // If using compact layout with clusters, check cluster data and expand parents
    if (layoutAlgorithm === 'compact' && clusterDataRef.current) {
      // Check if node exists in ANY cluster (even if collapsed)
      const nodeExistsInClusters = clusterDataRef.current.clusters.some(cluster =>
        cluster.nodes.some(n => n.id === focusNodeId)
      );
      
      if (!nodeExistsInClusters) {
        // Check if it's an orphan node
        const isOrphan = clusterDataRef.current.orphanNodes.some(n => n.id === focusNodeId);
        
        if (!isOrphan) {
          console.warn('[Focus] Node not found in cluster data:', focusNodeId);
          onFocusComplete?.(false);
          return;
        }
      }
      
      // Find parent clusters
      const parentClusterIds = findParentClusters(focusNodeId, clusterDataRef.current.clusters);
      
      if (parentClusterIds.length > 0) {
        // Expand all parent clusters
        const newExpansionState = { ...expansionState };
        parentClusterIds.forEach(clusterId => {
          newExpansionState[clusterId] = true; // true = expanded
        });
        setExpansionState(newExpansionState);
        
        // Wait for re-render with expanded clusters, then center
        setTimeout(() => {
          reactFlowInstanceRef.current?.fitView({
            nodes: [{ id: focusNodeId }],
            duration: 300,
            padding: 0.2,
          });
          onFocusComplete?.(true);
        }, 150); // Slightly longer delay to ensure re-render completes
      } else {
        // No parent clusters, node is already visible - just center
        console.log('[Focus] No parent clusters, centering on visible node');
        reactFlowInstanceRef.current?.fitView({
          nodes: [{ id: focusNodeId }],
          duration: 300,
          padding: 0.2,
        });
        onFocusComplete?.(true);
      }
    } else {
      // No clustering, just center on the node
      console.log('[Focus] No clustering, centering on node');
      
      // Verify node exists in nodes array
      const targetNode = nodes.find(n => n.id === focusNodeId);
      if (!targetNode) {
        console.warn('[Focus] Node not found:', focusNodeId);
        onFocusComplete?.(false);
        return;
      }
      
      reactFlowInstanceRef.current?.fitView({
        nodes: [{ id: focusNodeId }],
        duration: 300,
        padding: 0.2,
      });
      onFocusComplete?.(true);
    }
  }, [focusNodeId, nodes, loading, layoutAlgorithm, expansionState, onFocusComplete]);

  const handleNodeClick = useCallback(
    (_: React.MouseEvent, node: Node) => {
      if (onNodeClick) {
        onNodeClick(node.id);
      }
    },
    [onNodeClick]
  );

  if (loading) {
    return (
      <div style={{ 
        padding: '20px',
        fontFamily: 'var(--font-mono)',
        color: 'var(--text-secondary)',
        backgroundColor: 'var(--bg-primary)',
      }}>
        $ loading graph...
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ 
        padding: '20px',
        fontFamily: 'var(--font-mono)',
        color: 'var(--error)',
        backgroundColor: 'var(--bg-primary)',
      }}>
        <div>$ error: {error}</div>
        <button 
          onClick={loadGraph}
          style={{
            marginTop: '12px',
            padding: '6px 12px',
            backgroundColor: 'var(--bg-tertiary)',
            border: '1px solid var(--border)',
            borderRadius: '6px',
            color: 'var(--text-primary)',
            fontFamily: 'var(--font-mono)',
            fontSize: '12px',
            cursor: 'pointer',
          }}
        >
          retry
        </button>
      </div>
    );
  }

  return (
    <div style={{ height: '100%', width: '100%', backgroundColor: 'var(--bg-primary)' }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={handleNodeClick}
        onMoveEnd={handleViewportChange}
        defaultViewport={defaultViewport}
        nodeTypes={nodeTypes}
        onInit={(instance) => { 
          reactFlowInstanceRef.current = instance;
          // Set to locked mode initially by toggling interactivity off
          setTimeout(() => {
            const button = document.querySelector('.react-flow__controls-interactive') as HTMLButtonElement;
            if (button) button.click();
          }, 100);
        }}
        attributionPosition="bottom-right"
        proOptions={proOptions}
        minZoom={0.1}  // Allow zooming out much further
        maxZoom={2}    // Allow zooming in a bit more
        nodesDraggable={isInteractive}
        nodesConnectable={false}  // Never allow connecting nodes
        nodesFocusable={isInteractive}
        edgesFocusable={isInteractive}
        elementsSelectable={true}  // Allow selection for clicks
        panOnDrag={true}  // Allow panning
        panOnScroll={false}  // Zoom with scroll, not pan
        zoomOnScroll={true}  // Zoom with scroll
        zoomOnDoubleClick={false}  // Disable zoom on double click
        style={{
          opacity: isRenderable ? 1 : 0,
          transition: 'opacity 0.1s ease-in-out',
          pointerEvents: isRenderable ? 'auto' : 'none',
        }}
      >
        <Controls position="top-left" />
        <Background 
          color="var(--border)" 
          gap={16}
          style={backgroundStyle}
        />
      </ReactFlow>
      
      {/* Expand/Collapse All Controls */}
      <div style={{
        position: 'absolute',
        top: '16px',
        right: '16px',
        backgroundColor: 'var(--bg-tertiary)',
        padding: '8px 12px',
        borderRadius: '8px',
        border: '1px solid var(--border)',
        fontFamily: 'var(--font-mono)',
        fontSize: '11px',
        display: 'flex',
        gap: '8px',
        alignItems: 'center',
      }}>
        <button
          onClick={() => {
            setExpansionState({});
            // Fit view after collapse to see full graph
            setTimeout(() => {
              reactFlowInstanceRef.current?.fitView({ padding: 0.1, duration: 300 });
            }, 100);
          }}
          style={{
            padding: '4px 8px',
            backgroundColor: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            color: 'var(--text-primary)',
            fontFamily: 'var(--font-mono)',
            fontSize: '10px',
            cursor: 'pointer',
          }}
        >
          Collapse All
        </button>
        <button
          onClick={() => {
            // Expand all clusters by setting all to true
            const allExpanded: ExpansionState = {};
            nodes.forEach(node => {
              if (node.type === 'cluster') {
                allExpanded[node.id] = true;
              }
            });
            setExpansionState(allExpanded);
          }}
          style={{
            padding: '4px 8px',
            backgroundColor: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            color: 'var(--text-primary)',
            fontFamily: 'var(--font-mono)',
            fontSize: '10px',
            cursor: 'pointer',
          }}
        >
          Expand All
        </button>
      </div>

      {/* State Legend */}
      <div style={{ 
        position: 'absolute', 
        bottom: '16px', 
        left: '16px', 
        backgroundColor: 'var(--bg-tertiary)',
        padding: '12px',
        borderRadius: '8px',
        border: '1px solid var(--border)',
        fontFamily: 'var(--font-mono)',
        fontSize: '11px',
      }}>
        <div style={{ 
          fontWeight: 600, 
          marginBottom: '8px',
          color: 'var(--text-primary)',
        }}>
          State Legend
        </div>
        {Object.entries(stateColors).map(([state]) => (
          <div key={state} style={{ 
            display: 'flex', 
            alignItems: 'center', 
            gap: '8px', 
            marginTop: '4px',
          }}>
            <div style={{ 
              width: '12px', 
              height: '12px', 
              backgroundColor: stateColors[state as State],
              borderRadius: '2px',
              border: '1px solid var(--border)',
            }} />
            <span style={{ color: 'var(--text-secondary)' }}>{state}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
