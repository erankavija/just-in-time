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
  getRawDocumentUrl: (issueId: string | null | undefined, path: string, commit?: string) => {
    if (issueId) {
      const base = `/api/issues/${issueId}/documents/${encodeURIComponent(path)}/raw`;
      return commit ? `${base}?commit=${encodeURIComponent(commit)}` : base;
    }
    const params = new URLSearchParams({ path });
    if (commit) params.set('commit', commit);
    return `/api/documents/raw?${params.toString()}`;
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

// A minimal reveal.js-style HTML fixture (stripped to the structural essentials).
// reveal.js presentations are served with content_type: text/html — we verify
// that DocumentViewer routes them to HtmlRenderer (iframe) rather than markdown.
const REVEAL_JS_FIXTURE = [
  '<!doctype html>',
  '<html>',
  '<head><title>Modem Framework</title></head>',
  '<body class="reveal-viewport">',
  '<div class="reveal"><div class="slides">',
  '<section><h1>GF2 Modem Framework</h1></section>',
  '<section><h2>Architecture</h2></section>',
  '</div></div>',
  '<script src="dist/reveal.js"></script>',
  '<script>Reveal.initialize();</script>',
  '</body></html>',
].join('\n');

describe('DocumentViewer — HTML renderer suppresses history panel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetDocumentHistory.mockResolvedValue({ path: 'deck.html', commits: [] });
    mockGetDocumentContent.mockResolvedValue(
      makeContent(REVEAL_JS_FIXTURE, 'text/html'),
    );
    mockGetDocumentByPath.mockResolvedValue(
      makeContent(REVEAL_JS_FIXTURE, 'text/html'),
    );
  });

  it('does not render a History button for text/html documents', async () => {
    render(
      <DocumentViewer
        documentPath="deck.html"
        issueId="html-issue"
        documentRef={{ path: 'deck.html', label: 'My Deck' }}
      />,
    );
    await waitFor(() => {
      // The iframe (rendered by HtmlRenderer) should be present
      expect(document.querySelector('iframe')).toBeTruthy();
    });
    // No history button should be in the DOM
    const historyBtn = document.querySelector('.history-btn');
    expect(historyBtn).toBeNull();
  });
});

describe('DocumentViewer — reveal.js smoke test (fixture-backed)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetDocumentContent.mockResolvedValue(
      makeContent(REVEAL_JS_FIXTURE, 'text/html'),
    );
    mockGetDocumentByPath.mockResolvedValue(
      makeContent(REVEAL_JS_FIXTURE, 'text/html'),
    );
  });

  it('routes reveal.js deck (text/html) to HtmlRenderer — renders an iframe', async () => {
    render(
      <DocumentViewer
        documentPath="docs/presentations/deck.html"
        issueId="d4851c3d"
        documentRef={{
          path: 'docs/presentations/deck.html',
          label: 'GF2 Modem Framework',
        }}
      />,
    );
    await waitFor(() => {
      const iframe = document.querySelector('iframe') as HTMLIFrameElement | null;
      expect(iframe).toBeTruthy();
      // iframe src must point at the raw endpoint, not attempt to render HTML text
      expect(iframe?.getAttribute('src')).toContain('/raw');
    });
  });

  it('reveal.js iframe has full sandbox permissions required for presentation playback', async () => {
    render(
      <DocumentViewer
        documentPath="docs/presentations/deck.html"
        issueId="d4851c3d"
        documentRef={{ path: 'docs/presentations/deck.html', label: 'Modem Framework' }}
      />,
    );
    await waitFor(() => {
      const iframe = document.querySelector('iframe') as HTMLIFrameElement | null;
      expect(iframe).toBeTruthy();
      const sandbox = iframe?.getAttribute('sandbox') ?? '';
      // allow-scripts enables reveal.js initialisation
      expect(sandbox).toContain('allow-scripts');
      // allow-same-origin lets the deck fetch co-located CSS/JS assets
      expect(sandbox).toContain('allow-same-origin');
      // allow-popups lets in-deck links open
      expect(sandbox).toContain('allow-popups');
    });
  });

  it('does NOT render any reveal.js content as HTML text (no slide class in DOM)', async () => {
    render(
      <DocumentViewer
        documentPath="docs/presentations/deck.html"
        issueId="d4851c3d"
        documentRef={{ path: 'docs/presentations/deck.html', label: 'Modem Framework' }}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('iframe')).toBeTruthy();
    });
    // The raw HTML content should NOT be injected into the host document
    expect(document.querySelector('.reveal')).toBeNull();
    expect(document.querySelector('.slides')).toBeNull();
  });
});
