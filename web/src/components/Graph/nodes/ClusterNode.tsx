import { memo } from 'react';
import { Handle, Position, type NodeProps } from 'reactflow';
import type { DownstreamStats } from '../../../utils/strategicView';
import './ClusterNode.css';

/**
 * Data passed to cluster container node.
 * Hierarchy-agnostic - doesn't assume specific type names.
 */
export interface ClusterNodeData {
  /** Container node label/title */
  label: string;
  
  /** Icon for this type (optional) */
  icon?: string;
  
  /** Type name (e.g., "epic", "story") */
  typeName?: string;
  
  /** Whether this cluster is expanded (true) or collapsed (false) */
  isExpanded: boolean;
  
  /** Number of nodes hidden when collapsed */
  hiddenNodeCount: number;
  
  /** Callback when header is clicked to toggle expansion */
  onToggleExpansion: () => void;
  
  /** Original node data for styling */
  state?: string;
  priority?: string;
  labels?: string[];
  nodeId?: string;
  
  /** Whether this is a strategic node */
  isStrategic?: boolean;
  
  /** Downstream task statistics (only for strategic nodes) */
  downstreamStats?: DownstreamStats;
}

const stateColors: Record<string, string> = {
  backlog: 'var(--state-backlog)',
  ready: 'var(--state-ready)',
  in_progress: 'var(--state-in-progress)',
  gated: 'var(--state-gated)',
  done: 'var(--state-done)',
  rejected: 'var(--state-rejected)',
  archived: 'var(--state-archived)',
};

const priorityColors: Record<string, string> = {
  critical: 'var(--error)',
  high: 'var(--warning)',
  normal: 'var(--info)',
  low: 'var(--success)',
};

/**
 * Cluster container node component with collapse/expand functionality.
 * 
 * When expanded: Acts as visual container, shows all child nodes inside
 * When collapsed: Shows just this node with badge indicating hidden count
 */
const ClusterNode = memo(({ data }: NodeProps<ClusterNodeData>) => {
  const {
    label,
    icon,
    typeName,
    isExpanded,
    hiddenNodeCount,
    onToggleExpansion,
    state = 'backlog',
    priority = 'normal',
    nodeId = '',
    isStrategic = false,
    downstreamStats,
  } = data;

  const collapseIcon = isExpanded ? '⊟' : '⊞';
  const title = isExpanded
    ? 'Click to collapse cluster'
    : `Click to expand cluster (${hiddenNodeCount} hidden)`;

  const borderColor = state ? stateColors[state] : 'var(--border-color)';

  // Render exactly like a regular issue node, just with expand/collapse button
  return (
    <div style={{
      background: 'var(--bg-tertiary)',
      border: `2px solid ${borderColor}`,
      borderRadius: '8px',
      padding: 0,
      fontFamily: 'var(--font-mono)',
      fontSize: '12px',
      position: 'relative',
      boxShadow: '0 0 0 1px rgb(0 0 0 / 10%), 0 4px 6px -2px rgb(0 0 0 / 10%)',
    }}>
      {/* Header with state color background */}
      <div style={{ 
        fontSize: '10px', 
        color: 'rgba(255, 255, 255, 0.9)',
        padding: '6px 10px',
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        background: borderColor,
        borderTopLeftRadius: '6px',
        borderTopRightRadius: '6px',
      }}>
        <span style={{ opacity: 0.85 }}>
          {icon && `${icon} `}
          {typeName && `${typeName.charAt(0).toUpperCase() + typeName.slice(1)} `}
          #{nodeId.substring(0, 8)}
        </span>
        <button
          className="cluster-node-toggle-button-compact"
          onClick={(e) => {
            e.stopPropagation();
            onToggleExpansion();
          }}
          title={title}
          aria-label={title}
          style={{ 
            color: 'rgba(255, 255, 255, 0.85)',
            display: 'flex',
            alignItems: 'center',
            gap: '6px',
          }}
        >
          {!isExpanded && hiddenNodeCount > 0 && (
            <span style={{ 
              fontSize: '10px',
              fontWeight: 600,
              opacity: 0.95,
            }}>
              {hiddenNodeCount}
            </span>
          )}
          <span style={{ 
            lineHeight: '1', 
            display: 'inline-block',
            verticalAlign: 'middle',
            transform: 'translateY(-1px)'
          }}>
            {collapseIcon}
          </span>
        </button>
      </div>
      {/* Content */}
      <div style={{ padding: '10px 12px' }}>
        <div style={{ 
          fontWeight: 600,
          marginBottom: '6px',
          color: 'var(--text-primary)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          maxWidth: '180px',
        }}>
          {label}
        </div>
        <div style={{ 
          fontSize: '11px',
          display: 'flex',
          gap: '8px',
          alignItems: 'center',
        }}>
          <span style={{ color: priorityColors[priority] }}>
            ● {priority}
          </span>
          <span style={{ color: 'var(--text-secondary)' }}>
            | {state}
          </span>
        </div>
        {isStrategic && downstreamStats && (
          <div 
            style={{
              fontSize: '10px',
              color: 'var(--text-secondary)',
              fontFamily: 'var(--font-mono)',
              borderTop: '1px solid var(--border)',
              paddingTop: '4px',
              marginTop: '6px',
            }}
            title={`Downstream: ${downstreamStats.total} tasks (${downstreamStats.done} done, ${downstreamStats.inProgress} in progress, ${downstreamStats.blocked} blocked)`}
          >
            ↓ {downstreamStats.total} task{downstreamStats.total !== 1 ? 's' : ''}
            {downstreamStats.done > 0 && ` • ✓ ${downstreamStats.done}`}
            {downstreamStats.blocked > 0 && ` • ⚠ ${downstreamStats.blocked}`}
          </div>
        )}
      </div>
      <Handle type="target" position={Position.Left} />
      <Handle type="source" position={Position.Right} />
    </div>
  );
});

ClusterNode.displayName = 'ClusterNode';

export default ClusterNode;
