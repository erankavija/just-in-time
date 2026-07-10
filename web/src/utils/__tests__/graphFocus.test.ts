import { describe, it, expect } from 'vitest';
import { findParentClusters } from '../graphFocus';
import { graphNode } from '../../test/graphNode';
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
            graphNode('epic-1'),
            graphNode('task-1'),
            graphNode('task-2'),
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
            graphNode('epic-1'),
            graphNode('task-1'),
            graphNode('task-2'),
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
            graphNode('epic-1'),
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
            graphNode('story-1'),
            graphNode('task-1'),
            graphNode('task-2'),
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
            graphNode('epic-1'),
            graphNode('task-1'),
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
            graphNode('epic-2'),
            graphNode('task-2'),
            graphNode('task-3'),
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
            graphNode('milestone-1'),
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
            graphNode('epic-1'),
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
            graphNode('story-1'),
            graphNode('task-1'),
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
