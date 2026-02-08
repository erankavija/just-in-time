import { describe, it, expect } from 'vitest';
import { findParentClusters } from '../graphFocus';
import type { SubgraphCluster } from '../../types/subgraphCluster';

describe('graphFocus', () => {
  describe('findParentClusters', () => {
    it('should return empty array for node not in any cluster', () => {
      const clusters: SubgraphCluster[] = [
        {
          containerId: 'epic-1',
          containerLevel: 2,
          parentClusterId: null,
          nodes: [
            { id: 'epic-1', label: 'Epic 1', state: 'in_progress', priority: 'normal', labels: [], blocked: false },
            { id: 'task-1', label: 'Task 1', state: 'ready', priority: 'normal', labels: [], blocked: false },
            { id: 'task-2', label: 'Task 2', state: 'backlog', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
      ];

      const result = findParentClusters('task-999', clusters);
      expect(result).toEqual([]);
    });

    it('should find direct parent cluster', () => {
      const clusters: SubgraphCluster[] = [
        {
          containerId: 'epic-1',
          containerLevel: 2,
          parentClusterId: null,
          nodes: [
            { id: 'epic-1', label: 'Epic 1', state: 'in_progress', priority: 'normal', labels: [], blocked: false },
            { id: 'task-1', label: 'Task 1', state: 'ready', priority: 'normal', labels: [], blocked: false },
            { id: 'task-2', label: 'Task 2', state: 'backlog', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
      ];

      const result = findParentClusters('task-1', clusters);
      expect(result).toEqual(['epic-1']);
    });

    it('should find nested parent clusters (story within epic)', () => {
      const clusters: SubgraphCluster[] = [
        {
          containerId: 'epic-1',
          containerLevel: 2,
          parentClusterId: null,
          nodes: [
            { id: 'epic-1', label: 'Epic 1', state: 'in_progress', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
        {
          containerId: 'story-1',
          containerLevel: 3,
          parentClusterId: 'epic-1',
          nodes: [
            { id: 'story-1', label: 'Story 1', state: 'ready', priority: 'normal', labels: [], blocked: false },
            { id: 'task-1', label: 'Task 1', state: 'backlog', priority: 'normal', labels: [], blocked: false },
            { id: 'task-2', label: 'Task 2', state: 'backlog', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
      ];

      const result = findParentClusters('task-1', clusters);
      // Should return both story-1 and epic-1 (from innermost to outermost)
      expect(result).toEqual(['story-1', 'epic-1']);
    });

    it('should handle multiple clusters and find the correct parent', () => {
      const clusters: SubgraphCluster[] = [
        {
          containerId: 'epic-1',
          containerLevel: 2,
          parentClusterId: null,
          nodes: [
            { id: 'epic-1', label: 'Epic 1', state: 'in_progress', priority: 'normal', labels: [], blocked: false },
            { id: 'task-1', label: 'Task 1', state: 'ready', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
        {
          containerId: 'epic-2',
          containerLevel: 2,
          parentClusterId: null,
          nodes: [
            { id: 'epic-2', label: 'Epic 2', state: 'ready', priority: 'normal', labels: [], blocked: false },
            { id: 'task-2', label: 'Task 2', state: 'backlog', priority: 'normal', labels: [], blocked: false },
            { id: 'task-3', label: 'Task 3', state: 'backlog', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
      ];

      const result = findParentClusters('task-3', clusters);
      expect(result).toEqual(['epic-2']);
    });

    it('should handle deeply nested clusters (3 levels)', () => {
      const clusters: SubgraphCluster[] = [
        {
          containerId: 'milestone-1',
          containerLevel: 1,
          parentClusterId: null,
          nodes: [
            { id: 'milestone-1', label: 'Milestone 1', state: 'in_progress', priority: 'critical', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
        {
          containerId: 'epic-1',
          containerLevel: 2,
          parentClusterId: 'milestone-1',
          nodes: [
            { id: 'epic-1', label: 'Epic 1', state: 'in_progress', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
        {
          containerId: 'story-1',
          containerLevel: 3,
          parentClusterId: 'epic-1',
          nodes: [
            { id: 'story-1', label: 'Story 1', state: 'ready', priority: 'normal', labels: [], blocked: false },
            { id: 'task-1', label: 'Task 1', state: 'backlog', priority: 'normal', labels: [], blocked: false },
          ],
          internalEdges: [],
          incomingEdges: [],
          outgoingEdges: [],
        },
      ];

      const result = findParentClusters('task-1', clusters);
      expect(result).toEqual(['story-1', 'epic-1', 'milestone-1']);
    });

    it('should handle empty cluster list', () => {
      const result = findParentClusters('task-1', []);
      expect(result).toEqual([]);
    });
  });
});
