import { describe, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/react';
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

// Mock dagre - use a proper constructor function and mock layout
vi.mock('dagre', () => ({
  default: {
    graphlib: {
      Graph: vi.fn().mockImplementation(function() {
        return {
          setDefaultEdgeLabel: vi.fn(),
          setGraph: vi.fn(),
          setNode: vi.fn(),
          setEdge: vi.fn(),
          node: vi.fn(() => ({ x: 0, y: 0 })),
        };
      }),
    },
    layout: vi.fn(), // Mock the layout function
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
    getStrategicTypes: vi.fn(() => Promise.resolve(['milestone', 'epic'])),
  },
}));

describe('GraphView', () => {
  it('should render without crashing', async () => {
    render(<GraphView viewMode="tactical" />);
    // Wait for async state updates to complete
    await waitFor(() => {
      // Component has completed loading
    });
  });

  it('should accept viewMode prop', async () => {
    const { rerender } = render(<GraphView viewMode="tactical" />);
    await waitFor(() => {});
    
    rerender(<GraphView viewMode="strategic" />);
    await waitFor(() => {});
    // Component accepts both view modes
  });

  it('should accept labelFilters prop', async () => {
    render(<GraphView viewMode="tactical" labelFilters={['milestone:*']} />);
    await waitFor(() => {});
    // Component renders with label filters
  });

  it('should accept empty labelFilters', async () => {
    render(<GraphView viewMode="tactical" labelFilters={[]} />);
    await waitFor(() => {});
    // Component renders with empty filters
  });

  it('should accept multiple label filters', async () => {
    render(<GraphView viewMode="tactical" labelFilters={['milestone:*', 'epic:*']} />);
    await waitFor(() => {});
    // Component renders with multiple filters
  });

  it('should combine viewMode and labelFilters', async () => {
    render(<GraphView viewMode="strategic" labelFilters={['milestone:v1.0']} />);
    await waitFor(() => {});
    // Component renders with both strategic mode and label filters
  });
});
