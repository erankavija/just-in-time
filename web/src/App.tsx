import { useState, useEffect } from 'react';
import Split from 'react-split';
import { GraphView } from './components/Graph/GraphView';
import { IssueDetail } from './components/Issue/IssueDetail';
import { SearchBar } from './components/Search/SearchBar';
import { useTheme } from './hooks/useTheme';
import { useSearch } from './components/Search/useSearch';
import { apiClient } from './api/client';
import type { Issue } from './types/models';
import './App.css';

function App() {
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [allIssues, setAllIssues] = useState<Issue[]>([]);
  const { theme, toggleTheme } = useTheme();
  const searchResults = useSearch(searchQuery, allIssues);

  // Load all issues for client-side search
  useEffect(() => {
    apiClient.listIssues().then(setAllIssues).catch(console.error);
  }, []);

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <header className="app-header">
        <h1>$ jit --ui</h1>
        <button className="theme-toggle" onClick={toggleTheme}>
          {theme === 'dark' ? '☀️ Light' : '🌙 Dark'}
        </button>
      </header>
      
      <div style={{ padding: '1rem', borderBottom: '1px solid var(--border)' }}>
        <SearchBar
          onSearch={setSearchQuery}
          query={searchQuery}
          loading={searchResults.loading}
          error={searchResults.error}
          resultCount={searchResults.results.length}
        />
        
        {searchQuery && searchResults.results.length > 0 && (
          <div style={{ 
            marginTop: '0.5rem',
            maxHeight: '200px',
            overflow: 'auto',
            background: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            padding: '0.5rem',
          }}>
            {searchResults.results.map((result, idx) => (
              <div
                key={idx}
                style={{
                  padding: '0.5rem',
                  cursor: 'pointer',
                  borderBottom: idx < searchResults.results.length - 1 ? '1px solid var(--border)' : 'none',
                  fontFamily: 'var(--font-mono)',
                  fontSize: '0.875rem',
                }}
                onClick={() => {
                  if (result.issue) {
                    setSelectedIssueId(result.issue.id);
                    setSearchQuery(''); // Clear search after selection
                  }
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = 'var(--accent-dim)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = 'transparent';
                }}
              >
                {result.issue ? (
                  <>
                    <span style={{ color: 'var(--text-secondary)' }}>{result.issue.id}</span>
                    {' • '}
                    <span style={{ color: 'var(--text-primary)' }}>{result.issue.title}</span>
                    {result.type === 'client' && (
                      <span style={{ 
                        marginLeft: '0.5rem',
                        fontSize: '0.75rem',
                        color: 'var(--accent)',
                      }}>
                        ⚡
                      </span>
                    )}
                  </>
                ) : (
                  <span style={{ color: 'var(--text-secondary)' }}>
                    📄 {result.serverResult?.path}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
      
      <div style={{ flex: 1, overflow: 'hidden' }}>
        <Split
          className="split"
          sizes={[65, 35]}
          minSize={[300, 300]}
          gutterSize={8}
          direction="horizontal"
          cursor="col-resize"
        >
          <div style={{ height: '100%', position: 'relative' }}>
            <GraphView onNodeClick={setSelectedIssueId} />
          </div>
          
          <div style={{ 
            height: '100%',
            overflow: 'auto',
            backgroundColor: 'var(--bg-secondary)',
          }}>
            <IssueDetail issueId={selectedIssueId} />
          </div>
        </Split>
      </div>

      <footer className="app-footer">
        jit v0.1.0 | api: {window.location.hostname}:3000 | {theme} mode | search: {searchQuery ? `"${searchQuery}"` : 'ready'}
      </footer>
    </div>
  );
}

export default App;
