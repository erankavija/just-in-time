import { render, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { MermaidDiagram } from './MermaidDiagram';

type Runtime = {
  default: { render: ReturnType<typeof vi.fn>; initialize: ReturnType<typeof vi.fn> };
};

const runtime = vi.hoisted(() => {
  let resolve!: (module: Runtime) => void;
  const loaded = new Promise<Runtime>((done) => {
    resolve = done;
  });
  return { loaded, resolve };
});

vi.mock('mermaid', () => runtime.loaded);

it('test_mermaid_diagram_renders_only_current_source_after_runtime_loads', async () => {
  const { container, rerender } = render(<MermaidDiagram code="old source" />);
  rerender(<MermaidDiagram code="current source" />);
  const renderDiagram = vi.fn().mockResolvedValue({ svg: '<svg>current diagram</svg>' });

  runtime.resolve({ default: { render: renderDiagram, initialize: vi.fn() } });

  await waitFor(() => {
    expect(container.querySelector('svg')?.textContent).toBe('current diagram');
  });
  expect(renderDiagram).toHaveBeenCalledTimes(1);
  expect(renderDiagram).toHaveBeenCalledWith(expect.any(String), 'current source');
});
