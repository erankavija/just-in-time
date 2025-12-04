import { useState } from 'react';
import Split from 'react-split';
import { GraphView } from './components/Graph/GraphView';
import { IssueDetail } from './components/Issue/IssueDetail';
import { useTheme } from './hooks/useTheme';
import './App.css';

function App() {
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const { theme, toggleTheme } = useTheme();

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <header className="app-header">
        <h1>$ jit --ui</h1>
        <button className="theme-toggle" onClick={toggleTheme}>
          {theme === 'dark' ? '☀️ Light' : '🌙 Dark'}
        </button>
      </header>
      
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
        jit v0.1.0 | api: {window.location.hostname}:3000 | {theme} mode
      </footer>
    </div>
  );
}

export default App;
