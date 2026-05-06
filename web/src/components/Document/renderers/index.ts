import type React from 'react';
import type { DocumentContent, DocumentReference } from '../../../types/models';
import CsvRenderer from './CsvRenderer';
import HtmlRenderer from './HtmlRenderer';
import MarkdownRenderer from './MarkdownRenderer';
import TextCodeRenderer from './TextCodeRenderer';

export interface DocumentRendererProps {
  content: DocumentContent;
  issueId?: string;
  documentRef?: DocumentReference;
  searchTerm?: string;
  /** Whether search highlights are currently active. Passed from DocumentViewer. */
  highlightsActive?: boolean;
  /** Called by the renderer when the user clears highlights (e.g. via ESC). */
  onHighlightsCleared?: () => void;
}

export interface DocumentRendererCapabilities {
  showsHistory: boolean;
  supportsPreviewCap: boolean;
  supportsRawToggle: boolean;
  supportsSearchHighlight: boolean;
}

export interface DocumentRenderer {
  id: string;
  match: (content: DocumentContent, ref?: DocumentReference) => boolean;
  Component: React.FC<DocumentRendererProps>;
  capabilities: DocumentRendererCapabilities;
}

function resolvePath(content: DocumentContent, ref?: DocumentReference): string {
  return (ref?.path ?? content.path).toLowerCase();
}

function matchesExtension(
  content: DocumentContent,
  ref: DocumentReference | undefined,
  extensions: string[],
): boolean {
  const path = resolvePath(content, ref);
  return extensions.some((extension) => path.endsWith(`.${extension}`));
}

export const rendererRegistry: DocumentRenderer[] = [
  {
    id: 'html',
    match: (c) => c.content_type === 'text/html',
    Component: HtmlRenderer,
    capabilities: {
      showsHistory: false,
      supportsPreviewCap: false,
      supportsRawToggle: false,
      supportsSearchHighlight: false,
    },
  },
  {
    id: 'csv',
    match: (content, ref) => matchesExtension(content, ref, ['csv']),
    Component: CsvRenderer,
    capabilities: {
      showsHistory: true,
      supportsPreviewCap: true,
      supportsRawToggle: true,
      supportsSearchHighlight: true,
    },
  },
  {
    id: 'text-code',
    match: (content, ref) => matchesExtension(content, ref, ['txt', 'rs', 'cpp']),
    Component: TextCodeRenderer,
    capabilities: {
      showsHistory: true,
      supportsPreviewCap: true,
      supportsRawToggle: true,
      supportsSearchHighlight: true,
    },
  },
  {
    id: 'markdown',
    match: () => true,
    Component: MarkdownRenderer,
    capabilities: {
      showsHistory: true,
      supportsPreviewCap: false,
      supportsRawToggle: false,
      supportsSearchHighlight: true,
    },
  },
];

export function pickRenderer(content: DocumentContent, ref?: DocumentReference): DocumentRenderer {
  const found = rendererRegistry.find((r) => r.match(content, ref));
  // The last entry is a catch-all so this always returns a renderer.
  return found!;
}
