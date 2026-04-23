/**
 * DocumentViewer smoke tests: verifies that the renderer-registry dispatch
 * works end-to-end by rendering through DocumentViewer with a mocked API client.
 * Covers heading, fenced code block, and Mermaid diagram parity — all routed
 * through DocumentViewer (not MarkdownRenderer directly).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import type { DocumentContent } from '../../types/models';

// Mock MermaidDiagram before importing DocumentViewer so the module graph
// uses our stub, avoiding mermaid init in jsdom.
vi.mock('../MermaidDiagram', () => ({
  MermaidDiagram: ({ code }: { code: string }) => (
    <div data-testid="mermaid-diagram" data-code={code} />
  ),
}));

// apiClient mock — configured per-test via mockResolvedValue
const mockGetDocumentContent = vi.fn();
const mockGetDocumentByPath = vi.fn();
const mockGetDocumentHistory = vi.fn();

vi.mock('../../api/client', () => ({
  apiClient: {
    get getDocumentContent() { return mockGetDocumentContent; },
    get getDocumentByPath() { return mockGetDocumentByPath; },
    get getDocumentHistory() { return mockGetDocumentHistory; },
  },
}));

function makeContent(content: string, content_type = 'text/markdown'): DocumentContent {
  return { path: 'README.md', commit: 'abc1234def5678', content, content_type };
}

// Import after mocks are registered
const { DocumentViewer } = await import('./DocumentViewer');

describe('DocumentViewer — renderer dispatch smoke test', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetDocumentHistory.mockResolvedValue({ path: 'README.md', commits: [] });
  });

  it('renders an h1 heading from markdown content via MarkdownRenderer', async () => {
    mockGetDocumentContent.mockResolvedValue(makeContent('# Heading One'));
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Heading One');
    });
  });

  it('renders a fenced code block from markdown via MarkdownRenderer', async () => {
    mockGetDocumentContent.mockResolvedValue(makeContent('```js\nconsole.log("hi");\n```'));
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      // SyntaxHighlighter or plain code elements will appear in the DOM
      const codeEl = document.querySelector('code, div[class*="language-"], pre');
      expect(codeEl).toBeTruthy();
    });
  });

  it('renders a Mermaid diagram from markdown via MarkdownRenderer', async () => {
    mockGetDocumentContent.mockResolvedValue(makeContent('```mermaid\ngraph TD; A-->B;\n```'));
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      const mermaid = screen.getByTestId('mermaid-diagram');
      expect(mermaid).toBeDefined();
      expect(mermaid.getAttribute('data-code')).toContain('graph TD');
    });
  });

  it('shows the document footer with commit hash after content loads', async () => {
    mockGetDocumentContent.mockResolvedValue(makeContent('# Doc'));
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      expect(screen.getByText(/Commit:/)).toBeDefined();
    });
  });
});
