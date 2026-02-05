import { describe, it, expect } from 'vitest';
import { 
  getNodeLevel, 
  extractNodeType, 
  assignNodesToSubgraphs,
  aggregateEdgesForCollapsed 
} from './subgraphClustering';
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
      const node: GraphNode = {
        id: 'milestone-1',
        label: 'Version 1.0',
        state: 'backlog',
        priority: 'critical',
        labels: ['type:milestone', 'milestone:v1.0'],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(1);
    });

    it('should return correct level for epic', () => {
      const node: GraphNode = {
        id: 'epic-1',
        label: 'Feature Epic',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic', 'epic:feature-x'],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(2);
    });

    it('should return correct level for task', () => {
      const node: GraphNode = {
        id: 'task-1',
        label: 'Implement feature',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task', 'epic:feature-x'],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(4);
    });

    it('should handle multiple types at same level (task vs bug)', () => {
      const taskNode: GraphNode = {
        id: 'task-1',
        label: 'Task',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };
      
      const bugNode: GraphNode = {
        id: 'bug-1',
        label: 'Bug',
        state: 'ready',
        priority: 'high',
        labels: ['type:bug'],
      };
      
      expect(getNodeLevel(taskNode, hierarchy)).toBe(4);
      expect(getNodeLevel(bugNode, hierarchy)).toBe(4);
    });

    it('should return Infinity for nodes without type label', () => {
      const node: GraphNode = {
        id: 'orphan-1',
        label: 'Orphan',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['component:backend'],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(Infinity);
    });

    it('should return Infinity for unknown type', () => {
      const node: GraphNode = {
        id: 'unknown-1',
        label: 'Unknown',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:unknown'],
      };
      
      expect(getNodeLevel(node, hierarchy)).toBe(Infinity);
    });
  });

  describe('extractNodeType', () => {
    it('should extract type from type:X label', () => {
      const node: GraphNode = {
        id: 'test-1',
        label: 'Test',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task', 'epic:test'],
      };
      
      expect(extractNodeType(node)).toBe('task');
    });

    it('should return null if no type label exists', () => {
      const node: GraphNode = {
        id: 'test-1',
        label: 'Test',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['epic:test', 'component:web'],
      };
      
      expect(extractNodeType(node)).toBeNull();
    });

    it('should return first type if multiple exist (invalid but defensive)', () => {
      const node: GraphNode = {
        id: 'test-1',
        label: 'Test',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task', 'type:bug'],
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
        label: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const task2: GraphNode = {
        id: 'task-2',
        label: 'Task 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
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
        label: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
      };

      const epic2: GraphNode = {
        id: 'epic-2',
        label: 'Epic 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:epic'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const task2: GraphNode = {
        id: 'task-2',
        label: 'Task 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
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
        label: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
      };

      const epic2: GraphNode = {
        id: 'epic-2',
        label: 'Epic 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:epic'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const task2: GraphNode = {
        id: 'task-2',
        label: 'Task 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
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
        label: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
      };

      const epic2: GraphNode = {
        id: 'epic-2',
        label: 'Epic 2',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:epic'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
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
        label: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
      };

      const story: GraphNode = {
        id: 'story-1',
        label: 'Story 1',
        state: 'in_progress',
        priority: "normal",
        blocked: false,
        labels: ['type:story'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const task2: GraphNode = {
        id: 'task-2',
        label: 'Task 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
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

  describe('aggregateEdgesForCollapsed', () => {
    const hierarchy: HierarchyLevelMap = {
      milestone: 1,
      epic: 2,
      story: 3,
      task: 4,
      bug: 4,
    };

    it('should aggregate edges from collapsed story to external nodes', () => {
      const story: GraphNode = {
        id: 'story-1',
        label: 'Story 1',
        state: 'in_progress',
        priority: "normal",
        blocked: false,
        labels: ['type:story'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const task2: GraphNode = {
        id: 'task-2',
        label: 'Task 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const nodes = [story, task1, task2];
      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'story-1', to: 'task-2' },
        { from: 'task-2', to: 'external-1' }, // This should aggregate to story-1
      ];

      const expansionState: ExpansionState = {
        'story-1': false, // Collapsed
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState, hierarchy);

      // Should have 1 virtual edge: story-1 → external-1
      expect(virtualEdges).toHaveLength(1);
      expect(virtualEdges[0].from).toBe('story-1');
      expect(virtualEdges[0].to).toBe('external-1');
      expect(virtualEdges[0].count).toBe(1);
      expect(virtualEdges[0].sourceEdgeIds).toContain('task-2→external-1');
    });

    it('should aggregate multiple edges into single virtual edge', () => {
      const story: GraphNode = {
        id: 'story-1',
        label: 'Story 1',
        state: 'in_progress',
        priority: "normal",
        blocked: false,
        labels: ['type:story'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const task2: GraphNode = {
        id: 'task-2',
        label: 'Task 2',
        state: 'ready',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const nodes = [story, task1, task2];
      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'story-1', to: 'task-2' },
        { from: 'task-1', to: 'external-1' },
        { from: 'task-2', to: 'external-1' }, // Both tasks → external-1
      ];

      const expansionState: ExpansionState = {
        'story-1': false,
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState, hierarchy);

      // Should aggregate 2 edges into 1 virtual edge with count=2
      expect(virtualEdges).toHaveLength(1);
      expect(virtualEdges[0].from).toBe('story-1');
      expect(virtualEdges[0].to).toBe('external-1');
      expect(virtualEdges[0].count).toBe(2);
      expect(virtualEdges[0].sourceEdgeIds).toHaveLength(2);
    });

    it('should aggregate incoming edges to collapsed container', () => {
      const story: GraphNode = {
        id: 'story-1',
        label: 'Story 1',
        state: 'in_progress',
        priority: "normal",
        blocked: false,
        labels: ['type:story'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const nodes = [story, task1];
      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'external-1', to: 'task-1' }, // External → child
        { from: 'external-2', to: 'task-1' }, // Another external → child
      ];

      const expansionState: ExpansionState = {
        'story-1': false,
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState, hierarchy);

      // Should aggregate: external-1 → task-1 becomes external-1 → story-1
      //                   external-2 → task-1 becomes external-2 → story-1
      expect(virtualEdges).toHaveLength(2);
      
      const incoming1 = virtualEdges.find(e => e.from === 'external-1' && e.to === 'story-1');
      const incoming2 = virtualEdges.find(e => e.from === 'external-2' && e.to === 'story-1');
      
      expect(incoming1).toBeDefined();
      expect(incoming2).toBeDefined();
    });

    it('should not aggregate edges when container is expanded', () => {
      const story: GraphNode = {
        id: 'story-1',
        label: 'Story 1',
        state: 'in_progress',
        priority: "normal",
        blocked: false,
        labels: ['type:story'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const nodes = [story, task1];
      const edges: GraphEdge[] = [
        { from: 'story-1', to: 'task-1' },
        { from: 'task-1', to: 'external-1' },
      ];

      const expansionState: ExpansionState = {
        'story-1': true, // Expanded
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState, hierarchy);

      // Should have no virtual edges when expanded
      expect(virtualEdges).toHaveLength(0);
    });

    it('should handle nested collapse (story in epic)', () => {
      const epic: GraphNode = {
        id: 'epic-1',
        label: 'Epic 1',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:epic'],
      };

      const story: GraphNode = {
        id: 'story-1',
        label: 'Story 1',
        state: 'in_progress',
        priority: "normal",
        blocked: false,
        labels: ['type:story'],
      };

      const task1: GraphNode = {
        id: 'task-1',
        label: 'Task 1',
        state: 'done',
        priority: "normal",
        blocked: false,
        labels: ['type:task'],
      };

      const nodes = [epic, story, task1];
      const edges: GraphEdge[] = [
        { from: 'epic-1', to: 'story-1' },
        { from: 'story-1', to: 'task-1' },
        { from: 'task-1', to: 'external-1' },
      ];

      const expansionState: ExpansionState = {
        'epic-1': false, // Epic collapsed (hides story too)
        'story-1': false,
      };

      const virtualEdges = aggregateEdgesForCollapsed(nodes, edges, expansionState, hierarchy);

      // Epic collapsed → should aggregate all children's edges
      const epicEdge = virtualEdges.find(e => e.from === 'epic-1' && e.to === 'external-1');
      expect(epicEdge).toBeDefined();
      expect(epicEdge!.count).toBe(1);
    });
  });
});
