import Papa from 'papaparse';
import type { DocumentContent } from '../../../types/models';
import type { DocumentPreviewState } from './index';
import { DEFAULT_PREVIEW_MAX_ITEMS } from './constants';

export const CSV_PREVIEW_MAX_ROWS = DEFAULT_PREVIEW_MAX_ITEMS;

export function parseCsv(content: string): string[][] {
  const parsed = Papa.parse<string[]>(content, {
    skipEmptyLines: false,
  });

  return parsed.data.map((row) => row.map((cell) => cell ?? ''));
}

export function buildCsvPreviewState(content: DocumentContent): DocumentPreviewState {
  const rows = parseCsv(content.content);
  const totalItems = Math.max(rows.length - 1, 0);

  return {
    isCapped: totalItems > CSV_PREVIEW_MAX_ROWS,
    kind: 'rows',
    maxItems: CSV_PREVIEW_MAX_ROWS,
    totalItems,
  };
}

export function buildCsvPreviewContent(
  content: DocumentContent,
  previewState: DocumentPreviewState,
): DocumentContent {
  if (!previewState.isCapped) {
    return content;
  }

  const rows = parseCsv(content.content);
  const [headerRow = []] = rows;
  const cappedRows = [headerRow, ...rows.slice(1, previewState.maxItems + 1)];

  return {
    ...content,
    content: Papa.unparse(cappedRows),
  };
}
