import { useEffect, useRef, memo } from 'react';
import mermaid from 'mermaid';

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

export const MermaidDiagram = memo(function MermaidDiagram({ code }: { code: string }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const id = `mermaid-${Math.random().toString(36).slice(2)}`;
    mermaid.render(id, code).then(({ svg }) => {
      if (ref.current) ref.current.innerHTML = svg;
    }).catch((err) => {
      console.error('Mermaid rendering error:', err);
      if (ref.current) {
        ref.current.innerHTML = `<pre style="color: var(--text-secondary); font-family: var(--font-mono); font-size: 13px; white-space: pre-wrap;">${code}</pre>`;
      }
    });
  }, [code]);

  return (
    <div
      ref={ref}
      style={{
        backgroundColor: 'var(--bg-secondary)',
        padding: '20px',
        borderRadius: '6px',
        overflow: 'auto',
        margin: '1em 0',
      }}
    />
  );
});
