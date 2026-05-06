import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { DocumentContent } from '../../../types/models';
import CsvRenderer from './CsvRenderer';
import {
  CSV_PREVIEW_MAX_ROWS,
  buildCsvPreviewContent,
  buildCsvPreviewState,
} from './CsvRenderer.helpers';

Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
  value: vi.fn(),
  writable: true,
});

function makeContent(content: string): DocumentContent {
  return {
    path: 'results.csv',
    commit: 'abc1234',
    content,
    content_type: 'text/csv',
  };
}

describe('CsvRenderer', () => {
  it('renders parsed headers and rows in a table', () => {
    const content = makeContent([
      'prime,n,kernel',
      '31,1024,C',
      '7,256,F',
    ].join('\n'));

    render(<CsvRenderer content={content} />);

    expect(screen.getByRole('table')).toHaveStyle({ fontSize: '13px' });
    expect(screen.getByText('prime')).toBeDefined();
    expect(screen.getByText('1024')).toBeDefined();
    expect(screen.getByText('F')).toBeDefined();
  });

  it('sorts rows by the clicked column', async () => {
    const user = userEvent.setup();
    const content = makeContent([
      'prime,n,kernel',
      '31,1024,C',
      '7,256,F',
    ].join('\n'));

    render(<CsvRenderer content={content} />);

    await user.click(screen.getByRole('columnheader', { name: 'n' }));

    const rows = screen.getAllByRole('row');
    expect(rows[1]).toHaveTextContent('7256F');
    expect(rows[2]).toHaveTextContent('311024C');
  });

  it('highlights matching cells for the active search query', () => {
    const content = makeContent([
      'prime,n,kernel',
      '31,1024,C',
      '7,256,F',
    ].join('\n'));

    render(
      <CsvRenderer
        content={content}
        searchTerm="1024"
        highlightsActive={true}
      />,
    );

    const marks = document.querySelectorAll('mark');
    expect(marks).toHaveLength(1);
    expect(marks[0]).toHaveTextContent('1024');
  });

  it('renders sticky header cells for scrolling tables', () => {
    const content = makeContent([
      'prime,n,kernel',
      '31,1024,C',
      '7,256,F',
    ].join('\n'));

    render(<CsvRenderer content={content} />);

    expect(screen.getByRole('columnheader', { name: 'prime' })).toHaveStyle({
      position: 'sticky',
    });
  });

  it('builds a capped preview on full CSV rows without breaking multiline cells', () => {
    const content = makeContent([
      'prime,notes',
      '7,"line one\nline two"',
      ...Array.from({ length: 1005 }, (_, idx) => `${idx + 10},value-${idx}`),
    ].join('\n'));

    const previewState = buildCsvPreviewState(content);
    const previewContent = buildCsvPreviewContent(content, previewState);

    render(
      <CsvRenderer
        content={previewContent}
        previewState={previewState}
      />,
    );

    expect(previewState.isCapped).toBe(true);
    expect(previewState.kind).toBe('rows');
    expect(previewState.maxItems).toBe(CSV_PREVIEW_MAX_ROWS);
    expect(screen.getByText('line one line two')).toBeDefined();
    expect(screen.queryByText('value-1004')).toBeNull();
  });
});
