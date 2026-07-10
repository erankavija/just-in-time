import { describe, it, expect } from 'vitest';
import {
  getNodeLevel,
  assignNodesToSubgraphs,
  aggregateEdgesForCollapsed,
  assignNodesToClusters
} from './subgraphClustering';
import { graphNode } from '../test/graphNode';
import type { GraphNode, GraphEdge } from '../types/models';
import type { HierarchyLevelMap, ExpansionState } from '../types/subgraphCluster';

describe('subgraphClustering', () => {
  describe('getNodeLevel', () => {
    const hierarchy: HierarchyLevelMap = {
      milestone: 1,
      epic: 2,
      story: 3,
      task: 4,
      bug: 4,
    };

    it('should return correct level for milestone', () => {
      const node = graphNode('milestone-1', { type: 'milestone' });

      expect(getNodeLevel(node, hierarchy)).toBe(1);
    });

    it('should return correct level for epic', () => {
      const node = graphNode('epic-1', { type: 'epic' });

      expect(getNodeLevel(node, hierarchy)).toBe(2);
    });

    it('should return correct level for task', () => {
      const node = graphNode('task-1', { type: 'task' });

      expect(getNodeLevel(node, hierarchy)).toBe(4);
    });

    it('should handle multiple types at same level (task vs bug)', () => {
      const taskNode = graphNode('task-1', { type: 'task' });
      const bugNode = graphNode('bug-1', { type: 'bug' });

      expect(getNodeLevel(taskNode, hierarchy)).toBe(4);
      expect(getNodeLevel(bugNode, hierarchy)).toBe(4);
    });

    it('should return Infinity for nodes the server reports as untyped', () => {
      const node = graphNode('orphan-1', { type: null });

      expect(getNodeLevel(node, hierarchy)).toBe(Infinity);
    });

    it('should return Infinity for a type absent from the hierarchy', () => {
      const node = graphNode('unknown-1', { type: 'unknown' });

      expect(getNodeLevel(node, hierarchy)).toBe(Infinity);
    });
  });

  describe('assignNodesToSubgraphs', () => {
    const hierarchy: HierarchyLevelMap = {
      milestone: 1,
      epic: 2,
      story: 3,
      task: 4,
    };

    it('should create cluster for single epic with tasks', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1', 'task-2'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-1', to: 'task-2' },
      ];

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      // Uses level 2 (epic) as container level
      expect(result.clusters.size).toBe(1);
      expect(result.clusters.has('epic-1')).toBe(true);

      const cluster = result.clusters.get('epic-1')!;
      expect(cluster.nodes).toHaveLength(3); // epic + 2 tasks
      expect(cluster.nodes.map(n => n.id).sort()).toEqual(['epic-1', 'task-1', 'task-2']);
      expect(cluster.internalEdges).toHaveLength(2);
      expect(result.crossClusterEdges).toHaveLength(0);
    });

    it('should create separate clusters for multiple epics', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('epic-2', { type: 'epic', children: ['task-2'], cluster: 'epic-2', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'epic-2', cluster: 'epic-2', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-2', to: 'task-2' },
      ];

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      expect(result.clusters.size).toBe(2);
      expect(result.clusters.has('epic-1')).toBe(true);
      expect(result.clusters.has('epic-2')).toBe(true);

      const cluster1 = result.clusters.get('epic-1')!;
      expect(cluster1.nodes.map(n => n.id).sort()).toEqual(['epic-1', 'task-1']);

      const cluster2 = result.clusters.get('epic-2')!;
      expect(cluster2.nodes.map(n => n.id).sort()).toEqual(['epic-2', 'task-2']);
    });

    it('should preserve cross-cluster edges', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('epic-2', { type: 'epic', children: ['task-2'], cluster: 'epic-2', rank: 2 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'epic-2', cluster: 'epic-2', rank: 1 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-2', to: 'task-2' },
        { from: 'task-2', to: 'task-1' }, // Cross-cluster edge
      ];

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      expect(result.crossClusterEdges).toHaveLength(1);
      expect(result.crossClusterEdges[0]).toEqual({ from: 'task-2', to: 'task-1' });
    });

    it('should not pull higher-level nodes into cluster', () => {
      // epic-1 depends on epic-2, so the resolver makes epic-2 its child — but a
      // container at the clustering level always owns its own cluster.
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['epic-2', 'task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('epic-2', { type: 'epic', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-1', { type: 'task', parent: 'epic-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'epic-2' }, // Same level - should not cluster
        { from: 'epic-1', to: 'task-1' }, // Lower level - should cluster
      ];

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      const cluster1 = result.clusters.get('epic-1')!;
      expect(cluster1.nodes.map(n => n.id).sort()).toEqual(['epic-1', 'task-1']);
      // epic-2 should NOT be in epic-1's cluster
      expect(cluster1.nodes.find(n => n.id === 'epic-2')).toBeUndefined();

      // epic-1 → epic-2 should be a cross-cluster edge
      expect(result.crossClusterEdges.find(e => e.from === 'epic-1' && e.to === 'epic-2')).toBeDefined();
    });

    it('should handle stories as intermediate level', () => {
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

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      const cluster = result.clusters.get('epic-1')!;
      expect(cluster.nodes).toHaveLength(4); // epic + story + 2 tasks
      expect(cluster.nodes.map(n => n.id).sort()).toEqual(['epic-1', 'story-1', 'task-1', 'task-2']);
    });

    it('should cluster a node to the container that claims it directly, not to a deeper transitive reacher', () => {
      // Epic cx depends on tx directly; story sy reaches tx only through ty. The
      // resolver hands tx to cx, and the epic-level projection follows.
      const nodes: GraphNode[] = [
        graphNode('cx', { type: 'epic', children: ['tx'], cluster: 'cx', rank: 1 }),
        graphNode('sy', { type: 'story', children: ['ty'], cluster: 'sy', rank: 2 }),
        graphNode('ty', { type: 'task', parent: 'sy', cluster: 'sy', rank: 1 }),
        graphNode('tx', { type: 'task', parent: 'cx', cluster: 'cx', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'cx', to: 'tx' },
        { from: 'sy', to: 'ty' },
        { from: 'ty', to: 'tx' },
      ];

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      expect(result.clusters.size).toBe(1);
      expect(result.clusters.get('cx')!.nodes.map(n => n.id).sort()).toEqual(['cx', 'tx']);
      // sy and ty have no epic-level ancestor, so they stay outside every cluster.
      expect(result.orphanNodes.map(n => n.id).sort()).toEqual(['sy', 'ty']);
    });
  });

  describe('aggregateEdgesForCollapsed', () => {
    it('should aggregate edges from collapsed story to external nodes', () => {
      const nodes: GraphNode[] = [
        graphNode('story-1', { type: 'story', children: ['task-1', 'task-2'], cluster: 'story-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'story-1', to: 'task-2' },
        { from: 'task-2', to: 'external-1' }, // This should aggregate to story-1
      ];

      const expansionState: ExpansionState = {
        'story-1': false, // Collapsed
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState);

      // Should have 1 virtual edge: story-1 → external-1
      expect(virtualEdges).toHaveLength(1);
      expect(virtualEdges[0].from).toBe('story-1');
      expect(virtualEdges[0].to).toBe('external-1');
      expect(virtualEdges[0].count).toBe(1);
      expect(virtualEdges[0].sourceEdgeIds).toContain('task-2→external-1');
    });

    it('should aggregate multiple edges into single virtual edge', () => {
      const nodes: GraphNode[] = [
        graphNode('story-1', { type: 'story', children: ['task-1', 'task-2'], cluster: 'story-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'story-1', to: 'task-2' },
        { from: 'task-1', to: 'external-1' },
        { from: 'task-2', to: 'external-1' }, // Both tasks → external-1
      ];

      const expansionState: ExpansionState = {
        'story-1': false,
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState);

      // Should aggregate 2 edges into 1 virtual edge with count=2
      expect(virtualEdges).toHaveLength(1);
      expect(virtualEdges[0].from).toBe('story-1');
      expect(virtualEdges[0].to).toBe('external-1');
      expect(virtualEdges[0].count).toBe(2);
      expect(virtualEdges[0].sourceEdgeIds).toHaveLength(2);
    });

    it('should aggregate incoming edges to collapsed container', () => {
      const nodes: GraphNode[] = [
        graphNode('story-1', { type: 'story', children: ['task-1'], cluster: 'story-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'external-1', to: 'task-1' }, // External → child
        { from: 'external-2', to: 'task-1' }, // Another external → child
      ];

      const expansionState: ExpansionState = {
        'story-1': false,
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState);

      // Should aggregate: external-1 → task-1 becomes external-1 → story-1
      //                   external-2 → task-1 becomes external-2 → story-1
      expect(virtualEdges).toHaveLength(2);

      const incoming1 = virtualEdges.find(e => e.from === 'external-1' && e.to === 'story-1');
      const incoming2 = virtualEdges.find(e => e.from === 'external-2' && e.to === 'story-1');

      expect(incoming1).toBeDefined();
      expect(incoming2).toBeDefined();
    });

    it('should not aggregate edges when container is expanded', () => {
      const nodes: GraphNode[] = [
        graphNode('story-1', { type: 'story', children: ['task-1'], cluster: 'story-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'task-1', to: 'external-1' },
      ];

      const expansionState: ExpansionState = {
        'story-1': true, // Expanded
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState);

      // Should have no virtual edges when expanded
      expect(virtualEdges).toHaveLength(0);
    });

    it('should handle nested collapse (story in epic)', () => {
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['story-1'], cluster: 'epic-1', rank: 2 }),
        graphNode('story-1', { type: 'story', parent: 'epic-1', children: ['task-1'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'story-1' },
        { from: 'story-1', to: 'task-1' },
        { from: 'task-1', to: 'external-1' },
      ];

      const expansionState: ExpansionState = {
        'epic-1': false, // Epic collapsed (hides story too)
        'story-1': false,
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState);

      // Epic collapsed → should aggregate all children's edges
      const epicEdge = virtualEdges.find(e => e.from === 'epic-1' && e.to === 'external-1');
      expect(epicEdge).toBeDefined();
      expect(epicEdge!.count).toBe(1);
    });
  });

  describe('assignNodesToClusters (generic clustering)', () => {
    const hierarchy: HierarchyLevelMap = {
      milestone: 1,
      epic: 2,
      story: 3,
      task: 4,
    };

    it('should assign tasks to story sub-clusters at level 3', () => {
      // Epic contains Story1 (with Task1, Task2) and Story2 (with Task3)
      const nodes: GraphNode[] = [
        graphNode('epic-1', { type: 'epic', children: ['story-1', 'story-2'], cluster: 'epic-1', rank: 2 }),
        graphNode('story-1', { type: 'story', parent: 'epic-1', children: ['task-1', 'task-2'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'epic-1', rank: 0 }),
        graphNode('task-2', { type: 'task', parent: 'story-1', cluster: 'epic-1', rank: 0 }),
        graphNode('story-2', { type: 'story', parent: 'epic-1', children: ['task-3'], cluster: 'epic-1', rank: 1 }),
        graphNode('task-3', { type: 'task', parent: 'story-2', cluster: 'epic-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'story-1' },
        { from: 'story-1', to: 'task-1' },
        { from: 'story-1', to: 'task-2' },
        { from: 'epic-1', to: 'story-2' },
        { from: 'story-2', to: 'task-3' },
      ];

      const result = assignNodesToClusters(nodes, edges, hierarchy, 3); // Level 3 = story

      expect(result.clusters.size).toBe(2);

      const story1Cluster = result.clusters.get('story-1');
      expect(story1Cluster).toBeDefined();
      expect(story1Cluster!.nodes.map(n => n.id).sort()).toEqual(['story-1', 'task-1', 'task-2']);

      const story2Cluster = result.clusters.get('story-2');
      expect(story2Cluster).toBeDefined();
      expect(story2Cluster!.nodes.map(n => n.id).sort()).toEqual(['story-2', 'task-3']);
    });

    it('should handle story-to-story dependencies via task chains', () => {
      // Story1 → Task1 → Task2, Task2 → Story2 → Task3
      // Task2 depends on Story2's task → Story1's cluster still owns Task1 & Task2
      const nodes: GraphNode[] = [
        graphNode('story-1', { type: 'story', children: ['story-2', 'task-1', 'task-2'], cluster: 'story-1', rank: 4 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 3 }),
        graphNode('task-2', { type: 'task', parent: 'story-1', cluster: 'story-1', rank: 2 }),
        graphNode('story-2', { type: 'story', parent: 'story-1', children: ['task-3'], cluster: 'story-1', rank: 1 }),
        graphNode('task-3', { type: 'task', parent: 'story-2', cluster: 'story-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'task-1', to: 'task-2' },
        { from: 'task-2', to: 'story-2' }, // Cross-cluster dependency
        { from: 'story-2', to: 'task-3' },
      ];

      const result = assignNodesToClusters(nodes, edges, hierarchy, 3);

      expect(result.clusters.size).toBe(2);

      const story1Cluster = result.clusters.get('story-1');
      expect(story1Cluster!.nodes.map(n => n.id).sort()).toEqual(['story-1', 'task-1', 'task-2']);

      const story2Cluster = result.clusters.get('story-2');
      expect(story2Cluster!.nodes.map(n => n.id).sort()).toEqual(['story-2', 'task-3']);

      // Should have cross-cluster edge
      expect(result.crossClusterEdges.some(e => e.from === 'task-2' && e.to === 'story-2')).toBe(true);
    });

    it('should handle containers without children (childless clusters)', () => {
      const nodes: GraphNode[] = [
        graphNode('story-1', { type: 'story', cluster: 'story-1', rank: 0 }),
        graphNode('story-2', { type: 'story', children: ['task-1'], cluster: 'story-2', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-2', cluster: 'story-2', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'story-2', to: 'task-1' },
      ];

      const result = assignNodesToClusters(nodes, edges, hierarchy, 3);

      // Both stories should be clusters (even story-1 without children)
      expect(result.clusters.size).toBe(2);
      expect(result.clusters.has('story-1')).toBe(true);
      expect(result.clusters.has('story-2')).toBe(true);

      // Story-1 cluster contains only itself
      const story1Cluster = result.clusters.get('story-1');
      expect(story1Cluster!.nodes.map(n => n.id)).toEqual(['story-1']);

      // Story-2 cluster contains story and task
      const story2Cluster = result.clusters.get('story-2');
      expect(story2Cluster!.nodes.map(n => n.id).sort()).toEqual(['story-2', 'task-1']);
    });

    it('should work for epic-level clustering (level 2)', () => {
      const nodes: GraphNode[] = [
        graphNode('milestone-1', { type: 'milestone', children: ['epic-1'], cluster: 'milestone-1', rank: 3 }),
        graphNode('epic-1', { type: 'epic', parent: 'milestone-1', children: ['story-1'], cluster: 'milestone-1', rank: 2 }),
        graphNode('story-1', { type: 'story', parent: 'epic-1', children: ['task-1'], cluster: 'milestone-1', rank: 1 }),
        graphNode('task-1', { type: 'task', parent: 'story-1', cluster: 'milestone-1', rank: 0 }),
      ];

      const edges: GraphEdge[] = [
        { from: 'milestone-1', to: 'epic-1' },
        { from: 'epic-1', to: 'story-1' },
        { from: 'story-1', to: 'task-1' },
      ];

      const result = assignNodesToClusters(nodes, edges, hierarchy, 2); // Level 2 = epic

      expect(result.clusters.size).toBe(1);

      const epicCluster = result.clusters.get('epic-1');
      expect(epicCluster).toBeDefined();
      expect(epicCluster!.nodes.map(n => n.id).sort()).toEqual(['epic-1', 'story-1', 'task-1']);

      // Milestone should be an orphan (more strategic, not clustered)
      expect(result.orphanNodes.some(n => n.id === 'milestone-1')).toBe(true);
    });
  });
});
