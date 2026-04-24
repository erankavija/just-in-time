import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import HtmlRenderer from './HtmlRenderer';
import type { DocumentContent, DocumentReference } from '../../../types/models';

// Mock getRawDocumentUrl so tests are deterministic and don't rely on window.location.
vi.mock('../../../api/client', () => ({
  getRawDocumentUrl: (
    issueId: string | null | undefined,
    path: string,
    commit?: string,
  ): string => {
    if (issueId) {
      const encodedPath = encodeURIComponent(path);
      const base = `/api/issues/${issueId}/documents/${encodedPath}/raw`;
      return commit ? `${base}?commit=${encodeURIComponent(commit)}` : base;
    }
    const params = new URLSearchParams({ path });
    if (commit) params.set('commit', commit);
    return `/api/documents/raw?${params.toString()}`;
  },
}));

function makeContent(overrides?: Partial<DocumentContent>): DocumentContent {
  return {
    path: 'docs/deck.html',
    commit: 'abc1234',
    content: '',
    content_type: 'text/html',
    ...overrides,
  };
}

function makeRef(overrides?: Partial<DocumentReference>): DocumentReference {
  return {
    path: 'docs/deck.html',
    label: 'My Presentation',
    ...overrides,
  };
}

describe('HtmlRenderer', () => {
  it('renders a single iframe with correct sandbox attribute', () => {
    render(
      <HtmlRenderer
        content={makeContent()}
        issueId="issue-1"
        documentRef={makeRef()}
      />,
    );
    const iframes = document.querySelectorAll('iframe');
    expect(iframes).toHaveLength(1);

    const sandbox = iframes[0].getAttribute('sandbox') ?? '';
    expect(sandbox).toContain('allow-scripts');
    expect(sandbox).toContain('allow-same-origin');
    expect(sandbox).toContain('allow-popups');
  });

  it('iframe src points at raw endpoint', () => {
    render(
      <HtmlRenderer
        content={makeContent()}
        issueId="issue-42"
        documentRef={makeRef()}
      />,
    );
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    // jsdom prepends origin, so we check the pathname+search portion.
    expect(iframe.getAttribute('src')).toMatch(
      /\/api\/issues\/issue-42\/documents\/docs%2Fdeck\.html\/raw/,
    );
  });

  it('appends ?commit=… when content.commit is set', () => {
    render(
      <HtmlRenderer
        content={makeContent({ commit: 'deadbeef' })}
        issueId="issue-99"
        documentRef={makeRef()}
      />,
    );
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    const link = screen.getByText('Open in new tab');

    expect(iframe.getAttribute('src')).toContain('?commit=deadbeef');
    expect(link.getAttribute('href')).toContain('?commit=deadbeef');
  });

  it('"Open in new tab" link has target _blank and rel noopener', () => {
    render(
      <HtmlRenderer
        content={makeContent()}
        issueId="issue-1"
        documentRef={makeRef()}
      />,
    );
    const link = screen.getByText('Open in new tab');
    expect(link.getAttribute('target')).toBe('_blank');
    // rel must include both tokens (order may vary)
    const rel = link.getAttribute('rel') ?? '';
    expect(rel).toContain('noopener');
  });

  it('does NOT render search highlights', () => {
    render(
      <HtmlRenderer
        content={makeContent()}
        issueId="issue-1"
        documentRef={makeRef()}
        searchTerm="hello"
        highlightsActive={true}
      />,
    );
    const marks = document.querySelectorAll('mark');
    expect(marks).toHaveLength(0);
  });

  it('falls back to path-only URL when issueId is absent', () => {
    render(
      <HtmlRenderer
        content={makeContent({ path: 'shared/file.html' })}
        // issueId intentionally omitted
        documentRef={makeRef({ path: 'shared/file.html' })}
      />,
    );
    const iframe = document.querySelector('iframe') as HTMLIFrameElement;
    const link = screen.getByText('Open in new tab');

    const expectedPattern = /\/api\/documents\/raw\?path=shared%2Ffile\.html/;
    expect(iframe.getAttribute('src')).toMatch(expectedPattern);
    expect(link.getAttribute('href')).toMatch(expectedPattern);
  });

  it('uses path as label when documentRef has no label', () => {
    render(
      <HtmlRenderer
        content={makeContent()}
        issueId="issue-1"
        documentRef={{ path: 'docs/deck.html' }}
      />,
    );
    expect(screen.getByText('docs/deck.html')).toBeDefined();
  });

  it('uses content.path as label when documentRef is absent', () => {
    render(
      <HtmlRenderer
        content={makeContent({ path: 'no-ref.html' })}
        issueId="issue-1"
      />,
    );
    expect(screen.getByText('no-ref.html')).toBeDefined();
  });
});
