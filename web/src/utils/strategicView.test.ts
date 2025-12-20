import { describe, it, expect } from 'vitest';
import { filterStrategicNodes, filterStrategicEdges, calculateDownstreamStats } from './strategicView';
import type { GraphNode, GraphEdge } from '../types/models';

describe('Strategic View Filtering', () => {
  const createNode = (id: string, label: string, labels: string[] = []): GraphNode => ({
    id,
    label,
    state: 'ready',
    priority: 'normal',
    labels,
    blocked: false,
  });

  describe('filterStrategicNodes', () => {
    it('should return nodes with type:milestone labels', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Milestone', ['type:milestone', 'milestone:v1.0']),
        createNode('2', 'Task', ['component:backend']),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(1);
      expect(result[0].id).toBe('1');
    });

    it('should return nodes with type:epic labels', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Epic', ['type:epic', 'epic:auth']),
        createNode('2', 'Task', ['type:task']),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(1);
      expect(result[0].id).toBe('1');
    });

    it('should return nodes with both type:milestone and type:epic labels', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Milestone', ['type:milestone', 'milestone:v1.0']),
        createNode('2', 'Epic', ['type:epic', 'epic:auth']),
        createNode('3', 'Task', ['type:task', 'component:backend']),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(2);
      expect(result.map(n => n.id).sort()).toEqual(['1', '2']);
    });

    it('should return empty array when no strategic nodes', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Task', ['type:task', 'component:backend']),
        createNode('2', 'Task', ['type:feature']),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(0);
    });

    it('should handle nodes with multiple labels', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Epic', ['type:epic', 'epic:auth', 'milestone:v1.0', 'component:backend']),
        createNode('2', 'Task', ['type:task', 'component:backend']),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(1);
      expect(result[0].id).toBe('1');
    });

    it('should not match milestone: or epic: prefixes without type:', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Task', ['milestone:v1.0', 'epic:auth', 'component:backend']),
        createNode('2', 'Task', ['type:task']),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(0);
    });

    it('should handle empty labels array', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Task', []),
      ];

      const result = filterStrategicNodes(nodes);
      
      expect(result).toHaveLength(0);
    });
  });

  describe('filterStrategicEdges', () => {
    it('should only include edges between strategic nodes', () => {
      const strategicNodeIds = new Set(['1', '2']);
      const edges: GraphEdge[] = [
        { from: '1', to: '2' }, // strategic -> strategic
        { from: '1', to: '3' }, // strategic -> tactical
        { from: '3', to: '2' }, // tactical -> strategic
        { from: '3', to: '4' }, // tactical -> tactical
      ];

      const result = filterStrategicEdges(edges, strategicNodeIds);
      
      expect(result).toHaveLength(1);
      expect(result[0]).toEqual({ from: '1', to: '2' });
    });

    it('should handle empty strategic nodes', () => {
      const strategicNodeIds = new Set<string>();
      const edges: GraphEdge[] = [
        { from: '1', to: '2' },
      ];

      const result = filterStrategicEdges(edges, strategicNodeIds);
      
      expect(result).toHaveLength(0);
    });

    it('should handle empty edges', () => {
      const strategicNodeIds = new Set(['1', '2']);
      const edges: GraphEdge[] = [];

      const result = filterStrategicEdges(edges, strategicNodeIds);
      
      expect(result).toHaveLength(0);
    });
  });

  describe('calculateDownstreamStats', () => {
    it('should calculate stats for node with dependencies', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Epic', ['type:epic', 'epic:auth']),
        { ...createNode('2', 'Task1', ['type:task', 'component:backend']), state: 'done' },
        { ...createNode('3', 'Task2', ['type:task', 'component:backend']), state: 'in_progress' },
        { ...createNode('4', 'Task3', ['type:task', 'component:backend']), state: 'ready', blocked: true },
      ];
      const edges: GraphEdge[] = [
        { from: '1', to: '2' },
        { from: '1', to: '3' },
        { from: '1', to: '4' },
      ];

      const result = calculateDownstreamStats('1', nodes, edges);
      
      expect(result.total).toBe(3);
      expect(result.done).toBe(1);
      expect(result.inProgress).toBe(1);
      expect(result.blocked).toBe(1);
      expect(result.ready).toBe(1);
    });

    it('should handle transitive dependencies', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Epic', ['type:epic', 'epic:auth']),
        createNode('2', 'Task1', ['type:task', 'component:backend']),
        { ...createNode('3', 'Task2', ['type:task', 'component:backend']), state: 'done' },
      ];
      const edges: GraphEdge[] = [
        { from: '1', to: '2' },
        { from: '2', to: '3' }, // transitive: 1 -> 2 -> 3
      ];

      const result = calculateDownstreamStats('1', nodes, edges);
      
      expect(result.total).toBe(2); // Task1 + Task2
      expect(result.done).toBe(1);  // Task2
    });

    it('should return zero stats for node with no dependencies', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Task', ['type:task', 'component:backend']),
      ];
      const edges: GraphEdge[] = [];

      const result = calculateDownstreamStats('1', nodes, edges);
      
      expect(result.total).toBe(0);
      expect(result.done).toBe(0);
      expect(result.inProgress).toBe(0);
      expect(result.blocked).toBe(0);
      expect(result.ready).toBe(0);
    });

    it('should not count the node itself in stats', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Epic', ['type:epic', 'epic:auth']),
        createNode('2', 'Task', ['type:task', 'component:backend']),
      ];
      const edges: GraphEdge[] = [
        { from: '1', to: '2' },
      ];

      const result = calculateDownstreamStats('1', nodes, edges);
      
      expect(result.total).toBe(1); // Only Task, not Epic
    });

    it('should handle diamond dependencies without double counting', () => {
      const nodes: GraphNode[] = [
        createNode('1', 'Epic', ['type:epic', 'epic:auth']),
        createNode('2', 'Task1', ['type:task', 'component:backend']),
        createNode('3', 'Task2', ['type:task', 'component:backend']),
        createNode('4', 'Task3', ['type:task', 'component:backend']),
      ];
      const edges: GraphEdge[] = [
        { from: '1', to: '2' },
        { from: '1', to: '3' },
        { from: '2', to: '4' },
        { from: '3', to: '4' }, // diamond: 1 -> 2,3 -> 4
      ];

      const result = calculateDownstreamStats('1', nodes, edges);
      
      expect(result.total).toBe(3); // Task1, Task2, Task3 (counted once)
    });
  });
});
