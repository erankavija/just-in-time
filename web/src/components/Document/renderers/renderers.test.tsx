import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { pickRenderer, rendererRegistry } from './index';
import type { DocumentContent, DocumentReference } from '../../../types/models';
import MarkdownRenderer from './MarkdownRenderer';

function makeContent(
  content_type: string,
  content = '',
  path = 'test.md',
): DocumentContent {
  return { path, commit: 'abc1234', content, content_type };
}

function makeRef(path: string): DocumentReference {
  return { path };
}

// Mock MermaidDiagram to avoid mermaid init in jsdom
vi.mock('../../MermaidDiagram', () => ({
  MermaidDiagram: ({ code }: { code: string }) => (
    <div data-testid="mermaid-diagram" data-code={code} />
  ),
}));

describe('rendererRegistry / pickRenderer', () => {
  it('returns the markdown renderer for text/markdown', () => {
    const result = pickRenderer(makeContent('text/markdown'));
    expect(result.id).toBe('markdown');
  });

  it('returns the html renderer for text/html', () => {
    const result = pickRenderer(makeContent('text/html'));
    expect(result.id).toBe('html');
  });

  it('returns the csv renderer for .csv paths even when content-type is text/plain', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'results.csv'));
    expect(result.id).toBe('csv');
  });

  it('returns the text-code renderer for .txt paths even when content-type is text/plain', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'notes.txt'));
    expect(result.id).toBe('text-code');
  });

  it('returns the text-code renderer for .rs paths even when content-type is text/plain', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'src/lib.rs'));
    expect(result.id).toBe('text-code');
  });

  it('returns the text-code renderer for .cpp paths even when content-type is text/plain', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'benchmarks/reference/smoke.cpp'));
    expect(result.id).toBe('text-code');
  });

  it('falls back to markdown for unknown text-like extensions', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'logs/run.log'));
    expect(result.id).toBe('markdown');
  });

  it('returns the image renderer for .png paths', () => {
    const result = pickRenderer(makeContent('image/png', '', 'figures/result.png'));
    expect(result.id).toBe('image');
  });

  it('returns the image renderer for .svg paths', () => {
    const result = pickRenderer(makeContent('image/svg+xml', '', 'figures/diagram.svg'));
    expect(result.id).toBe('image');
  });

  it('returns the image renderer for .jpg paths', () => {
    const result = pickRenderer(makeContent('image/jpeg', '', 'photos/chart.jpg'));
    expect(result.id).toBe('image');
  });

  it('returns the image renderer for .jpeg paths', () => {
    const result = pickRenderer(makeContent('image/jpeg', '', 'photos/photo.jpeg'));
    expect(result.id).toBe('image');
  });

  it('returns the image renderer for .gif paths', () => {
    const result = pickRenderer(makeContent('image/gif', '', 'assets/anim.gif'));
    expect(result.id).toBe('image');
  });

  it('returns the image renderer for .webp paths', () => {
    const result = pickRenderer(makeContent('image/webp', '', 'assets/frame.webp'));
    expect(result.id).toBe('image');
  });

  it('returns the text-code renderer for .py paths', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'scripts/plot_benchmarks.py'));
    expect(result.id).toBe('text-code');
  });

  it('returns the text-code renderer for .sh paths', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'scripts/run.sh'));
    expect(result.id).toBe('text-code');
  });

  it('returns the text-code renderer for .js paths', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'scripts/helper.js'));
    expect(result.id).toBe('text-code');
  });

  it('returns the text-code renderer for .ts paths', () => {
    const result = pickRenderer(makeContent('text/plain', '', 'scripts/util.ts'));
    expect(result.id).toBe('text-code');
  });

  it('image renderer entry hides history via capability metadata', () => {
    const imageEntry = rendererRegistry.find((r) => r.id === 'image');
    expect(imageEntry).toBeDefined();
    expect(imageEntry?.capabilities.showsHistory).toBe(false);
    expect(imageEntry?.capabilities.supportsRawToggle).toBe(false);
    expect(imageEntry?.capabilities.supportsSearchHighlight).toBe(false);
  });

  it('uses documentRef.path for matching when it differs from content.path', () => {
    const result = pickRenderer(
      makeContent('text/plain', '', 'README.md'),
      makeRef('fixtures/report.csv'),
    );
    expect(result.id).toBe('csv');
  });

  it('falls back to markdown for an unknown content-type', () => {
    const result = pickRenderer(makeContent('application/unknown'));
    expect(result.id).toBe('markdown');
  });

  it('registry keeps markdown as the last catch-all renderer', () => {
    const ids = rendererRegistry.map((r) => r.id);
    expect(ids.at(-1)).toBe('markdown');
  });

  it('exposes capability metadata for rich/raw and preview-cap features', () => {
    const csvEntry = rendererRegistry.find((r) => r.id === 'csv');
    const textCodeEntry = rendererRegistry.find((r) => r.id === 'text-code');
    const htmlEntry = rendererRegistry.find((r) => r.id === 'html');

    expect(csvEntry?.capabilities.supportsRawToggle).toBe(true);
    expect(csvEntry?.capabilities.supportsPreviewCap).toBe(true);
    expect(textCodeEntry?.capabilities.supportsRawToggle).toBe(true);
    expect(textCodeEntry?.capabilities.supportsSearchHighlight).toBe(true);
    expect(htmlEntry?.capabilities.showsHistory).toBe(false);
  });
});

describe('MarkdownRenderer smoke test', () => {
  it('renders markdown headings as h1 elements', () => {
    const content = makeContent('text/markdown', '# Hello World');
    render(<MarkdownRenderer content={content} />);
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Hello World');
  });

  it('renders markdown bold as strong elements', () => {
    const content = makeContent('text/markdown', '**bold text**');
    render(<MarkdownRenderer content={content} />);
    expect(screen.getByText('bold text').tagName).toBe('STRONG');
  });

  it('renders plain text content', () => {
    const content = makeContent('text/markdown', 'Just some text');
    render(<MarkdownRenderer content={content} />);
    expect(screen.getByText('Just some text')).toBeDefined();
  });

  it('renders fenced code blocks inside a code element', () => {
    const content = makeContent('text/markdown', '```js\nconsole.log("hi");\n```');
    render(<MarkdownRenderer content={content} />);
    // SyntaxHighlighter renders a div with code; at minimum there should be a pre or div ancestor
    const codeEl = document.querySelector('code, div[class*="language-"]');
    expect(codeEl).toBeTruthy();
  });

  it('renders mermaid fenced block via MermaidDiagram component', () => {
    const content = makeContent('text/markdown', '```mermaid\ngraph TD; A-->B;\n```');
    render(<MarkdownRenderer content={content} />);
    const mermaid = screen.getByTestId('mermaid-diagram');
    expect(mermaid).toBeDefined();
    expect(mermaid.getAttribute('data-code')).toContain('graph TD');
  });
});
