import { describe, it, expect } from 'vitest';
import { getNodeLevel, extractNodeType, assignNodesToSubgraphs } from './subgraphClustering';
import type { GraphNode, GraphEdge } from '../types/models';
import type { HierarchyLevelMap } from '../types/subgraphCluster';

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
      const node: GraphNode = {
        id: 'milestone-1',
        title: 'Version 1.0',
        state: 'backlog',
        priority: 'critical',
        labels: ['type:milestone', 'milestone:v1.0'],
        dependencies: [],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(1);
    });

    it('should return correct level for epic', () => {
      const node: GraphNode = {
        id: 'epic-1',
        title: 'Feature Epic',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic', 'epic:feature-x'],
        dependencies: [],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(2);
    });

    it('should return correct level for task', () => {
      const node: GraphNode = {
        id: 'task-1',
        title: 'Implement feature',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task', 'epic:feature-x'],
        dependencies: [],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(4);
    });

    it('should handle multiple types at same level (task vs bug)', () => {
      const taskNode: GraphNode = {
        id: 'task-1',
        title: 'Task',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };
      
      const bugNode: GraphNode = {
        id: 'bug-1',
        title: 'Bug',
        state: 'ready',
        priority: 'high',
        labels: ['type:bug'],
        dependencies: [],
      };
      
      expect(getNodeLevel(taskNode, hierarchy)).toBe(4);
      expect(getNodeLevel(bugNode, hierarchy)).toBe(4);
    });

    it('should return Infinity for nodes without type label', () => {
      const node: GraphNode = {
        id: 'orphan-1',
        title: 'Orphan',
        state: 'ready',
        priority: 'normal',
        labels: ['component:backend'],
        dependencies: [],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(Infinity);
    });

    it('should return Infinity for unknown type', () => {
      const node: GraphNode = {
        id: 'unknown-1',
        title: 'Unknown',
        state: 'ready',
        priority: 'normal',
        labels: ['type:unknown'],
        dependencies: [],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(Infinity);
    });
  });

  describe('extractNodeType', () => {
    it('should extract type from type:X label', () => {
      const node: GraphNode = {
        id: 'test-1',
        title: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task', 'epic:test'],
        dependencies: [],
      };
      
      expect(extractNodeType(node)).toBe('task');
    });

    it('should return null if no type label exists', () => {
      const node: GraphNode = {
        id: 'test-1',
        title: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['epic:test', 'component:web'],
        dependencies: [],
      };
      
      expect(extractNodeType(node)).toBeNull();
    });

    it('should return first type if multiple exist (invalid but defensive)', () => {
      const node: GraphNode = {
        id: 'test-1',
        title: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task', 'type:bug'],
        dependencies: [],
      };
      
      expect(extractNodeType(node)).toBe('task');
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
      const epic: GraphNode = {
        id: 'epic-1',
        title: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
        dependencies: ['task-1', 'task-2'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        title: 'Task 1',
        state: 'done',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const task2: GraphNode = {
        id: 'task-2',
        title: 'Task 2',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const nodes = [epic, task1, task2];
      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'task-1' },
        { from: 'epic-1', to: 'task-2' },
      ];

      const result = assignNodesToSubgraphs(nodes, edges, hierarchy);

      expect(result.clusters.size).toBe(1);
      expect(result.clusters.has('epic-1')).toBe(true);
      
      const cluster = result.clusters.get('epic-1')!;
      expect(cluster.nodes).toHaveLength(3); // epic + 2 tasks
      expect(cluster.nodes.map(n => n.id).sort()).toEqual(['epic-1', 'task-1', 'task-2']);
      expect(cluster.internalEdges).toHaveLength(2);
      expect(result.crossClusterEdges).toHaveLength(0);
    });

    it('should create separate clusters for multiple epics', () => {
      const epic1: GraphNode = {
        id: 'epic-1',
        title: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
        dependencies: ['task-1'],
      };

      const epic2: GraphNode = {
        id: 'epic-2',
        title: 'Epic 2',
        state: 'ready',
        priority: 'normal',
        labels: ['type:epic'],
        dependencies: ['task-2'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        title: 'Task 1',
        state: 'done',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const task2: GraphNode = {
        id: 'task-2',
        title: 'Task 2',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const nodes = [epic1, epic2, task1, task2];
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
      const epic1: GraphNode = {
        id: 'epic-1',
        title: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
        dependencies: ['task-1'],
      };

      const epic2: GraphNode = {
        id: 'epic-2',
        title: 'Epic 2',
        state: 'ready',
        priority: 'normal',
        labels: ['type:epic'],
        dependencies: ['task-2'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        title: 'Task 1',
        state: 'done',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const task2: GraphNode = {
        id: 'task-2',
        title: 'Task 2',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: ['task-1'], // Cross-cluster dependency
      };

      const nodes = [epic1, epic2, task1, task2];
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
      const epic1: GraphNode = {
        id: 'epic-1',
        title: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
        dependencies: ['epic-2', 'task-1'],
      };

      const epic2: GraphNode = {
        id: 'epic-2',
        title: 'Epic 2',
        state: 'done',
        priority: 'normal',
        labels: ['type:epic'],
        dependencies: [],
      };

      const task1: GraphNode = {
        id: 'task-1',
        title: 'Task 1',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const nodes = [epic1, epic2, task1];
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
      const epic: GraphNode = {
        id: 'epic-1',
        title: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
        dependencies: ['story-1'],
      };

      const story: GraphNode = {
        id: 'story-1',
        title: 'Story 1',
        state: 'in_progress',
        priority: 'normal',
        labels: ['type:story'],
        dependencies: ['task-1', 'task-2'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        title: 'Task 1',
        state: 'done',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const task2: GraphNode = {
        id: 'task-2',
        title: 'Task 2',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        dependencies: [],
      };

      const nodes = [epic, story, task1, task2];
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
  });
});
