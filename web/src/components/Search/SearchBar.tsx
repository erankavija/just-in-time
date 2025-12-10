import React, { useState } from 'react';

interface SearchBarProps {
  onSearch: (query: string) => void;
  query?: string;
  loading?: boolean;
  error?: string | null;
  resultCount?: number;
}

export const SearchBar: React.FC<SearchBarProps> = ({
  onSearch,
  query: controlledQuery,
  loading = false,
  error = null,
  resultCount,
}) => {
  // Use controlled query if provided, otherwise use local state
  const [localQuery, setLocalQuery] = useState('');
  const query = controlledQuery !== undefined ? controlledQuery : localQuery;

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newQuery = e.target.value;
    if (controlledQuery === undefined) {
      setLocalQuery(newQuery);
    }
    onSearch(newQuery);
  };

  const handleClear = () => {
    if (controlledQuery === undefined) {
      setLocalQuery('');
    }
    onSearch('');
  };

  const styles = {
    searchBar: {
      width: '100%',
    },
    inputWrapper: {
      position: 'relative' as const,
      display: 'flex',
      alignItems: 'center',
      gap: '0.5rem',
    },
    input: {
      flex: 1,
      padding: '0.5rem 2.5rem 0.5rem 0.75rem',
      fontSize: '0.875rem',
      fontFamily: 'var(--font-mono)',
      background: 'var(--bg-secondary)',
      color: 'var(--text-primary)',
      border: '1px solid var(--border)',
      borderRadius: '4px',
      outline: 'none',
    },
    clearButton: {
      position: 'absolute' as const,
      right: '0.5rem',
      width: '1.5rem',
      height: '1.5rem',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      background: 'transparent',
      border: 'none',
      color: 'var(--text-secondary)',
      fontSize: '1.5rem',
      lineHeight: 1,
      cursor: 'pointer',
      opacity: 0.6,
    },
    status: {
      fontSize: '0.75rem',
      color: 'var(--text-secondary)',
      fontFamily: 'var(--font-mono)',
    },
    error: {
      marginTop: '0.5rem',
      padding: '0.5rem',
      fontSize: '0.75rem',
      color: '#ef4444',
      background: 'rgba(239, 68, 68, 0.1)',
      borderRadius: '4px',
      fontFamily: 'var(--font-mono)',
    },
    resultCount: {
      marginTop: '0.5rem',
      fontSize: '0.75rem',
      color: 'var(--text-secondary)',
      fontFamily: 'var(--font-mono)',
    },
  };

  return (
    <div style={styles.searchBar}>
      <div style={styles.inputWrapper}>
        <input
          type="text"
          style={styles.input}
          placeholder="Search issues..."
          value={query}
          onChange={handleChange}
          aria-label="Search"
        />
        
        {query && (
          <button
            style={styles.clearButton}
            onClick={handleClear}
            aria-label="Clear search"
            title="Clear"
          >
            ×
          </button>
        )}
        
        {loading && (
          <span style={styles.status}>Searching...</span>
        )}
      </div>
      
      {error && (
        <div style={styles.error} role="alert">
          {error}
        </div>
      )}
      
      {query && resultCount !== undefined && !error && (
        <div style={styles.resultCount}>
          {resultCount} {resultCount === 1 ? 'result' : 'results'}
        </div>
      )}
    </div>
  );
};
