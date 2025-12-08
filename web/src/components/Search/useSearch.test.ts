import { describe, it, expect } from 'vitest';
import { filterIssues } from '../../utils/clientSearch';
import type { Issue } from '../../types/models';

// Note: Hook tests require complex setup with React Testing Library
// For now, we test the underlying logic directly

const createMockIssue = (overrides: Partial<Issue>): Issue => ({
  id: 'mock-id',
  title: 'Mock Issue',
  description: 'Mock description',
  state: 'backlog',
  priority: 'normal',
  dependencies: [],
  labels: [],
  documents: [],
  gates: [],
  gates_status: [],
  created_at: '2024-01-01T00:00:00Z',
  updated_at: '2024-01-01T00:00:00Z',
  ...overrides,
});

describe('useSearch', () => {
  const mockIssues = [
    createMockIssue({ id: '1', title: 'Authentication bug' }),
    createMockIssue({ id: '2', title: 'UI polish' }),
    createMockIssue({ id: '3', title: 'Fix auth system' }),
  ];

  // Test the underlying search logic since hook testing has setup complexity
  it('should handle empty query', () => {
    const results = filterIssues(mockIssues, '');
    expect(results).toEqual([]);
  });

  it('should find issues client-side', () => {
    const results = filterIssues(mockIssues, 'auth');
    // Should find issues 1 and 3
    expect(results.length).toBe(2);
  });

  it('should find all matching terms', () => {
    const results = filterIssues(mockIssues, 'auth bug');
    // Only issue 1 has both "auth" and "bug"
    expect(results).toHaveLength(1);
    expect(results[0].issue.id).toBe('1');
  });

  it('should not find non-matching queries', () => {
    const results = filterIssues(mockIssues, 'nonexistent');
    expect(results).toEqual([]);
  });

  it('should be case insensitive', () => {
    const results = filterIssues(mockIssues, 'AUTH');
    expect(results.length).toBeGreaterThan(0);
  });
});

// TODO: Add full hook integration tests when React Testing Library setup is fixed
// These would test:
// - Debouncing behavior
// - Server API calls
// - Error handling
// - Result merging
