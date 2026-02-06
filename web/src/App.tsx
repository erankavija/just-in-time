import { useState, useEffect, useMemo } from 'react';
import Split from 'react-split';
import { GraphView } from './components/Graph/GraphView';
import { IssueDetail } from './components/Issue/IssueDetail';
import { SearchBar } from './components/Search/SearchBar';
import { LabelFilter } from './components/Labels/LabelFilter';
import { DocumentViewer } from './components/Document/DocumentViewer';
import { useTheme } from './hooks/useTheme';
import { useSearch } from './components/Search/useSearch';
import { apiClient } from './api/client';
import type { Issue } from './types/models';
import './App.css';

import type { ViewMode, LayoutAlgorithm } from './components/Graph/GraphView';

function App() {
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [allIssues, setAllIssues] = useState<Issue[]>([]);
  const [viewMode, setViewMode] = useState<ViewMode>('tactical');
  const [layoutAlgorithm, setLayoutAlgorithm] = useState<LayoutAlgorithm>('compact');
  const [labelFilters, setLabelFilters] = useState<string[]>([]);
  const [documentViewerState, setDocumentViewerState] = useState<{
    path: string;
    searchQuery?: string;
  } | null>(null);
  const { theme, toggleTheme } = useTheme();
  const searchResults = useSearch(searchQuery, allIssues);

  // Load all issues for client-side search
  useEffect(() => {
    apiClient.listIssues().then(setAllIssues).catch(console.error);
  }, []);

  // Extract all unique labels from issues
  const allLabels = useMemo(() => {
    const labelSet = new Set<string>();
    for (const issue of allIssues) {
      for (const label of issue.labels) {
        labelSet.add(label);
      }
    }
    return Array.from(labelSet).sort();
  }, [allIssues]);

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <header className="app-header">
        <h1>$ jit --ui</h1>
        <div style={{ flex: 1, maxWidth: '600px', margin: '0 2rem', position: 'relative' }}>
          <SearchBar
            onSearch={setSearchQuery}
            query={searchQuery}
            loading={searchResults.loading}
            error={searchResults.error}
            resultCount={searchResults.results.length}
          />
          
          {searchQuery && searchResults.results.length > 0 && (
            <div style={{ 
              position: 'absolute',
              top: '100%',
              left: 0,
              right: 0,
              marginTop: '0.5rem',
              maxHeight: '200px',
              overflow: 'auto',
              background: 'var(--bg-secondary)',
              border: '1px solid var(--border)',
              borderRadius: '4px',
              padding: '0.5rem',
              zIndex: 1000,
              boxShadow: '0 4px 12px rgba(0, 0, 0, 0.5)',
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
                    } else if (result.serverResult) {
                      // Open document viewer for document matches
                      setDocumentViewerState({
                        path: result.serverResult.path,
                        searchQuery: searchQuery,
                      });
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
                    <div style={{ display: 'flex', flexDirection: 'column', gap: '0.25rem' }}>
                      <div>
                        <span style={{ color: 'var(--text-secondary)' }}>📄 {result.serverResult?.path}</span>
                        <span style={{ marginLeft: '0.5rem', color: 'var(--text-muted)', fontSize: '0.75rem' }}>
                          Line {result.serverResult?.line_number}
                        </span>
                      </div>
                      {result.serverResult?.line_text && (
                        <div style={{ 
                          fontSize: '0.75rem',
                          color: 'var(--text-muted)',
                          paddingLeft: '1rem',
                          fontStyle: 'italic',
                          whiteSpace: 'nowrap',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                        }}>
                          {result.serverResult.line_text.trim()}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
        <div style={{ display: 'flex', gap: '0.5rem' }}>
          <button 
            className="theme-toggle"
            onClick={() => setViewMode(viewMode === 'tactical' ? 'strategic' : 'tactical')}
            title={viewMode === 'tactical' ? 'Switch to Strategic View' : 'Switch to Tactical View'}
          >
            {viewMode === 'tactical' ? '📋 Tactical' : '🎯 Strategic'}
          </button>
          <button className="theme-toggle" onClick={toggleTheme}>
            {theme === 'dark' ? '☀️ Light' : '🌙 Dark'}
          </button>
        </div>
      </header>

      <LabelFilter 
        labels={allLabels}
        selectedPatterns={labelFilters}
        onChange={setLabelFilters}
      />
      
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
            <GraphView 
              onNodeClick={setSelectedIssueId} 
              viewMode={viewMode}
              labelFilters={labelFilters}
              layoutAlgorithm={layoutAlgorithm}
              onLayoutChange={setLayoutAlgorithm}
            />
          </div>
          
          <div style={{ 
            height: '100%',
            overflow: 'auto',
            backgroundColor: 'var(--bg-secondary)',
          }}>
            <IssueDetail 
              issueId={selectedIssueId}
              allIssues={allIssues}
              onNavigate={setSelectedIssueId}
            />
          </div>
        </Split>
      </div>

      <footer className="app-footer">
        jit v0.1.0 | api: {window.location.hostname}:3000 | {theme} mode | search: {searchQuery ? `"${searchQuery}"` : 'ready'}
      </footer>

      {/* Document Viewer Modal */}
      {documentViewerState && (
        <div style={{
          position: 'fixed',
          top: 0,
          left: 0,
          right: 0,
          bottom: 0,
          backgroundColor: 'rgba(0, 0, 0, 0.8)',
          zIndex: 2000,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: '2rem',
        }}
        onClick={() => setDocumentViewerState(null)}
        >
          <div 
            style={{
              width: '90%',
              maxWidth: '1200px',
              height: '90%',
              backgroundColor: 'var(--bg-primary)',
              borderRadius: '8px',
              overflow: 'hidden',
              boxShadow: '0 10px 40px rgba(0, 0, 0, 0.5)',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <DocumentViewer
              documentPath={documentViewerState.path}
              searchQuery={documentViewerState.searchQuery}
              onClose={() => setDocumentViewerState(null)}
            />
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
