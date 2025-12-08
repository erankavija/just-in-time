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

interface GraphViewProps {
  onNodeClick?: (issueId: string) => void;
}

export function GraphView({ onNodeClick }: GraphViewProps) {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadGraph = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await apiClient.getGraph();
      
      const flowNodes: Node[] = data.nodes.map((node: ApiGraphNode) => ({
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
        },
      }));

      // Note: edge.from depends on edge.to, so for L->R layout, 
      // the dependency (to) should be on the left, and dependent (from) on the right
      const flowEdges: Edge[] = data.edges.map((edge) => ({
        id: `${edge.from}-${edge.to}`,
        source: edge.to,   // Swap: dependency goes on the left
        target: edge.from, // Swap: dependent goes on the right
        sourceHandle: 'right',
        targetHandle: 'left',
        type: 'smoothstep',
        animated: false,
        style: {
          stroke: 'var(--border-hover)',
          strokeWidth: 2,
        },
        markerEnd: {
          type: MarkerType.ArrowClosed,
          color: 'var(--border-hover)',
        },
      }));

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
  }, [setNodes, setEdges]);

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
