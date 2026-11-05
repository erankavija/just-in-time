import { useEffect, useMemo, useState } from 'react';
import { apiClient } from '../../api/client';
import type { SearchResult } from '../../types/models';

export interface UseSearchResult {
  results: SearchResult[];
  loading: boolean;
  error: string | null;
}

interface SearchState {
  identity: object;
  results: SearchResult[];
  error: string | null;
}

const DEBOUNCE_MS = 300;
const SERVER_LIMIT = 50;

export function useSearch(query: string, version: number): UseSearchResult {
  const searchQuery = query.trim();
  const identity = useMemo(() => ({ searchQuery, version }), [searchQuery, version]);
  const [state, setState] = useState<SearchState | null>(null);

  useEffect(() => {
    if (!searchQuery) return;

    let active = true;
    const timeoutId = setTimeout(() => {
      apiClient.search(searchQuery, { limit: SERVER_LIMIT })
        .then((response) => {
          if (active) setState({ identity, results: response.results, error: null });
        })
        .catch((error: unknown) => {
          if (active) {
            setState({
              identity,
              results: [],
              error: error instanceof Error ? error.message : 'Search failed',
            });
          }
        });
    }, DEBOUNCE_MS);

    return () => {
      active = false;
      clearTimeout(timeoutId);
    };
  }, [identity, searchQuery]);

  const current = state?.identity === identity ? state : null;
  return {
    results: searchQuery ? current?.results ?? [] : [],
    loading: Boolean(searchQuery) && current === null,
    error: searchQuery ? current?.error ?? null : null,
  };
}
