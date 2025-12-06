import { useEffect, useState, useRef, useCallback } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import mermaid from 'mermaid';
import 'katex/dist/katex.min.css';
import { apiClient } from '../../api/client';
import type { DocumentReference, DocumentContent, DocumentHistory } from '../../types/models';
import './Document.css';

// Initialize mermaid
mermaid.initialize({
  startOnLoad: false,
  theme: 'dark',
  themeVariables: {
    primaryColor: '#6366f1',
    primaryTextColor: '#e5e7eb',
    primaryBorderColor: '#4b5563',
    lineColor: '#9ca3af',
    secondaryColor: '#3b82f6',
    tertiaryColor: '#8b5cf6',
    background: '#1f2937',
    mainBkg: '#1f2937',
    textColor: '#e5e7eb',
    fontSize: '14px',
  },
});

interface DocumentViewerProps {
  issueId: string;
  documentRef: DocumentReference;
  onClose?: () => void;
}

export function DocumentViewer({ issueId, documentRef, onClose }: DocumentViewerProps) {
  const [content, setContent] = useState<DocumentContent | null>(null);
  const [selectedCommit, setSelectedCommit] = useState<string | undefined>(
    documentRef.commit || undefined
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const mermaidContainerRef = useRef<HTMLDivElement>(null);

  // Render mermaid diagrams after content loads
  useEffect(() => {
    if (content && mermaidContainerRef.current) {
      const renderMermaid = async () => {
        const mermaidElements = mermaidContainerRef.current?.querySelectorAll('.language-mermaid');
        if (mermaidElements) {
          for (let i = 0; i < mermaidElements.length; i++) {
            const element = mermaidElements[i] as HTMLElement;
            const code = element.textContent || '';
            const id = `mermaid-${Date.now()}-${i}`;
            try {
              const { svg } = await mermaid.render(id, code);
              element.innerHTML = svg;
              element.classList.remove('language-mermaid');
              element.classList.add('mermaid-rendered');
            } catch (error) {
              console.error('Mermaid rendering error:', error);
              element.innerHTML = `<div style="color: var(--error); padding: 10px;">Failed to render diagram</div>`;
            }
          }
        }
      };
      renderMermaid();
    }
  }, [content]);

  const loadContent = useCallback(async (path: string, commit?: string) => {
    try {
      setLoading(true);
      setError(null);
      const data = await apiClient.getDocumentContent(issueId, path, commit);
      setContent(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load document');
      console.error('Failed to load document:', err);
    } finally {
      setLoading(false);
    }
  }, [issueId]);

  useEffect(() => {
    loadContent(documentRef.path, selectedCommit);
  }, [documentRef.path, selectedCommit, loadContent]);

  if (loading) {
    return (
      <div className="document-viewer">
        <div style={{ padding: '20px', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: '12px' }}>
          $ loading document...
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="document-viewer">
        <div className="document-header">
          <h3>{documentRef.label || documentRef.path}</h3>
          <button className="close-btn" onClick={onClose}>✕</button>
        </div>
        <div style={{ padding: '20px', color: 'var(--error)', fontFamily: 'var(--font-mono)', fontSize: '12px' }}>
          $ error: {error}
        </div>
      </div>
    );
  }

  if (!content) {
    return null;
  }

  return (
    <div className="document-viewer">
      <div className="document-header">
        <div className="document-title">
          <span className="document-icon">📄</span>
          <h3>{documentRef.label || documentRef.path}</h3>
        </div>
        <div className="document-actions">
          <button 
            className="history-btn"
            onClick={() => setShowHistory(!showHistory)}
            title="View commit history"
          >
            {showHistory ? '✕' : '📜'} {showHistory ? 'Close' : 'History'}
          </button>
          {onClose && (
            <button className="close-btn" onClick={onClose} title="Close document">
              ✕
            </button>
          )}
        </div>
      </div>

      {showHistory && (
        <div className="document-history-panel">
          <DocumentHistory
            issueId={issueId}
            path={documentRef.path}
            currentCommit={content.commit}
            onSelectCommit={(commit) => {
              setSelectedCommit(commit);
              setShowHistory(false);
            }}
          />
        </div>
      )}

      <div className="document-content markdown-content" ref={mermaidContainerRef}>
        <ReactMarkdown
          remarkPlugins={[remarkMath, remarkGfm]}
          rehypePlugins={[rehypeKatex]}
          components={{
            code({ className, children, ...props }) {
              const match = /language-(\w+)/.exec(className || '');
              const language = match ? match[1] : '';
              const isInline = !className;
              
              // Handle mermaid diagrams
              if (language === 'mermaid') {
                return (
                  <pre className="language-mermaid" style={{ 
                    backgroundColor: 'var(--bg-secondary)',
                    padding: '20px',
                    borderRadius: '6px',
                    overflow: 'auto',
                  }}>
                    {String(children).replace(/\n$/, '')}
                  </pre>
                );
              }
              
              // Handle inline code
              if (isInline) {
                return (
                  <code className={className} {...props} style={{
                    backgroundColor: 'var(--bg-tertiary)',
                    padding: '2px 6px',
                    borderRadius: '3px',
                    fontSize: '0.9em',
                    fontFamily: 'var(--font-mono)',
                  }}>
                    {children}
                  </code>
                );
              }
              
              // Handle code blocks with syntax highlighting
              // Note: Type assertions needed due to react-syntax-highlighter v16 type definitions
              return match ? (
                <SyntaxHighlighter
                  {...({
                    style: vscDarkPlus,
                    language,
                    PreTag: 'div',
                    customStyle: {
                      margin: '1em 0',
                      borderRadius: '6px',
                      fontSize: '13px',
                    },
                    children: String(children).replace(/\n$/, ''),
                  } as React.ComponentProps<typeof SyntaxHighlighter>)}
                />
              ) : (
                <code className={className} {...props} style={{
                  display: 'block',
                  backgroundColor: 'var(--bg-tertiary)',
                  padding: '12px',
                  borderRadius: '6px',
                  fontFamily: 'var(--font-mono)',
                  fontSize: '13px',
                  overflowX: 'auto',
                }}>
                  {children}
                </code>
              );
            },
          }}
        >
          {content.content}
        </ReactMarkdown>
      </div>

      <div className="document-footer">
        <span className="commit-badge">
          Commit: {content.commit.substring(0, 8)}
        </span>
        {content.content_type && (
          <span className="content-type-badge">
            {content.content_type}
          </span>
        )}
      </div>
    </div>
  );
}

// DocumentHistory component - will be in separate file
interface DocumentHistoryProps {
  issueId: string;
  path: string;
  currentCommit: string;
  onSelectCommit: (commit: string) => void;
}

function DocumentHistory({ issueId, path, currentCommit, onSelectCommit }: DocumentHistoryProps) {
  const [history, setHistory] = useState<DocumentHistory | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadHistory = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await apiClient.getDocumentHistory(issueId, path);
      setHistory(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load history');
      console.error('Failed to load history:', err);
    } finally {
      setLoading(false);
    }
  }, [issueId, path]);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  if (loading) {
    return <div style={{ padding: '10px', color: 'var(--text-muted)' }}>Loading history...</div>;
  }

  if (error) {
    return <div style={{ padding: '10px', color: 'var(--error)' }}>Error: {error}</div>;
  }

  if (!history || history.commits.length === 0) {
    return <div style={{ padding: '10px', color: 'var(--text-muted)' }}>No history available</div>;
  }

  return (
    <div className="commit-list">
      <h4>Commit History</h4>
      {history.commits.map((commit) => (
        <div
          key={commit.commit}
          className={`commit-item ${commit.commit === currentCommit ? 'active' : ''}`}
          onClick={() => onSelectCommit(commit.commit)}
        >
          <div className="commit-hash">{commit.commit.substring(0, 8)}</div>
          <div className="commit-message">{commit.message}</div>
          <div className="commit-meta">
            {commit.author} • {new Date(commit.date).toLocaleDateString()}
          </div>
        </div>
      ))}
    </div>
  );
}
