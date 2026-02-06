import { useEffect, useState, useCallback } from 'react';
import ReactFlow, {
  Controls,
  Background,
  useNodesState,
  useEdgesState,
  MarkerType,
  Position,
  type Node,
  type Edge,
} from 'reactflow';
import dagre from 'dagre';
import 'reactflow/dist/style.css';
import { apiClient } from '../../api/client';
import type { State, Priority, GraphNode as ApiGraphNode, GraphEdge } from '../../types/models';
import { LabelBadge } from '../Labels/LabelBadge';
import { calculateDownstreamStats, type DownstreamStats } from '../../utils/strategicView';
import { applyFiltersToNode, applyFiltersToEdge, createStrategicFilter, createLabelFilter, type GraphFilter } from '../../utils/graphFilter';
import type { HierarchyLevelMap, ExpansionState } from '../../types/subgraphCluster';
import { prepareClusteredGraphForReactFlow } from '../../utils/clusteredGraphLayout';
import { createClusterAwareLayout } from '../../utils/clusterAwareLayout';

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
  allOriginalEdges: GraphEdge[]
) => {
  // Use the new cluster-aware layout algorithm with ALL original edges
  // This ensures orphan nodes are positioned correctly based on all dependencies
  const layoutResult = createClusterAwareLayout(
    clusterData.clusters,
    clusterData.crossClusterEdges,
    clusterData.orphanNodes,
    allOriginalEdges
  );
  
  const finalNodes: Node[] = [];
  
  // Add cluster container boxes (rendered behind nodes)
  layoutResult.clusters.forEach((clusterPos, clusterId) => {
    const cluster = clusterData.clusters.find(c => c.containerId === clusterId);
    if (!cluster) return;
    
    const containerNode = nodes.find(n => n.id === clusterId);
    const clusterTitle = containerNode?.data?.label || clusterId.substring(0, 8);
    
    finalNodes.push({
      id: `cluster-${clusterId}`,
      type: 'group', // ReactFlow group node type
      position: {
        x: clusterPos.x,
        y: clusterPos.y,
      },
      style: {
        width: clusterPos.width,
        height: clusterPos.height,
        backgroundColor: 'rgba(200, 200, 200, 0.1)',
        border: '2px solid rgba(150, 150, 150, 0.3)',
        borderRadius: '8px',
        padding: '40px 20px 20px 20px',
        zIndex: -1,
      },
      data: {
        label: clusterTitle,
      },
      draggable: false,
      selectable: false,
    });
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
  allOriginalEdges?: GraphEdge[]
) => {
  if (nodes.length === 0) {
    return { nodes: [], edges };
  }

  // If we have cluster data, use cluster-aware layout
  if (clusterData && clusterData.clusters.length > 0) {
    return getClusterAwareLayout(nodes, edges, clusterData, allOriginalEdges || []);
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
  allOriginalEdges?: GraphEdge[]
) => {
  switch (algorithm) {
    case 'compact':
      return getCompactLayout(nodes, edges, clusterData, allOriginalEdges);
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
}

export function GraphView({ 
  onNodeClick, 
  viewMode = 'tactical', 
  labelFilters = [],
  layoutAlgorithm = 'dagre',
  onLayoutChange,
}: GraphViewProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [nodeStats, setNodeStats] = useState<Map<string, DownstreamStats>>(new Map());
  const [strategicTypes, setStrategicTypes] = useState<string[]>(['milestone', 'epic']); // Default fallback
  const [hierarchyConfig, setHierarchyConfig] = useState<HierarchyLevelMap | null>(null);
  const [expansionState] = useState<ExpansionState>({});

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
        // Extract just the types mapping
        setHierarchyConfig(config.types);
      } catch (err) {
        console.warn('Failed to fetch hierarchy config:', err);
        // Fallback config based on strategic types
        const fallback: HierarchyLevelMap = {
          milestone: 1,
          epic: 2,
          story: 3,
          task: 4,
          bug: 4,
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
        stats.set(node.id, calculateDownstreamStats(node.id, data.nodes, data.edges));
      }
      setNodeStats(stats);
      
      // Apply clustering if using compact layout and hierarchy config is available
      let nodesToRender = visibleNodes;
      let edgesToRender = data.edges;
      let clusterData = null;
      
      if (layoutAlgorithm === 'compact' && hierarchyConfig) {
        const clustered = prepareClusteredGraphForReactFlow(
          visibleNodes,
          data.edges,
          hierarchyConfig,
          expansionState
        );
        
        nodesToRender = clustered.visibleNodes;
        clusterData = clustered; // Pass to layout function
        
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
      
      const flowNodes: Node[] = nodesToRender.map((node: ApiGraphNode) => {
        const stats = nodeStats.get(node.id);
        const hasDownstream = stats && stats.total > 0;
        const filterResult = nodeFilterResults.get(node.id)!;
        const isDimmed = filterResult.dimmed;
        
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
                }}>
                  #{node.id.substring(0, 8)}
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
                {node.labels && node.labels.length > 0 && (
                  <div style={{
                    display: 'flex',
                    flexWrap: 'wrap',
                    gap: '4px',
                    marginTop: '6px',
                  }}>
                    {node.labels.slice(0, 2).map((label) => (
                      <LabelBadge key={label} label={label} size="small" />
                    ))}
                    {node.labels.length > 2 && (
                      <span style={{
                        fontSize: '9px',
                        color: 'var(--text-muted)',
                        fontFamily: 'var(--font-mono)',
                      }}>
                        +{node.labels.length - 2}
                      </span>
                    )}
                  </div>
                )}
                {hasDownstream && (
                  <div 
                    style={{
                      fontSize: '10px',
                      color: 'var(--text-secondary)',
                      fontFamily: 'var(--font-mono)',
                      borderTop: '1px solid var(--border)',
                      paddingTop: '4px',
                      marginTop: '6px',
                    }}
                    title={`Downstream: ${stats.total} tasks (${stats.done} done, ${stats.inProgress} in progress, ${stats.blocked} blocked)`}
                  >
                    ↓ {stats.total} task{stats.total !== 1 ? 's' : ''}
                    {stats.done > 0 && ` • ✓ ${stats.done}`}
                    {stats.blocked > 0 && ` • ⚠ ${stats.blocked}`}
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
      const layouted = getLayoutedElements(flowNodes, flowEdges, layoutAlgorithm, clusterData, data.edges);
      setNodes(layouted.nodes);
      setEdges(layouted.edges);
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
        fitView
        attributionPosition="bottom-right"
        proOptions={proOptions}
      >
        <Controls />
        <Background 
          color="var(--border)" 
          gap={16}
          style={backgroundStyle}
        />
      </ReactFlow>
      
      {/* Layout Algorithm Toggle */}
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
        <span style={{ color: 'var(--text-secondary)' }}>Layout:</span>
        <button
          onClick={() => onLayoutChange?.('dagre')}
          style={{
            padding: '4px 8px',
            backgroundColor: layoutAlgorithm === 'dagre' ? 'var(--accent)' : 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            color: layoutAlgorithm === 'dagre' ? 'var(--bg-primary)' : 'var(--text-primary)',
            fontFamily: 'var(--font-mono)',
            fontSize: '10px',
            cursor: 'pointer',
          }}
        >
          Dagre
        </button>
        <button
          onClick={() => onLayoutChange?.('compact')}
          style={{
            padding: '4px 8px',
            backgroundColor: layoutAlgorithm === 'compact' ? 'var(--accent)' : 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            color: layoutAlgorithm === 'compact' ? 'var(--bg-primary)' : 'var(--text-primary)',
            fontFamily: 'var(--font-mono)',
            fontSize: '10px',
            cursor: 'pointer',
          }}
        >
          Compact
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
