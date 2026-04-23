import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { pickRenderer, rendererRegistry } from './index';
import type { DocumentContent } from '../../../types/models';
import MarkdownRenderer from './MarkdownRenderer';

function makeContent(content_type: string, content = ''): DocumentContent {
  return { path: 'test.md', commit: 'abc1234', content, content_type };
}

describe('rendererRegistry / pickRenderer', () => {
  it('returns the markdown renderer for text/markdown', () => {
    const result = pickRenderer(makeContent('text/markdown'));
    expect(result.id).toBe('markdown');
  });

  it('returns the html renderer for text/html', () => {
    const result = pickRenderer(makeContent('text/html'));
    expect(result.id).toBe('html');
  });

  it('falls back to markdown for an unknown content-type', () => {
    const result = pickRenderer(makeContent('application/unknown'));
    expect(result.id).toBe('markdown');
  });

  it('registry has html before markdown (html first, markdown catch-all last)', () => {
    const ids = rendererRegistry.map((r) => r.id);
    expect(ids.indexOf('html')).toBeLessThan(ids.indexOf('markdown'));
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
});
