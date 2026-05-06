import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockGetDocumentContent = vi.fn();
const mockGetDocumentByPath = vi.fn();
const mockRenderer = vi.fn(({ content }: { content: { content: string } }) => (
  <div data-testid="memoized-document-renderer">{content.content}</div>
));

vi.mock('../../api/client', () => ({
  apiClient: {
    get getDocumentContent() { return mockGetDocumentContent; },
    get getDocumentByPath() { return mockGetDocumentByPath; },
  },
}));

vi.mock('./renderers/index', () => ({
  pickRenderer: () => ({
    Component: mockRenderer,
    capabilities: {
      showsHistory: true,
      supportsPreviewCap: false,
      supportsRawToggle: false,
      supportsSearchHighlight: false,
    },
  }),
}));

const { DocumentViewer } = await import('./DocumentViewer');

describe('DocumentViewer memoization', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetDocumentByPath.mockResolvedValue({
      path: 'src/lib.rs',
      commit: 'abc1234def5678',
      content: 'pub fn stable() {}',
      content_type: 'text/plain',
    });
  });

  it('does not rerender the loaded document when parent state changes but effective props stay the same', async () => {
    const user = userEvent.setup();

    function Host() {
      const [tick, setTick] = useState(0);

      return (
        <div>
          <button onClick={() => setTick((value) => value + 1)}>
            Tick {tick}
          </button>
          <DocumentViewer
            documentPath="src/lib.rs"
            onClose={() => {}}
          />
        </div>
      );
    }

    render(<Host />);

    await screen.findByTestId('memoized-document-renderer');
    expect(mockRenderer).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole('button', { name: 'Tick 0' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Tick 1' })).toBeDefined();
    });
    expect(mockRenderer).toHaveBeenCalledTimes(1);
  });
});
