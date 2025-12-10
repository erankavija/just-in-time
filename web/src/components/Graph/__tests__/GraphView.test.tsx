import { describe, it, vi } from 'vitest';
import { render } from '@testing-library/react';
import { GraphView } from '../GraphView';

// Mock ReactFlow
vi.mock('reactflow', () => ({
  default: () => null,
  Controls: () => null,
  Background: () => null,
  useNodesState: () => [[], vi.fn(), vi.fn()],
  useEdgesState: () => [[], vi.fn(), vi.fn()],
  MarkerType: { ArrowClosed: 'arrowclosed' },
  Position: { Left: 'left', Right: 'right' },
}));

// Mock dagre
vi.mock('dagre', () => ({
  default: {
    graphlib: {
      Graph: vi.fn(() => ({
        setDefaultEdgeLabel: vi.fn(),
        setGraph: vi.fn(),
        setNode: vi.fn(),
        setEdge: vi.fn(),
        node: vi.fn(() => ({ x: 0, y: 0 })),
      })),
    },
  },
}));

// Mock API client
vi.mock('../../../api/client', () => ({
  apiClient: {
    getGraph: vi.fn(() => Promise.resolve({
      nodes: [
        {
          id: '1',
          label: 'Milestone v1.0',
          state: 'ready',
          priority: 'high',
          labels: ['milestone:v1.0'],
          blocked: false,
        },
        {
          id: '2',
          label: 'Epic Auth',
          state: 'in_progress',
          priority: 'high',
          labels: ['epic:auth'],
          blocked: false,
        },
        {
          id: '3',
          label: 'Task Login',
          state: 'done',
          priority: 'normal',
          labels: ['component:backend'],
          blocked: false,
        },
      ],
      edges: [
        { from: '1', to: '2' },
        { from: '2', to: '3' },
      ],
    })),
  },
}));

describe('GraphView', () => {
  it('should render without crashing', () => {
    render(<GraphView viewMode="tactical" />);
    // Component renders successfully
  });

  it('should accept viewMode prop', () => {
    const { rerender } = render(<GraphView viewMode="tactical" />);
    rerender(<GraphView viewMode="strategic" />);
    // Component accepts both view modes
  });
});
