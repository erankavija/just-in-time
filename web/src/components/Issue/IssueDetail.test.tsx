import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import { apiClient } from '../../api/client';
import type { Issue } from '../../types/models';
import { IssueDetail } from './IssueDetail';

vi.mock('../../api/client', () => ({
  apiClient: {
    getIssue: vi.fn(),
    listGates: vi.fn(),
  },
}));

const getIssue = vi.mocked(apiClient.getIssue);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

function issue(id: string, title: string): Issue {
  return {
    id,
    short_id: id,
    title,
    description: 'Issue details',
    state: 'ready',
    priority: 'normal',
    dependencies: [],
    labels: [],
    documents: [],
    gates_required: [],
    gates_status: {},
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  };
}

describe('IssueDetail request ordering', () => {
  beforeEach(() => getIssue.mockReset());
  afterEach(() => vi.restoreAllMocks());

  it('test_issue_detail_displays_dependency_title_from_full_graph_summary', async () => {
    const selected = issue('parent', 'Parent issue');
    selected.dependencies = [{ id: 'dependency', short_id: 'dep' }];
    getIssue.mockResolvedValue(selected);

    render(<IssueDetail
      issueId="parent"
      issueTitles={new Map([['dependency', 'Hidden dependency']])}
    />);

    expect(await screen.findByText('Hidden dependency')).toBeInTheDocument();
  });

  it('test_issue_detail_first_selection_displays_details_and_clears_loading', async () => {
    const first = deferred<Issue>();
    getIssue.mockReturnValueOnce(first.promise);
    render(<IssueDetail issueId="first" />);

    expect(screen.getByText('$ loading...')).toBeInTheDocument();
    await act(async () => first.resolve(issue('first', 'First issue')));

    expect(screen.getByRole('heading', { name: 'First issue' })).toBeInTheDocument();
    expect(screen.queryByText('$ loading...')).not.toBeInTheDocument();
  });

  it('test_issue_detail_first_selection_displays_error_and_clears_loading', async () => {
    const first = deferred<Issue>();
    getIssue.mockReturnValueOnce(first.promise);
    vi.spyOn(console, 'error').mockImplementation(() => {});
    render(<IssueDetail issueId="first" />);

    expect(screen.getByText('$ loading...')).toBeInTheDocument();
    await act(async () => first.reject(new Error('Cannot load first')));

    expect(screen.getByText('$ error: Cannot load first')).toBeInTheDocument();
    expect(screen.queryByText('$ loading...')).not.toBeInTheDocument();
  });

  it('test_issue_detail_earlier_selection_cannot_replace_later_selection', async () => {
    const first = deferred<Issue>();
    const second = deferred<Issue>();
    getIssue.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const view = render(<IssueDetail issueId="first" />);
    view.rerender(<IssueDetail issueId="second" />);

    await act(async () => second.resolve(issue('second', 'Second issue')));
    expect(screen.getByRole('heading', { name: 'Second issue' })).toBeInTheDocument();

    await act(async () => first.resolve(issue('first', 'Stale first issue')));
    expect(screen.getByRole('heading', { name: 'Second issue' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Stale first issue' })).not.toBeInTheDocument();
  });

  it('test_issue_detail_newer_version_wins_over_older_refresh', async () => {
    const first = deferred<Issue>();
    const oldRefresh = deferred<Issue>();
    const newRefresh = deferred<Issue>();
    getIssue
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(oldRefresh.promise)
      .mockReturnValueOnce(newRefresh.promise);
    const view = render(<IssueDetail issueId="first" version={1} />);
    await act(async () => first.resolve(issue('first', 'Initial issue')));
    expect(screen.getByRole('heading', { name: 'Initial issue' })).toBeInTheDocument();

    view.rerender(<IssueDetail issueId="first" version={2} />);
    expect(screen.queryByText('$ loading...')).not.toBeInTheDocument();
    view.rerender(<IssueDetail issueId="first" version={3} />);
    await act(async () => newRefresh.resolve(issue('first', 'Newest issue')));
    expect(screen.getByRole('heading', { name: 'Newest issue' })).toBeInTheDocument();

    await act(async () => oldRefresh.resolve(issue('first', 'Older issue')));
    expect(screen.getByRole('heading', { name: 'Newest issue' })).toBeInTheDocument();
  });

  it('test_issue_detail_clearing_and_reselecting_rejects_previous_response', async () => {
    const first = deferred<Issue>();
    const selectedAgain = deferred<Issue>();
    getIssue.mockReturnValueOnce(first.promise).mockReturnValueOnce(selectedAgain.promise);
    const view = render(<IssueDetail issueId="first" />);
    view.rerender(<IssueDetail issueId={null} />);
    expect(screen.getByText('$ select an issue from the graph')).toBeInTheDocument();

    view.rerender(<IssueDetail issueId="first" />);
    await act(async () => selectedAgain.resolve(issue('first', 'Current first issue')));
    expect(screen.getByRole('heading', { name: 'Current first issue' })).toBeInTheDocument();

    await act(async () => first.resolve(issue('first', 'Previous first issue')));
    expect(screen.getByRole('heading', { name: 'Current first issue' })).toBeInTheDocument();
  });
});
