import { useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import './IssueDetail.css';
import { apiClient } from '../../api/client';
import type { Issue } from '../../types/models';

interface IssueDetailProps {
  issueId: string | null;
}

export function IssueDetail({ issueId }: IssueDetailProps) {
  const [issue, setIssue] = useState<Issue | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    open: '🔵',
    ready: '🟢',
    in_progress: '🟡',
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
            remarkPlugins={[remarkMath]}
            rehypePlugins={[rehypeKatex]}
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
            📎 Documents ({issue.documents.length})
          </h2>
          <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
            {issue.documents.map((doc, idx) => (
              <li key={idx} style={{ 
                padding: '8px 10px',
                backgroundColor: 'var(--bg-tertiary)',
                marginBottom: '6px',
                borderRadius: '4px',
                fontSize: '11px',
                border: '1px solid var(--border)',
                fontFamily: 'var(--font-mono)',
              }}>
                <div style={{ 
                  fontWeight: 600,
                  color: 'var(--accent)',
                  marginBottom: '4px',
                }}>
                  {doc.path}
                </div>
                {doc.label && (
                  <div style={{ 
                    fontSize: '10px',
                    color: 'var(--text-muted)',
                    marginBottom: '2px',
                  }}>
                    {doc.label}
                  </div>
                )}
                {doc.commit && (
                  <div style={{ 
                    fontSize: '10px',
                    color: 'var(--text-muted)',
                  }}>
                    @ {doc.commit.substring(0, 7)}
                  </div>
                )}
              </li>
            ))}
          </ul>
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
    </div>
  );
}
