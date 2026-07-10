import { describe, it, expect } from 'vitest';
import { prepareClusteredGraphForReactFlow } from './clusteredGraphLayout';
import { graphNode } from '../test/graphNode';
import type { GraphNode, GraphEdge } from '../types/models';
import type { HierarchyLevelMap, ExpansionState } from '../types/subgraphCluster';

describe('clusteredGraphLayout', () => {
  describe('prepareClusteredGraphForReactFlow', () => {
    const hierarchy: HierarchyLevelMap = {
      epic: 2,
      story: 3,
      task: 4,
    };

    it('should convert simple graph to clustered format', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
      ];

      const expansionState: ExpansionState = {
        'epic-1': true, // Expanded
      };

      const result = prepareClusteredGraphForReactFlow(nodes, edges, hierarchy, expansionState);

      // Should only have epic cluster, not task sub-cluster
      expect(result.clusters.length).toBe(1);
      expect(result.visibleNodes).toHaveLength(2); // Epic + Task both visible
      expect(result.visibleEdges).toHaveLength(1); // Internal edge
      expect(result.virtualEdges).toHaveLength(0); // No collapsed containers
    });

    it('should hide children when container is collapsed', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1', 'task-2'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-1', to: 'task-2' },
      ];

      const expansionState: ExpansionState = {
        'epic-1': false, // Collapsed
      };

      const result = prepareClusteredGraphForReactFlow(nodes, edges, hierarchy, expansionState);

      expect(result.visibleNodes).toHaveLength(1); // Only epic visible
      expect(result.visibleNodes[0].id).toBe('epic-1');
      expect(result.visibleEdges).toHaveLength(0); // Internal edges hidden
    });

    it('should create virtual edges for collapsed containers', () => {
      // Setup: epic-1 contains task-1, which depends on epic-2's task-2
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1'], cluster: 'epic-1', rank: 2 }),
        graphNode('epic-2', { type: 'epic', children: ['task-2'], cluster: 'epic-2', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 1 }),
        graphNode('task-2', { type: 'task', parent: 'epic-2', cluster: 'epic-2', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-2', to: 'task-2' },
        { from: 'task-1', to: 'task-2' }, // Cross-cluster edge
      ];

      const expansionState: ExpansionState = {
        'epic-1': false, // Collapsed
        'epic-2': true,  // Expanded
      };

      const result = prepareClusteredGraphForReactFlow(nodes, edges, hierarchy, expansionState);

      // Epic-1 collapsed → task-1 hidden
      // Virtual edge should be: epic-1 → task-2 (aggregated from task-1 → task-2)
      expect(result.virtualEdges).toHaveLength(1);
      expect(result.virtualEdges[0].from).toBe('epic-1');
      expect(result.virtualEdges[0].to).toBe('task-2');
      expect(result.virtualEdges[0].count).toBe(1);

      // Visible nodes: epic-1, epic-2, task-2 (task-1 hidden)
      expect(result.visibleNodes).toHaveLength(3);
      expect(result.visibleNodes.map(n => n.id).sort()).toEqual(['epic-1', 'epic-2', 'task-2']);
    });

    it('should handle multiple clusters with cross-cluster edges', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('epic-2', { type: 'epic', children: ['task-2'], cluster: 'epic-2', rank: 2 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'epic-2', cluster: 'epic-2', rank: 1 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-2', to: 'task-2' },
        { from: 'task-2', to: 'task-1' }, // Cross-cluster
      ];

      const expansionState: ExpansionState = {
        'epic-1': true,
        'epic-2': true,
      };

      const result = prepareClusteredGraphForReactFlow(nodes, edges, hierarchy, expansionState);

      expect(result.clusters.length).toBe(2);
      expect(result.visibleNodes).toHaveLength(4); // All nodes visible

      // Should have cross-cluster edge preserved
      const crossEdge = result.visibleEdges.find(
        e => e.from === 'task-2' && e.to === 'task-1'
      );
      expect(crossEdge).toBeDefined();
    });

    it('should provide cluster metadata for UI rendering', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['story-1'], cluster: 'epic-1', rank: 2 }),
        graphNode('story-1', { type: 'story', parent: 'epic-1', children: ['task-1', 'task-2'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'story-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'story-1' },
        { from: 'story-1', to: 'task-1' },
        { from: 'story-1', to: 'task-2' },
      ];

      const expansionState: ExpansionState = {
        'epic-1': true,
        'story-1': false, // Story collapsed
      };

      const result = prepareClusteredGraphForReactFlow(nodes, edges, hierarchy, expansionState);

      const cluster = result.clusters.find(c => c.containerId === 'epic-1')!;
      expect(cluster).toBeDefined();
      expect(cluster.nodes).toHaveLength(4); // All 4 nodes in cluster

      // Visible nodes should be epic + story (tasks hidden by collapsed story)
      expect(result.visibleNodes).toHaveLength(2);
      expect(result.visibleNodes.map(n => n.id).sort()).toEqual(['epic-1', 'story-1']);
    });

    it('should handle empty expansion state (all expanded by default)', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
      ];

      const result = prepareClusteredGraphForReactFlow(nodes, edges, hierarchy, {});

      // Empty expansion state means everything collapsed (default: false)
      // Only epic-1 visible, task-1 hidden
      expect(result.visibleNodes).toHaveLength(1);
      expect(result.visibleNodes[0].id).toBe('epic-1');
    });
  });
});
