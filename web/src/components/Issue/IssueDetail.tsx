import { useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import 'katex/dist/katex.min.css';
import './IssueDetail.css';
import { apiClient } from '../../api/client';
import type { Issue, DocumentReference } from '../../types/models';
import { DocumentViewer } from '../Document/DocumentViewer';
import { LabelBadge } from '../Labels/LabelBadge';

interface IssueDetailProps {
  issueId: string | null;
}

export function IssueDetail({ issueId }: IssueDetailProps) {
  const [issue, setIssue] = useState<Issue | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedDocument, setSelectedDocument] = useState<DocumentReference | null>(null);

  useEffect(() => {
    if (!issueId) {
      setIssue(null);
      return;
    }

    loadIssue(issueId);
  }, [issueId]);

  const loadIssue = async (id: string) => {
    try {
      setLoading(true);
      setError(null);
      const data = await apiClient.getIssue(id);
      setIssue(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load issue');
      console.error('Failed to load issue:', err);
    } finally {
      setLoading(false);
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
        }}>
          $ issue #{issue.id.substring(0, 8)}
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
            {issue.dependencies.map((dep) => (
              <li key={dep} style={{ 
                padding: '8px 10px',
                backgroundColor: 'var(--bg-tertiary)',
                marginBottom: '6px',
                borderRadius: '4px',
                fontSize: '11px',
                color: 'var(--text-secondary)',
                border: '1px solid var(--border)',
                fontFamily: 'var(--font-mono)',
              }}>
                #{dep.substring(0, 8)}
              </li>
            ))}
          </ul>
        </section>
      )}

      {issue.gates_status && issue.gates_status.length > 0 && (
        <section style={{ marginBottom: '20px' }}>
          <h2 style={{ 
            fontSize: '13px',
            fontWeight: 600,
            marginBottom: '10px',
            color: 'var(--text-primary)',
          }}>
            ✓ Gates ({issue.gates_status.filter(g => g.state === 'passed').length}/{issue.gates_status.length} passed)
          </h2>
          <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
            {issue.gates_status.map((gate) => {
              const symbol = gate.state === 'passed' ? '[✓]' : gate.state === 'failed' ? '[✗]' : '[ ]';
              const color = gate.state === 'passed' ? 'var(--success)' : gate.state === 'failed' ? 'var(--error)' : 'var(--text-muted)';
              return (
                <li key={gate.gate_key} style={{ 
                  padding: '8px 10px',
                  backgroundColor: 'var(--bg-tertiary)',
                  marginBottom: '6px',
                  borderRadius: '4px',
                  fontSize: '11px',
                  border: '1px solid var(--border)',
                  fontFamily: 'var(--font-mono)',
                  display: 'flex',
                  gap: '8px',
                }}>
                  <span style={{ color }}>{symbol}</span>
                  <span style={{ color: 'var(--text-secondary)' }}>{gate.gate_key}</span>
                </li>
              );
            })}
          </ul>
        </section>
      )}

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
