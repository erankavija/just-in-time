import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { LabelBadge } from '../LabelBadge';

describe('LabelBadge', () => {
  it('test_label_badge_uses_api_namespace_order_for_configured_colors', () => {
    const { container } = render(
      <LabelBadge
        label="custom:one"
        namespaces={{
          custom: { description: 'Custom labels', unique: false },
          alpha: { description: 'Alpha labels', unique: false },
        }}
      />
    );

    expect(container.firstElementChild?.getAttribute('style')).toContain(
      'border: 1px solid var(--warning)'
    );
  });

  it('test_label_badge_uses_muted_color_for_unknown_namespace', () => {
    const { container } = render(
      <LabelBadge
        label="unknown:value"
        namespaces={{
          custom: { description: 'Custom labels', unique: false },
        }}
      />
    );

    expect(container.firstElementChild?.getAttribute('style')).toContain(
      'border: 1px solid var(--text-muted)'
    );
  });
});
