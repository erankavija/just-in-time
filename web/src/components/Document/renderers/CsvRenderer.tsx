import { useEffect, useMemo, useState, type FC, type ReactNode } from 'react';
import Papa from 'papaparse';
import type { DocumentRendererProps } from './index';

type SortDirection = 'asc' | 'desc';

interface SortState {
  columnIndex: number;
  direction: SortDirection;
}

function parseCsv(content: string): string[][] {
  const parsed = Papa.parse<string[]>(content, {
    skipEmptyLines: false,
  });

  return parsed.data.map((row) => row.map((cell) => cell ?? ''));
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function renderHighlightedText(
  value: string,
  searchTerm: string | undefined,
  highlightsActive: boolean,
): ReactNode {
  if (!searchTerm || !highlightsActive) {
    return value;
  }

  const terms = searchTerm
    .trim()
    .split(/\s+/)
    .filter(Boolean);

  if (terms.length === 0) {
    return value;
  }

  const regex = new RegExp(`(${terms.map(escapeRegex).join('|')})`, 'gi');
  const parts = value.split(regex);
  const normalizedTerms = new Set(terms.map((term) => term.toLowerCase()));

  return parts.map((part, index) => {
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

function compareValues(left: string, right: string): number {
  const leftNumber = Number(left);
  const rightNumber = Number(right);
  const leftIsNumber = left.trim() !== '' && Number.isFinite(leftNumber);
  const rightIsNumber = right.trim() !== '' && Number.isFinite(rightNumber);

  if (leftIsNumber && rightIsNumber) {
    return leftNumber - rightNumber;
  }

  return left.localeCompare(right, undefined, { numeric: true, sensitivity: 'base' });
}

const CsvRenderer: FC<DocumentRendererProps> = ({
  content,
  searchTerm,
  highlightsActive = true,
  onHighlightsCleared,
}) => {
  const [sortState, setSortState] = useState<SortState | null>(null);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && highlightsActive) {
        onHighlightsCleared?.();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [highlightsActive, onHighlightsCleared]);

  const rows = useMemo(() => parseCsv(content.content), [content.content]);

  const columnCount = useMemo(
    () => rows.reduce((max, row) => Math.max(max, row.length), 0),
    [rows],
  );

  const headers = useMemo(() => {
    if (rows.length === 0) {
      return [];
    }

    const [headerRow] = rows;
    return Array.from({ length: columnCount }, (_, index) => headerRow[index] || `Column ${index + 1}`);
  }, [columnCount, rows]);

  const bodyRows = useMemo(() => rows.slice(1), [rows]);

  const sortedRows = useMemo(() => {
    if (!sortState) {
      return bodyRows;
    }

    const nextRows = [...bodyRows];
    nextRows.sort((left, right) => {
      const compared = compareValues(left[sortState.columnIndex] ?? '', right[sortState.columnIndex] ?? '');
      return sortState.direction === 'asc' ? compared : -compared;
    });
    return nextRows;
  }, [bodyRows, sortState]);

  if (rows.length === 0 || columnCount === 0) {
    return (
      <div style={{ color: 'var(--text-muted)', fontSize: '0.875rem' }}>
        Empty CSV document
      </div>
    );
  }

  return (
    <div data-testid="csv-renderer">
      <div style={{ overflowX: 'auto' }}>
        <table
          style={{
            width: '100%',
            borderCollapse: 'collapse',
            fontSize: '13px',
            tableLayout: 'auto',
          }}
        >
          <thead>
            <tr>
              {headers.map((header, columnIndex) => {
                const isSortedColumn = sortState?.columnIndex === columnIndex;
                const ariaSort = !isSortedColumn
                  ? 'none'
                  : sortState.direction === 'asc'
                    ? 'ascending'
                    : 'descending';

                return (
                  <th
                    key={`${header}-${columnIndex}`}
                    aria-sort={ariaSort}
                    onClick={() => {
                      setSortState((current) => {
                        if (current?.columnIndex !== columnIndex) {
                          return { columnIndex, direction: 'asc' };
                        }
                        return {
                          columnIndex,
                          direction: current.direction === 'asc' ? 'desc' : 'asc',
                        };
                      });
                    }}
                    style={{
                      position: 'sticky',
                      top: 0,
                      zIndex: 1,
                      cursor: 'pointer',
                      textAlign: 'left',
                      padding: '0.75rem',
                      backgroundColor: 'var(--bg-tertiary)',
                      borderBottom: '1px solid var(--border)',
                      color: 'var(--text-primary)',
                    }}
                  >
                    {header}
                    {isSortedColumn && (
                      <span style={{ marginLeft: '0.5rem', color: 'var(--text-muted)' }}>
                        {sortState.direction === 'asc' ? '↑' : '↓'}
                      </span>
                    )}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {sortedRows.map((row, rowIndex) => (
              <tr key={`row-${rowIndex}`}>
                {Array.from({ length: columnCount }, (_, columnIndex) => (
                  <td
                    key={`cell-${rowIndex}-${columnIndex}`}
                    style={{
                      padding: '0.75rem',
                      borderBottom: '1px solid var(--border)',
                      color: 'var(--text-secondary)',
                      verticalAlign: 'top',
                    }}
                  >
                    {renderHighlightedText(
                      row[columnIndex] ?? '',
                      searchTerm,
                      highlightsActive,
                    )}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default CsvRenderer;
