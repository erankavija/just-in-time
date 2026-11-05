import { describe, it, expect, vi, beforeEach } from 'vitest';

const mockGet = vi.fn();

// Mock axios at module level
vi.mock('axios', () => ({
  default: {
    create: vi.fn(() => ({
      get: mockGet,
    })),
  },
}));

describe('apiClient - retained issue list', () => {
  it('test_list_issues_uses_supported_issues_route', async () => {
    mockGet.mockResolvedValue({ data: [] });

    const { apiClient } = await import('./client');
    await expect(apiClient.listIssues()).resolves.toEqual([]);
    expect(mockGet).toHaveBeenCalledWith('/issues');
  });
});

describe('apiClient - Document Methods', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('getDocumentContent', () => {
    it('should fetch document content at HEAD', async () => {
      const mockResponse = {
        data: {
          path: 'README.md',
          commit: 'abc123def456',
          content: '# Test Document\n\nContent here',
          content_type: 'text/markdown',
        },
      };

      mockGet.mockResolvedValue(mockResponse);
      
      const { apiClient } = await import('./client');
      const result = await apiClient.getDocumentContent('issue-123', 'README.md');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-123/documents/README.md/content');
      expect(result).toEqual(mockResponse.data);
      expect(result.content_type).toBe('text/markdown');
    });

    it('should fetch document content at specific commit', async () => {
      const mockResponse = {
        data: {
          path: 'docs/design.md',
          commit: 'def456abc789',
          content: '# Design Document\n\nOld version',
          content_type: 'text/markdown',
        },
      };

      mockGet.mockResolvedValue(mockResponse);

      const { apiClient } = await import('./client');
      const result = await apiClient.getDocumentContent('issue-456', 'docs/design.md', 'def456');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-456/documents/docs%2Fdesign.md/content?commit=def456');
      expect(result.commit).toBe('def456abc789');
    });

    it('should handle URL encoding of paths', async () => {
      const mockResponse = {
        data: {
          path: 'docs/file with spaces.md',
          commit: 'abc123',
          content: 'content',
          content_type: 'text/markdown',
        },
      };

      mockGet.mockResolvedValue(mockResponse);

      const { apiClient } = await import('./client');
      await apiClient.getDocumentContent('issue-789', 'docs/file with spaces.md');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-789/documents/docs%2Ffile%20with%20spaces.md/content');
    });
  });

  describe('getDocumentHistory', () => {
    it('should fetch document commit history', async () => {
      const mockResponse = {
        data: {
          path: 'README.md',
          commits: [
            {
              commit: 'abc123',
              author: 'John Doe',
              date: '2024-12-01 10:00:00',
              message: 'Update README',
            },
            {
              commit: 'def456',
              author: 'Jane Smith',
              date: '2024-11-28 15:30:00',
              message: 'Initial README',
            },
          ],
        },
      };

      mockGet.mockResolvedValue(mockResponse);

      const { apiClient } = await import('./client');
      const result = await apiClient.getDocumentHistory('issue-123', 'README.md');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-123/documents/README.md/history');
      expect(result.commits).toHaveLength(2);
      expect(result.commits[0].author).toBe('John Doe');
    });

    it('should handle paths with special characters', async () => {
      const mockResponse = {
        data: {
          path: 'docs/api-v2.0.md',
          commits: [],
        },
      };

      mockGet.mockResolvedValue(mockResponse);

      const { apiClient } = await import('./client');
      await apiClient.getDocumentHistory('issue-456', 'docs/api-v2.0.md');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-456/documents/docs%2Fapi-v2.0.md/history');
    });
  });
});

describe('getRawDocumentUrl', () => {
  it('returns issue-scoped raw URL with encoded path', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl('issue-1', 'docs/presentations/deck.html');
    expect(url).toBe('/api/issues/issue-1/documents/docs%2Fpresentations%2Fdeck.html/raw');
  });

  it('URL-encodes paths with spaces', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl('issue-1', 'docs/my file.html');
    expect(url).toBe('/api/issues/issue-1/documents/docs%2Fmy%20file.html/raw');
  });

  it('appends ?commit= query param when commit is provided', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl('issue-1', 'index.html', 'deadbeef');
    expect(url).toBe('/api/issues/issue-1/documents/index.html/raw?commit=deadbeef');
  });

  it('omits commit param when commit is undefined', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl('issue-1', 'index.html', undefined);
    expect(url).not.toContain('commit');
    expect(url).toBe('/api/issues/issue-1/documents/index.html/raw');
  });

  it('returns path-only variant when issueId is null', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl(null, 'shared/doc.html');
    expect(url).toBe('/api/documents/raw?path=shared%2Fdoc.html');
  });

  it('returns path-only variant when issueId is undefined', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl(undefined, 'shared/doc.html');
    expect(url).toBe('/api/documents/raw?path=shared%2Fdoc.html');
  });

  it('appends commit to path-only variant', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl(null, 'shared/doc.html', 'abc123');
    expect(url).toBe('/api/documents/raw?path=shared%2Fdoc.html&commit=abc123');
  });

  it('strips "working-tree" sentinel commit (issue-scoped)', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl('issue-1', 'index.html', 'working-tree');
    expect(url).not.toContain('commit');
    expect(url).toBe('/api/issues/issue-1/documents/index.html/raw');
  });

  it('strips "working-tree" sentinel commit (path-only)', async () => {
    const { getRawDocumentUrl } = await import('./client');
    const url = getRawDocumentUrl(null, 'index.html', 'working-tree');
    expect(url).not.toContain('commit');
    expect(url).toBe('/api/documents/raw?path=index.html');
  });
});
