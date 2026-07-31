import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, waitFor } from '@testing-library/react';
import { GraphView } from '../GraphView';
import { apiClient } from '../../../api/client';
import { prepareClusteredGraphForReactFlow } from '../../../utils/clusteredGraphLayout';

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

vi.mock('../../../utils/clusteredGraphLayout', () => ({
  prepareClusteredGraphForReactFlow: vi.fn(() => ({
    clusters: [],
    crossClusterEdges: [],
    visibleNodes: [],
    visibleEdges: [],
    virtualEdges: [],
    orphanNodes: [],
  })),
}));

// Mock API client
vi.mock('../../../api/client', () => ({
  apiClient: {
    getGraph: vi.fn(() => Promise.resolve({
      count: 3,
      nodes: [
        {
          id: '1',
          label: 'Milestone v1.0',
          state: 'ready',
          priority: 'high',
          labels: ['milestone:v1.0'],
          blocked: false,
          type: 'milestone',
          parent: null,
          children: ['2'],
          cluster: '1',
          rank: 2,
        },
        {
          id: '2',
          label: 'Epic Auth',
          state: 'in_progress',
          priority: 'high',
          labels: ['epic:auth'],
          blocked: false,
          type: 'epic',
          parent: '1',
          children: ['3'],
          cluster: '1',
          rank: 1,
        },
        {
          id: '3',
          label: 'Task Login',
          state: 'done',
          priority: 'normal',
          labels: ['component:backend'],
          blocked: false,
          type: 'task',
          parent: '2',
          children: [],
          cluster: '1',
          rank: 0,
        },
      ],
      edges: [
        { from: '1', to: '2' },
        { from: '2', to: '3' },
      ],
    })),
    getHierarchy: vi.fn(() => Promise.resolve({
      types: { defaultType: 1 },
      strategic_types: ['defaultType'],
      icons: {},
    })),
  },
}));

describe('GraphView', () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
      removeItem: (key: string) => void store.delete(key),
    });
  });

  it('should render without crashing', async () => {
    render(<GraphView />);
    // Wait for async state updates to complete
    await waitFor(() => {
      // Component has completed loading
    });
  });

  it('should accept labelFilters prop', async () => {
    render(<GraphView labelFilters={['milestone:*']} />);
    await waitFor(() => {});
    // Component renders with label filters
  });

  it('should accept empty labelFilters', async () => {
    render(<GraphView labelFilters={[]} />);
    await waitFor(() => {});
    // Component renders with empty filters
  });

  it('should accept multiple label filters', async () => {
    render(<GraphView labelFilters={['milestone:*', 'epic:*']} />);
    await waitFor(() => {});
    // Component renders with multiple filters
  });

  it('test_graph_view_omits_clustering_when_hierarchy_fetch_fails', async () => {
    vi.mocked(apiClient.getHierarchy).mockImplementation(
      () => new Promise((_, reject) => {
        setTimeout(() => reject(new Error('configuration unavailable')), 25);
      })
    );
    vi.mocked(prepareClusteredGraphForReactFlow).mockClear();
    await act(async () => {
      render(<GraphView layoutAlgorithm="compact" />);
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    expect(apiClient.getGraph).toHaveBeenCalled();

    expect(prepareClusteredGraphForReactFlow).not.toHaveBeenCalled();
  });
});
