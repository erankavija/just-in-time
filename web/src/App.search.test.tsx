import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import type { GraphNode, SearchResult } from './types/models';

const mocks = vi.hoisted(() => ({
  useSearch: vi.fn(),
  listIssues: vi.fn(),
  onChanged: (version: number): void => { throw new Error(`No SSE handler for ${version}`); },
  onGraphNodes: (nodes: GraphNode[]): void => { throw new Error(`No graph handler for ${nodes.length} nodes`); },
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
    listIssues: mocks.listIssues,
  },
}));
vi.mock('react-split', () => ({ default: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
vi.mock('./components/Graph/GraphView', () => ({
  GraphView: ({ onGraphNodes }: { onGraphNodes: (nodes: GraphNode[]) => void }) => {
    mocks.onGraphNodes = onGraphNodes;
    return <div />;
  },
}));
vi.mock('./components/Issue/IssueDetail', () => ({
  IssueDetail: ({ issueId, issueTitles }: { issueId: string | null; issueTitles?: ReadonlyMap<string, string> }) => (
    <div data-testid="issue-detail">
      {issueId}<span data-testid="dependency-title">{issueTitles?.get('dependency')}</span>
    </div>
  ),
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

function graphNode(id: string, label: string, labels: string[]): GraphNode {
  return {
    id, short_id: id, label, labels, state: 'ready', priority: 'normal', blocked: false,
    type: null, parent: null, children: [], cluster: null, rank: 0,
  };
}

describe('App search', () => {
  beforeEach(() => {
    vi.stubGlobal('__JIT_PRODUCT_VERSION__', 'test');
    mocks.useSearch.mockReset();
    mocks.listIssues.mockReset().mockImplementation(() => new Promise(() => {}));
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

  it('test_app_keeps_full_graph_labels_and_dependency_titles_under_a_filter', () => {
    render(<App />);
    act(() => mocks.onGraphNodes([
      graphNode('visible', 'Visible issue', ['team:visible']),
      graphNode('dependency', 'Hidden dependency', ['team:hidden']),
    ]));

    const filter = screen.getByPlaceholderText('Type to filter labels (e.g., namespace:value)');
    fireEvent.change(filter, { target: { value: 'team:visible' } });
    fireEvent.keyDown(filter, { key: 'Enter' });
    fireEvent.focus(filter);

    expect(screen.getByText('hidden')).toBeInTheDocument();
    expect(screen.getByTestId('dependency-title')).toHaveTextContent('Hidden dependency');
  });

  it('test_app_refreshes_graph_metadata_without_loading_issues', () => {
    render(<App />);
    act(() => mocks.onGraphNodes([graphNode('dependency', 'Old title', ['team:old'])]));
    const filter = screen.getByPlaceholderText('Type to filter labels (e.g., namespace:value)');
    fireEvent.focus(filter);
    expect(screen.getByText('old')).toBeInTheDocument();

    act(() => mocks.onChanged(7));
    act(() => mocks.onGraphNodes([graphNode('dependency', 'New title', ['team:new'])]));
    expect(screen.getByText('new')).toBeInTheDocument();
    expect(screen.queryByText('old')).not.toBeInTheDocument();
    expect(screen.getByTestId('dependency-title')).toHaveTextContent('New title');
    expect(mocks.listIssues).not.toHaveBeenCalled();
  });
});
