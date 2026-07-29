import { describe, it, expect, beforeAll, afterEach, vi } from 'vitest';
import { render, waitFor } from '@testing-library/react';
import { MermaidDiagram } from './MermaidDiagram';

// Exercises the real `mermaid` module rather than a stub: this is the only
// coverage of the diagram pipeline the component actually depends on — the
// parser that turns diagram source into an AST and the renderer that turns it
// into SVG. Every other suite stubs MermaidDiagram to isolate its own subject.
beforeAll(() => {
  // jsdom ships no SVG layout engine, so text and shape measurement return
  // nothing. Mermaid measures every label to size its nodes; fixed dimensions
  // let layout complete without changing what gets parsed or emitted.
  Object.defineProperty(SVGElement.prototype, 'getBBox', {
    configurable: true,
    value: () => ({ x: 0, y: 0, width: 100, height: 20 }),
  });
  Object.defineProperty(SVGElement.prototype, 'getComputedTextLength', {
    configurable: true,
    value: () => 100,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('MermaidDiagram', () => {
  it('renders diagram source into SVG carrying the declared node labels', async () => {
    const { container } = render(<MermaidDiagram code={'graph TD;\n  Alpha-->Beta;'} />);

    await waitFor(() => {
      const svg = container.querySelector('svg');
      expect(svg).not.toBeNull();
      expect(svg?.textContent).toContain('Alpha');
      expect(svg?.textContent).toContain('Beta');
    });
  });

  it('renders a flowchart edge between the two declared nodes', async () => {
    const { container } = render(<MermaidDiagram code={'graph LR;\n  Start-->Finish;'} />);

    await waitFor(() => {
      const svg = container.querySelector('svg');
      expect(svg).not.toBeNull();
      expect(svg?.querySelectorAll('path').length).toBeGreaterThan(0);
    });
  });

  it('falls back to the diagram source when the source does not parse', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const code = 'graph TD;\n  A --> ;;;((';

    const { container } = render(<MermaidDiagram code={code} />);

    await waitFor(() => {
      const fallback = container.querySelector('pre');
      expect(fallback).not.toBeNull();
      expect(fallback?.textContent).toBe(code);
    });
    expect(container.querySelector('svg')).toBeNull();
  });

  it('replaces the rendered diagram when the source changes', async () => {
    const { container, rerender } = render(<MermaidDiagram code={'graph TD;\n  First-->Second;'} />);

    await waitFor(() => {
      expect(container.querySelector('svg')?.textContent).toContain('First');
    });

    rerender(<MermaidDiagram code={'graph TD;\n  Third-->Fourth;'} />);

    await waitFor(() => {
      const svg = container.querySelector('svg');
      expect(svg?.textContent).toContain('Third');
      expect(svg?.textContent).not.toContain('First');
    });
  });
});
