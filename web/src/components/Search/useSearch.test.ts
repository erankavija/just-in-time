import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { apiClient } from '../../api/client';
import type { SearchResponse, SearchResult } from '../../types/models';
import { useSearch } from './useSearch';

vi.mock('../../api/client', () => ({ apiClient: { search: vi.fn() } }));

const search = vi.mocked(apiClient.search);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function response(query: string, results: SearchResult[]): SearchResponse {
  return { query, results, total: results.length, duration_ms: 1 };
}

const issueMatch: SearchResult = {
  issue_id: 'issue-1', path: 'issues/issue-1.json', line_number: 0,
  line_text: 'Issue title', matches: [],
};
const documentMatch: SearchResult = {
  path: 'docs/plan.md', line_number: 3, line_text: 'matching document text', matches: [],
};

describe('useSearch', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    search.mockReset();
  });

  afterEach(() => vi.useRealTimers());

  it('test_use_search_ignores_out_of_order_query_responses', async () => {
    const first = deferred<SearchResponse>();
    const second = deferred<SearchResponse>();
    search.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { result, rerender } = renderHook(
      ({ query, version }) => useSearch(query, version),
      { initialProps: { query: 'first', version: 0 } },
    );

    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    rerender({ query: 'second', version: 0 });
    expect(result.current.results).toEqual([]);
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    await act(async () => { second.resolve(response('second', [issueMatch, documentMatch])); });
    expect(result.current.results).toEqual([issueMatch, documentMatch]);

    await act(async () => { first.resolve(response('first', [documentMatch])); });
    expect(result.current.results).toEqual([issueMatch, documentMatch]);
    expect(search).toHaveBeenNthCalledWith(2, 'second', { limit: 50 });
  });

  it('test_use_search_invalidates_in_flight_results_when_version_changes', async () => {
    const oldVersion = deferred<SearchResponse>();
    const newVersion = deferred<SearchResponse>();
    search.mockReturnValueOnce(oldVersion.promise).mockReturnValueOnce(newVersion.promise);
    const { result, rerender } = renderHook(
      ({ version }) => useSearch('issue', version),
      { initialProps: { version: 1 } },
    );

    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    rerender({ version: 2 });
    expect(result.current.results).toEqual([]);
    await act(async () => { oldVersion.resolve(response('issue', [issueMatch])); });
    expect(result.current.results).toEqual([]);
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    await act(async () => { newVersion.resolve(response('issue', [documentMatch])); });
    expect(result.current.results).toEqual([documentMatch]);
    expect(search).toHaveBeenCalledTimes(2);
  });

  it('test_use_search_does_not_restore_results_when_query_returns_to_previous_text', async () => {
    const refreshed = deferred<SearchResponse>();
    search
      .mockResolvedValueOnce(response('alpha', [issueMatch]))
      .mockReturnValueOnce(refreshed.promise);
    const { result, rerender } = renderHook(
      ({ query }) => useSearch(query, 0),
      { initialProps: { query: 'alpha' } },
    );

    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    expect(result.current.results).toEqual([issueMatch]);
    rerender({ query: 'beta' });
    rerender({ query: 'alpha' });
    expect(result.current.results).toEqual([]);
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    await act(async () => { refreshed.resolve(response('alpha', [documentMatch])); });
    expect(result.current.results).toEqual([documentMatch]);
  });

  it('test_use_search_clearing_query_discards_pending_errors', async () => {
    const pending = deferred<SearchResponse>();
    search.mockReturnValueOnce(pending.promise);
    const { result, rerender } = renderHook(
      ({ query }) => useSearch(query, 0),
      { initialProps: { query: 'broken' } },
    );

    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    rerender({ query: '' });
    await act(async () => { pending.reject(new Error('stale failure')); });
    expect(result.current).toEqual({ results: [], loading: false, error: null });
  });
});
