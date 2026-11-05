import { useEffect, useRef, memo } from 'react';

export const MermaidDiagram = memo(function MermaidDiagram({ code }: { code: string }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const id = `mermaid-${Math.random().toString(36).slice(2)}`;
    let active = true;
    import('./mermaidRuntime')
      .then(({ default: mermaid }) => active ? mermaid.render(id, code) : undefined)
      .then((result) => {
        if (active && result) el.innerHTML = result.svg;
      })
      .catch((err) => {
        if (!active) return;
        console.error('Mermaid rendering error:', err);
        el.innerHTML = `<pre style="color: var(--text-secondary); font-family: var(--font-mono); font-size: 13px; white-space: pre-wrap;">${code}</pre>`;
      });
    return () => { active = false; };
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
