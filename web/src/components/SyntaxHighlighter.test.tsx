import { render, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import SyntaxHighlighter from './SyntaxHighlighter';

describe('SyntaxHighlighter', () => {
  it.each([
    ['js', 'const answer = 42;'],
    ['ts', 'const answer: number = 42;'],
    ['rust', 'fn answer() -> u32 { 42 }'],
    ['yml', 'answer: true'],
  ])('test_syntax_highlighter_loads_language_and_alias_%s', async (language, source) => {
    const { container } = render(
      <SyntaxHighlighter language={language}>{source}</SyntaxHighlighter>,
    );

    expect(container.querySelector('code')?.textContent).toBe(source);
    await waitFor(() => {
      expect(container.querySelector('code .token')).not.toBeNull();
    });
  });

  it('test_syntax_highlighter_preserves_unknown_language_source', () => {
    const source = '<unrecognized-language> & readable text';
    const { container } = render(
      <SyntaxHighlighter language="unknown-language">{source}</SyntaxHighlighter>,
    );

    expect(container.querySelector('code')?.textContent).toBe(source);
  });
});
