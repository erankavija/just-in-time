/**
 * @vitest-environment node
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

describe('CSS state variables', () => {
  const cssContent = readFileSync(join(__dirname, '../index.css'), 'utf-8');

  it('should define --state-backlog variable', () => {
    expect(cssContent).toContain('--state-backlog:');
  });

  it('should define --state-gated variable', () => {
    expect(cssContent).toContain('--state-gated:');
  });

  it('should not define --state-open variable', () => {
    const openMatches = cssContent.match(/--state-open:/g);
    expect(openMatches).toBeNull();
  });

  it('should define all 6 state colors', () => {
    expect(cssContent).toContain('--state-backlog:');
    expect(cssContent).toContain('--state-ready:');
    expect(cssContent).toContain('--state-in-progress:');
    expect(cssContent).toContain('--state-gated:');
    expect(cssContent).toContain('--state-done:');
    expect(cssContent).toContain('--state-archived:');
  });
});
