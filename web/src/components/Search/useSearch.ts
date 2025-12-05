import { useState, useEffect, useMemo, useCallback } from 'react';
import { filterIssues, type ClientSearchResult } from '../../utils/clientSearch';
import { apiClient } from '../../api/client';
import type { Issue, SearchResult } from '../../types/models';

export interface SearchResultItem {
  type: 'client' | 'server';
  issue?: Issue;
  serverResult?: SearchResult;
  score?: number;
}

export interface UseSearchResult {
  results: SearchResultItem[];
  loading: boolean;
  error: string | null;
}

const DEBOUNCE_MS = 300;
const MIN_QUERY_LENGTH = 3;
const SERVER_LIMIT = 50;

/**
 * Custom hook for hybrid client + server search
 * 
 * - Instant client-side filtering of loaded issues
 * - Debounced server search for deep content search
 * - Merges results with client results shown first
 * 
 * @param query - Search query string
 * @param allIssues - All loaded issues for client-side filtering
 */
export function useSearch(query: string, allIssues: Issue[]): UseSearchResult {
  const [serverResults, setServerResults] = useState<SearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Instant client-side filtering
  const clientResults: ClientSearchResult[] = useMemo(() => {
    if (!query.trim()) return [];
    return filterIssues(allIssues, query);
  }, [query, allIssues]);

  // Debounced server search
  const performServerSearch = useCallback(async (searchQuery: string) => {
    if (searchQuery.length < MIN_QUERY_LENGTH) {
      setServerResults([]);
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const response = await apiClient.search(searchQuery, { limit: SERVER_LIMIT });
      setServerResults(response.results);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Search failed';
      setError(message);
      console.error('Server search error:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  // Debounced effect for server search
  useEffect(() => {
    if (!query.trim()) {
      setServerResults([]);
      setLoading(false);
      setError(null);
      return;
    }

    const timeoutId = setTimeout(() => {
      performServerSearch(query);
    }, DEBOUNCE_MS);

    return () => clearTimeout(timeoutId);
  }, [query, performServerSearch]);

  // Merge client and server results
  const mergedResults: SearchResultItem[] = useMemo(() => {
    if (!query.trim()) return [];

    const results: SearchResultItem[] = [];
    const seenIssueIds = new Set<string>();

    // Add client results first (instant feedback)
    for (const clientResult of clientResults) {
      results.push({
        type: 'client',
        issue: clientResult.issue,
        score: clientResult.score,
      });
      seenIssueIds.add(clientResult.issue.id);
    }

    // Add server results that aren't already shown
    for (const serverResult of serverResults) {
      if (serverResult.issue_id && !seenIssueIds.has(serverResult.issue_id)) {
        results.push({
          type: 'server',
          serverResult,
        });
        seenIssueIds.add(serverResult.issue_id);
      } else if (!serverResult.issue_id) {
        // Document result (no issue_id)
        results.push({
          type: 'server',
          serverResult,
        });
      }
    }

    return results;
  }, [query, clientResults, serverResults]);

  return {
    results: mergedResults,
    loading,
    error,
  };
}
