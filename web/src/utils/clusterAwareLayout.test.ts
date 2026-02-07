import { describe, it, expect } from 'vitest';
import { 
  computeClusterPositions,
  layoutNodesWithinCluster,
  createClusterAwareLayout,
} from './clusterAwareLayout';
import type { SubgraphCluster } from '../types/subgraphCluster';
import type { GraphNode, GraphEdge } from '../types/models';

describe('computeClusterPositions', () => {
  it('should order clusters by topological sort of cross-cluster dependencies', () => {
    const clusters: SubgraphCluster[] = [
      {
        containerId: 'epic1',
        containerLevel: 2,
        nodes: [{ id: 'epic1', labels: ['type:epic'] } as GraphNode],
        internalEdges: [],
        outgoingEdges: [],
        incomingEdges: [],
      },
      {
        containerId: 'epic2',
        containerLevel: 2,
        nodes: [{ id: 'epic2', labels: ['type:epic'] } as GraphNode],
        internalEdges: [],
        outgoingEdges: [],
        incomingEdges: [],
      },
    ];
    
    // epic1 depends on epic2 (epic2 should be left, epic1 right)
    const crossClusterEdges: GraphEdge[] = [
      { from: 'epic1', to: 'epic2' },
    ];
    
    const positions = computeClusterPositions(clusters, crossClusterEdges);
    
    expect(positions.get('epic2')!.x).toBeLessThan(positions.get('epic1')!.x);
  });

  it('should handle clusters with no cross-cluster dependencies', () => {
    const clusters: SubgraphCluster[] = [
      {
        containerId: 'epic1',
        containerLevel: 2,
        nodes: [{ id: 'epic1', labels: ['type:epic'] } as GraphNode],
        internalEdges: [],
        outgoingEdges: [],
        incomingEdges: [],
      },
      {
        containerId: 'epic2',
        containerLevel: 2,
        nodes: [{ id: 'epic2', labels: ['type:epic'] } as GraphNode],
        internalEdges: [],
        outgoingEdges: [],
        incomingEdges: [],
      },
    ];
    
    const positions = computeClusterPositions(clusters, []);
    
    // Both should have positions, order doesn't matter
    expect(positions.has('epic1')).toBe(true);
    expect(positions.has('epic2')).toBe(true);
  });

  it('should handle transitive dependencies (A→B→C should order C,B,A)', () => {
    const clusters: SubgraphCluster[] = [
      { containerId: 'A', containerLevel: 2, parentClusterId: null, nodes: [{ id: 'A', labels: ['type:epic'] } as GraphNode], internalEdges: [], outgoingEdges: [], incomingEdges: [] },
      { containerId: 'B', containerLevel: 2, parentClusterId: null, nodes: [{ id: 'B', labels: ['type:epic'] } as GraphNode], internalEdges: [], outgoingEdges: [], incomingEdges: [] },
      { containerId: 'C', containerLevel: 2, parentClusterId: null, nodes: [{ id: 'C', labels: ['type:epic'] } as GraphNode], internalEdges: [], outgoingEdges: [], incomingEdges: [] },
    ];
    
    const crossClusterEdges: GraphEdge[] = [
      { from: 'A', to: 'B' },
      { from: 'B', to: 'C' },
    ];
    
    const positions = computeClusterPositions(clusters, crossClusterEdges);
    
    expect(positions.get('C')!.x).toBeLessThan(positions.get('B')!.x);
    expect(positions.get('B')!.x).toBeLessThan(positions.get('A')!.x);
  });
});

describe('layoutNodesWithinCluster', () => {
  it('should layout nodes vertically in a compact stack', () => {
    const nodes: GraphNode[] = [
      { id: 'task1', labels: ['type:task'] },
      { id: 'task2', labels: ['type:task'] },
      { id: 'task3', labels: ['type:task'] },
    ] as GraphNode[];
    
    const internalEdges: GraphEdge[] = [];
    
    const layout = layoutNodesWithinCluster(nodes, internalEdges);
    
    expect(layout.nodes).toHaveLength(3);
    expect(layout.width).toBeGreaterThan(0);
    expect(layout.height).toBeGreaterThan(0);
    
    // Nodes should be stacked vertically
    const y0 = layout.nodes[0].position.y;
    const y1 = layout.nodes[1].position.y;
    const y2 = layout.nodes[2].position.y;
    
    expect(y1).toBeGreaterThan(y0);
    expect(y2).toBeGreaterThan(y1);
  });

  it('should handle single-node clusters', () => {
    const nodes: GraphNode[] = [
      { id: 'epic1', labels: ['type:epic'] },
    ] as GraphNode[];
    
    const layout = layoutNodesWithinCluster(nodes, []);
    
    expect(layout.nodes).toHaveLength(1);
    expect(layout.nodes[0].id).toBe('epic1');
  });
});

describe('createClusterAwareLayout', () => {
  it('should create complete layout with cluster and node positions', () => {
    const clusters: SubgraphCluster[] = [
      {
        containerId: 'epic1',
        containerLevel: 2,
        nodes: [
          { id: 'epic1', labels: ['type:epic'] } as GraphNode,
          { id: 'task1', labels: ['type:task'] } as GraphNode,
        ],
        internalEdges: [
          { from: 'epic1', to: 'task1' },
        ],
        outgoingEdges: [],
        incomingEdges: [],
      },
    ];
    
    const crossClusterEdges: GraphEdge[] = [];
    
    const layout = createClusterAwareLayout(clusters, crossClusterEdges);
    
    expect(layout.nodes).toHaveLength(2);
    expect(layout.nodes.every(n => n.position && n.position.x >= 0 && n.position.y >= 0)).toBe(true);
  });
});
