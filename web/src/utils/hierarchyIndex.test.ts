import { describe, it, expect } from 'vitest';
import { createHierarchyConfig } from '../types/hierarchy';
import type { GraphNode } from '../types/models';
import {
  extractNodeType,
  extractTierLabel,
  derivePrimaryTierIndex,
  groupNodesByPrimaryTier,
} from './hierarchyIndex';

describe('hierarchyIndex', () => {
  describe('extractNodeType', () => {
    it('should extract type from type:X label', () => {
      const node: GraphNode = {
        id: '1',
        label: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['type:milestone', 'milestone:v1.0'],
        blocked: false,
      };
      expect(extractNodeType(node)).toBe('milestone');
    });

    it('should return null if no type label exists', () => {
      const node: GraphNode = {
        id: '1',
        label: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['milestone:v1.0'],
        blocked: false,
      };
      expect(extractNodeType(node)).toBeNull();
    });

    it('should handle multiple type labels and return first', () => {
      const node: GraphNode = {
        id: '1',
        label: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['type:epic', 'type:milestone'], // Invalid but defensive
        blocked: false,
      };
      expect(extractNodeType(node)).toBe('epic');
    });
  });

  describe('extractTierLabel', () => {
    it('should extract value from tierType:value label', () => {
      const node: GraphNode = {
        id: '1',
        label: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['type:epic', 'milestone:v1.0'],
        blocked: false,
      };
      expect(extractTierLabel(node, 'milestone')).toBe('v1.0');
    });

    it('should return null if no matching label exists', () => {
      const node: GraphNode = {
        id: '1',
        label: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task'],
        blocked: false,
      };
      expect(extractTierLabel(node, 'milestone')).toBeNull();
    });

    it('should handle label with colons in value', () => {
      const node: GraphNode = {
        id: '1',
        label: 'Test',
        state: 'ready',
        priority: 'normal',
        labels: ['release:2024-Q1:beta'],
        blocked: false,
      };
      expect(extractTierLabel(node, 'release')).toBe('2024-Q1:beta');
    });
  });

  describe('derivePrimaryTierIndex - default hierarchy (milestone → epic)', () => {
    const config = createHierarchyConfig(['milestone', 'epic']);

    const milestones: GraphNode[] = [
      {
        id: 'm1',
        label: 'Milestone v0.1',
        state: 'done',
        priority: 'high',
        labels: ['type:milestone', 'milestone:v0.1'],
        blocked: false,
      },
      {
        id: 'm2',
        label: 'Milestone v1.0',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:milestone', 'milestone:v1.0'],
        blocked: false,
      },
      {
        id: 'm3',
        label: 'Milestone v2.0',
        state: 'ready',
        priority: 'normal',
        labels: ['type:milestone', 'milestone:v2.0'],
        blocked: false,
      },
    ];

    const epic: GraphNode = {
      id: 'e1',
      label: 'Epic Auth',
      state: 'in_progress',
      priority: 'high',
      labels: ['type:epic', 'milestone:v1.0', 'epic:auth'],
      blocked: false,
    };

    const story: GraphNode = {
      id: 's1',
      label: 'Story Login',
      state: 'ready',
      priority: 'normal',
      labels: ['type:story', 'milestone:v1.0', 'epic:auth', 'story:login'],
      blocked: false,
    };

    const task: GraphNode = {
      id: 't1',
      label: 'Task Implement JWT',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task', 'milestone:v1.0', 'epic:auth', 'story:login'],
      blocked: false,
    };

    const unassigned: GraphNode = {
      id: 'u1',
      label: 'Unassigned task',
      state: 'ready',
      priority: 'low',
      labels: ['type:task'],
      blocked: false,
    };

    const allNodes = [...milestones, epic, story, task, unassigned];

    it('should assign index 0 to first milestone', () => {
      expect(derivePrimaryTierIndex(milestones[0], allNodes, config)).toBe(0);
    });

    it('should assign index 1 to second milestone', () => {
      expect(derivePrimaryTierIndex(milestones[1], allNodes, config)).toBe(1);
    });

    it('should inherit milestone index for epic', () => {
      expect(derivePrimaryTierIndex(epic, allNodes, config)).toBe(1);
    });

    it('should inherit milestone index for story', () => {
      expect(derivePrimaryTierIndex(story, allNodes, config)).toBe(1);
    });

    it('should inherit milestone index for task', () => {
      expect(derivePrimaryTierIndex(task, allNodes, config)).toBe(1);
    });

    it('should return -1 for unassigned node', () => {
      expect(derivePrimaryTierIndex(unassigned, allNodes, config)).toBe(-1);
    });
  });

  describe('derivePrimaryTierIndex - agile hierarchy (release → epic)', () => {
    const config = createHierarchyConfig(['release', 'epic']);

    const releases: GraphNode[] = [
      {
        id: 'r1',
        label: 'Release 2024-Q1',
        state: 'done',
        priority: 'high',
        labels: ['type:release', 'release:2024-Q1'],
        blocked: false,
      },
      {
        id: 'r2',
        label: 'Release 2024-Q2',
        state: 'in_progress',
        priority: 'high',
        labels: ['type:release', 'release:2024-Q2'],
        blocked: false,
      },
    ];

    const epic: GraphNode = {
      id: 'e1',
      label: 'Epic Mobile',
      state: 'in_progress',
      priority: 'high',
      labels: ['type:epic', 'release:2024-Q2', 'epic:mobile'],
      blocked: false,
    };

    const allNodes = [...releases, epic];

    it('should use release as primary tier', () => {
      expect(derivePrimaryTierIndex(releases[0], allNodes, config)).toBe(0);
      expect(derivePrimaryTierIndex(releases[1], allNodes, config)).toBe(1);
    });

    it('should inherit release index for epic', () => {
      expect(derivePrimaryTierIndex(epic, allNodes, config)).toBe(1);
    });
  });

  describe('derivePrimaryTierIndex - minimal hierarchy (milestone → task only)', () => {
    const config = createHierarchyConfig(['milestone']);

    const milestone: GraphNode = {
      id: 'm1',
      label: 'Milestone v1.0',
      state: 'ready',
      priority: 'high',
      labels: ['type:milestone', 'milestone:v1.0'],
      blocked: false,
    };

    const task: GraphNode = {
      id: 't1',
      label: 'Task Fix bug',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task', 'milestone:v1.0'],
      blocked: false,
    };

    const allNodes = [milestone, task];

    it('should work with single-tier hierarchy', () => {
      expect(derivePrimaryTierIndex(milestone, allNodes, config)).toBe(0);
      expect(derivePrimaryTierIndex(task, allNodes, config)).toBe(0);
    });
  });

  describe('groupNodesByPrimaryTier', () => {
    const config = createHierarchyConfig(['milestone', 'epic']);

    const nodes: GraphNode[] = [
      {
        id: 'm1',
        label: 'Milestone v1.0',
        state: 'ready',
        priority: 'high',
        labels: ['type:milestone', 'milestone:v1.0'],
        blocked: false,
      },
      {
        id: 'e1',
        label: 'Epic Auth',
        state: 'ready',
        priority: 'high',
        labels: ['type:epic', 'milestone:v1.0'],
        blocked: false,
      },
      {
        id: 't1',
        label: 'Task Login',
        state: 'ready',
        priority: 'normal',
        labels: ['type:task', 'milestone:v1.0'],
        blocked: false,
      },
      {
        id: 'm2',
        label: 'Milestone v2.0',
        state: 'backlog',
        priority: 'normal',
        labels: ['type:milestone', 'milestone:v2.0'],
        blocked: false,
      },
      {
        id: 't2',
        label: 'Task Feature',
        state: 'backlog',
        priority: 'normal',
        labels: ['type:task', 'milestone:v2.0'],
        blocked: false,
      },
      {
        id: 'u1',
        label: 'Unassigned',
        state: 'backlog',
        priority: 'low',
        labels: ['type:task'],
        blocked: false,
      },
    ];

    it('should group nodes by primary tier index', () => {
      const grouped = groupNodesByPrimaryTier(nodes, config);

      expect(grouped.size).toBe(3); // index 0, 1, -1
      expect(grouped.get(0)).toHaveLength(3); // m1, e1, t1
      expect(grouped.get(1)).toHaveLength(2); // m2, t2
      expect(grouped.get(-1)).toHaveLength(1); // u1
    });

    it('should maintain node order within groups', () => {
      const grouped = groupNodesByPrimaryTier(nodes, config);
      const tier0 = grouped.get(0)!;

      expect(tier0[0].id).toBe('m1');
      expect(tier0[1].id).toBe('e1');
      expect(tier0[2].id).toBe('t1');
    });
  });
});
