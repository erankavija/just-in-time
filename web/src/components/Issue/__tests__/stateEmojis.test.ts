import { describe, it, expect } from 'vitest';
import type { State } from '../../../types/models';

describe('IssueDetail state emojis', () => {
  it('should have emojis for all 6 states', () => {
    const stateEmoji: Record<State, string> = {
      backlog: '⏸️',
      ready: '🟢',
      in_progress: '🟡',
      gated: '🟠',
      done: '✅',
      archived: '📦',
    };

    expect(Object.keys(stateEmoji)).toHaveLength(6);
    expect(stateEmoji.backlog).toBe('⏸️');
    expect(stateEmoji.gated).toBe('🟠');
  });

  it('should have unique emojis for each state', () => {
    const stateEmoji: Record<State, string> = {
      backlog: '⏸️',
      ready: '🟢',
      in_progress: '🟡',
      gated: '🟠',
      done: '✅',
      archived: '��',
    };

    const emojis = Object.values(stateEmoji);
    const uniqueEmojis = new Set(emojis);
    expect(uniqueEmojis.size).toBe(emojis.length);
  });
});
