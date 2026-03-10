import { useEffect, useState, useCallback, useRef } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import 'katex/dist/katex.min.css';
import './IssueDetail.css';
import { apiClient } from '../../api/client';
import type { Issue, DocumentReference, GateDefinition, GateRunSummary, GateRunDetail } from '../../types/models';
import { DocumentViewer } from '../Document/DocumentViewer';
import { LabelBadge } from '../Labels/LabelBadge';

function timeAgo(dateStr: string): string {
  const seconds = Math.floor((Date.now() - new Date(dateStr).getTime()) / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

interface IssueDetailProps {
  issueId: string | null;
  allIssues?: Issue[];
  onNavigate?: (issueId: string) => void;
  onFocusInGraph?: (issueId: string) => void;
  /** Monotonic version from SSE — triggers re-fetch when changed */
  version?: number;
}

export function IssueDetail({ issueId, allIssues = [], onNavigate, onFocusInGraph, version }: IssueDetailProps) {
  const [issue, setIssue] = useState<Issue | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedDocument, setSelectedDocument] = useState<DocumentReference | null>(null);
  const [gateDefinitions, setGateDefinitions] = useState<Record<string, GateDefinition>>({});
  const [expandedGate, setExpandedGate] = useState<string | null>(null);
  const [gateRuns, setGateRuns] = useState<Record<string, GateRunSummary[]>>({});
  const [selectedRun, setSelectedRun] = useState<GateRunDetail | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  // Once an issue has loaded once, SSE-triggered refreshes update silently
  // (no loading overlay) so the pane doesn't flash on every version bump.
  const hasLoadedOnceRef = useRef(false);
  // Monotonic counter — each loadIssue call gets its own sequence number so
  // out-of-order responses (slow network, rapid version bumps) are discarded.
  const fetchSeqRef = useRef(0);

  useEffect(() => {
    if (!issueId) {
      setIssue(null);
      return;
    }

    loadIssue(issueId);
  }, [issueId, version]);

  // Load gate definitions when issue has gates
  const gateKeys = issue ? Object.keys(issue.gates_status) : [];
  const hasGates = gateKeys.length > 0;
  useEffect(() => {
    if (!hasGates) return;
    apiClient.listGates().then(gates => {
      const map: Record<string, GateDefinition> = {};
      for (const g of gates) map[g.key] = g;
      setGateDefinitions(map);
    }).catch(err => console.error('Failed to load gate definitions:', err));
  }, [hasGates]);

  // Reset gate UI state when issue changes
  useEffect(() => {
    hasLoadedOnceRef.current = false; // Show loading indicator for the newly selected issue
    fetchSeqRef.current = 0; // Reset sequence so stale in-flight requests are discarded
    setExpandedGate(null);
    setGateRuns({});
    setSelectedRun(null);
    setSelectedRunId(null);
  }, [issueId]);

  // When version bumps while gate history is expanded, refresh those runs so
  // the list reflects any new run that just completed.
  const expandedGateRef = useRef(expandedGate);
  useEffect(() => { expandedGateRef.current = expandedGate; }, [expandedGate]);
  const issueIdRef = useRef(issue?.id);
  useEffect(() => { issueIdRef.current = issue?.id; }, [issue?.id]);

  useEffect(() => {
    const gateKey = expandedGateRef.current;
    const id = issueIdRef.current;
    if (!gateKey || !id || version === 0) return;
    apiClient.listGateRuns(id, gateKey)
      .then(runs => setGateRuns(prev => ({ ...prev, [gateKey]: runs })))
      .catch(err => console.error('Failed to refresh gate runs:', err));
  }, [version]);

  const toggleGateHistory = useCallback(async (gateKey: string) => {
    if (expandedGate === gateKey) {
      setExpandedGate(null);
      setSelectedRun(null);
      setSelectedRunId(null);
      return;
    }
    setExpandedGate(gateKey);
    setSelectedRun(null);
    setSelectedRunId(null);
    if (!issue) return;
    if (!gateRuns[gateKey]) {
      try {
        const runs = await apiClient.listGateRuns(issue.id, gateKey);
        setGateRuns(prev => ({ ...prev, [gateKey]: runs }));
      } catch (err) {
        console.error('Failed to load gate runs:', err);
      }
    }
  }, [expandedGate, issue, gateRuns]);

  const loadRunDetail = useCallback(async (runId: string) => {
    if (!issue) return;
    if (selectedRunId === runId) {
      setSelectedRun(null);
      setSelectedRunId(null);
      return;
    }
    setSelectedRunId(runId);
    try {
      const detail = await apiClient.getGateRun(issue.id, runId);
      setSelectedRun(detail);
    } catch (err) {
      console.error('Failed to load gate run detail:', err);
    }
  }, [issue, selectedRunId]);

  const loadIssue = async (id: string) => {
    const seq = ++fetchSeqRef.current;
    const isBackground = hasLoadedOnceRef.current;
    try {
      if (!isBackground) setLoading(true);
      setError(null);
      const data = await apiClient.getIssue(id);
      // Discard responses that arrived out of order.
      if (fetchSeqRef.current !== seq) return;
      setIssue(data);
    } catch (err) {
      if (fetchSeqRef.current !== seq) return;
      setError(err instanceof Error ? err.message : 'Failed to load issue');
      console.error('Failed to load issue:', err);
    } finally {
      if (fetchSeqRef.current === seq) {
        hasLoadedOnceRef.current = true;
        if (!isBackground) setLoading(false);
      }
    }
  };

  if (!issueId) {
    return (
      <div style={{ 
        padding: '20px',
        color: 'var(--text-muted)',
        fontFamily: 'var(--font-mono)',
        fontSize: '12px',
      }}>
        $ select an issue from the graph
      </div>
    );
  }

  if (loading) {
    return (
      <div style={{ 
        padding: '20px',
        color: 'var(--text-secondary)',
        fontFamily: 'var(--font-mono)',
        fontSize: '12px',
      }}>
        $ loading...
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ 
        padding: '20px',
        color: 'var(--error)',
        fontFamily: 'var(--font-mono)',
        fontSize: '12px',
      }}>
        $ error: {error}
      </div>
    );
  }

  if (!issue) {
    return null;
  }

  const stateEmoji: Record<string, string> = {
    backlog: '⏸️',
    ready: '🟢',
    in_progress: '🟡',
    gated: '🟠',
    done: '✅',
    rejected: '🗑️',
    archived: '📦',
  };

  const priorityEmoji: Record<string, string> = {
    critical: '🔴',
    high: '🟠',
    normal: '🟡',
    low: '🟢',
  };

  return (
    <div style={{ 
      padding: '20px',
      height: '100%',
      overflow: 'auto',
      fontFamily: 'var(--font-mono)',
      fontSize: '12px',
    }}>
      <div style={{ 
        borderBottom: '1px solid var(--border)',
        paddingBottom: '16px',
        marginBottom: '16px',
      }}>
        <div style={{ 
          fontSize: '11px',
          color: 'var(--text-muted)',
          marginBottom: '6px',
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
        }}>
          <span>$ issue #{issue.id.substring(0, 8)}</span>
          {onFocusInGraph && (
            <button
              onClick={() => onFocusInGraph(issue.id)}
              style={{
                fontSize: '11px',
                color: 'var(--text-secondary)',
                backgroundColor: 'var(--bg-secondary)',
                border: '1px solid var(--border)',
                borderRadius: '4px',
                padding: '4px 8px',
                cursor: 'pointer',
                fontFamily: 'var(--font-mono)',
                display: 'flex',
                alignItems: 'center',
                gap: '4px',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.backgroundColor = 'var(--bg-tertiary)';
                e.currentTarget.style.borderColor = 'var(--accent)';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.backgroundColor = 'var(--bg-secondary)';
                e.currentTarget.style.borderColor = 'var(--border)';
              }}
            >
              📍 Focus in Graph
            </button>
          )}
        </div>
        <h1 style={{ 
          fontSize: '18px',
          margin: '0 0 12px 0',
          color: 'var(--text-primary)',
          fontWeight: 600,
        }}>
          {issue.title}
        </h1>
        
        <div style={{ display: 'flex', gap: '12px', flexWrap: 'wrap' }}>
          <span style={{ 
            fontSize: '11px',
            color: 'var(--text-secondary)',
            padding: '4px 8px',
            backgroundColor: 'var(--bg-tertiary)',
            borderRadius: '4px',
            border: '1px solid var(--border)',
          }}>
            {stateEmoji[issue.state]} {issue.state}
          </span>
          <span style={{ 
            fontSize: '11px',
            color: 'var(--text-secondary)',
            padding: '4px 8px',
            backgroundColor: 'var(--bg-tertiary)',
            borderRadius: '4px',
            border: '1px solid var(--border)',
          }}>
            {priorityEmoji[issue.priority]} {issue.priority}
          </span>
          {issue.assignee && (
            <span style={{ 
              fontSize: '11px',
              color: 'var(--text-secondary)',
              padding: '4px 8px',
              backgroundColor: 'var(--bg-tertiary)',
              borderRadius: '4px',
              border: '1px solid var(--border)',
            }}>
              @ {issue.assignee}
            </span>
          )}
        </div>

        {issue.labels && issue.labels.length > 0 && (
          <div style={{ 
            marginTop: '12px',
            display: 'flex',
            flexWrap: 'wrap',
            gap: '6px',
          }}>
            {issue.labels.map((label) => (
              <LabelBadge key={label} label={label} />
            ))}
          </div>
        )}
      </div>

      <section style={{ marginBottom: '20px' }}>
        <h2 style={{ 
          fontSize: '13px',
          fontWeight: 600,
          marginBottom: '10px',
          color: 'var(--text-primary)',
        }}>
          Description
        </h2>
        <div 
          className="markdown-content"
          style={{ 
            backgroundColor: 'var(--bg-tertiary)',
            padding: '16px',
            borderRadius: '6px',
            border: '1px solid var(--border)',
            fontSize: '14px',
            lineHeight: '1.7',
            color: 'var(--text-secondary)',
          }}
        >
          <ReactMarkdown
            remarkPlugins={[remarkMath, remarkGfm]}
            rehypePlugins={[rehypeKatex]}
            components={{
              code({ className, children, ...props }) {
                const match = /language-(\w+)/.exec(className || '');
                const isInline = !className;
                if (isInline) {
                  return (
                    <code className={className} {...props} style={{
                      backgroundColor: 'var(--bg-secondary)',
                      padding: '2px 6px',
                      borderRadius: '3px',
                      fontSize: '0.9em',
                      fontFamily: 'var(--font-mono)',
                    }}>
                      {children}
                    </code>
                  );
                }
                // Note: Type assertions needed due to react-syntax-highlighter v16 type definitions
                return match ? (
                  <SyntaxHighlighter
                    {...({
                      style: vscDarkPlus,
                      language: match[1],
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
                    backgroundColor: 'var(--bg-secondary)',
                    padding: '12px',
                    borderRadius: '6px',
                    fontFamily: 'var(--font-mono)',
                    fontSize: '13px',
                  }}>
                    {children}
                  </code>
                );
              },
            }}
          >
            {issue.description}
          </ReactMarkdown>
        </div>
      </section>

      {issue.dependencies && issue.dependencies.length > 0 && (
        <section style={{ marginBottom: '20px' }}>
          <h2 style={{ 
            fontSize: '13px',
            fontWeight: 600,
            marginBottom: '10px',
            color: 'var(--text-primary)',
          }}>
            → Dependencies ({issue.dependencies.length})
          </h2>
          <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
            {issue.dependencies.map((depId) => {
              const depIssue = allIssues.find(i => i.id === depId);
              const depTitle = depIssue?.title || 'Unknown issue';
              
              return (
                <li 
                  key={depId} 
                  style={{ 
                    padding: '8px 10px',
                    backgroundColor: 'var(--bg-tertiary)',
                    marginBottom: '6px',
                    borderRadius: '4px',
                    fontSize: '11px',
                    color: 'var(--text-secondary)',
                    border: '1px solid var(--border)',
                    fontFamily: 'var(--font-mono)',
                    cursor: onNavigate ? 'pointer' : 'default',
                    transition: 'background-color 0.2s',
                  }}
                  onClick={() => onNavigate?.(depId)}
                  onMouseEnter={(e) => {
                    if (onNavigate) {
                      e.currentTarget.style.backgroundColor = 'var(--bg-elevated)';
                      e.currentTarget.style.borderColor = 'var(--border-hover)';
                    }
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.backgroundColor = 'var(--bg-tertiary)';
                    e.currentTarget.style.borderColor = 'var(--border)';
                  }}
                >
                  <span style={{ color: 'var(--text-primary)', fontWeight: 500 }}>
                    {depTitle}
                  </span>
                  <span style={{ color: 'var(--text-muted)', marginLeft: '8px' }}>
                    #{depId.substring(0, 8)}
                  </span>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {issue.gates_status && Object.keys(issue.gates_status).length > 0 && (() => {
        const entries = Object.entries(issue.gates_status).sort(([a], [b]) => a.localeCompare(b));
        const passedCount = entries.filter(([, g]) => g.status === 'passed').length;
        return (
          <section style={{ marginBottom: '20px' }}>
            <h2 style={{
              fontSize: '13px',
              fontWeight: 600,
              marginBottom: '10px',
              color: 'var(--text-primary)',
            }}>
              Gates ({passedCount}/{entries.length} passed)
            </h2>
            <div style={{
              border: '1px solid var(--border)',
              borderRadius: '6px',
              overflow: 'hidden',
            }}>
              {entries.map(([gateKey, gateState], idx) => {
                const symbol = gateState.status === 'passed' ? '[✓]' : gateState.status === 'failed' ? '[✗]' : '[ ]';
                const color = gateState.status === 'passed' ? 'var(--success)' : gateState.status === 'failed' ? 'var(--error)' : 'var(--text-muted)';
                const def = gateDefinitions[gateKey];
                const isExpanded = expandedGate === gateKey;
                const runs = gateRuns[gateKey];

                return (
                  <div key={gateKey} style={{
                    borderTop: idx > 0 ? '1px solid var(--border)' : 'none',
                  }}>
                    <div style={{
                      padding: '10px 12px',
                      backgroundColor: 'var(--bg-tertiary)',
                    }}>
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', fontSize: '12px' }}>
                        <span style={{ color, fontWeight: 600 }}>{symbol}</span>
                        <span style={{ color: 'var(--text-primary)', fontWeight: 500 }}>{gateKey}</span>
                      </div>
                      {def && (
                        <div style={{ fontSize: '10px', color: 'var(--text-muted)', marginTop: '3px', marginLeft: '28px' }}>
                          {def.title} · {def.mode} · {def.stage}
                        </div>
                      )}
                      <div style={{ fontSize: '10px', color: 'var(--text-muted)', marginTop: '2px', marginLeft: '28px' }}>
                        {gateState.status === 'pending' ? 'pending' : (
                          <>{gateState.status}{gateState.updated_by ? ` by ${gateState.updated_by}` : ''} · {timeAgo(gateState.updated_at)}</>
                        )}
                      </div>
                      {(!def || def.mode === 'auto') && (
                        <div
                          style={{
                            fontSize: '10px',
                            color: 'var(--accent)',
                            marginTop: '4px',
                            marginLeft: '28px',
                            cursor: 'pointer',
                            userSelect: 'none',
                          }}
                          onClick={() => toggleGateHistory(gateKey)}
                        >
                          {isExpanded ? '▾' : '▸'} Run History{runs ? ` (${runs.length})` : ''}
                        </div>
                      )}
                    </div>

                    {isExpanded && (!def || def.mode === 'auto') && (
                      <div style={{
                        padding: '0 12px 10px 40px',
                        backgroundColor: 'var(--bg-tertiary)',
                      }}>
                        {!runs ? (
                          <div style={{ fontSize: '10px', color: 'var(--text-muted)' }}>Loading...</div>
                        ) : runs.length === 0 ? (
                          <div style={{ fontSize: '10px', color: 'var(--text-muted)' }}>No runs recorded</div>
                        ) : (
                          <div style={{
                            border: '1px solid var(--border)',
                            borderRadius: '4px',
                            overflow: 'hidden',
                          }}>
                            {runs.map((run, runIdx) => {
                              const runSymbol = run.status === 'passed' ? '✓' : run.status === 'failed' ? '✗' : run.status === 'error' ? '!' : '·';
                              const runColor = run.status === 'passed' ? 'var(--success)' : run.status === 'failed' || run.status === 'error' ? 'var(--error)' : 'var(--text-muted)';
                              const duration = run.duration_ms != null ? `(${(run.duration_ms / 1000).toFixed(1)}s)` : '';
                              const isSelected = selectedRunId === run.run_id;

                              return (
                                <div key={run.run_id} style={{
                                  borderTop: runIdx > 0 ? '1px solid var(--border)' : 'none',
                                }}>
                                  <div style={{
                                    padding: '6px 8px',
                                    fontSize: '10px',
                                    display: 'flex',
                                    gap: '8px',
                                    alignItems: 'center',
                                    backgroundColor: 'var(--bg-secondary)',
                                  }}>
                                    <span style={{ color: runColor, fontWeight: 600 }}>{runSymbol}</span>
                                    <span style={{ color: 'var(--text-secondary)' }}>{run.status}</span>
                                    <span style={{ color: 'var(--text-muted)' }}>{timeAgo(run.started_at)}</span>
                                    {duration && <span style={{ color: 'var(--text-muted)' }}>{duration}</span>}
                                    <span
                                      style={{ color: 'var(--accent)', cursor: 'pointer', marginLeft: 'auto' }}
                                      onClick={() => loadRunDetail(run.run_id)}
                                    >
                                      {isSelected ? 'Hide' : 'View'}
                                    </span>
                                  </div>
                                  {isSelected && selectedRun && (
                                    <div style={{
                                      padding: '8px',
                                      fontSize: '10px',
                                      backgroundColor: '#1e1e1e',
                                      color: '#d4d4d4',
                                      fontFamily: 'var(--font-mono)',
                                    }}>
                                      <div style={{ color: 'var(--text-muted)', marginBottom: '4px' }}>
                                        $ {selectedRun.command}
                                      </div>
                                      {selectedRun.stdout && (
                                        <pre style={{
                                          margin: '4px 0',
                                          padding: '6px',
                                          backgroundColor: '#252526',
                                          borderRadius: '3px',
                                          maxHeight: '200px',
                                          overflow: 'auto',
                                          whiteSpace: 'pre-wrap',
                                          wordBreak: 'break-all',
                                          fontSize: '10px',
                                          lineHeight: '1.4',
                                        }}>
                                          {selectedRun.stdout}
                                        </pre>
                                      )}
                                      {selectedRun.stderr && (
                                        <>
                                          <div style={{ color: 'var(--error)', marginTop: '4px', marginBottom: '2px' }}>stderr:</div>
                                          <pre style={{
                                            margin: '0',
                                            padding: '6px',
                                            backgroundColor: '#252526',
                                            borderRadius: '3px',
                                            maxHeight: '200px',
                                            overflow: 'auto',
                                            whiteSpace: 'pre-wrap',
                                            wordBreak: 'break-all',
                                            fontSize: '10px',
                                            lineHeight: '1.4',
                                            color: '#f48771',
                                          }}>
                                            {selectedRun.stderr}
                                          </pre>
                                        </>
                                      )}
                                      {selectedRun.exit_code != null && (
                                        <div style={{ color: 'var(--text-muted)', marginTop: '4px' }}>
                                          exit code: {selectedRun.exit_code}
                                          {selectedRun.commit && <> · commit: {selectedRun.commit.substring(0, 7)}</>}
                                        </div>
                                      )}
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </section>
        );
      })()}

      {issue.documents && issue.documents.length > 0 && (
        <section style={{ marginBottom: '20px' }}>
          <h2 style={{ 
            fontSize: '13px',
            fontWeight: 600,
            marginBottom: '10px',
            color: 'var(--text-primary)',
          }}>
            📄 Documents ({issue.documents.length})
          </h2>
          <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
            {issue.documents.map((doc, idx) => (
              <div 
                key={idx}
                onClick={() => setSelectedDocument(doc)}
                style={{ 
                  padding: '10px 12px',
                  backgroundColor: 'var(--bg-tertiary)',
                  borderRadius: '4px',
                  fontSize: '12px',
                  color: 'var(--text-secondary)',
                  border: '1px solid var(--border)',
                  cursor: 'pointer',
                  transition: 'all 0.2s',
                  fontFamily: 'var(--font-mono)',
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.backgroundColor = 'var(--bg-secondary)';
                  e.currentTarget.style.borderColor = 'var(--text-secondary)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.backgroundColor = 'var(--bg-tertiary)';
                  e.currentTarget.style.borderColor = 'var(--border)';
                }}
              >
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '4px' }}>
                  <span style={{ 
                    color: 'var(--text-primary)',
                    fontWeight: 500,
                  }}>
                    📄 {doc.label || doc.path}
                  </span>
                  {doc.commit && (
                    <span style={{ 
                      fontSize: '10px',
                      padding: '2px 6px',
                      backgroundColor: 'var(--bg-secondary)',
                      borderRadius: '3px',
                      color: 'var(--accent)',
                      border: '1px solid var(--border)',
                    }}>
                      @{doc.commit.substring(0, 8)}
                    </span>
                  )}
                </div>
                <div style={{ 
                  fontSize: '10px',
                  color: 'var(--text-muted)',
                }}>
                  {doc.path}
                </div>
                {doc.doc_type && (
                  <div style={{ 
                    fontSize: '10px',
                    color: 'var(--text-muted)',
                    marginTop: '2px',
                  }}>
                    Type: {doc.doc_type}
                  </div>
                )}
              </div>
            ))}
          </div>
        </section>
      )}

      <div style={{ 
        fontSize: '10px',
        color: 'var(--text-muted)',
        marginTop: '24px',
        paddingTop: '16px',
        borderTop: '1px solid var(--border)',
      }}>
        <div>created: {new Date(issue.created_at).toLocaleString()}</div>
        <div>updated: {new Date(issue.updated_at).toLocaleString()}</div>
      </div>

      {selectedDocument && (
        <div style={{ 
          position: 'fixed',
          top: 0,
          left: 0,
          right: 0,
          bottom: 0,
          backgroundColor: 'rgba(0, 0, 0, 0.5)',
          display: 'flex',
          justifyContent: 'center',
          alignItems: 'center',
          zIndex: 1000,
        }}>
          <div style={{ 
            width: '90%',
            maxWidth: '1200px',
            height: '90%',
            backgroundColor: 'var(--bg-primary)',
            borderRadius: '8px',
            overflow: 'hidden',
            boxShadow: '0 10px 40px rgba(0, 0, 0, 0.3)',
          }}>
            <DocumentViewer
              issueId={issue.id}
              documentRef={selectedDocument}
              onClose={() => setSelectedDocument(null)}
            />
          </div>
        </div>
      )}
    </div>
  );
}
