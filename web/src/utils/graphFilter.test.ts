import { describe, it, expect } from 'vitest';
import {
  applyFiltersToNode,
  applyFiltersToEdge,
  matchesAnyPattern,
  matchesPattern,
  createStrategicFilter,
  createLabelFilter,
  type GraphFilter,
  type StrategicFilterConfig,
  type LabelFilterConfig,
} from './graphFilter';
import type { GraphNode, GraphEdge } from '../types/models';

// Test fixtures
const createNode = (id: string, labels: string[]): GraphNode => ({
  id,
  label: `Issue ${id}`,
  state: 'ready',
  priority: 'normal',
  labels,
  blocked: false,
});

const createEdge = (from: string, to: string): GraphEdge => ({
  from,
  to,
});

describe('graphFilter', () => {
  describe('matchesPattern', () => {
    it('matches exact label', () => {
      expect(matchesPattern('milestone:v1.0', 'milestone:v1.0')).toBe(true);
      expect(matchesPattern('milestone:v1.0', 'milestone:v2.0')).toBe(false);
    });

    it('matches wildcard patterns', () => {
      expect(matchesPattern('milestone:v1.0', 'milestone:*')).toBe(true);
      expect(matchesPattern('milestone:v2.0', 'milestone:*')).toBe(true);
      expect(matchesPattern('epic:auth', 'milestone:*')).toBe(false);
    });

    it('handles edge cases', () => {
      expect(matchesPattern('', '')).toBe(true);
      expect(matchesPattern('test', '*')).toBe(true);
      expect(matchesPattern('', 'milestone:*')).toBe(false);
    });
  });

  describe('matchesAnyPattern', () => {
    it('returns true when no patterns provided', () => {
      expect(matchesAnyPattern(['milestone:v1.0'], [])).toBe(true);
    });

    it('matches when any label matches any pattern', () => {
      const labels = ['milestone:v1.0', 'epic:auth', 'component:api'];
      expect(matchesAnyPattern(labels, ['milestone:*'])).toBe(true);
      expect(matchesAnyPattern(labels, ['epic:auth'])).toBe(true);
      expect(matchesAnyPattern(labels, ['component:*'])).toBe(true);
    });

    it('returns false when no labels match', () => {
      const labels = ['component:ui', 'type:bug'];
      expect(matchesAnyPattern(labels, ['milestone:*'])).toBe(false);
      expect(matchesAnyPattern(labels, ['epic:*'])).toBe(false);
    });

    it('handles multiple patterns with OR logic', () => {
      const labels = ['epic:auth'];
      expect(matchesAnyPattern(labels, ['milestone:*', 'epic:*'])).toBe(true);
    });
  });

  describe('applyFiltersToNode - no filters', () => {
    it('shows node normally when no filters applied', () => {
      const node = createNode('1', ['component:api']);
      const result = applyFiltersToNode(node, []);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });
  });

  describe('applyFiltersToNode - strategic filter', () => {
    it('hides non-strategic nodes when strategic filter enabled', () => {
      const node = createNode('1', ['component:api']);
      const filters: GraphFilter[] = [createStrategicFilter(true)];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(false);
      expect(result.reason).toContain('strategic');
    });

    it('shows strategic nodes (milestone) when strategic filter enabled', () => {
      const node = createNode('1', ['milestone:v1.0', 'component:api']);
      const filters: GraphFilter[] = [createStrategicFilter(true)];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });

    it('shows strategic nodes (epic) when strategic filter enabled', () => {
      const node = createNode('1', ['epic:auth']);
      const filters: GraphFilter[] = [createStrategicFilter(true)];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });

    it('shows all nodes when strategic filter disabled', () => {
      const node = createNode('1', ['component:api']);
      const filters: GraphFilter[] = [createStrategicFilter(false)];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });
  });

  describe('applyFiltersToNode - label filter', () => {
    it('dims nodes that do not match label patterns', () => {
      const node = createNode('1', ['component:api']);
      const filters: GraphFilter[] = [createLabelFilter(['milestone:*'])];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(true);
      expect(result.reason).toContain('label');
    });

    it('shows matching nodes normally', () => {
      const node = createNode('1', ['milestone:v1.0', 'component:api']);
      const filters: GraphFilter[] = [createLabelFilter(['milestone:*'])];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });

    it('handles exact match patterns', () => {
      const node = createNode('1', ['epic:auth']);
      const filters: GraphFilter[] = [createLabelFilter(['epic:auth'])];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });

    it('shows all nodes when no patterns specified', () => {
      const node = createNode('1', ['component:api']);
      const filters: GraphFilter[] = [createLabelFilter([])];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });

    it('matches any of multiple patterns (OR logic)', () => {
      const node = createNode('1', ['epic:auth']);
      const filters: GraphFilter[] = [createLabelFilter(['milestone:*', 'epic:*'])];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });
  });

  describe('applyFiltersToNode - combined filters', () => {
    it('hides node when strategic filter hides it, even if label matches', () => {
      const node = createNode('1', ['component:api']);
      const filters: GraphFilter[] = [
        createStrategicFilter(true),
        createLabelFilter(['component:*']),
      ];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(false);
      expect(result.reason).toContain('strategic');
    });

    it('shows strategic node normally when both filters match', () => {
      const node = createNode('1', ['milestone:v1.0']);
      const filters: GraphFilter[] = [
        createStrategicFilter(true),
        createLabelFilter(['milestone:*']),
      ];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });

    it('dims strategic node when label filter does not match', () => {
      const node = createNode('1', ['milestone:v1.0']);
      const filters: GraphFilter[] = [
        createStrategicFilter(true),
        createLabelFilter(['epic:*']),
      ];
      const result = applyFiltersToNode(node, filters);
      
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(true);
      expect(result.reason).toContain('label');
    });
  });

  describe('applyFiltersToEdge', () => {
    it('hides edge when source node is hidden', () => {
      const edge = createEdge('1', '2');
      const sourceResult = { visible: false, dimmed: false };
      const targetResult = { visible: true, dimmed: false };
      
      const result = applyFiltersToEdge(edge, sourceResult, targetResult);
      expect(result.visible).toBe(false);
    });

    it('hides edge when target node is hidden', () => {
      const edge = createEdge('1', '2');
      const sourceResult = { visible: true, dimmed: false };
      const targetResult = { visible: false, dimmed: false };
      
      const result = applyFiltersToEdge(edge, sourceResult, targetResult);
      expect(result.visible).toBe(false);
    });

    it('dims edge when source node is dimmed', () => {
      const edge = createEdge('1', '2');
      const sourceResult = { visible: true, dimmed: true };
      const targetResult = { visible: true, dimmed: false };
      
      const result = applyFiltersToEdge(edge, sourceResult, targetResult);
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(true);
    });

    it('dims edge when target node is dimmed', () => {
      const edge = createEdge('1', '2');
      const sourceResult = { visible: true, dimmed: false };
      const targetResult = { visible: true, dimmed: true };
      
      const result = applyFiltersToEdge(edge, sourceResult, targetResult);
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(true);
    });

    it('dims edge when both nodes are dimmed', () => {
      const edge = createEdge('1', '2');
      const sourceResult = { visible: true, dimmed: true };
      const targetResult = { visible: true, dimmed: true };
      
      const result = applyFiltersToEdge(edge, sourceResult, targetResult);
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(true);
    });

    it('shows edge normally when both nodes are normal', () => {
      const edge = createEdge('1', '2');
      const sourceResult = { visible: true, dimmed: false };
      const targetResult = { visible: true, dimmed: false };
      
      const result = applyFiltersToEdge(edge, sourceResult, targetResult);
      expect(result.visible).toBe(true);
      expect(result.dimmed).toBe(false);
    });
  });

  describe('filter factory functions', () => {
    it('creates strategic filter correctly', () => {
      const filter = createStrategicFilter(true);
      expect(filter.type).toBe('strategic');
      expect((filter.config as StrategicFilterConfig).enabled).toBe(true);
    });

    it('creates label filter correctly', () => {
      const filter = createLabelFilter(['milestone:*', 'epic:*']);
      expect(filter.type).toBe('label');
      expect((filter.config as LabelFilterConfig).patterns).toEqual(['milestone:*', 'epic:*']);
    });
  });
});
