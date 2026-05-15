import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import ImageRenderer from './ImageRenderer';
import type { DocumentContent, DocumentReference } from '../../../types/models';

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
    path: 'dev/benchmarks/figures/result.png',
    commit: 'abc1234',
    content: '',
    content_type: 'image/png',
    ...overrides,
  };
}

function makeRef(overrides?: Partial<DocumentReference>): DocumentReference {
  return {
    path: 'dev/benchmarks/figures/result.png',
    label: 'Benchmark Figure',
    ...overrides,
  };
}

describe('ImageRenderer', () => {
  it('renders an img element with the raw URL as src', () => {
    render(
      <ImageRenderer
        content={makeContent()}
        issueId="issue-1"
        documentRef={makeRef()}
      />,
    );
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img).toBeTruthy();
    expect(img.getAttribute('src')).toMatch(
      /\/api\/issues\/issue-1\/documents\/dev%2Fbenchmarks%2Ffigures%2Fresult\.png\/raw/,
    );
  });

  it('appends ?commit=… when content.commit is set', () => {
    render(
      <ImageRenderer
        content={makeContent({ commit: 'deadbeef' })}
        issueId="issue-2"
        documentRef={makeRef()}
      />,
    );
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img.getAttribute('src')).toContain('?commit=deadbeef');
  });

  it('falls back to path-only URL when issueId is absent', () => {
    render(
      <ImageRenderer
        content={makeContent({ path: 'shared/plot.svg' })}
        documentRef={makeRef({ path: 'shared/plot.svg' })}
      />,
    );
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img.getAttribute('src')).toMatch(/\/api\/documents\/raw\?path=shared%2Fplot\.svg/);
  });

  it('uses documentRef.label as alt text', () => {
    render(
      <ImageRenderer
        content={makeContent()}
        issueId="issue-1"
        documentRef={makeRef({ label: 'My Figure' })}
      />,
    );
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img.getAttribute('alt')).toBe('My Figure');
  });

  it('falls back to content.path as alt text when documentRef is absent', () => {
    render(
      <ImageRenderer
        content={makeContent({ path: 'figures/chart.png' })}
        issueId="issue-1"
      />,
    );
    const img = document.querySelector('img') as HTMLImageElement;
    expect(img.getAttribute('alt')).toBe('figures/chart.png');
  });

  it('renders the image-renderer testid wrapper', () => {
    render(<ImageRenderer content={makeContent()} issueId="issue-1" />);
    expect(screen.getByTestId('image-renderer')).toBeDefined();
  });
});
