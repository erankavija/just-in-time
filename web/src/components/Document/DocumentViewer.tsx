import { useEffect, useState, useCallback } from 'react';
import { apiClient } from '../../api/client';
import type { DocumentReference, DocumentContent, DocumentHistory } from '../../types/models';
import type { DocumentPreviewState } from './renderers/index';
import { DEFAULT_PREVIEW_MAX_ITEMS } from './renderers/constants';
import { pickRenderer } from './renderers/index';
import './Document.css';

interface DocumentViewerProps {
  // Option 1: Via issue context
  issueId?: string;
  documentRef?: DocumentReference;
  // Option 2: Standalone via path
  documentPath?: string;
  searchQuery?: string;
  onClose?: () => void;
}

const RICH_PREVIEW_MAX_LINES = DEFAULT_PREVIEW_MAX_ITEMS;

interface PreviewState {
  isCapped: boolean;
  kind: 'lines' | 'rows';
  maxItems: number;
  totalItems: number;
}

function getPreviewState(
  content: DocumentContent,
  supportsPreviewCap: boolean,
): PreviewState {
  if (!supportsPreviewCap) {
    return { isCapped: false, kind: 'lines', maxItems: RICH_PREVIEW_MAX_LINES, totalItems: 0 };
  }

  const totalLines = content.content.split('\n').length;
  return {
    isCapped: totalLines > RICH_PREVIEW_MAX_LINES,
    kind: 'lines',
    maxItems: RICH_PREVIEW_MAX_LINES,
    totalItems: totalLines,
  };
}

function buildRichViewContent(
  content: DocumentContent,
  previewState: PreviewState,
): DocumentContent {
  if (!previewState.isCapped) {
    return content;
  }

  return {
    ...content,
    content: content.content.split('\n').slice(0, previewState.maxItems).join('\n'),
  };
}

export function DocumentViewer({ issueId, documentRef, documentPath, searchQuery, onClose }: DocumentViewerProps) {
  const [content, setContent] = useState<DocumentContent | null>(null);
  const [selectedCommit, setSelectedCommit] = useState<string | undefined>(
    documentRef?.commit || undefined
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  // Owned here so the ESC hint in the header stays in sync with renderer state.
  const [highlightsActive, setHighlightsActive] = useState(true);
  const [viewMode, setViewMode] = useState<'raw' | 'rich'>('rich');

  const loadContent = useCallback(async (path: string, commit?: string) => {
    try {
      setLoading(true);
      setError(null);

      // Use standalone endpoint if no issueId, otherwise use issue-specific endpoint
      let data: DocumentContent;
      if (issueId) {
        data = await apiClient.getDocumentContent(issueId, path, commit);
      } else {
        data = await apiClient.getDocumentByPath(path, commit);
      }

      setContent(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load document');
      console.error('Failed to load document:', err);
    } finally {
      setLoading(false);
    }
  }, [issueId]);

  useEffect(() => {
    const path = documentPath || documentRef?.path;
    if (path) {
      loadContent(path, selectedCommit);
    }
  }, [documentPath, documentRef?.path, selectedCommit, loadContent]);

  // Reset highlight state whenever the search query changes so ESC hint reappears.
  useEffect(() => {
    setHighlightsActive(true);
  }, [searchQuery]);

  useEffect(() => {
    setViewMode('rich');
  }, [content?.path, documentRef?.path, documentPath]);

  if (loading) {
    return (
      <div className="document-viewer">
        <div style={{ padding: '20px', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: '12px' }}>
          $ loading document...
        </div>
      </div>
    );
  }

  const displayPath = documentPath || documentRef?.path || 'Unknown';
  const displayLabel = documentRef?.label || displayPath;

  if (error) {
    return (
      <div className="document-viewer">
        <div className="document-header">
          <h3>{displayLabel}</h3>
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

  const activeRenderer = pickRenderer(content, documentRef);
  const supportsRawToggle = activeRenderer.capabilities.supportsRawToggle;
  const supportsSearchHighlight = activeRenderer.capabilities.supportsSearchHighlight;
  const previewState: DocumentPreviewState = activeRenderer.getPreviewState?.(content)
    ?? getPreviewState(content, activeRenderer.capabilities.supportsPreviewCap);
  const rendererContent = viewMode === 'raw'
    ? content
    : activeRenderer.prepareRichContent?.(content, previewState)
      ?? buildRichViewContent(content, previewState);
  const Renderer = activeRenderer.Component;
  const showHistoryControls = activeRenderer.capabilities.showsHistory;
  const showSearchHint = Boolean(searchQuery)
    && supportsSearchHighlight
    && viewMode === 'rich'
    && highlightsActive;

  return (
    <div className="document-viewer">
      <div className="document-header">
        <div className="document-title">
          <span className="document-icon">📄</span>
          <h3>{displayLabel}</h3>
          {searchQuery && (
            <span style={{ marginLeft: '1rem', fontSize: '0.875rem', color: 'var(--text-muted)' }}>
              Searching: "{searchQuery}"
              {showSearchHint && (
                <span style={{ marginLeft: '0.5rem', fontSize: '0.75rem', opacity: 0.7 }}>
                  (ESC to clear)
                </span>
              )}
            </span>
          )}
        </div>
        <div className="document-actions">
          {supportsRawToggle && (
            <button
              className="history-btn"
              onClick={() => setViewMode((current) => current === 'rich' ? 'raw' : 'rich')}
              title={viewMode === 'rich' ? 'Switch to raw view' : 'Switch to rich view'}
            >
              {viewMode === 'rich' ? 'Raw view' : 'Rich view'}
            </button>
          )}
          {showHistoryControls && issueId && documentRef && (
            <button
              className="history-btn"
              onClick={() => setShowHistory(!showHistory)}
              title="View commit history"
            >
              {showHistory ? '✕' : '📜'} {showHistory ? 'Close' : 'History'}
            </button>
          )}
          {onClose && (
            <button className="close-btn" onClick={onClose} title="Close document">
              ✕
            </button>
          )}
        </div>
      </div>

      {previewState.isCapped && viewMode === 'rich' && (
        <div
          data-testid="preview-cap-notice"
          style={{
            padding: '0.75rem 1rem',
            borderBottom: '1px solid var(--border-color)',
            color: 'var(--text-muted)',
            fontSize: '0.875rem',
          }}
        >
          Preview capped at {previewState.maxItems} {previewState.kind} ({previewState.totalItems} total). Switch to raw view for the full document.
        </div>
      )}

      {showHistoryControls && showHistory && issueId && documentRef && (
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

      <div className="document-content markdown-content" style={{ position: 'relative' }}>
        {viewMode === 'raw' && supportsRawToggle ? (
          <pre
            data-testid="raw-document-view"
            style={{
              margin: 0,
              padding: '1rem',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              fontFamily: 'var(--font-mono)',
              fontSize: '13px',
              lineHeight: 1.5,
            }}
          >
            {content.content}
          </pre>
        ) : (
          <Renderer
            content={rendererContent}
            issueId={issueId}
            documentRef={documentRef}
            previewState={previewState}
            searchTerm={supportsSearchHighlight ? searchQuery : undefined}
            highlightsActive={supportsSearchHighlight ? highlightsActive : false}
            onHighlightsCleared={() => setHighlightsActive(false)}
          />
        )}
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
