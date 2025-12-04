import { describe, it, expect } from 'vitest';
import type { State } from '../../../types/models';

describe('GraphView state colors', () => {
  it('should have colors defined for all 6 states', () => {
    const stateColors: Record<State, string> = {
      backlog: 'var(--state-backlog)',
      ready: 'var(--state-ready)',
      in_progress: 'var(--state-in-progress)',
      gated: 'var(--state-gated)',
      done: 'var(--state-done)',
      archived: 'var(--state-archived)',
    };

    expect(Object.keys(stateColors)).toHaveLength(6);
    expect(stateColors.backlog).toBe('var(--state-backlog)');
    expect(stateColors.gated).toBe('var(--state-gated)');
  });
});
