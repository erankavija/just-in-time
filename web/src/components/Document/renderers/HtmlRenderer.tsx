import type { FC } from 'react';
import { getRawDocumentUrl } from '../../../api/client';
import type { DocumentRendererProps } from './index';
import styles from './HtmlRenderer.module.css';

const HtmlRenderer: FC<DocumentRendererProps> = ({
  content,
  issueId,
  documentRef,
  // searchTerm, highlightsActive, and onHighlightsCleared are intentionally
  // unused: search highlighting and the history panel are not meaningful
  // through an iframe boundary.
}) => {
  const label = documentRef?.label ?? content.path;
  const rawUrl = issueId
    ? getRawDocumentUrl(issueId, content.path, content.commit ?? undefined)
    : getRawDocumentUrl(null, content.path, content.commit ?? undefined);

  return (
    <div className={styles.htmlRenderer}>
      <div className={styles.toolbar}>
        <div className={styles.toolbarTitle}>
          <span className={styles.icon}>🌐</span>
          <span className={styles.label}>{label}</span>
        </div>
        <div className={styles.toolbarActions}>
          <span className={styles.contentTypeBadge}>{content.content_type}</span>
          <a
            href={rawUrl}
            target="_blank"
            rel="noopener noreferrer"
            className={styles.openInNewTab}
            aria-label="Open document in new tab"
          >
            Open in new tab
          </a>
        </div>
      </div>
      <iframe
        src={rawUrl}
        sandbox="allow-scripts allow-same-origin allow-popups"
        title={label}
        className={styles.iframe}
      />
    </div>
  );
};

export default HtmlRenderer;
