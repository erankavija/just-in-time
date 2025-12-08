interface LabelBadgeProps {
  label: string;
  size?: 'small' | 'normal';
}

export function LabelBadge({ label, size = 'normal' }: LabelBadgeProps) {
  // Parse label into namespace:value
  const parts = label.split(':');
  const namespace = parts[0];
  const value = parts.slice(1).join(':');

  // Color mapping for common namespaces
  const namespaceColors: Record<string, string> = {
    milestone: 'var(--info)',
    epic: 'var(--warning)',
    component: 'var(--success)',
    type: 'var(--text-muted)',
    team: 'var(--text-secondary)',
  };

  const color = namespaceColors[namespace] || 'var(--text-muted)';
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
