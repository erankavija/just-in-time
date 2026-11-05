import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { SearchResult } from './types/models';

const mocks = vi.hoisted(() => ({
  useSearch: vi.fn(),
  onChanged: (version: number): void => { throw new Error(`No SSE handler for ${version}`); },
}));

vi.mock('./components/Search/useSearch', () => ({ useSearch: mocks.useSearch }));
vi.mock('./hooks/useEventStream', () => ({
  useEventStream: ({ onChanged }: { onChanged: (version: number) => void }) => {
    mocks.onChanged = onChanged;
    return { connected: true };
  },
}));
vi.mock('./hooks/useTheme', () => ({ useTheme: () => ({ theme: 'dark', toggleTheme: vi.fn() }) }));
vi.mock('./api/client', () => ({
  apiClient: {
    getHealth: vi.fn().mockImplementation(() => new Promise(() => {})),
    getNamespaces: vi.fn().mockImplementation(() => new Promise(() => {})),
    listIssues: vi.fn().mockImplementation(() => new Promise(() => {})),
  },
}));
vi.mock('react-split', () => ({ default: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
vi.mock('./components/Graph/GraphView', () => ({ GraphView: () => <div /> }));
vi.mock('./components/Issue/IssueDetail', () => ({
  IssueDetail: ({ issueId }: { issueId: string | null }) => <div data-testid="issue-detail">{issueId}</div>,
}));
vi.mock('./components/Document/DocumentViewer', () => ({
  DocumentViewer: ({ documentPath }: { documentPath: string }) => <div data-testid="document-viewer">{documentPath}</div>,
}));

const issueMatch: SearchResult = {
  issue_id: 'issue-1', path: 'issues/issue-1.json', line_number: 0,
  line_text: 'Issue title', matches: [],
};
const documentMatch: SearchResult = {
  path: 'docs/plan.md', line_number: 4, line_text: 'matching document text', matches: [],
};

describe('App search', () => {
  beforeEach(() => {
    vi.stubGlobal('__JIT_PRODUCT_VERSION__', 'test');
    mocks.useSearch.mockReset();
    mocks.useSearch.mockImplementation((query: string) => ({
      results: query ? [issueMatch, documentMatch] : [], loading: false, error: null,
    }));
  });

  afterEach(() => vi.unstubAllGlobals());

  it('test_app_routes_server_issue_and_document_matches_to_their_views', () => {
    render(<App />);
    fireEvent.change(screen.getByRole('textbox', { name: 'Search' }), { target: { value: 'match' } });

    fireEvent.click(screen.getByText('Issue title'));
    expect(screen.getByTestId('issue-detail')).toHaveTextContent('issue-1');

    fireEvent.change(screen.getByRole('textbox', { name: 'Search' }), { target: { value: 'match' } });
    fireEvent.click(screen.getByText(/docs\/plan\.md/));
    expect(screen.getByTestId('document-viewer')).toHaveTextContent('docs/plan.md');
  });

  it('test_app_passes_sse_version_to_search_hook', () => {
    render(<App />);
    expect(mocks.useSearch).toHaveBeenCalledWith('', 0);

    act(() => mocks.onChanged(7));
    expect(mocks.useSearch).toHaveBeenLastCalledWith('', 7);
  });
});
