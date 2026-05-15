import type { FC } from 'react';
import { getRawDocumentUrl } from '../../../api/client';
import type { DocumentRendererProps } from './index';

const ImageRenderer: FC<DocumentRendererProps> = ({
  content,
  issueId,
  documentRef,
}) => {
  const src = issueId
    ? getRawDocumentUrl(issueId, content.path, content.commit ?? undefined)
    : getRawDocumentUrl(null, content.path, content.commit ?? undefined);

  return (
    <div data-testid="image-renderer">
      <img
        src={src}
        alt={documentRef?.label ?? content.path}
        style={{ maxWidth: '100%', height: 'auto', display: 'block' }}
      />
    </div>
  );
};

export default ImageRenderer;
