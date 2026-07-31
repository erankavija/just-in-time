import type { NamespaceInfo } from '../../types/models';

interface LabelBadgeProps {
  label: string;
  size?: 'small' | 'normal';
  namespaces?: Record<string, NamespaceInfo>;
}

const NAMESPACE_COLORS = [
  'var(--info)',
  'var(--warning)',
  'var(--success)',
  'var(--text-secondary)',
  'var(--accent)',
];

function getNamespaceColor(namespace: string, namespaces?: Record<string, NamespaceInfo>): string {
  if (!namespaces || !(namespace in namespaces)) {
    return 'var(--text-muted)';
  }

  const namespaceIndex = Object.keys(namespaces).sort().indexOf(namespace);
  return NAMESPACE_COLORS[namespaceIndex % NAMESPACE_COLORS.length];
}

export function LabelBadge({ label, size = 'normal', namespaces }: LabelBadgeProps) {
  // Parse label into namespace:value
  const parts = label.split(':');
  const namespace = parts[0];
  const value = parts.slice(1).join(':');

  const color = getNamespaceColor(namespace, namespaces);
  const fontSize = size === 'small' ? '9px' : '10px';
  const padding = size === 'small' ? '2px 5px' : '3px 6px';

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: '3px',
        backgroundColor: 'var(--bg-primary)',
        border: `1px solid ${color}`,
        borderRadius: '4px',
        padding,
        fontSize,
        fontFamily: 'var(--font-mono)',
        color,
        whiteSpace: 'nowrap',
      }}
    >
      <span style={{ opacity: 0.7 }}>{namespace}</span>
      <span style={{ fontWeight: 600 }}>{value}</span>
    </span>
  );
}
