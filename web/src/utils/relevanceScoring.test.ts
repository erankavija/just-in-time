import { describe, it, expect } from 'vitest';
import type { GraphNode, GraphEdge } from '../types/models';
import {
  scoreNodeRelevance,
  computeNodeDegree,
  orderNodesByRelevance,
} from './relevanceScoring';

describe('relevanceScoring', () => {
  const nodes: GraphNode[] = [
    {
      id: 'selected',
      label: 'Selected item',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task'],
      blocked: false,
    },
    {
      id: 'blocker',
      label: 'Blocking item',
      state: 'in_progress',
      priority: 'high',
      labels: ['type:task'],
      blocked: false,
    },
    {
      id: 'blocked',
      label: 'Blocked item',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task'],
      blocked: true,
    },
    {
      id: 'critical',
      label: 'Critical priority',
      state: 'ready',
      priority: 'critical',
      labels: ['type:task'],
      blocked: false,
    },
    {
      id: 'high-degree',
      label: 'High degree item',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task'],
      blocked: false,
    },
    {
      id: 'low-priority',
      label: 'Low priority',
      state: 'ready',
      priority: 'low',
      labels: ['type:task'],
      blocked: false,
    },
    {
      id: 'normal-1',
      label: 'Normal item 1',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task'],
      blocked: false,
    },
    {
      id: 'normal-2',
      label: 'Normal item 2',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task'],
      blocked: false,
    },
  ];

  const edges: GraphEdge[] = [
    // high-degree has 4 dependencies (high degree)
    { from: 'high-degree', to: 'blocker' },
    { from: 'high-degree', to: 'critical' },
    { from: 'high-degree', to: 'normal-1' },
    { from: 'high-degree', to: 'normal-2' },
    // blocker blocks selected
    { from: 'selected', to: 'blocker' },
  ];

  describe('computeNodeDegree', () => {
    it('should count total edges (in + out)', () => {
      expect(computeNodeDegree('high-degree', edges)).toBe(4); // 4 outbound
      expect(computeNodeDegree('blocker', edges)).toBe(2); // 1 inbound + 1 outbound (from high-degree, to selected)
      expect(computeNodeDegree('selected', edges)).toBe(1); // 1 outbound
      expect(computeNodeDegree('normal-1', edges)).toBe(1); // 1 inbound
      expect(computeNodeDegree('low-priority', edges)).toBe(0); // no edges
    });
  });

  describe('scoreNodeRelevance', () => {
    it('should score selected/highlighted path highest', () => {
      const score = scoreNodeRelevance('selected', nodes, edges, new Set(['selected']));
      expect(score).toBeGreaterThan(10000); // Selected path weight
    });

    it('should score blocked items high', () => {
      const blockedScore = scoreNodeRelevance('blocked', nodes, edges, new Set());
      const normalScore = scoreNodeRelevance('normal-1', nodes, edges, new Set());
      expect(blockedScore).toBeGreaterThan(normalScore);
    });

    it('should factor in priority (critical > high > normal > low)', () => {
      const criticalScore = scoreNodeRelevance('critical', nodes, edges, new Set());
      const highScore = scoreNodeRelevance('blocker', nodes, edges, new Set()); // blocker has high priority
      const normalScore = scoreNodeRelevance('normal-1', nodes, edges, new Set());
      const lowScore = scoreNodeRelevance('low-priority', nodes, edges, new Set());

      expect(criticalScore).toBeGreaterThan(highScore);
      expect(highScore).toBeGreaterThan(normalScore);
      expect(normalScore).toBeGreaterThan(lowScore);
    });

    it('should factor in node degree', () => {
      // high-degree has 4 edges (degree 4), low-priority has 0 edges (degree 0)
      // Test with same priority to isolate degree factor
      const updatedNodes = nodes.map(n => 
        n.id === 'low-priority' ? { ...n, priority: 'normal' as const } : n
      );
      const highDegreeScore = scoreNodeRelevance('high-degree', updatedNodes, edges, new Set());
      const noEdgesScore = scoreNodeRelevance('low-priority', updatedNodes, edges, new Set());
      
      // With same priority, high-degree should score higher due to degree
      expect(highDegreeScore).toBeGreaterThan(noEdgesScore);
    });

    it('should use stable ID as tie-breaker', () => {
      // Two nodes with same priority, not blocked, no edges
      const score1 = scoreNodeRelevance('normal-1', nodes, edges, new Set());
      const score2 = scoreNodeRelevance('normal-2', nodes, edges, new Set());
      
      // Scores should be different (ID hash tie-breaker)
      expect(score1).not.toBe(score2);
    });

    it('should combine multiple factors correctly', () => {
      // Selected path should trump everything
      const selectedScore = scoreNodeRelevance('selected', nodes, edges, new Set(['selected']));
      const criticalScore = scoreNodeRelevance('critical', nodes, edges, new Set());
      expect(selectedScore).toBeGreaterThan(criticalScore);

      // Blocked should be more important than just high priority
      const blockedScore = scoreNodeRelevance('blocked', nodes, edges, new Set());
      const highPriorityScore = scoreNodeRelevance('blocker', nodes, edges, new Set());
      expect(blockedScore).toBeGreaterThan(highPriorityScore);
    });
  });

  describe('orderNodesByRelevance', () => {
    it('should order nodes by relevance score descending', () => {
      const ordered = orderNodesByRelevance(nodes, edges, new Set(['selected']));

      // Selected should be first
      expect(ordered[0].id).toBe('selected');
      
      // Blocked should be near the top
      expect(ordered.slice(0, 3).some(n => n.id === 'blocked')).toBe(true);
      
      // Critical priority should be high
      expect(ordered.slice(0, 4).some(n => n.id === 'critical')).toBe(true);
      
      // Low priority should be near the end
      const lowIndex = ordered.findIndex(n => n.id === 'low-priority');
      expect(lowIndex).toBeGreaterThan(ordered.length / 2);
    });

    it('should maintain deterministic ordering for same scores', () => {
      const ordered1 = orderNodesByRelevance(nodes, edges, new Set());
      const ordered2 = orderNodesByRelevance(nodes, edges, new Set());
      
      expect(ordered1.map(n => n.id)).toEqual(ordered2.map(n => n.id));
    });

    it('should handle empty node list', () => {
      const ordered = orderNodesByRelevance([], edges, new Set());
      expect(ordered).toHaveLength(0);
    });

    it('should handle empty selected set', () => {
      const ordered = orderNodesByRelevance(nodes, edges, new Set());
      expect(ordered).toHaveLength(nodes.length);
      // Should still order by blocked, priority, degree
      expect(ordered[0].id).toBe('blocked'); // Blocked is most relevant without selection
    });
  });
});
