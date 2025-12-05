import { describe, it, expect } from 'vitest';
import { filterIssues, calculateScore } from './clientSearch';
import type { Issue } from '../types/models';

const createMockIssue = (overrides: Partial<Issue>): Issue => ({
  id: 'mock-id',
  title: 'Mock Issue',
  description: 'Mock description',
  state: 'backlog',
  priority: 'normal',
  dependencies: [],
  documents: [],
  gates: [],
  gates_status: [],
  created_at: '2024-01-01T00:00:00Z',
  updated_at: '2024-01-01T00:00:00Z',
  ...overrides,
});

describe('clientSearch', () => {
  describe('filterIssues', () => {
    it('should return empty array for empty query', () => {
      const issues = [createMockIssue({})];
      const results = filterIssues(issues, '');
      expect(results).toEqual([]);
    });

    it('should find issues by title', () => {
      const issues = [
        createMockIssue({ id: '1', title: 'Authentication bug' }),
        createMockIssue({ id: '2', title: 'UI polish' }),
      ];
      const results = filterIssues(issues, 'authentication');
      expect(results).toHaveLength(1);
      expect(results[0].issue.id).toBe('1');
    });

    it('should find issues by description', () => {
      const issues = [
        createMockIssue({ id: '1', description: 'Fix login authentication' }),
        createMockIssue({ id: '2', description: 'Update UI' }),
      ];
      const results = filterIssues(issues, 'login');
      expect(results).toHaveLength(1);
      expect(results[0].issue.id).toBe('1');
    });

    it('should find issues by ID prefix', () => {
      const issues = [
        createMockIssue({ id: 'abc123' }),
        createMockIssue({ id: 'def456' }),
      ];
      const results = filterIssues(issues, 'abc');
      expect(results).toHaveLength(1);
      expect(results[0].issue.id).toBe('abc123');
    });

    it('should be case insensitive', () => {
      const issues = [createMockIssue({ title: 'Authentication' })];
      const results = filterIssues(issues, 'AUTH');
      expect(results).toHaveLength(1);
    });

    it('should handle multiple search terms', () => {
      const issues = [
        createMockIssue({ title: 'Fix auth bug' }),
        createMockIssue({ title: 'Auth system' }),
        createMockIssue({ title: 'Bug tracker' }),
      ];
      const results = filterIssues(issues, 'auth bug');
      expect(results).toHaveLength(1);
      expect(results[0].issue.title).toBe('Fix auth bug');
    });

    it('should sort by score descending', () => {
      const issues = [
        createMockIssue({ id: '1', title: 'Other', description: 'test content' }),
        createMockIssue({ id: '2', title: 'test title' }),
        createMockIssue({ id: 'test123' }),
      ];
      const results = filterIssues(issues, 'test');
      // ID match should score highest
      expect(results[0].issue.id).toBe('test123');
    });
  });

  describe('calculateScore', () => {
    it('should score ID matches highest', () => {
      const issue = createMockIssue({ id: 'auth123' });
      const score = calculateScore(issue, ['auth']);
      expect(score).toBeGreaterThan(10);
    });

    it('should score title matches higher than description', () => {
      const issueTitle = createMockIssue({ title: 'auth fix', description: 'other' });
      const issueDesc = createMockIssue({ title: 'other', description: 'auth fix' });
      
      const scoreTitle = calculateScore(issueTitle, ['auth']);
      const scoreDesc = calculateScore(issueDesc, ['auth']);
      
      expect(scoreTitle).toBeGreaterThan(scoreDesc);
    });

    it('should return 0 for no matches', () => {
      const issue = createMockIssue({ title: 'foo', description: 'bar' });
      const score = calculateScore(issue, ['test']);
      expect(score).toBe(0);
    });

    it('should accumulate score for multiple term matches', () => {
      const issue = createMockIssue({ title: 'auth bug fix' });
      const scoreOne = calculateScore(issue, ['auth']);
      const scoreTwo = calculateScore(issue, ['auth', 'bug']);
      expect(scoreTwo).toBeGreaterThan(scoreOne);
    });
  });
});
