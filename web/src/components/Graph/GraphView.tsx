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
import type { State, Priority, GraphNode as ApiGraphNode } from '../../types/models';
import { LabelBadge } from '../Labels/LabelBadge';
import { calculateDownstreamStats, type DownstreamStats } from '../../utils/strategicView';
import { applyFiltersToNode, applyFiltersToEdge, createStrategicFilter, createLabelFilter, type GraphFilter } from '../../utils/graphFilter';

// State colors using CSS variables
const stateColors: Record<State, string> = {
  backlog: 'var(--state-backlog)',
  ready: 'var(--state-ready)',
  in_progress: 'var(--state-in-progress)',
  gated: 'var(--state-gated)',
  done: 'var(--state-done)',
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

// Dagre layout algorithm
const getLayoutedElements = (nodes: Node[], edges: Edge[], direction = 'LR') => {
  const dagreGraph = new dagre.graphlib.Graph();
  dagreGraph.setDefaultEdgeLabel(() => ({}));
  dagreGraph.setGraph({ 
    rankdir: direction,
    nodesep: 80,
    ranksep: 120,
    marginx: 40,
    marginy: 40,
  });

  nodes.forEach((node) => {
    dagreGraph.setNode(node.id, { width: 220, height: 80 });
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
        x: nodeWithPosition.x - 110,
        y: nodeWithPosition.y - 40,
      },
    };
  });

  return { nodes: layoutedNodes, edges };
};

export type ViewMode = 'tactical' | 'strategic';

interface GraphViewProps {
  onNodeClick?: (issueId: string) => void;
  viewMode?: ViewMode;
  labelFilters?: string[]; // e.g., ["milestone:v1.0", "epic:*"]
}

export function GraphView({ onNodeClick, viewMode = 'tactical', labelFilters = [] }: GraphViewProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [nodeStats, setNodeStats] = useState<Map<string, DownstreamStats>>(new Map());

  const loadGraph = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await apiClient.getGraph();
      
      // Build filter configuration
      const filters: GraphFilter[] = [];
      if (viewMode === 'strategic') {
        filters.push(createStrategicFilter(true));
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
      
      const flowNodes: Node[] = visibleNodes.map((node: ApiGraphNode) => {
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
      const flowEdges: Edge[] = data.edges
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

      // Apply dagre layout
      const layouted = getLayoutedElements(flowNodes, flowEdges);
      setNodes(layouted.nodes);
      setEdges(layouted.edges);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load graph');
      console.error('Failed to load graph:', err);
    } finally {
      setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [setNodes, setEdges, viewMode, labelFilters]); // nodeStats is setState, not a dependency

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
        proOptions={{ hideAttribution: true }}
      >
        <Controls />
        <Background 
          color="var(--border)" 
          gap={16}
          style={{ backgroundColor: 'var(--bg-primary)' }}
        />
      </ReactFlow>
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
