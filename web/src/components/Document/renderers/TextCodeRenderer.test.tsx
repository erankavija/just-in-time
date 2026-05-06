import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentContent } from '../../../types/models';
import TextCodeRenderer from './TextCodeRenderer';

Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
  value: vi.fn(),
  writable: true,
});

function makeContent(path: string, content: string): DocumentContent {
  return {
    path,
    commit: 'abc1234',
    content,
    content_type: 'text/plain',
  };
}

describe('TextCodeRenderer', () => {
  it('renders Rust source with line numbers', () => {
    const content = makeContent('src/lib.rs', [
      'pub fn demo() {',
      '    println!("hello");',
      '}',
    ].join('\n'));

    render(<TextCodeRenderer content={content} documentRef={{ path: 'src/lib.rs' }} />);

    expect(screen.getByTestId('text-code-renderer')).toBeDefined();
    expect(screen.getByTestId('text-code-renderer').textContent).toContain('pub fn demo() {');
    expect(screen.getByText('1')).toBeDefined();
    expect(screen.getByText('2')).toBeDefined();
  });

  it('renders source code inside a single horizontally scrollable block', () => {
    const content = makeContent('src/lib.rs', [
      'pub fn demo() {',
      '    let very_long_identifier_name_that_should_scroll = "value";',
      '}',
    ].join('\n'));

    render(<TextCodeRenderer content={content} documentRef={{ path: 'src/lib.rs' }} />);

    expect(screen.getByTestId('source-code-scroll-region')).toHaveStyle({
      overflowX: 'auto',
      fontSize: '13px',
    });
  });

  it('renders plain text with wrapping controls and line numbers', async () => {
    const user = userEvent.setup();
    const content = makeContent('notes.txt', 'alpha beta gamma');

    render(<TextCodeRenderer content={content} documentRef={{ path: 'notes.txt' }} />);

    expect(screen.getByRole('button', { name: 'Wrap off' })).toBeDefined();
    expect(screen.getByTestId('plain-text-line-content')).toHaveStyle({
      whiteSpace: 'pre-wrap',
      fontSize: '13px',
    });

    await user.click(screen.getByRole('button', { name: 'Wrap off' }));

    expect(screen.getByRole('button', { name: 'Wrap on' })).toBeDefined();
    expect(screen.getByTestId('plain-text-line-content')).toHaveStyle({ whiteSpace: 'pre' });
  });

  it('highlights matching plain-text content', () => {
    const content = makeContent('notes.txt', 'alpha beta gamma');

    render(
      <TextCodeRenderer
        content={content}
        documentRef={{ path: 'notes.txt' }}
        searchTerm="beta"
        highlightsActive={true}
      />,
    );

    const marks = document.querySelectorAll('mark');
    expect(marks).toHaveLength(1);
    expect(marks[0]).toHaveTextContent('beta');
  });

  it('highlights matching source-code lines', () => {
    const content = makeContent('src/lib.rs', [
      'pub fn demo() {',
      '    let needle_value = 42;',
      '}',
    ].join('\n'));

    render(
      <TextCodeRenderer
        content={content}
        documentRef={{ path: 'src/lib.rs' }}
        searchTerm="needle_value"
        highlightsActive={true}
      />,
    );

    const highlightedLines = document.querySelectorAll('[data-highlighted="true"]');
    expect(highlightedLines.length).toBeGreaterThan(0);
  });

  it('renders only the capped preview content provided by the viewer shell', () => {
    const content = makeContent(
      'notes.txt',
      Array.from({ length: 1000 }, (_, idx) => `line-${idx}`).join('\n'),
    );

    render(
      <TextCodeRenderer
        content={content}
        documentRef={{ path: 'notes.txt' }}
        previewState={{ isCapped: true, kind: 'lines', maxItems: 1000, totalItems: 1020 }}
      />,
    );

    expect(screen.getByText('line-999')).toBeDefined();
    expect(screen.queryByText('line-1019')).toBeNull();
  });
});
