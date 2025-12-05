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

  describe('getDocumentDiff', () => {
    it('should fetch document diff with from and to commits', async () => {
      const mockResponse = {
        data: {
          path: 'src/main.rs',
          from: 'abc123',
          to: 'def456',
          diff: '--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n+// New comment\n fn main() {',
        },
      };

      mockGet.mockResolvedValue(mockResponse);

      const { apiClient } = await import('./client');
      const result = await apiClient.getDocumentDiff('issue-789', 'src/main.rs', 'abc123', 'def456');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-789/documents/src%2Fmain.rs/diff?from=abc123&to=def456');
      expect(result.from).toBe('abc123');
      expect(result.to).toBe('def456');
      expect(result.diff).toContain('--- a/src/main.rs');
    });

    it('should fetch document diff with only from commit (defaults to HEAD)', async () => {
      const mockResponse = {
        data: {
          path: 'README.md',
          from: 'abc123',
          to: 'HEAD',
          diff: '--- a/README.md\n+++ b/README.md\n@@ -1 +1,2 @@\n # Title\n+New line',
        },
      };

      mockGet.mockResolvedValue(mockResponse);

      const { apiClient } = await import('./client');
      const result = await apiClient.getDocumentDiff('issue-101', 'README.md', 'abc123');

      expect(mockGet).toHaveBeenCalledWith('/issues/issue-101/documents/README.md/diff?from=abc123');
      expect(result.to).toBe('HEAD');
    });

    it('should handle errors gracefully', async () => {
      mockGet.mockRejectedValue(new Error('Network error'));

      const { apiClient } = await import('./client');
      await expect(apiClient.getDocumentDiff('issue-999', 'missing.md', 'abc')).rejects.toThrow('Network error');
    });
  });
});
