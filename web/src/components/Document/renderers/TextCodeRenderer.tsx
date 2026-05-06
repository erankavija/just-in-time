import { useEffect, useMemo, useState, type FC, type ReactNode } from 'react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import type { DocumentRendererProps } from './index';

type TextCodeMode = 'plain-text' | 'source-code';

interface LineRecord {
  lineNumber: number;
  text: string;
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function resolvePath(content: DocumentRendererProps['content'], documentRef?: DocumentRendererProps['documentRef']): string {
  return (documentRef?.path ?? content.path).toLowerCase();
}

function resolveMode(path: string): TextCodeMode {
  return path.endsWith('.txt') ? 'plain-text' : 'source-code';
}

function resolveLanguage(path: string): string | undefined {
  if (path.endsWith('.rs')) {
    return 'rust';
  }

  if (path.endsWith('.cpp')) {
    return 'cpp';
  }

  return undefined;
}

function splitLines(content: string): LineRecord[] {
  return content.split('\n').map((text, index) => ({
    lineNumber: index + 1,
    text,
  }));
}

function renderHighlightedPlainText(
  value: string,
  searchTerm: string | undefined,
  highlightsActive: boolean,
): ReactNode {
  if (!searchTerm || !highlightsActive) {
    return value;
  }

  const terms = searchTerm.trim().split(/\s+/).filter(Boolean);
  if (terms.length === 0) {
    return value;
  }

  const regex = new RegExp(`(${terms.map(escapeRegex).join('|')})`, 'gi');
  const normalizedTerms = new Set(terms.map((term) => term.toLowerCase()));

  return value.split(regex).map((part, index) => {
    if (part.length === 0) {
      return null;
    }

    return normalizedTerms.has(part.toLowerCase()) ? (
      <mark key={`${part}-${index}`} style={{ backgroundColor: 'rgba(255, 215, 0, 0.4)', padding: '0 2px' }}>
        {part}
      </mark>
    ) : (
      <span key={`${part}-${index}`}>{part}</span>
    );
  });
}

function lineMatchesSearch(line: string, searchTerm: string | undefined): boolean {
  if (!searchTerm) {
    return false;
  }

  return searchTerm
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .some((term) => line.toLowerCase().includes(term.toLowerCase()));
}

const TextCodeRenderer: FC<DocumentRendererProps> = ({
  content,
  documentRef,
  searchTerm,
  highlightsActive = true,
  onHighlightsCleared,
}) => {
  const [wrapPlainText, setWrapPlainText] = useState(true);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && highlightsActive) {
        onHighlightsCleared?.();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [highlightsActive, onHighlightsCleared]);

  const path = resolvePath(content, documentRef);
  const mode = resolveMode(path);
  const language = resolveLanguage(path);
  const lines = useMemo(() => splitLines(content.content), [content.content]);

  return (
    <div data-testid="text-code-renderer">
      {mode === 'plain-text' && (
        <div style={{ marginBottom: '0.75rem' }}>
          <button
            className="history-btn"
            onClick={() => setWrapPlainText((current) => !current)}
            title={wrapPlainText ? 'Disable line wrapping' : 'Enable line wrapping'}
          >
            {wrapPlainText ? 'Wrap off' : 'Wrap on'}
          </button>
        </div>
      )}

      <div>
        {lines.map((line) => {
          const isHighlighted = highlightsActive && lineMatchesSearch(line.text, searchTerm);

          return (
            <div
              key={line.lineNumber}
              data-highlighted={isHighlighted ? 'true' : 'false'}
              style={{
                display: 'grid',
                gridTemplateColumns: '56px minmax(0, 1fr)',
                alignItems: 'stretch',
                backgroundColor: isHighlighted ? 'rgba(255, 215, 0, 0.08)' : 'transparent',
              }}
            >
              <div
                style={{
                  padding: '0.125rem 0.75rem 0.125rem 0',
                  textAlign: 'right',
                  color: 'var(--text-muted)',
                  userSelect: 'none',
                  fontSize: '12px',
                }}
              >
                {line.lineNumber}
              </div>

              {mode === 'plain-text' ? (
                <div
                  data-testid={line.lineNumber === 1 ? 'plain-text-line-content' : undefined}
                  style={{
                    whiteSpace: wrapPlainText ? 'pre-wrap' : 'pre',
                    overflowX: wrapPlainText ? 'visible' : 'auto',
                    wordBreak: wrapPlainText ? 'break-word' : 'normal',
                    fontFamily: 'var(--font-mono)',
                    fontSize: '13px',
                    lineHeight: 1.5,
                  }}
                >
                  {renderHighlightedPlainText(line.text, searchTerm, highlightsActive)}
                </div>
              ) : (
                <div style={{ minWidth: 0 }}>
                  <SyntaxHighlighter
                    language={language}
                    style={vscDarkPlus}
                    PreTag="div"
                    customStyle={{
                      margin: 0,
                      padding: 0,
                      background: 'transparent',
                      fontSize: '13px',
                      minHeight: '1.5rem',
                    }}
                  >
                    {line.text.length > 0 ? line.text : ' '}
                  </SyntaxHighlighter>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};

export default TextCodeRenderer;
