/**
 * DocumentViewer smoke tests: verifies that the renderer-registry dispatch
 * works end-to-end by rendering through DocumentViewer with a mocked API client.
 */
import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { DocumentViewer } from './DocumentViewer';
import type { DocumentContent } from '../../types/models';

// Mock the apiClient so DocumentViewer can load content without a network
vi.mock('../../api/client', () => {
  const makeDocumentContent = (content: string, content_type = 'text/markdown'): DocumentContent => ({
    path: 'README.md',
    commit: 'abc1234def5678',
    content,
    content_type,
  });

  return {
    apiClient: {
      getDocumentContent: vi.fn(() => Promise.resolve(makeDocumentContent('# Heading\n\n`inline code`\n\n```js\nconsole.log("hi");\n```'))),
      getDocumentByPath: vi.fn(() => Promise.resolve(makeDocumentContent('# Heading\n\n`inline code`\n\n```js\nconsole.log("hi");\n```'))),
      getDocumentHistory: vi.fn(() => Promise.resolve({ path: 'README.md', commits: [] })),
    },
  };
});

// Mock MermaidDiagram to avoid mermaid init in jsdom
vi.mock('../MermaidDiagram', () => ({
  MermaidDiagram: ({ code }: { code: string }) => (
    <div data-testid="mermaid-diagram" data-code={code} />
  ),
}));

describe('DocumentViewer — renderer dispatch smoke test', () => {
  it('renders an h1 heading from markdown content via MarkdownRenderer', async () => {
    render(
      <DocumentViewer
        documentPath="README.md"
        issueId="test-issue-id"
      />
    );
    await waitFor(() => {
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Heading');
    });
  });

  it('renders a code block from fenced markdown via MarkdownRenderer', async () => {
    render(
      <DocumentViewer
        documentPath="README.md"
        issueId="test-issue-id"
      />
    );
    await waitFor(() => {
      // SyntaxHighlighter or plain code elements will appear
      const codeEl = document.querySelector('code, div[class*="language-"], pre');
      expect(codeEl).toBeTruthy();
    });
  });

  it('shows the document footer with commit hash', async () => {
    render(
      <DocumentViewer
        documentPath="README.md"
        issueId="test-issue-id"
      />
    );
    await waitFor(() => {
      expect(screen.getByText(/Commit:/)).toBeDefined();
    });
  });
});
