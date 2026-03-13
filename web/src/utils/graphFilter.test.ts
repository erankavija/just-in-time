import { describe, it, expect } from 'vitest';
import {
  applyFiltersToNode,
  applyFiltersToEdge,
  matchesAnyPattern,
  matchesPattern,
  createLabelFilter,
  type GraphFilter,
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
    it('creates label filter correctly', () => {
      const filter = createLabelFilter(['milestone:*', 'epic:*']);
      expect(filter.type).toBe('label');
      expect((filter.config as LabelFilterConfig).patterns).toEqual(['milestone:*', 'epic:*']);
    });
  });
});
