import { useEffect, useRef, useState, type ComponentProps } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkMath from 'remark-math';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import 'katex/dist/katex.min.css';
import { MermaidDiagram } from '../../MermaidDiagram';
import type { DocumentRendererProps } from './index';

// Stable module-level constants — prevents ReactMarkdown from seeing new references
// on every re-render, which would unmount/remount MermaidDiagram and cause flashing.
const REMARK_PLUGINS = [remarkMath, remarkGfm];
const REHYPE_PLUGINS = [rehypeKatex];
const MD_COMPONENTS: ComponentProps<typeof ReactMarkdown>['components'] = {
  code({ className, children, ...props }) {
    const match = /language-(\w+)/.exec(className || '');
    const language = match ? match[1] : '';
    const isInline = !className;

    if (language === 'mermaid') {
      return <MermaidDiagram code={String(children).replace(/\n$/, '')} />;
    }

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
};

export default function MarkdownRenderer({ content, searchTerm }: DocumentRendererProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  // Persisted across rerenders so that ESC-to-clear stays cleared when content updates.
  const [highlightsActive, setHighlightsActive] = useState(true);

  // Handle ESC key to clear highlights
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && highlightsActive) {
        setHighlightsActive(false);
        if (containerRef.current) {
          const marks = containerRef.current.querySelectorAll('mark');
          marks.forEach(mark => {
            const text = mark.textContent;
            const textNode = document.createTextNode(text || '');
            mark.parentNode?.replaceChild(textNode, mark);
          });
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [highlightsActive]);

  // Highlight search term in rendered content
  useEffect(() => {
    if (searchTerm && highlightsActive && containerRef.current) {
      setTimeout(() => {
        const container = containerRef.current;
        if (!container) return;

        const searchTerms = searchTerm.trim().split(/\s+/);

        const highlightText = (node: Node) => {
          if (node.nodeType === Node.TEXT_NODE) {
            const text = node.textContent || '';
            let hasMatch = false;
            let html = text;

            searchTerms.forEach(term => {
              const regex = new RegExp(`(${term})`, 'gi');
              if (regex.test(text)) {
                hasMatch = true;
                html = html.replace(regex, '<mark style="background-color: rgba(255, 215, 0, 0.4); padding: 0 2px;">$1</mark>');
              }
            });

            if (hasMatch && node.parentNode) {
              const span = document.createElement('span');
              span.innerHTML = html;
              node.parentNode.replaceChild(span, node);

              // Scroll to first match
              const firstMark = container.querySelector('mark');
              if (firstMark) {
                firstMark.scrollIntoView({ behavior: 'smooth', block: 'center' });
              }
            }
          } else if (node.nodeType === Node.ELEMENT_NODE) {
            // Don't highlight inside code blocks or other special elements
            const element = node as Element;
            if (!element.matches('code, pre, script, style')) {
              Array.from(node.childNodes).forEach(highlightText);
            }
          }
        };

        highlightText(container);
      }, 500);
    }
  }, [searchTerm, highlightsActive, content]);

  return (
    <div ref={containerRef}>
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        rehypePlugins={REHYPE_PLUGINS}
        components={MD_COMPONENTS}
      >
        {content.content}
      </ReactMarkdown>
    </div>
  );
}
