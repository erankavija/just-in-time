import type React from 'react';
import type { DocumentContent, DocumentReference } from '../../../types/models';
import HtmlRenderer from './HtmlRenderer';
import MarkdownRenderer from './MarkdownRenderer';

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

export interface DocumentRenderer {
  id: string;
  match: (content: DocumentContent, ref?: DocumentReference) => boolean;
  Component: React.FC<DocumentRendererProps>;
  /**
   * When true, DocumentViewer should suppress the history panel and history
   * button for this renderer (e.g. HtmlRenderer renders in an iframe, so
   * commit history navigation is not meaningful).
   */
  noHistory?: boolean;
}

export const rendererRegistry: DocumentRenderer[] = [
  {
    id: 'html',
    match: (c) => c.content_type === 'text/html',
    Component: HtmlRenderer,
    noHistory: true,
  },
  {
    id: 'markdown',
    match: () => true,
    Component: MarkdownRenderer,
  },
];

export function pickRenderer(content: DocumentContent, ref?: DocumentReference): DocumentRenderer {
  const found = rendererRegistry.find((r) => r.match(content, ref));
  // The last entry is a catch-all so this always returns a renderer.
  return found!;
}
