/**
 * DocumentViewer smoke tests: verifies that the renderer-registry dispatch
 * works end-to-end by rendering a markdown-linked document through DocumentViewer
 * (not MarkdownRenderer directly) — covers heading, code block, and Mermaid parity.
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

// A single fixture that represents an existing linked markdown document with
// heading, fenced code block, and Mermaid diagram — the three parity checks
// required by issue 42bd50ce acceptance criteria.
const LINKED_DOC_FIXTURE = [
  '# Project Overview',
  '',
  'Here is a code snippet:',
  '',
  '```js',
  'console.log("hello world");',
  '```',
  '',
  'And a diagram:',
  '',
  '```mermaid',
  'graph TD; A-->B;',
  '```',
].join('\n');

// Import after mocks are registered
const { DocumentViewer } = await import('./DocumentViewer');

describe('DocumentViewer — renderer dispatch smoke test (fixture-backed)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetDocumentHistory.mockResolvedValue({ path: 'README.md', commits: [] });
    // Default: both fetch methods return the same fixture
    mockGetDocumentContent.mockResolvedValue(makeContent(LINKED_DOC_FIXTURE));
    mockGetDocumentByPath.mockResolvedValue(makeContent(LINKED_DOC_FIXTURE));
  });

  it('renders an h1 heading from the linked markdown document', async () => {
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Project Overview');
    });
  });

  it('renders a fenced code block from the linked markdown document', async () => {
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      // SyntaxHighlighter or plain code/pre elements will appear in the DOM
      const codeEl = document.querySelector('code, div[class*="language-"], pre');
      expect(codeEl).toBeTruthy();
    });
  });

  it('renders a Mermaid diagram from the linked markdown document', async () => {
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      const mermaid = screen.getByTestId('mermaid-diagram');
      expect(mermaid).toBeDefined();
      expect(mermaid.getAttribute('data-code')).toContain('graph TD');
    });
  });

  it('shows the document footer with commit hash after content loads', async () => {
    render(<DocumentViewer documentPath="README.md" issueId="test-issue-id" />);
    await waitFor(() => {
      expect(screen.getByText(/Commit:/)).toBeDefined();
    });
  });
});
