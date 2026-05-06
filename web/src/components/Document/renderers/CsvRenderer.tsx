import type { FC } from 'react';
import type { DocumentRendererProps } from './index';
import MarkdownRenderer from './MarkdownRenderer';

const CsvRenderer: FC<DocumentRendererProps> = (props) => (
  <MarkdownRenderer {...props} />
);

export default CsvRenderer;
