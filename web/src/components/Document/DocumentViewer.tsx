import { useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import { apiClient } from '../../api/client';
import type { DocumentReference, DocumentContent } from '../../types/models';
import './Document.css';

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

  useEffect(() => {
    loadContent(documentRef.path, selectedCommit);
  }, [issueId, documentRef.path, selectedCommit]);

  const loadContent = async (path: string, commit?: string) => {
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
  };

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

      <div className="document-content markdown-content">
        <ReactMarkdown
          remarkPlugins={[remarkMath]}
          rehypePlugins={[rehypeKatex]}
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
  const [history, setHistory] = useState<any>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadHistory();
  }, [issueId, path]);

  const loadHistory = async () => {
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
  };

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
      {history.commits.map((commit: any) => (
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
