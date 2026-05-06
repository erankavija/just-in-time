import type { FC } from 'react';
import type { DocumentRendererProps } from './index';
import MarkdownRenderer from './MarkdownRenderer';

const TextCodeRenderer: FC<DocumentRendererProps> = (props) => (
  <MarkdownRenderer {...props} />
);

export default TextCodeRenderer;
